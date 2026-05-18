use std::sync::Arc;
use std::path::Path;
use log::{error, debug};

use xilem::view::{
    Axis, flex, grid, image, label, portal, sized_box, text_button,
    FlexExt, GridExt, WidgetView, ObjectFit,
};
use xilem::palette;
use xilem::masonry::widgets::GridParams;
use vello::peniko::{ImageData, ImageFormat};

use crate::dao::RecentDao;
use crate::ui::utils::get_thumbnail_path;
use crate::ui::main_viewmodel::PAGE_SIZE;
use crate::page::PageRenderState;
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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            home: HomeViewState::new(),
            view: ViewKind::Home,
            page_render_state: Arc::new(PageRenderState::new()),
        }
    }

    /// 打开指定索引的历史记录文档
    pub fn open_document(&mut self, idx: usize) {
        if let Some(item) = self.home.records.get(idx) {
            let path = item.path.clone();
            let title = item.title.clone();

            // 启动文档加载
            self.start_loading_document(&path);

            self.view = ViewKind::Document { path, title };
        }
    }

    /// 启动后台加载文档
    fn start_loading_document(&self, path: &str) {
        let pv = Arc::clone(&self.page_render_state);
        let path_owned = path.to_string();

        // 发送加载命令给解码线程
        if let Err(e) = pv.decode_service.load_pdf(&path_owned) {
            error!("Failed to start document load: {}", e);
            return;
        }

        // 后台线程轮询加载结果
        std::thread::spawn(move || {
            let mut attempts = 0;
            loop {
                if let Some(result) = pv.decode_service.try_recv_load_result() {
                    match result {
                        Ok(pages_info) => {
                            debug!("Document loaded: {} pages", pages_info.len());
                            // 为每个页面创建 Page 并设置
                            let pages: Vec<crate::page::Page> = pages_info
                                .into_iter()
                                .map(|info| crate::page::Page::new(info, 0.0, 0.0, 0.0, 0.0))
                                .collect();
                            pv.set_pages(pages);
                            // 触发初始布局
                            pv.update_view_size(800.0, 600.0, 1.0, true);
                            // 触发初始可见页计算
                            pv.update_offset(0.0, 0.0);
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
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }

    /// 返回历史记录首页
    pub fn back_to_home(&mut self) {
        self.page_render_state.close();
        self.view = ViewKind::Home;
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
}

/// UI 层历史条目
#[derive(Clone)]
pub struct UIHistoryItem {
    pub id: i32,
    pub title: String,
    pub path: String,
    pub page: i32,
    pub read_times: i32,
    pub update_at: i64,
    pub has_thumbnail: bool,
}

impl HomeViewState {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            page_index: 0,
            total_pages: 0,
            total_records: 0,
            thumbnails: Vec::new(),
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
                            read_times: r.read_times,
                            update_at: r.update_at,
                            has_thumbnail: !cache_path.is_empty(),
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
                    let resized = img.thumbnail(100, 140);
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

    pub fn next_page(&mut self) {
        if self.page_index + 1 < self.total_pages {
            self.page_index += 1;
            self.load_history();
        }
    }

    pub fn prev_page(&mut self) {
        if self.page_index > 0 {
            self.page_index -= 1;
            self.load_history();
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

const GRID_COLS: i32 = 4;

/// 主页视图
pub fn home_view(state: &mut AppState) -> Box<dyn WidgetView<AppState>> {
    // ---- 工具栏 ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("打开文档", |_: &mut AppState| {
                debug!("打开文档 - 待集成文件对话框");
            })
            .background_color(palette::css::DODGER_BLUE)
            .color(palette::css::WHITE),
            text_button("清除历史", |s: &mut AppState| {
                s.home.clear_history();
            })
            .background_color(palette::css::INDIAN_RED)
            .color(palette::css::WHITE),
            label(format!("共 {} 条", state.home.total_records))
                .color(palette::css::DIM_GRAY)
                .flex(1.0),
            text_button("◀ 上一页", |s: &mut AppState| s.home.prev_page()),
            label(format!(
                "{}/{}",
                state.home.page_index + 1,
                state.home.total_pages.max(1)
            )),
            text_button("下一页 ▶", |s: &mut AppState| s.home.next_page()),
        ),
    )
    .padding(8.0)
    .background_color(palette::css::WHITE_SMOKE);

    // ---- 历史网格 ----
    let num_records = state.home.records.len();
    let grid_rows = if num_records == 0 {
        1
    } else {
        (num_records as i32 + GRID_COLS - 1) / GRID_COLS
    };

    let grid_items: Vec<_> = if num_records > 0 {
        (0..num_records)
            .map(|i| {
                let col = (i as i32) % GRID_COLS;
                let row = (i as i32) / GRID_COLS;
                let item = &state.home.records[i];
                let title = item.title.clone();
                let page_text = format!("第 {} 页", item.page);

                let card = if let Some(thumb) =
                    state.home.thumbnails.get(i).and_then(|t| t.as_ref())
                {
                    let img_data = ImageData {
                        data: Arc::clone(&thumb.rgba),
                        format: ImageFormat::Rgba8,
                        width: thumb.width,
                        height: thumb.height,
                    };
                    flex(Axis::Vertical, (
                        sized_box(
                            image(img_data).fit(ObjectFit::ScaleDown),
                            100.0,
                            140.0,
                        )
                        .border(palette::css::LIGHT_GRAY, 1.0),
                        label(title).padding(4.0),
                        label(page_text).padding(4.0).color(palette::css::GRAY),
                        text_button("打开", move |s: &mut AppState| s.open_document(i))
                            .background_color(palette::css::DODGER_BLUE)
                            .color(palette::css::WHITE)
                            .padding((6.0, 2.0)),
                    ))
                    .background_color(palette::css::WHITE)
                    .border(palette::css::LIGHT_GRAY, 1.0)
                    .hovered_border_color(palette::css::DODGER_BLUE)
                } else {
                    flex(Axis::Vertical, (
                        sized_box(
                            label("").background_color(palette::css::GAINSBORO),
                            100.0,
                            140.0,
                        )
                        .border(palette::css::LIGHT_GRAY, 1.0),
                        label(title).padding(4.0),
                        label(page_text).padding(4.0).color(palette::css::GRAY),
                        text_button("打开", move |s: &mut AppState| s.open_document(i))
                            .background_color(palette::css::DODGER_BLUE)
                            .color(palette::css::WHITE)
                            .padding((6.0, 2.0)),
                    ))
                    .background_color(palette::css::WHITE)
                    .border(palette::css::LIGHT_GRAY, 1.0)
                    .hovered_border_color(palette::css::DODGER_BLUE)
                };

                card.grid_pos(col, row)
            })
            .collect()
    } else {
        vec![label("暂无阅读记录，点击「打开文档」开始阅读").grid_pos(0, 0)]
    };

    let grid_widget = grid(grid_items, GRID_COLS, grid_rows).spacing(8.0);

    // ---- 根布局 ----
    flex(Axis::Vertical, (toolbar, portal(grid_widget).flex(1.0))).boxed()
}
