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
use crate::controllers::DocumentController;

use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};
use crate::ui::utils::get_thumbnail_path;

/// 打开文件对话框
fn pick_file() -> Option<String> {
    let file_path = rfd::FileDialog::new()
        .add_filter("支持的文件", &["pdf", "epub", "mobi", "cbz", "docx", "xps", "djvu", "tif", "tiff"])
        .set_title("选择文档")
        .pick_file();

    file_path.map(|path| path.to_string_lossy().to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info")
    ).init();

    // 初始化应用数据目录
    let data_dir = dirs::data_dir().expect("Unable to get data directory");
    let app_data_dir = data_dir.join("RReader");
    fs::create_dir_all(&app_data_dir).expect("Unable to create app data directory");

    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    debug!("Database path: {:?}", db_path);
    debug!("Database URL: {}", database_url);
    std::env::set_var("DATABASE_URL", &database_url);

    // 初始化数据库
    tokio::task::block_in_place(|| {
        futures::executor::block_on(async {
            crate::dao::ensure_database_ready(&db_path).await
                .expect("Failed to initialize database");
        });
    });

    RecentDao::init_sync().unwrap();

    let viewmodel: Rc<RefCell<MainViewmodel>> = Rc::new(RefCell::new(MainViewmodel::new()));

    let tts_service = Arc::new(Mutex::new(TtsService::new()));

    let mut app_handler = AppHandler::new(viewmodel.clone(), Arc::clone(&tts_service));

    if let Err(e) = viewmodel.borrow_mut().load_history(0) {
        log::error!("Failed to load history: {}", e);
    }

    // TODO: 使用 Xilem 启动 UI
    // 后续步骤将用 Xilem 的 App 启动替代 Slint 的 app.run()
    info!("RReader 启动 - 正在等待 Xilem UI 集成...");

    // 临时阻塞等待后续Xilem集成
    // 使用一个简单的循环保持进程运行
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // app_handler.save();
    // Ok(())
}
