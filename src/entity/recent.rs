use sea_orm::entity::prelude::*;
use sea_orm::{Set, NotSet};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "recents")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_path: String,
    pub update_at: i64,
    pub page: i32,
    pub page_count: i32,
    pub create_at: i64,
    pub crop: i32,
    pub reflow: i32,
    pub scroll_ori: i32,
    pub zoom: f32,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub name: String,
    pub ext: String,
    pub size: i64,
    pub read_times: i32,
    pub progress: i64,
    pub favorited: i32,
    pub in_recent: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

pub type Recent = Model;

#[derive(Debug, Clone)]
pub struct NewRecent {
    pub book_path: String,
    pub update_at: i64,
    pub page: i32,
    pub page_count: i32,
    pub create_at: i64,
    pub crop: i32,
    pub reflow: i32,
    pub scroll_ori: i32,
    pub zoom: f32,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub name: String,
    pub ext: String,
    pub size: i64,
    pub read_times: i32,
    pub progress: i64,
    pub favorited: i32,
    pub in_recent: i32,
}

impl NewRecent {
    pub fn into_active_model(self) -> ActiveModel {
        ActiveModel {
            id: NotSet,
            book_path: Set(self.book_path),
            update_at: Set(self.update_at),
            page: Set(self.page),
            page_count: Set(self.page_count),
            create_at: Set(self.create_at),
            crop: Set(self.crop),
            reflow: Set(self.reflow),
            scroll_ori: Set(self.scroll_ori),
            zoom: Set(self.zoom),
            scroll_x: Set(self.scroll_x),
            scroll_y: Set(self.scroll_y),
            name: Set(self.name),
            ext: Set(self.ext),
            size: Set(self.size),
            read_times: Set(self.read_times),
            progress: Set(self.progress),
            favorited: Set(self.favorited),
            in_recent: Set(self.in_recent),
        }
    }
}

impl Recent {
    pub fn new(book_path: String) -> NewRecent {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        NewRecent {
            book_path,
            update_at: now,
            create_at: now,
            page: 0,
            page_count: 0,
            crop: 1,
            reflow: 0,
            scroll_ori: 1,
            zoom: 1.0,
            scroll_x: 0,
            scroll_y: 0,
            name: "".to_string(),
            ext: "".to_string(),
            size: 0,
            read_times: 0,
            progress: 0,
            favorited: 0,
            in_recent: 0,
        }
    }

    pub fn encode(
        book_path: String,
        page: i32,
        page_count: i32,
        crop: i32,
        scroll_ori: i32,
        reflow: i32,
        zoom: f32,
        scroll_x: i32,
        scroll_y: i32,
        name: String,
        ext: String,
        size: i64,
        read_times: i32,
        progress: i64,
        favorited: i32,
        in_recent: i32,
    ) -> NewRecent {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        NewRecent {
            book_path,
            update_at: now,
            create_at: now,
            page: page,
            page_count: page_count,
            crop: crop,
            reflow: reflow,
            scroll_ori: scroll_ori,
            zoom: zoom,
            scroll_x: scroll_x,
            scroll_y: scroll_y,
            name,
            ext,
            size,
            read_times,
            progress,
            favorited,
            in_recent,
        }
    }
}
