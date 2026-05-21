use std::sync::Arc;
use std::path::Path;
use std::time::SystemTime;
use log::{error, debug};
use sea_orm::ActiveValue;

use xilem::masonry::layout::Length;
use xilem::view::{
    button, flex, grid, image, label, portal, resize_observer, sized_box, text_button,
    zstack, FlexExt, GridExt, ObjectFit, ZStackExt,
};
use xilem::WidgetView;
use xilem::kurbo::Size;
use xilem::masonry::kurbo::Axis;
use xilem::masonry::layout::UnitPoint;
use xilem::palette;
use xilem::masonry::widgets::GridParams;
use xilem::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use xilem::style::Style;

use crate::dao::RecentDao;
use crate::ui::utils::{get_thumbnail_path, pick_file};
use crate::ui::main_viewmodel::PAGE_SIZE;
use crate::page::{PageRenderState, render_state::process_visible_nodes};
use image::DynamicImage;

/// 顶层应用状态，管理视图切换
pub enum ViewKind {
    Home,
    Document { path: String, title: String },
}

pub struct AppState {
    pub home: HomeViewState,
    pub view: ViewKind,
    pub page_render_state: Arc<PageRenderState>,
    /// 文档视图专用 UI 状态（仅在 Document view 下有意义）
    pub document_ui: crate::ui::document_view::DocumentUiState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            home: HomeViewState::new(),
            view: ViewKind::Home,
            page_render_state: Arc::new(PageRenderState::new()),
            document_ui: crate::ui::document_view::DocumentUiState::default(),
        }
    }

    /// 打开指定索引的历史记录文档
    pub fn open_document(&mut self, idx: usize) {
        if let Some(item) = self.home.records.get(idx) {
            let path = item.path.clone();
            let title = item.title.clone();

            // 启动文档加载，传入历史记录中的位置
            self.start_loading_document(&path, item.page.max(1), item.zoom, item.crop);

            self.view = ViewKind::Document { path, title };
        }
    }

    /// 启动后台加载文档，并定位到指定页面
    fn start_loading_document(&self, path: &str, init_page: i32, init_zoom: f32, init_crop: i32) {
        let pv = Arc::clone(&self.page_render_state);
        let path_owned = path.to_string();

        // 发送加载命令给解码线程
        if let Err(e) = pv.decode_service.load_pdf(&path_owned) {
            error!("Failed to start document load: {}", e);
            return;
        }

        // 后台线程：等待文档加载完成 → 设置页面 → 恢复阅读位置
        std::thread::spawn(move || {
            let mut attempts = 0;
            loop {
                if let Some(result) = pv.decode_service.try_recv_load_result() {
                    match result {
                        Ok(pages_info) => {
                            let page_count = pages_info.len();
                            debug!("Document loaded: {} pages", page_count);
                            let pages: Vec<crate::page::Page> = pages_info
                                .into_iter()
                                .map(|info| crate::page::Page::new(info, 0.0, 0.0, 0.0, 0.0, 1.0))
                                .collect();
                            pv.set_pages(pages);
                            pv.update_view_size(800.0, 600.0, init_zoom, true);
                            // jump_to_page 使用 0-based 索引
                            let target_page = (init_page as usize)
                                .saturating_sub(1)
                                .min(page_count.saturating_sub(1));
                            pv.jump_to_page(target_page);
                            process_visible_nodes(&pv);
                            // 加载大纲
                            if let Ok(outline) = pv.decode_service.get_outline() {
                                pv.write().outline_items = outline;
                            }
                        }
                        Err(e) => {
                            error!("Failed to load document: {}", e);
                        }
                    }
                    break;
                }
                attempts += 1;
                if attempts >= 300 {
                    error!("Document load timed out");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }

    /// 返回历史记录首页
    pub fn back_to_home(&mut self) {
        let doc_path = match &self.view {
            ViewKind::Document { path, .. } => Some(path.clone()),
            _ => None,
        };

        if let Some(ref path) = doc_path {
            let (page, scroll_x, scroll_y, zoom, crop) = {
                let r = self.page_render_state.read();
                let page = r.visible_pages.first().map(|&p| p + 1).unwrap_or(1);
                (page, r.view_offset.0, r.view_offset.1, r.zoom, r.crop)
            };

            let now = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            let existing = RecentDao::find_by_path_sync(path).ok().flatten();

            if let Some(rec) = existing {
                let active = crate::entity::recent::ActiveModel {
                    id: ActiveValue::Set(rec.id),
                    page: ActiveValue::Set(page as i32),
                    scroll_x: ActiveValue::Set(scroll_x as i32),
                    scroll_y: ActiveValue::Set(scroll_y as i32),
                    zoom: ActiveValue::Set(zoom),
                    crop: ActiveValue::Set(crop),
                    update_at: ActiveValue::Set(now),
                    ..Default::default()
                };
                let _ = RecentDao::update_by_path_sync(path, active);
            } else {
                let title = match &self.view {
                    ViewKind::Document { ref title, .. } => title.clone(),
                    _ => String::new(),
                };
                let active = crate::entity::recent::ActiveModel {
                    id: ActiveValue::NotSet,
                    book_path: ActiveValue::Set(path.clone()),
                    name: ActiveValue::Set(title),
                    page: ActiveValue::Set(page as i32),
                    crop: ActiveValue::Set(crop),
                    zoom: ActiveValue::Set(zoom),
                    scroll_x: ActiveValue::Set(scroll_x as i32),
                    scroll_y: ActiveValue::Set(scroll_y as i32),
                    update_at: ActiveValue::Set(now),
                    create_at: ActiveValue::Set(now),
                    ..Default::default()
                };
                let _ = RecentDao::insert_sync(active);
            }
        }

        self.page_render_state.close();
        self.view = ViewKind::Home;
        self.home.load_history();
    }

}

impl Default for AppState {
    fn default() -> Self {
        let mut s = Self::new();
        s.home.load_history();
        s
    }
}

/// 缩略图像素数据
struct ThumbData {
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
}

/// 主页视图状态
pub struct HomeViewState {
    pub records: Vec<UIHistoryItem>,
    pub page_index: usize,
    pub total_pages: usize,
    pub total_records: usize,
    thumbnails: Vec<Option<ThumbData>>,
    pub grid_cols: i32,
}

/// UI 层历史条目
#[derive(Clone)]
pub struct UIHistoryItem {
    pub id: i32,
    pub title: String,
    pub path: String,
    pub page: i32,
    pub page_count: i32,
    pub read_times: i32,
    pub update_at: i64,
    pub has_thumbnail: bool,
    pub zoom: f32,
    pub crop: i32,
}

impl HomeViewState {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            page_index: 0,
            total_pages: 0,
            total_records: 0,
            thumbnails: Vec::new(),
            grid_cols: 4,
        }
    }

    pub fn load_history(&mut self) {
        match RecentDao::find_all_ordered_by_update_at_desc_sync() {
            Ok(all) => {
                self.total_records = all.len();
                self.total_pages = if self.total_records == 0 {
                    0
                } else {
                    (self.total_records + PAGE_SIZE - 1) / PAGE_SIZE
                };

                let start = self.page_index * PAGE_SIZE;
                let end = (start + PAGE_SIZE).min(all.len());
                let page = &all[start..end];

                self.records = page
                    .iter()
                    .map(|r| {
                        let cache_path = get_thumbnail_path(&r.book_path);
                        UIHistoryItem {
                            id: r.id,
                            title: if r.name.is_empty() {
                                r.book_path
                                    .rsplit(std::path::MAIN_SEPARATOR)
                                    .next()
                                    .unwrap_or(&r.book_path)
                                    .to_string()
                            } else {
                                r.name.clone()
                            },
                            path: r.book_path.clone(),
                            page: r.page,
                            page_count: r.page_count,
                            read_times: r.read_times,
                            update_at: r.update_at,
                            has_thumbnail: !cache_path.is_empty(),
                            zoom: r.zoom,
                            crop: r.crop,
                        }
                    })
                    .collect();

                self.load_thumbnails();
                debug!(
                    "history loaded: page={}, items={}, total={}",
                    self.page_index,
                    self.records.len(),
                    self.total_records
                );
            }
            Err(e) => {
                error!("load history failed: {}", e);
                self.records.clear();
                self.total_records = 0;
                self.total_pages = 0;
            }
        }
    }

    fn load_thumbnails(&mut self) {
        self.thumbnails.clear();
        for item in &self.records {
            let cache_path = get_thumbnail_path(&item.path);
            if cache_path.is_empty() {
                self.thumbnails.push(None);
                continue;
            }
            match image::open(&cache_path) {
                Ok(img) => {
                    let resized = img.thumbnail(160, 200);
                    let rgba = resized.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    self.thumbnails.push(Some(ThumbData {
                        rgba: rgba.into_raw().into(),
                        width: w,
                        height: h,
                    }));
                }
                Err(e) => {
                    debug!("failed to load thumbnail: {}", e);
                    self.thumbnails.push(None);
                }
            }
        }
    }

    pub fn clear_history(&mut self) {
        if let Err(e) = RecentDao::clear_all_sync() {
            error!("clear history failed: {}", e);
        }
        self.page_index = 0;
        self.load_history();
    }
}

/// 主页视图
pub fn home_view(state: &mut AppState) -> impl WidgetView<AppState> + use<> {
    // ---- 工具栏 ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("打开文档", |s: &mut AppState| {
                if let Some(path) = pick_file() {
                    let title = std::path::Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    s.start_loading_document(&path, 1, 1.0, 0);
                    s.view = ViewKind::Document { path, title };
                }
            }),
            text_button("清除历史", |s: &mut AppState| {
                s.home.clear_history();
            }),
            label(format!("共 {} 条", state.home.total_records))
                .color(palette::css::DIM_GRAY)
                .flex(1.0),
        ),
    )
    .padding(Length::const_px(8.0))
    .background(palette::css::WHITE);

    // ---- 历史网格 ----
    let grid_cols = state.home.grid_cols.max(1);
    let num_records = state.home.records.len();
    let grid_rows = if num_records == 0 {
        1
    } else {
        (num_records as i32 + grid_cols - 1) / grid_cols
    };

    let grid_items: Vec<_> = if num_records > 0 {
        (0..num_records)
            .map(|i| {
                let col = (i as i32) % grid_cols;
                let row = (i as i32) / grid_cols;
                let item = &state.home.records[i];
                let page_text = format!("{}/{}", item.page, item.page_count);

                let cover = if let Some(thumb) =
                    state.home.thumbnails.get(i).and_then(|t| t.as_ref())
                {
                    let img_data = ImageData {
                        data: Blob::from(thumb.rgba.to_vec()),
                        format: ImageFormat::Rgba8,
                        alpha_type: ImageAlphaType::Alpha,
                        width: thumb.width,
                        height: thumb.height,
                    };
                    sized_box(
                        image(img_data).fit(ObjectFit::Cover),
                    )
                    .fixed_width(Length::const_px(160.0))
                    .fixed_height(Length::const_px(200.0))
                    .boxed()
                } else {
                    sized_box(
                        label("").background(palette::css::GAINSBORO),
                    )
                    .fixed_width(Length::const_px(160.0))
                    .fixed_height(Length::const_px(200.0))
                    .boxed()
                };

                button(
                    zstack((
                        cover,
                        label(page_text)
                            .background_color(palette::css::BLACK.multiply_alpha(0.55))
                            .color(palette::css::WHITE)
                            .padding(Length::const_px(6.0))
                            .alignment(UnitPoint::BOTTOM_RIGHT),
                    ))
                    .background(palette::css::WHITE),
                    move |s: &mut AppState| s.open_document(i),
                )
                .background(palette::css::WHITE)
                .boxed()
                .grid_pos(col, row)
            })
            .collect()
    } else {
        vec![label("暂无阅读记录，点击「打开文档」开始阅读").boxed().grid_pos(0, 0)]
    };

    let grid_widget = grid(grid_items, grid_cols, grid_rows).gap(Length::const_px(8.0));

    resize_observer(
        |s: &mut AppState, size: Size| {
            let gap = 32.0_f64;
            let card = 160.0_f64;
            let cols = ((size.width - 16.0_f64) / (card + gap)).floor().max(1.0) as i32;
            // 仅在值真正变化时才更新，避免无限触发 rebuild
            if s.home.grid_cols != cols {
                s.home.grid_cols = cols;
            }
        },
        flex(Axis::Vertical, (toolbar, portal(grid_widget).flex(1.0)))
            .padding(Length::const_px(8.0))
            .background(palette::css::WHITE),
    )
    .boxed()
}
