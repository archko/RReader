use std::sync::RwLock;

use log::debug;

use super::{Orientation, Page};
use crate::cache::PageCache;
use crate::decoder::decode_service::{Priority, RenderPage, VisibilityChecker};
use crate::decoder::pdf::utils::generate_thumbnail_key;
use crate::decoder::{DecodeService, Rect};
use std::sync::Arc;

/// 文档渲染核心状态（线程安全，适用于 Xilem/Vello 侧）
pub struct PageRenderState {
    pub decode_service: Arc<DecodeService>,
    pub cache: PageCache,
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

    /// 更新视图尺寸并重新布局
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
    }

    /// 更新偏移并重新计算可见页面
    pub fn update_offset(&self, x: f32, y: f32) {
        let mut inner = self.inner.write().unwrap();
        inner.view_offset = (x, y);
        let pages_info = self.collect_visible_pages(&mut inner);
        // 提交解码任务
        if !pages_info.is_empty() {
            self.decode_service.render_pages(pages_info);
        }
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

    /// 计算可见页面列表并返回 RenderPage 列表
    fn collect_visible_pages(&self, inner: &mut Inner) -> Vec<RenderPage> {
        inner.visible_pages.clear();
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

        let mut render_tasks = Vec::new();
        if first <= last && first < inner.pages.len() {
            for i in first..=last.min(inner.pages.len() - 1) {
                inner.visible_pages.push(i);
                let page = &inner.pages[i];
                if page.width > 0.0 && page.height > 0.0 {
                    let key = generate_thumbnail_key(page);
                    if self.cache.get_page_image_by_key(&key).is_none() {
                        render_tasks.push(RenderPage {
                            key,
                            page_info: page.info.clone(),
                            crop: inner.crop,
                            priority: Priority::Thumbnail,
                            visibility_checker: None,
                        });
                    }
                }
            }
        }
        debug!(
            "visible_pages: {:?}, tasks: {}",
            inner.visible_pages, render_tasks.len()
        );
        render_tasks
    }

    /// 处理解码结果（返回 true 表示有新的图像数据）
    pub fn poll_decode_results(&self) -> bool {
        let mut updated = false;
        while let Some(result) = self.decode_service.try_recv_result() {
            if let Some(img) = image::RgbaImage::from_raw(
                result.image_width,
                result.image_height,
                result.image_data,
            ) {
                self.cache
                    .put_page_image_by_key(result.key, image::DynamicImage::ImageRgba8(img));
                updated = true;
            }
        }
        updated
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
        let tasks = self.collect_visible_pages(&mut inner);
        if !tasks.is_empty() {
            self.decode_service.render_pages(tasks);
        }
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

// ===== 布局算法（与 PageViewState 相同） =====

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
