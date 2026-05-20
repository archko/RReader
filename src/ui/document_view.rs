use xilem::masonry::layout::Length;
use xilem::view::{flex, label, sized_box, text_button, FlexExt};
use xilem::WidgetView;
use xilem::masonry::kurbo::Axis;
use xilem::palette;
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

    // ---- 顶部工具栏（不含大纲） ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("← 返回", |s: &mut AppState| s.back_to_home()),
            label(format!("📂 {}", path))
                .color(palette::css::DIM_GRAY)
                .padding(Length::const_px(4.0)),
            label("").flex(1.0),
            text_button("◀", |s: &mut AppState| s.prev_page()),
            label(format!("{}/{}", page_current, page_total)),
            text_button("▶", |s: &mut AppState| s.next_page()),
            label("").padding(Length::const_px(4.0)),
            text_button("方向", |s: &mut AppState| s.toggle_orientation()),
            text_button("切边", |s: &mut AppState| s.toggle_crop()),
            text_button("AI", |_| log::debug!("AI")),
            text_button("书签", |_| log::debug!("书签")),
            text_button("🔍−", |s: &mut AppState| s.zoom_out()),
            text_button("🔍+", |s: &mut AppState| s.zoom_in()),
        ),
    )
    .padding(Length::const_px(8.0))
    .background(palette::css::LIGHT_STEEL_BLUE);

    // ---- 大纲面板 + 主区域 ----
    let main_area = if state.document_ui.outline_visible {
        let outline_items = state.page_render_state.read().outline_items.clone();
        let outline_panel_width = 220.0;

        // 左侧大纲面板
        let outline_panel = flex(Axis::Vertical, (
            // 面板标题栏
            flex(Axis::Horizontal, (
                label("大纲").flex(1.0),
                text_button("✕", |s: &mut AppState| {
                    s.document_ui.outline_visible = false;
                }),
            ))
            .padding(Length::const_px(8.0))
            .background(palette::css::LIGHT_STEEL_BLUE),
            // 条目列表
            flex(Axis::Vertical, outline_items.iter().map(|item| {
                label(format!("{}{}", "  ".repeat(item.level as usize), item.title))
                    .padding(Length::const_px(4.0))
            }).collect::<Vec<_>>()).flex(1.0),
        ))
        .background(palette::css::WHITE_SMOKE);

        flex(Axis::Horizontal, (
            sized_box(outline_panel),
            flex(Axis::Vertical, (toolbar, canvas.flex(1.0))).flex(1.0),
        ))
        .boxed()
    } else {
        // 大纲收起：左侧保留一个小按钮用于展开
        let content = flex(Axis::Vertical, (toolbar, canvas.flex(1.0)));
        flex(Axis::Horizontal, (
            // 展开大纲的窄按钮
            text_button("☰", |s: &mut AppState| {
                s.document_ui.outline_visible = true;
            })
            .padding(Length::const_px(4.0)),
            content.flex(1.0),
        ))
        .boxed()
    };

    main_area
}
