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
use floem::style::TextOverflow;
use floem::event::EventPropagation;
use dirs;

mod cache;
mod decoder;
mod page;
mod ui;
mod dao;
mod entity;

use page::{PageViewState, Orientation};
use crate::decoder::pdf::utils::generate_thumbnail_key;

use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};

use std::env::set_var;

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

fn app_view(initial_history: Vec<HistoryItem>) -> impl IntoView {
    let page_view_state = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));
    let document_opened = RwSignal::new(false);
    let current_page = RwSignal::new(1);
    let zoom_level = RwSignal::new(1.0f32);
    let file_path = RwSignal::new(String::new());
    let page_count = RwSignal::new(0);
    let viewport_size = RwSignal::new((800.0, 600.0)); // Default viewport size

    // History items from database
    let history_items = RwSignal::new(initial_history);

    let status_text = move || format!("Page {} / {} | Zoom: {:.1}% | File: {}",
                                      current_page.get(),
                                      page_count.get(),
                                      zoom_level.get() * 100.0,
                                      file_path.get());

    // 工具栏布局 - 响应式
    let toolbar = dyn_view(move || {
        let state = page_view_state.clone();
        let document_opened_inner = document_opened.clone();
        let current_page_inner = current_page.clone();
        let zoom_level_inner = zoom_level.clone();
        let file_path_inner = file_path.clone();
        let history_items_inner = history_items.clone();
        let page_count_inner = page_count.clone();

        if document_opened.get() {
            // 文档打开时的工具栏
            let open_button = button("Open")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({

                    let state = state.clone();
                    let document_opened = document_opened_inner.clone();
                    let current_page = current_page_inner.clone();
                    let zoom_level = zoom_level_inner.clone();
                    let file_path = file_path_inner.clone();
                    let history_items = history_items_inner.clone();
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
                                // Add to history
                                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                                    let new_item = HistoryItem {
                                        title: file_name.to_string(),
                                        path: path.to_string_lossy().to_string(),
                                        page: 1,
                                    };
                                    let mut items = history_items.get();
                                    items.insert(0, new_item);
                                    history_items.set(items);
                                }
                            }
                        }
                        EventPropagation::Continue
                    }
                });

            let back_button = button("Back")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let document_opened = document_opened_inner.clone();
                    move |_| {
                        document_opened.set(false);
                        EventPropagation::Continue
                    }
                });

            let prev_button = button("Previous")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let state = state.clone();
                    let current_page = current_page_inner.clone();
                    move |_| {
                        let new_page = (current_page.get() as usize).saturating_sub(1);
                        if new_page > 0 {
                            current_page.set(new_page as i32);
                            let _ = state.borrow_mut().jump_to_page(new_page.saturating_sub(1));
                        }
                        EventPropagation::Continue
                    }
                });

            let next_button = button("Next")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let state = state.clone();
                    let current_page = current_page_inner.clone();
                    let page_count = page_count_inner.clone();
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

            let zoom_in_button = button("Zoom +")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let zoom_level = zoom_level_inner.clone();
                    let state = state.clone();
                    move |_| {
                        let new_zoom = (zoom_level.get() + 0.1).min(4.0);
                        zoom_level.set(new_zoom);
                        state.borrow_mut().update_zoom(new_zoom);
                        EventPropagation::Continue
                    }
                });

            let zoom_out_button = button("Zoom -")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let zoom_level = zoom_level_inner.clone();
                    let state = state.clone();
                    move |_| {
                        let new_zoom = (zoom_level.get() - 0.1).max(0.5);
                        zoom_level.set(new_zoom);
                        state.borrow_mut().update_zoom(new_zoom);
                        EventPropagation::Continue
                    }
                });

            h_stack((
                open_button,
                back_button,
                // 页面和缩放信息
                label(move || format!("Page {} / {} | Zoom: {:.1}%",
                                      current_page.get(),
                                      page_count.get(),
                                      zoom_level.get() * 100.0))
                    .style(|s| s.padding_right(8.0)),
                // 文件路径，伸缩显示
                container(label(move || file_path.get()))
                    .style(|s| s.flex_grow(1.0).text_overflow(TextOverflow::Ellipsis)),
                // 导航按钮
                prev_button,
                next_button,
                zoom_out_button,
                zoom_in_button,
            ))
            .style(|s| {
                s.padding(10.0)
                    .background(Color::rgb(0.95, 0.95, 0.95))
                    .gap(10.0)
            })
        } else {
            // 未打开文档时的工具栏
            let open_button = button("Open")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({

                    let state = state.clone();
                    let document_opened = document_opened_inner.clone();
                    let current_page = current_page_inner.clone();
                    let zoom_level = zoom_level_inner.clone();
                    let file_path = file_path_inner.clone();
                    let history_items = history_items_inner.clone();
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
                                // Add to history
                                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                                    let new_item = HistoryItem {
                                        title: file_name.to_string(),
                                        path: path.to_string_lossy().to_string(),
                                        page: 1,
                                    };
                                    let mut items = history_items.get();
                                    items.insert(0, new_item);
                                    history_items.set(items);
                                }
                            }
                        }
                        EventPropagation::Continue
                    }
                });

            let clear_button = button("Clear")
                .style(|s| s.padding(8.0).min_width(70.0))
                .on_click({
                    let history_items = history_items_inner.clone();
                    move |_| {
                        history_items.set(vec![]);
                        EventPropagation::Continue
                    }
                });

            h_stack((
                open_button,
                clear_button,
            ))
            .style(|s| {
                s.padding(10.0)
                    .background(Color::rgb(0.95, 0.95, 0.95))  // 浅色背景
                    .gap(10.0)
            })
        }
    });

    // 主内容区域 - 响应式
    let content = dyn_view(move || {
        if document_opened.get() {
            // 文档查看区域 (这里需要实现文档显示)
            container(label(|| "Document View"))
                .style(|s| s.padding(10.0).size(100.pct(), 100.pct()))
        } else {
            // 历史记录
            container(history_grid(history_items))
                .style(|s| s.padding(10.0).size(100.pct(), 100.pct()))
        }
    });

    // 整体布局：垂直堆叠工具栏和内容
    container(v_stack((
        toolbar,
        scroll(content).style(|s| s.size(100.pct(), 100.pct())),
    )))
    .keyboard_navigable()
    .on_resize(move |rect| {
        viewport_size.set((rect.width(), rect.height()));
    })
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
    // 使用响应式网格布局
    let grid = dyn_stack(
        move || {
            let items = history_items.get();
            // 计算行数（每行4个）
            let rows = (items.len() + 3) / 4;
            (0..rows).map(|row_idx| row_idx).collect::<Vec<_>>()
        },
        |row_idx| *row_idx,
        move |row_idx| {
            let items = history_items.get();
            let start_idx = row_idx * 4;

            // 创建当前行的卡片，总是4个，使用empty填充
            let card0 = create_card(items.get(start_idx).cloned());
            let card1 = create_card(items.get(start_idx + 1).cloned());
            let card2 = create_card(items.get(start_idx + 2).cloned());
            let card3 = create_card(items.get(start_idx + 3).cloned());

            h_stack((card0, card1, card2, card3)).style(|s| s.gap(10.0).margin_bottom(10.0))
        }
    );
    
    grid.style(|s| s.padding(10.0))
}

