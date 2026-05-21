use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tracing::{Span, trace_span};

use xilem::masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, NoAction, PaintCtx,
    PointerButtonEvent, PointerEvent, PointerScrollEvent, PointerUpdate, ScrollDelta,
    PropertiesMut, PropertiesRef, RegisterCtx, UpdateCtx, Widget, WidgetId,
};
use xilem::masonry::dpi;
use xilem::masonry::imaging::Painter;
use xilem::masonry::kurbo::{Axis, Size};
use xilem::masonry::layout::{LenReq, Length};
use xilem::masonry::palette;
use xilem::masonry::accesskit::{Node, Role};
use xilem::core::{MessageCtx, MessageProxy, MessageResult, Mut, View, ViewMarker, ViewId, ViewPathTracker};

use crate::page::render_state::{PageRenderState, process_visible_nodes};

struct DragSample {
    time: std::time::Instant,
    x: f64,
    y: f64,
}

/// 文档画布 Widget - 处理滚动、拖拽、点击等交互
pub struct DocumentCanvasWidget {
    state: Arc<PageRenderState>,
    is_dragging: bool,
    is_flinging: bool,
    fling_velocity_x: f32,
    fling_velocity_y: f32,
    start_offset: (f32, f32),
    start_pos: (f64, f64),
    pointer_down_pos: (f64, f64),
    pointer_down_time: std::time::Instant,
    drag_samples: VecDeque<DragSample>,
    size: Size,
}

impl DocumentCanvasWidget {
    pub fn new(state: Arc<PageRenderState>) -> Self {
        Self {
            state,
            is_dragging: false,
            is_flinging: false,
            fling_velocity_x: 0.0,
            fling_velocity_y: 0.0,
            start_offset: (0.0, 0.0),
            start_pos: (0.0, 0.0),
            pointer_down_pos: (0.0, 0.0),
            pointer_down_time: std::time::Instant::now(),
            drag_samples: VecDeque::new(),
            size: Size::ZERO,
        }
    }

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
        process_visible_nodes(&self.state);
        true
    }

    fn handle_click(&self, pos: dpi::PhysicalPosition<f64>) {
        let inner = self.state.read();
        let doc_x = pos.x as f32 - inner.view_offset.0;
        let doc_y = pos.y as f32 - inner.view_offset.1;

        for &page_idx in &inner.visible_pages {
            if let Some(page) = inner.pages.get(page_idx) {
                if let Some(link) = page.find_link_at(doc_x, doc_y) {
                    if let Some(ref target_page) = link.page {
                        if let Ok(page_num) = target_page.parse::<usize>() {
                            drop(inner);
                            self.state.jump_to_page(page_num);
                            process_visible_nodes(&self.state);
                        }
                        return;
                    } else if let Some(ref _uri) = link.uri {
                        return;
                    }
                }
            }
        }
    }
}

impl Widget for DocumentCanvasWidget {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Scroll(PointerScrollEvent { delta, .. }) => {
                let (x, y) = match delta {
                    ScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                    ScrollDelta::LineDelta(x, y) => (*x, *y),
                    ScrollDelta::PageDelta(x, y) => (*x, *y),
                };
                if self.apply_scroll(-x, -y) {
                    ctx.request_render();
                }
            }

            PointerEvent::Down(PointerButtonEvent { button, state, .. }) => {
                if *button == Some(xilem::masonry::core::PointerButton::Primary) {
                    let pos = state.position;
                    let (ox, oy) = {
                        let r = self.state.read();
                        (r.view_offset.0, r.view_offset.1)
                    };
                    self.is_flinging = false;
                    self.is_dragging = true;
                    self.drag_samples.clear();
                    self.start_offset = (ox, oy);
                    self.start_pos = (pos.x, pos.y);
                    self.pointer_down_pos = (pos.x, pos.y);
                    self.pointer_down_time = std::time::Instant::now();
                    ctx.capture_pointer();
                }
            }

