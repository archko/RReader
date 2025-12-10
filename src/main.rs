#![allow(unused)]
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use anyhow::Result;
use env_logger::Env;
use log::{debug, error, info};
use slint::{ComponentHandle, ModelRc, VecModel};

slint::include_modules!();

mod cache;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use page::{PageViewState, Orientation};
use tts::TtsService;
use crate::decoder::pdf::utils::{generate_thumbnail_key, convert_to_slint_image};

use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};
use crate::ui::utils::get_thumbnail_path;

fn convert_history_records_to_items(records: &[Recent]) -> Vec<UIRecent> {
    records
        .iter()
        .map(|record| {
            let path = record.book_path.clone();
            let cache_path = get_thumbnail_path(&path);

            let (thumbnail, has_thumbnail) = if !cache_path.is_empty() {
                if let Ok(dynamic_image) = image::open(&cache_path) {
                    (convert_to_slint_image(&dynamic_image), true)
                } else {
                    (slint::Image::default(), false)
                }
            } else {
                (slint::Image::default(), false)
            };

            UIRecent {
                title: record.name.clone().into(),
                path: path.into(),
                thumbnail,
                has_thumbnail,
                page: record.page,
            }
        })
        .collect()
}

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

fn set_history_to_ui(app: &MainWindow, ui_history_items: Vec<UIRecent>) {
    let history_model = Rc::new(VecModel::from(ui_history_items.clone()));
    app.set_history_items(ModelRc::from(history_model));

    let width = *HISTORY_VIEWPORT_WIDTH.read().unwrap();
    let columns = (width / 188.0).floor().max(1.0) as usize;
    let grouped: Vec<Vec<UIRecent>> = ui_history_items.chunks(columns).map(|c| c.to_vec()).collect();
    let rows: Vec<HistoryRow> = grouped.into_iter().map(|vec| HistoryRow { items: ModelRc::from(Rc::new(VecModel::from(vec))) }).collect();
    let history_rows_model = Rc::new(VecModel::from(rows));
    app.set_history_rows(ModelRc::from(history_rows_model));
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info")  // 默认日志级别：info
    ).init();
    let app = MainWindow::new()?;
    let page_view_state: Rc<RefCell<PageViewState>> = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));

    // 初始化数据库
    // 获取用户数据目录，创建缓存目录
    let data_dir = dirs::data_dir().expect("Unable to get data directory");
    let app_data_dir = data_dir.join("RReader");
    fs::create_dir_all(&app_data_dir).expect("Unable to create app data directory");

    // 构建数据库路径
    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    debug!("Database path: {:?}", db_path);
    debug!("Database URL: {}", database_url);
    std::env::set_var("DATABASE_URL", &database_url);

    // 确保数据库文件和表存在（第一次运行时会创建）
    tokio::task::block_in_place(|| {
        futures::executor::block_on(async {
            crate::dao::ensure_database_ready(&db_path).await.expect("Failed to initialize database");
        });
    });

    // 检查数据库版本或初始化
    // Sea-ORM 不自动升级版本，使用固定的模式
    RecentDao::init_sync().unwrap();

    // 创建主视图模型
    let viewmodel: Rc<RefCell<MainViewmodel>> = Rc::new(RefCell::new(MainViewmodel::new()));
    let _ = viewmodel.borrow_mut().load_history(0); // 加载第一页历史记录

    // 创建TTS服务
    let tts_service = Arc::new(Mutex::new(TtsService::new()));

    // 设置历史记录到UI
    {
        let viewmodel_binding = viewmodel.borrow();
        let history_records = viewmodel_binding.get_current_records();
        let ui_history_items = convert_history_records_to_items(history_records);

        set_history_to_ui(&app, ui_history_items);
    }

    setup_open_handler(&app, page_view_state.clone(), viewmodel.clone());
    setup_viewport_handler(&app, page_view_state.clone());
    setup_history_viewport_handler(&app, viewmodel.clone());
    setup_scroll_handler(&app, page_view_state.clone());
    setup_page_handler(&app, page_view_state.clone());
    setup_zoom_handler(&app, page_view_state.clone());
    setup_page_click_handler(&app, page_view_state.clone());
    setup_back_to_history_handler(&app, page_view_state.clone(), viewmodel.clone());
    setup_page_down_handler(&app, page_view_state.clone());
    setup_page_up_handler(&app, page_view_state.clone());
    setup_history_item_click_handler(&app, page_view_state.clone(), viewmodel.clone());
    setup_speak_page_handler(&app, page_view_state.clone(), tts_service.clone());

    // 设置定时器处理解码结果 - 必须保持timer存活
    let decode_timer = {
        let weak_app = app.as_weak();
        let state_clone = Rc::clone(&page_view_state);
        let timer = slint::Timer::default();
        let timer_count = Rc::new(RefCell::new(0));
        let timer_count_clone = Rc::clone(&timer_count);
        
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(100),
            move || {
                let mut count = timer_count_clone.borrow_mut();
                *count += 1;
                if *count % 10 == 0 {
                    debug!("[Main] 定时器运行中... count={}", *count);
                }
                
                if let Some(app) = weak_app.upgrade() {
                    // 处理所有待处理的结果
                    let mut had_results = false;
                    let mut result_count = 0;
                    {
                        let mut state = state_clone.borrow_mut();
                        // 检查是否有结果（不消费）
                        while let Some(result) = state.decode_service.try_recv_result() {
                            had_results = true;
                            result_count += 1;
                            debug!("[Main] 收到解码结果: page={}, key={}, size={}x{}",
                                result.page_info.index, result.key, result.image_width, result.image_height);

                            // 注意：mupdf_to_pixels 返回的 RGBA 数据中 alpha 值为未预乘，若 Slint 期望预乘则需后续处理
                            let slint_image = slint::Image::from_rgba8_premultiplied(
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                    &result.image_data,
                                    result.image_width,
                                    result.image_height,
                                ),
                            );

                            // 更新缓存
                            state.cache.put_thumbnail(result.key.clone(), slint_image);
                            info!("[Main] 已更新缓存: key={}", result.key);

                            // 更新链接
                            state.page_links
                                .borrow_mut()
                                .insert(result.page_info.index, result.links);
                        }
                    }
                    
                    if had_results {
                        debug!("[Main] 处理了 {} 个解码结果，刷新视图", result_count);
                        refresh_view(&app, &state_clone.borrow());
                    }
                }
            },
        );
        timer // 返回timer以保持其存活
    };

    app.run()?;
    
    // 停止定时器
    decode_timer.stop();
    Ok(())
}

