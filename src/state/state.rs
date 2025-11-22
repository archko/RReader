use std::path::Path;
use std::rc::Rc;

use anyhow::Result;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::cache::PageCache;
use crate::decoder::pdf::PdfDecoder;
use crate::decoder::Decoder;
use crate::page::{Orientation, PageViewState};
use crate::render::DecodeService;

pub struct RenderedPage {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub image: Image,
}

pub struct AppState {
    decoder: Option<Rc<dyn Decoder>>,
    view_state: Option<PageViewState>,
    decode_service: Option<DecodeService>,
    page_cache: Rc<PageCache>,
    zoom: f32,
    orientation: Orientation,
    crop_enabled: bool,
    viewport: (f32, f32),
    view_offset: (f32, f32),
}

impl AppState {
    const DEFAULT_VIEW_WIDTH: f32 = 800.0;
    const DEFAULT_VIEW_HEIGHT: f32 = 600.0;

    pub fn new() -> Self {
        Self {
            decoder: None,
            view_state: None,
            decode_service: None,
            page_cache: Rc::new(PageCache::new(80, 200)),
            zoom: 1.0,
            orientation: Orientation::Vertical,
            crop_enabled: false,
            viewport: (Self::DEFAULT_VIEW_WIDTH, Self::DEFAULT_VIEW_HEIGHT),
            view_offset: (0.0, 0.0),
        }
    }

    pub fn load_pdf<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let decoder = Rc::new(PdfDecoder::open(path)?);
        let mut view_state =
            PageViewState::new(decoder.clone(), self.orientation, self.crop_enabled)?;
        if self.viewport.0 > 0.0 && self.viewport.1 > 0.0 {
            view_state.update_view_size(self.viewport.0, self.viewport.1, self.zoom);
        }
        view_state.update_offset(self.view_offset.0, self.view_offset.1);

        self.page_cache.clear();
        self.decode_service = Some(DecodeService::new(decoder.clone(), self.page_cache.clone()));
        self.view_state = Some(view_state);
        self.decoder = Some(decoder);

        println!("[STATE] PDF loaded successfully, page count: {}", self.page_count());
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
        self.page_cache.clear();

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
            println!("[STATE] No view state available");
            return result;
        };
        if self.viewport.0 <= 0.0 || self.viewport.1 <= 0.0 {
            println!("[STATE] Invalid viewport size: {:?}", self.viewport);
            return result;
        }
        let Some(service) = self.decode_service.as_ref() else {
            println!("[STATE] No decode service available");
            return result;
        };

        // Ensure visibility list reflects the latest offset
        view_state.update_offset(self.view_offset.0, self.view_offset.1);
        println!("[STATE] Processing {} visible pages", view_state.visible_pages.len());

        for &idx in &view_state.visible_pages {
            println!("[STATE] Processing visible page index: {}", idx);
            if let Some(page) = view_state.pages.get(idx) {
                if page.width <= 0.0 || page.height <= 0.0 {
                    eprintln!("[STATE] Invalid page dimensions for page {}: {}x{}", idx, page.width, page.height);
                    continue;
                }
                match service.render_full_page(&page.info, view_state.crop_enabled) {
                    Ok(image) => {
                        println!("[STATE] Successfully rendered page {}: {}x{}", idx, image.width(), image.height());
                        result.push(RenderedPage {
                            index: page.info.index,
                            x: page.bounds.left,
                            y: page.bounds.top,
                            width: page.width,
                            height: page.height,
                            image: convert_to_slint_image(&image),
                        });
                    }
                    Err(err) => {
                        eprintln!("Failed to render page {}: {err}", page.info.index);
                    }
                }
            } else {
                eprintln!("[STATE] Page {} not found in view state", idx);
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
}

fn convert_to_slint_image(image: &image::DynamicImage) -> Image {
    println!("[STATE] Converting image with dimensions: {}x{}", image.width(), image.height());
    let rgba_image = image.to_rgba8();
    let (width, height) = rgba_image.dimensions();

    let slint_image = Image::from_rgba8_premultiplied(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &rgba_image,
        width,
        height,
    ));
    println!("[STATE] Successfully converted image to Slint image");
    slint_image
}