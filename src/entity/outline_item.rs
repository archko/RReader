#[derive(Debug, Clone)]
pub struct OutlineItem {
    pub title: String,
    pub uri: String,
    pub page: i32,
    pub level: i32,
}

impl OutlineItem {
    pub fn new(title: String, uri: String, page: i32, level: i32) -> OutlineItem {
        OutlineItem {
            title,
            uri,
            page,
            level
        }
    }
}