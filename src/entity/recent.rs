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

impl Recent {
    pub fn new(book_path: String) -> ActiveModel {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        ActiveModel {
            id: NotSet,
            book_path: Set(book_path),
            update_at: Set(now),
            create_at: Set(now),
            page: Set(0),
            page_count: Set(0),
            crop: Set(1),
            reflow: Set(0),
            scroll_ori: Set(1),
            zoom: Set(1.0),
            scroll_x: Set(0),
            scroll_y: Set(0),
            name: Set("".to_string()),
            ext: Set("".to_string()),
            size: Set(0),
            read_times: Set(0),
            progress: Set(0),
            favorited: Set(0),
            in_recent: Set(0),
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
    ) -> ActiveModel {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        ActiveModel {
            id: NotSet,
            book_path: Set(book_path),
            update_at: Set(now),
            create_at: Set(now),
            page: Set(page),
            page_count: Set(page_count),
            crop: Set(crop),
            reflow: Set(reflow),
            scroll_ori: Set(scroll_ori),
            zoom: Set(zoom),
            scroll_x: Set(scroll_x),
            scroll_y: Set(scroll_y),
            name: Set(name),
            ext: Set(ext),
            size: Set(size),
            read_times: Set(read_times),
            progress: Set(progress),
            favorited: Set(favorited),
            in_recent: Set(in_recent),
        }
    }
}
