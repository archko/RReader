use std::sync::Arc;
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

/// 缩略图像素数据
struct ThumbData {
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
}

/// 主页视图状态（Xilem 响应式状态）
pub struct HomeViewState {
    /// 当前页的历史记录
    pub records: Vec<UIHistoryItem>,
    /// 当前页码（从0开始）
    pub page_index: usize,
    /// 总页数
    pub total_pages: usize,
    /// 总记录数
    pub total_records: usize,
    /// 缩略图缓存（与 records 一一对应）
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

    /// 从数据库刷新当前页数据
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

impl Default for HomeViewState {
    fn default() -> Self {
        let mut s = Self::new();
        s.load_history();
        s
    }
}

const GRID_COLS: i32 = 4;

/// 构建单个历史卡片
fn history_card(
    state: &HomeViewState,
    idx: usize,
) -> impl WidgetView<HomeViewState> {
    let item = &state.records[idx];
    let title = item.title.clone();
    let page_text = format!("第 {} 页", item.page);

    if let Some(thumb) = state.thumbnails.get(idx).and_then(|t| t.as_ref()) {
        let img_data = ImageData {
            data: Arc::clone(&thumb.rgba),
            format: ImageFormat::Rgba8,
            width: thumb.width,
            height: thumb.height,
        };
        flex(Axis::Vertical, (
            sized_box(image(img_data).fit(ObjectFit::ScaleDown), 100.0, 140.0)
                .border(palette::css::LIGHT_GRAY, 1.0),
            label(title).padding(4.0),
            label(page_text).padding(4.0).color(palette::css::GRAY),
        ))
        .background_color(palette::css::WHITE)
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
        ))
        .background_color(palette::css::WHITE)
    }
}

/// 主页视图（入口）
pub fn home_view(state: &mut HomeViewState) -> Box<dyn WidgetView<HomeViewState>> {
    // ---- 顶部工具栏 ----
    let toolbar = flex(
        Axis::Horizontal,
        (
            text_button("打开文档", |_: &mut HomeViewState| {
                debug!("打开文档 - 待集成文件对话框");
            })
            .background_color(palette::css::DODGER_BLUE)
            .color(palette::css::WHITE),
            text_button("清除历史", |s: &mut HomeViewState| {
                s.clear_history();
            })
            .background_color(palette::css::INDIAN_RED)
            .color(palette::css::WHITE),
            label(format!("共 {} 条", state.total_records))
                .color(palette::css::DIM_GRAY)
                .flex(1.0),
            text_button("◀ 上一页", |s: &mut HomeViewState| s.prev_page()),
            label(format!(
                "{}/{}",
                state.page_index + 1,
                state.total_pages.max(1)
            )),
            text_button("下一页 ▶", |s: &mut HomeViewState| s.next_page()),
        ),
    )
    .padding(8.0)
    .background_color(palette::css::WHITE_SMOKE);

    // ---- 历史网格 ----
    let num_records = state.records.len();
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
                history_card(state, i).grid_pos(col, row)
            })
            .collect()
    } else {
        vec![
            label("暂无阅读记录，点击「打开文档」开始阅读")
                .grid_pos(0, 0),
        ]
    };

    let grid_widget = grid(grid_items, GRID_COLS, grid_rows).spacing(8.0);

    // ---- 根布局 ----
    let root = flex(
        Axis::Vertical,
        (toolbar, portal(grid_widget).flex(1.0)),
    );

    root.boxed()
}
