#![allow(unused)]
#![allow(dead_code)]

use tokio;
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
use floem::views::Decorators;
use floem::reactive::create_effect;
use floem::views::empty;
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
use crate::entity::Recent;
use sea_orm::ActiveValue;

use std::cell::RefCell;

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

fn app_view(viewmodel: Rc<RefCell<MainViewmodel>>, initial_history: Vec<HistoryItem>) -> impl IntoView {
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
    let state_for_toolbar = page_view_state.clone();
    let toolbar = dyn_view(move || {
        let viewmodel = viewmodel.clone();
        let state = state_for_toolbar.clone();
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
                                current_page.set(0);
                                zoom_level.set(1.0);
                                // save to database
                                let full_path = path.to_string_lossy().to_string();
                                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
                                let size = match path.metadata() {
                                    Ok(md) => md.len() as i64,
                                    _ => 0,
                                };
                                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
                                let active_model = crate::entity::recent::ActiveModel {
                                    id: ActiveValue::NotSet,
                                    book_path: ActiveValue::Set(full_path),
                                    update_at: ActiveValue::Set(now),
                                    create_at: ActiveValue::Set(now),
                                    page: ActiveValue::Set(1),
                                    page_count: ActiveValue::Set(0),
                                    crop: ActiveValue::Set(1),
                                    reflow: ActiveValue::Set(0),
                                    scroll_ori: ActiveValue::Set(1),
                                    zoom: ActiveValue::Set(1.0),
                                    scroll_x: ActiveValue::Set(0),
                                    scroll_y: ActiveValue::Set(0),
                                    name: ActiveValue::Set(name),
                                    ext: ActiveValue::Set(ext),
                                    size: ActiveValue::Set(size),
                                    read_times: ActiveValue::Set(1),
                                    progress: ActiveValue::Set(0),
                                    favorited: ActiveValue::Set(0),
                                    in_recent: ActiveValue::Set(1),
                                };
                                if let Err(e) = viewmodel.borrow().add_recent(active_model) {
                                    error!("Failed to save recent: {}", e);
                                }
                                // update history_items
                                let mut vm_load = MainViewmodel::new();
                                if let Ok(_) = vm_load.load_history(0) {
                                    let updated_items: Vec<_> = vm_load.get_current_records().iter().map(|r| HistoryItem {
                                        title: r.name.clone(),
                                        path: r.book_path.clone(),
                                        page: r.page,
                                    }).collect();
                                    history_items.set(updated_items);
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
                                current_page.set(0);
                                zoom_level.set(1.0);
                                // save to database
                                let full_path = path.to_string_lossy().to_string();
                                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
                                let size = match path.metadata() {
                                    Ok(md) => md.len() as i64,
                                    _ => 0,
                                };
                                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
                                let active_model = crate::entity::recent::ActiveModel {
                                    id: ActiveValue::NotSet,
                                    book_path: ActiveValue::Set(full_path),
                                    update_at: ActiveValue::Set(now),
                                    create_at: ActiveValue::Set(now),
                                    page: ActiveValue::Set(1),
                                    page_count: ActiveValue::Set(0),
                                    crop: ActiveValue::Set(1),
                                    reflow: ActiveValue::Set(0),
                                    scroll_ori: ActiveValue::Set(1),
                                    zoom: ActiveValue::Set(1.0),
                                    scroll_x: ActiveValue::Set(0),
                                    scroll_y: ActiveValue::Set(0),
                                    name: ActiveValue::Set(name),
                                    ext: ActiveValue::Set(ext),
                                    size: ActiveValue::Set(size),
                                    read_times: ActiveValue::Set(1),
                                    progress: ActiveValue::Set(0),
                                    favorited: ActiveValue::Set(0),
                                    in_recent: ActiveValue::Set(1),
                                };
                                if let Err(e) = viewmodel.borrow().add_recent(active_model) {
                                    error!("Failed to save recent: {}", e);
                                }
                                // update history_items
                                let mut vm_load = MainViewmodel::new();
                                if let Ok(_) = vm_load.load_history(0) {
                                    let updated_items: Vec<_> = vm_load.get_current_records().iter().map(|r| HistoryItem {
                                        title: r.name.clone(),
                                        path: r.book_path.clone(),
                                        page: r.page,
                                    }).collect();
                                    history_items.set(updated_items);
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
    let state_for_content = page_view_state.clone();
    let state_for_history = page_view_state.clone();
    let content = dyn_view(move || {
        if document_opened.get() {
            document_view(
                state_for_content.clone(),
                viewport_size,
                current_page,
                page_count,
            ).into_any()
        } else {
            container(history_grid(
                history_items,
                state_for_history.clone(),
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
            ))
                .style(|s| s.padding(10.0).size(100.pct(), 100.pct()))
                .into_any()
        }
    });

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
    std::env::set_var("DATABASE_URL", &database_url);

    if !db_path.exists() {
        crate::dao::ensure_database_ready(&db_path).await?;
    }
    RecentDao::init().await?;
    Ok(())
}

fn main() {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("info")
    ).init();

    // Create Tokio runtime and keep it alive for the entire application
    let runtime = tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime");

    // Enter the runtime context so all subsequent operations have access to it
    let _guard = runtime.enter();

    runtime.block_on(async {
        if let Err(e) = setup_database().await {
            eprintln!("Failed to setup database: {e}");
        }
    });

    let viewmodel = Rc::new(RefCell::new(MainViewmodel::new()));

    {
        let mut vm_borrow = viewmodel.borrow_mut();
        if let Err(_e) = vm_borrow.load_history(0) {
            // Ignore error, use empty
        }
        let initial_history_vec = vm_borrow.get_current_records().iter().map(|r| HistoryItem {
            title: r.name.clone(),
            path: r.book_path.clone(),
            page: r.page,
        }).collect::<Vec<_>>();
        drop(vm_borrow);

        // Launch the floem window with initial data
        // The runtime will be kept alive until the application exits
        floem::launch(move || app_view(viewmodel, initial_history_vec));
    }
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct HistoryItem {
    title: String,
    path: String,
    page: i32,
}

fn history_grid(
    history_items: RwSignal<Vec<HistoryItem>>,
    page_view_state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
) -> impl IntoView {
    // 使用 Flexbox 自动换行的网格布局
    let grid = dyn_stack(
        move || history_items.get(),
        |item| item.path.clone(),
        move |item| {
            create_card(
                Some(item),
                page_view_state.clone(),
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
            )
        }
    )
    .style(|s| {
        s.flex_direction(floem::taffy::FlexDirection::Row)
            .flex_wrap(floem::taffy::FlexWrap::Wrap)
            .gap(10.0)
            .padding(10.0)
    });
    
    grid
}

fn create_card(
    item: Option<HistoryItem>,
    page_view_state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
) -> impl IntoView {
    if let Some(item) = item {
        // history_card logic
        // Extract filename from path
        let filename = std::path::Path::new(&item.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        
        let title_text = item.title.clone();
        let path_text = filename;
        let item_path = item.path.clone();
        
        container(v_stack((
            container(
                label(|| "📄")
                    .style(|s| s.font_size(64.0))
            )
            .style(|s| {
                s.size(160.0, 160.0)
                    .background(Color::rgb(0.94, 0.94, 0.94))
                    .border_radius(4.0)
                    .justify_content(Some(floem::taffy::JustifyContent::Center))
                    .align_items(Some(floem::taffy::AlignItems::Center))
            }),
            
            container(
                label(move || title_text.clone())
                    .style(|s| {
                        s.font_size(14.0)
                            .color(Color::rgb(0.2, 0.2, 0.2))
                            .line_height(1.2)
                            .max_width(160.0)
                            .text_overflow(TextOverflow::Ellipsis)
                    })
            )
            .style(|s| {
                s.width(160.0)
                    .max_height(34.0)  // 限制最大高度为两行 (14px * 1.2 * 2 ≈ 34px)
                    .margin_top(8.0)
            }),
            
            // 文件路径
            container(
                label(move || path_text.clone())
                    .style(|s| {
                        s.font_size(12.0)
                            .color(Color::rgb(0.4, 0.4, 0.4))
                            .max_width(160.0)
                            .text_overflow(TextOverflow::Ellipsis)
                    })
            )
            .style(|s| {
                s.width(160.0)
                    .height(16.0)
            }),
        )))
        .style(|s| {
            s.size(180.0, 240.0)
                .padding(10.0)
                .background(Color::rgb(1.0, 1.0, 1.0))
                .border_radius(6.0)
        })
        .on_click(move |_| {
            let path = PathBuf::from(&item_path);
            if path.exists() {
                info!("Opening file from history: {}", item_path);
                let result = page_view_state.borrow_mut().open_document(&path);
                if result.is_ok() {
                    // 更新页面计数
                    let state = page_view_state.borrow();
                    page_count.set(state.pages.len() as i32);
                    drop(state);
                    
                    document_opened.set(true);
                    file_path.set(item_path.clone());
                    current_page.set(0);
                    zoom_level.set(1.0);
                }
            }
            EventPropagation::Continue
        })
    } else {
        // empty_card logic
        container(v_stack((
            container(label(|| ""))
                .style(|s| {
                    s.size(160.0, 160.0)
                        .background(Color::rgb(0.94, 0.94, 0.94))
                        .border_radius(4.0)
                }),
            container(label(|| ""))
                .style(|s| {
                    s.size(160.0, 32.0)
                        .margin_top(8.0)
                }),
            container(label(|| ""))
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

fn document_view(
    page_view_state: Rc<RefCell<PageViewState>>,
    viewport_size: RwSignal<(f64, f64)>,
    current_page: RwSignal<i32>,
    page_count: RwSignal<i32>,
) -> impl IntoView {
    let rendered_pages = RwSignal::new(Vec::<RenderedPage>::new());
    
    // 监听视口大小变化并更新渲染页面
    let state_for_viewport = page_view_state.clone();
    create_effect(move |_| {
        let (width, height) = viewport_size.get();
        if width > 0.0 && height > 0.0 {
            let mut state = state_for_viewport.borrow_mut();
            let zoom = state.zoom;
            state.update_view_size(width as f32, height as f32, zoom, false);
            state.update_visible_pages();
            page_count.set(state.pages.len() as i32);
            drop(state);
            
            let state = state_for_viewport.borrow();
            update_rendered_pages(&state, &rendered_pages);
        }
    });
    
    let doc_container = dyn_stack(
        move || rendered_pages.get(),
        |page| page.page_index,
        move |page| {
            let img_arc = page.image_data.clone();
            let page_idx = page.page_index;
            
            if let Some(dynamic_img) = img_arc {
                // 将 DynamicImage 转换为 RGBA8 格式
                let rgba = dynamic_img.to_rgba8();
                let img_width = rgba.width();
                let img_height = rgba.height();
                let bytes = rgba.into_raw();
                
                // 创建图像视图
                create_image_view(bytes, img_width, img_height, page.width, page.height).into_any()
            } else {
                container(
                    label(move || format!("Loading page {}...", page_idx + 1))
                        .style(|s| s.font_size(14.0).color(Color::rgb(0.5, 0.5, 0.5)))
                )
                .style(move |s| {
                    s.width(page.width)
                        .height(page.height)
                        .background(Color::rgb(0.95, 0.95, 0.95))
                        .justify_content(Some(floem::taffy::JustifyContent::Center))
                        .align_items(Some(floem::taffy::AlignItems::Center))
                        .border(1.0)
                        .border_color(Color::rgb(0.8, 0.8, 0.8))
                })
                .into_any()
            }
        }
    )
    .style(|s| s.flex_direction(floem::taffy::FlexDirection::Column));
    
    // 创建可滚动容器，并监听滚动事件
    let state_for_scroll = page_view_state.clone();
    let scroll_view = scroll(
        container(doc_container)
            .style(|s| s.background(Color::WHITE).padding(20.0))
    )
    .on_scroll(move |rect| {
        // rect 包含滚动容器的位置信息
        // 滚动偏移量是负值（向下滚动时 y 为负）
        let offset_x = -rect.x0 as f32;
        let offset_y = -rect.y0 as f32;
        
        info!("[Scroll Event] offset: ({}, {})", offset_x, offset_y);
        
        // 更新 PageViewState 的偏移量
        let mut state = state_for_scroll.borrow_mut();
        state.update_offset(offset_x, offset_y);
        state.update_visible_pages();
        
        // 更新当前页码
        if let Some(first_visible) = state.get_first_visible_page() {
            current_page.set((first_visible + 1) as i32);
            info!("[Scroll Event] First visible page: {}", first_visible + 1);
        }

        // 更新渲染页面
        update_rendered_pages(&state, &rendered_pages);
    })
    .style(|s| s.size(100.pct(), 100.pct()).background(Color::rgb(0.9, 0.9, 0.9)));
    
    scroll_view
}

#[derive(Clone, Debug)]
struct RenderedPage {
    page_index: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    image_data: Option<std::sync::Arc<image::DynamicImage>>,
}

fn update_rendered_pages(
    state: &PageViewState,
    rendered_pages: &RwSignal<Vec<RenderedPage>>,
) {
    let mut pages = Vec::new();
    
    info!("[update_rendered_pages] visible_pages: {:?}", state.visible_pages);
    
    for &idx in &state.visible_pages {
        if let Some(page) = state.pages.get(idx) {
            let key = generate_thumbnail_key(page);
            let image_data = state.cache.get_thumbnail(&key);
            
            info!("[update_rendered_pages] Page {}: has_image={}", idx, image_data.is_some());
            
            pages.push(RenderedPage {
                page_index: idx,
                x: page.bounds.left as f64,
                y: page.bounds.top as f64,
                width: page.width as f64,
                height: page.height as f64,
                image_data,
            });
        }
    }
    
    info!("[update_rendered_pages] Total rendered pages: {}", pages.len());
    rendered_pages.set(pages);
}

fn create_image_view(
    bytes: Vec<u8>,
    img_width: u32,
    img_height: u32,
    display_width: f64,
    display_height: f64,
) -> impl IntoView {
    // 将 RGBA 数据编码为 PNG 格式
    use image::{ImageBuffer, Rgba, codecs::png::PngEncoder, ImageEncoder};
    
    let img_buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(img_width, img_height, bytes)
        .expect("Failed to create image buffer");
    
    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder.write_image(
        img_buffer.as_raw(),
        img_width,
        img_height,
        image::ExtendedColorType::Rgba8,
    ).expect("Failed to encode PNG");
    
    // 使用 Floem 的 img 函数显示 PNG 编码的图像
    container(
        img(move || png_bytes.clone())
            .style(move |s| {
                s.width(display_width)
                    .height(display_height)
            })
    )
    .style(move |s| {
        s.width(display_width)
            .height(display_height)
            .border(1.0)
            .border_color(Color::rgb(0.8, 0.8, 0.8))
    })
}
