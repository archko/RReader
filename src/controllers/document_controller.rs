//use crate::ui::MainViewmodel;
//use std::cell::RefCell;
//use std::rc::Rc;
//use crate::page::{PageViewState, Orientation};
//use crate::decoder::{PageInfo};
//use crate::decoder::pdf::utils::{generate_thumbnail_key};
//use crate::tts::TtsService;
//use std::sync::Arc;
//use std::sync::Mutex;
//use log::{debug, info, error};
//use crate::controllers::history_controller::{convert_history_records_to_items, set_history_to_ui};
//
//use crate::AppWindow;
//
//pub struct DocumentController {
//    viewmodel: Rc<RefCell<MainViewmodel>>,
//    page_view_state: Rc<RefCell<PageViewState>>,
//    tts_service: Arc<Mutex<TtsService>>,
//    load_timer: RefCell<Option<Timer>>,
//}
//
//impl DocumentController {
//    pub fn new(viewmodel: Rc<RefCell<MainViewmodel>>, tts_service: Arc<Mutex<TtsService>>) -> Self {
//        let page_view_state = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));
//        Self { viewmodel, page_view_state, tts_service, load_timer: RefCell::new(None) }
//    }
//
//    /// 初始化UI，将控制器连接到Slint窗口
//    pub fn initialize_ui(&self, window: &AppWindow) {
//        self.setup_callbacks(window);
//    }
//
//    /// 设置文档相关的回调
//    fn setup_callbacks(&self, window: &AppWindow) {
//        // View大小变化回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let weak_window = window.as_weak();
//            window.on_viewport_changed(move |width, height| {
//                debug!("on_viewport_changed: width={}, height={}", width, height);
//                if width == 0.0 || height == 0.0 {
//                    return;
//                }
//
//                let mut current_page = None;
//                {
//                    let borrowed_state = page_view_state.borrow();
//                    if !borrowed_state.pages.is_empty() {
//                        current_page = borrowed_state.get_first_visible_page();
//                    }
//                }
//                {
//                    info!("current_page: width={}", current_page.unwrap_or_default());
//                    let mut state = page_view_state.borrow_mut();
//                    let zoom = state.zoom;
//                    state.update_view_size(width as f32, height as f32, zoom, false);
//
//                    // 如果当前页面不再可见，则跳转到该页面，大纲显示与隐藏会触发布局变化
//                    if let Some(page) = current_page {
//                        if let Some((new_x, new_y)) = state.jump_to_page(page) {
//                            // 同步更新 UI 偏移量，但临时禁用滚动事件
//                            if let Some(window) = weak_window.upgrade() {
//                                window.set_scroll_events_enabled(false);
//                                window.set_offset_x(new_x);
//                                window.set_offset_y(new_y);
//                                window.set_scroll_events_enabled(true);
//                            }
//                        }
//                    }
//                    state.update_visible_pages();
//
//                    if let Some(window) = weak_window.upgrade() {
//                        window.set_total_width(state.total_width);
//                        window.set_total_height(state.total_height);
//                        Self::refresh_view(&window, &state);
//                    }
//                }
//            });
//        }
//
//        // 缩放变化回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let weak_window = window.as_weak();
//            window.on_zoom_changed(move |zoom| {
//                if let Some(window) = weak_window.upgrade() {
//                    let mut state = page_view_state.borrow_mut();
//
//                    // 获取当前页面，缩放后保持在同一页面
//                    let current_page = state.get_first_visible_page();
//
//                    state.update_zoom(zoom as f32);
//
//                    // 更新总尺寸
//                    window.set_total_width(state.total_width);
//                    window.set_total_height(state.total_height);
//
//                    // 如果有当前页面，跳转回该页面并同步偏移量
//                    if let Some(page) = current_page {
//                        if let Some((new_x, new_y)) = state.jump_to_page(page) {
//                            // 同步更新 UI 偏移量，但临时禁用滚动事件
//                            window.set_scroll_events_enabled(false);
//                            window.set_offset_x(new_x);
//                            window.set_offset_y(new_y);
//                            window.set_scroll_events_enabled(true);
//                        }
//                    }
//
//                    state.update_visible_pages();
//                    Self::refresh_view(&window, &state);
//                }
//            });
//        }
//
//        // 滚动变化回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let weak_window = window.as_weak();
//            window.on_scroll_changed(move |x, y| {
//                if let Some(window) = weak_window.upgrade() {
//                    debug!("on_scroll_changed");
//                    let mut state = page_view_state.borrow_mut();
//                    state.update_offset(x as f32, y as f32);
//                    state.update_visible_pages();
//                    Self::refresh_view(&window, &state);
//                }
//            });
//        }
//
//        // 页码变化回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let weak_window = window.as_weak();
//            window.on_page_changed(move |page_index| {  // page_index is 1-based from UI
//                let mut state = page_view_state.borrow_mut();
//                info!("on_page_changed.page={:?}", page_index);
//                if state.jump_to_page((page_index - 1) as usize).is_some() {
//                    state.update_visible_pages();
//
//                    if let Some(window) = weak_window.upgrade() {
//                        Self::refresh_view(&window, &state);
//
//                        window.set_offset_x(state.view_offset.0);
//                        window.set_offset_y(state.view_offset.1);
//                    }
//                }
//            });
//        }
//
//        // 返回历史回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let viewmodel = Rc::clone(&self.viewmodel);
//            let weak_window = window.as_weak();
//            window.on_back_to_history(move || {
//                if let Some(window) = weak_window.upgrade() {
//                    let current_path = window.get_file_path().to_string();
//
//                    if !current_path.is_empty() {
//                        // 获取当前可见页的第一页
//                        let page = page_view_state.borrow().get_first_visible_page();
//                        let zoom = page_view_state.borrow().zoom;
//                        let (offset_x, offset_y) = page_view_state.borrow().view_offset;
//
//                        info!("back to history: page:{:?}, zoom:{:?}, offset_x:{:?}, offset_y:{:?}, path:{:?}", page, zoom, offset_x, offset_y, current_path);
//                        // 更新记录的状态
//                        let update_result = viewmodel.borrow().update_recent_with_state(&current_path, page, zoom, offset_x, offset_y);
//                        if let Err(e) = update_result {
//                            error!("Failed to update recent state: {e}");
//                        }
//                    }
//
//                    let _ = viewmodel.borrow_mut().load_history(0);
//                    let vm_binding = viewmodel.borrow();
//                    let history_records = vm_binding.get_current_records();
//                    let ui_history_items = convert_history_records_to_items(history_records);
//                    set_history_to_ui(&window, ui_history_items);
//
//                    // 清空文件路径
//                    window.set_file_path(SharedString::from(""));
//                    window.set_document_opened(false);
//                }
//
//                // 重置页面状态
//                let mut borrowed_state = page_view_state.borrow_mut();
//                borrowed_state.shutdown();
//            });
//        }
//
//        // 页面点击回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let weak_window = window.as_weak();
//            window.on_page_clicked(move |x, y, page_index| {
//                debug!("on_page_clicked: x={}, y={}, page_index={}", x, y, page_index);
//
//                // 先处理链接，获取跳转页面
//                let jump_to_page = {
//                    let state = page_view_state.borrow();
//                    if let Some(link) = state.handle_click(page_index as usize, x as f32, y as f32) {
//                        info!("Clicked link: uri={:?}, page={:?}", link.uri, link.page);
//                        // 处理链接类型
//                        if let Some(uri) = &link.uri {
//                            debug!("URI link clicked: {}", uri);
//                            None
//                        } else if let Some(page) = link.page {
//                            debug!("Page link clicked: {}", page);
//                            Self::parse_page_from_param(&page)
//                        } else {
//                            None
//                        }
//                    } else {
//                        None
//                    }
//                };
//
//                if let Some(page_num) = jump_to_page {
//                    if let Some(window) = weak_window.upgrade() {
//                        let mut state = page_view_state.borrow_mut();
//                        if state.jump_to_page(page_num).is_some() {
//                            state.update_visible_pages();
//                            Self::refresh_view(&window, &state);
//
//                            window.set_scroll_events_enabled(false);
//                            window.set_offset_x(state.view_offset.0);
//                            window.set_offset_y(state.view_offset.1);
//                            window.set_scroll_events_enabled(true);
//                        }
//                    }
//                }
//            });
//        }
//
//        // 朗读页面回调
//        {
//            let page_view_state = Rc::clone(&self.page_view_state);
//            let tts_service = Arc::clone(&self.tts_service);
//            window.on_speak_page(move || {
//                // 如果正在朗读，停止朗读
//                // TODO: 需要添加检查方式，目前简化处理，先停止再开始
//                if let Some(page_index) = page_view_state.borrow().get_first_visible_page() {
//                    match page_view_state.borrow().get_reflow_from_page(page_index) {
//                        Ok(reflow_entries) => {
//                            if !reflow_entries.is_empty() {
//                                info!("[TTS] Speaking reflow text from page {} onwards, {} entries", page_index, reflow_entries.len());
//                                let tts = Arc::clone(&tts_service);
//
//                                // 将所有reflow条目的文本拼接成一个长文本并发送
//                                let combined_text = reflow_entries.into_iter()
//                                    .map(|entry| entry.data)
//                                    .collect::<Vec<String>>()
//                                    .join(" ");
//
//                                if !combined_text.is_empty() {
//                                    let mut tts_locked = tts.lock().unwrap();
//                                    tts_locked.stop_speaking(); // 先停止之前的朗读
//                                    tts_locked.speak_text(combined_text);
//                                } else {
//                                    error!("[TTS] No valid text content to speak");
//                                }
//                            } else {
//                                error!("[TTS] No reflow entries found");
//                            }
//                        }
//                        Err(e) => {
//                            error!("[TTS] Failed to get reflow data: {}", e);
//                        }
//                    }
//                } else {
//                    error!("[TTS] No visible page found");
//                }
//            });
//        }
//    }
//
//    /// 刷新视图
//    pub(crate) fn refresh_view(window: &AppWindow, state: &PageViewState) {
//        if state.pages.is_empty() {
//            debug!("No pages to refresh");
//            return;
//        }
//
//        debug!("refresh_view: visible_pages={:?}", state.visible_pages);
//
//        let rendered_pages = state.visible_pages
//            .iter()
//            .filter_map(|&idx| state.pages.get(idx))
//            .map(|page| {
//                // 尝试从缓存获取图像，如果不存在则使用默认图像
//                let key = crate::decoder::pdf::utils::generate_thumbnail_key(page);
//                let image = {
//                    if let Some(cached_image) = state.cache.get_thumbnail(&key) {
//                        //debug!("从缓存获取图像: key={}, page={}", key, page.info.index);
//                        cached_image.as_ref().clone()
//                    } else {
//                        //debug!("缓存中没有图像，显示页码: key={}, page={}", key, page.info.index);
//                        slint::Image::default()
//                    }
//                };
//
//                crate::PageData {
//                    x: page.bounds.left,
//                    y: page.bounds.top,
//                    width: page.width,
//                    height: page.height,
//                    image,
//                    page_index: page.info.index as i32,
//                }
//            })
//            .collect::<Vec<_>>();
//
//        info!("refresh_view {} page_models", rendered_pages.len());
//        let model = Rc::new(VecModel::from(rendered_pages));
//        window.set_document_pages(ModelRc::from(model));
//
//        if let Some(first_visible) = state.get_first_visible_page() {
//            window.set_current_page((first_visible + 1) as i32);  // UI expects 1-based page numbers
//        }
//    }
//
//    /// 打开文档 - 触发异步文档加载流程
//    pub fn open_document(&self, window: &AppWindow, path: &str) {
//        info!("Opening document: {}", path);
//
//        if let Some(mut old_timer) = self.load_timer.take() {
//            old_timer.stop();
//        }
//
//        let weak_window = window.as_weak();
//        let path_str = path.to_string();
//        let state = Rc::clone(&self.page_view_state);
//        let viewmodel_clone = Rc::clone(&self.viewmodel);
//
//        let timer_active = Rc::new(RefCell::new(true));
//        let timer_active_clone = Rc::clone(&timer_active);
//
//        let mut timer = Timer::default();
//        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(100), move || {
//            if !*timer_active_clone.borrow() {
//                return;
//            }
//
//            let result = {
//                let borrowed = state.borrow();
//                borrowed.decode_service.try_recv_load_result()
//            };
//            if let Some(result) = result {
//                *timer_active_clone.borrow_mut() = false;
//
//                if let Some(window) = weak_window.upgrade() {
//                    Self::handle_document_opened(&window, result, &path_str, Rc::clone(&state), Rc::clone(&viewmodel_clone));
//                }
//            }
//        });
//
//        self.load_timer.replace(Some(timer));
//
//        let open_result = self.page_view_state.borrow_mut().open_document(path);
//    }
//
//    fn handle_document_opened(window: &AppWindow, result: Result<Vec<PageInfo>, anyhow::Error>, path: &str, page_view_state: Rc<RefCell<PageViewState>>, viewmodel: Rc<RefCell<MainViewmodel>>) {
//        match result {
//            Ok(pages) => {
//                let mut state = page_view_state.borrow_mut();
//                state.set_pages_from_info(pages);
//
//                // 先查询数据库是否存在记录
//                let existing_recent = viewmodel.borrow().get_recent_by_path(path).unwrap_or(None);
//
//                let (zoom, page, scroll_x, scroll_y) = if let Some(ref rec) = existing_recent {
//                    (rec.zoom, rec.page, rec.scroll_x, rec.scroll_y)
//                } else {
//                    (1.0, 1, 0, 0) // 默认值
//                };
//
//                window.set_file_path(path.into());
//                window.set_zoom(zoom);
//                window.set_current_page(page);
//                window.set_document_opened(true);
//                window.set_page_count(state.pages.len() as i32);
//
//                let width = state.view_size.0;
//                let height = state.view_size.1;
//
//                state.update_view_size(
//                    width,
//                    height,
//                    zoom,
//                    true
//                );
//                let (total_width, total_height) = (state.total_width, state.total_height);
//                window.set_total_width(total_width);
//                window.set_total_height(total_height);
//
//                if state.jump_to_page((page - 1) as usize).is_some() {
//                }
//                window.set_offset_x(scroll_x as f32);
//                window.set_offset_y(scroll_y as f32);
//
//                Self::set_outline_to_ui(window, &state);
//
//                if existing_recent.is_none() {
//                    let recent = crate::entity::Recent::encode(
//                        path.to_string(),
//                        0, // 默认页
//                        0, // 默认页数，会被更新
//                        1, // crop
//                        1, // scroll_ori (vertical)
//                        0, // reflow
//                        1.0, // zoom
//                        0, // scroll_x
//                        0, // scroll_y
//                        path.split('/').next_back().unwrap_or("").to_string(), // name
//                        path.split('.').next_back().unwrap_or("").to_string(), // ext
//                        0, // size
//                        0, // read_times
//                        1, // progress
//                        0, // favorited
//                        0, // in_recent
//                    );
//                    if let Err(e) = viewmodel.borrow().add_recent(recent) {
//                        error!("Failed to add recent: {e}");
//                    }
//                }
//
//                state.update_visible_pages();
//                Self::refresh_view(window, &state);
//            }
//            Err(err) => {
//                error!("Failed to open PDF: {err}");
//                window.set_error_message("打开文档失败".into());
//                window.set_show_error_dialog(true);
//                let mut borrowed_state = page_view_state.borrow_mut();
//                borrowed_state.shutdown();
//            }
//        }
//    }
//
//    pub fn close_document(&self, window: &AppWindow) {
//        let mut state = self.page_view_state.borrow_mut();
//        state.reset();
//        window.set_file_path(SharedString::from(""));
//        window.set_document_opened(false);
//    }
//
//    pub fn page_view_state(&self) -> Rc<RefCell<PageViewState>> {
//        Rc::clone(&self.page_view_state)
//    }
//
//    /// 设置大纲项到UI
//    fn set_outline_to_ui(app: &AppWindow, page_view_state: &PageViewState) {
//        let ui_outline_items: Vec<crate::OutlineItem> = page_view_state.outline_items.iter().map(|oi| crate::OutlineItem {
//            title: oi.title.clone().into(),
//            page: oi.page,
//            level: oi.level,
//        }).collect();
//        app.set_outline_items(ModelRc::from(Rc::new(VecModel::from(ui_outline_items))));
//    }
//
//    fn parse_page_from_param(page_param: &str) -> Option<usize> {
//        if page_param.starts_with("#page=") {
//            // 以 '&' 为分割符，得到第一个元素
//            let first_part = page_param.split('&').next()?;
//            // 以 '=' 为分割符，得到第二个元素（数字部分）
//            let parts: Vec<&str> = first_part.split('=').collect();
//            if parts.len() == 2 {
//                parts[1].parse::<usize>().ok()
//            } else {
//                None
//            }
//        } else {
//            None
//        }
//    }
//}
//