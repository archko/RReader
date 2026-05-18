use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;

/// 缓存键与页面位置的映射关系
#[derive(Clone)]
pub struct SlotEntry {
    pub cache_key: String,
    pub page_index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 页面数据，用于UI渲染
#[derive(Clone)]
pub struct PageData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub image: Option<DynamicImage>,
    pub page_index: i32,
}

/// 稳定索引模型，支持全量同步和增量更新
pub struct ViewModel {
    pub slots: RefCell<Vec<SlotEntry>>,
    pub data: RefCell<Vec<PageData>>,
}

impl ViewModel {
    pub fn new() -> Self {
        Self {
            slots: RefCell::new(Vec::new()),
            data: RefCell::new(Vec::new()),
        }
    }

    /// 全量重建
    pub fn sync(&self, slots: Vec<SlotEntry>, datas: Vec<PageData>) {
        *self.slots.borrow_mut() = slots;
        *self.data.borrow_mut() = datas;
    }

    /// 增量更新单个条目（解码完成时调用）
    /// 返回 true 表示该 slot 仍在 model 中（即仍在视口内）
    pub fn apply_tile(
        &self,
        cache_key: &str,
        image: DynamicImage,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        page_index: i32,
    ) -> bool {
        let slot_idx = {
            let slots = self.slots.borrow();
            slots.iter().position(|s| s.cache_key == cache_key)
        };

        if let Some(idx) = slot_idx {
            let mut data = self.data.borrow_mut();
            if idx < data.len() {
                data[idx] = PageData {
                    x,
                    y,
                    width,
                    height,
                    image: Some(image),
                    page_index,
                };
            }
            true
        } else {
            false
        }
    }

    pub fn clear(&self) {
        self.slots.borrow_mut().clear();
        self.data.borrow_mut().clear();
    }

    pub fn slot_count(&self) -> usize {
        self.slots.borrow().len()
    }
}

impl Default for ViewModel {
    fn default() -> Self {
        Self::new()
    }
}
