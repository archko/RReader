use xilem::view::{Axis, flex, label, text_button, FlexExt, WidgetView};
use xilem::palette;

use super::home_view::AppState;
use super::document_canvas::DocumentCanvasView;

/// 文档视图
pub fn document_view(state: &mut AppState) -> Box<dyn WidgetView<AppState>> {
    let (path, _title) = match &state.view {
        super::home_view::ViewKind::Document { path, title } => (path.clone(), title.clone()),
        _ => ("".to_string(), "".to_string()),
    };

    // 创建自定义 Canvas View
    let canvas = DocumentCanvasView::new(state.page_render_state.clone());

    // ---- 顶部工具栏 ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("← 返回", |s: &mut AppState| s.back_to_home())
                .background_color(palette::css::DODGER_BLUE)
                .color(palette::css::WHITE),
            label(format!("📂 {}", path))
                .color(palette::css::DIM_GRAY)
                .padding((4.0, 0.0)),
            label("").flex(1.0),
            text_button("方向", |_| log::debug!("切换方向"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("切边", |_| log::debug!("切边"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("AI", |_| log::debug!("AI"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("大纲", |_| log::debug!("大纲"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("书签", |_| log::debug!("书签"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("🔍−", |_| log::debug!("缩小"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
            text_button("🔍+", |_| log::debug!("放大"))
                .background_color(palette::css::LIGHT_SLATE_GRAY)
                .color(palette::css::WHITE),
        ),
    )
    .padding(8.0)
    .background_color(palette::css::LIGHT_STEEL_BLUE);

    // ---- 根布局：工具栏 + 自定义画布 ----
    flex(Axis::Vertical, (toolbar, canvas.flex(1.0))).boxed()
}
