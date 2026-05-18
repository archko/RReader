use std::sync::Arc;

use log::debug;
use vello::peniko::{Brush, ImageBrush, ImageData, ImageFormat, Color};
use vello::kurbo::{Affine, Rect, Size};
use vello::Scene;
use vello::Fill;

use masonry::widget::Widget;
use masonry::event::PointerEvent;
use masonry::paint::PaintCtx;
use masonry::{
    BoxConstraints, EventCtx, EventHandling, LayoutCtx, LifeCtx, UpdateCtx, LifeCycle, UpdateEvent,
};

use xilem::core::{Pod, View, ViewCtx, ViewMarker, MessageContext, MessageResult};

use crate::page::render_state::PageRenderState;

// ─────────────────────────────────────────────
// 1. 自定义 Masonry Widget
// ─────────────────────────────────────────────

pub struct DocumentCanvasWidget {
    state: Arc<PageRenderState>,
    /// 标记是否需要重绘
    needs_paint: bool,
}

impl DocumentCanvasWidget {
    pub fn new(state: Arc<PageRenderState>) -> Self {
        Self {
            state,
            needs_paint: true,
        }
    }
}

impl Widget for DocumentCanvasWidget {
    fn on_pointer_event(&mut self, event: &PointerEvent, ctx: &mut EventCtx) -> EventHandling {
        match event {
            PointerEvent::PointerScroll { delta, .. } => {
                let x = delta.x as f32;
                let y = delta.y as f32;

                // 更新 offset（注意 scroll delta 相反方向）
                let (old_x, old_y) = {
                    let r = self.state.read();
                    (r.view_offset.0, r.view_offset.1)
                };
                let new_x = old_x - x;
                let new_y = old_y - y;

                // 钳位
                let (tw, th, vw, vh) = {
                    let r = self.state.read();
                    (r.total_width, r.total_height, r.view_size.0, r.view_size.1)
                };
                let clamped_x = new_x.clamp(-(tw - vw).max(0.0), 0.0);
                let clamped_y = new_y.clamp(-(th - vh).max(0.0), 0.0);

                self.state.update_offset(clamped_x, clamped_y);
                ctx.request_paint();
                EventHandling::Handled
            }
            _ => EventHandling::Handled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintCtx) {
        // 1) 轮询解码结果
        let _updated = self.state.poll_decode_results();
        if _updated {
            ctx.request_paint(); // 有新图像，继续重绘
        }

        // 2) 绘制可见页面
        let inner = self.state.read();
        let scene: &mut Scene = &mut *ctx.scene;
        let (off_x, off_y) = inner.view_offset;
        let scroll = Affine::translate(off_x as f64, off_y as f64);

        // 白色背景
        let bg = Rect::new(0.0, 0.0, 100000.0, 100000.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Color::WHITE,
            None,
            &bg,
        );

        for &page_idx in &inner.visible_pages {
            if let Some(page) = inner.pages.get(page_idx) {
                for node in &page.nodes {
                    let pixel_rect = node.to_pixel_rect(
                        page.width,
                        page.height,
                        page.bounds.left,
                        page.bounds.top,
                    );

                    // 解码器 Rect → kurbo Rect
                    let draw_rect = Rect::new(
                        pixel_rect.left as f64,
                        pixel_rect.top as f64,
                        pixel_rect.right as f64,
                        pixel_rect.bottom as f64,
                    );

                    // 从缓存获取图像并绘制
                    if let Some(img_arc) = self.state.cache.get_page_image_by_key(&node.cache_key) {
                        let rgba = img_arc.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let data: Arc<[u8]> = rgba.into_raw().into();
                        let image_data = ImageData {
                            data,
                            format: ImageFormat::Rgba8,
                            width: w,
                            height: h,
                        };
                        let brush: Brush = ImageBrush::new(image_data).into();
                        scene.fill(Fill::NonZero, scroll, &brush, None, &draw_rect);
                    }
                }
            }
        }

        self.needs_paint = false;
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx, bc: &BoxConstraints) -> Size {
        let r = self.state.read();
        let w = r.total_width.max(r.view_size.0);
        let h = r.total_height.max(r.view_size.1);
        bc.constrain(Size::new(w as f64, h as f64))
    }

    fn on_status_change(&mut self, _ctx: &mut LifeCtx, _old: &masonry::Status, _new: &masonry::Status) {}
    fn lifecycle(&mut self, _ctx: &mut LifeCtx, _event: &LifeCycle) {}
    fn update(&mut self, _ctx: &mut UpdateCtx, _event: &UpdateEvent) {}
    fn compute_max_intrinsic(
        &mut self,
        _axis: masonry::Axis,
        _bc: &BoxConstraints,
        _ctx: &mut LayoutCtx,
    ) -> f64 {
        0.0
    }
}

// ─────────────────────────────────────────────
// 2. Xilem View 包装
// ─────────────────────────────────────────────

pub struct DocumentCanvasView {
    state: Arc<PageRenderState>,
}

impl DocumentCanvasView {
    pub fn new(state: Arc<PageRenderState>) -> Self {
        Self { state }
    }
}

impl ViewMarker for DocumentCanvasView {}

impl<AppState> View<AppState, (), ViewCtx> for DocumentCanvasView {
    type Element = Pod<DocumentCanvasWidget>;
    type ViewState = ();

    fn build(&self, _ctx: &mut ViewCtx, _state: &mut AppState) -> (Self::Element, Self::ViewState) {
        let widget = DocumentCanvasWidget::new(Arc::clone(&self.state));
        (Pod::new(widget), ())
    }

    fn rebuild(
        &self,
        _prev: &Self,
        _vs: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        _element: &mut Self::Element,
        _state: &mut AppState,
    ) {
        // DocumentCanvasWidget 通过 Arc<PageRenderState> 共享状态，无需额外更新
    }

    fn teardown(
        &self,
        _vs: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        _element: &mut Self::Element,
    ) {
    }

    fn message(
        &self,
        _vs: &mut Self::ViewState,
        _ctx: &mut MessageContext,
        _element: &mut Self::Element,
        _state: &mut AppState,
    ) -> MessageResult<()> {
        MessageResult::Nop
    }
}
