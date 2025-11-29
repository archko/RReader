use crate::schema::recents::dsl::recents;
use diesel::prelude::*;
use dotenvy::dotenv;
use std::env;

use crate::dao::get_connection;
use crate::entity::recent::NewRecent;
use crate::entity::Recent;

pub struct RecentDao;

impl RecentDao {
    pub fn init() -> Result<(), Box<dyn std::error::Error>> {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        SqliteConnection::establish(&database_url)
            .unwrap_or_else(|_| panic!("Error connecting to {}", database_url));
        //create_tables()?;
        Ok(())
    }

    pub fn insert(other_recent: NewRecent) -> Result<Recent, Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        let inserted = diesel::insert_into(recents)
            .values(&other_recent)
            .returning(Recent::as_returning())
            .get_result(&mut conn)
            .expect("Error saving new post");

        Ok(inserted)
    }

    pub fn find_by_id(other_id: i32) -> Result<Option<Recent>, Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        let result = recents
            .find(other_id)
            .select(Recent::as_select())
            .first(&mut conn)
            .optional()?;
        Ok(result)
    }

    pub fn find_all() -> Result<Vec<Recent>, Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        let results = recents.select(Recent::as_select()).load(&mut conn)?;
        Ok(results)
    }

    pub fn update(id: i32, other_recent: &NewRecent) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        diesel::update(recents.find(id))
            .set(other_recent)
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn delete(other_id: i32) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        diesel::delete(recents.find(other_id)).execute(&mut conn)?;
        Ok(())
    }

    pub fn find_by_path(other_path: &str) -> Result<Option<Recent>, Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        let result = recents
            .filter(recents::book_path.eq(other_path))
            .select(Recent::as_select())
            .first::<Recent>(&mut conn)
            .optional()?;
        Ok(result)
    }

    pub fn update_by_path(
        other_path: &str,
        other_recent: &NewRecent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        diesel::update(recents)
            .filter(recents::book_path.eq(other_path))
            .set(other_recent)
            .execute(&mut conn)
            .is_ok();
        Ok(())
    }

    pub fn delete_by_path(other_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = get_connection()?;
        diesel::delete(recents.filter(recents::book_path.eq(other_path))).execute(&mut conn)?;
        Ok(())
    }
}