            PointerEvent::Move(PointerUpdate { current: state, .. }) => {
                let pos = state.position;
                if self.is_dragging {
                    self.drag_samples.push_back(DragSample {
                        time: std::time::Instant::now(),
                        x: pos.x,
                        y: pos.y,
                    });
                    while self.drag_samples.len() > 5 {
                        self.drag_samples.pop_front();
                    }

                    let dx = (pos.x - self.start_pos.0) as f32;
                    let dy = (pos.y - self.start_pos.1) as f32;
                    let new_x = self.start_offset.0 + dx;
                    let new_y = self.start_offset.1 + dy;

                    let (tw, th, vw, vh) = {
                        let r = self.state.read();
                        (r.total_width, r.total_height, r.view_size.0, r.view_size.1)
                    };
                    let clamped_x = new_x.clamp(-(tw - vw).max(0.0), 0.0);
                    let clamped_y = new_y.clamp(-(th - vh).max(0.0), 0.0);

                    self.state.update_offset(clamped_x, clamped_y);
                    process_visible_nodes(&self.state);
                    ctx.request_render();
                }
            }

            PointerEvent::Up(PointerButtonEvent { state, .. }) => {
                let pos = state.position;
                let was_dragging = self.is_dragging;
                self.is_dragging = false;

                if was_dragging {
                    // compute fling velocity from drag samples
                    if self.drag_samples.len() >= 2 {
                        let first = self.drag_samples.front().unwrap();
                        let last = self.drag_samples.back().unwrap();
                        let dt = last.time.duration_since(first.time).as_secs_f64();
                        if dt > 0.02 {
                            self.fling_velocity_x = ((last.x - first.x) / dt) as f32;
                            self.fling_velocity_y = ((last.y - first.y) / dt) as f32;
                            let speed = (self.fling_velocity_x.powi(2)
                                + self.fling_velocity_y.powi(2))
                            .sqrt();
                            if speed > 80.0 {
                                self.is_flinging = true;
                            }
                        }
                    }
                    self.drag_samples.clear();

                    let dist = ((pos.x - self.pointer_down_pos.0).powi(2)
                        + (pos.y - self.pointer_down_pos.1).powi(2))
                    .sqrt();
                    let elapsed = self.pointer_down_time.elapsed();

                    if dist < 10.0 && elapsed < std::time::Duration::from_millis(500) {
                        self.is_flinging = false;
                        self.handle_click(pos);
                        ctx.request_render();
                    }
                }
            }

