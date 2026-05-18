#![allow(unused)]
#![allow(dead_code)]
#![allow(non_snake_case)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use env_logger::Env;
use log::info;
use winit::error::EventLoopError;

mod app_handler;
mod cache;
mod controllers;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use xilem::{EventLoop, WindowOptions, Xilem};
use ui::{HomeViewState, home_view};

/// 打开文件对话框（由 Xilem 回调触发）
fn pick_file() -> Option<String> {
    let file_path = rfd::FileDialog::new()
        .add_filter("支持的文件", &[
            "pdf", "epub", "mobi", "cbz", "docx", "xps", "djvu", "tif", "tiff",
        ])
        .set_title("选择文档")
        .pick_file();
    file_path.map(|p| p.to_string_lossy().to_string())
}

#[tokio::main]
async fn main() -> Result<(), EventLoopError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // 初始化应用数据目录
    let data_dir = dirs::data_dir().expect("Unable to get data directory");
    let app_data_dir = data_dir.join("RReader");
    std::fs::create_dir_all(&app_data_dir).expect("Unable to create app data directory");

    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    info!("Database: {}", database_url);
    std::env::set_var("DATABASE_URL", &database_url);

    // 初始化数据库
    tokio::task::block_in_place(|| {
        futures::executor::block_on(async {
            crate::dao::ensure_database_ready(&db_path)
                .await
                .expect("Failed to initialize database");
        });
    });
    crate::dao::RecentDao::init_sync().unwrap();

    // 启动 Xilem UI
    let state = HomeViewState::default();
    let app = Xilem::new_simple(
        state,
        home_view,
        WindowOptions::new("RReader - 文档阅读").with_min_inner_size(winit::dpi::LogicalSize::new(
            900.0,
            700.0,
        )),
    );
    app.run_in(EventLoop::with_user_event())?;

    Ok(())
}
