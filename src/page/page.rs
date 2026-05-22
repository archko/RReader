use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use arc_swap::ArcSwapOption;
use floem::context::PaintCx;
use floem::kurbo::Rect as KurboRect;
use floem::peniko::{Color, ImageData};
use floem::Renderer;
use log::info;

use super::{PageNode, PageNodePool, Orientation, PageViewState};
use crate::cache::PageCache;
use crate::decoder::{Link, PageInfo, Rect};
use crate::decoder::decode_service::{DecodeService, DecodeCallbackRef};
use super::page_view_state::PageCallback;

pub struct Page {
    pub info: PageInfo,
    pub bounds: Rect,
    pub visible_nodes: HashMap<usize, PageNode>,
    pub width: f32,
    pub height: f32,
    pub is_decoding: bool,

    pub thumb_bitmap: ArcSwapOption<ImageData>,
    pub is_thumb_loading: AtomicBool,
    pub x_offset: f32,
    pub y_offset: f32,
    pub total_scale: f32,
    pub base_zoom: f32,
    pub links: Mutex<Vec<Link>>,
    pub links_loaded: AtomicBool,
    pub tile_config: TileConfig,
    pub crop: i32,
    node_pool: PageNodePool,
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32, base_zoom: f32, crop: i32) -> Self {
        let bounds = Rect::new(x_offset, y_offset, x_offset + width, y_offset + height);
        let tile_config = TileConfig::from_size(width, height);
        let total_scale = if width > 0.0 { width / info.width } else { 1.0 };
        Self {
            info,
            bounds,
            visible_nodes: HashMap::new(),
            links: Mutex::new(Vec::new()),
            width,
            height,
            is_decoding: false,
            thumb_bitmap: ArcSwapOption::new(None),
            is_thumb_loading: AtomicBool::new(false),
            x_offset,
            y_offset,
            total_scale,
            base_zoom,
            links_loaded: AtomicBool::new(false),
            tile_config,
            crop,
            node_pool: PageNodePool::new(),
        }
    }

    pub fn update(&mut self, width: f32, height: f32, bounds: Rect, base_zoom: f32) {
        self.width = width;
        self.height = height;
        self.bounds = bounds;
        self.x_offset = bounds.left;
        self.y_offset = bounds.top;
        self.total_scale = width / self.info.width;
        self.base_zoom = base_zoom;
        self.invalidate_nodes();
    }

    pub fn invalidate_nodes(&mut self) {
        self.tile_config = TileConfig::from_size(self.width, self.height);
    }

    pub fn x_offset(&self) -> f32 { self.bounds.left }
    pub fn y_offset(&self) -> f32 { self.bounds.top }

    pub fn update_visible_nodes(
        &mut self,
        viewport: &Rect,
        decode_service: &DecodeService,
        cache: &PageCache,
        crop: i32,
        zoom: f32,
        orientation: Orientation,
        state_arc: Arc<PageViewState>,
    ) {
        let config = &self.tile_config;
        let ori = match orientation {
            Orientation::Vertical => 0,
            Orientation::Horizontal => 1,
        };

        if config.is_single_block() {
            let old_keys: Vec<usize> = self.visible_nodes.keys().copied().collect();
            for k in &old_keys {
                if let Some(n) = self.visible_nodes.remove(k) {
                    self.node_pool.release(n);
                }
            }
            let bounds = Rect::new(0.0, 0.0, 1.0, 1.0);
            let node = self.node_pool.acquire(self.info.index, bounds, zoom, ori, crop);
            self.visible_nodes.insert(0, node);
            if let Some(n) = self.visible_nodes.get_mut(&0) {
                if n.needs_decoding() && cache.get_page_image_by_key(&n.cache_key).is_none() {
                    let cb: DecodeCallbackRef = Arc::new(PageCallback {
                        state: Arc::clone(&state_arc),
                        page_idx: self.info.index,
                        node_key: Some(0),
                        cache_key: n.cache_key.clone(),
                    });
                    n.decode(self.width, self.height, &self.info, crop, decode_service, cb);
                }
            }
            return;
        }

        let (col_range, row_range) = self.visible_tile_ranges(config, viewport);

        let mut needed: Vec<usize> = Vec::new();
        for row in row_range {
            for col in col_range.clone() {
                needed.push(row * config.x_blocks + col);
            }
        }

        self.visible_nodes.retain(|k, _| needed.contains(k));

        for &key in &needed {
            if !self.visible_nodes.contains_key(&key) {
                let col = key % config.x_blocks;
                let row = key / config.x_blocks;
                let bounds = Self::tile_logical_bounds(col, row, config);
                let node = self.node_pool.acquire(self.info.index, bounds, zoom, ori, crop);
                self.visible_nodes.insert(key, node);
            }
            if let Some(n) = self.visible_nodes.get_mut(&key) {
                if n.needs_decoding() && cache.get_page_image_by_key(&n.cache_key).is_none() {
                    let cb: DecodeCallbackRef = Arc::new(PageCallback {
                        state: Arc::clone(&state_arc),
                        page_idx: self.info.index,
                        node_key: Some(key),
                        cache_key: n.cache_key.clone(),
                    });
                    n.decode(self.width, self.height, &self.info, crop, decode_service, cb);
                }
            }
        }
    }

    fn visible_tile_ranges(&self, config: &TileConfig, viewport: &Rect) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let tile_w = self.width / config.x_blocks as f32;
        let tile_h = self.height / config.y_blocks as f32;
        let left = ((viewport.left - self.bounds.left) / tile_w).floor() as isize;
        let right = ((viewport.right - self.bounds.left) / tile_w).ceil() as isize;
        let top = ((viewport.top - self.bounds.top) / tile_h).floor() as isize;
        let bottom = ((viewport.bottom - self.bounds.top) / tile_h).ceil() as isize;
        let col_start = left.max(0) as usize;
        let col_end = (right as usize).min(config.x_blocks);
        let row_start = top.max(0) as usize;
        let row_end = (bottom as usize).min(config.y_blocks);
        (col_start..col_end, row_start..row_end)
    }

    fn tile_logical_bounds(col: usize, row: usize, config: &TileConfig) -> Rect {
        let x_blocks = config.x_blocks as f32;
        let y_blocks = config.y_blocks as f32;
        Rect::new(
            col as f32 / x_blocks,
            row as f32 / y_blocks,
            (col + 1) as f32 / x_blocks,
            (row + 1) as f32 / y_blocks,
        )
    }

    pub fn draw(&self, cx: &mut PaintCx, cache: &PageCache) {
        let bx = self.bounds.left as f64;
        let by = self.bounds.top as f64;
        let bw = self.width as f64;
        let bh = self.height as f64;

        // Draw thumbnail (background)
        let thumb_key = format!("thumb-{}-{}", self.info.index, self.crop);
        if let Some(img) = cache.get_thumbnail(&thumb_key) {
            //info!("draw: {}: {}: {}: {}", thumb_key, self.bounds.top, bh);
            draw_image(cx, &img, bx, by, bw, bh, &thumb_key);
        }

        // Draw visible nodes (tiles)
        /*for (_nk, node) in &self.visible_nodes {
            let bitmap_guard = node.bitmap.load();
            if let Some(bitmap) = bitmap_guard.as_ref() {
                let nx = bx + node.bounds.left as f64 * self.width as f64;
                let ny = by + node.bounds.top as f64 * self.height as f64;
                let nw = (node.bounds.right - node.bounds.left) as f64 * self.width as f64;
                let nh = (node.bounds.bottom - node.bounds.top) as f64 * self.height as f64;
                draw_image(cx, bitmap, nx, ny, nw, nh, &node.cache_key);
            }
        }*/

        // Draw links
        if self.links_loaded.load(Ordering::Acquire) {
            let links = self.links.lock().unwrap();
            for link in links.iter() {
                let lx = bx + link.bounds.left as f64 / self.info.width as f64 * self.width as f64;
                let ly = by + link.bounds.top as f64 / self.info.height as f64 * self.height as f64;
                let lw = (link.bounds.right - link.bounds.left) as f64 / self.info.width as f64 * self.width as f64;
                let lh = (link.bounds.bottom - link.bounds.top) as f64 / self.info.height as f64 * self.height as f64;
                let link_rect = KurboRect::from_origin_size((lx, ly), (lw, lh));
                cx.fill(&link_rect, Color::from_rgba8(0, 100, 255, 40), 0.0);
            }
        }
    }

    pub fn find_link_at(&self, doc_x: f32, doc_y: f32) -> Option<Link> {
        let page_x = doc_x - self.bounds.left;
        let page_y = doc_y - self.bounds.top;
        if page_x < 0.0 || page_y < 0.0 || page_x > self.width || page_y > self.height {
            return None;
        }
        let rel_x = page_x * self.info.width / self.width;
        let rel_y = page_y * self.info.height / self.height;
        self.links.lock().unwrap().iter()
            .find(|link| {
                rel_x >= link.bounds.left && rel_x <= link.bounds.right
                    && rel_y >= link.bounds.top && rel_y <= link.bounds.bottom
            })
            .cloned()
    }

    pub fn needs_decoding(&self) -> bool { !self.is_decoding }

    pub fn recycle(&mut self) {
        for (_, node) in self.visible_nodes.drain() {
            self.node_pool.release(node);
        }
        self.is_decoding = false;
    }

    pub fn clear_thumb(&mut self) {
        self.thumb_bitmap.store(None);
        self.is_thumb_loading.store(false, Ordering::Release);
    }

    pub fn node_pool(&self) -> &PageNodePool { &self.node_pool }
    pub fn node_pool_mut(&mut self) -> &mut PageNodePool { &mut self.node_pool }
}

