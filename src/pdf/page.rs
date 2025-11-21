use crate::pdf::utils::{create_matrix, PdfConfig};
use anyhow::Result;
use mupdf::{Page, Rect as MuRect, TextPage};

pub struct PdfPage {
    page: Page,
    index: usize,
    config: PdfConfig,
    bounds: Option<MuRect>,
    text_page: Option<TextPage>,
}

impl PdfPage {
    pub fn new(page: Page, index: usize, config: PdfConfig) -> Self {
        Self {
            page,
            index,
            config,
            bounds: None,
            text_page: None,
        }
    }

    pub fn get_index(&self) -> usize {
        self.index
    }

    pub fn get_width(&self) -> f32 {
        let bounds = self.page.bounds().unwrap_or(MuRect {
            x0: 0.0,
            y0: 0.0,
            x1: 595.0,
            y1: 842.0,
        });
        bounds.x1 - bounds.x0
    }

    pub fn get_height(&self) -> f32 {
        let bounds = self.page.bounds().unwrap_or(MuRect {
            x0: 0.0,
            y0: 0.0,
            x1: 595.0,
            y1: 842.0,
        });
        bounds.y1 - bounds.y0
    }

    pub fn get_bounds(&self) -> MuRect {
        self.page.bounds().unwrap_or(MuRect {
            x0: 0.0,
            y0: 0.0,
            x1: 595.0,
            y1: 842.0,
        })
    }

    pub fn get_scaled_size(&self) -> (f32, f32) {
        let width = self.get_width() * self.config.zoom;
        let height = self.get_height() * self.config.zoom;
        (width, height)
    }

    pub fn render(&self) -> Result<mupdf::Pixmap> {
        self.render_with_size(None)
    }

    /// 根据容器尺寸渲染页面
    /// view_width: 容器宽度，如果提供则根据此宽度自适应缩放
    /// view_height: 容器高度（可选）
    pub fn render_with_size(&self, view_size: Option<(f32, f32)>) -> Result<mupdf::Pixmap> {
        let bounds = self.get_bounds();
        let page_width = bounds.x1 - bounds.x0;
        let page_height = bounds.y1 - bounds.y0;

        // 计算最终的缩放比例
        let final_zoom = if let Some((view_width, _view_height)) = view_size {
            // 根据容器宽度计算缩放比例
            let width_scale = view_width / page_width;
            // 应用用户设置的 zoom
            width_scale * self.config.zoom
        } else {
            // 没有提供容器尺寸，使用原始 zoom
            self.config.zoom
        };

        // 使用更高的 DPI 来提高清晰度 (72 DPI * zoom * 2 for retina)
        let dpi_scale = 2.0; // 提高渲染质量
        let zoom = final_zoom * dpi_scale;
        let matrix = create_matrix(zoom, self.config.rotation);

        let width = (page_width * zoom) as i32;
        let height = (page_height * zoom) as i32;

        let colorspace = mupdf::Colorspace::device_rgb();
        let mut pixmap = mupdf::Pixmap::new(&colorspace, 0, 0, width, height, true)?;
        pixmap.clear()?;

        let mut device = mupdf::Device::from_pixmap(&pixmap)?;
        self.page.run(&mut device, &matrix)?;

        Ok(pixmap)
    }

    pub fn get_text_page(&mut self) -> Result<&TextPage> {
        if self.text_page.is_none() {
            let opts = mupdf::TextPageOptions::empty();
            let text_page = self.page.to_text_page(opts)?;
            self.text_page = Some(text_page);
        }

        Ok(self.text_page.as_ref().unwrap())
    }

    pub fn get_text(&mut self) -> Result<String> {
        let text_page = self.get_text_page()?;
        Ok(text_page.to_text()?)
    }

    pub fn get_text_selection(
        &mut self,
        _start_x: f32,
        _start_y: f32,
        _end_x: f32,
        _end_y: f32,
    ) -> Result<String> {
        let text_page = self.get_text_page()?;
        // mupdf 0.5.0 可能不支持 copy_selection，暂时返回全部文本
        Ok(text_page.to_text()?)
    }

    pub fn get_links(&self) -> Result<Vec<PdfLink>> {
        let links = self.page.links()?;
        let mut pdf_links = Vec::new();

        for link in links {
            let uri = link.uri.clone();
            let link_type = if uri.starts_with("http") {
                LinkType::Url
            } else if uri.starts_with('#') {
                LinkType::Internal
            } else {
                LinkType::Unknown
            };

            pdf_links.push(PdfLink {
                bounds: link.bounds,
                uri,
                link_type,
            });
        }

        Ok(pdf_links)
    }

    pub fn has_crop(&self) -> bool {
        // 简化实现，假设没有裁剪
        false
    }

    pub fn get_crop_bounds(&self) -> Option<MuRect> {
        if self.has_crop() {
            Some(self.get_bounds())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct PdfLink {
    pub bounds: MuRect,
    pub uri: String,
    pub link_type: LinkType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkType {
    Url,
    Internal,
    Unknown,
}
