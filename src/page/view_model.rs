use slint::{Image, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use crate::PageData;

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

/// 稳定索引模型，支持全量同步和增量更新
pub struct ViewModel {
    pub slots: RefCell<Vec<SlotEntry>>,
    model: RefCell<Rc<VecModel<PageData>>>,
}

impl ViewModel {
    pub fn new() -> Self {
        Self {
            slots: RefCell::new(Vec::new()),
            model: RefCell::new(Rc::new(VecModel::default())),
        }
    }

    /// 全量重建：传入新的 slots 和 model，返回 model 的 Rc
    pub fn sync(&self, slots: Vec<SlotEntry>, datas: Vec<PageData>) -> Rc<VecModel<PageData>> {
        let model = Rc::new(VecModel::from(datas));
        *self.slots.borrow_mut() = slots;
        *self.model.borrow_mut() = Rc::clone(&model);
        model
    }

    /// 增量更新单个条目（解码完成时调用）
    /// 返回 true 表示该 slot 仍在 model 中（即仍在视口内）
    pub fn apply_tile(
        &self,
        cache_key: &str,
        image: Image,
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
            let model = self.model.borrow();
            model.set(
                idx,
                PageData {
                    x,
                    y,
                    width,
                    height,
                    image,
                    page_index,
                },
            );
            true
        } else {
            false
        }
    }

    pub fn clear(&self) {
        self.slots.borrow_mut().clear();
        let new_model = Rc::new(VecModel::default());
        *self.model.borrow_mut() = new_model;
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
