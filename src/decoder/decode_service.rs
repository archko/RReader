use anyhow::Result;
use log::{debug, info};
use std::path::{Path, PathBuf};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::hash::{Hash, Hasher};
use std::collections::{hash_map::DefaultHasher, VecDeque, HashSet};
use std::fs;

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, Link, PageInfo, Rect};
use crate::ui::utils::generate_thumbnail_hash;
use std::sync::Arc;

/// 可见性检查回调类型：传入页面索引，返回是否可见
pub type VisibilityChecker = Arc<dyn Fn(usize) -> bool + Send + Sync>;

/// 渲染页面请求
#[derive(Clone)]
pub struct RenderPage {
    pub key: String,
    pub page_info: PageInfo,
    pub crop: i32,
    pub priority: Priority,
    /// 可见性检查回调：传入页面bounds，返回是否可见
    pub visibility_checker: Option<VisibilityChecker>,
}

impl std::fmt::Debug for RenderPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPage")
            .field("key", &self.key)
            .field("page_info", &self.page_info)
            .field("crop", &self.crop)
            .field("priority", &self.priority)
            .field("has_visibility_checker", &self.visibility_checker.is_some())
            .finish()
    }
}

impl Hash for RenderPage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl PartialEq for RenderPage {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.page_info.index == other.page_info.index
            && (self.page_info.width - other.page_info.width).abs() < 0.1
            && (self.page_info.height - other.page_info.height).abs() < 0.1
            && (self.page_info.scale - other.page_info.scale).abs() < 0.001
            && self.crop == other.crop
    }
}

impl Eq for RenderPage {}

