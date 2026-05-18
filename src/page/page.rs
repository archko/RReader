use std::collections::HashMap;
use vello::Scene;
use vello::peniko::{Brush, ImageBrush, ImageData, ImageFormat, Color};
use vello::kurbo::Rect as KurboRect;
use vello::Fill;
use vello::kurbo::Affine;
use std::sync::Arc;

use super::PageNode;
use crate::cache::PageCache;
use crate::decoder::decode_service::{RenderPage, Priority};
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
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32) -> Self {
        let bounds = Rect::new(x_offset, y_offset, x_offset + width, y_offset + height);
        Self {
            info,
            bounds,
            visible_nodes: HashMap::new(),
            links: Vec::new(),
            width,
            height,
            is_decoding: false,
        }
    }

    pub fn update(&mut self, width: f32, height: f32, bounds: Rect) {
        self.width = width;
        self.height = height;
        self.bounds = bounds;
    }

    pub fn x_offset(&self) -> f32 { self.bounds.left }
    pub fn y_offset(&self) -> f32 { self.bounds.top }

    /// 绘制当前所有可见 node
    pub fn draw(&self, scene: &mut Scene, scroll: Affine, cache: &PageCache) {
        for node in self.visible_nodes.values() {
            let pixel_rect = node.to_pixel_rect(
                self.width, self.height, self.bounds.left, self.bounds.top,
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

    /// 根据文档坐标系的视口，计算当前哪些 tile 应可见。
    /// - 已存在 node 保留；
    /// - 新可见的创建 node，缺图则提交解码；
    /// - 移出视口的 node 移除。
    pub fn update_visible_nodes(&mut self, viewport: &Rect, cache: &PageCache, crop: i32) -> Vec<RenderPage> {
        let config = TileConfig::from_size(self.width, self.height);
        let mut tasks = Vec::new();

        // 1) 计算视口覆盖的 tile 行列范围
        let (col_range, row_range) = self.visible_tile_ranges(&config, viewport);

        // 2) 收集应在可见集内的 key 集合
        let mut needed: Vec<usize> = Vec::new();
        for row in row_range.clone() {
            for col in col_range.clone() {
                needed.push(row * config.x_blocks + col);
            }
        }

        // 3) 移除不在 needed 中的过时 node
        self.visible_nodes.retain(|k, _| needed.contains(k));

        // 4) 为所需但缺失的 tile 创建 node
        for &key in &needed {
            if !self.visible_nodes.contains_key(&key) {
                let col = key % config.x_blocks;
                let row = key / config.x_blocks;
                let bounds = Self::tile_logical_bounds(col, row, &config);
                let node = PageNode::new(self.info.index, bounds);
                self.visible_nodes.insert(key, node);

                // 缺图 → 提交解码
                if let Some(n) = self.visible_nodes.get(&key) {
                    if cache.get_page_image_by_key(&n.cache_key).is_none() {
                        tasks.push(RenderPage {
                            key: n.cache_key.clone(),
                            page_info: self.info.clone(),
                            crop,
                            priority: Priority::Thumbnail,
                            visibility_checker: None,
                        });
                    }
                }
            }
        }

        tasks
    }

    /// 计算视口覆盖的 tile 行列范围 [col_start..col_end, row_start..row_end)
    fn visible_tile_ranges(&self, config: &TileConfig, viewport: &Rect) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        // tile 在文档坐标中的像素尺寸
        let tile_w = self.width / config.x_blocks as f32;
        let tile_h = self.height / config.y_blocks as f32;

        // 将视口边界从文档坐标转为 tile 索引
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
        let overlap = 0.001_f32;

        let base_left = col as f32 / x_blocks;
        let base_top = row as f32 / y_blocks;
        let base_right = (col + 1) as f32 / x_blocks;
        let base_bottom = (row + 1) as f32 / y_blocks;

        Rect::new(
            if col == 0 { base_left } else { base_left - overlap },
            if row == 0 { base_top } else { base_top - overlap },
            if col == config.x_blocks - 1 { base_right } else { base_right + overlap },
            if row == config.y_blocks - 1 { base_bottom } else { base_bottom + overlap },
        )
    }

    // ── 链接查找 ──────────────────────────────

    pub fn find_link_at(&self, x: f32, y: f32) -> Option<&Link> {
        let page_x = x - self.bounds.left;
        let page_y = y - self.bounds.top;
        self.links.iter().find(|link| {
            page_x >= link.bounds.left && page_x <= link.bounds.right
                && page_y >= link.bounds.top && page_y <= link.bounds.bottom
        })
    }

    pub fn needs_decoding(&self) -> bool { !self.is_decoding }

    /// 回收所有 node 资源
    pub fn recycle(&mut self) {
        for node in self.visible_nodes.values_mut() {
            node.recycle();
        }
        self.visible_nodes.clear();
        self.is_decoding = false;
    }
}

/// 两个 Rect 是否相交
pub fn rects_intersect(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

// ── TileConfig ─────────────────────────────────

pub struct TileConfig {
    pub x_blocks: usize,
    pub y_blocks: usize,
}

impl TileConfig {
    const MIN_BLOCK_SIZE: f32 = 256.0 * 2.0;
    const MAX_BLOCK_SIZE: f32 = 256.0 * 3.0;

    pub fn from_size(width: f32, height: f32) -> Self {
        if width <= Self::MAX_BLOCK_SIZE && height <= Self::MAX_BLOCK_SIZE {
            return Self { x_blocks: 1, y_blocks: 1 };
        }
        Self {
            x_blocks: Self::calc_block_count(width),
            y_blocks: Self::calc_block_count(height),
        }
    }

    fn is_single_block(&self) -> bool { self.x_blocks == 1 && self.y_blocks == 1 }
    fn calc_block_count(length: f32) -> usize {
        if length <= Self::MIN_BLOCK_SIZE { return 1; }
        let mut bc = (length / Self::MAX_BLOCK_SIZE).ceil() as usize;
        if bc == 0 { bc = 1; }
        bc
    }
}
