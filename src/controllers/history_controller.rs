use std::sync::{Arc, Mutex, LazyLock, RwLock};
use image::DynamicImage;
use std::rc::Rc;
use crate::entity::Recent;
use crate::ui::MainViewmodel;
use std::cell::RefCell;
use std::rc::Rc as StdRc;
use crate::ui::utils::get_thumbnail_path;
use crate::controllers::DocumentController;
use log::{debug};

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

/// UI历史记录条目（用于后续Xilem UI渲染）
#[derive(Clone)]
pub struct UIRecent {
    pub title: String,
    pub path: String,
    pub thumbnail: Option<DynamicImage>,
    pub has_thumbnail: bool,
    pub page: i32,
}

/// 将历史记录转换为UI项目
pub fn convert_history_records_to_items(records: &[Recent]) -> Vec<UIRecent> {
    records
        .iter()
        .map(|record| {
            let path = record.book_path.clone();
            let cache_path = get_thumbnail_path(&path);

            let (thumbnail, has_thumbnail) = if !cache_path.is_empty() {
                if let Ok(dynamic_image) = image::open(&cache_path) {
                    (Some(dynamic_image), true)
                } else {
                    (None, false)
                }
            } else {
                (None, false)
            };

            UIRecent {
                title: record.name.clone(),
                path: path,
                thumbnail,
                has_thumbnail,
                page: record.page,
            }
        })
        .collect()
}

pub trait HistoryController {
    /// 获取所有历史记录
    fn get_history_items(&self) -> Result<Vec<Recent>, Box<dyn std::error::Error>>;

    /// 添加或更新历史记录
    fn add_or_update_history(&self, path: &str, name: &str) -> Result<(), Box<dyn std::error::Error>>;

    /// 删除历史记录
    fn remove_history(&self, id: i32) -> Result<(), Box<dyn std::error::Error>>;

    /// 清空所有历史记录
    fn clear_history(&self) -> Result<(), Box<dyn std::error::Error>>;

    /// 获取最近使用的文档
    fn get_recent_documents(&self, limit: usize) -> Result<Vec<Recent>, Box<dyn std::error::Error>>;

    /// 获取当前UI历史条目
    fn get_ui_history_items(&self) -> Vec<UIRecent>;
}

/// 历史控制器指针类型
pub type HistoryControllerPointer = Box<dyn HistoryController>;

pub struct DefaultHistoryController {
    viewmodel: StdRc<RefCell<MainViewmodel>>,
    document_controller: Rc<RefCell<DocumentController>>,
}

impl DefaultHistoryController {
    pub fn new(viewmodel: StdRc<RefCell<MainViewmodel>>, document_controller: Rc<RefCell<DocumentController>>) -> Self {
        Self { viewmodel, document_controller }
    }
}

impl HistoryController for DefaultHistoryController {
    fn get_history_items(&self) -> Result<Vec<Recent>, Box<dyn std::error::Error>> {
        let binding = self.viewmodel.borrow();
        let records = binding.get_current_records();
        Ok(records.to_vec())
    }

    fn add_or_update_history(&self, path: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;

        let new_record = crate::entity::recent::ActiveModel {
            book_path: sea_orm::ActiveValue::Set(path.to_string()),
            name: sea_orm::ActiveValue::Set(name.to_string()),
            page: sea_orm::ActiveValue::Set(0),
            page_count: sea_orm::ActiveValue::Set(0),
            update_at: sea_orm::ActiveValue::Set(now),
            ..Default::default()
        };

        self.viewmodel.borrow().add_recent(new_record)?;
        Ok(())
    }

    fn remove_history(&self, id: i32) -> Result<(), Box<dyn std::error::Error>> {
        crate::dao::RecentDao::delete_sync(id)?;
        Ok(())
    }

    fn clear_history(&self) -> Result<(), Box<dyn std::error::Error>> {
        crate::dao::RecentDao::clear_all_sync()?;
        Ok(())
    }

    fn get_recent_documents(&self, limit: usize) -> Result<Vec<Recent>, Box<dyn std::error::Error>> {
        let records = crate::dao::RecentDao::find_all_ordered_by_update_at_desc_sync()?;
        Ok(records.into_iter().take(limit).collect())
    }

    fn get_ui_history_items(&self) -> Vec<UIRecent> {
        let history_items = self.get_history_items().unwrap_or_default();
        convert_history_records_to_items(&history_items)
    }
}