/// 解码任务
pub enum DecodeTask {
    /// 加载文档
    LoadDocument {
        path: PathBuf,
        response_tx: Sender<Result<Vec<PageInfo>>>,
    },
    /// 批量渲染页面
    RenderPages {
        pages: Vec<RenderPage>,
    },
    /// 获取大纲
    GetOutline {
        response_tx: Sender<Result<Vec<crate::entity::OutlineItem>>>,
    },
    /// 获取页面文本
    GetPageText {
        page_index: usize,
        response_tx: Sender<Result<String>>,
    },
    /// 解析reflow数据（从指定页面开始的后续页面）
    ExtractReflowData {
        start_page: usize,
        response_tx: Sender<Result<Vec<crate::entity::ReflowEntry>>>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn save_cover_thumbnail(path: &Path, dec: &Box<dyn Decoder>, first_page: &PageInfo) {
        let path_str = path.to_string_lossy();
        let hash = generate_thumbnail_hash(&path_str);
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
                Ok((pixels, width, height)) => {
                    let rgba_img = image::RgbaImage::from_raw(width, height, pixels).unwrap();
                    let image = image::DynamicImage::ImageRgba8(rgba_img);
                    if fs::create_dir_all(&cache_dir).is_ok()
                        && image.save(&cache_path).is_ok() {
                        info!("[DecodeService] Saved thumbnail to {:?}", cache_path);
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
        let mut task_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut current_visible: HashSet<RenderPage> = HashSet::new();

        loop {
            // 1. 先检查是否有新任务（非阻塞）
            while let Ok(task) = task_rx.try_recv() {
                if Self::handle_task(
                    task,
                    &mut decoder,
                    &mut task_queue,
                    &mut current_visible,
                ) {
                    // 收到 Shutdown 信号
                    return;
                }
            }

            // 2. 处理队列中的一个任务
            if let Some(render_page) = task_queue.pop_front() {
                // 使用回调验证页面是否可见
                let is_visible = if let Some(ref checker) = render_page.visibility_checker {
                    checker(render_page.page_info.index)
                } else {
                    // 如果没有回调，回退到旧的检查方式
                    current_visible.contains(&render_page)
                };

                if !is_visible {
                    info!("[DecodeService] 跳过不可见页: page={}, key={}", 
                        render_page.page_info.index, render_page.key);
                    // 继续处理下一个任务
                    continue;
                }

                // 执行解码
                if let Some(ref dec) = decoder {
                    let start_time = Instant::now();
                    
                    match dec.render_page(&render_page.page_info, render_page.crop != 0) {
                        Ok((image_data, width, height)) => {
                            //std::thread::sleep(std::time::Duration::from_secs(2));
                            let links = dec.get_page_links(render_page.page_info.index)
                                .unwrap_or_default();

                            let duration = start_time.elapsed();
                            info!(
                                "[DecodeService] 页面 {} 解码完成，耗时: {:?}, links: {}",
                                render_page.page_info.index, duration, links.len()
                            );

                            let result = DecodeResult {
                                key: render_page.key.clone(),
                                page_info: render_page.page_info.clone(),
                                image_data,
                                image_width: width,
                                image_height: height,
                                links,
                            };

                            if result_tx.send(result).is_err() {
                                info!("[DecodeService] Result channel closed");
                                return;
                            }
                        }
                        Err(e) => {
                            info!("[DecodeService] 页面 {} 解码失败: {}", render_page.page_info.index, e);
                        }
                    }
                }
                
                // 解码完一个任务后，继续下一个循环（会先检查新任务）
                continue;
            }

            // 3. 队列为空，阻塞等待新任务
            match task_rx.recv() {
                Ok(task) => {
                    if Self::handle_task(
                        task,
                        &mut decoder,
                        &mut task_queue,
                        &mut current_visible,
                    ) {
                        // 收到 Shutdown 信号
                        break;
                    }
                }
                Err(_) => {
                    info!("[DecodeService] Task channel closed");
                    break;
                }
            }
        }
    }

    /// 处理单个任务，返回 true 表示应该退出循环
    fn handle_task(
        task: DecodeTask,
        decoder: &mut Option<Box<dyn Decoder>>,
        task_queue: &mut VecDeque<RenderPage>,
        current_visible: &mut HashSet<RenderPage>,
    ) -> bool {
        match task {
            DecodeTask::LoadDocument { path, response_tx } => {
                info!("[DecodeService] Loading document: {:?}", path);
                match PdfDecoder::open(&path) {
                    Ok(pdf_decoder) => {
                        let boxed_decoder = Box::new(pdf_decoder);
                        let pages_result = boxed_decoder.get_all_pages();
                        *decoder = Some(boxed_decoder);
                        let first_page = if let Ok(ref pages) = pages_result {
                            if !pages.is_empty() {
                                Some(pages[0].clone())
                            } else {
                                None
                            }
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
                false
            }
            DecodeTask::RenderPages { pages } => {
                debug!("[DecodeService] 收到批量渲染任务: {} 页", pages.len());
                
                // 1. 更新当前可见页集合（用于后续验证）
                current_visible.clear();
                current_visible.extend(pages.iter().cloned());

                // 2. 将新任务加入队列（去重：检查队列中是否已存在相同key的任务）
                for page in pages {
                    let already_queued = task_queue.iter().any(|p| p.key == page.key);
                    if !already_queued {
                        debug!("[DecodeService] 加入队列: page={}, key={}", page.page_info.index, page.key);
                        task_queue.push_back(page);
                    } else {
                        info!("[DecodeService] 跳过重复任务: page={}, key={}", page.page_info.index, page.key);
                    }
                }
                
                info!("[DecodeService] 当前队列长度: {}, 可见页数: {}", 
                    task_queue.len(), current_visible.len());
                false
            }
            DecodeTask::GetOutline { response_tx } => {
                if let Some(ref dec) = decoder {
                    let outline_result = dec.get_outline_items();
                    let _ = response_tx.send(outline_result);
                } else {
                    let _ = response_tx.send(Ok(Vec::new()));
                }
                false
            }
            DecodeTask::GetPageText { page_index, response_tx } => {
                if let Some(ref dec) = decoder {
                    let text_result = dec.get_page_text(page_index);
                    let _ = response_tx.send(text_result);
                } else {
                    let _ = response_tx.send(Err(anyhow::anyhow!("No decoder")));
                }
                false
            }
            DecodeTask::ExtractReflowData { start_page, response_tx } => {
                if let Some(ref dec) = decoder {
                    let reflow_result = dec.get_reflow_from_page(start_page);
                    let _ = response_tx.send(reflow_result);
                } else {
                    let _ = response_tx.send(Err(anyhow::anyhow!("No decoder")));
                }
                false
            }
            DecodeTask::Shutdown => {
                info!("[DecodeService] Shutting down decode thread");
                true
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

    /// 获取页面文本（同步等待）
    pub fn get_page_text(&self, page_index: usize) -> Result<String> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::GetPageText { page_index, response_tx })
            .map_err(|e| anyhow::anyhow!("Failed to send page text task: {}", e))?;

        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive page text response: {}", e))?
    }

    /// 从指定页面开始获取后续页面的reflow数据
    pub fn get_reflow_from_page(&self, start_page: usize) -> Result<Vec<crate::entity::ReflowEntry>> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::ExtractReflowData {
                start_page,
                response_tx
            })
            .map_err(|e| anyhow::anyhow!("Failed to send reflow task: {}", e))?;

        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive reflow response: {}", e))?
    }

    /// 批量提交渲染任务（异步，不等待）
    pub fn render_pages(&self, pages: Vec<RenderPage>) {
        if !pages.is_empty() {
            let _ = self.task_sender.send(DecodeTask::RenderPages { pages });
        }
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

impl Default for DecodeService {
    fn default() -> Self {
        Self::new()
    }
}
