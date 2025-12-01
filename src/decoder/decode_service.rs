use anyhow::Result;
use image::DynamicImage;
use log::{debug, info};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, Link, PageInfo};

/// 解码结果
pub struct DecodeResult {
    pub image: DynamicImage,
    pub page_info: PageInfo,
    pub links: Vec<Link>,
}

pub struct DecodeTask {
    pub key: String,
    pub page_info: PageInfo,
    pub crop: i32,
    pub priority: Priority,
    pub callback: Box<dyn FnOnce(Result<DecodeResult>)>,
}

pub enum Priority {
    Thumbnail = 0, // 最高优先级
    FullImage = 1, // 中优先级
    Cropped = 2,   // 低优先级
}

pub struct DecodeService {
    pub(crate) decoder: Option<Rc<dyn Decoder>>,
    request_queue: RefCell<VecDeque<DecodeTask>>,
}

impl DecodeService {
    pub fn new() -> Self {
        Self {
            decoder: None,
            request_queue: RefCell::new(VecDeque::new()),
        }
    }

    pub fn load_pdf<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let decoder = Rc::new(PdfDecoder::open(path)?);
        self.decoder = Some(decoder);
        Ok(())
    }

    // 解码缩略图
    pub fn render_page(&self, decode_task: DecodeTask) {
        self.request_queue.borrow_mut().push_back(decode_task);
    }

    // 处理队列中的一个解码请求
    pub fn process_next_request(&self) -> bool {
        if let Some(request) = self.request_queue.borrow_mut().pop_front() {
            let start_time = Instant::now();

            let decoder = match self.decoder.as_ref() {
                Some(decoder) => decoder,
                None => {
                    info!(
                        "[DecodeService] 页面 {} 渲染失败: No decoder available",
                        request.page_info.index
                    );
                    return false;
                }
            };

            let result = decoder.render_page(&request.page_info, request.crop != 0);

            let links = if let Ok(links) = decoder.get_page_links(request.page_info.index) {
                links
            } else {
                Vec::new()
            };

            let duration = start_time.elapsed();
            if let Ok(image) = result {
                info!(
                    "[DecodeService] 页面 {} 渲染并缓存完成，耗时: {:?}, links:{:?}",
                    request.page_info.index, duration, links.len()
                );

                let decode_result = DecodeResult {
                    image,
                    page_info: request.page_info.clone(),
                    links,
                };

                (request.callback)(Ok(decode_result));
            } else if let Err(e) = result {
                info!(
                    "[DecodeService] 页面 {} 渲染失败: {}",
                    request.page_info.index, e
                );
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
        info!("[DecodeService] Destroying decoder service and clearing queue");

        self.request_queue.borrow_mut().clear();
        self.decoder = None;
    }
}
