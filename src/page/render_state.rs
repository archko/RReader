use std::sync::{Arc, RwLock, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use log::debug;

use super::{Orientation, Page};
use crate::cache::PageCache;
use crate::decoder::DecodeService;
use crate::decoder::Rect;

/// 文档渲染核心状态（线程安全，适用于 Xilem/Vello 侧）
pub struct PageRenderState {
    pub decode_service: Arc<DecodeService>,
    pub cache: PageCache,
    /// paint 时检查此标记，若为 true 则请求下一帧以显示新解码的图片
    pub repaint_needed: AtomicBool,
    inner: RwLock<Inner>,
}

struct Inner {
    pub pages: Vec<Page>,
    pub view_offset: (f32, f32),
    pub zoom: f32,
    pub total_width: f32,
    pub total_height: f32,
    pub view_size: (f32, f32),
    pub visible_pages: Vec<usize>,
    pub orientation: Orientation,
    pub crop: i32,
    pub preload_screens: f32,
}

impl PageRenderState {
    pub fn new() -> Self {
        Self {
            decode_service: Arc::new(DecodeService::new()),
            cache: PageCache::new(24, 10),
            repaint_needed: AtomicBool::new(false),
            inner: RwLock::new(Inner {
                pages: Vec::new(),
                view_offset: (0.0, 0.0),
                zoom: 1.0,
                total_width: 0.0,
                total_height: 0.0,
                view_size: (0.0, 0.0),
                visible_pages: Vec::new(),
                orientation: Orientation::Vertical,
                crop: 0,
                preload_screens: 1.0,
            }),
        }
    }

    /// 读取访问（用于 paint）
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap()
    }

    /// 写入访问（用于 scroll / zoom）
    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap()
    }

    /// 设置页面（文档加载成功后调用）
    pub fn set_pages(&self, pages: Vec<Page>) {
        let mut inner = self.inner.write().unwrap();
        inner.pages = pages;
    }

    /// 更新视图尺寸并重新布局，同时重新计算可见页并提交解码
    pub fn update_view_size(&self, width: f32, height: f32, zoom: f32, force: bool) {
        let mut inner = self.inner.write().unwrap();
        let size_changed = inner.view_size.0 != width || inner.view_size.1 != height;
        let zoom_changed = (inner.zoom - zoom).abs() > 0.001;
        if !size_changed && !zoom_changed && !force {
            return;
        }
        inner.view_size = (width, height);
        inner.zoom = zoom;
        Self::recalculate_layout(&mut inner, &self.cache);
        // 尺寸变化 → 重新计算可见 tile → 提交解码
        submit_visible_decode_tasks(&self.cache, &self.decode_service, &mut inner);
    }

    /// 更新偏移并重新计算可见页面（写路径：触发解码提交）
    pub fn update_offset(&self, x: f32, y: f32) {
        let mut inner = self.inner.write().unwrap();
        inner.view_offset = (x, y);
        submit_visible_decode_tasks(&self.cache, &self.decode_service, &mut inner);
    }

    /// 重新计算布局
    fn recalculate_layout(inner: &mut Inner, _cache: &PageCache) {
        if inner.view_size.0 == 0.0 || inner.view_size.1 == 0.0 {
            return;
        }
        match inner.orientation {
            Orientation::Vertical => layout_vertical(inner),
            Orientation::Horizontal => layout_horizontal(inner),
        }
    }

    /// 跳转到指定页面
    pub fn jump_to_page(&self, page_index: usize) {
        let mut inner = self.inner.write().unwrap();
        if page_index >= inner.pages.len() {
            return;
        }
        let page = &inner.pages[page_index];
        let new_offset = match inner.orientation {
            Orientation::Vertical => (inner.view_offset.0, -page.bounds.top),
            Orientation::Horizontal => (-page.bounds.left, inner.view_offset.1),
        };
        inner.view_offset = new_offset;
        submit_visible_decode_tasks(&self.cache, &self.decode_service, &mut inner);
    }

    /// 调整缩放（工具按钮用）
    pub fn update_zoom(&self, new_zoom: f32) {
        let (vw, vh) = {
            let r = self.read();
            (r.view_size.0, r.view_size.1)
        };
        self.update_view_size(vw, vh, new_zoom, true);
    }

    pub fn close(&self) {
        let mut inner = self.inner.write().unwrap();
        inner.pages.clear();
        inner.visible_pages.clear();
        inner.total_width = 0.0;
        inner.total_height = 0.0;
        self.cache.clear();
    }
}

