use xilem::masonry::layout::Length;
use xilem::view::{flex, label, sized_box, text_button, FlexExt};
use xilem::WidgetView;
use xilem::masonry::kurbo::Axis;
use xilem::style::Style;

use super::home_view::{AppState, ViewKind};
use super::document_canvas::DocumentCanvasView;
use crate::page::render_state::process_visible_nodes;

/// 文档视图专属的 UI 状态
pub struct DocumentUiState {
    pub outline_visible: bool,
}

impl Default for DocumentUiState {
    fn default() -> Self {
        Self { outline_visible: false }
    }
}

impl AppState {
    pub fn zoom_out(&mut self) {
        let zoom = self.page_render_state.read().zoom;
        self.page_render_state.update_zoom((zoom * 0.8).max(0.1));
        process_visible_nodes(&self.page_render_state);
    }

    pub fn zoom_in(&mut self) {
        let zoom = self.page_render_state.read().zoom;
        self.page_render_state.update_zoom((zoom * 1.25).min(10.0));
        process_visible_nodes(&self.page_render_state);
    }

    pub fn toggle_orientation(&mut self) {
        use crate::page::Orientation;
        let new_ori = match self.page_render_state.read().orientation {
            Orientation::Vertical => Orientation::Horizontal,
            Orientation::Horizontal => Orientation::Vertical,
        };
        self.page_render_state.write().orientation = new_ori;
        let (vw, vh, zoom) = {
            let r = self.page_render_state.read();
            (r.view_size.0, r.view_size.1, r.zoom)
        };
        self.page_render_state.update_view_size(vw, vh, zoom, true);
        process_visible_nodes(&self.page_render_state);
    }

    pub fn toggle_crop(&mut self) {
        let (vw, vh, zoom, crop) = {
            let r = self.page_render_state.read();
            (r.view_size.0, r.view_size.1, r.zoom, r.crop)
        };
        let new_crop = if crop == 1 { 0 } else { 1 };
        self.page_render_state.write().crop = new_crop;
        self.page_render_state.update_view_size(vw, vh, zoom, true);
        process_visible_nodes(&self.page_render_state);
    }

    pub fn prev_page(&mut self) {
        let current = self.page_render_state.read().visible_pages.first().copied().unwrap_or(0);
        if current > 0 {
            self.page_render_state.jump_to_page(current - 1);
            process_visible_nodes(&self.page_render_state);
        }
    }

    pub fn next_page(&mut self) {
        let current = self.page_render_state.read().visible_pages.first().copied().unwrap_or(0);
        self.page_render_state.jump_to_page(current + 1);
        process_visible_nodes(&self.page_render_state);
    }
}

// ── 文档视图 ─────────────────────────────────────

pub fn document_view(state: &mut AppState) -> impl WidgetView<AppState> + use<> {
    let (path, _title) = match &state.view {
        ViewKind::Document { path, title } => (path.clone(), title.clone()),
        _ => ("".to_string(), "".to_string()),
    };

    let canvas = DocumentCanvasView::new(state.page_render_state.clone());

    let page_total = state.page_render_state.read().pages.len();
    let page_current = state.page_render_state.read().visible_pages.first().copied().unwrap_or(0) + 1;

    // ---- 顶部工具栏 ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("←", |s: &mut AppState| s.back_to_home()),
            label(format!("📂 {}", path)),
            label("").flex(1.0),
            text_button("◀", |s: &mut AppState| s.prev_page()),
            label(format!("{}/{}", page_current, page_total)),
            text_button("▶", |s: &mut AppState| s.next_page()),
            text_button("方向", |s: &mut AppState| s.toggle_orientation()),
            text_button("切边", |s: &mut AppState| s.toggle_crop()),
            text_button("🔍−", |s: &mut AppState| s.zoom_out()),
            text_button("🔍+", |s: &mut AppState| s.zoom_in()),
        ),
    )
    .padding(Length::const_px(4.0));

    // ---- 大纲面板 + 主区域 ----
    let outline_items = state.page_render_state.read().outline_items.clone();

    let toggle_btn = text_button("☰", |s: &mut AppState| {
        s.document_ui.outline_visible = !s.document_ui.outline_visible;
    })
    .padding(Length::const_px(4.0));

    let outline_panel = if state.document_ui.outline_visible {
        let items: Vec<_> = outline_items.iter().map(|item| {
            let page = item.page;
            text_button(
                format!("{}{}", "  ".repeat(item.level as usize), item.title),
                move |s: &mut AppState| {
                    s.page_render_state.jump_to_page(page.saturating_sub(1) as usize);
                    process_visible_nodes(&s.page_render_state);
                },
            )
            .padding(Length::const_px(2.0))
            .boxed()
        }).collect();

        let panel = flex(Axis::Vertical, (
            flex(Axis::Horizontal, (
                label("大纲").flex(1.0),
                text_button("✕", |s: &mut AppState| {
                    s.document_ui.outline_visible = false;
                }),
            ))
            .padding(Length::const_px(8.0)),
            flex(Axis::Vertical, items).flex(1.0),
        ));

        sized_box(panel).width(Length::const_px(250.0)).boxed()
    } else {
        sized_box(label("")).width(Length::const_px(0.0)).boxed()
    };

    let main_area = flex(Axis::Horizontal, (
        outline_panel,
        toggle_btn,
        flex(Axis::Vertical, (toolbar, canvas.flex(1.0))).flex(1.0),
    ));

    main_area
}
