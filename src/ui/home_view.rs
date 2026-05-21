use image;
use log::debug;

use crate::ui::utils::get_thumbnail_path;

/// 缩略图 RGBA 数据
#[derive(Clone)]
pub struct ThumbData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 主页条目
#[derive(Debug, Clone)]
pub struct HomeItem {
    pub item_path: String,
    pub title: String,
    pub page: i32,
    pub page_count: i32,
}

/// 主页视图模型 - 负责加载封面/缩略图
pub struct HomeView {
    pub records: Vec<HomeItem>,
    pub thumbnails: Vec<Option<ThumbData>>,
}

impl Default for HomeView {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeView {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            thumbnails: Vec::new(),
        }
    }

    /// 从缓存文件加载缩略图
    pub fn load_thumbnails(&mut self) {
        self.thumbnails.clear();
        for item in &self.records {
            let cache_path = get_thumbnail_path(&item.item_path);
            if cache_path.is_empty() {
                self.thumbnails.push(None);
                continue;
            }
            match image::open(&cache_path) {
                Ok(img) => {
                    let resized = img.thumbnail(160, 200);
                    let rgba = resized.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    self.thumbnails.push(Some(ThumbData {
                        rgba: rgba.into_raw().into(),
                        width: w,
                        height: h,
                    }));
                }
                Err(e) => {
                    debug!("failed to load thumbnail: {}", e);
                    self.thumbnails.push(None);
                }
            }
        }
    }
}
