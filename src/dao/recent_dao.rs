use sea_orm::{prelude::Expr, *};
use dotenvy::dotenv;
use std::env;

use crate::entity::recent::{ActiveModel, Entity, Model as Recent, NewRecent};

pub struct RecentDao;

impl RecentDao {
    pub async fn init() -> Result<(), DbErr> {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        crate::dao::init_db(&database_url).await?;
        //crate::dao::create_tables().await?;
        Ok(())
    }

    pub async fn insert(other_recent: NewRecent) -> Result<Recent, DbErr> {
        let db = crate::dao::get_connection().await?;
        let active_model = other_recent.into_active_model();
        let result = active_model.insert(&*db).await?;
        Ok(result)
    }

    pub async fn find_by_id(other_id: i32) -> Result<Option<Recent>, DbErr> {
        let db = crate::dao::get_connection().await?;
        let result = Entity::find_by_id(other_id).one(&*db).await?;
        Ok(result)
    }

    pub async fn find_all() -> Result<Vec<Recent>, DbErr> {
        let db = crate::dao::get_connection().await?;
        let results = Entity::find().all(&*db).await?;
        Ok(results)
    }

    pub async fn update(id: i32, other_recent: &NewRecent) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        let mut active_model: ActiveModel = Entity::find_by_id(id).one(&*db).await?
            .ok_or_else(|| DbErr::Custom("Record not found".to_string()))?
            .into();

        active_model.book_path = Set(other_recent.book_path.clone());
        active_model.update_at = Set(other_recent.update_at);
        active_model.page = Set(other_recent.page);
        active_model.page_count = Set(other_recent.page_count);
        active_model.crop = Set(other_recent.crop);
        active_model.reflow = Set(other_recent.reflow);
        active_model.scroll_ori = Set(other_recent.scroll_ori);
        active_model.zoom = Set(other_recent.zoom);
        active_model.scroll_x = Set(other_recent.scroll_x);
        active_model.scroll_y = Set(other_recent.scroll_y);
        active_model.name = Set(other_recent.name.clone());
        active_model.ext = Set(other_recent.ext.clone());
        active_model.size = Set(other_recent.size);
        active_model.read_times = Set(other_recent.read_times);
        active_model.progress = Set(other_recent.progress);
        active_model.favorited = Set(other_recent.favorited);
        active_model.in_recent = Set(other_recent.in_recent);

        active_model.update(&*db).await?;
        Ok(())
    }

    pub async fn delete(other_id: i32) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        Entity::delete_by_id(other_id).exec(&*db).await?;
        Ok(())
    }

    pub async fn find_by_path(other_path: &str) -> Result<Option<Recent>, DbErr> {
        let db = crate::dao::get_connection().await?;
        let result = Entity::find()
            .filter(crate::entity::recent::Column::BookPath.eq(other_path))
            .one(&*db)
            .await?;
        Ok(result)
    }

    pub async fn update_by_path(
        other_path: &str,
        other_recent: &NewRecent,
    ) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        Entity::update_many()
            .col_expr(crate::entity::recent::Column::UpdateAt, Expr::value(other_recent.update_at))
            .col_expr(crate::entity::recent::Column::Page, Expr::value(other_recent.page))
            .col_expr(crate::entity::recent::Column::PageCount, Expr::value(other_recent.page_count))
            .col_expr(crate::entity::recent::Column::Crop, Expr::value(other_recent.crop))
            .col_expr(crate::entity::recent::Column::Reflow, Expr::value(other_recent.reflow))
            .col_expr(crate::entity::recent::Column::ScrollOri, Expr::value(other_recent.scroll_ori))
            .col_expr(crate::entity::recent::Column::Zoom, Expr::value(other_recent.zoom))
            .col_expr(crate::entity::recent::Column::ScrollX, Expr::value(other_recent.scroll_x))
            .col_expr(crate::entity::recent::Column::ScrollY, Expr::value(other_recent.scroll_y))
            .col_expr(crate::entity::recent::Column::Name, Expr::value(&other_recent.name))
            .col_expr(crate::entity::recent::Column::Ext, Expr::value(&other_recent.ext))
            .col_expr(crate::entity::recent::Column::Size, Expr::value(other_recent.size))
            .col_expr(crate::entity::recent::Column::ReadTimes, Expr::value(other_recent.read_times))
            .col_expr(crate::entity::recent::Column::Progress, Expr::value(other_recent.progress))
            .col_expr(crate::entity::recent::Column::Favorited, Expr::value(other_recent.favorited))
            .col_expr(crate::entity::recent::Column::InRecent, Expr::value(other_recent.in_recent))
            .filter(crate::entity::recent::Column::BookPath.eq(other_path))
            .exec(&*db)
            .await?;
        Ok(())
    }

    pub async fn delete_by_path(other_path: &str) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        Entity::delete_many()
            .filter(crate::entity::recent::Column::BookPath.eq(other_path))
            .exec(&*db)
            .await?;
        Ok(())
    }

    // Synchronous versions using join handle for compatibility
    pub fn init_sync() -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::init().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn insert_sync(other_recent: NewRecent) -> Result<Recent, Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::insert(other_recent).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn find_by_id_sync(other_id: i32) -> Result<Option<Recent>, Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::find_by_id(other_id).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn find_all_sync() -> Result<Vec<Recent>, Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::find_all().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn update_sync(id: i32, other_recent: &NewRecent) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::update(id, other_recent).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn delete_sync(other_id: i32) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::delete(other_id).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn find_by_path_sync(other_path: &str) -> Result<Option<Recent>, Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::find_by_path(other_path).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn update_by_path_sync(
        other_path: &str,
        other_recent: &NewRecent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::update_by_path(other_path, other_recent).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn delete_by_path_sync(other_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::delete_by_path(other_path).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }
}
