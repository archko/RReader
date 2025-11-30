use sea_orm::{Database, DatabaseConnection, DbErr};
use lazy_static::lazy_static;
use std::sync::Arc;
use tokio::sync::Mutex;

lazy_static! {
    static ref DATABASE: Mutex<Option<Arc<DatabaseConnection>>> = Mutex::new(None);
}

pub async fn init_db(database_url: &str) -> Result<(), DbErr> {
    let db = Database::connect(database_url).await?;
    *DATABASE.lock().await = Some(Arc::new(db));
    Ok(())
}

pub async fn get_connection() -> Result<Arc<DatabaseConnection>, DbErr> {
    let db_ref = DATABASE.lock().await;
    match db_ref.as_ref() {
        Some(db) => Ok(db.clone()),
        None => Err(DbErr::Custom("DB not initialized".to_string())),
    }
}

pub async fn create_tables() -> Result<(), DbErr> {
    let db = get_connection().await?;

    // Sea-ORM handles table creation through migrations or the database should be pre-created
    // For now, we'll assume the tables exist or are created elsewhere
    Ok(())
}