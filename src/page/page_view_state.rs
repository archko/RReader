use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use floem::ext_event::{register_ext_trigger, create_trigger, ExtSendTrigger};

use anyhow::Result;
use log::{debug, info};

use super::{Page, PageNode};
use crate::cache::PageCache;
use crate::decoder::{DecodeService, Link, PageInfo, Rect};
use crate::decoder::decode_service::{DecodeCallback, DecodeResult, DecodeCallbackRef, RenderPage, TaskType};
use crate::entity::OutlineItem;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// 页面视图状态 - 与 PageRenderState 相同设计模式
/// 使用 RwLock<Inner> 保证线程安全，供回调线程和 UI 线程共享
pub struct PageViewState {
    pub decode_service: Arc<DecodeService>,
    pub cache: PageCache,
    pub repaint_trigger: ExtSendTrigger,
    pub page_links: Arc<std::sync::Mutex<HashMap<usize, Vec<Link>>>>,
    inner: RwLock<Inner>,
    self_arc: OnceLock<Arc<Self>>,
}

pub(crate) struct Inner {
    pub pages: Vec<Page>,
    pub zoom: f32,
    pub view_size: (f32, f32),
    pub total_width: f32,
    pub total_height: f32,
    pub visible_pages: Vec<usize>,
    pub orientation: Orientation,
    pub crop: i32,
    pub preload_screens: f32,
    pub outline_items: Vec<OutlineItem>,
    pub view_offset: (f32, f32),
}

// ===== PageCallback - 解码回调 =====

pub struct PageCallback {
    pub state: Arc<PageViewState>,
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

        // 将原始 RGBA 数据转换为 DynamicImage
        let rgba = image::RgbaImage::from_raw(
            result.image_width,
            result.image_height,
            result.image_data,
        );
        let dynamic_image = match rgba {
            Some(img) => image::DynamicImage::ImageRgba8(img),
            None => {
                log::error!("[PageCallback] Failed to create image from raw data");
                return;
            }
        };

        match self.node_key {
            Some(nk) => {
                let arc = self.state.cache.put_page_image_by_key(
                    self.cache_key.clone(),
                    dynamic_image,
                );
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
                let arc = self.state.cache.put_thumbnail(
                    self.cache_key.clone(),
                    dynamic_image,
                );
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
        register_ext_trigger(self.state.repaint_trigger);
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
        register_ext_trigger(self.state.repaint_trigger);
    }
}

// ===== PageViewState 实现 =====

impl PageViewState {
    pub fn new(orientation: Orientation, crop: i32) -> Self {
        Self {
            decode_service: Arc::new(DecodeService::new()),
            cache: PageCache::new(32, 20),
            repaint_trigger: create_trigger(),
            page_links: Arc::new(std::sync::Mutex::new(HashMap::new())),
            inner: RwLock::new(Inner {
                pages: Vec::new(),
                zoom: 1.0,
                view_size: (0.0, 0.0),
                total_width: 0.0,
                total_height: 0.0,
                visible_pages: Vec::new(),
                orientation,
                crop,
                preload_screens: 1.0,
                outline_items: Vec::new(),
                view_offset: (0.0, 0.0),
            }),
            self_arc: OnceLock::new(),
        }
    }

    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap()
    }

    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap()
    }

    // ===== 文档操作 =====

    /// 打开文档（异步，通过 decode_service 加载）
    pub fn init_self_arc(&self, arc: Arc<Self>) {
        self.self_arc.set(arc).ok();
    }

    pub fn open_document(&self, path: &Path) -> Result<()> {
        self.decode_service.load_pdf(path)
    }

    /// 从 PageInfo 列表设置页面
    pub fn set_pages_from_info(&self, pages_info: Vec<PageInfo>) {
        let crop = { self.read().crop };
        let pages: Vec<Page> = pages_info
            .into_iter()
            .map(|info| Page::new(info, 0.0, 0.0, 0.0, 0.0, 1.0, crop))
            .collect();
        let mut inner = self.inner.write().unwrap();
        inner.pages = pages;
    }

    // ===== 视口与布局 =====

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
        let visible_rect = compute_visible_rect(
            (x, y), inner.view_size, inner.orientation, inner.preload_screens,
        );
        // 暂时用 view_offset 的概念 - 将 x,y 存储为偏移
        // 但 Inner 没有 view_offset 字段，通过 visible_rect 计算可见页
        let old_visible = std::mem::take(&mut inner.visible_pages);

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
        inner.view_offset = match inner.orientation {
            Orientation::Vertical => (0.0, page_bounds.top),
            Orientation::Horizontal => (page_bounds.left, 0.0),
        };
        // 重新计算可见范围，以目标页面为中心
        let visible_rect = match inner.orientation {
            Orientation::Vertical => {
                let preload = inner.view_size.1 * inner.preload_screens;
                Rect::new(0.0, page_bounds.top, inner.view_size.0, page_bounds.top + inner.view_size.1 + preload)
            }
            Orientation::Horizontal => {
                let preload = inner.view_size.0 * inner.preload_screens;
                Rect::new(page_bounds.left, 0.0, page_bounds.left + inner.view_size.0 + preload, inner.view_size.1)
            }
        };
        let old_visible = std::mem::take(&mut inner.visible_pages);

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

    pub fn update_zoom(&self, new_zoom: f32) {
        let (vw, vh) = {
            let r = self.read();
            (r.view_size.0, r.view_size.1)
        };
        self.update_view_size(vw, vh, new_zoom, true);
    }

    // ===== 可见页面管理 =====

    /// 重新计算可见页面（用于解码线程触发的刷新）
    pub fn update_visible_pages(&self) {
        let mut inner = self.inner.write().unwrap();
        Self::recalculate_visible_pages(&mut inner);
    }

    pub fn get_first_visible_page(&self) -> Option<usize> {
        self.read().visible_pages.first().copied()
    }

    /// 获取可见页面的引用（方便 canvas 绘制）
    pub fn get_visible_pages_snapshot(&self) -> Vec<(usize, String)> {
        let inner = self.read();
        inner.visible_pages.iter().filter_map(|&idx| {
            inner.pages.get(idx).map(|p| {
                let key = thumbnail_cache_key(p.info.index, inner.crop);
                (idx, key)
            })
        }).collect()
    }

    // ===== 资源清理 =====

    pub fn shutdown(&self) {
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
        if let Ok(mut links) = self.page_links.lock() {
            links.clear();
        }
    }

    // ===== 内部布局/可见页计算 =====

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
}

