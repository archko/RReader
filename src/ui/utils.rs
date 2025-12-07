use dirs;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use slint::{Image, SharedPixelBuffer};

// 生成简单hash用于缓存图片名
pub fn generate_thumbnail_hash(path: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

// 获取缓存缩略图路径
pub fn get_thumbnail_path(book_path: &str) -> String {
    if let Some(data_dir) = dirs::data_dir() {
        let cache_dir = data_dir.join("RReader").join("images");
        let hash = generate_thumbnail_hash(book_path);
        let cache_path = cache_dir.join(format!("{}.png", hash));
        //log::info!("[Thumbnail] expected cache_path: {:?}, exists: {}", cache_path, cache_path.exists());
        if cache_path.exists() {
            cache_path.to_string_lossy().to_string()
        } else {
            "".to_string()
        }
    } else {
        //log::warn!("[Thumbnail] data_dir is None for: {:?}", book_path);
        "".to_string()
    }
}

#[derive(Clone)]
struct CachedImageData {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl CachedImageData {
    fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        Self { data, width, height }
    }

    fn to_slint_image(&self) -> Image {
        let shared_buffer = SharedPixelBuffer::clone_from_slice(
            &self.data,
            self.width,
            self.height,
        );
        Image::from_rgba8(shared_buffer)
    }
}
