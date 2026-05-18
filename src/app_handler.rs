use std::sync::{Arc, Mutex};
use crate::controllers::{HistoryControllerPointer, DocumentController};
use crate::controllers::history_controller::DefaultHistoryController;
use crate::ui::MainViewmodel;
use crate::tts::TtsService;
use std::cell::RefCell;
use std::rc::Rc;

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
