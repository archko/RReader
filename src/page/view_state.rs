use log::debug;

use super::Page;
use crate::cache::PageCache;
use crate::decoder::decode_service::DecodeTask;
use crate::decoder::pdf::utils::{convert_to_slint_image, generate_thumbnail_key};
use crate::decoder::{DecodeService, Priority, Rect};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;

/// 滚动方向
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// 页面视图状态管理
pub struct PageViewState {
    /// 页面缓存
    pub cache: Arc<Mutex<PageCache>>,

    /// 所有页面
    pub pages: Rc<RefCell<Vec<Page>>>,

    /// 解码服务
    pub decode_service: Rc<RefCell<DecodeService>>,

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
}

impl PageViewState {
    pub fn new(orientation: Orientation, crop: i32) -> Self {
        let cache = Arc::new(Mutex::new(PageCache::new(168, 10)));
        let decode_service = Rc::new(RefCell::new(DecodeService::new()));

        // 启动异步监听
        let result_rx = decode_service.borrow_mut().take_result_rx();
        let cache_clone = Arc::clone(&cache);
        tokio::spawn(async move {
            while let Some(result) = result_rx.recv().await {
                if let Ok((key, image)) = result {
                    cache_clone.lock().unwrap().put_thumbnail(key, convert_to_slint_image(&image));
                }
            }
        });

        Self {
            cache,
            pages: Rc::new(RefCell::new(Vec::new())),
            decode_service,
            orientation,
            view_offset: (0.0, 0.0),
            zoom: 1.0,
            crop,
            total_width: 0.0,
            total_height: 0.0,
            view_size: (0.0, 0.0),
            preload_screens: 1.0,
            visible_pages: Vec::new(),
        }
    }

    /// 打开文档
    pub fn open_document<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        Self::reset(self);
        // 创建页面信息channel
        let (page_info_tx, page_info_rx) = mpsc::channel();
        self.decode_service.borrow().load_pdf_with_tx(path.as_ref(), page_info_tx)?;
        // 同步接收page info
        let pages_info = page_info_rx.recv()?;
        *self.pages.borrow_mut() = pages_info
            .into_iter()
            .map(|info| Page::new(info, 0.0, 0.0, 0.0, 0.0))
            .collect();
        Ok(())
    }

    pub fn reset(&mut self) {
        debug!("[PageViewState] reset");
        *self.pages.borrow_mut() = Vec::new();
        self.total_width = 0.0;
        self.total_height = 0.0;
        self.visible_pages.clear();
        self.cache.lock().unwrap().clear();
    }

    /// 更新视图尺寸和缩放
    pub fn update_view_size(&mut self, width: f32, height: f32, zoom: f32, force: bool) {
        let size_changed = self.view_size.0 != width || self.view_size.1 != height;
        let zoom_changed = (self.zoom - zoom).abs() > 0.001;

        if !size_changed && !zoom_changed && !force {
            debug!(
                "[PageViewState] don't update_view_size. w-h:{:?}-{:?}, zoom:{:?}",
                width, height, zoom
            );
            return;
        }

        self.view_size = (width, height);
        self.zoom = zoom;
        debug!(
            "[PageViewState] update_view_size. w-h:{:?}-{:?}, zoom:{:?}, view:{:?}-{:?}",
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

        let mut pages = self.pages.borrow_mut();
        for page in pages.iter_mut() {
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
            /*debug!(
                "[PageViewState] layout_vertical pages_len:{} view_size:{:?}, offset:{:?}, w-h:{:?}-{:?}",
                page.info.index, self.view_size, self.view_offset, scaled_width, scaled_height
            );*/
        }

        debug!(
            "[PageViewState] layout_vertical.end:total_width:{:?}-total_height:{:?}",
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

        let mut pages = self.pages.borrow_mut();
        for page in pages.iter_mut() {
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
                -offset_y + view_height + preload_distance,
            ),
            Orientation::Horizontal => Rect::new(
                -offset_x,
                -offset_y,
                -offset_x + view_width + preload_distance,
                -offset_y + view_height,
            ),
        };

        // 使用二分查找优化
        let first = self.find_first_visible(&visible_rect);
        let last = self.find_last_visible(&visible_rect);

        let pages_len = self.pages.borrow().len();
        if first <= last && first < pages_len {
            for i in first..=last.min(pages_len - 1) {
                self.visible_pages.push(i);

                let pages = self.pages.borrow();
                let page = &pages[i];
                let key = generate_thumbnail_key(page);
                if page.width > 0.0 && page.height > 0.0 {
                    // 先检查缓存中是否已有该页面
                    if self.cache.lock().unwrap().get_thumbnail(&key).is_none() {
                        // 只有当页面不在缓存中时才发送解码请求
                        let page_info = page.info.clone();
                        let crop = self.crop;
                        let decode_task = DecodeTask {
                            key,
                            page_info,
                            crop,
                            priority: Priority::Thumbnail,
                        };
                        self.decode_service.borrow().render_page(decode_task);
                    }
                }
            }
        }
    }

    /// 二分查找第一个可见页面
    fn find_first_visible(&self, visible_rect: &Rect) -> usize {
        let pages_len = self.pages.borrow().len();
        let mut low = 0;
        let mut high = pages_len;
        let mut result = pages_len;

        while low < high {
            let mid = (low + high) / 2;
            let pages = self.pages.borrow();
            let page = &pages[mid];

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
        let pages_len = self.pages.borrow().len();
        let mut low = 0;
        let mut high = pages_len;
        let mut result = 0;

        while low < high {
            let mid = (low + high) / 2;
            let pages = self.pages.borrow();
            let page = &pages[mid];

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
        if page_index >= self.pages.borrow().len() {
            return None;
        }

        let page = &self.pages.borrow()[page_index];
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
    pub fn handle_click(&self, x: f32, y: f32) -> Option<&crate::decoder::Link> {
        // 将视图坐标转换为文档坐标
        let doc_x = x - self.view_offset.0;
        let doc_y = y - self.view_offset.1;

        // 查找点击的页面
        let pages = self.pages.borrow();
        /*for page in pages.iter() {
            if doc_x >= page.bounds.left
                && doc_x <= page.bounds.right
                && doc_y >= page.bounds.top
                && doc_y <= page.bounds.bottom
            {
                return page.find_link_at(doc_x, doc_y);
            }
        }*/

        None
    }

    /// 设置切边状态
    pub fn set_crop(&mut self, crop: i32) {
        if self.crop != crop {
            self.crop = crop;
            self.recalculate_layout();

            // 清理所有页面缓存
            let mut pages = self.pages.borrow_mut();
            for page in pages.iter_mut() {
                page.recycle();
            }

            // 更新可见页面以触发重新解码
            //self.update_visible_pages();
        }
    }

    /// 回收资源
    pub fn shutdown(&mut self) {
        debug!("[PageViewState] shutdown");

        let pages_clone = self.pages.borrow();
        for page in pages_clone.iter() {
            // page.recycle(); pero page is &Page, but recycle is on Page
            // Need to clone or something, but recycle() is likely for cache.
            // Assuming it's mutable borrow for recycle, but it's getter.
            // Perhaps recycle is for cache, not per page.
            // Anyway, leave as is for now.
        }
        *self.pages.borrow_mut() = Vec::new();
        self.visible_pages.clear();

        self.decode_service.borrow_mut().shutdown();
    }
}