fn draw_image(
    cx: &mut PaintCx,
    image_data: &ImageData,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cache_key: &str,
) {
    let image_brush = floem::peniko::ImageBrush::new(image_data.clone());
    let rect = KurboRect::from_origin_size((x, y), (w, h));

    cx.draw_img(
        floem::floem_renderer::Img {
            img: image_brush,
            hash: cache_key.as_bytes(),
        },
        rect,
    );
}

pub fn rects_intersect(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

pub struct TileConfig {
    pub x_blocks: usize,
    pub y_blocks: usize,
}

impl TileConfig {
    const MIN_BLOCK: f32 = 256.0;
    const MAX_BLOCK: f32 = 512.0;

    pub fn from_size(width: f32, height: f32) -> Self {
        if width <= Self::MAX_BLOCK && height <= Self::MAX_BLOCK {
            return Self { x_blocks: 1, y_blocks: 1 };
        }
        Self {
            x_blocks: Self::calc_axis_blocks(width),
            y_blocks: Self::calc_axis_blocks(height),
        }
    }

    pub fn is_single_block(&self) -> bool { self.x_blocks == 1 && self.y_blocks == 1 }

    fn calc_axis_blocks(length: f32) -> usize {
        if length <= 0.0 { return 1; }
        if length <= Self::MAX_BLOCK { return 1; }
        let mut blocks = (length / Self::MAX_BLOCK).ceil() as usize;
        let actual_block_size = length / blocks as f32;
        if actual_block_size < Self::MIN_BLOCK {
            blocks = (length / Self::MIN_BLOCK).ceil() as usize;
        }
        blocks
    }
}
