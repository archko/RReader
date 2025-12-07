use sea_orm::{prelude::Expr, *};
use dotenvy::dotenv;
use std::env;

use crate::entity::recent::{ActiveModel, Entity, Model as Recent};

pub struct RecentDao;

impl RecentDao {
    pub async fn init() -> Result<(), DbErr> {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        crate::dao::init_db(&database_url).await?;
        Ok(())
    }

    pub async fn insert(other_recent: ActiveModel) -> Result<Recent, DbErr> {
        let db = crate::dao::get_connection().await?;
        let result = other_recent.insert(&*db).await?;
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

    pub async fn find_all_ordered_by_update_at_desc() -> Result<Vec<Recent>, DbErr> {
        let db = crate::dao::get_connection().await?;
        let results = Entity::find()
            .order_by_desc(crate::entity::recent::Column::UpdateAt)
            .all(&*db)
            .await?;
        Ok(results)
    }

    pub async fn update(id: i32, update_data: ActiveModel) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        update_data.update(&*db).await?;
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
        update_data: ActiveModel,
    ) -> Result<(), DbErr> {
        let db = crate::dao::get_connection().await?;
        let mut updater = Entity::update_many();

        if let ActiveValue::Set(ref val) = update_data.update_at {
            updater = updater.col_expr(crate::entity::recent::Column::UpdateAt, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.page {
            updater = updater.col_expr(crate::entity::recent::Column::Page, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.page_count {
            updater = updater.col_expr(crate::entity::recent::Column::PageCount, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.crop {
            updater = updater.col_expr(crate::entity::recent::Column::Crop, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.reflow {
            updater = updater.col_expr(crate::entity::recent::Column::Reflow, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.scroll_ori {
            updater = updater.col_expr(crate::entity::recent::Column::ScrollOri, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.zoom {
            updater = updater.col_expr(crate::entity::recent::Column::Zoom, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.scroll_x {
            updater = updater.col_expr(crate::entity::recent::Column::ScrollX, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.scroll_y {
            updater = updater.col_expr(crate::entity::recent::Column::ScrollY, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.name {
            updater = updater.col_expr(crate::entity::recent::Column::Name, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.ext {
            updater = updater.col_expr(crate::entity::recent::Column::Ext, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.size {
            updater = updater.col_expr(crate::entity::recent::Column::Size, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.read_times {
            updater = updater.col_expr(crate::entity::recent::Column::ReadTimes, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.progress {
            updater = updater.col_expr(crate::entity::recent::Column::Progress, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.favorited {
            updater = updater.col_expr(crate::entity::recent::Column::Favorited, Expr::value(val.clone()));
        }
        if let ActiveValue::Set(ref val) = update_data.in_recent {
            updater = updater.col_expr(crate::entity::recent::Column::InRecent, Expr::value(val.clone()));
        }

        updater
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

    pub fn insert_sync(other_recent: ActiveModel) -> Result<Recent, Box<dyn std::error::Error>> {
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

    pub fn find_all_ordered_by_update_at_desc_sync() -> Result<Vec<Recent>, Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::find_all_ordered_by_update_at_desc().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
            })
        })
    }

    pub fn update_sync(id: i32, update_data: ActiveModel) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::update(id, update_data).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
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
        update_data: ActiveModel,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                Self::update_by_path(other_path, update_data).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
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
