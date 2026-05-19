use std::sync::{Arc, RwLock, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use log::debug;

use super::Page;
use crate::cache::PageCache;
use crate::decoder::DecodeService;
use crate::decoder::decode_service::{RenderPage, Priority};
use crate::decoder::Rect;
use crate::entity::OutlineItem;

/// 滚动方向
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// 文档渲染核心状态（线程安全）
/// 职责：纯状态管理——布局计算、可见页面维护。
/// 解码任务提交由外部函数 `process_visible_nodes` 负责。
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
    pub outline_items: Vec<OutlineItem>,
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
                outline_items: Vec::new(),
            }),
        }
    }

    /// 读取访问（用于 paint）
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap()
    }

    /// 写入访问
    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap()
    }

    /// 设置页面（文档加载成功后调用）
    pub fn set_pages(&self, pages: Vec<Page>) {
        let mut inner = self.inner.write().unwrap();
        inner.pages = pages;
    }

    // ===== 纯状态管理方法（不访问 cache / decode_service） =====

    /// 更新视图尺寸并重新布局 + 重新计算可见页
    pub fn update_view_size(&self, width: f32, height: f32, zoom: f32, force: bool) {
        let mut inner = self.inner.write().unwrap();
        let size_changed = inner.view_size.0 != width || inner.view_size.1 != height;
        let zoom_changed = (inner.zoom - zoom).abs() > 0.001;
        if !size_changed && !zoom_changed && !force {
            return;
        }
        inner.view_size = (width, height);
        inner.zoom = zoom;
        Self::recalculate_layout(&mut inner);
        Self::recalculate_visible_pages(&mut inner);
    }

    /// 更新偏移并重新计算可见页（纯状态）
    pub fn update_offset(&self, x: f32, y: f32) {
        let mut inner = self.inner.write().unwrap();
        inner.view_offset = (x, y);
        Self::recalculate_visible_pages(&mut inner);
    }

    /// 重新计算布局
    fn recalculate_layout(inner: &mut Inner) {
        if inner.view_size.0 == 0.0 || inner.view_size.1 == 0.0 {
            return;
        }
        match inner.orientation {
            Orientation::Vertical => layout_vertical(inner),
            Orientation::Horizontal => layout_horizontal(inner),
        }
    }

    /// 纯可见页计算（二分查找 + 差异回收，不涉及解码）
    fn recalculate_visible_pages(inner: &mut Inner) {
        let old_visible = std::mem::take(&mut inner.visible_pages);

        let visible_rect = compute_visible_rect(inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens);

        let first = find_first_visible(&inner.pages, &visible_rect, inner.orientation);
        let last = find_last_visible(&inner.pages, &visible_rect, inner.orientation);

        // 回收不再可见的 page
        for &old_idx in &old_visible {
            if old_idx < first || old_idx > last {
                if let Some(page) = inner.pages.get_mut(old_idx) {
                    page.recycle();
                }
            }
        }

        // 设置新的可见页列表
        if first <= last && first < inner.pages.len() {
            for i in first..=last.min(inner.pages.len() - 1) {
                inner.visible_pages.push(i);
            }
        }
    }

    /// 跳转到指定页面（纯状态）
    pub fn jump_to_page(&self, page_index: usize) {
        let mut inner = self.inner.write().unwrap();
        if page_index >= inner.pages.len() {
            return;
        }
        let page_bounds = inner.pages[page_index].bounds;
        let new_offset = match inner.orientation {
            Orientation::Vertical => (inner.view_offset.0, -page_bounds.top),
            Orientation::Horizontal => (-page_bounds.left, inner.view_offset.1),
        };
        inner.view_offset = new_offset;
        Self::recalculate_visible_pages(&mut inner);
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

// ===== 可见页解码提交（与状态管理分离） =====

/// 计算当前可见页面的瓦片 node 并提交缺失的解码任务。
/// 此函数不修改状态管理的核心字段（offset / zoom / visible_pages），
/// 只负责读取可见页列表 → 创建/回收 PageNode → 检查缓存 → 提交解码。
pub fn process_visible_nodes(state: &PageRenderState) {
    let mut inner = state.inner.write().unwrap();
    let visible_rect = compute_visible_rect(inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens);
    let crop = inner.crop;
    let mut all_tasks: Vec<RenderPage> = Vec::new();

    for &page_idx in &inner.visible_pages {
        if let Some(page) = inner.pages.get_mut(page_idx) {
            // 1) 瓦片 node 管理（仅创建/回收）
            let decode_keys = page.update_visible_nodes(&visible_rect);

            // 2) 懒加载链接
            //page.load_links();

            // 3) 为缺失的瓦片提交解码
            for &key in &decode_keys {
                if let Some(node) = page.visible_nodes.get_mut(&key) {
                    if node.is_decoding {
                        continue;
                    }
                    if state.cache.get_page_image_by_key(&node.cache_key).is_some() {
                        continue;
                    }
                    node.is_decoding = true;
                    all_tasks.push(RenderPage {
                        key: node.cache_key.clone(),
                        page_info: page.info.clone(),
                        crop,
                        priority: Priority::FullImage,
                        visibility_checker: None,
                    });
                }
            }

            // 4) 缩略图检查 / 提交
            let thumb_key = format!("thumb-{}", page.info.index);
            if page.thumb_bitmap.is_none() && !page.is_thumb_loading {
                if let Some(img) = state.cache.get_thumbnail(&thumb_key) {
                    page.thumb_bitmap = Some(img);
                } else {
                    page.is_thumb_loading = true;
                    let max_original = page.info.width.max(page.info.height);
                    let thumb_scale = 300.0 / max_original;
                    let mut thumb_info = page.info.clone();
                    thumb_info.scale = thumb_scale;
                    all_tasks.push(RenderPage {
                        key: thumb_key,
                        page_info: thumb_info,
                        crop: 0,
                        priority: Priority::Thumbnail,
                        visibility_checker: None,
                    });
                }
            }
        }
    }

    if !all_tasks.is_empty() {
        state.decode_service.render_pages(all_tasks);
    }
}

// ===== 后台缓存消费者 =====

/// 启动后台缓存消费线程。
/// 将解码完成的位图写入 LRU 缓存（缩略图走 thumbnail_cache，瓦片走 image_cache），
/// 并设置 repaint_needed 标记触发重绘。
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
                        if result.key.starts_with("thumb-") {
                            state.cache.put_thumbnail(result.key, dyn_img);
                        } else {
                            state.cache.put_page_image_by_key(result.key, dyn_img);
                        }
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

fn compute_visible_rect(offset: (f32, f32), view_size: (f32, f32), orientation: Orientation, preload_screens: f32) -> Rect {
    let (off_x, off_y) = offset;
    let (vw, vh) = view_size;
    match orientation {
        Orientation::Vertical => {
            let preload = vh * preload_screens;
            Rect::new(-off_x, -off_y, -off_x + vw, -off_y + vh + preload)
        }
        Orientation::Horizontal => {
            let preload = vw * preload_screens;
            Rect::new(-off_x, -off_y, -off_x + vw + preload, -off_y + vh)
        }
    }
}

// ===== 二分查找 =====

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
