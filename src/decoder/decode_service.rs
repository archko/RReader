use anyhow::Result;
use image::DynamicImage;
use std::rc::Rc;
use std::path::Path;
use std::collections::VecDeque;
use std::cell::RefCell;

use crate::cache::PageCache;
use crate::decoder::{Decoder, PageInfo};
use crate::decoder::pdf::PdfDecoder;

// 解码请求类型
pub enum DecodeRequest {
    RenderFullPage { 
        page_info: PageInfo, 
        crop: i32,
        callback: Box<dyn FnOnce(Result<DynamicImage>)>,
    },
}

pub struct DecodeService {
    pub(crate) decoder: Option<Rc<dyn Decoder>>,
    cache: Rc<PageCache>,
    request_queue: RefCell<VecDeque<DecodeRequest>>,
}

impl DecodeService {
    pub fn new() -> Self {
        Self {
            decoder: None,
            cache: Rc::new(PageCache::new(80, 200)),
            request_queue: RefCell::new(VecDeque::new()),
        }
    }

    pub fn load_pdf<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let decoder = Rc::new(PdfDecoder::open(path)?);
        self.cache.clear();
        self.decoder = Some(decoder);
        Ok(())
    }

    // 将解码请求加入队列
    pub fn render_full_page<F>(&self, page_info: PageInfo, crop: i32, callback: F) 
    where 
        F: FnOnce(Result<DynamicImage>) + 'static,
    {
        let request = DecodeRequest::RenderFullPage {
            page_info,
            crop,
            callback: Box::new(callback),
        };
        
        self.request_queue.borrow_mut().push_back(request);
    }

    // 处理队列中的一个解码请求
    pub fn process_next_request(&self) -> bool {
        if let Some(request) = self.request_queue.borrow_mut().pop_front() {
            match request {
                DecodeRequest::RenderFullPage { page_info, crop, callback } => {
                    let result = self.decoder.as_ref()
                        .ok_or_else(|| anyhow::anyhow!("No decoder available"))
                        .and_then(|decoder| {
                            decoder.render_page(&page_info, crop != 0)
                        });
                    callback(result);
                }
            }
            true
        } else {
            false // 队列为空
        }
    }

    // 处理队列中的所有解码请求
    pub fn process_all_requests(&self) {
        while self.process_next_request() {
            // 继续处理直到队列为空
        }
    }
    
    pub fn page_count(&self) -> usize {
        self.decoder
            .as_ref()
            .map(|decoder| decoder.page_count())
            .unwrap_or(0)
    }

    pub fn destroy(&mut self) {
        self.decoder = None;
        self.cache.clear();
    }
}