use anyhow::Result;
use log::{debug, info};
use std::path::{Path, PathBuf};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use image::GenericImageView;

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, Link, PageInfo};

/// 解码任务
pub enum DecodeTask {
    /// 加载文档
    LoadDocument {
        path: PathBuf,
        response_tx: Sender<Result<Vec<PageInfo>>>,
    },
    /// 渲染页面
    RenderPage {
        key: String,
        page_info: PageInfo,
        crop: i32,
        priority: Priority,
    },
    /// 获取大纲
    GetOutline {
        response_tx: Sender<Result<Vec<crate::entity::OutlineItem>>>,
    },
    /// 关闭服务
    Shutdown,
}

/// 解码结果（原始数据，可以跨线程传递）
pub struct DecodeResult {
    pub key: String,
    pub page_info: PageInfo,
    pub image_data: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub links: Vec<Link>,
}

pub enum Priority {
    Thumbnail = 0, // 最高优先级
    FullImage = 1, // 中优先级
    Cropped = 2,   // 低优先级
}

/// 解码服务 - 单线程解码，通过channel通信
pub struct DecodeService {
    task_sender: Sender<DecodeTask>,
    result_receiver: Mutex<Receiver<DecodeResult>>,
    decode_thread: Option<JoinHandle<()>>,
}

impl DecodeService {
    /// 保存封面缩略图
    fn save_cover_thumbnail(path: &PathBuf, dec: &Box<dyn Decoder>, first_page: &PageInfo) {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some(data_dir) = dirs::data_dir() {
            let cache_dir = data_dir.join("RReader").join("images");
            let cache_path = cache_dir.join(format!("{}.png", hash));
            if cache_path.exists() {
                info!("[DecodeService] Cover thumbnail already exists: {:?}", cache_path);
                return;
            }
            // 计算缩放到最大 300 像素的 scale
            let max_original = first_page.width.max(first_page.height);
            let effective_scale = 300.0 / max_original;
            let new_page_info = PageInfo {
                index: first_page.index,
                width: first_page.width,
                height: first_page.height,
                scale: effective_scale / 2.0, // 因为内部会乘以 2.0 (DPI scale)
                crop_bounds: first_page.crop_bounds,
            };
            match dec.render_page(&new_page_info, false) {
                Ok(image) => {
                    if fs::create_dir_all(&cache_dir).is_ok() {
                        if image.save(&cache_path).is_ok() {
                            info!("[DecodeService] Saved thumbnail to {:?}", cache_path);
                        }
                    }
                }
                Err(e) => {
                    info!("[DecodeService] Failed to render cover: {}", e);
                }
            }
        }
    }
}

impl DecodeService {
    pub fn new() -> Self {
        let (task_tx, task_rx) = unbounded::<DecodeTask>();
        let (result_tx, result_rx) = unbounded::<DecodeResult>();

        // 启动解码线程
        let decode_thread = thread::spawn(move || {
            Self::decode_loop(task_rx, result_tx);
        });

        Self {
            task_sender: task_tx,
            result_receiver: Mutex::new(result_rx),
            decode_thread: Some(decode_thread),
        }
    }

    /// 解码线程主循环
    fn decode_loop(task_rx: Receiver<DecodeTask>, result_tx: Sender<DecodeResult>) {
        let mut decoder: Option<Box<dyn Decoder>> = None;

        loop {
            match task_rx.recv() {
                Ok(task) => match task {
                    DecodeTask::LoadDocument { path, response_tx } => {
                        info!("[DecodeService] Loading document: {:?}", path);
                        match PdfDecoder::open(&path) {
                            Ok(pdf_decoder) => {
                                let boxed_decoder = Box::new(pdf_decoder);
                                let pages_result = boxed_decoder.get_all_pages();
                                decoder = Some(boxed_decoder);
                                let first_page = if pages_result.is_ok() && !pages_result.as_ref().unwrap().is_empty() {
                                    Some(pages_result.as_ref().unwrap()[0].clone())
                                } else {
                                    None
                                };
                                let _ = response_tx.send(pages_result);
                                if let Some(fp) = first_page {
                                    if let Some(ref dec) = decoder {
                                        Self::save_cover_thumbnail(&path, dec, &fp);
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = response_tx.send(Err(e));
                            }
                        }
                    }
                    DecodeTask::RenderPage {
                        key,
                        page_info,
                        crop,
                        priority,
                    } => {
                        if let Some(ref dec) = decoder {
                            let start_time = Instant::now();
                            
                            match dec.render_page(&page_info, crop != 0) {
                                Ok(image) => {
                                    let links = dec.get_page_links(page_info.index).unwrap_or_default();
                                    
                                    // 将DynamicImage转换为原始字节数据
                                    let rgba_image = image.to_rgba8();
                                    let (width, height) = rgba_image.dimensions();
                                    let image_data = rgba_image.into_raw();
                                    
                                    let duration = start_time.elapsed();
                                    info!(
                                        "[DecodeService] 页面 {} 解码完成，耗时: {:?}, links: {}",
                                        page_info.index, duration, links.len()
                                    );

                                    let result = DecodeResult {
                                        key,
                                        page_info,
                                        image_data,
                                        image_width: width,
                                        image_height: height,
                                        links,
                                    };

                                    if result_tx.send(result).is_err() {
                                        info!("[DecodeService] Result channel closed");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    info!("[DecodeService] 页面 {} 解码失败: {}", page_info.index, e);
                                }
                            }
                        }
                    }
                    DecodeTask::GetOutline { response_tx } => {
                        if let Some(ref dec) = decoder {
                            let outline_result = dec.get_outline_items();
                            let _ = response_tx.send(outline_result);
                        } else {
                            let _ = response_tx.send(Ok(Vec::new()));
                        }
                    }
                    DecodeTask::Shutdown => {
                        info!("[DecodeService] Shutting down decode thread");
                        break;
                    }
                },
                Err(_) => {
                    info!("[DecodeService] Task channel closed");
                    break;
                }
            }
        }
    }

    /// 加载PDF文档（同步等待）
    pub fn load_pdf<P: AsRef<Path>>(&self, path: P) -> Result<Vec<PageInfo>> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::LoadDocument {
                path: path.as_ref().to_path_buf(),
                response_tx,
            })
            .map_err(|e| anyhow::anyhow!("Failed to send load task: {}", e))?;

        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive load response: {}", e))?
    }

    /// 获取大纲（同步等待）
    pub fn get_outline(&self) -> Result<Vec<crate::entity::OutlineItem>> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::GetOutline { response_tx })
            .map_err(|e| anyhow::anyhow!("Failed to send outline task: {}", e))?;

        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive outline response: {}", e))?
    }

    /// 提交渲染任务（异步，不等待）
    pub fn render_page(&self, key: String, page_info: PageInfo, crop: i32, priority: Priority) {
        let _ = self.task_sender.send(DecodeTask::RenderPage {
            key,
            page_info,
            crop,
            priority,
        });
    }

    /// 尝试接收解码结果（非阻塞）
    pub fn try_recv_result(&self) -> Option<DecodeResult> {
        self.result_receiver.lock().unwrap().try_recv().ok()
    }

    /// 关闭服务
    pub fn destroy(&mut self) {
        info!("[DecodeService] Destroying decoder service");
        let _ = self.task_sender.send(DecodeTask::Shutdown);
    }
}

impl Drop for DecodeService {
    fn drop(&mut self) {
        self.destroy();
        if let Some(handle) = self.decode_thread.take() {
            let _ = handle.join();
        }
    }
}
