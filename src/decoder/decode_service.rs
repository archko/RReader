use anyhow::Result;
use image::DynamicImage;
use std::rc::Rc;

use crate::cache::PageCache;
use crate::decoder::{Decoder, PageInfo};

pub struct DecodeService {
    _decoder: Rc<dyn Decoder>,
    _cache: Rc<PageCache>,
}

impl DecodeService {
    pub fn new(decoder: Rc<dyn Decoder>, cache: Rc<PageCache>) -> Self {
        Self {
            _decoder: decoder,
            _cache: cache,
        }
    }

    pub fn render_full_page(&self, page_info: &PageInfo, crop: i32) -> Result<DynamicImage> {
        self._decoder.render_page(page_info, crop != 0)
        //Ok(DynamicImage::new_rgba8(1, 1))
    }
}

pub struct CropResult {
    pub crop_bounds: (f32, f32, f32, f32),
}