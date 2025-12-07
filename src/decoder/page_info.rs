use super::Rect;

/// 页面信息
#[derive(Debug, Clone)]
pub struct PageInfo {
    pub index: usize,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub crop_bounds: Option<Rect>,
}

impl PageInfo {
    pub fn new(index: usize, width: f32, height: f32) -> Self {
        Self {
            index,
            width,
            height,
            scale: 1.0,
            crop_bounds: None,
        }
    }

    pub fn get_width(&self, use_crop: bool) -> f32 {
        if use_crop {
            if let Some(crop) = &self.crop_bounds {
                return crop.width();
            }
        }
        self.width
    }

    pub fn get_height(&self, use_crop: bool) -> f32 {
        if use_crop {
            if let Some(crop) = &self.crop_bounds {
                return crop.height();
            }
        }
        self.height
    }

    pub fn has_crop(&self) -> bool {
        self.crop_bounds.is_some()
    }
}
