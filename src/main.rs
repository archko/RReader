#![allow(unused)]
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use env_logger::Env;
use log::{debug, error, info};

use floem::prelude::*;
use floem::event::EventPropagation;
use floem::views::{Container, Decorators};
use floem::view::IntoView;

use dirs;

mod cache;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use page::{PageViewState, Orientation, HistoryItem};
use page::history_view::create_history_view;
use page::document_view::{create_document_view, DocumentViewData};
use tts::TtsService;
use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::Recent;

async fn setup_database() -> Result<()> {
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
            crate::dao::ensure_database_ready(&db_path)
                .await
                .expect("Failed to initialize database");
        });
    });

    RecentDao::init_sync().unwrap();
    Ok(())
}

// ============================================================
// app_view — 应用主视图
//
// main.rs 仅负责在「历史视图」与「文档视图」之间切换。
// 每个视图都是自包含的：
//   - 工具栏固定在顶部（不滚动）
//   - 内容区由内部的 Scroll 处理滚动
// 没有外层 Scroll，Container 提供 100% 窗口大小约束。
// ============================================================

fn app_view(viewmodel: Rc<RefCell<MainViewmodel>>, initial_history: Vec<HistoryItem>) -> impl IntoView {
    let page_view_state = Arc::new(PageViewState::new(Orientation::Vertical, 0));
    page_view_state.init_self_arc(page_view_state.clone());
    let document_opened = RwSignal::new(false);
    let current_page = RwSignal::new(1);
    let zoom_level = RwSignal::new(1.0f32);
    let file_path = RwSignal::new(String::new());
    let page_count = RwSignal::new(0);
    let viewport_size = RwSignal::new((800.0, 600.0));

    let history_items = RwSignal::new(initial_history);
    let decode_refresh_trigger = RwSignal::new(0u64);
    let doc_info_trigger = RwSignal::new(0u64);

    // 用 dyn_view 切换两个自包含视图（互斥）
    let state_for_content = page_view_state.clone();
    let vm = viewmodel.clone();
    let content = dyn_view(move || {
        if document_opened.get() {
            let data = DocumentViewData {
                page_view_state: state_for_content.clone(),
                document_opened,
                current_page,
                page_count,
                zoom_level,
                file_path,
                viewport_size,
                decode_refresh_trigger,
                doc_info_trigger,
            };
            create_document_view(data).into_any()
        } else {
            create_history_view(
                history_items,
                state_for_content.clone(),
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
                vm.clone(),
            )
            .into_any()
        }
    });

    // Container 提供 100% 窗口大小约束，没有外层 Scroll
    let vs = viewport_size;
    Container::new(content)
        .on_event(floem::event::listener::WindowResized, move |_cx, size| {
            vs.set((size.width, size.height));
            EventPropagation::Continue
        })
        .style(|s| s.keyboard_navigable().size(100.pct(), 100.pct()))
}

// ============================================================
// main — 程序入口
// ============================================================

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info"),
    )
    .init();

    setup_database().await?;

    let viewmodel = Rc::new(RefCell::new(MainViewmodel::new()));
    let initial_history_vec = {
        let mut vm_borrow = viewmodel.borrow_mut();
        let _ = vm_borrow.load_history(0);
        vm_borrow
            .get_current_records()
            .iter()
            .map(|r| HistoryItem {
                title: r.name.clone(),
                path: r.book_path.clone(),
                page: r.page,
                page_count: r.page_count,
            })
            .collect::<Vec<_>>()
    };

    floem::launch(move || app_view(viewmodel, initial_history_vec));

    Ok(())
}