/// 打开文档事件
fn setup_open_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>, viewmodel: Rc<RefCell<MainViewmodel>>) {
    let weak_app = app.as_weak();

    app.on_open_file(move || {
        let file_path = rfd::FileDialog::new()
            .add_filter("PDF Files", &["pdf"])
            .add_filter("PDF Files", &["epub"])
            .add_filter("PDF Files", &["mobi"])
            .add_filter("All Files", &["cbz"])
            .add_filter("All Files", &["docx"])
            .add_filter("All Files", &["xps"])
            .add_filter("All Files", &["djvu"])
            .add_filter("All Files", &["tif"])
            .add_filter("All Files", &["tiff"])
            .set_title("Select PDF File")
            .pick_file();

        if let Some(path) = file_path {
            let path_str = path.to_string_lossy().to_string();
            if let Some(app) = weak_app.upgrade() {
                app.set_file_path(path_str.clone().into());
            }

            let open_result = page_view_state.borrow_mut().open_document(&path);
            match open_result {
                Ok(_) => {
                    // 先查询数据库是否存在记录
                    let existing_recent = viewmodel.borrow().get_recent_by_path(&path_str).unwrap_or(None);

                    let (zoom, page, scroll_x, scroll_y) = if let Some(ref rec) = existing_recent {
                        (rec.zoom, rec.page, rec.scroll_x, rec.scroll_y)
                    } else {
                        (1.0, 1, 0, 0) // 默认值
                    };

                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(zoom);
                        app.set_current_page(page);
                        app.set_document_opened(true);

                        let mut borrowed_state = page_view_state.borrow_mut();
                        let width = borrowed_state.view_size.0;
                        let height = borrowed_state.view_size.1;
                        
                        borrowed_state.update_view_size(
                            width,
                            height,
                            zoom,
                            true
                        );

                        // 设置保存的位置
                        if borrowed_state.jump_to_page((page - 1) as usize).is_some() {
                            // 可以在这里做一些处理
                        }
                        //borrowed_state.update_offset(scroll_x as f32, scroll_y as f32);

                        // 设置大纲项到UI
                        set_outline_to_ui(&app, &borrowed_state);
                    }

                    // 如果没有记录，插入新记录
                    if existing_recent.is_none() {
                        let recent = Recent::encode(
                            path_str.clone(),
                            0, // 默认页
                            0, // 默认页数，会被更新
                            1, // crop
                            1, // scroll_ori (vertical)
                            0, // reflow
                            1.0, // zoom
                            0, // scroll_x
                            0, // scroll_y
                            path_str.split('/').next_back().unwrap_or("").to_string(), // name
                            path_str.split('.').next_back().unwrap_or("").to_string(), // ext
                            0, // size
                            0, // read_times
                            1, // progress
                            0, // favorited
                            0, // in_recent
                        );
                        if let Err(e) = viewmodel.borrow().add_recent(recent) {
                            error!("Failed to add recent: {e}");
                        }
                    }

                    // 文档打开后立即刷新视图
                    if let Some(app) = weak_app.upgrade() {
                        page_view_state.borrow_mut().update_visible_pages();
                        refresh_view(&app, &page_view_state.borrow());
                    }
                }
                Err(err) => {
                    error!("Failed to open PDF: {err}");
                }
            }
        }
    });
}

