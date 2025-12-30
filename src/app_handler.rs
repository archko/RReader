use std::sync::{Arc, Mutex};
use slint::ComponentHandle;
use crate::controllers::{HistoryControllerPointer, DocumentController};
use crate::controllers::history_controller::DefaultHistoryController;
use crate::ui::MainViewmodel;
use crate::tts::TtsService;
use std::cell::RefCell;
use std::rc::Rc;

use crate::AppWindow;

pub struct AppHandler {
    history_controller: HistoryControllerPointer,
    document_controller: Rc<RefCell<DocumentController>>,
}

impl AppHandler {
    pub fn new(viewmodel: Rc<RefCell<MainViewmodel>>, tts_service: Arc<Mutex<TtsService>>) -> Self {
        let document_controller = Rc::new(RefCell::new(DocumentController::new(viewmodel.clone(), Arc::clone(&tts_service))));
        let history_controller: HistoryControllerPointer = Box::new(DefaultHistoryController::new(viewmodel, Rc::clone(&document_controller)));

        Self {
            history_controller,
            document_controller,
        }
    }

    pub fn initialize_ui(&mut self, window: &AppWindow) {
        self.history_controller.setup_history_callbacks(window);

        self.document_controller.borrow().initialize_ui(window);

        if let Err(e) = self.history_controller.refresh_history_ui(window) {
            log::error!("Failed to refresh history UI: {}", e);
        }
    }

    pub fn document_controller(&self) -> Rc<RefCell<DocumentController>> {
        Rc::clone(&self.document_controller)
    }

    pub fn history_controller(&self) -> &HistoryControllerPointer {
        &self.history_controller
    }

    pub fn save(&self) {
        log::debug!("保存应用状态");
    }

    pub fn reload(&self) {
        log::debug!("重新加载应用状态");
    }
}
