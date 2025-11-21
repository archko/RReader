use anyhow::Result;
use image::DynamicImage;

pub mod pdf;

/// 矩形区域
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
}

/// 页面信息
#[derive(Debug, Clone)]
pub struct PageInfo {
    pub index: usize,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub crop_bounds: Option<Rect>,
}

impl PageInfo {
    pub fn new(index: usize, width: f32, height: f32) -> Self {
        Self {
            index,
            width,
            height,
            scale: 1.0,
            crop_bounds: None,
        }
    }

    pub fn get_width(&self, use_crop: bool) -> f32 {
        if use_crop {
            if let Some(crop) = &self.crop_bounds {
                return crop.width();
            }
        }
        self.width
    }

    pub fn get_height(&self, use_crop: bool) -> f32 {
        if use_crop {
            if let Some(crop) = &self.crop_bounds {
                return crop.height();
            }
        }
        self.height
    }

    pub fn has_crop(&self) -> bool {
        self.crop_bounds.is_some()
    }
}

/// 链接类型
#[derive(Debug, Clone, PartialEq)]
pub enum LinkType {
    Page(usize), // 内部页面链接
    Url(String), // 外部URL链接
}

/// 链接
#[derive(Debug, Clone)]
pub struct Link {
    pub bounds: Rect,
    pub link_type: LinkType,
}

/// 文档解码器统一接口
/// 注意：由于某些底层库（如 mupdf）不是线程安全的，
/// 这个 trait 不要求 Send + Sync
pub trait DocumentDecoder {
    /// 获取文档页数
    fn page_count(&self) -> usize;

    /// 获取页面原始尺寸
    fn get_page_size(&self, index: usize) -> Result<(f32, f32)>;

    /// 获取所有页面信息
    fn get_all_pages(&self) -> Result<Vec<PageInfo>>;

    /// 渲染完整页面
    /// - page: 页面信息
    /// - crop: 是否使用切边
    fn render_page(&self, page: &PageInfo, crop: bool) -> Result<DynamicImage>;

    /// 渲染页面区域（用于分块渲染）
    /// - page_index: 页面索引
    /// - region: 要渲染的区域（PDF坐标系）
    /// - scale: 缩放比例
    fn render_region(&self, page_index: usize, region: Rect, scale: f32) -> Result<DynamicImage>;

    /// 获取页面链接
    fn get_page_links(&self, page_index: usize) -> Result<Vec<Link>>;

    /// 获取页面文本（用于搜索/TTS）
    fn get_page_text(&self, page_index: usize) -> Result<String>;

    /// 关闭文档
    fn close(&mut self);
}
