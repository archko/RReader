use std::collections::HashMap;
use vello::Scene;
use vello::peniko::{Brush, ImageBrush, ImageData, ImageFormat, Color};
use vello::kurbo::Rect as KurboRect;
use vello::Fill;
use vello::kurbo::Affine;
use std::sync::Arc;

use super::{PageNode, PageNodePool, Orientation, PageRenderState};
use crate::cache::PageCache;
use crate::decoder::{Link, PageInfo, Rect};
use crate::decoder::decode_service::{DecodeService, VisibilityChecker};

pub struct Page {
    pub info: PageInfo,
    /// 页面在文档坐标中的位置
    pub bounds: Rect,
    /// 当前可见的 tile node, key = row * x_blocks + col
    pub visible_nodes: HashMap<usize, PageNode>,
    pub links: Vec<Link>,
    pub width: f32,
    pub height: f32,
    pub is_decoding: bool,

    /// 低分辨率缩略图
    pub thumb_bitmap: Option<Arc<image::DynamicImage>>,
    pub is_thumb_loading: bool,
    pub x_offset: f32,
    pub y_offset: f32,
    pub total_scale: f32,
    /// 上次 layout 时的 zoom 值（用于 scaleRatio 修正）
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

    /// 重新计算瓦片分块配置
    pub fn invalidate_nodes(&mut self) {
        self.tile_config = TileConfig::from_size(self.width, self.height);
    }

    pub fn x_offset(&self) -> f32 { self.bounds.left }
    pub fn y_offset(&self) -> f32 { self.bounds.top }

    pub fn draw(&self, scene: &mut Scene, scroll: Affine, cache: &PageCache,
                current_zoom: f32, crop: i32,
                vis_left: f32, vis_top: f32, vis_right: f32, vis_bottom: f32) {
        let scale_ratio = if self.base_zoom > 0.0 { current_zoom / self.base_zoom } else { 1.0 };

        let adj_left = self.bounds.left * scale_ratio;
        let adj_top = self.bounds.top * scale_ratio;
        let adj_right = self.bounds.right * scale_ratio;
        let adj_bottom = self.bounds.bottom * scale_ratio;

        let adj_width = self.width * scale_ratio;
        let adj_height = self.height * scale_ratio;

        let thumb_key = format!("thumb-{}-{}", self.info.index, crop);
        let thumb_img = self.thumb_bitmap.clone()
            .or_else(|| cache.get_thumbnail(&thumb_key));
        if let Some(ref img) = thumb_img {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let data: Arc<[u8]> = rgba.into_raw().into();
            let image_data = ImageData { data, format: ImageFormat::Rgba8, width: w, height: h };
            let brush: Brush = ImageBrush::new(image_data).into();
            let draw_rect = KurboRect::new(
                adj_left as f64, adj_top as f64,
                adj_right as f64, adj_bottom as f64,
            );
            scene.fill(Fill::NonZero, scroll, &brush, None, &draw_rect);
        }

        for node in self.visible_nodes.values() {
            node.draw(scene, scroll, adj_width, adj_height, adj_left, adj_top, cache,
                      vis_left, vis_top, vis_right, vis_bottom);
        }
    }

    /// 绘制链接高亮区域
    pub fn draw_links(&self, scene: &mut Scene, scroll: Affine, scale_ratio: f32) {
        if self.links.is_empty() {
            return;
        }
        let adj_left = self.bounds.left * scale_ratio;
        let adj_top = self.bounds.top * scale_ratio;
        let adj_width = self.width * scale_ratio;
        let adj_height = self.height * scale_ratio;

        let link_color = Color::rgba(0.0, 0.5, 1.0, 0.3);
        let border_color = Color::rgba(0.0, 0.5, 1.0, 0.8);

        for link in &self.links {
            let link_rect = KurboRect::new(
                (adj_left + link.bounds.left * adj_width / self.info.width) as f64,
                (adj_top + link.bounds.top * adj_height / self.info.height) as f64,
                (adj_left + link.bounds.right * adj_width / self.info.width) as f64,
                (adj_top + link.bounds.bottom * adj_height / self.info.height) as f64,
            );
            scene.fill(Fill::NonZero, scroll, &link_color, None, &link_rect);
            let stroke_width = 1.0;
            let stroke_rect = KurboRect::new(
                link_rect.x0 - stroke_width / 2.0,
                link_rect.y0 - stroke_width / 2.0,
                link_rect.x1 + stroke_width / 2.0,
                link_rect.y1 + stroke_width / 2.0,
            );
            scene.fill(Fill::NonZero, scroll, &border_color, None, &stroke_rect);
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

        // 单块优化
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
                    let checker: Option<VisibilityChecker> = Some(Arc::new({
                        let sa = Arc::clone(&state_arc);
                        move |page_idx| sa.read().visible_pages.contains(&page_idx)
                    }));
                    n.decode(self.width, self.height, &self.info, crop, decode_service, checker);
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
                    let tile_key = n.cache_key.clone();
                    let checker: Option<VisibilityChecker> = Some(Arc::new({
                        let sa = Arc::clone(&state_arc);
                        move |page_idx| {
                            sa.read().pages.get(page_idx)
                                .map(|p| p.visible_nodes.values().any(|n| n.cache_key == tile_key))
                                .unwrap_or(false)
                        }
                    }));
                    n.decode(self.width, self.height, &self.info, crop, decode_service, checker);
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

    pub fn load_links(&mut self) {
        if !self.links_loaded {
            self.links_loaded = true;
        }
    }

    /// 点击检测链接
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

// ── TileConfig ─────────────────────────────────

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
