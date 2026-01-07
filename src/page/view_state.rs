use log::{debug, info};

use super::Page;
use crate::cache::PageCache;
use crate::decoder::decode_service::{Priority, RenderPage, VisibilityChecker};
use crate::decoder::pdf::utils::{generate_thumbnail_key};
use crate::decoder::{DecodeService, Link, Rect};
use crate::entity::OutlineItem;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// 滚动方向
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// 页面视图状态管理
pub struct PageViewState {
    /// 页面缓存
    pub cache: Rc<PageCache>,

    /// 所有页面
    pub pages: Vec<Page>,

    /// 解码服务
    pub decode_service: Arc<DecodeService>,

    /// 滚动方向
    pub orientation: Orientation,

    /// 视图偏移 (x, y)
    pub view_offset: (f32, f32),

    /// 缩放比例
    pub zoom: f32,

    /// 是否启用切边
    pub crop: i32,

    /// 文档总宽度
    pub total_width: f32,

    /// 文档总高度
    pub total_height: f32,

    /// 视图尺寸 (width, height)
    pub view_size: (f32, f32),

    /// 预加载距离（屏幕数）
    pub preload_screens: f32,

    /// 当前可见页面索引列表
    pub visible_pages: Vec<usize>,

    /// 页面链接缓存，以页码为键存储链接列表
    pub page_links: Rc<RefCell<HashMap<usize, Vec<Link>>>>,

    pub outline_items: Vec<OutlineItem>,

    /// 可见区域（用于跨线程可见性检查）
    visible_rect: Arc<Mutex<Rect>>,

    /// 页面bounds映射（用于跨线程可见性检查）
    page_bounds_map: Arc<Mutex<HashMap<usize, Rect>>>,
}

