use std::sync::{Arc, RwLock, atomic::{AtomicBool, Ordering}};

use log::debug;
use xilem::masonry::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

use super::Page;
use crate::cache::PageCache;
use crate::decoder::DecodeService;
use crate::decoder::decode_service::{RenderPage, TaskType, DecodeCallback, DecodeResult};
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
    // 👇 新增：用于存放唤醒 UI 的跨线程回调
    pub wake_up_ui: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    inner: RwLock<Inner>,
}

pub(crate) struct Inner {
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

pub struct PageCallback {
    pub state: Arc<PageRenderState>,
    pub page_idx: usize,
    /// None = 缩略图模式，Some(key) = 瓦片模式
    pub node_key: Option<usize>,
    pub cache_key: String,
}

impl DecodeCallback for PageCallback {
    fn should_render(&self, _page_idx: usize) -> bool {
        match self.node_key {
            Some(nk) => self.state.read().pages.get(self.page_idx)
                .and_then(|p| p.visible_nodes.get(&nk))
                .map(|n| n.cache_key == self.cache_key && n.is_decoding)
                .unwrap_or(false),
            None => self.state.read().visible_pages.contains(&self.page_idx),
        }
    }

    fn on_completed(&self, result: DecodeResult) {
        if result.key != self.cache_key { return; }

        let blob = Blob::from(result.image_data);
        let image_data = ImageData {
            data: blob,
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: result.image_width,
            height: result.image_height,
        };
        match self.node_key {
            Some(nk) => {
                let arc = self.state.cache.put_page_image_by_key(self.cache_key.clone(), image_data);
                let mut inner = self.state.write();
                if let Some(page) = inner.pages.get_mut(self.page_idx) {
                    if let Some(node) = page.visible_nodes.get_mut(&nk) {
                        if node.cache_key == self.cache_key {
                            node.bitmap = Some(arc);
                            node.is_decoding = false;
                        }
                    }
                }
            }
            None => {
                let arc = self.state.cache.put_thumbnail(self.cache_key.clone(), image_data);
                let mut inner = self.state.write();
                if let Some(page) = inner.pages.get_mut(self.page_idx) {
                    if page.is_thumb_loading {
                        page.thumb_bitmap = Some(arc);
                        page.is_thumb_loading = false;
                    }
                    if !result.links.is_empty() {
                        page.links = result.links;
                        page.links_loaded = true;
                    }
                }
            }
        }
        //self.state.repaint_needed.store(true, Ordering::Release);
        self.state.repaint_needed.store(true, Ordering::SeqCst);
        // 解码完成，立刻跨线程拍醒主线程的事件循环
        self.state.wake_ui(); 
    }

    fn on_error(&self, _page_idx: usize) {
        let mut inner = self.state.write();
        if let Some(page) = inner.pages.get_mut(self.page_idx) {
            match self.node_key {
                Some(nk) => {
                    if let Some(node) = page.visible_nodes.get_mut(&nk) {
                        node.is_decoding = false;
                    }
                }
                None => page.is_thumb_loading = false,
            }
        }
        self.state.repaint_needed.store(true, Ordering::Release);
    }
}

impl PageRenderState {
    pub fn new() -> Self {
        Self {
            decode_service: Arc::new(DecodeService::new()),
            cache: PageCache::new(32, 20),
            repaint_needed: AtomicBool::new(false),
            wake_up_ui: RwLock::new(None),
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
    
    // 👇 新增：允许 UI 视图在构建时注册唤醒函数
    pub fn set_wake_up_callback(&self, callback: impl Fn() + Send + Sync + 'static) {
        let mut wake = self.wake_up_ui.write().unwrap();
        *wake = Some(Arc::new(callback));
    }

    // 👇 新增：触发唤醒
    pub fn wake_ui(&self) {
        if let Some(callback) = self.wake_up_ui.read().unwrap().as_ref() {
            callback();
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

    fn recalculate_visible_pages(inner: &mut Inner) {
        let old_visible = std::mem::take(&mut inner.visible_pages);

        let visible_rect = compute_visible_rect(
            inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens,
        );

        // scale_ratio 用于将页面 bounds 从布局坐标系转换到视图坐标系
        // 当 zoom = 1.0 时，页面 bounds 已经是基于视口大小计算的，无需额外缩放
        let first = find_first_visible(&inner.pages, &visible_rect, inner.orientation);
        let last = find_last_visible(&inner.pages, &visible_rect, inner.orientation);

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
    }
}

impl Default for PageRenderState {
    fn default() -> Self {
        Self::new()
    }
}

fn thumbnail_cache_key(page_index: usize, crop: i32) -> String {
    format!("thumb-{}-{}", page_index, crop)
}

fn calculate_thumbnail_scale(page_width: f32, page_height: f32, target_width: f32) -> f32 {
    /*let max_dim = page_width.max(page_height);
    let base_size = if max_dim > 100_000.0 { 60.0 }
        else if max_dim > 30_000.0 { 80.0 }
        else if max_dim > 20_000.0 { 120.0 }
        else if max_dim > 10_000.0 { 180.0 }
        else { 360.0 };
    base_size / max_dim*/
    if page_width > 0.0 {
        target_width / page_width
    } else {
        1.0
    }
}

// ===== 可见页节点管理 + 解码提交 =====

pub fn process_visible_nodes(state: &Arc<PageRenderState>) {
    let mut inner = state.inner.write().unwrap();
    let visible_rect = compute_visible_rect(
        inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens,
    );
    let crop = inner.crop;
    let zoom = inner.zoom;
    let orientation = inner.orientation;
    let view_width = inner.view_size.0;

    let visible_pages = inner.visible_pages.clone();
    for &page_idx in &visible_pages {
        if let Some(page) = inner.pages.get_mut(page_idx) {
            let thumb_key = thumbnail_cache_key(page.info.index, crop);
            if page.thumb_bitmap.is_none() && !page.is_thumb_loading {
                if let Some(img) = state.cache.get_thumbnail(&thumb_key) {
                    page.thumb_bitmap = Some(img);
                } else {
                    page.is_thumb_loading = true;
                    let thumb_scale = calculate_thumbnail_scale(page.info.width, page.info.height, view_width * zoom);
                    let mut thumb_info = page.info.clone();
                    thumb_info.scale = thumb_scale;
                    state.decode_service.render_pages(vec![RenderPage {
                        key: thumb_key.clone(),
                        page_info: thumb_info,
                        crop,
                        task_type: TaskType::Page,
                        callback: Some(Arc::new(PageCallback {
                            state: Arc::clone(state),
                            page_idx: page.info.index,
                            node_key: None,
                            cache_key: thumb_key,
                        })),
                        region: None,
                    }]);
                }
            }

            /*page.update_visible_nodes(
                &visible_rect, &state.decode_service, &state.cache,
                crop, zoom, orientation, Arc::clone(state),
            );*/
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

fn find_first_visible(
    pages: &[Page], visible_rect: &Rect, orientation: Orientation,
) -> usize {
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

fn find_last_visible(
    pages: &[Page], visible_rect: &Rect, orientation: Orientation,
) -> usize {
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
