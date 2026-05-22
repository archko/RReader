use std::sync::Arc;

use floem::context::PaintCx;
use floem::reactive::RwSignal;
use floem::views::canvas;
use floem::view::IntoView;
use floem::prelude::{SignalGet, SignalUpdate};
use floem::views::Decorators;

use crate::page::PageViewState;

// ============================================================
// create_document_canvas — 创建文档绘制画布
//
// 画布固定为视口大小（不随缩放改变），通过 translate_x/y 样式实现滚动。
// 每个可见 Page 调用自己的 draw() 方法完成绘制：
//   - 缩略图（底色）→ page.draw()
//   - 高精度瓦片节点 → page.draw() 内部遍历 visible_nodes
//   - 链接高亮 → page.draw() 内部遍历 links
// ============================================================

pub fn create_document_canvas(
    page_view_state: Arc<PageViewState>,
    decode_refresh_trigger: RwSignal<u64>,
    doc_info_trigger: RwSignal<u64>,
) -> impl IntoView {
    let state_for_canvas = page_view_state.clone();
    let state_for_style = page_view_state.clone();

    canvas(move |cx: &mut PaintCx, _bounds| {
        let _ = decode_refresh_trigger.get();
        let _ = doc_info_trigger.get();

        let inner = state_for_canvas.read();
        for &idx in &inner.visible_pages {
            inner.pages[idx].draw(cx, &state_for_canvas.cache);
        }
    })
    .style(move |s| {
        let _ = doc_info_trigger.get();
        let inner = state_for_style.read();
        s.translate_x(inner.view_offset.0 as f64)
         .translate_y(inner.view_offset.1 as f64)
         .flex_direction(floem::taffy::FlexDirection::Column)
         .flex_grow(1.0)
    })
}
