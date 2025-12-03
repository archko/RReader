#![allow(unused)]
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use anyhow::Result;
use env_logger::Env;
use log::{debug, error, info};
use floem::prelude::*;
use floem::event::EventPropagation;
use dirs;

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

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

fn app_view(initial_history: Vec<HistoryItem>) -> impl IntoView {
    let page_view_state = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));
    let document_opened = RwSignal::new(false);
    let current_page = RwSignal::new(1);
    let zoom_level = RwSignal::new(1.0f32);
    let file_path = RwSignal::new(String::new());
    let page_count = RwSignal::new(0);

    // History items from database
    let history_items = RwSignal::new(initial_history);

    let open_button = button("Open").on_click({
        let state = page_view_state.clone();
        let document_opened = document_opened.clone();
        let current_page = current_page.clone();
        let zoom_level = zoom_level.clone();
        let file_path = file_path.clone();
        move |_| {
            let file_path_selected = rfd::FileDialog::new()
                .add_filter("PDF Files", &["pdf"])
                .add_filter("ePub Files", &["epub"])
                .add_filter("MOBI Files", &["mobi"])
                .add_filter("All Files", &["*"])
                .set_title("Select File")
                .pick_file();

            if let Some(path) = file_path_selected.clone() {
                let result = state.borrow_mut().open_document(&path);
                if result.is_ok() {
                    document_opened.set(true);
                    file_path.set(path.to_string_lossy().to_string());
                    current_page.set(1);
                    zoom_level.set(1.0);
                    if let Some(items) = path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()) {
                        // Update viewmodel etc.
                    }
                }
            }
            EventPropagation::Continue
        }
    });

    let prev_button = button("Previous").on_click({
        let state = page_view_state.clone();
        let current_page = current_page.clone();
        move |_| {
            let new_page = (current_page.get() as usize).saturating_sub(1);
            if new_page > 0 {
                current_page.set(new_page as i32);
                let _ = state.borrow_mut().jump_to_page(new_page.saturating_sub(1));
            }
            EventPropagation::Continue
        }
    });

    let next_button = button("Next").on_click({
        let state = page_view_state.clone();
        let current_page = current_page.clone();
        let page_count = page_count.clone();
        move |_| {
            let new_page = current_page.get() as usize + 1;
            let max_pages = page_count.get() as usize;
            if new_page <= max_pages {
                current_page.set(new_page as i32);
                let _ = state.borrow_mut().jump_to_page(new_page.saturating_sub(1));
            }
            EventPropagation::Continue
        }
    });

    let zoom_in_button = button("Zoom +").on_click({
        let zoom_level = zoom_level.clone();
        let state = page_view_state.clone();
        move |_| {
            let new_zoom = (zoom_level.get() + 0.1).min(4.0);
            zoom_level.set(new_zoom);
            state.borrow_mut().update_zoom(new_zoom);
            EventPropagation::Continue
        }
    });

    let zoom_out_button = button("Zoom -").on_click({
        let zoom_level = zoom_level.clone();
        let state = page_view_state.clone();
        move |_| {
            let new_zoom = (zoom_level.get() - 0.1).max(0.5);
            zoom_level.set(new_zoom);
            state.borrow_mut().update_zoom(new_zoom);
            EventPropagation::Continue
        }
    });

    let status_text = move || format!("Page {} / {} | Zoom: {:.1}% | File: {}",
                                      current_page.get(),
                                      page_count.get(),
                                      zoom_level.get() * 100.0,
                                      file_path.get());

    let history_list = scroll(container(history_grid(history_items)).style(|s| s.width(100.pct()))).style(|s| s.size(100.pct(), 100.pct()));

    let clear_button = button("Clear").on_click({
        let history_items = history_items.clone();
        move |_| {
            history_items.set(vec![]);
            EventPropagation::Continue
        }
    });

    // Always show toolbar + history grid
    container(stack((
        h_stack((
            open_button,
            clear_button,
        )).style(|s| s.justify_end().gap(8.0).padding(10.0)),
        scroll(history_list),
    )))
    .keyboard_navigable()
    .style(|s| s.size(100.pct(), 100.pct()))
}

