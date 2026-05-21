use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use floem::action::exec_after;
use floem::context::PaintCx;
use floem::kurbo::Rect;
use floem::peniko::{Blob, Color, ImageAlphaType, ImageData};
use floem::reactive::RwSignal;
use floem::views::canvas;
use floem::view::IntoView;
use floem::floem_renderer;
use floem::prelude::{SignalGet, SignalUpdate, Renderer};
use floem::views::Decorators;
use log::debug;

use crate::page::PageViewState;
use crate::decoder::pdf::utils::generate_thumbnail_key;

// ============================================================
// RepaintLoop — 基于回调的解码刷新循环
//
// 解码线程通过 PageCallback::on_completed() 直接写入 cache，
// 完成后设置 repaint_needed = true。本循环定期检查该标志，
// 一旦发现为 true 就递增 decode_refresh_trigger 信号，
// 触发 Canvas 重新绘制。
//
// 对比旧方案（定时轮询 try_recv_result()），本方案：
// 1. 解码回调直接写入缓存 — 零拷贝延迟
// 2. 轻量轮询仅检查 AtomicBool — 无锁争用
// 3. Canvas 通过信号驱动重绘 — 与 Floem 响应式模型一致
// ============================================================

pub fn start_repaint_loop(
    page_view_state: Rc<RefCell<PageViewState>>,
    decode_refresh_trigger: RwSignal<u64>,
) {
    fn poll_repaint(state: Rc<RefCell<PageViewState>>, trigger: RwSignal<u64>) {
        let needs_repaint = state.borrow().repaint_needed.load(Ordering::Acquire);

        if needs_repaint {
            state.borrow().repaint_needed.store(false, Ordering::Release);
            debug!("[RepaintLoop] 触发重绘");
            trigger.update(|v| *v += 1);
        }

        exec_after(Duration::from_millis(50), move |_| {
            poll_repaint(state, trigger);
        });
    }

    poll_repaint(page_view_state, decode_refresh_trigger);
}

// ============================================================
// create_document_canvas — 创建文档绘制画布
//
// 参数:
//   - page_view_state: 页面视图状态
//   - decode_refresh_trigger: 解码完成时递增，触发重绘
//   - doc_info_trigger: 文档信息变更时递增，触发样式重算
//
// 绘制逻辑:
//   1. 先绘制缩略图（底色）
//   2. 再绘制高精度瓦片节点（覆盖缩略图）
// ============================================================

pub fn create_document_canvas(
    page_view_state: Rc<RefCell<PageViewState>>,
    decode_refresh_trigger: RwSignal<u64>,
    doc_info_trigger: RwSignal<u64>,
) -> impl IntoView {
    let state_for_canvas = page_view_state.clone();
    let state_for_style = page_view_state.clone();

    canvas(move |cx: &mut PaintCx, _bounds| {
        // 订阅信号 — 当解码完成或文档信息变更时重新执行
        let _ = decode_refresh_trigger.get();
        let _ = doc_info_trigger.get();

        let state = state_for_canvas.borrow();
        // 通过内部 RwLock 读取 Inner 的字段
        let inner = state.read();
        let pages_snapshot: Vec<_> = inner
            .visible_pages
            .iter()
            .filter_map(|&idx| inner.pages.get(idx))
            .map(|page| {
                // 收集需要绘制的信息，避免长期持有 inner 锁
                let thumb_key = generate_thumbnail_key(page);
                let thumb_img = state.cache.get_thumbnail(&thumb_key);
                let nodes: Vec<_> = page
                    .visible_nodes
                    .iter()
                    .filter_map(|(_nk, node)| {
                        node.bitmap.as_ref().map(|bitmap| {
                            let nx = node.bounds.left as f64;
                            let ny = node.bounds.top as f64;
                            let nw = (node.bounds.right - node.bounds.left) as f64;
                            let nh = (node.bounds.bottom - node.bounds.top) as f64;
                            let actual_x = page.bounds.left as f64 + nx * page.width as f64;
                            let actual_y = page.bounds.top as f64 + ny * page.height as f64;
                            let actual_w = nw * page.width as f64;
                            let actual_h = nh * page.height as f64;
                            (
                                bitmap.clone(),
                                actual_x,
                                actual_y,
                                actual_w,
                                actual_h,
                                node.cache_key.clone(),
                            )
                        })
                    })
                    .collect();

                (
                    page.bounds.left as f64,
                    page.bounds.top as f64,
                    page.width as f64,
                    page.height as f64,
                    thumb_img,
                    nodes,
                )
            })
            .collect();
        // 释放 inner 锁和 RefCell 借入
        drop(inner);
        drop(state);

        // --- 执行绘制 ---
        for (bx, by, bw, bh, thumb_img, nodes) in &pages_snapshot {
            if let Some(img) = thumb_img {
                draw_image(cx, img, *bx, *by, *bw, *bh, "");
            } else {
                // 占位背景
                let rect = Rect::from_origin_size((*bx, *by), (*bw, *bh));
                cx.fill(&rect, Color::from_rgb8(240, 240, 240), 0.0);
            }

            // 绘制高精度节点（覆盖在缩略图上）
            for (bitmap, nx, ny, nw, nh, cache_key) in nodes {
                draw_image(cx, bitmap, *nx, *ny, *nw, *nh, cache_key);
            }
        }
    })
    .style(move |s| {
        let _ = doc_info_trigger.get();
        let state = state_for_style.borrow();
        let inner = state.read();
        s.flex_direction(floem::taffy::FlexDirection::Column)
            .width(inner.total_width as f64)
            .height(inner.total_height as f64)
    })
}

// ============================================================
// draw_image — 用 ImageBrush 绘制图片到 Canvas
// ============================================================

fn draw_image(
    cx: &mut PaintCx,
    dynamic_img: &image::DynamicImage,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cache_key: &str,
) {
    let rgba = dynamic_img.to_rgba8();
    let (img_w, img_h) = rgba.dimensions();
    let blob = Blob::new(Arc::new(rgba.into_raw()));

    let image_data = ImageData {
        data: blob,
        format: floem::peniko::ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: img_w,
        height: img_h,
    };

    // 使用 cache_key 的哈希作为 ImageBrush 的标识
    let mut hasher = DefaultHasher::new();
    cache_key.hash(&mut hasher);
    let hash_val = hasher.finish().to_le_bytes();

    let image_brush = floem::peniko::ImageBrush::new(image_data);
    let rect = Rect::from_origin_size((x, y), (w, h));

    cx.draw_img(
        floem_renderer::Img {
            img: image_brush,
            hash: &hash_val,
        },
        rect,
    );
}
