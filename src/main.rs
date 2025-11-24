use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use env_logger::Env;
use log::{debug, error};
use slint::{ComponentHandle, ModelRc, VecModel};

mod cache;
mod decoder;
mod page;
mod ui;

use page::{PageViewState, Orientation};

slint::include_modules!();

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        Env::default().default_filter_or("debug")  // 默认日志级别：info
    ).init();
    let app = MainWindow::new()?;
    let page_view_state: Rc<RefCell<PageViewState>> = Rc::new(RefCell::new(PageViewState::new(Orientation::Vertical, 0)));

    setup_open_handler(&app, page_view_state.clone());
    setup_viewport_handler(&app, page_view_state.clone());
    setup_scroll_handler(&app, page_view_state.clone());
    setup_page_handler(&app, page_view_state.clone());
    setup_zoom_handler(&app, page_view_state.clone());
    setup_back_to_history_handler(&app, page_view_state.clone());

    app.run()?;
    Ok(())
}

fn setup_open_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
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
            if let Some(app) = weak_app.upgrade() {
                app.set_file_path(path.to_string_lossy().to_string().into());
            }
            
            let open_result = page_view_state.borrow_mut().open_document(&path);
            match open_result {
                Ok(_) => {
                    if let Some(app) = weak_app.upgrade() {
                        app.set_zoom(1.0);
                        app.set_document_opened(true);

                        let zoom = 1.0; // 初始缩放值
                        let mut borrowed_state = page_view_state.borrow_mut();
                        let width = borrowed_state.view_size.0;
                        let height = borrowed_state.view_size.1;
                        borrowed_state.update_view_size(
                            width, 
                            height, 
                            zoom,
                            true
                        );
                    }
                    
                    // 文档打开后立即刷新视图
                    if let Some(app) = weak_app.upgrade() {
                        page_view_state.borrow_mut().update_visible_pages();
                        refresh_view(&app, &page_view_state.borrow());
                    }
                }
                Err(err) => {
                    error!("Failed to open PDF: {err}");
                }
            }
        }
    });
}

fn setup_viewport_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_viewport_changed(move |width, height| {
        debug!("[Main] setup_viewport_handler.width: {:?}, height: {:?}", width, height);
        /// 视口变化处理
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            let zoom = borrowed_state.zoom;
            borrowed_state.update_view_size(width, height, zoom, false);
            borrowed_state.update_visible_pages();
        }
        debug!("[Main] setup_viewport_handler");
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
        }
    });
}

fn setup_scroll_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_scroll_changed(move |offset_x, offset_y| {
        // 滚动处理
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            borrowed_state.update_offset(offset_x, offset_y);
            borrowed_state.update_visible_pages();
        }
        
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
        }
    });
}

fn setup_page_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_page_changed(move |page_index| {
        // 页面跳转处理
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            if borrowed_state.jump_to_page(page_index as usize).is_some() {
                borrowed_state.update_visible_pages();
                
                if let Some(app) = weak_app.upgrade() {
                    refresh_view(&app, &*borrowed_state);
                }
            }
        }
    });
}

fn setup_zoom_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_zoom_changed(move |zoom| {
        // 缩放处理
        {
            let mut borrowed_state = page_view_state.borrow_mut();
            let (view_width, view_height) = borrowed_state.view_size;
            borrowed_state.update_view_size(view_width, view_height, zoom, true);
            borrowed_state.update_visible_pages();
        }
        
        if let Some(app) = weak_app.upgrade() {
            refresh_view(&app, &page_view_state.borrow());
        }
    });
}

fn refresh_view(app: &MainWindow, page_view_state: &PageViewState) {
    let state = page_view_state;
    if state.pages.is_empty() {
        debug!("[Main] No pages to refresh");
        return;
    }

    // 刷新视图显示
    let rendered_pages = page_view_state.visible_pages
        .iter()
        .filter_map(|&idx| page_view_state.pages.get(idx))
        .map(|page| {
            // 尝试从缓存获取图像，如果不存在则使用默认图像
            let image = {
                if let Some(cached_image) = page_view_state.decode_service.borrow().cache.get_page_image(page.info.index, page.info.scale) {
                    convert_to_slint_image(&(*cached_image))
                } else {
                    slint::Image::default()
                }
            };
            
            PageData {
                x: page.bounds.left,
                y: page.bounds.top,
                width: page.width,
                height: page.height,
                image,
            }
        })
        .collect::<Vec<_>>();

    debug!(
        "[Main] refresh_view {} page_models",
        rendered_pages.len()
    );
    let model = Rc::new(VecModel::from(rendered_pages));
    app.set_document_pages(ModelRc::from(model));
    app.set_page_count(page_view_state.pages.len() as i32);
    app.set_zoom(page_view_state.zoom);

    if let Some(first_visible) = page_view_state.get_first_visible_page() {
        app.set_current_page(first_visible as i32);
    }

    let (total_width, total_height) = (page_view_state.total_width, page_view_state.total_height);
    app.set_total_width(total_width);
    app.set_total_height(total_height);

    let (offset_x, offset_y) = (-page_view_state.view_offset.0, -page_view_state.view_offset.1);
    app.set_scroll_events_enabled(false);
    app.set_offset_x(offset_x);
    app.set_offset_y(offset_y);
    app.set_scroll_events_enabled(true);
}

fn setup_back_to_history_handler(app: &MainWindow, page_view_state: Rc<RefCell<PageViewState>>) {
    let weak_app = app.as_weak();
    app.on_back_to_history(move || {
        // 清空文件路径
        if let Some(app) = weak_app.upgrade() {
            app.set_file_path("".into());
            app.set_document_opened(false);
        }
        
        // 重置页面状态
        let mut borrowed_state = page_view_state.borrow_mut();
        borrowed_state.shutdown();
    });
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