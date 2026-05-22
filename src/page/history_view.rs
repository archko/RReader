use std::path::PathBuf;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use floem::action::exec_after;
use floem::event::EventPropagation;
use floem::peniko::Color;
use floem::prelude::*;
use floem::reactive::Effect;
use floem::style::{NoWrapOverflow, ObjectFit, TextOverflow};
use floem::views::{
    Button, Container, Decorators, Label, Scroll, Stack, img_from_path, VirtualStack,
};
use log::{debug, error, info};
use sea_orm::ActiveValue;

use crate::dao::RecentDao;
use crate::decoder::PageInfo;
use crate::entity::recent::ActiveModel;
use crate::page::PageViewState;
use crate::ui::MainViewmodel;
use crate::ui::utils::get_thumbnail_path;

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
    page_view_state: Arc<PageViewState>,
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

    // ============================================================
    // 虚拟化历史网格（行分组 + VirtualStack + 动态列数）
    // ============================================================

    const CARD_WIDTH: f64 = 180.0;
    const CARD_GAP: f64 = 10.0;
    const CONTAINER_PADDING: f64 = 10.0;

    // 动态列数信号：根据容器实际宽度计算
    let items_per_row: RwSignal<usize> = RwSignal::new(4);

    // 行分组信号：当 history_items 或 items_per_row 变化时重新分组
    let rows_signal: RwSignal<Vec<Vec<HistoryItem>>> = RwSignal::new(Vec::new());
    Effect::new(move |_| {
        let items = history_items.get();
        let cols = items_per_row.get().max(1);
        let new_rows: Vec<Vec<HistoryItem>> = items
            .chunks(cols)
            .map(|c| c.to_vec())
            .collect();
        rows_signal.set(new_rows);
    });

    let grid = VirtualStack::with_view(
        move || rows_signal,
        move |row| {
            Stack::horizontal_from_iter(
                row.into_iter().map(|item| {
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
                }),
            )
            .style(|s| s.gap(CARD_GAP))
        },
    )
    .style(|s| {
        s.flex_col()
            .gap(CARD_GAP)
            .padding(CONTAINER_PADDING)
    });

    // 创建 Scroll，获取其 ViewId 以测量容器宽度
    let mut scroll = grid.scroll();
    let scroll_id = scroll.id();

    // 根据 Scroll 容器宽度计算每行卡片数
    // formula: cols = floor((container_width - 2*padding + gap) / (card_width + gap))
    let recalc_items_per_row = move || {
        let rect = scroll_id.get_content_rect_local();
        let width = rect.width();
        if width > CONTAINER_PADDING * 2.0 {
            let available = width - CONTAINER_PADDING * 2.0;
            let cols = ((available + CARD_GAP) / (CARD_WIDTH + CARD_GAP))
                .floor()
                .max(1.0) as usize;
            if cols != items_per_row.get_untracked() {
                items_per_row.set(cols);
            }
        }
    };

    // 首次布局完成后进行一次测量
    exec_after(Duration::from_millis(50), move |_| {
        recalc_items_per_row();
    });

    // 窗口大小变化时重新测量（延迟执行，等布局计算完成后）
    let scroll = scroll
        .style(|s| s.flex_grow(1.0).min_height(0))
        .on_event_stop(listener::WindowResized, move |_cx, _size| {
            // 推迟到布局更新完成后执行，避免 get_content_rect_local() 返回旧值
            exec_after(Duration::from_millis(0), move |_| {
                recalc_items_per_row();
            });
        });

    Stack::vertical((toolbar, scroll)).style(|s| s.size(100.pct(), 100.pct()))
}

// ============================================================
// create_history_toolbar — 主页工具栏（Open / Clear）
// ============================================================

fn create_history_toolbar(
    page_view_state: Arc<PageViewState>,
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

                    let result = state.open_document(&path);
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
        .style(|s| s.padding(10.0).gap(10.0).background(Color::from_rgb8(255, 255, 255)))
}

// ============================================================
// create_history_card — 单个历史卡片
// ============================================================

pub fn create_history_card(
    item: HistoryItem,
    page_view_state: Arc<PageViewState>,
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

    let thumb_cache_path = get_thumbnail_path(&item_path);
    let has_thumbnail = !thumb_cache_path.is_empty();

    Container::new(
        Stack::vertical((
            // 封面区域：有缩略图则显示，否则显示占位图标
            if has_thumbnail {
                // 直接从缓存路径加载缩略图，固定尺寸并裁剪显示
                let thumb_path = thumb_cache_path.clone();
                Container::new(
                    img_from_path(move || PathBuf::from(thumb_path.clone()))
                        .style(|s| s.size(160, 200).object_fit(ObjectFit::Cover)),
                )
                .style(|s| {
                    s.flex_grow(1.0)
                        .justify_content(floem::taffy::JustifyContent::Center)
                        .align_items(floem::taffy::AlignItems::Center)
                })
            } else {
                Container::new(Label::new("📄").style(|s| s.font_size(48.0)))
                    .style(|s| {
                        s.flex_grow(1.0)
                            .justify_content(floem::taffy::JustifyContent::Center)
                            .align_items(floem::taffy::AlignItems::Center)
                    })
            },
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
        // 推迟到当前事件处理完成后执行，避免嵌套事件循环导致崩溃
        let path = PathBuf::from(&item_path);
        let path_exists = path.exists();
        let item_path_clone = item_path.clone();
        let state = page_view_state.clone();
        let doc_opened = document_opened.clone();
        let fp = file_path.clone();
        let cp = current_page.clone();
        let zl = zoom_level.clone();
        let pc = page_count.clone();
        let vm = viewmodel.clone();

        if path_exists {
            exec_after(Duration::from_millis(0), move |_| {
                info!("从历史记录打开文件: {}", item_path_clone);
                let result = state.open_document(&path);
                if result.is_ok() {
                    poll_document_load(
                        state,
                        doc_opened,
                        fp,
                        cp,
                        zl,
                        pc,
                        vm,
                        item_path_clone,
                    );
                }
            });
        }
        EventPropagation::Continue
    })
}

// ============================================================
// poll_document_load — 回调式轮询文档加载结果
// ============================================================

pub fn poll_document_load(
    state: Arc<PageViewState>,
    document_opened: RwSignal<bool>,
    file_path: RwSignal<String>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    page_count: RwSignal<i32>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
    path_str: String,
) {
    let result = state.decode_service.try_recv_load_result();

    if let Some(result) = result {
        match result {
            Ok(pages) => {
                state.set_pages_from_info(pages);
                // 通过内部 RwLock 读取 Inner 字段
                let (width, height) = {
                    let inner = state.read();
                    (inner.view_size.0, inner.view_size.1)
                };

                state.update_view_size(width, height, 1.0, true);

                page_count.set({
                    let inner = state.read();
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
