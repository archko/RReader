use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwapOption;
use floem::peniko::ImageData;
use crate::decoder::{Rect, PageInfo};
use crate::decoder::decode_service::{DecodeService, RenderPage, TaskType, DecodeCallbackRef};
use crate::cache::PageCache;

pub struct PageNode {
    pub page_index: usize,
    pub bounds: Rect,
    pub cache_key: String,
    pub bitmap: ArcSwapOption<ImageData>,
    pub is_decoding: AtomicBool,
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
            bitmap: ArcSwapOption::new(None),
            is_decoding: AtomicBool::new(false),
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
        self.bitmap.store(None);
        self.is_decoding.store(false, Ordering::Release);
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

    pub fn decode(&mut self, _page_width: f32, _page_height: f32, _page_info: &PageInfo,
                  _crop: i32, decode_service: &DecodeService,
                  callback: DecodeCallbackRef) {
        if self.is_decoding.load(Ordering::Acquire) || self.bitmap.load().is_some() {
            return;
        }
        let scale = _page_info.scale;
        let offset_x = if _crop != 0 {
            if let Some(crop) = _page_info.crop_bounds { crop.left * scale } else { 0.0 }
        } else {
            0.0
        };
        let offset_y = if _crop != 0 {
            if let Some(crop) = _page_info.crop_bounds { crop.top * scale } else { 0.0 }
        } else {
            0.0
        };
        let region = Rect::new(
            self.bounds.left * _page_width + offset_x,
            self.bounds.top * _page_height + offset_y,
            self.bounds.right * _page_width + offset_x,
            self.bounds.bottom * _page_height + offset_y,
        );
        decode_service.render_pages(vec![RenderPage {
            key: self.cache_key.clone(),
            page_info: _page_info.clone(),
            crop: _crop,
            task_type: TaskType::Node,
            callback: Some(callback),
            region: Some(region),
        }]);
        self.is_decoding.store(true, Ordering::Release);
    }

    pub fn needs_decoding(&self) -> bool {
        self.bitmap.load().is_none() && !self.is_decoding.load(Ordering::Acquire)
    }

    pub fn recycle(&mut self) {
        self.bitmap.store(None);
        self.is_decoding.store(false, Ordering::Release);
        self.cached_pixel_rect = None;
        self.cached_page_size = None;
    }
}
