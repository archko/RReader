use sea_orm::{Database, DatabaseConnection, DbErr, EntityTrait, Statement, ConnectionTrait};
use lazy_static::lazy_static;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::path::Path;
use log::{debug, info};

lazy_static! {
    static ref DATABASE: Mutex<Option<Arc<DatabaseConnection>>> = Mutex::new(None);
}

/// 确保数据库文件和表存在，如果不存在则创建
pub async fn ensure_database_ready(db_path: &Path) -> Result<(), DbErr> {
    info!("ensure_database_ready:{:?}", db_path);
    if !db_path.exists() {
        // 数据库文件不存在，先创建空文件
        if let Err(e) = std::fs::File::create(db_path) {
            return Err(sea_orm::DbErr::Custom(format!("Failed to create database file: {}", e)));
        }

        // 连接数据库
        let db_path_str = db_path.to_string_lossy();
        let database_url = format!("sqlite:///{}", db_path_str);

        let db = Database::connect(&database_url).await?;
        *DATABASE.lock().await = Some(Arc::new(db));

        // 创建表
        create_tables().await?;
    }

    Ok(())
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

    // 检查表是否存在，如果不存在则创建
    let check_stmt = Statement::from_string(
        db.get_database_backend(),
        "SELECT name FROM sqlite_master WHERE type='table' AND name='recents'".to_string(),
    );

    let result: Vec<String> = db.query_all(check_stmt).await?
        .iter()
        .filter_map(|row| row.try_get("", "name").ok())
        .collect();

    if result.is_empty() {
        debug!("ensure_database_ready.表不存在，创建之:{:?}", db_path);
        // 表不存在，创建之
        db.execute_unprepared(r#"
            CREATE TABLE recents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                book_path TEXT NOT NULL UNIQUE,
                update_at INTEGER NOT NULL,
                page INTEGER DEFAULT 0,
                page_count INTEGER DEFAULT 0,
                create_at INTEGER NOT NULL,
                crop INTEGER DEFAULT 1,
                reflow INTEGER DEFAULT 0,
                scroll_ori INTEGER DEFAULT 1,
                zoom REAL DEFAULT 1.0,
                scroll_x INTEGER DEFAULT 0,
                scroll_y INTEGER DEFAULT 0,
                name TEXT,
                ext TEXT,
                size INTEGER,
                read_times INTEGER DEFAULT 0,
                progress INTEGER DEFAULT 0,
                favorited INTEGER DEFAULT 0,
                in_recent INTEGER DEFAULT 0
            )
        "#).await?;
    }

    Ok(())
}
