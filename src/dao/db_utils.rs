use diesel::prelude::*;
use diesel::r2d2::{Pool, ConnectionManager};
use diesel::SqliteConnection;
use lazy_static::lazy_static;
use std::sync::Mutex;

type DbPool = Pool<ConnectionManager<SqliteConnection>>;
type PooledConnection = diesel::r2d2::PooledConnection<ConnectionManager<SqliteConnection>>;

lazy_static! {
    static ref POOL: Mutex<Option<DbPool>> = Mutex::new(None);
}

pub fn init_db(database_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let manager = ConnectionManager::<SqliteConnection>::new(database_url);
    let pool = Pool::builder().build(manager)?;
    *POOL.lock().unwrap() = Some(pool);
    Ok(())
}

pub fn get_connection() -> Result<PooledConnection, Box<dyn std::error::Error>> {
    let pool_ref = POOL.lock().unwrap();
    let pool = pool_ref.as_ref().ok_or("DB pool not initialized")?;
    Ok(pool.get()?)
}

pub fn create_tables() -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = get_connection()?;
    diesel::sql_query(r#"
        CREATE TABLE IF NOT EXISTS recent (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT,
            update_at BIGINT,
            page BIGINT,
            page_count BIGINT,
            create_at BIGINT,
            crop BIGINT,
            reflow BIGINT,
            scroll_ori BIGINT,
            zoom REAL,
            scroll_x BIGINT,
            scroll_y BIGINT,
            name TEXT,
            ext TEXT,
            size BIGINT,
            read_times BIGINT,
            progress BIGINT,
            favorited BIGINT,
            in_recent BIGINT
        )
    "#).execute(&mut conn)?;
    Ok(())
}
