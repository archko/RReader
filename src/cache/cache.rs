use image::DynamicImage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ImageCache {
    cache: Arc<Mutex<HashMap<String, CachedImage>>>,
    max_size: usize,
}

#[derive(Clone)]
pub struct CachedImage {
    pub image: Arc<DynamicImage>,
    pub timestamp: std::time::Instant,
    pub access_count: u64,
}

impl ImageCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            max_size,
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<DynamicImage>> {
        let mut cache = self.cache.lock().unwrap();

        if let Some(cached) = cache.get_mut(key) {
            cached.access_count += 1;
            cached.timestamp = std::time::Instant::now();
            return Some(cached.image.clone());
        }

        None
    }

    pub fn put(&self, key: String, image: DynamicImage) -> Arc<DynamicImage> {
        let mut cache = self.cache.lock().unwrap();

        // 如果缓存已满，清理最久未使用的项
        if cache.len() >= self.max_size {
            self.evict_lru(&mut cache);
        }

        let cached_image = CachedImage {
            image: Arc::new(image),
            timestamp: std::time::Instant::now(),
            access_count: 1,
        };

        let image_ref = cached_image.image.clone();
        cache.insert(key, cached_image);

        image_ref
    }

    pub fn remove(&self, key: &str) -> bool {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(key).is_some()
    }

    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    pub fn size(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }

    fn evict_lru(&self, cache: &mut HashMap<String, CachedImage>) {
        // 找到最久未使用的项
        let mut oldest_key = None;
        let mut oldest_time = std::time::Instant::now();

        for (key, cached) in cache.iter() {
            if cached.timestamp < oldest_time {
                oldest_time = cached.timestamp;
                oldest_key = Some(key.clone());
            }
        }

        if let Some(key) = oldest_key {
            cache.remove(&key);
        }
    }
}

pub struct PageCache {
    image_cache: ImageCache,
    thumbnail_cache: ImageCache,
}

impl PageCache {
    pub fn new(max_images: usize, max_thumbnails: usize) -> Self {
        Self {
            image_cache: ImageCache::new(max_images),
            thumbnail_cache: ImageCache::new(max_thumbnails),
        }
    }

    pub fn get_page_image(&self, page_index: usize, zoom: f32) -> Option<Arc<DynamicImage>> {
        let key = format!("page_{}_{:.2}", page_index, zoom);
        self.image_cache.get(&key)
    }

    pub fn put_page_image(
        &self,
        page_index: usize,
        zoom: f32,
        image: DynamicImage,
    ) -> Arc<DynamicImage> {
        let key = format!("page_{}_{:.2}", page_index, zoom);
        self.image_cache.put(key, image)
    }

    pub fn get_thumbnail(&self, page_index: usize) -> Option<Arc<DynamicImage>> {
        let key = format!("thumb_{}", page_index);
        self.thumbnail_cache.get(&key)
    }

    pub fn put_thumbnail(&self, page_index: usize, image: DynamicImage) -> Arc<DynamicImage> {
        let key = format!("thumb_{}", page_index);
        self.thumbnail_cache.put(key, image)
    }

    pub fn clear(&self) {
        self.image_cache.clear();
        self.thumbnail_cache.clear();
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(10)
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::new(8, 20)
    }
}
