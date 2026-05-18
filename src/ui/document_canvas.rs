use std::sync::Arc;
use std::sync::atomic::Ordering;

use vello::peniko::Color;
use vello::kurbo::{Affine, Rect, Size, Vec2};
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

pub struct DocumentCanvasWidget {
    state: Arc<PageRenderState>,
    /// 拖拽状态
    is_dragging: bool,
    start_offset: (f32, f32),
    start_pos: (f64, f64),
}

impl DocumentCanvasWidget {
    pub fn new(state: Arc<PageRenderState>) -> Self {
        Self {
            state,
            is_dragging: false,
            start_offset: (0.0, 0.0),
            start_pos: (0.0, 0.0),
        }
    }

    /// 应用 scroll delta 到 offset（含钳位），返回 true 表示有实际变化
    fn apply_scroll(&mut self, dx: f32, dy: f32) -> bool {
        let (old_x, old_y, tw, th, vw, vh) = {
            let r = self.state.read();
            (
                r.view_offset.0,
                r.view_offset.1,
                r.total_width,
                r.total_height,
                r.view_size.0,
                r.view_size.1,
            )
        };
        let new_x = (old_x + dx).clamp(-(tw - vw).max(0.0), 0.0);
        let new_y = (old_y + dy).clamp(-(th - vh).max(0.0), 0.0);
        if (new_x - old_x).abs() < 0.5 && (new_y - old_y).abs() < 0.5 {
            return false;
        }
        self.state.update_offset(new_x, new_y);
        true
    }
}

impl Widget for DocumentCanvasWidget {
    fn on_pointer_event(&mut self, event: &PointerEvent, ctx: &mut EventCtx) -> EventHandling {
        match event {
            // ── 鼠标滚轮 ──
            PointerEvent::PointerScroll { delta, .. } => {
                let x = delta.x as f32;
                let y = delta.y as f32;
                if self.apply_scroll(-x, -y) {
                    ctx.request_paint();
                }
                EventHandling::Handled
            }

            // ── 拖拽平移 ──
            PointerEvent::PointerDown { pos, button, .. } => {
                if *button == masonry::event::PointerButton::Primary {
                    let (ox, oy) = {
                        let r = self.state.read();
                        (r.view_offset.0, r.view_offset.1)
                    };
                    self.is_dragging = true;
                    self.start_offset = (ox, oy);
                    self.start_pos = (pos.x, pos.y);
                    ctx.set_active(true);
                }
                EventHandling::Handled
            }

            PointerEvent::PointerMove { pos, .. } => {
                if self.is_dragging {
                    let dx = (pos.x - self.start_pos.0) as f32;
                    let dy = (pos.y - self.start_pos.1) as f32;
                    // 拖拽：offset 朝手指相反方向移动
                    let new_x = self.start_offset.0 + dx;
                    let new_y = self.start_offset.1 + dy;

                    let (tw, th, vw, vh) = {
                        let r = self.state.read();
                        (r.total_width, r.total_height, r.view_size.0, r.view_size.1)
                    };
                    let clamped_x = new_x.clamp(-(tw - vw).max(0.0), 0.0);
                    let clamped_y = new_y.clamp(-(th - vh).max(0.0), 0.0);

                    self.state.update_offset(clamped_x, clamped_y);
                    ctx.request_paint();
                }
                EventHandling::Handled
            }

            PointerEvent::PointerUp { .. } => {
                self.is_dragging = false;
                EventHandling::Handled
            }

            _ => EventHandling::Handled,
        }
    }

    fn paint(&mut self, ctx: &mut PaintCtx) {
        // 检查后台缓存消费者是否存入了新图片
        if self.state.repaint_needed.swap(false, Ordering::Acquire) {
            ctx.request_paint();
        }

        let inner = self.state.read();
        let scene: &mut Scene = &mut *ctx.scene;
        let scroll = Affine::translate(inner.view_offset.0 as f64, inner.view_offset.1 as f64);

        // 白色背景
        let bg = Rect::new(0.0, 0.0, 2000.0, 1200.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &Color::WHITE, None, &bg);

        // 由每个 Page 自己绘制自己的 node
        for &page_idx in &inner.visible_pages {
            if let Some(page) = inner.pages.get(page_idx) {
                page.draw(scene, scroll, &self.state.cache);
            }
        }
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx, bc: &BoxConstraints) -> Size {
        let (tw, th, vw, vh, zoom) = {
            let r = self.state.read();
            (r.total_width, r.total_height, r.view_size.0, r.view_size.1, r.zoom)
        };
        let desired = Size::new(tw.max(vw) as f64, th.max(vh) as f64);
        let constrained = bc.constrain(desired);

        // 将实际 viewport 尺寸写回 render_state（触发 layout 重算）
        let new_vw = constrained.width.max(1.0) as f32;
        let new_vh = constrained.height.max(1.0) as f32;
        if (new_vw - vw).abs() > 0.5 || (new_vh - vh).abs() > 0.5 {
            self.state.update_view_size(new_vw, new_vh, zoom, false);
        }

        constrained
    }

    fn on_status_change(&mut self, _ctx: &mut LifeCtx, _old: &masonry::Status, _new: &masonry::Status) {}
    fn lifecycle(&mut self, _ctx: &mut LifeCtx, _event: &LifeCycle) {}
    fn update(&mut self, _ctx: &mut UpdateCtx, _event: &UpdateEvent) {}
    fn compute_max_intrinsic(&mut self, _axis: masonry::Axis, _bc: &BoxConstraints, _ctx: &mut LayoutCtx) -> f64 { 0.0 }
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

    fn rebuild(&self, _prev: &Self, _vs: &mut Self::ViewState, _ctx: &mut ViewCtx, _element: &mut Self::Element, _state: &mut AppState) {}

    fn teardown(&self, _vs: &mut Self::ViewState, _ctx: &mut ViewCtx, _element: &mut Self::Element) {}

    fn message(&self, _vs: &mut Self::ViewState, _ctx: &mut MessageContext, _element: &mut Self::Element, _state: &mut AppState) -> MessageResult<()> {
        MessageResult::Nop
    }
}
