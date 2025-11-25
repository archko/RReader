use anyhow::Result;
use log::{debug, info};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use crate::cache::PageCache;
use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, PageInfo};

// 解码请求类型
pub enum DecodeRequest {
    RenderFullPage {
        page_info: PageInfo,
        crop: i32,
        callback: Box<dyn FnOnce(Result<Image>)>,
    },
}

pub struct DecodeService {
    pub(crate) decoder: Option<Rc<dyn Decoder>>,
    pub cache: Rc<PageCache>,
    request_queue: RefCell<VecDeque<DecodeRequest>>,
}

impl DecodeService {
    pub fn new() -> Self {
        Self {
            decoder: None,
            cache: Rc::new(PageCache::new(8, 20)),
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
        F: FnOnce(Result<Image>) + 'static,
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
                DecodeRequest::RenderFullPage {
                    page_info,
                    crop,
                    callback,
                } => {
                    let start_time = Instant::now();
                    let result = self
                        .decoder
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("No decoder available"))
                        .and_then(|decoder| decoder.render_page(&page_info, crop != 0));

                    let duration = start_time.elapsed();
                    if let Ok(image) = result {
                        //self.cache.put_page_image(page_info.index, page_info.scale, image);
                        let slint_image = Self::convert_to_slint_image(&image);
                        self.cache
                            .put_page_image(page_info.index, page_info.scale, slint_image);
                        info!(
                            "[DecodeService] 页面 {} 渲染并缓存完成，耗时: {:?}",
                            page_info.index, duration
                        );
                        //callback(Ok(slint_image));
                    } else if let Err(e) = result {
                        debug!("[DecodeService] 页面 {} 渲染失败: {}", page_info.index, e);
                    }
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
        debug!("[DecodeService] Destroying decoder service and clearing queue");

        self.request_queue.borrow_mut().clear();
        self.decoder = None;
        self.cache.clear();
    }

    pub fn convert_to_slint_image(image: &image::DynamicImage) -> Image {
        //let start_time = Instant::now();
        /*debug!(
            "[STATE] Converting image with dimensions: {}x{}",
            image.width(),
            image.height()
        );*/
        let rgba_image = image.to_rgba8();
        let (width, height) = rgba_image.dimensions();

        let slint_image = Image::from_rgba8_premultiplied(
            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba_image, width, height),
        );
        //let duration = start_time.elapsed();
        //info!("[STATE] Successfully converted image to Slint image，耗时: {:?}", duration);
        slint_image
    }
}