impl PageViewState {
    pub fn new(orientation: Orientation, crop_int: i32) -> Self {
        Self {
            cache: Rc::new(PageCache::new(24, 10)),
            pages: Vec::new(),
            decode_service: Arc::new(DecodeService::new()),
            orientation,
            view_offset: (0.0, 0.0),
            zoom: 1.0,
            crop: crop_int,
            total_width: 0.0,
            total_height: 0.0,
            view_size: (0.0, 0.0),
            preload_screens: 1.0,
            visible_pages: Vec::new(),
            page_links: Rc::new(RefCell::new(HashMap::new())),
            outline_items: Vec::new(),
            visible_rect: Arc::new(Mutex::new(Rect::new(0.0, 0.0, 0.0, 0.0))),
            page_bounds_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 打开文档
    pub fn open_document<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        Self::reset(self);
        self.decode_service.load_pdf(path)?;
        Ok(())
    }

    pub fn set_pages_from_info(&mut self, pages_info: Vec<crate::decoder::PageInfo>) {
        let pages: Vec<Page> = pages_info
            .into_iter()
            .map(|info| Page::new(info, 0.0, 0.0, 0.0, 0.0))
            .collect();
        self.pages = pages;

        self.outline_items = self.decode_service.get_outline().unwrap_or_default();
    }

    pub fn reset(&mut self) {
        info!("reset");
        self.pages.clear();
        self.total_width = 0.0;
        self.total_height = 0.0;
        self.visible_pages.clear();
        self.cache.clear();
        self.page_links.borrow_mut().clear();
        self.outline_items.clear();
    }

    /// 更新视图尺寸和缩放
    pub fn update_view_size(&mut self, width: f32, height: f32, zoom: f32, force: bool) {
        let size_changed = self.view_size.0 != width || self.view_size.1 != height;
        let zoom_changed = (self.zoom - zoom).abs() > 0.001;

        if !size_changed && !zoom_changed && !force {
            info!(
                "don't update_view_size. w-h:{:?}-{:?}, zoom:{:?}",
                width, height, zoom
            );
            return;
        }

        self.view_size = (width, height);
        self.zoom = zoom;
        info!(
            "update_view_size. w-h:{:?}-{:?}, zoom:{:?}, view:{:?}-{:?}",
            width, height, zoom, self.view_size.0, self.view_size.1
        );

        self.recalculate_layout();
    }

    pub fn update_zoom(&mut self, zoom: f32) {
        self.update_view_size(self.view_size.0, self.view_size.1, zoom, true);
    }

    /// 更新偏移量
    pub fn update_offset(&mut self, x: f32, y: f32) {
        self.view_offset = (x, y);
    }

    /// 重新计算页面布局
    fn recalculate_layout(&mut self) {
        if self.view_size.0 == 0.0 || self.view_size.1 == 0.0 {
            return;
        }

        match self.orientation {
            Orientation::Vertical => self.layout_vertical(),
            Orientation::Horizontal => self.layout_horizontal(),
        }
    }

    /// 垂直布局
    fn layout_vertical(&mut self) {
        let view_width = self.view_size.0;
        let scaled_width = view_width * self.zoom;
        let mut current_y = 0.0;

        for page in &mut self.pages {
            let page_width = page.info.get_width(self.crop == 1);
            let page_height = page.info.get_height(self.crop == 1);

            // 计算缩放比例
            let scale = scaled_width / page_width;
            let scaled_height = page_height * scale;

            // 更新页面
            let bounds = Rect::new(0.0, current_y, scaled_width, current_y + scaled_height);
            page.update(scaled_width, scaled_height, bounds);
            page.info.scale = scale;

            current_y += scaled_height;
        }

        debug!(
            "layout_vertical.end:total_width:{:?}-total_height:{:?}",
            scaled_width, current_y
        );
        self.total_width = scaled_width;
        self.total_height = current_y;
    }

    /// 水平布局
    fn layout_horizontal(&mut self) {
        let view_height = self.view_size.1;
        let scaled_height = view_height * self.zoom;
        let mut current_x = 0.0;

        for page in &mut self.pages {
            let page_width = page.info.get_width(self.crop == 1);
            let page_height = page.info.get_height(self.crop == 1);

            // 计算缩放比例
            let scale = scaled_height / page_height;
            let scaled_width = page_width * scale;

            // 更新页面
            let bounds = Rect::new(current_x, 0.0, current_x + scaled_width, scaled_height);
            page.update(scaled_width, scaled_height, bounds);
            page.info.scale = scale;

            current_x += scaled_width;
        }

        self.total_width = current_x;
        self.total_height = scaled_height;
    }

    /// 更新可见页面列表
    pub fn update_visible_pages(&mut self) {
        self.visible_pages.clear();

        let (offset_x, offset_y) = self.view_offset;
        let (view_width, view_height) = self.view_size;

        // 计算预加载区域
        let preload_distance = match self.orientation {
            Orientation::Vertical => view_height * self.preload_screens,
            Orientation::Horizontal => view_width * self.preload_screens,
        };

        // 可见区域（包含预加载）
        let visible_rect = match self.orientation {
            Orientation::Vertical => Rect::new(
                -offset_x,
                -offset_y,
                -offset_x + view_width,
                -offset_y + view_height  + preload_distance,
            ),
            Orientation::Horizontal => Rect::new(
                -offset_x,
                -offset_y,
                -offset_x + view_width + preload_distance,
                -offset_y + view_height,
            ),
        };

        // 更新共享的可见区域
        *self.visible_rect.lock().unwrap() = visible_rect.clone();

        // 更新页面bounds映射
        {
            let mut bounds_map = self.page_bounds_map.lock().unwrap();
            bounds_map.clear();
            for page in &self.pages {
                bounds_map.insert(page.info.index, page.bounds.clone());
            }
        }

        // 使用二分查找优化
        let first = self.find_first_visible(&visible_rect);
        let last = self.find_last_visible(&visible_rect);

        debug!("update_visible_pages: first={}, last={}, total_pages={}", 
            first, last, self.pages.len());

        let mut render_pages = Vec::new();

        // 创建可见性检查回调
        let visible_rect_arc = Arc::clone(&self.visible_rect);
        let page_bounds_map_arc = Arc::clone(&self.page_bounds_map);
        let orientation = self.orientation;
        let visibility_checker: VisibilityChecker = Arc::new(move |page_index: usize| -> bool {
            let current_visible = visible_rect_arc.lock().unwrap();
            let bounds_map = page_bounds_map_arc.lock().unwrap();
            
            // 获取页面bounds
            if let Some(page_bounds) = bounds_map.get(&page_index) {
                // 根据方向判断页面是否与可见区域相交
                let intersects = match orientation {
                    Orientation::Vertical => {
                        page_bounds.bottom > current_visible.top && page_bounds.top < current_visible.bottom
                    },
                    Orientation::Horizontal => {
                        page_bounds.right > current_visible.left && page_bounds.left < current_visible.right
                    },
                };
                intersects
            } else {
                // 如果找不到bounds，默认为不可见
                false
            }
        });

        if first <= last && first < self.pages.len() {
            for i in first..=last.min(self.pages.len() - 1) {
                self.visible_pages.push(i);

                let page = &self.pages[i];
                let key = generate_thumbnail_key(page);
                
                if page.width > 0.0 && page.height > 0.0 {
                    // 先检查缓存中是否已有该页面
                    if self.cache.get_thumbnail(&key).is_none() {
                        debug!("需要解码: page={}, key={}", page.info.index, key);
                        
                        render_pages.push(RenderPage {
                            key,
                            page_info: page.info.clone(),
                            crop: self.crop,
                            priority: Priority::Thumbnail,
                            visibility_checker: Some(Arc::clone(&visibility_checker)),
                        });
                    } else {
                        debug!("页面已在缓存中: page={}, key={}", page.info.index, key);
                    }
                }
            }
        }
        
        info!("update_visible_pages完成: visible_pages={:?}", self.visible_pages);

        // 批量提交解码任务
        if !render_pages.is_empty() {
            debug!("批量提交 {} 个解码任务:", render_pages.len());
            self.decode_service.render_pages(render_pages);
        }
    }

    /// 二分查找第一个可见页面
    fn find_first_visible(&self, visible_rect: &Rect) -> usize {
        let mut low = 0;
        let mut high = self.pages.len();
        let mut result = self.pages.len();

        while low < high {
            let mid = (low + high) / 2;
            let page = &self.pages[mid];

            let is_visible = match self.orientation {
                Orientation::Vertical => page.bounds.bottom > visible_rect.top,
                Orientation::Horizontal => page.bounds.right > visible_rect.left,
            };

            if is_visible {
                result = mid;
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        result
    }

    /// 二分查找最后一个可见页面
    fn find_last_visible(&self, visible_rect: &Rect) -> usize {
        let mut low = 0;
        let mut high = self.pages.len();
        let mut result = 0;

        while low < high {
            let mid = (low + high) / 2;
            let page = &self.pages[mid];

            let is_visible = match self.orientation {
                Orientation::Vertical => page.bounds.top < visible_rect.bottom,
                Orientation::Horizontal => page.bounds.left < visible_rect.right,
            };

            if is_visible {
                result = mid;
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        result
    }

    /// 跳转到指定页面
    pub fn jump_to_page(&mut self, page_index: usize) -> Option<(f32, f32)> {
        if page_index >= self.pages.len() {
            return None;
        }

        let page = &self.pages[page_index];
        let new_offset = match self.orientation {
            Orientation::Vertical => (self.view_offset.0, -page.bounds.top),
            Orientation::Horizontal => (-page.bounds.left, self.view_offset.1),
        };

        self.view_offset = new_offset;
        Some(new_offset)
    }

    /// 获取当前第一个可见页面索引
    pub fn get_first_visible_page(&self) -> Option<usize> {
        self.visible_pages.first().copied()
    }

    /// 处理点击事件
    pub fn handle_click(
        &self,
        index: usize,
        doc_x: f32,
        doc_y: f32,
    ) -> Option<crate::decoder::Link> {
        // 根据 index 获取链接缓存
        if let Some(links) = self.page_links.borrow().get(&index) {
            // 判断点击是否在页面范围内（假设点击的是指定页面）
            if index < self.pages.len() {
                let page = &self.pages[index];
                // 检查点击位置是否在链接区域内
                let scale = page.info.scale;
                for link in links {
                    let scaled_left = link.bounds.left * scale;
                    let scaled_right = link.bounds.right * scale;
                    let scaled_top = link.bounds.top * scale;
                    let scaled_bottom = link.bounds.bottom * scale;
                    debug!("link check: click_x={}, click_y={}, link=({}, {}, {}, {}) scaled to ({}, {}, {}, {})",
                                   doc_x, doc_y, link.bounds.left, link.bounds.top, link.bounds.right, link.bounds.bottom,
                                   scaled_left, scaled_top, scaled_right, scaled_bottom);
                    if doc_x >= scaled_left
                        && doc_x <= scaled_right
                        && doc_y >= scaled_top
                        && doc_y <= scaled_bottom
                    {
                        return Some(link.clone());
                    }
                }
            }
        } else {
            info!(
                "handle_click no links cached for page_index:{}",
                index
            );
        }

        None
    }

    /// 设置切边状态
    pub fn set_crop(&mut self, crop: i32) {
        if self.crop != crop {
            self.crop = crop;
            self.recalculate_layout();

            // 清理所有页面缓存
            for page in &mut self.pages {
                page.recycle();
            }

            // 更新可见页面以触发重新解码
            self.update_visible_pages();
        }
    }

    /// 获取页面文本
    pub fn get_page_text(&self, page_index: usize) -> Result<String, Box<dyn std::error::Error>> {
        Ok(self.decode_service.get_page_text(page_index)?)
    }

    /// 从指定页面开始获取后续页面的reflow数据
    pub fn get_reflow_from_page(&self, start_page: usize) -> Result<Vec<crate::entity::ReflowEntry>, Box<dyn std::error::Error>> {
        Ok(self.decode_service.get_reflow_from_page(start_page)?)
    }

    /// 回收资源
    pub fn shutdown(&mut self) {
        info!("shutdown");

        for page in &mut self.pages {
            page.recycle();
        }
        self.pages.clear();
        self.visible_pages.clear();

        self.page_links.borrow_mut().clear();
        self.outline_items.clear();
        self.cache.clear();
    }
}
