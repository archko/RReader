use image::{DynamicImage, ImageBuffer, Rgba};
use mupdf::{Document, Matrix, Outline, Pixmap};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use regex::Regex;

use crate::{entity::OutlineItem, page::Page};

pub fn create_matrix(zoom: f32, rotation: f32) -> Matrix {
    let mut matrix = Matrix::new(zoom, 0.0, 0.0, zoom, 0.0, 0.0);
    if rotation != 0.0 {
        let rotate_matrix = Matrix::new_rotate(rotation);
        matrix.concat(rotate_matrix);
    }
    matrix
}

pub fn mupdf_to_image(pixmap: &Pixmap) -> DynamicImage {
    let width = pixmap.width() as u32;
    let height = pixmap.height() as u32;
    let samples = pixmap.samples();
    let n = pixmap.n() as usize; // 每个像素的组件数

    let mut img_buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) as usize) * n;
            if idx + n <= samples.len() {
                let pixel = if n == 4 {
                    // RGBA
                    Rgba([
                        samples[idx],
                        samples[idx + 1],
                        samples[idx + 2],
                        samples[idx + 3],
                    ])
                } else if n == 3 {
                    // RGB
                    Rgba([samples[idx], samples[idx + 1], samples[idx + 2], 255])
                } else {
                    // 灰度或其他
                    Rgba([samples[idx], samples[idx], samples[idx], 255])
                };
                img_buffer.put_pixel(x, y, pixel);
            }
        }
    }

    DynamicImage::ImageRgba8(img_buffer)
}

pub fn convert_to_slint_image(image: &image::DynamicImage) -> Image {
    let rgba_image = image.to_rgba8();
    let (width, height) = rgba_image.dimensions();

    let slint_image = Image::from_rgba8_premultiplied(
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba_image, width, height),
    );
    slint_image
}

pub fn generate_thumbnail_key(page: &Page) -> String {
    format!("{}-{}-{}", page.info.index, page.info.width, page.info.height)
}

/// MuPDF outline processing
/// Load document outline items
pub fn load_outline_items(doc: &Document) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    if let Ok(outlines) = doc.outlines() {
        process_outline_hierarchy(doc, &outlines, &mut items, 0);
    }
    items
}

fn process_outline_hierarchy(doc: &Document, outlines: &[Outline], items: &mut Vec<OutlineItem>, level: i32) {
    for outline in outlines {
        let title = outline.title.clone();
        let uri = outline.uri.clone();

        // Extract page from URI, default to 0 if uri is None
        let (page, uri_val) = if let Some(ref uri_str) = uri {
            (extract_page_from_uri(uri_str.clone()), uri_str.clone())
        } else {
            (0, "".to_string())
        };

        let item = OutlineItem::new(title, uri_val, page, level);
        items.push(item);

        // Recursively process children with increased level
        if let children = &outline.down {
            process_outline_hierarchy(doc, &children, items, level + 1);
        }
    }
}

fn extract_page_from_uri(uri: String) -> i32 {
    let pattern = Regex::new(r"#page=(\d+)").unwrap();
    if let Some(captures) = pattern.captures(&uri) {
        if let Some(page_match) = captures.get(1) {
            if let Ok(page) = page_match.as_str().parse::<i32>() {
                // PDFs are typically 1-based, but convert to 0-based for array indexing
                return (page - 1).max(0);
            }
        }
    }

    // Try to parse the whole URI as a page number
    if let Ok(page) = uri.parse::<i32>() {
        return (page - 1).max(0);
    }

    // Default to page 0
    0
}
