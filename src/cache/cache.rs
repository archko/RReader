use xilem::masonry::peniko::ImageData;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ImageCache {
    cache: Arc<Mutex<LruCache<String, Arc<ImageData>>>>,
}

impl ImageCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(
                LruCache::new(NonZeroUsize::new(max_size).unwrap_or(NonZeroUsize::new(1).unwrap())),
            )),
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<ImageData>> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(key).cloned()
    }

    pub fn put(&self, key: String, image: ImageData) -> Arc<ImageData> {
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

#[derive(Clone)]
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

    pub fn get_page_image_by_key(&self, key: &str) -> Option<Arc<ImageData>> {
        self.image_cache.get(key)
    }

    pub fn put_page_image_by_key(&self, key: String, image: ImageData) -> Arc<ImageData> {
        self.image_cache.put(key, image)
    }

    pub fn get_thumbnail(&self, key: &str) -> Option<Arc<ImageData>> {
        self.thumbnail_cache.get(key)
    }

    pub fn put_thumbnail(&self, key: String, image: ImageData) -> Arc<ImageData> {
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
