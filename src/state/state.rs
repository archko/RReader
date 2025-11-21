use crate::pdf::{PdfDocument, renderer::PageRenderer};
use crate::cache::PageCache;

pub struct AppState {
    document: Option<PdfDocument>,
    current_page: usize,
    zoom: f32,
    rotation: f32,
    renderer: PageRenderer,
    cache: PageCache,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            document: None,
            current_page: 0,
            zoom: 1.0,
            rotation: 0.0,
            renderer: PageRenderer::new(),
            cache: PageCache::default(),
        }
    }
    
    pub fn load_document(&mut self, document: PdfDocument) {
        self.document = Some(document);
        self.current_page = 0;
        self.cache.clear();
    }
    
    pub fn get_page_count(&self) -> usize {
        self.document.as_ref().map(|doc| doc.get_page_count()).unwrap_or(0)
    }
    
    pub fn get_current_page(&self) -> Option<usize> {
        if self.document.is_some() {
            Some(self.current_page)
        } else {
            None
        }
    }
    
    pub fn get_page(&mut self, page_index: usize) -> Option<slint::Image> {
        if let Some(document) = &mut self.document {
            if page_index >= document.get_page_count() {
                return None;
            }
            
            // 尝试从缓存获取
            if let Some(cached_image) = self.cache.get_page_image(page_index, self.zoom) {
                return Some(convert_to_slint_image(&cached_image));
            }
            
            // 设置文档的 zoom
            document.set_zoom(self.zoom);
            document.set_rotation(self.rotation);
            
            // 渲染新页面
            match document.get_page(page_index) {
                Ok(page) => {
                    match self.renderer.render_page(&page) {
                        Ok(image) => {
                            let slint_image = convert_to_slint_image(&image);
                            self.cache.put_page_image(page_index, self.zoom, image);
                            Some(slint_image)
                        }
                        Err(e) => {
                            eprintln!("Failed to render page {}: {}", page_index, e);
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get page {}: {}", page_index, e);
                    None
                }
            }
        } else {
            None
        }
    }
    
    pub fn get_thumbnail(&mut self, page_index: usize) -> Option<slint::Image> {
        if let Some(document) = &mut self.document {
            if page_index >= document.get_page_count() {
                return None;
            }
            
            // 尝试从缓存获取缩略图
            if let Some(cached_thumb) = self.cache.get_thumbnail(page_index) {
                return Some(convert_to_slint_image(&cached_thumb));
            }
            
            // 渲染新缩略图
            match document.get_page(page_index) {
                Ok(page) => {
                    match self.renderer.render_thumbnail(&page, 150) {
                        Ok(image) => {
                            let slint_image = convert_to_slint_image(&image);
                            self.cache.put_thumbnail(page_index, image);
                            Some(slint_image)
                        }
                        Err(e) => {
                            eprintln!("Failed to render thumbnail for page {}: {}", page_index, e);
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get page {} for thumbnail: {}", page_index, e);
                    None
                }
            }
        } else {
            None
        }
    }
    
    pub fn set_current_page(&mut self, page: usize) {
        if let Some(document) = &self.document {
            if page < document.get_page_count() {
                self.current_page = page;
            }
        }
    }
    
    pub fn set_zoom(&mut self, zoom: f32) {
        let new_zoom = zoom.max(0.1).min(5.0);
        if (self.zoom - new_zoom).abs() > 0.001 {
            self.zoom = new_zoom;
            self.renderer.set_zoom(self.zoom);
            // 清除缓存以强制重新渲染
            self.cache.clear();
        }
    }
    
    pub fn get_zoom(&self) -> f32 {
        self.zoom
    }
    
    pub fn set_rotation(&mut self, rotation: f32) {
        self.rotation = rotation % 360.0;
        self.renderer.set_rotation(self.rotation);
    }
    
    pub fn get_rotation(&self) -> f32 {
        self.rotation
    }
    
    pub fn next_page(&mut self) {
        if let Some(document) = &self.document {
            if self.current_page < document.get_page_count() - 1 {
                self.current_page += 1;
            }
        }
    }
    
    pub fn prev_page(&mut self) {
        if self.current_page > 0 {
            self.current_page -= 1;
        }
    }
    
    pub fn get_current_document(&self) -> Option<&PdfDocument> {
        self.document.as_ref()
    }
}

fn convert_to_slint_image(image: &image::DynamicImage) -> slint::Image {
    let rgba_image = image.to_rgba8();
    let (width, height) = rgba_image.dimensions();
    
    slint::Image::from_rgba8_premultiplied(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &rgba_image,
            width,
            height,
        )
    )
}