use std::sync::{Arc, Mutex, LazyLock, RwLock};
use slint::{ModelRc, VecModel, ComponentHandle};
use std::rc::Rc;
use crate::entity::Recent;
use crate::ui::MainViewmodel;
use std::cell::RefCell;
use std::rc::Rc as StdRc;
use crate::decoder::pdf::utils::convert_to_slint_image;
use crate::ui::utils::get_thumbnail_path;
use crate::controllers::DocumentController;
use log::{debug};

static HISTORY_VIEWPORT_WIDTH: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(1024.0));

/// 将历史记录转换为UI项目
pub fn convert_history_records_to_items(records: &[Recent]) -> Vec<crate::UIRecent> {
    records
        .iter()
        .map(|record| {
            let path = record.book_path.clone();
            let cache_path = get_thumbnail_path(&path);

            let (thumbnail, has_thumbnail) = if !cache_path.is_empty() {
                if let Ok(dynamic_image) = image::open(&cache_path) {
                    (convert_to_slint_image(&dynamic_image), true)
                } else {
                    (slint::Image::default(), false)
                }
            } else {
                (slint::Image::default(), false)
            };

            crate::UIRecent {
                title: record.name.clone().into(),
                path: path.into(),
                thumbnail,
                has_thumbnail,
                page: record.page,
            }
        })
        .collect()
}

/// 设置历史记录到UI
pub fn set_history_to_ui(app: &crate::AppWindow, ui_history_items: Vec<crate::UIRecent>) {
    let history_model = Rc::new(VecModel::from(ui_history_items.clone()));
    app.set_history_items(ModelRc::from(history_model));

    let width = *HISTORY_VIEWPORT_WIDTH.read().unwrap();
    let columns = (width / 188.0).floor().max(1.0) as usize;
    let grouped: Vec<Vec<crate::UIRecent>> = ui_history_items.chunks(columns).map(|c| c.to_vec()).collect();
    let rows: Vec<crate::HistoryRow> = grouped.into_iter().map(|vec| crate::HistoryRow { items: ModelRc::from(Rc::new(VecModel::from(vec))) }).collect();
    let history_rows_model = Rc::new(VecModel::from(rows));
    app.set_history_rows(ModelRc::from(history_rows_model));
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

    /// 刷新历史记录UI显示
    fn refresh_history_ui(&self, window: &crate::AppWindow) -> Result<(), Box<dyn std::error::Error>>;

    /// 设置历史记录相关回调
    fn setup_history_callbacks(&self, window: &crate::AppWindow);
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

    fn refresh_history_ui(&self, window: &crate::AppWindow) -> Result<(), Box<dyn std::error::Error>> {
        let history_items = self.get_history_items()?;
        let ui_history_items = convert_history_records_to_items(&history_items);
        set_history_to_ui(window, ui_history_items);
        Ok(())
    }

    fn setup_history_callbacks(&self, window: &crate::AppWindow) {
        let weak_window = window.as_weak();
        let weak_window2 = window.as_weak();
        let weak_window3 = window.as_weak();
        let history_controller = self as *const dyn HistoryController;
        let document_controller = Rc::clone(&self.document_controller);

        window.on_history_item_clicked(move |ui_recent| {
            let path_str = ui_recent.path.to_string();
            let path_obj = std::path::Path::new(&path_str);

            if path_obj.exists() {
                log::info!("Opening history document: {}", path_str);
                if let Some(window) = weak_window.upgrade() {
                    document_controller.borrow().open_document(&window, &path_str);
                }
            } else {
                log::error!("File does not exist: {}", path_str);
            }
        });

        let viewmodel = StdRc::clone(&self.viewmodel);
        window.on_history_viewport_changed(move |width, height| {
            debug!("[Main] on_history_viewport_changed.width: {:?}, height: {:?}", width, height);

            let old_width = *HISTORY_VIEWPORT_WIDTH.read().unwrap();
            if (width - old_width).abs() > 0.1 { // 宽度变化阈值，防止抖动
                *HISTORY_VIEWPORT_WIDTH.write().unwrap() = width;

                if let Some(window) = weak_window2.upgrade() {
                    let viewmodel_binding = viewmodel.borrow();
                    let history_records = viewmodel_binding.get_current_records();
                    let ui_history_items = convert_history_records_to_items(history_records);
                    set_history_to_ui(&window, ui_history_items);
                    debug!("[Main] Updated history column count for new viewport width: {}", width);
                }
            }
        });

        window.on_clear_history(move || {
            let controller = unsafe { &*history_controller };
            if let Err(e) = controller.clear_history() {
                log::warn!("Failed to clear history: {}", e);
            }

            if let Some(window) = weak_window3.upgrade() {
                if let Err(e) = unsafe { &*history_controller }.refresh_history_ui(&window) {
                    log::error!("Failed to refresh history after clear: {}", e);
                }
            }
        });
    }
}
