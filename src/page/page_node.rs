use xilem::masonry::imaging::Painter;
use xilem::masonry::peniko::{Blob, Brush, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
use xilem::masonry::kurbo::Rect as KurboRect;
use std::sync::Arc;

use crate::decoder::{Rect, PageInfo};
use crate::decoder::decode_service::{DecodeService, RenderPage, TaskType, DecodeCallbackRef};
use crate::cache::PageCache;

pub struct PageNode {
    pub page_index: usize,
    pub bounds: Rect,
    pub cache_key: String,
    pub bitmap: Option<Arc<image::DynamicImage>>,
    pub is_decoding: bool,
    cached_pixel_rect: Option<Rect>,
    cached_page_size: Option<(f32, f32, f32, f32)>,
}

impl PageNode {
    pub fn new(page_index: usize, bounds: Rect, zoom: f32, orientation: i32, crop: i32) -> Self {
        let cache_key = Self::generate_cache_key(page_index, &bounds, zoom, orientation, crop);
        Self {
            page_index,
            bounds,
            cache_key,
            bitmap: None,
            is_decoding: false,
            cached_pixel_rect: None,
            cached_page_size: None,
        }
    }

    fn generate_cache_key(page_index: usize, bounds: &Rect, zoom: f32, orientation: i32, crop: i32) -> String {
        format!(
            "{}_{:.2}_{:.4}_{:.4}_{:.4}_{:.4}_{}_{}",
            page_index, bounds.left, bounds.top, bounds.right, bounds.bottom,
            zoom, orientation, crop
        )
    }

    pub fn update(&mut self, page_index: usize, bounds: Rect, zoom: f32, orientation: i32, crop: i32) {
        self.page_index = page_index;
        self.bounds = bounds;
        self.cache_key = Self::generate_cache_key(page_index, &self.bounds, zoom, orientation, crop);
        self.bitmap = None;
        self.is_decoding = false;
        self.cached_pixel_rect = None;
        self.cached_page_size = None;
    }

    pub fn to_pixel_rect(&mut self, page_width: f32, page_height: f32, x_offset: f32, y_offset: f32) -> Rect {
        if let Some((cw, ch, cx, cy)) = self.cached_page_size {
            if (cw - page_width).abs() < 0.1
                && (ch - page_height).abs() < 0.1
                && (cx - x_offset).abs() < 0.1
                && (cy - y_offset).abs() < 0.1
            {
                return self.cached_pixel_rect.unwrap();
            }
        }
        let rect = Rect::new(
            self.bounds.left * page_width + x_offset,
            self.bounds.top * page_height + y_offset,
            self.bounds.right * page_width + x_offset,
            self.bounds.bottom * page_height + y_offset,
        );
        self.cached_pixel_rect = Some(rect);
        self.cached_page_size = Some((page_width, page_height, x_offset, y_offset));
        rect
    }

    pub fn get_pixel_rect(&self, page_width: f32, page_height: f32, x_offset: f32, y_offset: f32) -> Rect {
        Rect::new(
            self.bounds.left * page_width + x_offset,
            self.bounds.top * page_height + y_offset,
            self.bounds.right * page_width + x_offset,
            self.bounds.bottom * page_height + y_offset,
        )
    }

    /// 绘制节点
    /// scroll_x, scroll_y: 视口在大画布中的位置（visLeft, visTop），正值
    /// page_width, page_height: 当前缩放后的页面尺寸
    /// x_offset, y_offset: 页面在大画布中的位置（currentBounds.left/top）
    pub fn draw(&self, painter: &mut Painter<'_>, scroll_x: f32, scroll_y: f32,
                page_width: f32, page_height: f32, x_offset: f32, y_offset: f32,
                cache: &PageCache,
                vis_left: f32, vis_top: f32, vis_right: f32, vis_bottom: f32) {
        let pixel_rect = self.get_pixel_rect(page_width, page_height, x_offset, y_offset);

        // 检查是否在可见区域内
        if pixel_rect.left > vis_right || pixel_rect.right < vis_left
            || pixel_rect.top > vis_bottom || pixel_rect.bottom < vis_top {
            return;
        }

        // 绘制坐标 = pixel_rect - scroll（视口位置）
        let draw_rect = KurboRect::new(
            (pixel_rect.left - scroll_x) as f64,
            (pixel_rect.top - scroll_y) as f64,
            (pixel_rect.right - scroll_x) as f64,
            (pixel_rect.bottom - scroll_y) as f64,
        );
        let img = self.bitmap.clone().or_else(|| cache.get_page_image_by_key(&self.cache_key));
        if let Some(img_arc) = img {
            let rgba = img_arc.to_rgba8();
            let (w, h) = rgba.dimensions();
            let data = Blob::from(rgba.into_raw());
            let image_data = ImageData { data, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: w, height: h };
            let brush: Brush = ImageBrush::new(image_data).into();
            painter.fill(draw_rect, &brush).draw();
        }
    }

    pub fn decode(&mut self, _page_width: f32, _page_height: f32, _page_info: &PageInfo,
                  _crop: i32, decode_service: &DecodeService,
                  callback: DecodeCallbackRef) {
        if self.is_decoding || self.bitmap.is_some() {
            return;
        }
        decode_service.render_pages(vec![RenderPage {
            key: self.cache_key.clone(),
            page_info: _page_info.clone(),
            crop: _crop,
            task_type: TaskType::Node,
            callback: Some(callback),
        }]);
        self.is_decoding = true;
    }

    pub fn needs_decoding(&self) -> bool {
        self.bitmap.is_none() && !self.is_decoding
    }

    pub fn recycle(&mut self) {
        self.bitmap = None;
        self.is_decoding = false;
        self.cached_pixel_rect = None;
        self.cached_page_size = None;
    }
}
