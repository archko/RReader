use image::{DynamicImage, ImageBuffer, Rgba};
use log::debug;
use mupdf::{Document, Matrix, Outline, Pixmap};
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
    let (pixels, width, height) = mupdf_to_pixels(pixmap);
    let rgba_img = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels).unwrap();
    DynamicImage::ImageRgba8(rgba_img)
}

pub fn mupdf_to_pixels(pixmap: &Pixmap) -> (Vec<u8>, u32, u32) {
    let width = pixmap.width();
    let height = pixmap.height();
    let samples = pixmap.samples();
    let n = pixmap.n() as usize;
    let total_pixels = (width * height) as usize;
    let mut buffer = vec![0u8; total_pixels * 4];

    match n {
        4 => {
            // RGBA: 直接整块拷贝，避免逐像素循环
            let copy_len = total_pixels * 4;
            let valid_len = copy_len.min(samples.len());
            buffer[..valid_len].copy_from_slice(&samples[..valid_len]);
        }
        3 => {
            // RGB -> RGBA: 按行处理，每行连续拷贝
            let w = width as usize;
            for y in 0..height as usize {
                let row_start = y * w;
                let src_off = row_start * 3;
                let dst_off = row_start * 4;
                let src_row = &samples[src_off..src_off + w * 3];
                let dst_row = &mut buffer[dst_off..dst_off + w * 4];
                // 展开循环：3字节RGB + 1字节Alpha
                for i in 0..w {
                    let si = i * 3;
                    let di = i * 4;
                    dst_row[di]     = src_row[si];
                    dst_row[di + 1] = src_row[si + 1];
                    dst_row[di + 2] = src_row[si + 2];
                    dst_row[di + 3] = 255;
                }
            }
        }
        _ => {
            // 灰度或其他，广播到RGBA
            let w = width as usize;
            for y in 0..height as usize {
                for x in 0..w {
                    let src_idx = (y * w + x) * n;
                    let dst_idx = (y * w + x) * 4;
                    let gray = if src_idx < samples.len() { samples[src_idx] } else { 255 };
                    buffer[dst_idx]     = gray;
                    buffer[dst_idx + 1] = gray;
                    buffer[dst_idx + 2] = gray;
                    buffer[dst_idx + 3] = 255;
                }
            }
        }
    }

    (buffer, width, height)
}

pub fn generate_thumbnail_key(page: &Page) -> String {
    format!(
        "{}-{}-{}",
        page.info.index, page.info.width, page.info.height
    )
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

fn process_outline_hierarchy(
    _doc: &Document,
    outlines: &[Outline],
    items: &mut Vec<OutlineItem>,
    level: i32,
) {
    for outline in outlines {
        let title = outline.title.clone();
        let uri = outline.uri.clone();
        let page = if let Some(dest) = &outline.dest {
            dest.loc.page_number as i32
        } else {
            extract_page_from_uri(uri.clone().unwrap_or_default()) as i32
        };
        //debug!("extract_page_from_uri:{:?}, {:?}", page, uri.clone());

        let item = OutlineItem::new(title, uri, page, level);
        //debug!("outline:{:?}, {:?}", level, item.clone());
        items.push(item);

        // Recursively process children with increased level
        let children = &outline.down;
        process_outline_hierarchy(_doc, children, items, level + 1);
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
