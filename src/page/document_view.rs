use std::sync::Arc;

use floem::event::EventPropagation;
use floem::peniko::kurbo::Vec2;
use floem::prelude::*;
use floem::reactive::Effect;
use floem::style::{NoWrapOverflow, TextOverflow};
use floem::views::scroll::ScrollChanged;
use floem::views::{Button, Container, Decorators, Label, Scroll, Stack};
use log::info;

use crate::page::PageViewState;
use crate::page::document_canvas::{create_document_canvas, start_repaint_loop};

pub struct DocumentViewData {
    pub page_view_state: Arc<PageViewState>,
    pub document_opened: RwSignal<bool>,
    pub current_page: RwSignal<i32>,
    pub page_count: RwSignal<i32>,
    pub zoom_level: RwSignal<f32>,
    pub file_path: RwSignal<String>,
    pub viewport_size: RwSignal<(f64, f64)>,
    pub decode_refresh_trigger: RwSignal<u64>,
    pub doc_info_trigger: RwSignal<u64>,
}

// ============================================================
// create_document_view — 文档视图（工具栏 + 滚动画布）
//
// 回调式解码流程：
//   1. 解码线程 PageCallback::on_completed() 直接写入 cache
//   2. start_repaint_loop 每 50ms 检查 repaint_needed 标志
//   3. 发现标志为 true → 递增 decode_refresh_trigger
//   4. Canvas 的闭包追踪该信号 → 从 cache 读取最新图片 → 重绘
//   5. 无需轮询 try_recv_result()
// ============================================================

pub fn create_document_view(data: DocumentViewData) -> impl IntoView {
    let DocumentViewData {
        page_view_state,
        document_opened,
        current_page,
        zoom_level,
        file_path,
        page_count,
        viewport_size,
        decode_refresh_trigger,
        doc_info_trigger,
    } = data;

    // 启动回调式刷新循环
    start_repaint_loop(page_view_state.clone(), decode_refresh_trigger);

    // --- 工具栏 ---
    let toolbar = create_document_toolbar(
        page_view_state.clone(),
        document_opened,
        current_page,
        zoom_level,
        file_path,
        page_count,
        decode_refresh_trigger,
    );

    // --- 视口大小变化监听 ---
    let state_for_resize = page_view_state.clone();
    let trigger_for_resize = decode_refresh_trigger.clone();
    Effect::new(move |_| {
        let (width, height) = viewport_size.get();
        if width > 0.0 && height > 0.0 {
            let zoom = {
                let inner = state_for_resize.read();
                inner.zoom
            };
            state_for_resize.update_view_size(width as f32, height as f32, zoom, false);
            state_for_resize.process_visible_nodes();
            page_count.set(state_for_resize.read().pages.len() as i32);
            trigger_for_resize.update(|v| *v += 1);
        }
    });

    // --- Canvas 画布 ---
    let doc_canvas = create_document_canvas(
        page_view_state.clone(),
        decode_refresh_trigger,
        doc_info_trigger,
    );

    let canvas_container = Container::new(doc_canvas).style(|s| s.padding(20.0));

    // --- 滚动容器 ---
    // 使用 ScrollChanged 自定义事件监听滚动位置变化
    // 注意：仅画布容器可滚动，工具栏固定在顶部（使用 flex_grow）
    let state_for_scroll = page_view_state.clone();
    let trigger_for_scroll = decode_refresh_trigger.clone();
    let cp = current_page.clone();
    let scrolled = Scroll::new(canvas_container)
        .on_event_stop(ScrollChanged::listener(), move |_cx, event: &ScrollChanged| {
            let offset = event.offset;
            let offset_x = -offset.x as f32;
            let offset_y = -offset.y as f32;

            state_for_scroll.update_offset(offset_x, offset_y);
            state_for_scroll.process_visible_nodes();

            // 更新当前页码信号
            if let Some(first_visible) = state_for_scroll.get_first_visible_page() {
                cp.set((first_visible + 1) as i32);
            }

            info!("[Scroll] offset=({:.0},{:.0})", offset_x, offset_y);

            // 触发 Canvas 重绘（Canvas 会从 cache 读取最新图像）
            trigger_for_scroll.update(|v| *v += 1);
        })
        .style(|s| s.flex_grow(1.0).min_height(0));

    Stack::vertical((toolbar, scrolled)).style(|s| s.size(100.pct(), 100.pct()))
}