async fn setup_database() -> Result<()> {
    let data_dir = dirs::data_dir().unwrap();
    let app_data_dir = data_dir.join("RReader");
    fs::create_dir_all(&app_data_dir)?;
    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    set_var("DATABASE_URL", &database_url);

    if !db_path.exists() {
        crate::dao::ensure_database_ready(&db_path).await?;
    }
    RecentDao::init_sync().unwrap();
    Ok(())
}

use std::env::set_var;

fn main() {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info")
    ).init();

    // Synchronously initialize database and load initial data
    let runtime = tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime");

    runtime.block_on(async {
        if let Err(e) = setup_database().await {
            eprintln!("Failed to setup database: {e}");
        }
        RecentDao::init_sync().expect("Failed to init DAO");
    });

    let initial_history = runtime.block_on(load_initial_history())
        .unwrap_or_else(|_| vec![]);

    // Launch the floem window with initial data
    floem::launch(move || app_view(initial_history));
}

async fn load_initial_history() -> Result<Vec<HistoryItem>> {
    let mut viewmodel = MainViewmodel::new();
    if let Err(_e) = viewmodel.load_history(0) {
        // Ignore error for now, return empty
        return Ok(vec![]);
    }
    let records = viewmodel.get_current_records();

    let history_items = records.into_iter()
        .map(|record| HistoryItem {
            title: record.name.clone(),
            path: record.book_path.clone(),
            page: record.page,
        })
        .collect();

    Ok(history_items)
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct HistoryItem {
    title: String,
    path: String,
    page: i32,
}

fn history_grid(history_items: RwSignal<Vec<HistoryItem>>) -> impl IntoView {
    // Return single column for now due to Floem constraints
    // Each row is a horizontal stack of 4 cards
    dyn_stack(
        move || {
            let items = history_items.get();
            items.chunks(4).map(|chunk| {
                let row_data: Vec<HistoryItem> = chunk.to_vec();
                row_data
            }).collect::<Vec<_>>()
        },
        move |row_data| row_data.clone(),
        move |row_data| {
            // Create h_stack for each row
            match row_data.len() {
                4 => h_stack((history_card(row_data[0].clone()), history_card(row_data[1].clone()), history_card(row_data[2].clone()), history_card(row_data[3].clone()))).style(|s| s.gap(8.0)),
                3 => h_stack((history_card(row_data[0].clone()), history_card(row_data[1].clone()), history_card(row_data[2].clone()), empty().style(|s| s.size(180.0, 240.0)))).style(|s| s.gap(8.0)),
                2 => h_stack((history_card(row_data[0].clone()), history_card(row_data[1].clone()), empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)))).style(|s| s.gap(8.0)),
                1 => h_stack((history_card(row_data[0].clone()), empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)))).style(|s| s.gap(8.0)),
                _ => h_stack((empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)), empty().style(|s| s.size(180.0, 240.0)))).style(|s| s.gap(8.0)),
            }
        }
    ).style(|s| s.gap(8.0))
}

fn history_card(item: HistoryItem) -> impl IntoView {
    // Extract filename from path
    let filename = std::path::Path::new(&item.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // Create a card similar to Slint's design (180px width, 240px height)
    // Card: 180x240, thumbnail: 160x160, bottom content: 20x80 with some padding
    container(v_stack((
        // Thumbnail placeholder (originally slint-logo-full-light.svg)
        container(label(|| ""))
            .style(|s| s.size(160.0, 160.0)),
        // Title and path
        label(move || format!("{}\n{}", item.title.clone(), filename))
            .style(|s| s.font_size(14.0)),
    )))
    .style(|s| s.size(180.0, 240.0))
    .style(|s| s.padding(10.0))
}