impl Default for PageViewState {
    fn default() -> Self {
        Self::new(Orientation::Vertical, 0)
    }
}

// ===== 缩略图管理 =====

fn thumbnail_cache_key(page_index: usize, crop: i32) -> String {
    format!("thumb-{}-{}", page_index, crop)
}

fn calculate_thumbnail_scale(page_width: f32, page_height: f32) -> f32 {
    let max_dim = page_width.max(page_height);
    let base_size = if max_dim > 100_000.0 { 60.0 }
        else if max_dim > 30_000.0 { 80.0 }
        else if max_dim > 20_000.0 { 120.0 }
        else if max_dim > 10_000.0 { 180.0 }
        else { 360.0 };
    base_size / max_dim
}

// ===== 可见页节点管理 + 解码提交 =====

impl PageViewState {
    pub fn process_visible_nodes(&self) {
        let self_arc = self.self_arc.get().expect("self_arc not initialized");
        let mut inner = self.inner.write().unwrap();
        let visible_rect = compute_visible_rect(
            inner.view_offset, inner.view_size, inner.orientation, inner.preload_screens,
        );
        let crop = inner.crop;
        let zoom = inner.zoom;
        let orientation = inner.orientation;

        let visible_pages = inner.visible_pages.clone();
        for &page_idx in &visible_pages {
            if let Some(page) = inner.pages.get_mut(page_idx) {
                let thumb_key = thumbnail_cache_key(page.info.index, crop);
                if page.thumb_bitmap.is_none() && !page.is_thumb_loading {
                    if let Some(img) = self.cache.get_thumbnail(&thumb_key) {
                        page.thumb_bitmap = Some(img);
                    } else {
                        page.is_thumb_loading = true;
                        let thumb_scale = calculate_thumbnail_scale(page.info.width, page.info.height);
                        let mut thumb_info = page.info.clone();
                        thumb_info.scale = thumb_scale;
                        self.decode_service.render_pages(vec![RenderPage {
                            key: thumb_key.clone(),
                            page_info: thumb_info,
                            crop,
                            task_type: TaskType::Page,
                            callback: Some(Arc::new(PageCallback {
                                state: Arc::clone(self_arc),
                                page_idx: page.info.index,
                                node_key: None,
                                cache_key: thumb_key,
                            })),
                            region: None,
                        }]);
                    }
                }

                page.update_visible_nodes(
                    &visible_rect, &self.decode_service, &self.cache,
                    crop, zoom, orientation, Arc::clone(self_arc),
                );
            }
        }

        register_ext_trigger(self.repaint_trigger);
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
