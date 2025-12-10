use crate::decoder::pdf::utils::mupdf_to_pixels;
use crate::decoder::{Decoder, Link, LinkType, PageInfo, Rect};
use crate::entity::{ReflowEntry, ReflowData};
use anyhow::Result;
use image::DynamicImage;
use log::{info, debug};
use mupdf::{Colorspace, Device, Document, Matrix, Pixmap};
use regex::Regex;
use std::cell::RefCell;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub struct PdfDecoder {
    document: RefCell<Document>,
    page_count: usize,
    pages_info: Vec<PageInfo>,
    pdf_path: std::path::PathBuf,
}

impl PdfDecoder {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        info!("[PDF] Opening document: {:?}", path.as_ref());
        let mut document = Document::open(path.as_ref().to_str().unwrap())?;
        let path_str = path.as_ref().to_string_lossy().to_lowercase();
        if path_str.ends_with(".epub") || path_str.ends_with(".mobi") {
            document.layout(1024.0, 1280.0, 25.0)?;
        }
        let page_count = document.page_count()? as usize;
        info!("[PDF] Document opened with {} pages", page_count);

        // 预加载所有页面尺寸
        let mut pages_info = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let page = document.load_page(i as i32)?;
            let bounds = page.bounds()?;
            let width = bounds.x1 - bounds.x0;
            let height = bounds.y1 - bounds.y0;
            pages_info.push(PageInfo::new(i, width, height));
        }

        Ok(Self {
            document: RefCell::new(document),
            page_count,
            pages_info,
            pdf_path: path.as_ref().to_path_buf(),
        })
    }
}

impl PdfDecoder {
    fn get_cache_path(pdf_path: &Path) -> PathBuf {
        use std::path::PathBuf;
        let file_name = pdf_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        dirs::data_dir()
            .expect("Cannot get data directory")
            .join("RReader")
            .join("reflow")
            .join(format!("{}_reflow.json", file_name))
    }

    fn get_or_create_reflow_data(&self, pdf_path: &Path) -> Result<ReflowData> {
        let cache_path = Self::get_cache_path(pdf_path);

        if cache_path.exists() {
            return self.load_reflow_from_cache(&cache_path);
        }

        let page_count = self.page_count();
        let file_size = fs::metadata(pdf_path)?.len();

        let mut reflow = Vec::new();
        for page in 0..page_count {
            let text = self.get_page_text(page)?;
            // 过滤掉字符数 <= 5 的页面
            if text.chars().count() > 5 {
                reflow.push(ReflowEntry {
                    data: text,
                    page: page.to_string(),
                });
            }
        }

        let reflow_data = ReflowData {
            page_count: reflow.len(),
            file_size,
            reflow,
        };

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&reflow_data)?;
        fs::write(&cache_path, json)?;

        Ok(reflow_data)
    }

    fn load_reflow_from_cache(&self, cache_path: &PathBuf) -> Result<ReflowData> {
        let content = fs::read_to_string(cache_path)?;
        let reflow_data: ReflowData = serde_json::from_str(&content)?;
        Ok(reflow_data)
    }
}

impl Decoder for PdfDecoder {
    fn page_count(&self) -> usize {
        self.page_count
    }

    fn get_page_size(&self, index: usize) -> Result<(f32, f32)> {
        if index >= self.pages_info.len() {
            anyhow::bail!("Page index out of bounds");
        }
        let page = &self.pages_info[index];
        Ok((page.width, page.height))
    }

    fn get_all_pages(&self) -> Result<Vec<PageInfo>> {
        Ok(self.pages_info.clone())
    }

    fn render_page(&self, page: &PageInfo, crop: bool) -> Result<(Vec<u8>, u32, u32)> {
        debug!("[PDF] Rendering page {} with crop={}", page.index, crop);
        let document = self.document.borrow();
        let mupdf_page = document.load_page(page.index as i32)?;

        let bounds = if crop && page.crop_bounds.is_some() {
            page.crop_bounds.unwrap()
        } else {
            let b = mupdf_page.bounds()?;
            Rect::new(b.x0, b.y0, b.x1, b.y1)
        };

        let scale = page.scale * 2.0; // DPI scale for retina
        let matrix = Matrix::new(scale, 0.0, 0.0, scale, 0.0, 0.0);

        let width = ((bounds.width()) * scale) as i32;
        let height = ((bounds.height()) * scale) as i32;

        let colorspace = Colorspace::device_rgb();
        let mut pixmap = Pixmap::new(&colorspace, 0, 0, width, height, true)?;
        pixmap.clear()?;

        let mut device = Device::from_pixmap(&pixmap)?;
        mupdf_page.run(&device, &matrix)?;

        Ok(mupdf_to_pixels(&pixmap))
    }

    fn render_region(&self, page_index: usize, region: Rect, scale: f32) -> Result<(Vec<u8>, u32, u32)> {
        let document = self.document.borrow();
        let page = document.load_page(page_index as i32)?;

        let dpi_scale = 2.0;
        let final_scale = scale * dpi_scale;

        // 创建变换矩阵，包含偏移
        let mut matrix = Matrix::new(final_scale, 0.0, 0.0, final_scale, 0.0, 0.0);
        matrix.e = -region.left * final_scale;
        matrix.f = -region.top * final_scale;

        let width = (region.width() * final_scale) as i32;
        let height = (region.height() * final_scale) as i32;

        let colorspace = Colorspace::device_rgb();
        let mut pixmap = Pixmap::new(&colorspace, 0, 0, width, height, true)?;
        pixmap.clear()?;

        let mut device = Device::from_pixmap(&pixmap)?;
        page.run(&device, &matrix)?;

        Ok(mupdf_to_pixels(&pixmap))
    }

    fn get_page_links(&self, page_index: usize) -> Result<Vec<Link>> {
        let document = self.document.borrow();
        let page = document.load_page(page_index as i32)?;
        let links = page.links()?;

        let mut result = Vec::new();
        for link in links {
            let bounds = Rect::new(
                link.bounds.x0,
                link.bounds.y0,
                link.bounds.x1,
                link.bounds.y1,
            );

            let (link_type, uri, page) = if link.uri.starts_with("http") {
                (LinkType::Url, Some(link.uri.clone()), None)
            } else if link.uri.starts_with('#') {
                (LinkType::Page, None, Some(link.uri.clone()))
            } else {
                (LinkType::Unknown, None, None)
            };

            result.push(Link {
                bounds,
                link_type,
                uri,
                page,
            });
        }

        Ok(result)
    }

    fn get_page_text(&self, page_index: usize) -> Result<String> {
        let document = self.document.borrow();
        let page = document.load_page(page_index as i32)?;
        let opts = mupdf::TextPageOptions::empty();
        let text_page = page.to_text_page(opts)?;
        Ok(text_page.to_text()?)
    }

    fn get_outline_items(&self) -> Result<Vec<crate::entity::OutlineItem>> {
        use crate::decoder::pdf::utils::load_outline_items;
        Ok(load_outline_items(&self.document.borrow()))
    }

    fn get_reflow_from_page(&self, start_page: usize) -> Result<Vec<ReflowEntry>> {
        let reflow_data = self.get_or_create_reflow_data(&self.pdf_path)?;

        let start_index = reflow_data.reflow
            .iter()
            .position(|entry| entry.page.parse::<usize>().unwrap_or(0) >= start_page)
            .unwrap_or(reflow_data.reflow.len());

        Ok(reflow_data.reflow[start_index..].to_vec())
    }

    fn close(&mut self) {
        // Document 会在 Drop 时自动关闭
    }
}
