use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use floem::action::exec_after;
use floem::event::EventPropagation;
use floem::peniko::Color;
use floem::prelude::*;
use floem::reactive::Effect;
use floem::style::{NoWrapOverflow, TextOverflow};
use floem::views::{Button, Container, Decorators, DynStack, Label, Scroll, Stack};
use log::{debug, error, info};
use sea_orm::ActiveValue;

use crate::dao::RecentDao;
use crate::decoder::PageInfo;
use crate::entity::recent::ActiveModel;
use crate::page::PageViewState;
use crate::ui::MainViewmodel;

// ============================================================
// HistoryItem — 历史记录条目
// ============================================================

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct HistoryItem {
    pub title: String,
    pub path: String,
    pub page: i32,
    pub page_count: i32,
}

// ============================================================
// HistoryView — 历史记录网格视图
// ============================================================

pub fn create_history_view(
    history_items: RwSignal<Vec<HistoryItem>>,
    page_view_state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
) -> impl IntoView {
    // --- 工具栏 ---
    let toolbar = create_history_toolbar(
        page_view_state.clone(),
        document_opened,
        file_path,
        current_page,
        zoom_level,
        page_count,
        history_items,
        viewmodel.clone(),
    );

    // --- 历史网格 ---
    let grid = dyn_stack(
        move || history_items.get(),
        |item| item.path.clone(),
        move |item| {
            create_history_card(
                item,
                page_view_state.clone(),
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
                viewmodel.clone(),
            )
        },
    )
    .style(|s| {
        s.flex_direction(floem::taffy::FlexDirection::Row)
            .flex_wrap(floem::taffy::FlexWrap::Wrap)
            .gap(10.0)
            .padding(10.0)
    });

    // 注意：Container 不能设 size(100%, 100%)，否则它的布局尺寸被锁定为视口大小，
    // 导致 Scroll 检测 content_size == viewport_size，无法滚动。
    // 让 Container 自然包裹 grid 内容，Scroll 就能正确计算溢出。
    let scroll = Scroll::new(Container::new(grid))
        .style(|s| s.flex_grow(1.0).min_height(0));

    Stack::vertical((toolbar, scroll)).style(|s| s.size(100.pct(), 100.pct()))
}

// ============================================================
// create_history_toolbar — 主页工具栏（Open / Clear）
// ============================================================

fn create_history_toolbar(
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
// create_history_card — 单个历史卡片
// ============================================================

pub fn create_history_card(
    item: HistoryItem,
    page_view_state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
) -> impl IntoView {
    let title_text = item.title.clone();
    let item_path = item.path.clone();
    let page = item.page;
    let total_pages = item.page_count;

    Container::new(
        Stack::vertical((
            // 封面占位区域（flex-grow 撑满剩余空间）
            Container::new(Label::new("📄").style(|s| s.font_size(48.0)))
                .style(|s| {
                    s.flex_grow(1.0)
                        .justify_content(floem::taffy::JustifyContent::Center)
                        .align_items(floem::taffy::AlignItems::Center)
                }),
            // 底部半透明信息叠加层：页数/总页数 + 标题
            Container::new(Stack::vertical((
                Container::new(Label::derived(move || format!("{}/{}", page, total_pages)).style(|s| {
                    s.font_size(11.0)
                        .color(Color::from_rgb8(220, 220, 220))
                        .text_overflow(TextOverflow::NoWrap(NoWrapOverflow::Ellipsis))
                })),
                Container::new(Label::new(title_text).style(|s| {
                    s.font_size(12.0)
                        .color(Color::from_rgb8(255, 255, 255))
                        .text_overflow(TextOverflow::NoWrap(NoWrapOverflow::Ellipsis))
                })),
            )))
            .style(|s| {
                s.background(Color::from_rgba8(0, 0, 0, 160))
                    .padding(6.0)
                    .gap(2.0)
            }),
        ))
        .style(|s| s.width(100.pct())),
    )
    .style(|s| {
        s.size(180.0, 240.0)
            .border_radius(4.0)
            .border(1.0)
            .border_color(Color::from_rgb8(104, 104, 104))
            .hover(|s| s.border_color(Color::from_rgb8(104, 104, 204)))
    })
    .on_event(listener::Click, move |_cx, _event| {
        let path = PathBuf::from(&item_path);
        if path.exists() {
            info!("从历史记录打开文件: {}", item_path);
            let result = page_view_state.borrow_mut().open_document(&path);
            if result.is_ok() {
                let path_str = item_path.clone();
                poll_document_load(
                    page_view_state.clone(),
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
    })
}

// ============================================================
// poll_document_load — 回调式轮询文档加载结果
// ============================================================

pub fn poll_document_load(
    state: Rc<RefCell<PageViewState>>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
    path_str: String,
) {
    let result = {
        let borrowed = state.borrow();
        borrowed.decode_service.try_recv_load_result()
    };

    if let Some(result) = result {
        match result {
            Ok(pages) => {
                state.borrow_mut().set_pages_from_info(pages);
                // 通过内部 RwLock 读取 Inner 字段
                let (width, height) = {
                    let s = state.borrow();
                    let inner = s.read();
                    (inner.view_size.0, inner.view_size.1)
                };

                state.borrow().update_view_size(width, height, 1.0, true);

                page_count.set({
                    let s = state.borrow();
                    let inner = s.read();
                    inner.pages.len() as i32
                });
                document_opened.set(true);
                file_path.set(path_str.clone());
                current_page.set(1);
                zoom_level.set(1.0);

                // 保存到数据库
                save_to_recent(&path_str, &viewmodel);
            }
            Err(e) => {
                error!("加载文档失败: {}", e);
            }
        }
    } else {
        // 继续轮询（回调式：使用 exec_after）
        exec_after(Duration::from_millis(100), move |_| {
            poll_document_load(
                state,
                document_opened,
                file_path,
                current_page,
                zoom_level,
                page_count,
                viewmodel,
                path_str,
            );
        });
    }
}

/// 保存打开记录到数据库
fn save_to_recent(path_str: &str, viewmodel: &Rc<RefCell<MainViewmodel>>) {
    let name = std::path::Path::new(path_str)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let ext = std::path::Path::new(path_str)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let size = match std::path::Path::new(path_str).metadata() {
        Ok(md) => md.len() as i64,
        _ => 0,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let active_model = ActiveModel {
        id: ActiveValue::NotSet,
        book_path: ActiveValue::Set(path_str.to_string()),
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
        error!("保存最近记录失败: {}", e);
    }
}