/// 视图创建事件
fn setup_viewport_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_viewport_changed(move |width, height| {
        debug!("[Main] setup_viewport_handler.width: {:?}, height: {:?}", width, height);
        let mut current_page = None;
        {
            let borrowed_state = page_view_state.borrow();
            if !borrowed_state.pages.is_empty() {
                current_page = borrowed_state.get_first_visible_page();
            }
        }
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            let zoom = borrowed_state.zoom;
            borrowed_state.update_view_size(width, height, zoom, false);
            borrowed_state.update_visible_pages();

            // 如果当前页面不再可见，则跳转到该页面,大纲显示与隐藏会触发布局变化
            if let Some(page) = current_page {
                borrowed_state.jump_to_page(page);
                borrowed_state.update_visible_pages();
            }
        }
        debug!("[Main] setup_viewport_handler");
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
        }
    });
}

fn setup_history_viewport_handler(app: &MainWindow, viewmodel: Rc<RefCell<MainViewmodel>>) {
    let weak_app = app.as_weak();
    app.on_history_viewport_changed(move |width, height| {
        debug!("[Main] on_history_viewport_changed.width: {:?}, height: {:?}", width, height);

        let old_width = *HISTORY_VIEWPORT_WIDTH.read().unwrap();
        if (width - old_width).abs() > 0.1 { // 宽度变化阈值，防止抖动
            *HISTORY_VIEWPORT_WIDTH.write().unwrap() = width;

            // 重新计算列数并更新历史记录布局
            if let Some(app) = weak_app.upgrade() {
                let viewmodel_binding = viewmodel.borrow();
                let history_records = viewmodel_binding.get_current_records();
                let ui_history_items = convert_history_records_to_items(history_records);
                set_history_to_ui(&app, ui_history_items);
                debug!("[Main] Updated history column count for new viewport width: {}", width);
            }
        }
    });
}

/// 滚动处理
fn setup_scroll_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_scroll_changed(move |offset_x, offset_y| {
        if let Some(app) = weak_app.upgrade() {
            debug!("[Main] setup_scroll_handler, {offset_x}, {offset_y}");
            update_view_offset(&app, &mut page_view_state.borrow_mut(), offset_x, offset_y);
        }
    });
}

