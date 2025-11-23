use anyhow::Result;
use image::DynamicImage;
use std::rc::Rc;
use std::path::Path;
use std::collections::VecDeque;
use std::cell::RefCell;

use crate::cache::PageCache;
use crate::decoder::{Decoder, PageInfo};
use crate::page::{Orientation, PageViewState};
use crate::decoder::pdf::PdfDecoder;

pub enum DecodeRequest {
    RenderFullPage { 
        page_info: PageInfo, 
        crop: i32,
        callback: Box<dyn FnOnce(Result<DynamicImage>)>,
    },
}

pub struct DecodeService {
    decoder: Option<Rc<dyn Decoder>>,
    cache: Rc<PageCache>,
    view_state: Option<PageViewState>,
    zoom: f32,
    orientation: Orientation,
    crop: i32,
    viewport: (f32, f32),
    view_offset: (f32, f32),
    request_queue: RefCell<VecDeque<DecodeRequest>>,
}

impl DecodeService {
    const DEFAULT_VIEW_WIDTH: f32 = 800.0;
    const DEFAULT_VIEW_HEIGHT: f32 = 600.0;

    pub fn new() -> Self {
        Self {
            decoder: None,
            cache: Rc::new(PageCache::new(80, 200)),
            view_state: None,
            zoom: 1.0,
            orientation: Orientation::Vertical,
            crop: 0,
            viewport: (Self::DEFAULT_VIEW_WIDTH, Self::DEFAULT_VIEW_HEIGHT),
            view_offset: (0.0, 0.0),
            request_queue: RefCell::new(VecDeque::new()),
        }
    }

    pub fn load_pdf<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let decoder = Rc::new(PdfDecoder::open(path)?);
        let mut view_state = PageViewState::new(decoder.clone(), self.orientation, self.crop)?;
        if self.viewport.0 > 0.0 && self.viewport.1 > 0.0 {
            view_state.update_view_size(self.viewport.0, self.viewport.1, self.zoom);
        }
        view_state.update_offset(self.view_offset.0, self.view_offset.1);

        self.cache.clear();
        self.view_state = Some(view_state);
        self.decoder = Some(decoder);

        Ok(())
    }

    pub fn page_count(&self) -> usize {
        self.view_state
            .as_ref()
            .map(|state| state.pages.len())
            .unwrap_or(0)
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let clamped = zoom.clamp(0.3, 5.0);
        if (self.zoom - clamped).abs() < f32::EPSILON {
            return;
        }

        self.zoom = clamped;
        self.cache.clear();

        if let Some(view_state) = self.view_state.as_mut() {
            view_state.update_view_size(self.viewport.0, self.viewport.1, self.zoom);
            view_state.update_offset(self.view_offset.0, self.view_offset.1);
        }
    }

    pub fn update_viewport(&mut self, width: f32, height: f32) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }

        self.viewport = (width, height);
        if let Some(view_state) = self.view_state.as_mut() {
            view_state.update_view_size(width, height, self.zoom);
            view_state.update_offset(self.view_offset.0, self.view_offset.1);
        }
    }

    pub fn update_scroll_from_viewport(&mut self, viewport_x: f32, viewport_y: f32) {
        self.view_offset = (-viewport_x, -viewport_y);
        if let Some(view_state) = self.view_state.as_mut() {
            view_state.update_offset(self.view_offset.0, self.view_offset.1);
        }
    }

    pub fn current_viewport_offset(&self) -> (f32, f32) {
        (-self.view_offset.0, -self.view_offset.1)
    }

    pub fn total_size(&self) -> (f32, f32) {
        self.view_state
            .as_ref()
            .map(|state| (state.total_width, state.total_height))
            .unwrap_or((0.0, 0.0))
    }

    pub fn first_visible_page(&self) -> Option<usize> {
        self.view_state
            .as_ref()
            .and_then(|state| state.get_first_visible_page())
    }

    pub fn collect_visible_pages(&mut self) -> Vec<RenderedPage> {
        let mut result = Vec::new();
        let Some(view_state) = self.view_state.as_mut() else {
            return result;
        };
        if self.viewport.0 <= 0.0 || self.viewport.1 <= 0.0 {
            return result;
        }
        let Some(decoder) = self.decoder.as_ref() else {
            return result;
        };

        // Ensure visibility list reflects the latest offset
        view_state.update_offset(self.view_offset.0, self.view_offset.1);

        for &idx in &view_state.visible_pages {
            if let Some(page) = view_state.pages.get(idx) {
                if page.width <= 0.0 || page.height <= 0.0 {
                    continue;
                }
                // 使用队列方式处理页面渲染
                let page_info = page.info.clone();
                let crop = view_state.crop;
                
                // 这里我们简化处理，直接渲染，但在实际应用中应该使用队列
                match decoder.render_page(&page_info, crop != 0) {
                    Ok(image) => {
                        result.push(RenderedPage {
                            index: page.info.index,
                            x: page.bounds.left,
                            y: page.bounds.top,
                            width: page.width,
                            height: page.height,
                            image,
                        });
                    }
                    Err(err) => {
                        eprintln!("Failed to render page {}: {err}", page.info.index);
                    }
                }
            }
        }

        result
    }

    pub fn jump_to_page(&mut self, page_index: usize) -> Option<(f32, f32)> {
        let view_state = self.view_state.as_mut()?;
        let offset = view_state.jump_to_page(page_index)?;
        self.view_offset = offset;
        view_state.update_offset(offset.0, offset.1);
        Some(self.current_viewport_offset())
    }

    // 将解码请求加入队列
    pub fn render_full_page<F>(&self, page_info: PageInfo, crop: i32, callback: F) 
    where 
        F: FnOnce(Result<DynamicImage>) + 'static,
    {
        let request = DecodeRequest::RenderFullPage {
            page_info,
            crop,
            callback: Box::new(callback),
        };
        
        self.request_queue.borrow_mut().push_back(request);
    }

    // 处理队列中的一个解码请求
    pub fn process_next_request(&self) -> bool {
        if let Some(request) = self.request_queue.borrow_mut().pop_front() {
            match request {
                DecodeRequest::RenderFullPage { page_info, crop, callback } => {
                    let result = self.decoder.as_ref()
                        .ok_or_else(|| anyhow::anyhow!("No decoder available"))
                        .and_then(|decoder| {
                            decoder.render_page(&page_info, crop != 0)
                        });
                    callback(result);
                }
            }
            true
        } else {
            false // 队列为空
        }
    }

    // 处理队列中的所有解码请求
    pub fn process_all_requests(&self) {
        while self.process_next_request() {
            // 继续处理直到队列为空
        }
    }
}

pub struct RenderedPage {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub image: DynamicImage,
}

pub struct CropResult {
    pub crop_bounds: (f32, f32, f32, f32),
}