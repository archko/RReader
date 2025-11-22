use image::{DynamicImage, ImageBuffer, Rgba};
use mupdf::{Matrix, Pixmap};

#[derive(Clone)]
pub struct PdfConfig {
    pub zoom: f32,
    pub rotation: f32,
    pub crop_enabled: bool,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            rotation: 0.0,
            crop_enabled: false,
        }
    }
}

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
