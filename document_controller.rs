use std::rc::Rc;
use std::cell::RefCell;
use std::path::Path;
use crate::page::{PageViewState};
use crate::ui::MainViewmodel;
use crate::dao::RecentDao;
use crate::entity::{Recent};
use crate::ui_generated::MainWindow;
use slint::ComponentHandle;
use log::{error, info};

pub struct DocumentController {
    app_weak: slint::Weak<MainWindow>,
    page_view_state: Rc<RefCell<PageViewState>>,
    viewmodel: Rc<RefCell<MainViewmodel>>,
}

impl DocumentController {
    pub fn new(
        app: &MainWindow, 
        state: Rc<RefCell<PageViewState>>, 
        vm: Rc<RefCell<MainViewmodel>>
    ) -> Rc<Self> {
        Rc::new(Self {
            app_weak: app.as_weak(),
            page_view_state: state,
            viewmodel: vm,
        })
    }

    // 将逻辑封装在这里
    pub fn handle_open_file(&self) {
        let file_path = rfd::FileDialog::new()
            .add_filter("PDF Files", &["pdf"])
            .add_filter("PDF Files", &["epub"])
            .add_filter("PDF Files", &["mobi"])
            .add_filter("All Files", &["cbz"])
            .add_filter("All Files", &["docx"])
            .add_filter("All Files", &["xps"])
            .add_filter("All Files", &["djvu"])
            .add_filter("All Files", &["tif"])
            .add_filter("All Files", &["tiff"])
            .set_title("Select PDF File")
            .pick_file();

        if let Some(path) = file_path {
            let path_str = path.to_string_lossy().to_string();
            if let Some(app) = self.app_weak.upgrade() {
                app.set_file_path(path_str.clone().into());
                
                // 1. 执行打开文档
                if let Err(e) = self.page_view_state.borrow_mut().open_document(&path) {
                    error!("Failed to open PDF: {e}");
                    return;
                }

                // 2. 数据库逻辑：查询历史
                let existing_recent = self.viewmodel.borrow().get_recent_by_path(&path_str).unwrap_or(None);
                
                let (zoom, page) = match existing_recent {
                    Some(ref rec) => (rec.zoom, rec.page),
                    None => {
                        self.create_new_history(&path_str);
                        (1.0, 1)
                    }
                };

                // 3. 更新 UI 状态
                self.sync_ui_after_open(app, zoom, page);
            }
        }
    }

    fn create_new_history(&self, path_str: &str) {
        let recent = Recent::encode(
            path_str.to_string(),
            0, 0, 1, 1, 0, 1.0, 0, 0,
            path_str.split('/').last().unwrap_or("").to_string(),
            path_str.split('.').last().unwrap_or("").to_string(),
            0, 0, 1, 0, 0,
        );
        let _ = self.viewmodel.borrow().add_recent(recent);
    }

    fn sync_ui_after_open(&self, app: MainWindow, zoom: f32, page: i32) {
        app.set_zoom(zoom);
        app.set_current_page(page);
        app.set_document_opened(true);

        let mut state = self.page_view_state.borrow_mut();
        let (w, h) = state.view_size;
        state.update_view_size(w, h, zoom, true);
        let _ = state.jump_to_page((page - 1) as usize);
        
        // 调用你之前定义的辅助函数
        crate::set_outline_to_ui(&app, &state);
        state.update_visible_pages();
        crate::refresh_view(&app, &state);
    }
}
