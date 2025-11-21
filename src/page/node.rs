use image::DynamicImage;
use std::sync::Arc;
use crate::decoder::Rect;

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
}

impl PageNode {
    pub fn new(page_index: usize, bounds: Rect) -> Self {
        let cache_key = format!(
            "{}_{:.2}_{:.2}_{:.2}_{:.2}",
            page_index,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom
        );
        
        Self {
            page_index,
            bounds,
            cache_key,
            bitmap: None,
            is_decoding: false,
        }
    }
    
    /// 将逻辑坐标转换为像素坐标
    pub fn to_pixel_rect(&self, page_width: f32, page_height: f32, x_offset: f32, y_offset: f32) -> Rect {
        Rect::new(
            self.bounds.left * page_width + x_offset,
            self.bounds.top * page_height + y_offset,
            self.bounds.right * page_width + x_offset,
            self.bounds.bottom * page_height + y_offset,
        )
    }
    
    /// 回收资源
    pub fn recycle(&mut self) {
        self.bitmap = None;
        self.is_decoding = false;
    }
}