/// 页面跳转处理
fn setup_page_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_page_changed(move |page_index| {  // page_index is 1-based from UI
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            if borrowed_state.jump_to_page((page_index - 1) as usize).is_some() {
                borrowed_state.update_visible_pages();

                if let Some(app) = weak_app.upgrade() {
                    refresh_view(&app, &borrowed_state);
                }
            }
        }
    });
}

/// 缩放处理
fn setup_zoom_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_zoom_changed(move |zoom| {
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            let (view_width, view_height) = borrowed_state.view_size;
            borrowed_state.update_view_size(view_width, view_height, zoom, true);
            borrowed_state.update_visible_pages();
        }
        
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
        }
    });
}

/// 刷新视图显示
fn refresh_view(app: &MainWindow, page_view_state: &PageViewState) {
    let state = page_view_state;
    if state.pages.is_empty() {
        debug!("[Main] No pages to refresh");
        return;
    }

    debug!("[Main] refresh_view: visible_pages={:?}", page_view_state.visible_pages);

    let rendered_pages = page_view_state.visible_pages
        .iter()
        .filter_map(|&idx| page_view_state.pages.get(idx))
        .map(|page| {
            // 尝试从缓存获取图像，如果不存在则使用默认图像
            let key = generate_thumbnail_key(page);
            let image = {
                if let Some(cached_image) = page_view_state.cache.get_thumbnail(&key) {
                    debug!("[Main] 从缓存获取图像: key={}, page={}", key, page.info.index);
                    cached_image.as_ref().clone()
                } else {
                    debug!("[Main] 缓存中没有图像，显示页码: key={}, page={}", key, page.info.index);
                    slint::Image::default()
                }
            };
            
            PageData {
                x: page.bounds.left,
                y: page.bounds.top,
                width: page.width,
                height: page.height,
                image,
                page_index: page.info.index as i32,
            }
        })
        .collect::<Vec<_>>();

    debug!("[Main] refresh_view {} page_models", rendered_pages.len());
    let model = Rc::new(VecModel::from(rendered_pages));
    app.set_document_pages(ModelRc::from(model));
    app.set_page_count(page_view_state.pages.len() as i32);
    app.set_zoom(page_view_state.zoom);

    if let Some(first_visible) = page_view_state.get_first_visible_page() {
        app.set_current_page((first_visible + 1) as i32);  // UI expects 1-based page numbers
    }

    let (total_width, total_height) = (page_view_state.total_width, page_view_state.total_height);
    app.set_total_width(total_width);
    app.set_total_height(total_height);

    let (offset_x, offset_y) = (page_view_state.view_offset.0, page_view_state.view_offset.1);
    debug!(
        "[Main] refresh_view.offset: ({}, {}), total.w-h: ({}, {})",
        offset_x, offset_y, total_width, total_height
    );
}

fn setup_back_to_history_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>, viewmodel: Rc<RefCell<MainViewmodel>>) {
    let weak_app = app.as_weak();
    let weak_viewmodel = Rc::downgrade(&viewmodel);
    app.on_back_to_history(move || {
        if let Some(app) = weak_app.upgrade() {
            let current_path = app.get_file_path().to_string();

            if !current_path.is_empty() {
                // 获取当前可见页的第一页
                let page = page_view_state.borrow().get_first_visible_page();
                let zoom = page_view_state.borrow().zoom;
                let (offset_x, offset_y) = page_view_state.borrow().view_offset;

                info!("back to history: page:{:?}, zoom:{:?}, offset_x:{:?}, offset_y:{:?}, path:{:?}", page, zoom, offset_x, offset_y, current_path);
                // 更新记录的状态
                if let Some(vm) = weak_viewmodel.upgrade() {
                    let update_result = vm.borrow().update_recent_with_state(&current_path, page, zoom, offset_x, offset_y);
                    if let Err(e) = update_result {
                        error!("Failed to update recent state: {e}");
                    }
                }
            }

            if let Some(vm) = weak_viewmodel.upgrade() {
                let _ = vm.borrow_mut().load_history(0);
                let vm_binding = vm.borrow();
                let history_records = vm_binding.get_current_records();
                let ui_history_items = convert_history_records_to_items(history_records);
                set_history_to_ui(&app, ui_history_items);
            }

            // 清空文件路径
            app.set_file_path("".into());
            app.set_document_opened(false);
        }

        // 重置页面状态
        let mut borrowed_state = page_view_state.borrow_mut();
        borrowed_state.shutdown();
    });
}

