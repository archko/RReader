use anyhow::Result;
use log::{debug, info};
use std::path::{Path, PathBuf};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{ Instant, Duration};
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
    load_result_sender: Sender<Result<Vec<PageInfo>>>,
    load_result_receiver: Mutex<Receiver<Result<Vec<PageInfo>>>>,
    decode_thread: Option<JoinHandle<()>>,
    /// 是否有解码任务正在处理（用于主线程避免空轮询）
    work_pending: AtomicBool,
    /// 解码线程是否发生过 panic 崩溃（与线程共享）
    error_occurred: Arc<AtomicBool>,
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
                info!("Cover thumbnail already exists: {:?}", cache_path);
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
                        info!("Saved thumbnail to {:?}", cache_path);
                    }
                }
                Err(e) => {
                    info!("Failed to render cover: {}", e);
                }
            }
        }
    }
}

impl DecodeService {
    pub fn new() -> Self {
        let (task_tx, task_rx) = unbounded::<DecodeTask>();
        let (result_tx, result_rx) = unbounded::<DecodeResult>();
        let (load_result_tx, load_result_rx) = unbounded::<Result<Vec<PageInfo>>>();
        let error_flag = Arc::new(AtomicBool::new(false));
        let error_flag_clone = Arc::clone(&error_flag);

        // 启动解码线程
        let load_result_tx_for_thread = load_result_tx.clone();
        let decode_thread = thread::spawn(move || {
            Self::decode_loop(task_rx, result_tx, load_result_tx_for_thread, error_flag_clone);
        });

        Self {
            task_sender: task_tx,
            result_receiver: Mutex::new(result_rx),
            load_result_sender: load_result_tx,
            load_result_receiver: Mutex::new(load_result_rx),
            decode_thread: Some(decode_thread),
            work_pending: AtomicBool::new(false),
            error_occurred: error_flag,
        }
    }

    /// 解码线程主循环（带 panic 恢复）
    fn decode_loop(task_rx: Receiver<DecodeTask>, result_tx: Sender<DecodeResult>, load_result_tx: Sender<Result<Vec<PageInfo>>>, error_flag: Arc<AtomicBool>) {
        let mut decoder: Option<Box<dyn Decoder>> = None;
        let mut task_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut current_visible: HashSet<RenderPage> = HashSet::new();

        loop {
            // 1. 先检查是否有新任务（非阻塞，用 safe 版本防止 panic 杀死线程）
            while let Ok(task) = task_rx.try_recv() {
                if Self::safe_handle_task(
                    task,
                    &mut decoder,
                    &mut task_queue,
                    &mut current_visible,
                    &load_result_tx,
                    &error_flag,
                ) {
                    return;
                }
            }

            // 2. 处理队列中的一个任务
            if let Some(render_page) = task_queue.pop_front() {
                // 使用回调验证页面是否可见
                let is_visible = if let Some(ref checker) = render_page.visibility_checker {
                    checker(render_page.page_info.index)
                } else {
                    current_visible.contains(&render_page)
                };

                if !is_visible {
                    task_queue.clear();
                    continue;
                }

                // 执行解码并用 catch_unwind 保护，防止 MuPDF 内部崩溃杀死线程
                let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(ref dec) = decoder {
                        let start_time = Instant::now();
                        match dec.render_page(&render_page.page_info, render_page.crop != 0) {
                            Ok((image_data, width, height)) => {
                                let links = dec.get_page_links(render_page.page_info.index)
                                    .unwrap_or_default();
                                let duration = start_time.elapsed();
                                info!("页面 {} 解码完成，耗时: {:?}, links: {}",
                                    render_page.page_info.index, duration, links.len());
                                Some((render_page.key.clone(), render_page.page_info.clone(),
                                      image_data, width, height, links))
                            }
                            Err(e) => {
                                info!("页面 {} 解码失败: {}", render_page.page_info.index, e);
                                None
                            }
                        }
                    } else {
                        None
                    }
                }));