fn create_card(item: Option<HistoryItem>) -> impl IntoView {
    if let Some(item) = item {
        // history_card logic
        // Extract filename from path
        let filename = std::path::Path::new(&item.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        
        // 组合标题和路径
        let full_text = format!("{}", item.title);
        let path_text = filename;
        
        container(v_stack((
            // 缩略图容器
            container(
                // 这里可以替换为实际的缩略图渲染
                label(|| "📄")
                    .style(|s| s.font_size(48.0))
            )
            .style(|s| {
                s.size(160.0, 160.0)
                    .background(Color::rgb(0.94, 0.94, 0.94))
                    .border_radius(4.0)
                    //.justify_content(JustifyContent::Center)
            }),
            
            // 标题（最多两行）
            container(
                label(move || truncate_text(&full_text, 2, 20))
                    .style(|s| {
                        s.font_size(14.0)
                            //.font_weight(FontWeight::Bold)
                            .color(Color::rgb(0.2, 0.2, 0.2))
                            //.text_overflow(TextOverflow::Ellipsis)
                    })
            )
            .style(|s| {
                s.size(160.0, 32.0)
                    .margin_top(8.0)
            }),
            
            // 文件路径
            container(
                label(move || truncate_text(&path_text, 1, 25))
                    .style(|s| {
                        s.font_size(12.0)
                            .color(Color::rgb(0.4, 0.4, 0.4))
                            //.text_overflow(TextOverflow::Ellipsis)
                    })
            )
            .style(|s| {
                s.size(160.0, 16.0)
            }),
        )))
        .style(|s| {
            s.size(180.0, 240.0)
                .padding(10.0)
                .background(Color::rgb(1.0, 1.0, 1.0))
                .border_radius(6.0)
        })
        .on_click(move |_| {
            // 卡片点击事件 - 打开选中的文件
            let path = PathBuf::from(&item.path);
            if path.exists() {
                // 这里添加打开文件的逻辑
                info!("Opening file: {}", item.path);
            }
            EventPropagation::Continue
        })
    } else {
        // empty_card logic
        container(v_stack((
            // 缩略图容器
            container(
                label(|| "")
            )
            .style(|s| {
                s.size(160.0, 160.0)
                    .background(Color::rgb(0.94, 0.94, 0.94))
                    .border_radius(4.0)
            }),
            
            // 标题
            container(
                label(|| "")
            )
            .style(|s| {
                s.size(160.0, 32.0)
                    .margin_top(8.0)
            }),
            
            // 文件路径
            container(
                label(|| "")
            )
            .style(|s| {
                s.size(160.0, 16.0)
            }),
        )))
        .style(|s| {
            s.size(180.0, 240.0)
                .padding(10.0)
                .background(Color::rgb(1.0, 1.0, 1.0))
                .border_radius(6.0)
        })
    }
}

fn truncate_text(text: &str, max_lines: usize, max_chars_per_line: usize) -> String {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_chars = 0;
    
    for c in text.chars() {
        current_line.push(c);
        current_chars += 1;
        
        if current_chars >= max_chars_per_line || c == '\n' {
            lines.push(current_line);
            current_line = String::new();
            current_chars = 0;
            
            if lines.len() >= max_lines {
                break;
            }
        }
    }
    
    if !current_line.is_empty() && lines.len() < max_lines {
        lines.push(current_line);
    }
    
    if lines.len() >= max_lines {
        let last_line = lines.last_mut().unwrap();
        if last_line.len() > 3 {
            *last_line = format!("{}...", last_line[0..last_line.len()-3].to_string());
        }
    }
    
    lines.join("\n")
}