/// 向下翻页（减少Y轴偏移量）,开始偏移量是0,
fn setup_page_down_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_page_down(move || {
        if let Some(app) = weak_app.upgrade() {
            let viewport_height = app.get_viewport_height();
            let current_offset_y = app.get_offset_y();
            
            let offset_y = current_offset_y - viewport_height + 16.0;
            let offset_x = app.get_offset_x();

            debug!("[Main] on_page_down, {offset_x}, {current_offset_y}, {offset_y}, height:{viewport_height}");

            update_view_offset(&app, &mut page_view_state.borrow_mut(), offset_x, offset_y);
        }
    });
}

/// 向上翻页（增加Y轴偏移量）
fn setup_page_up_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_page_up(move || {
        if let Some(app) = weak_app.upgrade() {
            let viewport_height = app.get_viewport_height();
            let current_offset_y = app.get_offset_y();
            
            let offset_y = current_offset_y + viewport_height - 16.0;
            let offset_x = app.get_offset_x();

            debug!("[Main] on_page_up, {offset_x}, {current_offset_y}, {offset_y}, height:{viewport_height}");

            update_view_offset(&app, &mut page_view_state.borrow_mut(), offset_x, offset_y);
        }
    });
}

/// 页面点击处理
fn setup_page_click_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_page_clicked(move |x, y, page_index| {
        debug!("[Main] setup_page_click_handler: x={x}, y={y}, page_index={page_index}");

        // The coordinates are already in document space (after adding page.x, page.y)
        let jump_to_page = if let Some(link) = page_view_state.borrow().handle_click(page_index as usize, x, y) {
            debug!("[Main] Clicked link: uri={:?}, page={:?}", link.uri, link.page);
            // TODO: Handle link types
            if let Some(uri) = &link.uri {
                debug!("[Main] URI link clicked: {}", uri);
                None
            } else if let Some(page) = link.page {
                debug!("[Main] Page link clicked: {}", page);
                parse_page_from_param(&page)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(page_num) = jump_to_page {
            if let Some(app) = weak_app.upgrade() {
                let mut borrowed_state = page_view_state.borrow_mut();
                if borrowed_state.jump_to_page(page_num).is_some() {
                    borrowed_state.update_visible_pages();
                    refresh_view(&app, &borrowed_state);
                }
            }
        }
    });
}

fn parse_page_from_param(page_param: &str) -> Option<usize> {
    if page_param.starts_with("#page=") {
        let start = "#page=".len();
        let end = page_param[start..].find('&').map(|pos| pos + start).unwrap_or(page_param.len());
        let num_str = &page_param[start..end];
        num_str.parse::<usize>().ok()
    } else {
        None
    }
}

/// 历史记录项点击处理
fn setup_history_item_click_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>, viewmodel: Rc<RefCell<MainViewmodel>>) {
    let weak_app = app.as_weak();

    app.on_history_item_clicked(move |ui_recent| {
        let path_str = ui_recent.path.to_string();
        let path_obj = std::path::Path::new(&path_str);

        if path_obj.exists() {
            if let Some(app) = weak_app.upgrade() {
                app.set_file_path(path_str.clone().into());
            }

            // 从MainViewmodel的current_page_records查询与ui_recent路径相同的记录
            let recent_record = viewmodel.borrow().get_current_records()
                .iter()
                .find(|rec| rec.book_path == path_str)
                .cloned();

            let open_result = page_view_state.borrow_mut().open_document(path_obj);
            match open_result {
                Ok(_) => {
                    if let Some(app) = weak_app.upgrade() {
                        // 使用从viewmodel查询到的recent对象替换ui_recent
                        if let Some(ref rec) = recent_record {
                            app.set_zoom(rec.zoom);
                            app.set_current_page(rec.page);
                            app.set_document_opened(true);
                            info!("history_item.page:{:?}, zoom:{:?}, path:{:?}", rec.page, rec.zoom, rec.book_path);

                            let zoom = rec.zoom;
                            let mut borrowed_state = page_view_state.borrow_mut();
                            let width = borrowed_state.view_size.0;
                            let height = borrowed_state.view_size.1;

                            // 设置大纲项到UI
                            set_outline_to_ui(&app, &borrowed_state);

                            borrowed_state.update_view_size(
                                width,
                                height,
                                zoom,
                                true
                            );
                            // 设置保存的位置
                            //borrowed_state.update_offset(rec.scroll_x as f32, rec.scroll_y as f32);
                            if borrowed_state.jump_to_page((rec.page - 1) as usize).is_some() {
                                // 可以在这里做一些处理
                            }
                        } else {
                            // 如果没有找到记录，使用默认值
                            app.set_zoom(1.0);
                            app.set_current_page(1);
                            app.set_document_opened(true);

                            let zoom = 1.0;
                            let mut borrowed_state = page_view_state.borrow_mut();
                            let width = borrowed_state.view_size.0;
                            let height = borrowed_state.view_size.1;

                            // 设置大纲项到UI
                            set_outline_to_ui(&app, &borrowed_state);

                            borrowed_state.update_offset(0.0, 0.0);
                            borrowed_state.update_view_size(
                                width,
                                height,
                                zoom,
                                true
                            );
                        }
                    }

                    // 文档打开后立即刷新视图
                    if let Some(app) = weak_app.upgrade() {
                        page_view_state.borrow_mut().update_visible_pages();
                        refresh_view(&app, &page_view_state.borrow());
                    }
                }
                Err(err) => {
                    error!("Failed to open PDF from history: {err}");
                }
            }
        } else {
            error!("File does not exist: {path_str}");
        }
    });
}

