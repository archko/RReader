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

mod app_handler;
mod cache;
mod controllers;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use app_handler::AppHandler;
use page::{PageViewState, Orientation};
use tts::TtsService;
use crate::decoder::pdf::utils::{generate_thumbnail_key, convert_to_slint_image};
use crate::controllers::DocumentController;

use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};
use crate::ui::utils::get_thumbnail_path;

/// 设置文档相关回调
fn setup_document_callbacks(app: &AppWindow, document_controller: Rc<RefCell<DocumentController>>) {
    let weak_app = app.as_weak();
    let document_controller_clone = Rc::clone(&document_controller);

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
                document_controller_clone.borrow().open_document(&app, &path_str);
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info")  // 默认日志级别：info
    ).init();

    let app = AppWindow::new()?;

    let data_dir = dirs::data_dir().expect("Unable to get data directory");
    let app_data_dir = data_dir.join("RReader");
    fs::create_dir_all(&app_data_dir).expect("Unable to create app data directory");

    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    debug!("Database path: {:?}", db_path);
    debug!("Database URL: {}", database_url);
    std::env::set_var("DATABASE_URL", &database_url);

    tokio::task::block_in_place(|| {
        futures::executor::block_on(async {
            crate::dao::ensure_database_ready(&db_path).await.expect("Failed to initialize database");
        });
    });

    RecentDao::init_sync().unwrap();

    let viewmodel: Rc<RefCell<MainViewmodel>> = Rc::new(RefCell::new(MainViewmodel::new()));

    //BusyLayerController::invoke_set_busy();

    let tts_service = Arc::new(Mutex::new(TtsService::new()));

    let mut app_handler = AppHandler::new(viewmodel.clone(), Arc::clone(&tts_service));

    setup_document_callbacks(&app, app_handler.document_controller());
    if let Err(e) = viewmodel.borrow_mut().load_history(0) {
        log::error!("Failed to load history: {}", e);
    }

    app_handler.initialize_ui(&app);

    //BusyLayerController::invoke_unset_busy();

    let decode_timer = {
        let weak_app = app.as_weak();
        let state_clone = Rc::clone(&app_handler.document_controller().borrow().page_view_state());
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
                    let mut had_results = false;
                    let mut result_count = 0;
                    {
                        let mut state = state_clone.borrow_mut();
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
                        use crate::controllers::DocumentController;
                        DocumentController::refresh_view(&app, &state_clone.borrow());
                    }
                }
            },
        );
        timer
    };

    app.run()?;

    decode_timer.stop();
    app_handler.save();

    Ok(())
}
