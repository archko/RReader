use std::sync::{Arc, RwLock, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use log::debug;

use super::Page;
use super::Orientation;
use crate::cache::PageCache;
use crate::decoder::DecodeService;
use crate::decoder::decode_service::{RenderPage, TaskType};
use crate::decoder::Rect;
use crate::entity::OutlineItem;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

pub struct PageRenderState {
    pub decode_service: Arc<DecodeService>,
    pub cache: PageCache,
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

    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap()
    }

    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap()
    }

    pub fn set_pages(&self, pages: Vec<Page>) {
        let mut inner = self.inner.write().unwrap();
        inner.pages = pages;
    }

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

    /// 更新偏移并重新计算可见页
    pub fn update_offset(&self, x: f32, y: f32) {
        let mut inner = self.inner.write().unwrap();
        inner.view_offset = (x, y);
        Self::recalculate_visible_pages(&mut inner);
    }

    fn recalculate_layout(inner: &mut Inner) {
        if inner.view_size.0 == 0.0 || inner.view_size.1 == 0.0 {
            return;
        }
        match inner.orientation {
            Orientation::Vertical => layout_vertical(inner),
            Orientation::Horizontal => layout_horizontal(inner),
        }
    }

    /// 可见页计算 —— scaleRatio 修正（支持 zoom 变化后不重算 layout 的场景）
    fn recalculate_visible_pages(inner: &mut Inner) {
        let old_visible = std::mem::take(&mut inner.visible_pages);

        let visible_rect = compute_visible_rect(
            inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens,
        );

        let scale_ratio = 1.0;
        let first = find_first_visible(&inner.pages, &visible_rect, inner.orientation, scale_ratio);
        let last = find_last_visible(&inner.pages, &visible_rect, inner.orientation, scale_ratio);

        for &old_idx in &old_visible {
            if old_idx < first || old_idx > last {
                if let Some(page) = inner.pages.get_mut(old_idx) {
                    page.recycle();
                }
            }
        }

        if first <= last && first < inner.pages.len() {
            for i in first..=last.min(inner.pages.len() - 1) {
                inner.visible_pages.push(i);
            }
        }
    }

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

    pub fn update_zoom(&self, new_zoom: f32) {
        let (vw, vh) = {
            let r = self.read();
            (r.view_size.0, r.view_size.1)
        };
        self.update_view_size(vw, vh, new_zoom, true);
    }

    /// 关闭并清理资源
    pub fn close(&self) {
        {
            let mut inner = self.inner.write().unwrap();
            for page in &mut inner.pages {
                page.recycle();
                page.clear_thumb();
            }
            inner.pages.clear();
            inner.visible_pages.clear();
            inner.total_width = 0.0;
            inner.total_height = 0.0;
        }
        self.cache.clear();
        self.decode_service.destroy();
    }
}

impl Default for PageRenderState {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 可见页节点管理 + 解码提交（委托给 Page） =====

pub fn process_visible_nodes(state: &PageRenderState) {
    let mut inner = state.inner.write().unwrap();
    let visible_rect = compute_visible_rect(
        inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens,
    );
    let crop = inner.crop;
    let zoom = inner.zoom;
    let orientation = inner.orientation;

    for &page_idx in &inner.visible_pages {
        if let Some(page) = inner.pages.get_mut(page_idx) {
            page.update_visible_nodes(
                &visible_rect,
                &state.decode_service,
                &state.cache,
                crop,
                zoom,
                orientation,
            );

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
                    state.decode_service.render_pages(vec![RenderPage {
                        key: thumb_key,
                        page_info: thumb_info,
                        crop: 0,
                        task_type: TaskType::Page,
                        visibility_checker: None,
                    }]);
                }
            }
        }
    }

    state.repaint_needed.store(true, Ordering::Release);
}

// ===== 解码结果消费（由 paint 方法在主线程轮询，见 document_canvas.rs） =====

pub fn consume_decode_result(state: &PageRenderState, result: crate::decoder::decode_service::DecodeResult) {
    let is_thumb = result.key.starts_with("thumb-");
    let key = result.key.clone();

    if let Some(img) = image::RgbaImage::from_raw(
        result.image_width,
        result.image_height,
        result.image_data,
    ) {
        let dyn_img = image::DynamicImage::ImageRgba8(img);
        if is_thumb {
            state.cache.put_thumbnail(key.clone(), dyn_img);
        } else {
            state.cache.put_page_image_by_key(key.clone(), dyn_img);
        }
    }

    let mut inner = state.inner.write().unwrap();
    if is_thumb {
        if let Some(page) = inner.pages.get_mut(result.page_info.index) {
            if page.is_thumb_loading {
                page.is_thumb_loading = false;
                if let Some(img) = state.cache.get_thumbnail(&key) {
                    page.thumb_bitmap = Some(img);
                }
            }
        }
    } else {
        for page in &mut inner.pages {
            let mut matched = false;
            for (_nk, node) in &mut page.visible_nodes {
                if node.cache_key == key && node.is_decoding {
                    node.is_decoding = false;
                    if let Some(img) = state.cache.get_page_image_by_key(&key) {
                        node.bitmap = Some(img);
                    }
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
        }
    }

    state.repaint_needed.store(true, Ordering::Release);
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
        page.update(scaled_width, scaled_height, bounds, inner.zoom);
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
        page.update(scaled_width, scaled_height, bounds, inner.zoom);
        page.info.scale = scale;
        current_x += scaled_width;
    }
    inner.total_width = current_x;
    inner.total_height = scaled_height;
}

fn compute_visible_rect(
    offset: (f32, f32), view_size: (f32, f32),
    orientation: Orientation, preload_screens: f32,
) -> Rect {
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

fn find_first_visible(
    pages: &[Page], visible_rect: &Rect,
    orientation: Orientation, scale_ratio: f32,
) -> usize {
    let mut low = 0;
    let mut high = pages.len();
    let mut result = pages.len();
    while low < high {
        let mid = (low + high) / 2;
        let page = &pages[mid];
        let is_visible = match orientation {
            Orientation::Vertical => page.bounds.bottom * scale_ratio > visible_rect.top,
            Orientation::Horizontal => page.bounds.right * scale_ratio > visible_rect.left,
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

fn find_last_visible(
    pages: &[Page], visible_rect: &Rect,
    orientation: Orientation, scale_ratio: f32,
) -> usize {
    let mut low = 0;
    let mut high = pages.len();
    let mut result = 0;
    while low < high {
        let mid = (low + high) / 2;
        let page = &pages[mid];
        let is_visible = match orientation {
            Orientation::Vertical => page.bounds.top * scale_ratio < visible_rect.bottom,
            Orientation::Horizontal => page.bounds.left * scale_ratio < visible_rect.right,
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
