use anyhow::Result;
use slint::ComponentHandle;
use std::rc::Rc;
use std::cell::RefCell;

mod pdf;
mod ui;
mod cache;
mod state;

use pdf::PdfDocument;
use state::AppState;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    let app = MainWindow::new()?;
    
    let app_state = Rc::new(RefCell::new(AppState::new()));
    
    let app_weak = app.as_weak();
    let state_clone = app_state.clone();
    
    app.on_open_file(move |_| {
        // 打开文件选择对话框
        let file_path = rfd::FileDialog::new()
            .add_filter("PDF Files", &["pdf"])
            .add_filter("All Files", &["*"])
            .set_title("Select PDF File")
            .pick_file();
        
        if let Some(path) = file_path {
            let app = app_weak.unwrap();
            let mut state = state_clone.borrow_mut();
            
            match PdfDocument::open(&path) {
                Ok(doc) => {
                    state.load_document(doc);
                    app.set_page_count(state.get_page_count() as i32);
                    app.set_current_page(0);
                    
                    if let Some(page) = state.get_page(0) {
                        app.set_page_image(page.into());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to open PDF: {}", e);
                }
            }
        }
    });
    
    let app_weak = app.as_weak();
    let state_clone = app_state.clone();
    
    app.on_page_changed(move |page_num| {
        let app = app_weak.unwrap();
        let mut state = state_clone.borrow_mut();
        
        state.set_current_page(page_num as usize);
        
        if let Some(page) = state.get_page(page_num as usize) {
            app.set_current_page(page_num);
            app.set_page_image(page.into());
        }
    });
    
    let app_weak = app.as_weak();
    let state_clone = app_state.clone();
    
    app.on_zoom_changed(move |zoom| {
        let app = app_weak.unwrap();
        let mut state = state_clone.borrow_mut();
        
        state.set_zoom(zoom);
        
        if let Some(current_page) = state.get_current_page() {
            if let Some(page) = state.get_page(current_page) {
                app.set_page_image(page.into());
            }
        }
    });
    
    app.run()?;
    
    Ok(())
}