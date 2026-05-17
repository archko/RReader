use lru::LruCache;
use slint::Image;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// O(1) LRU 图片缓存，淘汰策略为最近最少使用
pub struct ImageCache {
    cache: Arc<Mutex<LruCache<String, Arc<Image>>>>,
}

impl ImageCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(
                LruCache::new(NonZeroUsize::new(max_size).unwrap_or(NonZeroUsize::new(1).unwrap())),
            )),
        }
    }

    /// 获取图片并标记为最近使用
    pub fn get(&self, key: &str) -> Option<Arc<Image>> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(key).cloned()
    }

    /// 存入图片（自动淘汰最久未使用的项）
    pub fn put(&self, key: String, image: Image) -> Arc<Image> {
        let mut cache = self.cache.lock().unwrap();
        let arc = Arc::new(image);
        let cloned = arc.clone();
        cache.put(key, arc);
        cloned
    }

    pub fn remove(&self, key: &str) -> bool {
        let mut cache = self.cache.lock().unwrap();
        cache.pop(key).is_some()
    }

    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    pub fn size(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }
}

/// 双层缓存：全尺寸页面图片（24张）+ 缩略图（10张）
pub struct PageCache {
    pub image_cache: ImageCache,
    pub thumbnail_cache: ImageCache,
}

impl PageCache {
    pub fn new(max_images: usize, max_thumbnails: usize) -> Self {
        Self {
            image_cache: ImageCache::new(max_images),
            thumbnail_cache: ImageCache::new(max_thumbnails),
        }
    }

    // ===== 全尺寸页面图片缓存 =====
    // 使用字符串 key 直接操作，key 格式由调用方决定

    pub fn get_page_image_by_key(&self, key: &str) -> Option<Arc<Image>> {
        self.image_cache.get(key)
    }

    pub fn put_page_image_by_key(&self, key: String, image: Image) -> Arc<Image> {
        self.image_cache.put(key, image)
    }

    // ===== 缩略图缓存 =====

    pub fn get_thumbnail(&self, key: &str) -> Option<Arc<Image>> {
        self.thumbnail_cache.get(key)
    }

    pub fn put_thumbnail(&self, key: String, image: Image) -> Arc<Image> {
        self.thumbnail_cache.put(key, image)
    }

    pub fn clear(&self) {
        self.image_cache.clear();
        self.thumbnail_cache.clear();
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new(24, 10)
    }
}