/// 设置大纲项到UI
fn set_outline_to_ui(app: &MainWindow, page_view_state: &PageViewState) {
    let ui_outline_items: Vec<UIOutlineItem> = page_view_state.outline_items.iter().map(|oi| UIOutlineItem {
        title: oi.title.clone().into(),
        page: oi.page,
        level: oi.level,
    }).collect();
    app.set_outline_items(ModelRc::from(Rc::new(VecModel::from(ui_outline_items)) as Rc<dyn slint::Model<Data = UIOutlineItem>>));
}

fn update_view_offset(app: &MainWindow, page_view_state: &mut PageViewState, offset_x:f32, offset_y:f32) {
    //debug!("[Main] update_view_offset, {offset_x}, {offset_y}");
    page_view_state.update_offset(offset_x, offset_y);
    page_view_state.update_visible_pages();

    refresh_view(app, page_view_state);
}

/// TTS
fn setup_speak_page_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>, tts_service: Arc<Mutex<TtsService>>) {
    app.on_speak_page(move || {
        // 如果正在朗读，停止朗读
        // TODO: 需要添加检查方式，目前简化处理，先停止再开始
        
        if let Some(page_index) = page_view_state.borrow().get_first_visible_page() {
            match page_view_state.borrow().get_reflow_from_page(page_index) {
                Ok(reflow_entries) => {
                    if !reflow_entries.is_empty() {
                        info!("[TTS] Speaking reflow text from page {} onwards, {} entries", page_index, reflow_entries.len());
                        let tts = Arc::clone(&tts_service);

                        // 将所有reflow条目的文本拼接成一个长文本并发送
                        let combined_text = reflow_entries.into_iter()
                            .map(|entry| entry.data)
                            .collect::<Vec<String>>()
                            .join(" ");

                        if !combined_text.is_empty() {
                            let mut tts_locked = tts.lock().unwrap();
                            tts_locked.stop_speaking(); // 先停止之前的朗读
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
    });
}