                match render_result {
                    Ok(Some((key, page_info, image_data, width, height, links))) => {
                        let result = DecodeResult {
                            key,
                            page_info,
                            image_data,
                            image_width: width,
                            image_height: height,
                            links,
                        };
                        if result_tx.send(result).is_err() {
                            info!("Result channel closed");
                            return;
                        }
                    }
                    Ok(None) => {
                        // 正常失败（如解码错误），继续处理下一个
                    }
                    Err(panic_info) => {
                        // 解码线程 panic！无效化解码器防止重复崩溃
                        let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic".to_string()
                        };
                        log::error!("解码器渲染时崩溃: {}. 已无效化解码器，请重新打开文档。", msg);
                        error_flag.store(true, Ordering::Release);
                        decoder = None;
                        task_queue.clear();
                        current_visible.clear();
                    }
                }

                continue;
            }

            // 3. 队列为空，阻塞等待新任务
            match task_rx.recv() {
                Ok(task) => {
                    if Self::safe_handle_task(
                        task,
                        &mut decoder,
                        &mut task_queue,
                        &mut current_visible,
                        &load_result_tx,
                        &error_flag,
                    ) {
                        break;
                    }
                }
                Err(_) => {
                    info!("Task channel closed");
                    break;
                }
            }
        }
    }

    /// 安全版本的 handle_task，用 catch_unwind 防止 panic 杀死解码线程
    fn safe_handle_task(
        task: DecodeTask,
        decoder: &mut Option<Box<dyn Decoder>>,
        task_queue: &mut VecDeque<RenderPage>,
        current_visible: &mut HashSet<RenderPage>,
        load_result_tx: &Sender<Result<Vec<PageInfo>>>,
        error_flag: &Arc<AtomicBool>,
    ) -> bool {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::handle_task(task, decoder, task_queue, current_visible, load_result_tx)
        })) {
            Ok(should_exit) => should_exit,
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                log::error!("解码器处理任务时崩溃: {}. 已无效化解码器，请重新打开文档。", msg);
                error_flag.store(true, Ordering::Release);
                *decoder = None;
                task_queue.clear();
                current_visible.clear();
                false // 不退出线程，等待新的 LoadDocument 请求
            }
        }
    }

    /// 处理单个任务，返回 true 表示应该退出循环
    fn handle_task(
        task: DecodeTask,
        decoder: &mut Option<Box<dyn Decoder>>,
        task_queue: &mut VecDeque<RenderPage>,
        current_visible: &mut HashSet<RenderPage>,
        load_result_tx: &Sender<Result<Vec<PageInfo>>>,
    ) -> bool {
        match task {
            DecodeTask::LoadDocument { path } => {
                info!("Loading document: {:?}", path);
                match PdfDecoder::open(&path) {
                    Ok(pdf_decoder) => {
                        info!("PdfDecoder::open 成功");
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
                        let _ = load_result_tx.send(pages_result);
                        if let Some(fp) = first_page {
                            if let Some(ref dec) = decoder {
                                Self::save_cover_thumbnail(&path, dec, &fp);
                            }
                        }
                    }
                    Err(e) => {
                        info!("PdfDecoder::open 失败: {}", e);
                        let _ = load_result_tx.send(Err(e));
                    }
                }
                false
            }
            DecodeTask::RenderPages { pages } => {
                debug!("收到批量渲染任务: {} 页", pages.len());
                
                // 1. 更新当前可见页集合（用于后续验证）
                current_visible.clear();
                current_visible.extend(pages.iter().cloned());

                // 2. 将新任务加入队列（去重：检查队列中是否已存在相同key的任务）
                for page in pages {
                    let already_queued = task_queue.iter().any(|p| p.key == page.key);
                    if !already_queued {
                        debug!("加入队列: page={}, key={}", page.page_info.index, page.key);
                        task_queue.push_back(page);
                    } else {
                        info!("跳过重复任务: page={}, key={}", page.page_info.index, page.key);
                    }
                }
                
                info!("当前队列长度: {}, 可见页数: {}", 
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
                info!("Shutting down decode thread");
                true
            }
        }
    }

    /// 加载PDF文档（异步）
    pub fn load_pdf<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        // 清除之前的错误标志，新文档从头开始
        self.clear_error();
        self.task_sender
            .send(DecodeTask::LoadDocument {
                path: path.as_ref().to_path_buf(),
            })
            .map_err(|e| anyhow::anyhow!("Failed to send load task: {}", e))
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
            self.work_pending.store(true, Ordering::Release);
            let _ = self.task_sender.send(DecodeTask::RenderPages { pages });
        }
    }

    /// 是否有未处理完的解码任务
    pub fn is_work_pending(&self) -> bool {
        self.work_pending.load(Ordering::Acquire)
    }

    /// 标记解码任务已全部处理完毕
    pub fn clear_work_pending(&self) {
        self.work_pending.store(false, Ordering::Release);
    }

    /// 解码线程是否发生过 panic（如 MuPDF 内部崩溃）
    pub fn has_error(&self) -> bool {
        self.error_occurred.load(Ordering::Acquire)
    }

    /// 清除错误标志（重新打开文档前调用）
    pub fn clear_error(&self) {
        self.error_occurred.store(false, Ordering::Release);
    }

    /// 尝试接收解码结果（非阻塞）
    pub fn try_recv_result(&self) -> Option<DecodeResult> {
        self.result_receiver.lock().unwrap().try_recv().ok()
    }

    /// 尝试接收加载结果（非阻塞）
    pub fn try_recv_load_result(&self) -> Option<Result<Vec<PageInfo>>> {
        //info!("try_recv_load_result");
        self.load_result_receiver.lock().unwrap().try_recv().ok()
    }

    /// 关闭服务（发送 Shutdown 信号给解码线程）
    pub fn destroy(&self) {
        info!("Destroying decoder service");
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
