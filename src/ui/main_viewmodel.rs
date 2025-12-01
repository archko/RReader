use crate::dao::RecentDao;
use crate::entity::Recent;
use crate::entity::recent::ActiveModel;
use std::time::SystemTime;
use log::debug;
use sea_orm::ActiveValue;

pub const PAGE_SIZE: usize = 16;

pub struct MainViewmodel{
    pub page_index: usize,
    total_records: usize,
    current_page_records: Vec<Recent>,
}

impl MainViewmodel {
    pub fn new() -> Self {
        Self {
            page_index: 0,
            total_records: 0,
            current_page_records: Vec::new(),
        }
    }

    /// 加载历史记录，可分页，按update_at倒序
    pub fn load_history(&mut self, page: usize) -> Result<(), Box<dyn std::error::Error>> {
        let all_recent = RecentDao::find_all_ordered_by_update_at_desc_sync()?;
        self.total_records = all_recent.len();
        self.page_index = page;

        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(all_recent.len());
        self.current_page_records = all_recent[start..end].to_vec();

        debug!("load_history:page:{}, count:{:?}", page, self.current_page_records.len());

        Ok(())
    }

    /// 获取当前页的记录
    pub fn get_current_records(&self) -> &[Recent] {
        &self.current_page_records
    }

    /// 获取总页数
    pub fn get_total_pages(&self) -> usize {
        if PAGE_SIZE == 0 {
            0
        } else {
            (self.total_records + PAGE_SIZE - 1) / PAGE_SIZE
        }
    }

    /// 获取总记录数
    pub fn get_total_records(&self) -> usize {
        self.total_records
    }

    /// 是否有下一页
    pub fn has_next_page(&self) -> bool {
        self.page_index < self.get_total_pages() - 1
    }

    /// 是否有上一页
    pub fn has_prev_page(&self) -> bool {
        self.page_index > 0
    }

    /// 下一页
    pub fn next_page(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.has_next_page() {
            self.load_history(self.page_index + 1)?;
        }
        Ok(())
    }

    /// 上一页
    pub fn prev_page(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.has_prev_page() {
            self.load_history(self.page_index - 1)?;
        }
        Ok(())
    }

    /// 获取指定路径的记录
    pub fn get_recent_by_path(&self, path: &str) -> Result<Option<Recent>, Box<dyn std::error::Error>> {
        RecentDao::find_by_path_sync(path)
    }

    /// 更新指定路径的阅读次数
    pub fn update_read_times(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(mut rec) = RecentDao::find_by_path_sync(path)? {
            rec.read_times += 1;
            let now = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as i64;
            let active = ActiveModel {
                id: ActiveValue::Set(rec.id),
                read_times: ActiveValue::Set(rec.read_times),
                update_at: ActiveValue::Set(now),
                ..Default::default()
            };
            RecentDao::update_by_path_sync(path, active)?;
        }
        Ok(())
    }

    /// 更新指定路径的状态（页面、缩放、滚动位置），同时更新阅读次数和更新时间
    pub fn update_recent_with_state(&self, path: &str, page: Option<usize>, zoom: f32, scroll_x: f32, scroll_y: f32) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(mut rec) = RecentDao::find_by_path_sync(path)? {
            let page_val = page.map(|p| (p + 1) as i32).unwrap_or(rec.page); // 如果没有提供页面，使用当前值
            let read_times = rec.read_times + 1; // 增加阅读次数
            let now = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as i64;
            let active = ActiveModel {
                id: ActiveValue::Set(rec.id),
                page: ActiveValue::Set(page_val),
                zoom: ActiveValue::Set(zoom),
                scroll_x: ActiveValue::Set(scroll_x as i32),
                scroll_y: ActiveValue::Set(scroll_y as i32),
                read_times: ActiveValue::Set(read_times),
                update_at: ActiveValue::Set(now),
                ..Default::default()
            };
            RecentDao::update_by_path_sync(path, active)?;
        }
        Ok(())
    }

    /// 添加新记录（打开文档时调用）
    pub fn add_recent(&self, new_recent: ActiveModel) -> Result<(), Box<dyn std::error::Error>> {
        // 从 ActiveModel 中获取 book_path
        let book_path = match new_recent.book_path {
            ActiveValue::Set(ref path) => path.clone(),
            _ => return Err("book_path must be set".into()),
        };

        // 先查找是否已存在
        if let Some(_existing) = RecentDao::find_by_path_sync(&book_path)? {
            debug!("update recent:{:?}", new_recent);
            // 更新现有记录
            RecentDao::update_by_path_sync(&book_path, new_recent)?;
        } else {
            // 插入新记录
            debug!("insert recent:{:?}", new_recent);
            RecentDao::insert_sync(new_recent)?;
        }

        Ok(())
    }
}