impl Default for PageRenderState {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 布局算法 =====

fn layout_vertical(inner: &mut Inner) {
    let view_width = inner.view_size.0;
    let scaled_width = view_width * inner.zoom;
    let mut current_y = 0.0;
    for page in &mut inner.pages {
        let pw = page.info.get_width(inner.crop == 1);
        let ph = page.info.get_height(inner.crop == 1);
        let scale = scaled_width / pw;
        let scaled_height = ph * scale;
        let bounds = Rect::new(0.0, current_y, scaled_width, current_y + scaled_height);
        page.update(scaled_width, scaled_height, bounds);
        page.info.scale = scale;
        current_y += scaled_height;
    }
    inner.total_width = scaled_width;
    inner.total_height = current_y;
}

fn layout_horizontal(inner: &mut Inner) {
    let view_height = inner.view_size.1;
    let scaled_height = view_height * inner.zoom;
    let mut current_x = 0.0;
    for page in &mut inner.pages {
        let pw = page.info.get_width(inner.crop == 1);
        let ph = page.info.get_height(inner.crop == 1);
        let scale = scaled_height / ph;
        let scaled_width = pw * scale;
        let bounds = Rect::new(current_x, 0.0, current_x + scaled_width, scaled_height);
        page.update(scaled_width, scaled_height, bounds);
        page.info.scale = scale;
        current_x += scaled_width;
    }
    inner.total_width = current_x;
    inner.total_height = scaled_height;
}

fn find_first_visible(pages: &[Page], visible_rect: &Rect, orientation: Orientation) -> usize {
    let mut low = 0;
    let mut high = pages.len();
    let mut result = pages.len();
    while low < high {
        let mid = (low + high) / 2;
        let page = &pages[mid];
        let is_visible = match orientation {
            Orientation::Vertical => page.bounds.bottom > visible_rect.top,
            Orientation::Horizontal => page.bounds.right > visible_rect.left,
        };
        if is_visible {
            result = mid;
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    result
}

/// 根据当前 offset / view_size 计算可见页面及其可见 tile。
/// 对每个可见页调用 Page::update_visible_nodes() 来确定哪些 tile 在视口内，
/// 并对缺缓存的 tile 提交解码。
/// 将旧可见集中已移出视口的 page 回收可见性资源。
fn submit_visible_decode_tasks(
    cache: &PageCache,
    decode_service: &DecodeService,
    inner: &mut Inner,
) {
    // 1) 记住旧的可见集
    let old_visible = std::mem::take(&mut inner.visible_pages);

    // 2) 计算新可见范围
    let (off_x, off_y) = inner.view_offset;
    let (vw, vh) = inner.view_size;
    let preload = match inner.orientation {
        Orientation::Vertical => vh * inner.preload_screens,
        Orientation::Horizontal => vw * inner.preload_screens,
    };
    let visible_rect = match inner.orientation {
        Orientation::Vertical => Rect::new(-off_x, -off_y, -off_x + vw, -off_y + vh + preload),
        Orientation::Horizontal => Rect::new(-off_x, -off_y, -off_x + vw + preload, -off_y + vh),
    };

    let first = find_first_visible(&inner.pages, &visible_rect, inner.orientation);
    let last = find_last_visible(&inner.pages, &visible_rect, inner.orientation);

    // 3) 回收移出视口的 page
    for &old_idx in &old_visible {
        if old_idx < first || old_idx > last {
            if let Some(page) = inner.pages.get_mut(old_idx) {
                page.recycle();
            }
        }
    }

    // 4) 对新可见页更新 tile 可见性 + 提交解码
    let mut total_tasks = 0;
    if first <= last && first < inner.pages.len() {
        for i in first..=last.min(inner.pages.len() - 1) {
            inner.visible_pages.push(i);
            let page = &mut inner.pages[i];
            if page.width > 0.0 && page.height > 0.0 {
                let tasks = page.update_visible_nodes(&visible_rect, cache, inner.crop);
                total_tasks += tasks.len();
                if !tasks.is_empty() {
                    decode_service.render_pages(tasks);
                }
            }
        }
    }
    debug!("visible_pages: {:?}, tasks: {}", inner.visible_pages, total_tasks);
}

/// 启动后台缓存消费线程。
/// 将解码完成的全页图片写入 LRU 缓存，并扩散到该页每个 node 的 cache_key 下。
/// paint 时每个 node 用自己的 key 查缓存，有就画，没有就跳过。
pub fn spawn_cache_consumer(state: Arc<PageRenderState>) {
    std::thread::spawn(move || {
        loop {
            match state.decode_service.try_recv_result() {
                Some(result) => {
                    if let Some(img) = image::RgbaImage::from_raw(
                        result.image_width,
                        result.image_height,
                        result.image_data,
                    ) {
                        let dyn_img = image::DynamicImage::ImageRgba8(img);
                        // 以 result.key（node 级 key）存储
                        state.cache.put_page_image_by_key(result.key, dyn_img);
                        state.repaint_needed.store(true, Ordering::Release);
                    }
                }
                None => {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    });
}

fn find_last_visible(pages: &[Page], visible_rect: &Rect, orientation: Orientation) -> usize {
    let mut low = 0;
    let mut high = pages.len();
    let mut result = 0;
    while low < high {
        let mid = (low + high) / 2;
        let page = &pages[mid];
        let is_visible = match orientation {
            Orientation::Vertical => page.bounds.top < visible_rect.bottom,
            Orientation::Horizontal => page.bounds.left < visible_rect.right,
        };
        if is_visible {
            result = mid;
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    result
}
