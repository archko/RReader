use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;
use crate::ui::MainViewmodel;
use crate::page::{PageViewState, Orientation, ViewModel, PageData};
use crate::page::view_model::SlotEntry;
use crate::decoder::{PageInfo};
use crate::decoder::pdf::utils::generate_thumbnail_key;
use crate::tts::TtsService;
use std::sync::Arc;
use std::sync::Mutex;
use log::{debug, info, error};
use crate::controllers::history_controller::{convert_history_records_to_items, UIRecent};

/// 文档状态变化时的通知回调
pub struct DocumentCallbacks {
    pub on_pages_updated: Option<Box<dyn Fn(Vec<PageData>, usize)>>,
    pub on_tile_updated: Option<Box<dyn Fn(usize, PageData)>>,
    pub on_document_opened: Option<Box<dyn Fn()>>,
    pub on_error: Option<Box<dyn Fn(String)>>,
}

impl DocumentCallbacks {
    pub fn new() -> Self {
        Self {
            on_pages_updated: None,
            on_tile_updated: None,
            on_document_opened: None,
            on_error: None,
        }
    }
}

pub struct DocumentController {
    viewmodel: Rc<RefCell<MainViewmodel>>,
    page_view_state: Rc<RefCell<PageViewState>>,
    tts_service: Arc<Mutex<TtsService>>,
    callbacks: RefCell<DocumentCallbacks>,
    current_path: RefCell<String>,
}

