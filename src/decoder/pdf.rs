use anyhow::Result;
use image::DynamicImage;
use mupdf::{Colorspace, Device, Document, Matrix, Pixmap};
use std::cell::RefCell;
use std::path::Path;

use super::{Decoder, Link, LinkType, PageInfo, Rect};
use crate::pdf::utils::mupdf_to_image;

pub struct PdfDecoder {
    document: RefCell<Document>,
    page_count: usize,
    pages_info: Vec<PageInfo>,
}

impl PdfDecoder {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        println!("[PDF] Opening document: {:?}", path.as_ref());
        let document = Document::open(path.as_ref().to_str().unwrap())?;
        let page_count = document.page_count()? as usize;
        println!("[PDF] Document opened with {} pages", page_count);

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
        })
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

    fn render_page(&self, page: &PageInfo, crop: bool) -> Result<DynamicImage> {
        println!("[PDF] Rendering page {} with crop={}", page.index, crop);
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
        mupdf_page.run(&mut device, &matrix)?;

        Ok(mupdf_to_image(&pixmap))
    }

    fn render_region(&self, page_index: usize, region: Rect, scale: f32) -> Result<DynamicImage> {
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
        page.run(&mut device, &matrix)?;

        Ok(mupdf_to_image(&pixmap))
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

    fn close(&mut self) {
        // Document 会在 Drop 时自动关闭
    }
}
