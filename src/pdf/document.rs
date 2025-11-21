use anyhow::{Result, Context};
use mupdf::Document;
use std::path::Path;
use std::sync::{Arc, Mutex};
use crate::pdf::PdfPage;
use crate::pdf::utils::PdfConfig;

pub struct PdfDocument {
    document: Arc<Mutex<Document>>,
    page_count: usize,
    config: PdfConfig,
}

impl PdfDocument {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let document = Document::open(&path_str)
            .with_context(|| format!("Failed to open PDF file: {}", path_str))?;
        
        let page_count = document.page_count()
            .with_context(|| "Failed to get page count")? as usize;
        
        Ok(Self {
            document: Arc::new(Mutex::new(document)),
            page_count,
            config: PdfConfig::default(),
        })
    }
    
    pub fn get_page_count(&self) -> usize {
        self.page_count
    }
    
    pub fn get_page(&self, index: usize) -> Result<PdfPage> {
        if index >= self.page_count {
            return Err(anyhow::anyhow!("Page index {} out of range", index));
        }
        
        let document = self.document.lock().unwrap();
        let page = document.load_page(index as i32)
            .with_context(|| format!("Failed to load page {}", index))?;
        
        Ok(PdfPage::new(page, index, self.config.clone()))
    }
    
    pub fn set_zoom(&mut self, zoom: f32) {
        self.config.zoom = zoom;
    }
    
    pub fn get_zoom(&self) -> f32 {
        self.config.zoom
    }
    
    pub fn set_rotation(&mut self, rotation: f32) {
        self.config.rotation = rotation;
    }
    
    pub fn get_rotation(&self) -> f32 {
        self.config.rotation
    }
    
    pub fn set_crop_enabled(&mut self, enabled: bool) {
        self.config.crop_enabled = enabled;
    }
    
    pub fn is_crop_enabled(&self) -> bool {
        self.config.crop_enabled
    }
    
    pub fn get_metadata(&self) -> Result<Metadata> {
        // mupdf 0.5.0 的 API 可能不支持 get_meta_data
        // 暂时返回空的 metadata
        Ok(Metadata {
            title: None,
            author: None,
            subject: None,
            creator: None,
            producer: None,
            creation_date: None,
            mod_date: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
}