use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use slint::{ComponentHandle, ModelRc, VecModel};

mod cache;
mod decoder;
mod page;
mod pdf;
mod render;
mod state;
mod ui;

use state::AppState;

slint::include_modules!();

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let app = MainWindow::new()?;
    let app_state = Rc::new(RefCell::new(AppState::new()));

    setup_open_handler(&app, app_state.clone());
    setup_viewport_handler(&app, app_state.clone());
    setup_scroll_handler(&app, app_state.clone());
    setup_page_handler(&app, app_state.clone());
    setup_zoom_handler(&app, app_state.clone());

    app.run()?;
    Ok(())
}

fn setup_open_handler(app: &MainWindow, state: Rc<RefCell<AppState>>) {
    let weak_app = app.as_weak();

    app.on_open_file(move || {
        let file_path = rfd::FileDialog::new()
            .add_filter("PDF Files", &["pdf"])
            .add_filter("All Files", &["*"])
            .set_title("Select PDF File")
            .pick_file();

        if let Some(path) = file_path {
            let mut state = state.borrow_mut();
            match state.load_pdf(&path) {
                Ok(_) => {
                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(state.zoom());
                        refresh_view(&app, &mut state);
                    }
                }
                Err(err) => {
                    eprintln!("Failed to open PDF: {err}");
                }
            }
        }
    });
}

fn setup_viewport_handler(app: &MainWindow, state: Rc<RefCell<AppState>>) {
    let weak_app = app.as_weak();
    app.on_viewport_changed(move |width, height| {
        let mut state = state.borrow_mut();
        state.update_viewport(width, height);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut state);
        }
    });
}

fn setup_scroll_handler(app: &MainWindow, state: Rc<RefCell<AppState>>) {
    let weak_app = app.as_weak();
    app.on_scroll_changed(move |offset_x, offset_y| {
        let mut state = state.borrow_mut();
        state.update_scroll_from_viewport(offset_x, offset_y);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut state);
        }
    });
}

fn setup_page_handler(app: &MainWindow, state: Rc<RefCell<AppState>>) {
    let weak_app = app.as_weak();
    app.on_page_changed(move |page_index| {
        let mut state = state.borrow_mut();
        if state.jump_to_page(page_index as usize).is_some() {
            if let Some(app) = weak_app.upgrade() {
                refresh_view(&app, &mut state);
            }
        }
    });
}

fn setup_zoom_handler(app: &MainWindow, state: Rc<RefCell<AppState>>) {
    let weak_app = app.as_weak();
    app.on_zoom_changed(move |zoom| {
        let mut state = state.borrow_mut();
        state.set_zoom(zoom);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut state);
        }
    });
}

fn refresh_view(app: &MainWindow, state: &mut AppState) {
    let rendered_pages = state.collect_visible_pages();
    let page_models: Vec<PageData> = rendered_pages
        .into_iter()
        .map(|page| PageData {
            x: page.x,
            y: page.y,
            width: page.width,
            height: page.height,
            image: page.image,
        })
        .collect();

    println!("[Main] refresh_view {} page_models", page_models.len());
    let model = Rc::new(VecModel::from(page_models));
    app.set_document_pages(ModelRc::from(model));
    app.set_page_count(state.page_count() as i32);
    app.set_zoom(state.zoom());

    if let Some(first_visible) = state.first_visible_page() {
        app.set_current_page(first_visible as i32);
    }

    let (total_width, total_height) = state.total_size();
    app.set_total_width(total_width);
    app.set_total_height(total_height);

    let (offset_x, offset_y) = state.current_viewport_offset();
    app.set_scroll_events_enabled(false);
    app.set_offset_x(offset_x);
    app.set_offset_y(offset_y);
    app.set_scroll_events_enabled(true);
}
