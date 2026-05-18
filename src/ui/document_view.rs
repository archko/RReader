use xilem::view::{Axis, flex, label, text_button, FlexExt, WidgetView};
use xilem::palette;

use super::home_view::AppState;

/// 文档视图
pub fn document_view(state: &mut AppState) -> Box<dyn WidgetView<AppState>> {
    let (path, title) = match &state.view {
        super::home_view::ViewKind::Document { path, title } => (path.clone(), title.clone()),
        _ => ("".to_string(), "".to_string()),
    };

    // ---- 顶部标题栏 ----
    let title_bar = flex(
        Axis::Horizontal,
        (
            text_button("← 返回", |s: &mut AppState| s.back_to_home())
                .background_color(palette::css::DODGER_BLUE)
                .color(palette::css::WHITE),
            label(title)
                .color(palette::css::BLACK)
                .flex(1.0),
            text_button("打开文件", |_: &mut AppState| {
                log::debug!("打开文件对话框");
            })
            .background_color(palette::css::DODGER_BLUE)
            .color(palette::css::WHITE),
        ),
    )
    .padding(8.0)
    .background_color(palette::css::LIGHT_STEEL_BLUE);

    // ---- 文档路径 ----
    let path_bar = flex(
        Axis::Horizontal,
        (label(format!("📂 {}", path)).color(palette::css::DIM_GRAY),),
    )
    .padding((8.0, 4.0))
    .background_color(palette::css::WHITE_SMOKE);

    // ---- 文档渲染区域（占位） ----
    let content_area = flex(
        Axis::Vertical,
        (
            label("文档渲染区域")
                .color(palette::css::GRAY)
                .flex(1.0),
        ),
    )
    .flex(1.0);

    // ---- 根布局 ----
    flex(Axis::Vertical, (title_bar, path_bar, content_area)).boxed()
}
