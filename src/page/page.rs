use super::PageNode;
use crate::decoder::{Link, PageInfo, Rect};

pub struct Page {
    /// 页面信息
    pub info: PageInfo,

    /// 页面在文档中的绝对边界（文档坐标）
    pub bounds: Rect,

    /// 渲染块列表（逻辑坐标 0.0~1.0）
    pub nodes: Vec<PageNode>,

    /// 页面链接（文档坐标）
    pub links: Vec<Link>,

    /// 缩放后的宽度
    pub width: f32,

    /// 缩放后的高度
    pub height: f32,
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32) -> Self {
        let bounds = Rect::new(x_offset, y_offset, x_offset + width, y_offset + height);

        let mut page = Self {
            info,
            bounds,
            nodes: Vec::new(),
            links: Vec::new(),
            width,
            height,
        };
        page.invalidate_nodes();
        page
    }

    /// 更新页面尺寸和位置
    pub fn update(&mut self, width: f32, height: f32, bounds: Rect) {
        self.width = width;
        self.height = height;
        self.bounds = bounds;
        //self.invalidate_nodes();
    }

    /// 获取 X 偏移
    pub fn x_offset(&self) -> f32 {
        self.bounds.left
    }

    /// 获取 Y 偏移
    pub fn y_offset(&self) -> f32 {
        self.bounds.top
    }

    /// 回收所有节点资源
    pub fn recycle(&mut self) {
        for node in &mut self.nodes {
            node.recycle();
        }
        self.nodes.clear();
    }

    /// 查找指定坐标的链接
    pub fn find_link_at(&self, x: f32, y: f32) -> Option<&Link> {
        // 将文档坐标转换为页面坐标
        let page_x = x - self.bounds.left;
        let page_y = y - self.bounds.top;

        self.links.iter().find(|link| {
            page_x >= link.bounds.left
                && page_x <= link.bounds.right
                && page_y >= link.bounds.top
                && page_y <= link.bounds.bottom
        })
    }
}

impl Page {
    /// 重新计算页面的分块配置（PageNode 列表）
    fn invalidate_nodes(&mut self) {
        // 页面尚未有有效尺寸时不分块
        if self.width <= 0.0 || self.height <= 0.0 {
            self.nodes.clear();
            return;
        }

        let config = TileConfig::from_size(self.width, self.height);

        // 如果是单个块，直接整页一个 node
        if config.is_single_block() {
            self.nodes = vec![PageNode::new(
                self.info.index,
                Rect::new(0.0, 0.0, 1.0, 1.0),
            )];
            return;
        }

        let mut nodes = Vec::new();
        for y in 0..config.y_blocks {
            for x in 0..config.x_blocks {
                let base_left = x as f32 / config.x_blocks as f32;
                let base_top = y as f32 / config.y_blocks as f32;
                let base_right = (x + 1) as f32 / config.x_blocks as f32;
                let base_bottom = (y + 1) as f32 / config.y_blocks as f32;

                // 轻微重叠，避免边缘出现缝隙
                let overlap = 0.001_f32;
                let left = if x == 0 {
                    base_left
                } else {
                    base_left - overlap
                };
                let top = if y == 0 { base_top } else { base_top - overlap };
                let right = if x == config.x_blocks - 1 {
                    base_right
                } else {
                    base_right + overlap
                };
                let bottom = if y == config.y_blocks - 1 {
                    base_bottom
                } else {
                    base_bottom + overlap
                };

                nodes.push(PageNode::new(
                    self.info.index,
                    Rect::new(left, top, right, bottom),
                ));
            }
        }

        self.nodes = nodes;
    }
}

/// 分块配置（参考 Kotlin 版 TileConfig）
struct TileConfig {
    x_blocks: usize,
    y_blocks: usize,
}

impl TileConfig {
    const MIN_BLOCK_SIZE: f32 = 256.0 * 3.0; // 约 768 像素
    const MAX_BLOCK_SIZE: f32 = 256.0 * 4.0; // 约 1024 像素

    fn is_single_block(&self) -> bool {
        self.x_blocks == 1 && self.y_blocks == 1
    }

    fn from_size(width: f32, height: f32) -> Self {
        // 小页面直接整页渲染
        if width <= Self::MAX_BLOCK_SIZE && height <= Self::MAX_BLOCK_SIZE {
            return Self {
                x_blocks: 1,
                y_blocks: 1,
            };
        }

        let x_blocks = Self::calc_block_count(width);
        let y_blocks = Self::calc_block_count(height);
        Self { x_blocks, y_blocks }
    }

    fn calc_block_count(length: f32) -> usize {
        if length <= Self::MIN_BLOCK_SIZE {
            return 1;
        }

        let mut block_count = (length / Self::MAX_BLOCK_SIZE).ceil() as usize;
        if block_count == 0 {
            block_count = 1;
        }
        let actual_block_size = length / block_count as f32;

        if (Self::MIN_BLOCK_SIZE..=Self::MAX_BLOCK_SIZE).contains(&actual_block_size) {
            block_count
        } else {
            (length / Self::MIN_BLOCK_SIZE).ceil() as usize
        }
    }
}
