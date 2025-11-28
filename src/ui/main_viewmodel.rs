use crate::dao::RecentDao;
use crate::entity::Recent;
use crate::entity::NewRecent;

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

    /// 加载历史记录，可分页
    pub fn load_history(&mut self, page: usize) -> Result<(), Box<dyn std::error::Error>> {
        let all_recent = RecentDao::find_all()?;
        self.total_records = all_recent.len();
        self.page_index = page;

        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(all_recent.len());
        self.current_page_records = all_recent[start..end].to_vec();

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

    /// 添加新记录（打开文档时调用）
    pub fn add_recent(&self, new_recent: NewRecent) -> Result<(), Box<dyn std::error::Error>> {
        // 先查找是否已存在
        if let Some(existing) = RecentDao::find_by_path(&new_recent.book_path)? {
            // 更新现有记录
            RecentDao::update_by_path(&existing.book_path, &new_recent)?;
        } else {
            // 插入新记录
            RecentDao::insert(new_recent)?;
        }

        Ok(())
    }
}
