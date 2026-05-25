use std::collections::HashMap;
use log::info;
use xilem::masonry::imaging::Painter;
use xilem::masonry::peniko::{ImageData, Color, Fill};
use xilem::masonry::kurbo::Affine;
use xilem::masonry::kurbo::Rect as KurboRect;
use std::sync::Arc;

use super::{PageNode, PageNodePool, Orientation, PageRenderState};
use crate::cache::PageCache;
use crate::decoder::{Link, PageInfo, Rect};
use crate::decoder::decode_service::{DecodeService, DecodeCallbackRef};
use super::render_state::PageCallback;

pub struct Page {
    pub info: PageInfo,
    pub bounds: Rect,
    pub visible_nodes: HashMap<usize, PageNode>,
    pub links: Vec<Link>,
    pub width: f32,
    pub height: f32,
    pub is_decoding: bool,

    pub thumb_bitmap: Option<Arc<ImageData>>,
    pub is_thumb_loading: bool,
    pub x_offset: f32,
    pub y_offset: f32,
    pub total_scale: f32,
    pub base_zoom: f32,
    pub links_loaded: bool,
    pub tile_config: TileConfig,
    node_pool: PageNodePool,
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32, base_zoom: f32) -> Self {
        let bounds = Rect::new(x_offset, y_offset, x_offset + width, y_offset + height);
        let tile_config = TileConfig::from_size(width, height);
        let total_scale = if width > 0.0 { width / info.width } else { 1.0 };
        Self {
            info,
            bounds,
            visible_nodes: HashMap::new(),
            links: Vec::new(),
            width,
            height,
            is_decoding: false,
            thumb_bitmap: None,
            is_thumb_loading: false,
            x_offset,
            y_offset,
            total_scale,
            base_zoom,
            links_loaded: false,
            tile_config,
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

    /// 绘制页面
    /// - scroll_x, scroll_y: 视口在大画布中的偏移（offset），正值
    /// - 绘制坐标 = 页面 bounds - offset
    pub fn draw(&self, painter: &mut Painter<'_>, scroll_x: f32, scroll_y: f32, 
                cache: &PageCache, current_zoom: f32, crop: i32,
                vis_left: f32, vis_top: f32, vis_right: f32, vis_bottom: f32) {
        // 计算当前缩放下的实际显示尺寸和位置
        // Page 的属性是基于 base_zoom 计算的，但当前的 zoom 可能已经改变
        let scale_ratio = if self.base_zoom > 0.0 { current_zoom / self.base_zoom } else { 1.0 };

        // 计算当前 bounds（参考 kreader: currentBounds = bounds * scaleRatio）
        let current_left = self.bounds.left * scale_ratio;
        let current_top = self.bounds.top * scale_ratio;
        let current_right = self.bounds.right * scale_ratio;
        let current_bottom = self.bounds.bottom * scale_ratio;
        let current_width = self.width * scale_ratio;
        let current_height = self.height * scale_ratio;

        // 检查页面是否真正可见（参考 kreader 的 overlaps 检查）
        let is_actually_visible = current_left < vis_right
            && current_right > vis_left
            && current_top < vis_bottom
            && current_bottom > vis_top;

        if !is_actually_visible {
            return;
        }

        let thumb_key = format!("thumb-{}-{}", self.info.index, crop);
        let thumb_img = self.thumb_bitmap.clone()
            .or_else(|| cache.get_thumbnail(&thumb_key));
        if let Some(ref img) = thumb_img {
            let draw_left = current_left - scroll_x;
            let draw_top = current_top - scroll_y;

            let transform = Affine::translate((draw_left as f64, draw_top as f64))
                * Affine::scale_non_uniform(
                    current_width as f64 / img.width as f64,
                    current_height as f64 / img.height as f64,
                );
            painter.draw_image(&**img, transform);
        }

        /*for node in self.visible_nodes.values() {
            node.draw(painter, scroll_x, scroll_y, current_width, current_height, current_left, current_top, cache,
                      vis_left, vis_top, vis_right, vis_bottom);
        }*/
    }

    pub fn draw_links(&self, painter: &mut Painter<'_>, scroll_x: f32, scroll_y: f32, scale_ratio: f32) {
        if self.links.is_empty() { return; }

        let current_left = self.bounds.left * scale_ratio;
        let current_top = self.bounds.top * scale_ratio;
        let current_width = self.width * scale_ratio;
        let current_height = self.height * scale_ratio;

        let link_color = Color::new([0.0, 0.5, 1.0, 0.3]);
        let border_color = Color::new([0.0, 0.5, 1.0, 0.8]);

        for link in &self.links {
            let link_rect = KurboRect::new(
                (current_left + link.bounds.left * current_width / self.info.width - scroll_x) as f64,
                (current_top + link.bounds.top * current_height / self.info.height - scroll_y) as f64,
                (current_left + link.bounds.right * current_width / self.info.width - scroll_x) as f64,
                (current_top + link.bounds.bottom * current_height / self.info.height - scroll_y) as f64,
            );
            painter.fill(link_rect, link_color).draw();
            let stroke_width = 1.0;
            let stroke_rect = KurboRect::new(
                link_rect.x0 - stroke_width / 2.0,
                link_rect.y0 - stroke_width / 2.0,
                link_rect.x1 + stroke_width / 2.0,
                link_rect.y1 + stroke_width / 2.0,
            );
            painter.fill(stroke_rect, border_color).draw();
        }
    }

    pub fn update_visible_nodes(
        &mut self,
        viewport: &Rect,
        decode_service: &DecodeService,
        cache: &PageCache,
        crop: i32,
        zoom: f32,
        orientation: Orientation,
        state_arc: Arc<PageRenderState>,
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

    pub fn find_link_at(&self, doc_x: f32, doc_y: f32) -> Option<&Link> {
        let page_x = doc_x - self.bounds.left;
        let page_y = doc_y - self.bounds.top;
        if page_x < 0.0 || page_y < 0.0 || page_x > self.width || page_y > self.height {
            return None;
        }
        let rel_x = page_x * self.info.width / self.width;
        let rel_y = page_y * self.info.height / self.height;
        self.links.iter().find(|link| {
            rel_x >= link.bounds.left && rel_x <= link.bounds.right
                && rel_y >= link.bounds.top && rel_y <= link.bounds.bottom
        })
    }

    pub fn needs_decoding(&self) -> bool { !self.is_decoding }

    pub fn recycle(&mut self) {
        for (_, node) in self.visible_nodes.drain() {
            self.node_pool.release(node);
        }
        self.is_decoding = false;
    }

    pub fn clear_thumb(&mut self) {
        self.thumb_bitmap = None;
        self.is_thumb_loading = false;
    }

    pub fn node_pool(&self) -> &PageNodePool { &self.node_pool }
    pub fn node_pool_mut(&mut self) -> &mut PageNodePool { &mut self.node_pool }
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
