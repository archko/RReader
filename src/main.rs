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
use floem::views::{Button, Container, Decorators, Label, Scroll, Stack};
use floem::view::IntoView;

use dirs;

mod cache;
mod controllers;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use page::{PageViewState, Orientation, HistoryItem};
use page::history_view::{create_history_view, poll_document_load};
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
// 架构说明（回调式文档渲染）：
//   1. home 模式：显示历史网格 + Open/Clear 工具栏
//   2. document 模式：显示文档画布 + 文档工具栏
//   3. 解码流程：不再轮询 try_recv_result()，而是通过
//      PageCallback::on_completed() 回调直接写入缓存
//      → start_repaint_loop 检测 AtomicBool
//      → 递增信号触发 Canvas 重绘
// ============================================================

fn app_view(viewmodel: Rc<RefCell<MainViewmodel>>, initial_history: Vec<HistoryItem>) -> impl IntoView {
    let page_view_state = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));
    let document_opened = RwSignal::new(false);
    let current_page = RwSignal::new(1);
    let zoom_level = RwSignal::new(1.0f32);
    let file_path = RwSignal::new(String::new());
    let page_count = RwSignal::new(0);
    let viewport_size = RwSignal::new((800.0, 600.0));

    // 历史记录
    let history_items = RwSignal::new(initial_history);

    // 刷新触发器（信号驱动 Canvas 重绘）
    let decode_refresh_trigger = RwSignal::new(0u64);
    let doc_info_trigger = RwSignal::new(0u64);

    // --- 主页工具栏（未打开文档时显示）---
    let home_toolbar = create_home_toolbar(
        page_view_state.clone(),
        document_opened,
        file_path,
        current_page,
        zoom_level,
        page_count,
        history_items,
        viewmodel.clone(),
    );

    // --- 主内容区域（历史网格 ↔ 文档视图切换）---
    let state_for_content = page_view_state.clone();
    let content = dyn_view(move || {
        if document_opened.get() {
            // 文档模式：使用回调式 document_view
            let data = DocumentViewData {
                page_view_state: state_for_content.clone(),
                document_opened: document_opened,
                current_page: current_page,
                page_count: page_count,
                zoom_level: zoom_level,
                file_path: file_path,
                viewport_size: viewport_size,
                decode_refresh_trigger: decode_refresh_trigger,
                doc_info_trigger: doc_info_trigger,
            };
            create_document_view(data).into_any()
        } else {
            // 主页模式：历史网格
            Container::new(create_history_view(
                history_items,
                state_for_content.clone(),
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
                viewmodel.clone(),
            ))
            .style(|s| s.padding(10.0).size(100.pct(), 100.pct()))
            .into_any()
        }
    });

    // --- 整体布局 ---
    let vs = viewport_size;
    Container::new(Stack::vertical((
        home_toolbar,
        Scroll::new(content).style(|s| s.size(100.pct(), 100.pct())),
    )))
    .on_event(floem::event::listener::WindowResized, move |_cx, size| {
        vs.set((size.width, size.height));
        EventPropagation::Continue
    })
    .style(|s| s.keyboard_navigable().size(100.pct(), 100.pct()))
}

// ============================================================
// create_home_toolbar — 主页工具栏（Open / Clear）
// ============================================================

fn create_home_toolbar(
    page_view_state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
    history_items: RwSignal<Vec<HistoryItem>>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
) -> impl IntoView {
    let open_button = Button::new("Open")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let state = page_view_state.clone();
            let document_opened = document_opened.clone();
            let current_page = current_page.clone();
            let zoom_level = zoom_level.clone();
            let file_path = file_path.clone();
            let history_items = history_items.clone();
            let page_count = page_count.clone();
            let viewmodel = viewmodel.clone();

            move |_cx, _event| {
                let file_path_selected = rfd::FileDialog::new()
                    .add_filter("PDF Files", &["pdf"])
                    .add_filter("ePub Files", &["epub"])
                    .add_filter("MOBI Files", &["mobi"])
                    .add_filter("All Files", &["*"])
                    .set_title("Select File")
                    .pick_file();

                if let Some(path) = file_path_selected {
                    let path_str = path.to_string_lossy().to_string();
                    info!("打开文件: {}", path_str);

                    let result = state.borrow_mut().open_document(&path);
                    if result.is_ok() {
                        poll_document_load(
                            state.clone(),
                            document_opened,
                            file_path,
                            current_page,
                            zoom_level,
                            page_count,
                            viewmodel.clone(),
                            path_str,
                        );
                    }
                }
                EventPropagation::Continue
            }
        });

    let clear_button = Button::new("Clear")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let history_items = history_items.clone();
            move |_cx, _event| {
                history_items.set(vec![]);
                EventPropagation::Continue
            }
        });

    let label = Label::derived(move || {
        format!(
            "RReader — {} history items",
            history_items.get().len()
        )
    })
    .style(|s| s.padding_right(8.0));

    Stack::horizontal((open_button, clear_button, label))
        .style(|s| s.padding(10.0).gap(10.0))
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
            })
            .collect::<Vec<_>>()
    };

    floem::launch(move || app_view(viewmodel, initial_history_vec));

    Ok(())
}
