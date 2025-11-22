use crate::decoder::pdf::utils::mupdf_to_image;
use crate::decoder::pdf::PdfPage;
use anyhow::Result;
use image::DynamicImage;

pub struct PageRender {
    zoom: f32,
    rotation: f32,
}

impl PageRender {
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            rotation: 0.0,
        }
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.max(0.1).min(5.0); // 限制缩放范围
    }

    pub fn set_rotation(&mut self, rotation: f32) {
        self.rotation = rotation % 360.0;
    }

    pub fn render_page(&self, page: &PdfPage) -> Result<DynamicImage> {
        let pixmap = page.render()?;
        let image = mupdf_to_image(&pixmap);
        Ok(image)
    }

    /// 根据容器尺寸渲染页面
    pub fn render_page_with_size(
        &self,
        page: &PdfPage,
        view_width: f32,
        view_height: f32,
    ) -> Result<DynamicImage> {
        let pixmap = page.render_with_size(Some((view_width, view_height)))?;
        let image = mupdf_to_image(&pixmap);
        Ok(image)
    }

    pub fn render_thumbnail(&self, page: &PdfPage, _max_size: u32) -> Result<DynamicImage> {
        // 直接渲染页面，缩放会在 PdfPage 的配置中处理
        self.render_page(page)
    }

    pub fn render_page_with_overlay(
        &self,
        page: &PdfPage,
        links: &[crate::decoder::Link],
    ) -> Result<DynamicImage> {
        let mut image = self.render_page(page)?;

        // 在图像上绘制链接高亮
        self.draw_links(&mut image, page, links)?;

        Ok(image)
    }

    fn draw_links(
        &self,
        image: &mut DynamicImage,
        page: &PdfPage,
        links: &[crate::decoder::Link],
    ) -> Result<()> {
        use image::Rgba;
        use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut};
        use imageproc::rect::Rect;

        let (page_width, page_height) = page.get_scaled_size();
        let page_bounds = page.get_bounds();

        // 转换为 RgbaImage 以便绘制
        let rgba_image = image.to_rgba8();
        let mut rgba_image = rgba_image;

        for link in links {
            // 转换链接边界到图像坐标
            let x = ((link.bounds.left - page_bounds.x0) / (page_bounds.x1 - page_bounds.x0)
                * page_width) as i32;
            let y = ((link.bounds.top - page_bounds.y0) / (page_bounds.y1 - page_bounds.y0)
                * page_height) as i32;
            let width = ((link.bounds.right - link.bounds.left) / (page_bounds.x1 - page_bounds.x0)
                * page_width) as i32;
            let height = ((link.bounds.bottom - link.bounds.top)
                / (page_bounds.y1 - page_bounds.y0)
                * page_height) as i32;

            if width > 0 && height > 0 {
                let rect = Rect::at(x, y).of_size(width as u32, height as u32);

                // 根据链接类型选择颜色
                let color = match link.link_type {
                    crate::decoder::LinkType::Url => Rgba([51, 110, 229, 102]), // 半透明蓝色
                    crate::decoder::LinkType::Page => Rgba([255, 165, 0, 102]), // 半透明橙色
                    _ => Rgba([128, 128, 128, 102]),                            // 半透明灰色
                };

                draw_filled_rect_mut(&mut rgba_image, rect, color);
                draw_hollow_rect_mut(&mut rgba_image, rect, Rgba([0, 0, 0, 255]));
            }
        }

        *image = DynamicImage::ImageRgba8(rgba_image);

        Ok(())
    }
}

impl Default for PageRender {
    fn default() -> Self {
        Self::new()
    }
}
