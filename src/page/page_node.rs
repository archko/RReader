use crate::decoder::Rect;
use image::DynamicImage;
use std::sync::Arc;

/// 页面渲染块（Tile）
/// 用于大页面的分块渲染
pub struct PageNode {
    /// 页面索引
    pub page_index: usize,

    /// 逻辑边界（0.0~1.0 相对坐标）
    pub bounds: Rect,

    /// 缓存键
    pub cache_key: String,

    /// 渲染的图像（可选，按需加载）
    pub bitmap: Option<Arc<DynamicImage>>,

    /// 是否正在解码
    pub is_decoding: bool,

    /// 像素矩形缓存（避免重复计算）
    cached_pixel_rect: Option<Rect>,
    /// 上次计算像素矩形时的参数
    cached_page_size: Option<(f32, f32, f32, f32)>, // (page_width, page_height, x_offset, y_offset)
}

impl PageNode {
    pub fn new(page_index: usize, bounds: Rect) -> Self {
        let cache_key = Self::generate_cache_key(page_index, &bounds);

        Self {
            page_index,
            bounds,
            cache_key,
            bitmap: None,
            is_decoding: false,
            cached_pixel_rect: None,
            cached_page_size: None,
        }
    }

    /// 生成缓存键
    fn generate_cache_key(page_index: usize, bounds: &Rect) -> String {
        format!(
            "{}_{:.2}_{:.2}_{:.2}_{:.2}",
            page_index, bounds.left, bounds.top, bounds.right, bounds.bottom
        )
    }

    /// 更新 PageNode 数据（用于对象池复用）
    pub fn update(&mut self, page_index: usize, bounds: Rect) {
        self.page_index = page_index;
        self.bounds = bounds;
        self.cache_key = Self::generate_cache_key(page_index, &self.bounds);
        self.bitmap = None;
        self.is_decoding = false;
        self.cached_pixel_rect = None;
        self.cached_page_size = None;
    }

    /// 将逻辑坐标转换为像素坐标（带缓存）
    pub fn to_pixel_rect(
        &mut self,
        page_width: f32,
        page_height: f32,
        x_offset: f32,
        y_offset: f32,
    ) -> Rect {
        // 检查缓存是否有效
        if let Some((cw, ch, cx, cy)) = self.cached_page_size {
            if (cw - page_width).abs() < 0.1
                && (ch - page_height).abs() < 0.1
                && (cx - x_offset).abs() < 0.1
                && (cy - y_offset).abs() < 0.1
            {
                return self.cached_pixel_rect.unwrap();
            }
        }

        // 重新计算
        let rect = Rect::new(
            self.bounds.left * page_width + x_offset,
            self.bounds.top * page_height + y_offset,
            self.bounds.right * page_width + x_offset,
            self.bounds.bottom * page_height + y_offset,
        );

        // 更新缓存
        self.cached_pixel_rect = Some(rect);
        self.cached_page_size = Some((page_width, page_height, x_offset, y_offset));

        rect
    }
    
    // 检查是否需要解码
    pub fn needs_decoding(&self) -> bool {
        self.bitmap.is_none() && !self.is_decoding
    }

    /// 回收资源
    pub fn recycle(&mut self) {
        self.bitmap = None;
        self.is_decoding = false;
        self.cached_pixel_rect = None;
        self.cached_page_size = None;
    }
}
