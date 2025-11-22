use super::Rect;

/// 链接类型
#[derive(Debug, Clone, PartialEq)]
pub enum LinkType {
    Page, // 内部页面链接
    Url, // 外部URL链接
    Unknown,
}

/// 链接
#[derive(Debug, Clone)]
pub struct Link {
    pub bounds: Rect,
    pub uri: Option<String>,
    pub page: Option<String>,
    pub link_type: LinkType,
}

/*#[derive(Debug, Clone)]
pub struct PdfLink {
    pub bounds: MuRect,
    pub uri: String,
    pub link_type: LinkType,
}*/