// ============================================================
// create_document_toolbar — 文档模式的工具栏
// ============================================================

fn create_document_toolbar(
    page_view_state: Arc<PageViewState>,
    document_opened: RwSignal<bool>,
    current_page: RwSignal<i32>,
    zoom_level: RwSignal<f32>,
    file_path: RwSignal<String>,
    page_count: RwSignal<i32>,
    decode_refresh_trigger: RwSignal<u64>,
) -> impl IntoView {
    let state = page_view_state.clone();

    let back_button = Button::new("← Back")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let state = state.clone();
            let document_opened = document_opened.clone();
            move |_cx, _event| {
                state.shutdown();
                document_opened.set(false);
                EventPropagation::Continue
            }
        });

    let prev_button = Button::new("◀ Prev")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let state = state.clone();
            let current_page = current_page.clone();
            let trigger = decode_refresh_trigger.clone();
            move |_cx, _event| {
                let new_page = (current_page.get() as usize).saturating_sub(1);
                if new_page > 0 {
                    current_page.set(new_page as i32);
                    let _ = state.jump_to_page(new_page.saturating_sub(1));
                    trigger.update(|v| *v += 1);
                }
                EventPropagation::Continue
            }
        });

    let next_button = Button::new("Next ▶")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let state = state.clone();
            let current_page = current_page.clone();
            let page_count = page_count.clone();
            let trigger = decode_refresh_trigger.clone();
            move |_cx, _event| {
                let new_page = current_page.get() as usize + 1;
                let max_pages = page_count.get() as usize;
                if new_page <= max_pages {
                    current_page.set(new_page as i32);
                    let _ = state.jump_to_page(new_page.saturating_sub(1));
                    trigger.update(|v| *v += 1);
                }
                EventPropagation::Continue
            }
        });

    let zoom_in_button = Button::new("Zoom +")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let zoom_level = zoom_level.clone();
            let state = state.clone();
            let trigger = decode_refresh_trigger.clone();
            move |_cx, _event| {
                let new_zoom = (zoom_level.get() + 0.1).min(4.0);
                zoom_level.set(new_zoom);
                state.update_zoom(new_zoom);
                trigger.update(|v| *v += 1);
                EventPropagation::Continue
            }
        });

    let zoom_out_button = Button::new("Zoom -")
        .style(|s| s.padding(8.0).min_width(70.0))
        .on_event(listener::Click, {
            let zoom_level = zoom_level.clone();
            let state = state.clone();
            let trigger = decode_refresh_trigger.clone();
            move |_cx, _event| {
                let new_zoom = (zoom_level.get() - 0.1).max(0.5);
                zoom_level.set(new_zoom);
                state.update_zoom(new_zoom);
                trigger.update(|v| *v += 1);
                EventPropagation::Continue
            }
        });

    Stack::horizontal((
        back_button,
        Label::derived(move || {
            format!(
                "Page {} / {} | Zoom: {:.1}%",
                current_page.get(),
                page_count.get(),
                zoom_level.get() * 100.0
            )
        })
        .style(|s| s.padding_right(8.0)),
        Container::new(Label::derived(move || file_path.get()))
            .style(|s| {
                s.flex_grow(1.0)
                    .text_overflow(TextOverflow::NoWrap(NoWrapOverflow::Ellipsis))
            }),
        prev_button,
        next_button,
        zoom_out_button,
        zoom_in_button,
    ))
    .style(|s| s.padding(10.0).gap(10.0))
}