impl DocumentController {
    pub fn new(viewmodel: Rc<RefCell<MainViewmodel>>, tts_service: Arc<Mutex<TtsService>>) -> Self {
        let page_view_state = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));
        Self {
            viewmodel,
            page_view_state,
            tts_service,
            callbacks: RefCell::new(DocumentCallbacks::new()),
            current_path: RefCell::new(String::new()),
        }
    }

    /// 设置UI回调
    pub fn set_callbacks(&self, callbacks: DocumentCallbacks) {
        self.callbacks.replace(callbacks);
    }

    /// 打开文档 - 触发异步文档加载流程
    pub fn open_document(&self, path: &str) {
        info!("Opening document: {}", path);

        let path_str = path.to_string();
        let state = Rc::clone(&self.page_view_state);
        let viewmodel_clone = Rc::clone(&self.viewmodel);

        // 记录当前路径
        *self.current_path.borrow_mut() = path_str.clone();

        // 提交加载任务给解码器
        if let Err(e) = self.page_view_state.borrow_mut().open_document(&path_str) {
            error!("Failed to start document load: {e}");
            return;
        }

        // 启动后台线程轮询加载结果
        std::thread::spawn(move || {
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 300; // 30秒超时
            loop {
                let result = {
                    let borrowed = state.borrow();
                    borrowed.decode_service.try_recv_load_result()
                };
                if let Some(result) = result {
                    Self::handle_document_opened(
                        result, &path_str,
                        Rc::clone(&state),
                        Rc::clone(&viewmodel_clone),
                    );
                    break;
                }
                attempts += 1;
                if attempts >= MAX_ATTEMPTS {
                    error!("Document load timed out: {}", path_str);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }

    fn handle_document_opened(
        result: Result<Vec<PageInfo>, anyhow::Error>,
        path: &str,
        page_view_state: Rc<RefCell<PageViewState>>,
        viewmodel: Rc<RefCell<MainViewmodel>>,
    ) {
        match result {
            Ok(pages) => {
                let mut state = page_view_state.borrow_mut();
                state.set_pages_from_info(pages);

                // 先查询数据库是否存在记录
                let existing_recent = viewmodel.borrow().get_recent_by_path(path).unwrap_or(None);

                let (zoom, page, scroll_x, scroll_y) = if let Some(ref rec) = existing_recent {
                    (rec.zoom, rec.page, rec.scroll_x, rec.scroll_y)
                } else {
                    (1.0, 1, 0, 0)
                };

                let width = state.view_size.0;
                let height = state.view_size.1;

                state.update_view_size(width, height, zoom, true);
                state.update_visible_pages();

                if existing_recent.is_none() {
                    let recent = crate::entity::Recent::encode(
                        path.to_string(),
                        0, 0, 1, 1, 0, 1.0, 0, 0,
                        path.split('/').next_back().unwrap_or("").to_string(),
                        path.split('.').next_back().unwrap_or("").to_string(),
                        0, 0, 1, 0, 0,
                    );
                    if let Err(e) = viewmodel.borrow().add_recent(recent) {
                        error!("Failed to add recent: {e}");
                    }
                }

                info!("Document opened: {} pages", state.pages.len());
                // 通知UI更新
                // 后续由Xilem UI组件读取页面状态
            }
            Err(err) => {
                error!("Failed to open PDF: {err}");
                let mut borrowed_state = page_view_state.borrow_mut();
                borrowed_state.shutdown();
            }
        }
    }

    /// 处理解码线程返回的结果（轮询调用）
    pub fn poll_decode_results(&self) {
        let mut state = self.page_view_state.borrow_mut();
        while let Some(result) = state.decode_service.try_recv_result() {
            let dynamic_image = {
                let img_buffer = image::ImageBuffer::from_raw(
                    result.image_width,
                    result.image_height,
                    result.image_data,
                );
                if let Some(buf) = img_buffer {
                    DynamicImage::ImageRgba8(buf)
                } else {
                    continue;
                }
            };

            let img_for_ui = dynamic_image.clone();

            // 更新缓存
            state.cache.put_page_image_by_key(result.key.clone(), dynamic_image);

            // 更新链接
            state.page_links
                .borrow_mut()
                .insert(result.page_info.index, result.links);

            // 增量更新视图
            Self::apply_tile(
                &state,
                &result.key,
                img_for_ui,
                result.page_info.index,
                result.image_width,
                result.image_height,
            );
        }
    }

    /// 全量同步视图数据
    pub(crate) fn sync_view(state: &PageViewState) -> Vec<PageData> {
        if state.pages.is_empty() {
            debug!("No pages to sync");
            return Vec::new();
        }

        debug!("sync_view: visible_pages={:?}", state.visible_pages);

        let mut slots = Vec::with_capacity(state.visible_pages.len());
        let mut datas = Vec::with_capacity(state.visible_pages.len());

        for &idx in &state.visible_pages {
            if let Some(page) = state.pages.get(idx) {
                let key = generate_thumbnail_key(page);
                let image = state.cache.get_page_image_by_key(&key)
                    .map(|a| a.as_ref().clone());

                slots.push(SlotEntry {
                    cache_key: key,
                    page_index: idx,
                    x: page.bounds.left,
                    y: page.bounds.top,
                    width: page.width,
                    height: page.height,
                });

                datas.push(PageData {
                    x: page.bounds.left,
                    y: page.bounds.top,
                    width: page.width,
                    height: page.height,
                    image,
                    page_index: idx as i32,
                });
            }
        }

        info!("sync_view: {} 个条目", datas.len());
        state.view_model.sync(slots, datas);
        datas
    }

    /// 增量更新单个解码完成的图片
    pub(crate) fn apply_tile(
        state: &PageViewState,
        key: &str,
        image: DynamicImage,
        page_index: usize,
        image_width: u32,
        image_height: u32,
    ) {
        if let Some(page) = state.pages.get(page_index) {
            state.view_model.apply_tile(
                key, image,
                page.bounds.left, page.bounds.top,
                image_width as f32, image_height as f32,
                page_index as i32,
            );
        }
    }

    pub fn close_document(&self) {
        let mut state = self.page_view_state.borrow_mut();
        state.reset();
    }

    pub fn page_view_state(&self) -> Rc<RefCell<PageViewState>> {
        Rc::clone(&self.page_view_state)
    }

    pub fn page_view_state_ref(&self) -> std::cell::Ref<PageViewState> {
        self.page_view_state.borrow()
    }

    /// 获取当前页面数据（供Xilem UI消费）
    pub fn get_current_page_data(&self) -> Vec<PageData> {
        let state = self.page_view_state.borrow();
        Self::sync_view(&state)
    }

    /// 更新视图大小
    pub fn update_view_size(&self, width: f32, height: f32, zoom: f32) {
        let mut state = self.page_view_state.borrow_mut();
        state.update_view_size(width, height, zoom, false);
        state.update_visible_pages();
    }

    /// 更新缩放
    pub fn update_zoom(&self, zoom: f32) {
        let mut state = self.page_view_state.borrow_mut();
        let current_page = state.get_first_visible_page();
        state.update_zoom(zoom);
        if let Some(page) = current_page {
            state.jump_to_page(page);
        }
        state.update_visible_pages();
    }

    /// 更新滚动偏移
    pub fn update_scroll(&self, x: f32, y: f32) {
        let mut state = self.page_view_state.borrow_mut();
        state.update_offset(x, y);
        state.update_visible_pages();
    }

    /// 跳转到页面
    pub fn jump_to_page(&self, page_index: usize) -> Option<(f32, f32)> {
        let mut state = self.page_view_state.borrow_mut();
        let result = state.jump_to_page(page_index);
        if result.is_some() {
            state.update_visible_pages();
        }
        result
    }

    /// 返回历史记录页面
    pub fn back_to_history(&self) {
        let current_path = self.current_path.borrow().clone();
        if !current_path.is_empty() {
            let (page, zoom, offset_x, offset_y) = {
                let state = self.page_view_state.borrow();
                let page = state.get_first_visible_page();
                let zoom = state.zoom;
                let (offset_x, offset_y) = state.view_offset;
                (page, zoom, offset_x, offset_y)
            };

            info!("back to history: page:{:?}, zoom:{:?}, offset_x:{:?}, offset_y:{:?}, path:{:?}",
                page, zoom, offset_x, offset_y, current_path);

            let update_result = self.viewmodel.borrow()
                .update_recent_with_state(&current_path, page, zoom, offset_x, offset_y);
            if let Err(e) = update_result {
                error!("Failed to update recent state: {e}");
            }
        }

        let _ = self.viewmodel.borrow_mut().load_history(0);

        // 重置页面状态
        let mut borrowed_state = self.page_view_state.borrow_mut();
        borrowed_state.shutdown();
        *self.current_path.borrow_mut() = String::new();
    }

    /// 处理页面点击（链接跳转）
    pub fn handle_page_click(&self, x: f32, y: f32, page_index: usize) -> Option<usize> {
        let state = self.page_view_state.borrow();
        if let Some(link) = state.handle_click(page_index, x, y) {
            info!("Clicked link: uri={:?}, page={:?}", link.uri, link.page);
            if let Some(uri) = &link.uri {
                debug!("URI link clicked: {}", uri);
                None
            } else if let Some(page) = link.page {
                Self::parse_page_from_param(&page)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// TTS朗读当前可见页面
    pub fn speak_page(&self) {
        if let Some(page_index) = self.page_view_state.borrow().get_first_visible_page() {
            match self.page_view_state.borrow().get_reflow_from_page(page_index) {
                Ok(reflow_entries) => {
                    if !reflow_entries.is_empty() {
                        info!("[TTS] Speaking reflow text from page {} onwards, {} entries",
                            page_index, reflow_entries.len());
                        let tts = Arc::clone(&self.tts_service);

                        let combined_text = reflow_entries.into_iter()
                            .map(|entry| entry.data)
                            .collect::<Vec<String>>()
                            .join(" ");

                        if !combined_text.is_empty() {
                            let mut tts_locked = tts.lock().unwrap();
                            tts_locked.stop_speaking();
                            tts_locked.speak_text(combined_text);
                        } else {
                            error!("[TTS] No valid text content to speak");
                        }
                    } else {
                        error!("[TTS] No reflow entries found");
                    }
                }
                Err(e) => {
                    error!("[TTS] Failed to get reflow data: {}", e);
                }
            }
        } else {
            error!("[TTS] No visible page found");
        }
    }

    fn parse_page_from_param(page_param: &str) -> Option<usize> {
        if page_param.starts_with("#page=") {
            let first_part = page_param.split('&').next()?;
            let parts: Vec<&str> = first_part.split('=').collect();
            if parts.len() == 2 {
                parts[1].parse::<usize>().ok()
            } else {
                None
            }
        } else {
            None
        }
    }
}
