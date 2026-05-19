use std::collections::HashMap;
use vello::Scene;
use vello::peniko::{Brush, ImageBrush, ImageData, ImageFormat, Color};
use vello::kurbo::Rect as KurboRect;
use vello::Fill;
use vello::kurbo::Affine;
use std::sync::Arc;

use super::{PageNode, PageNodePool};
use crate::cache::PageCache;
use crate::decoder::{Link, PageInfo, Rect};

pub struct Page {
    pub info: PageInfo,
    /// 页面在文档坐标中的位置
    pub bounds: Rect,
    /// 当前可见的 tile node（不预创建，按需生成），key = row * x_blocks + col
    pub visible_nodes: HashMap<usize, PageNode>,
    pub links: Vec<Link>,
    pub width: f32,
    pub height: f32,
    pub is_decoding: bool,

    // ── 设计文档对齐字段 ──
    /// 低分辨率缩略图，保证快速首屏显示
    pub thumb_bitmap: Option<Arc<image::DynamicImage>>,
    pub is_thumb_loading: bool,
    /// 页面在文档中的偏移（design doc 中的 xOffset / yOffset）
    pub x_offset: f32,
    pub y_offset: f32,
    /// 整体缩放比例 totalScale = width / info.width
    pub total_scale: f32,
    /// 链接是否已加载（懒加载）
    pub links_loaded: bool,
    /// 瓦片分块配置，由 invalidate_nodes() 计算
    pub tile_config: TileConfig,
    /// PageNode 对象池（避免高频 GC）
    node_pool: PageNodePool,
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32) -> Self {
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
            links_loaded: false,
            tile_config,
            node_pool: PageNodePool::new(),
        }
    }

    pub fn update(&mut self, width: f32, height: f32, bounds: Rect) {
        self.width = width;
        self.height = height;
        self.bounds = bounds;
        self.x_offset = bounds.left;
        self.y_offset = bounds.top;
        self.total_scale = width / self.info.width;
        self.invalidate_nodes();
    }

    /// 重新计算瓦片分块配置（design doc 中的 invalidateNodes）
    pub fn invalidate_nodes(&mut self) {
        self.tile_config = TileConfig::from_size(self.width, self.height);
    }

    pub fn x_offset(&self) -> f32 { self.bounds.left }
    pub fn y_offset(&self) -> f32 { self.bounds.top }

    /// 绘制当前所有可见 node（缩略图作为底层，高清瓦片覆盖其上）
    pub fn draw(&self, scene: &mut Scene, scroll: Affine, cache: &PageCache) {
        // 1) 尝试从缓存获取缩略图，低分辨率优先显示
        let thumb_key = format!("thumb-{}", self.info.index);
        let thumb_img = self.thumb_bitmap.clone()
            .or_else(|| cache.get_thumbnail(&thumb_key));
        if let Some(ref img) = thumb_img {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let data: Arc<[u8]> = rgba.into_raw().into();
            let image_data = ImageData { data, format: ImageFormat::Rgba8, width: w, height: h };
            let brush: Brush = ImageBrush::new(image_data).into();
            let draw_rect = KurboRect::new(
                self.bounds.left as f64, self.bounds.top as f64,
                self.bounds.right as f64, self.bounds.bottom as f64,
            );
            scene.fill(Fill::NonZero, scroll, &brush, None, &draw_rect);
        }

        // 2) 高清瓦片
        for node in self.visible_nodes.values() {
            // 注意：这里需要可变引用来更新缓存，但 draw 签名是 &self
            // 实际使用时，像素矩形缓存是性能优化，不缓存也可以正常工作
            let pixel_rect = Rect::new(
                node.bounds.left * self.width + self.bounds.left,
                node.bounds.top * self.height + self.bounds.top,
                node.bounds.right * self.width + self.bounds.left,
                node.bounds.bottom * self.height + self.bounds.top,
            );
            let draw_rect = KurboRect::new(
                pixel_rect.left as f64, pixel_rect.top as f64,
                pixel_rect.right as f64, pixel_rect.bottom as f64,
            );
            if let Some(img_arc) = cache.get_page_image_by_key(&node.cache_key) {
                let rgba = img_arc.to_rgba8();
                let (w, h) = rgba.dimensions();
                let data: Arc<[u8]> = rgba.into_raw().into();
                let image_data = ImageData { data, format: ImageFormat::Rgba8, width: w, height: h };
                let brush: Brush = ImageBrush::new(image_data).into();
                scene.fill(Fill::NonZero, scroll, &brush, None, &draw_rect);
            }
        }
    }

    /// 绘制链接高亮区域（设计文档 drawLinks）
    pub fn draw_links(&self, scene: &mut Scene, scroll: Affine) {
        if self.links.is_empty() {
            return;
        }

        // 链接区域高亮颜色：半透明蓝色边框
        let link_color = Color::rgba(0.0, 0.5, 1.0, 0.3);
        let border_color = Color::rgba(0.0, 0.5, 1.0, 0.8);

        for link in &self.links {
            // 将链接边界从页面对齐坐标转换为文档坐标
            let link_rect = KurboRect::new(
                (self.bounds.left + link.bounds.left * self.width) as f64,
                (self.bounds.top + link.bounds.top * self.height) as f64,
                (self.bounds.left + link.bounds.right * self.width) as f64,
                (self.bounds.top + link.bounds.bottom * self.height) as f64,
            );

            // 填充半透明背景
            scene.fill(Fill::NonZero, scroll, &link_color, None, &link_rect);
            
            // 绘制边框（使用 stroke 需要引入 Stroke 类，这里用细矩形模拟）
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

    /// 仅管理瓦片 node 的创建/回收，返回需要解码的 node key 列表。
    /// 不访问 cache，不下发 decode 任务（纯 node 生命周期管理）。
    pub fn update_visible_nodes(&mut self, viewport: &Rect) -> Vec<usize> {
        let config = &self.tile_config;
        let (col_range, row_range) = self.visible_tile_ranges(config, viewport);

        let mut needed: Vec<usize> = Vec::new();
        for row in row_range.clone() {
            for col in col_range.clone() {
                needed.push(row * config.x_blocks + col);
            }
        }

        // 移除不再需要的 node
        self.visible_nodes.retain(|k, _| needed.contains(k));

        // 按需从池中获取或创建 node
        let mut decode_needed = Vec::new();
        for &key in &needed {
            if !self.visible_nodes.contains_key(&key) {
                let col = key % config.x_blocks;
                let row = key / config.x_blocks;
                let bounds = Self::tile_logical_bounds(col, row, config);
                let node = self.node_pool.acquire(self.info.index, bounds);
                self.visible_nodes.insert(key, node);
            }
            if let Some(n) = self.visible_nodes.get(&key) {
                if n.needs_decoding() {
                    decode_needed.push(key);
                }
            }
        }

        decode_needed
    }

    /// 计算视口覆盖的 tile 行列范围 [col_start..col_end, row_start..row_end)
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

    /// 根据行列计算 tile 的逻辑边界 [0,1]
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

    // ── 懒加载链接 ──

    pub fn load_links(&mut self) {
        if !self.links_loaded {
            self.links_loaded = true;
            // 链接数据已在解码结果中返回，只需标记加载
        }
    }

    pub fn find_link_at(&self, x: f32, y: f32) -> Option<&Link> {
        let page_x = x - self.bounds.left;
        let page_y = y - self.bounds.top;
        self.links.iter().find(|link| {
            page_x >= link.bounds.left && page_x <= link.bounds.right
                && page_y >= link.bounds.top && page_y <= link.bounds.bottom
        })
    }

    pub fn needs_decoding(&self) -> bool { !self.is_decoding }

    /// 回收所有 node 到对象池（保留 thumb_bitmap 以快速恢复显示）
    pub fn recycle(&mut self) {
        for (_, node) in self.visible_nodes.drain() {
            self.node_pool.release(node);
        }
        self.is_decoding = false;
    }
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
