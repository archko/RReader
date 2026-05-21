#![allow(unused)]
#![allow(dead_code)]
#![allow(non_snake_case)]

use log::info;
use xilem::masonry::dpi::LogicalSize;
use xilem::masonry::layout::AsUnit;
use xilem::masonry::peniko::Color;
use xilem::masonry::properties::{Background, BorderColor, ContentColor, Padding};
use xilem::masonry::theme::default_property_set;
use xilem::masonry::widgets::{Button, Label};
use xilem::palette;

mod cache;
mod dao;
mod decoder;
mod entity;
mod page;
mod tts;
mod ui;

use xilem::WidgetView;
use xilem::{EventLoop, WindowOptions, Xilem};
use ui::{AppState, ViewKind, home_view, document_view};

/// 根视图：根据当前状态切换 Home / Document
fn app_logic(state: &mut AppState) -> impl WidgetView<AppState> + use<> {
    match state.view {
        ViewKind::Home => home_view(state).boxed(),
        ViewKind::Document { .. } => document_view(state).boxed(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 初始化应用数据目录
    let data_dir = dirs::data_dir().expect("Unable to get data directory");
    let app_data_dir = data_dir.join("RReader");
    std::fs::create_dir_all(&app_data_dir).expect("Unable to create app data directory");

    let db_path = app_data_dir.join("book.db");
    let database_url = format!("sqlite:///{}", db_path.display());
    info!("Database: {}", database_url);
    std::env::set_var("DATABASE_URL", &database_url);

    // 初始化数据库
    tokio::task::block_in_place(|| {
        futures::executor::block_on(async {
            crate::dao::ensure_database_ready(&db_path)
                .await
                .expect("Failed to initialize database");
        });
    });
    crate::dao::RecentDao::init_sync().unwrap();

    // 启动 Xilem UI
    let state = AppState::default();

    let mut theme = default_property_set();
    theme.insert::<Button, _>(Background::Color(palette::css::WHITE));
    theme.insert::<Button, _>(BorderColor { color: palette::css::GAINSBORO });
    theme.insert::<Button, _>(Padding::from_vh(2.px(), 4.px()));
    theme.insert::<Label, _>(ContentColor::new(Color::from_rgb8(0x33, 0x33, 0x33)));

    let app = Xilem::new_simple(
        state,
        app_logic,
        WindowOptions::new("RReader - 文档阅读").with_min_inner_size(LogicalSize::new(
            900.0,
            700.0,
        )),
    )
    .with_default_base_color(palette::css::WHITE)
    .with_default_properties(theme);
    app.run_in(EventLoop::with_user_event())?;

    Ok(())
}
