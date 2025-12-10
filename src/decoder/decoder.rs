use crate::{decoder::{Link, PageInfo, Rect}, entity::OutlineItem};
use crate::entity::ReflowEntry;
use std::path::{Path};

/// 文档解码器统一接口
/// 注意：由于某些底层库（如 mupdf）不是线程安全的，
/// 这个 trait 不要求 Send + Sync
pub trait Decoder {
    /// 获取文档页数
    fn page_count(&self) -> usize;

    /// 获取页面原始尺寸
    fn get_page_size(&self, index: usize) -> anyhow::Result<(f32, f32)>;

    /// 获取所有页面信息
    fn get_all_pages(&self) -> anyhow::Result<Vec<PageInfo>>;

    /// 渲染完整页面，返回原始RGBA像素数据
    /// - page: 页面信息
    /// - crop: 是否使用切边
    fn render_page(&self, page: &PageInfo, crop: bool) -> anyhow::Result<(Vec<u8>, u32, u32)>;

    /// 渲染页面区域（用于分块渲染），返回原始RGBA像素数据
    /// - page_index: 页面索引
    /// - region: 要渲染的区域（PDF坐标系）
    /// - scale: 缩放比例
    fn render_region(
        &self,
        page_index: usize,
        region: Rect,
        scale: f32,
    ) -> anyhow::Result<(Vec<u8>, u32, u32)>;

    /// 获取页面链接
    fn get_page_links(&self, page_index: usize) -> anyhow::Result<Vec<Link>>;

    /// 获取页面文本（用于搜索/TTS）
    fn get_page_text(&self, page_index: usize) -> anyhow::Result<String>;

    fn get_outline_items(&self) -> anyhow::Result<Vec<OutlineItem>>;

    /// 从指定页面开始获取后续页面的reflow数据
    /// - start_page: 起始页面索引
    fn get_reflow_from_page(&self, start_page: usize) -> anyhow::Result<Vec<ReflowEntry>>;

    /// 关闭文档
    fn close(&mut self);
}
