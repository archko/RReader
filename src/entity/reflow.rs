use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ReflowEntry {
    pub data: String,
    pub page: String,
}

#[derive(Serialize, Deserialize)]
pub struct ReflowData {
    pub page_count: usize,
    pub file_size: u64,
    pub reflow: Vec<ReflowEntry>,
}