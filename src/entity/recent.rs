use crate::schema::recents;
use diesel::{Insertable, Queryable, AsChangeset, Selectable};

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = recents)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Recent {
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

#[derive(Debug, Insertable, AsChangeset, Clone)]
#[diesel(table_name = recents)]
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
