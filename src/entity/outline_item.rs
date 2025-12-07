#[derive(Debug, Clone)]
pub struct OutlineItem {
    pub title: String,
    pub uri: Option<String>,
    pub page: i32,
    pub level: i32,
}

impl OutlineItem {
    pub fn new(title: String, uri: Option<String>, page: i32, level: i32) -> OutlineItem {
        OutlineItem {
            title,
            uri,
            page,
            level
        }
    }
}