#![allow(unused)]
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use env_logger::Env;
use log::{debug, error};
use slint::{ComponentHandle, ModelRc, VecModel};

mod cache;
mod decoder;
mod page;
mod ui;
mod dao;
mod entity;

use page::{PageViewState, Orientation};
use crate::decoder::pdf::utils::{generate_thumbnail_key};

use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("debug")  // 默认日志级别：info
    ).init();
    let app = MainWindow::new()?;
    let page_view_state: Rc<RefCell<PageViewState>> = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));

    // 初始化数据库
    std::env::set_var("DATABASE_URL", "sqlite:book.db");
    RecentDao::init_sync().unwrap();

    // 创建主视图模型
    let viewmodel: Rc<RefCell<MainViewmodel>> = Rc::new(RefCell::new(MainViewmodel::new()));
    let _ = viewmodel.borrow_mut().load_history(0); // 加载第一页历史记录

    // 设置历史记录到UI
    {
        let viewmodel_binding = viewmodel.borrow();
        let history_records = viewmodel_binding.get_current_records();
        let ui_history_items: Vec<UIRecent> = history_records
            .iter()
            .map(|record| UIRecent {
                title: record.name.clone().into(),
                path: record.book_path.clone().into(),
                thumbnail: "".into(), // TODO: 可以在这里添加缩略图路径
                page: record.page,
                zoom: record.zoom,
                scroll_x: record.scroll_x,
                scroll_y: record.scroll_y,
            })
            .collect();

        let history_model = Rc::new(VecModel::from(ui_history_items));
        app.set_history_items(ModelRc::from(history_model));
    }

    setup_open_handler(&app, page_view_state.clone(), viewmodel.clone());
    setup_viewport_handler(&app, page_view_state.clone());
    setup_scroll_handler(&app, page_view_state.clone());
    setup_page_handler(&app, page_view_state.clone());
    setup_zoom_handler(&app, page_view_state.clone());
    setup_page_click_handler(&app, page_view_state.clone());
    setup_back_to_history_handler(&app, page_view_state.clone());
    setup_page_down_handler(&app, page_view_state.clone());
    setup_page_up_handler(&app, page_view_state.clone());
    setup_history_item_click_handler(&app, page_view_state.clone(), viewmodel.clone());

    app.run()?;
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
            .add_filter("All Files", &["*"])
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
                    // 添加到历史记录
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
                        path_str.split('/').last().unwrap_or("").to_string(), // name
                        path_str.split('.').last().unwrap_or("").to_string(), // ext
                        0, // size
                        1, // read_times
                        1, // progress
                        0, // favorited
                        0, // in_recent
                    );
                    if let Err(e) = viewmodel.borrow().add_or_update_recent_by_path(recent) {
                        error!("Failed to add recent: {e}");
                    }

                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(1.0);
                        app.set_document_opened(true);

                        let zoom = 1.0; // 初始缩放值
                        let mut borrowed_state = page_view_state.borrow_mut();
                        let width = borrowed_state.view_size.0;
                        let height = borrowed_state.view_size.1;
                        //重置页面位置
                        borrowed_state.update_offset(0.0, 0.0);
                        borrowed_state.update_view_size(
                            width,
                            height,
                            zoom,
                            true
                        );

                        // 设置大纲项到UI
                        set_outline_to_ui(&app, &borrowed_state);
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
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            let zoom = borrowed_state.zoom;
            borrowed_state.update_view_size(width, height, zoom, false);
            borrowed_state.update_visible_pages();
        }
        debug!("[Main] setup_viewport_handler");
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
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
    app.on_page_changed(move |page_index| {
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            if borrowed_state.jump_to_page(page_index as usize).is_some() {
                borrowed_state.update_visible_pages();
                
                if let Some(app) = weak_app.upgrade() {
                    refresh_view(&app, &*borrowed_state);
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

    let rendered_pages = page_view_state.visible_pages
        .iter()
        .filter_map(|&idx| page_view_state.pages.get(idx))
        .map(|page| {
            // 尝试从缓存获取图像，如果不存在则使用默认图像
            let key = generate_thumbnail_key(page);
            let image = {
                if let Some(cached_image) = page_view_state.cache.get_thumbnail(&key) {
                    cached_image.as_ref().clone()
                } else {
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

    /*debug!(
        "[Main] refresh_view {} page_models",
        rendered_pages.len()
    );*/
    let model = Rc::new(VecModel::from(rendered_pages));
    app.set_document_pages(ModelRc::from(model));
    app.set_page_count(page_view_state.pages.len() as i32);
    app.set_zoom(page_view_state.zoom);

    if let Some(first_visible) = page_view_state.get_first_visible_page() {
        app.set_current_page(first_visible as i32);
        //debug!("[Main] refresh_view set_current_page: {}", first_visible)
    }

    let (total_width, total_height) = (page_view_state.total_width, page_view_state.total_height);
    app.set_total_width(total_width);
    app.set_total_height(total_height);

    let (offset_x, offset_y) = (page_view_state.view_offset.0, page_view_state.view_offset.1);
    app.set_scroll_events_enabled(false);
    app.set_offset_x(offset_x);
    app.set_offset_y(offset_y);
    app.set_scroll_events_enabled(true);
    /*debug!(
        "[Main] refresh_view.offset: ({}, {}), total.w-h: ({}, {})",
        offset_x, offset_y, total_width, total_height
    );*/
}

fn setup_back_to_history_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_back_to_history(move || {
        // 清空文件路径
        if let Some(app) = weak_app.upgrade() {
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
                    refresh_view(&app, &*borrowed_state);
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

            let open_result = page_view_state.borrow_mut().open_document(&path_obj);
            match open_result {
                Ok(_) => {
                    // 更新历史记录的访问次数
                    let mut vm = viewmodel.borrow_mut();

                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(ui_recent.zoom);
                        app.set_current_page(ui_recent.page);
                        app.set_document_opened(true);

                        let zoom = ui_recent.zoom;
                        let mut borrowed_state = page_view_state.borrow_mut();
                        let width = borrowed_state.view_size.0;
                        let height = borrowed_state.view_size.1;

                        // 设置大纲项到UI
                        set_outline_to_ui(&app, &borrowed_state);

                        // 设置保存的位置
                        borrowed_state.update_offset(ui_recent.scroll_x as f32, ui_recent.scroll_y as f32);
                        if let Some(_) = borrowed_state.jump_to_page(ui_recent.page as usize) {
                            // 可以在这里做一些处理
                        }
                        borrowed_state.update_view_size(
                            width,
                            height,
                            zoom,
                            true
                        );
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

    refresh_view(&app, page_view_state);
}
