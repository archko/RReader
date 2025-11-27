use image::{DynamicImage, ImageBuffer, Rgba};
use mupdf::{Matrix, Pixmap};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::page::Page;

/*#[derive(Clone)]
pub struct PdfConfig {
    pub zoom: f32,
    pub rotation: f32,
    pub crop: i32,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            rotation: 0.0,
            crop: 0,
        }
    }
}*/

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
    //let start_time = Instant::now();
    /*debug!(
        "[STATE] Converting image with dimensions: {}x{}",
        image.width(),
        image.height()
    );*/
    let rgba_image = image.to_rgba8();
    let (width, height) = rgba_image.dimensions();

    let slint_image = Image::from_rgba8_premultiplied(
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba_image, width, height),
    );
    //let duration = start_time.elapsed();
    //info!("[STATE] Successfully converted image to Slint image，耗时: {:?}", duration);
    slint_image
}

pub fn generate_thumbnail_key(page: &Page) -> String {
    format!("{}-{}-{}", page.info.index, page.info.width, page.info.height)
}