            _ => {}
        }
    }

    fn on_anim_frame(
        &mut self,
        ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        interval: u64,
    ) {
        let mut needs_anim = false;

        if self.is_flinging {
            let dt = (interval as f32).min(50_000.0) / 1_000_000.0;
            let decay = (-2.0 * dt).exp();
            self.fling_velocity_x *= decay;
            self.fling_velocity_y *= decay;

            let dx = self.fling_velocity_x * dt;
            let dy = self.fling_velocity_y * dt;

            if dx.abs() < 0.5 && dy.abs() < 0.5 {
                self.is_flinging = false;
            } else if self.apply_scroll(dx, dy) {
                ctx.request_render();
                needs_anim = true;
            } else {
                // 继续减速直到停止
                needs_anim = true;
            }
        }

        // 解码完成需要重绘
        if self.state.repaint_needed.swap(false, Ordering::Acquire) {
            ctx.request_render();
        }

        // 仅在有动画（fling）时才请求下一帧
        if needs_anim {
            ctx.request_anim_frame();
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        // 使用所有可用空间
        match len_req {
            LenReq::FitContent(space) => space,
            _ => Length::const_px(100.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let old_size = self.size;
        self.size = size;

        if old_size != size {
            let (tw, th, zoom) = {
                let r = self.state.read();
                (r.total_width, r.total_height, r.zoom)
            };
            let new_vw = size.width.max(1.0) as f32;
            let new_vh = size.height.max(1.0) as f32;
            self.state.update_view_size(new_vw, new_vh, zoom, false);
            process_visible_nodes(&self.state);
        }

        // 设置裁剪区域
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        // repaint_needed 由解码回调设置，级联刷新
        if self.state.repaint_needed.swap(false, Ordering::Acquire) {
            // 通过 ctx 请求重绘 - PaintCtx 没有 request_render，但我们可以通过其他方式
        }

        let inner = self.state.read();

        // 绘制白色背景
        painter.fill_rect(
            xilem::masonry::kurbo::Rect::new(0.0, 0.0, self.size.width, self.size.height),
            palette::css::WHITE,
        );

        // view_offset: 负值，表示视口相对于大画布左上角的偏移
        // 例如：view_offset.y = -100 表示视口向下滚动了 100 像素
        // 可见区域计算（参考 kreader）：
        // visLeft = -offset.x, visTop = -offset.y
        // visRight = viewSize.width - offset.x
        // visBottom = viewSize.height - offset.y
        let offset_x = inner.view_offset.0;  // 负值
        let offset_y = inner.view_offset.1;  // 负值

        let vis_left = -offset_x;  // 正值，表示可见区域在大画布中的左边界
        let vis_top = -offset_y;   // 正值，表示可见区域在大画布中的上边界
        let vis_right = inner.view_size.0 - offset_x;
        let vis_bottom = inner.view_size.1 - offset_y;

        let current_zoom = inner.zoom;
        let crop = inner.crop;

        // 绘制可见页面
        for &page_idx in &inner.visible_pages {
            if let Some(page) = inner.pages.get(page_idx) {
                // 传递负的 offset，因为 draw 中需要 scroll = -offset
                page.draw(
                    painter,
                    -offset_x,
                    -offset_y,
                    &self.state.cache,
                    current_zoom,
                    crop,
                    vis_left,
                    vis_top,
                    vis_right,
                    vis_bottom,
                );
                page.draw_links(painter, -offset_x, -offset_y, 1.0);
            }
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Canvas
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn make_trace_span(&self, widget_id: WidgetId) -> Span {
        trace_span!("DocumentCanvas", id = widget_id.trace())
    }
}

/// DocumentCanvas 的 View 实现
pub struct DocumentCanvasView {
    state: Arc<PageRenderState>,
}

impl DocumentCanvasView {
    pub fn new(state: Arc<PageRenderState>) -> Self {
        Self { state }
    }
}

impl ViewMarker for DocumentCanvasView {}

impl<AppState> View<AppState, (), xilem::ViewCtx> for DocumentCanvasView
where
    AppState: 'static,
{
    type Element = xilem::Pod<DocumentCanvasWidget>;
    type ViewState = ();

    fn build(
        &self,
        ctx: &mut xilem::ViewCtx,
        _state: &mut AppState,
    ) -> (Self::Element, Self::ViewState) {
        let proxy = ctx.proxy();
        let path: Arc<[ViewId]> = ctx.view_path().into();
        let msg_proxy = MessageProxy::<()>::new(proxy, path);
        let _ = msg_proxy.message(());

        let widget = DocumentCanvasWidget::new(Arc::clone(&self.state));
        (xilem::Pod::new(widget), ())
    }

    fn rebuild(
        &self,
        _prev: &Self,
        _vs: &mut Self::ViewState,
        _ctx: &mut xilem::ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _state: &mut AppState,
    ) {
        // 重建时不需要无条件请求动画帧
        // 由 on_anim_frame 按需请求
    }

    fn teardown(
        &self,
        _vs: &mut Self::ViewState,
        _ctx: &mut xilem::ViewCtx,
        _element: Mut<'_, Self::Element>,
    ) {
    }

    fn message(
        &self,
        _vs: &mut Self::ViewState,
        _message: &mut MessageCtx,
        mut element: Mut<'_, Self::Element>,
        _app_state: &mut AppState,
    ) -> MessageResult<()> {
        // 不无条件请求动画帧，由 widget 的 on_anim_frame 按需处理
        MessageResult::Nop
    }
}
