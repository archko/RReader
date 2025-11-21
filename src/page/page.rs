use crate::decoder::{PageInfo, Rect, Link};
use super::PageNode;

/// 单个页面
pub struct Page {
    /// 页面信息
    pub info: PageInfo,
    
    /// 页面在文档中的绝对边界
    pub bounds: Rect,
    
    /// 渲染块列表
    pub nodes: Vec<PageNode>,
    
    /// 页面链接
    pub links: Vec<Link>,
    
    /// 缩放后的宽度
    pub width: f32,
    
    /// 缩放后的高度
    pub height: f32,
}

impl Page {
    pub fn new(info: PageInfo, width: f32, height: f32, x_offset: f32, y_offset: f32) -> Self {
        let bounds = Rect::new(
            x_offset,
            y_offset,
            x_offset + width,
            y_offset + height,
        );
        
        // 默认创建一个完整的 node（不分块）
        let node = PageNode::new(info.index, Rect::new(0.0, 0.0, 1.0, 1.0));
        
        Self {
            info,
            bounds,
            nodes: vec![node],
            links: Vec::new(),
            width,
            height,
        }
    }
    
    /// 更新页面尺寸和位置
    pub fn update(&mut self, width: f32, height: f32, bounds: Rect) {
        self.width = width;
        self.height = height;
        self.bounds = bounds;
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
