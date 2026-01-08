use crate::decoder::pdf::utils::mupdf_to_pixels;
use crate::decoder::{Decoder, Link, LinkType, PageInfo, Rect};
use crate::entity::{ReflowEntry, ReflowData};
use anyhow::Result;
use image::DynamicImage;
use log::{info, debug};
use mupdf::{Colorspace, Context, Device, Document, Matrix, Pixmap};
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
    fn get_def_font_size() -> f32 {
        25.0
    }

    fn generate_font_css(font_path: Option<&str>, margin: &str) -> String {
        let mut buffer = String::new();

        // 1. 全局配置：强制左对齐，这是消除计算量最直接的手段
        buffer.push_str(&format!(
            "    @page {{ margin: {0} !important; }}\n\
                body {{ \
                    padding: {0} !important; \
                    margin: 0 !important; \
                    text-align: left !important; \
                    line-height: 1.5 !important; \
                    orphans: 1 !important; widows: 1 !important; \
                }}\n", margin));

        // 2. 暴力降噪：屏蔽所有导致微调计算的属性
        // word-spacing 和 letter-spacing 在 justify 时会导致 layout 引擎执行数百万次浮点加法
        buffer.push_str("    * {\n");
        buffer.push_str("        text-align: left !important;\n"); 
        buffer.push_str("        word-break: break-all !important;\n"); 
        buffer.push_str("        word-spacing: 0 !important;\n");      
        buffer.push_str("        letter-spacing: 0 !important;\n");    
        buffer.push_str("        box-sizing: border-box !important;\n"); // 简化盒模型计算
        buffer.push_str("    }\n");

        // 3. 针对图片：禁止浮动（Float）
        // MuPDF 处理 float 时需要计算文字环绕路径，这是导致 40 秒卡顿的头号嫌疑犯
        buffer.push_str("    img {\n");
        buffer.push_str("        max-width: 100% !important;\n");
        buffer.push_str("        height: auto !important;\n");
        buffer.push_str("        display: inline-block !important;\n"); 
        buffer.push_str("        float: none !important;\n"); // 强制取消浮动，瞬间降低计算维度
        buffer.push_str("        page-break-inside: avoid !important;\n"); 
        buffer.push_str("    }\n");

        // 4. 块级元素：解决递归边距计算
        // 很多 EPUB 嵌套了 10 层 div，每层都有不同的 margin，MuPDF 算这个非常慢
        buffer.push_str("    p, div, section, article, li {\n");
        buffer.push_str("        display: block !important;\n");
        buffer.push_str("        text-indent: 2em !important;\n");
        buffer.push_str("        margin-top: 0.5em !important;\n"); 
        buffer.push_str("        margin-bottom: 0.5em !important;\n");
        buffer.push_str("        margin-left: 0 !important;\n"); // 屏蔽复杂的层级偏移
        buffer.push_str("        margin-right: 0 !important;\n");
        buffer.push_str("    }\n");

        // 5. 内联元素（公式保护）：保持原有渲染逻辑
        buffer.push_str("    span, b, i, em, strong, a, sub, sup, code {\n");
        buffer.push_str("        display: inline !important;\n"); 
        buffer.push_str("        text-indent: 0 !important;\n");
        buffer.push_str("        white-space: normal !important;\n");
        buffer.push_str("    }\n");

        buffer
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_lowercase();
        info!("Opening document: {:?}", &path_str);
        
        let mut document = Document::open(&path_str)?;
        info!("Document opened");
        if path_str.ends_with(".epub") || path_str.ends_with(".mobi") {
            let css = Self::generate_font_css(None, "20px");
            info!("应用自定义CSS: {}", css);

            let mut ctx = mupdf::Context::get();
            ctx.set_use_document_css(false);  // 禁用文档CSS，只使用用户CSS
            ctx.set_user_css(&css)?;
            ctx.disable_icc();

            let font_size = Self::get_def_font_size();
            let fs = font_size as f32;
            let w = 1280.0;
            let h = 1024.0;
            info!("layout.width:{}, height:{}, font:{}->{}, open:{:?}", w, h, font_size, fs, path.as_ref());

            document.layout(w, h, fs)?;
        }
        let page_count = document.page_count()? as usize;
        info!("Document opened with {} pages", page_count);

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
        debug!("Rendering page {} with crop={}", page.index, crop);
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
        let opts = mupdf::TextPageFlags::empty();
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
