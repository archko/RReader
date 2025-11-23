use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use env_logger::Env;
use log::{debug, error, info};
use slint::{ComponentHandle, ModelRc, VecModel};

mod cache;
mod decoder;
mod page;
mod ui;

use decoder::DecodeService;

slint::include_modules!();

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("debug")  // 默认日志级别：info
    ).init();
    let app = MainWindow::new()?;
    let decode_service = Rc::new(RefCell::new(DecodeService::new()));

    setup_open_handler(&app, decode_service.clone());
    setup_viewport_handler(&app, decode_service.clone());
    setup_scroll_handler(&app, decode_service.clone());
    setup_page_handler(&app, decode_service.clone());
    setup_zoom_handler(&app, decode_service.clone());

    app.run()?;
    Ok(())
}

fn setup_open_handler(app: &MainWindow, service: Rc<RefCell<DecodeService>>) {
    let weak_app = app.as_weak();

    app.on_open_file(move || {
        let file_path = rfd::FileDialog::new()
            .add_filter("PDF Files", &["pdf"])
            .add_filter("PDF Files", &["epub"])
            .add_filter("PDF Files", &["mobi"])
            .add_filter("All Files", &["*"])
            .set_title("Select PDF File")
            .pick_file();

        if let Some(path) = file_path {
            let mut service = service.borrow_mut();
            match service.load_pdf(&path) {
                Ok(_) => {
                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(service.zoom());
                        refresh_view(&app, &mut service);
                    }
                }
                Err(err) => {
                    error!("Failed to open PDF: {err}");
                }
            }
        }
    });
}

fn setup_viewport_handler(app: &MainWindow, service: Rc<RefCell<DecodeService>>) {
    let weak_app = app.as_weak();
    app.on_viewport_changed(move |width, height| {
        let mut service = service.borrow_mut();
        service.update_viewport(width, height);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut service);
        }
    });
}

fn setup_scroll_handler(app: &MainWindow, service: Rc<RefCell<DecodeService>>) {
    let weak_app = app.as_weak();
    app.on_scroll_changed(move |offset_x, offset_y| {
        let mut service = service.borrow_mut();
        service.update_scroll_from_viewport(offset_x, offset_y);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut service);
        }
    });
}

fn setup_page_handler(app: &MainWindow, service: Rc<RefCell<DecodeService>>) {
    let weak_app = app.as_weak();
    app.on_page_changed(move |page_index| {
        let mut service = service.borrow_mut();
        if service.jump_to_page(page_index as usize).is_some() {
            if let Some(app) = weak_app.upgrade() {
                refresh_view(&app, &mut service);
            }
        }
    });
}

fn setup_zoom_handler(app: &MainWindow, service: Rc<RefCell<DecodeService>>) {
    let weak_app = app.as_weak();
    app.on_zoom_changed(move |zoom| {
        let mut service = service.borrow_mut();
        service.set_zoom(zoom);
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &mut service);
        }
    });
}

fn refresh_view(app: &MainWindow, service: &mut DecodeService) {
    let rendered_pages = service.collect_visible_pages();
    let page_models: Vec<PageData> = rendered_pages
        .into_iter()
        .map(|page| PageData {
            x: page.x,
            y: page.y,
            width: page.width,
            height: page.height,
            image: convert_to_slint_image(&page.image),
        })
        .collect();

    debug!(
        "[Main] refresh_view {} page_models",
        page_models.len()
    );
    let model = Rc::new(VecModel::from(page_models));
    app.set_document_pages(ModelRc::from(model));
    app.set_page_count(service.page_count() as i32);
    app.set_zoom(service.zoom());

    if let Some(first_visible) = service.first_visible_page() {
        app.set_current_page(first_visible as i32);
    }

    let (total_width, total_height) = service.total_size();
    app.set_total_width(total_width);
    app.set_total_height(total_height);

    let (offset_x, offset_y) = service.current_viewport_offset();
    app.set_scroll_events_enabled(false);
    app.set_offset_x(offset_x);
    app.set_offset_y(offset_y);
    app.set_scroll_events_enabled(true);
}

fn convert_to_slint_image(image: &image::DynamicImage) -> slint::Image {
    debug!(
        "[STATE] Converting image with dimensions: {}x{}",
        image.width(),
        image.height()
    );
    let rgba_image = image.to_rgba8();
    let (width, height) = rgba_image.dimensions();

    let slint_image = slint::Image::from_rgba8_premultiplied(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba_image, width, height),
    );
    debug!("[STATE] Successfully converted image to Slint image");
    slint_image
}
