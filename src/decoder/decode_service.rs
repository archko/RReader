use anyhow::Result;
use log::{debug, info};
use std::path::{Path, PathBuf};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Instant, Duration};
use std::collections::{VecDeque, HashSet};
use std::fs;

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, Link, PageInfo, Rect};
use crate::ui::utils::generate_thumbnail_hash;
use std::sync::Arc;

/// 任务类型（用于三队列优先级调度）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskType {
    Page = 0,  // 最高优先级（缩略图）
    Node = 1,  // 中优先级（瓦片）
    Crop = 2,  // 低优先级（裁剪检测）
}

/// 可见性检查回调类型：传入页面索引，返回是否可见
pub type VisibilityChecker = Arc<dyn Fn(usize) -> bool + Send + Sync>;

/// 渲染页面请求
#[derive(Clone)]
pub struct RenderPage {
    pub key: String,
    pub page_info: PageInfo,
    pub crop: i32,
    pub task_type: TaskType,
    /// 可见性检查回调：执行前检查页面是否仍需要渲染
    pub visibility_checker: Option<VisibilityChecker>,
}

impl std::fmt::Debug for RenderPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPage")
            .field("key", &self.key)
            .field("page_info", &self.page_info)
            .field("crop", &self.crop)
            .field("task_type", &self.task_type)
            .field("has_visibility_checker", &self.visibility_checker.is_some())
            .finish()
    }
}

/// 解码任务（管理员任务通过 channel 传递）
enum DecodeTask {
    /// 加载文档
    LoadDocument {
        path: PathBuf,
    },
    /// 批量渲染任务（内部分发到三队列）
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
    /// 解析reflow数据
    ExtractReflowData {
        start_page: usize,
        response_tx: Sender<Result<Vec<crate::entity::ReflowEntry>>>,
    },
    /// 关闭服务
    Shutdown,
}

/// 解码结果
pub struct DecodeResult {
    pub key: String,
    pub page_info: PageInfo,
    pub image_data: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub links: Vec<Link>,
}

/// 解码服务 - 三队列优先级调度，单线程解码
///
/// 优先级顺序：Page(缩略图) > Node(瓦片) > Crop(裁剪检测)
/// 内部维护三个独立队列，每次 selectNextTask 按优先级 poll。
pub struct DecodeService {
    task_sender: Sender<DecodeTask>,
    result_receiver: Mutex<Receiver<DecodeResult>>,
    load_result_sender: Sender<Result<Vec<PageInfo>>>,
    load_result_receiver: Mutex<Receiver<Result<Vec<PageInfo>>>>,
    decode_thread: Option<JoinHandle<()>>,
    /// 是否有解码任务正在处理
    work_pending: AtomicBool,
    /// 解码线程是否发生过 panic
    error_occurred: Arc<AtomicBool>,
}

impl DecodeService {
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
            let max_original = first_page.width.max(first_page.height);
            let effective_scale = 300.0 / max_original;
            let new_page_info = PageInfo {
                index: first_page.index,
                width: first_page.width,
                height: first_page.height,
                scale: effective_scale / 2.0,
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

    fn decode_loop(
        task_rx: Receiver<DecodeTask>,
        result_tx: Sender<DecodeResult>,
        load_result_tx: Sender<Result<Vec<PageInfo>>>,
        error_flag: Arc<AtomicBool>,
    ) {
        let mut decoder: Option<Box<dyn Decoder>> = None;
        // 三优先级队列
        let mut page_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut node_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut crop_queue: VecDeque<RenderPage> = VecDeque::new();
        // 已提交任务的 key 去重
        let mut pending_keys: HashSet<String> = HashSet::new();

        loop {
            // 1. 接收管理员任务（非阻塞）
            while let Ok(task) = task_rx.try_recv() {
                match Self::safe_handle_task(
                    task,
                    &mut decoder,
                    &mut page_queue,
                    &mut node_queue,
                    &mut crop_queue,
                    &mut pending_keys,
                    &load_result_tx,
                    &error_flag,
                ) {
                    TaskHandled::Exit => return,
                    TaskHandled::Continue => {}
                }
            }

            // 2. 按优先级选择下一个任务
            let task = page_queue.pop_front()
                .or_else(|| node_queue.pop_front())
                .or_else(|| crop_queue.pop_front());

            if let Some(render_page) = task {
                // 可见性检查
                let is_visible = if let Some(ref checker) = render_page.visibility_checker {
                    checker(render_page.page_info.index)
                } else {
                    pending_keys.contains(&render_page.key)
                };

                if !is_visible {
                    pending_keys.remove(&render_page.key);
                    continue;
                }

                // 执行解码
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

                pending_keys.remove(&render_page.key);

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
                    Ok(None) => {}
                    Err(panic_info) => {
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
                        page_queue.clear();
                        node_queue.clear();
                        crop_queue.clear();
                        pending_keys.clear();
                    }
                }
            } else {
                // 3. 队列全空，阻塞等待新任务
                match task_rx.recv() {
                    Ok(task) => {
                        match Self::safe_handle_task(
                            task,
                            &mut decoder,
                            &mut page_queue,
                            &mut node_queue,
                            &mut crop_queue,
                            &mut pending_keys,
                            &load_result_tx,
                            &error_flag,
                        ) {
                            TaskHandled::Exit => break,
                            TaskHandled::Continue => {}
                        }
                    }
                    Err(_) => {
                        info!("Task channel closed");
                        break;
                    }
                }
            }
        }
    }

    enum TaskHandled {
        Exit,
        Continue,
    }

    fn safe_handle_task(
        task: DecodeTask,
        decoder: &mut Option<Box<dyn Decoder>>,
        page_queue: &mut VecDeque<RenderPage>,
        node_queue: &mut VecDeque<RenderPage>,
        crop_queue: &mut VecDeque<RenderPage>,
        pending_keys: &mut HashSet<String>,
        load_result_tx: &Sender<Result<Vec<PageInfo>>>,
        error_flag: &Arc<AtomicBool>,
    ) -> TaskHandled {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::handle_task(task, decoder, page_queue, node_queue, crop_queue, pending_keys, load_result_tx)
        })) {
            Ok(should_exit) => {
                if should_exit { TaskHandled::Exit } else { TaskHandled::Continue }
            }
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
                page_queue.clear();
                node_queue.clear();
                crop_queue.clear();
                pending_keys.clear();
                TaskHandled::Continue
            }
        }
    }

    fn handle_task(
        task: DecodeTask,
        decoder: &mut Option<Box<dyn Decoder>>,
        page_queue: &mut VecDeque<RenderPage>,
        node_queue: &mut VecDeque<RenderPage>,
        crop_queue: &mut VecDeque<RenderPage>,
        pending_keys: &mut HashSet<String>,
        load_result_tx: &Sender<Result<Vec<PageInfo>>>,
    ) -> bool {
        match task {
            DecodeTask::LoadDocument { path } => {
                info!("Loading document: {:?}", path);
                match PdfDecoder::open(&path) {
                    Ok(pdf_decoder) => {
                        let boxed_decoder = Box::new(pdf_decoder);
                        let pages_result = boxed_decoder.get_all_pages();
                        *decoder = Some(boxed_decoder);
                        let first_page = if let Ok(ref pages) = pages_result {
                            if !pages.is_empty() { Some(pages[0].clone()) } else { None }
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
                        let _ = load_result_tx.send(Err(e));
                    }
                }
                false
            }
            DecodeTask::RenderPages { pages } => {
                debug!("收到批量渲染任务: {} 页", pages.len());
                for page in pages {
                    if pending_keys.contains(&page.key) {
                        debug!("跳过重复任务: key={}", page.key);
                        continue;
                    }
                    pending_keys.insert(page.key.clone());
                    match page.task_type {
                        TaskType::Page => {
                            debug!("加入Page队列: key={}", page.key);
                            page_queue.push_back(page);
                        }
                        TaskType::Node => {
                            debug!("加入Node队列: key={}", page.key);
                            node_queue.push_back(page);
                        }
                        TaskType::Crop => {
                            crop_queue.push_back(page);
                        }
                    }
                }
                debug!("队列状态 - Page: {}, Node: {}, Crop: {}, pending: {}",
                    page_queue.len(), node_queue.len(), crop_queue.len(), pending_keys.len());
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

    // ===== 公开 API =====

    pub fn load_pdf<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.clear_error();
        self.task_sender
            .send(DecodeTask::LoadDocument {
                path: path.as_ref().to_path_buf(),
            })
            .map_err(|e| anyhow::anyhow!("Failed to send load task: {}", e))
    }

    pub fn get_outline(&self) -> Result<Vec<crate::entity::OutlineItem>> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::GetOutline { response_tx })
            .map_err(|e| anyhow::anyhow!("Failed to send outline task: {}", e))?;
        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive outline response: {}", e))?
    }

    pub fn get_page_text(&self, page_index: usize) -> Result<String> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::GetPageText { page_index, response_tx })
            .map_err(|e| anyhow::anyhow!("Failed to send page text task: {}", e))?;
        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive page text response: {}", e))?
    }

    pub fn get_reflow_from_page(&self, start_page: usize) -> Result<Vec<crate::entity::ReflowEntry>> {
        let (response_tx, response_rx) = unbounded();
        self.task_sender
            .send(DecodeTask::ExtractReflowData { start_page, response_tx })
            .map_err(|e| anyhow::anyhow!("Failed to send reflow task: {}", e))?;
        response_rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive reflow response: {}", e))?
    }

    /// 批量提交渲染任务（内部分发到对应优先级队列）
    pub fn render_pages(&self, pages: Vec<RenderPage>) {
        if !pages.is_empty() {
            self.work_pending.store(true, Ordering::Release);
            let _ = self.task_sender.send(DecodeTask::RenderPages { pages });
        }
    }

    pub fn is_work_pending(&self) -> bool {
        self.work_pending.load(Ordering::Acquire)
    }

    pub fn clear_work_pending(&self) {
        self.work_pending.store(false, Ordering::Release);
    }

    pub fn has_error(&self) -> bool {
        self.error_occurred.load(Ordering::Acquire)
    }

    pub fn clear_error(&self) {
        self.error_occurred.store(false, Ordering::Release);
    }

    pub fn try_recv_result(&self) -> Option<DecodeResult> {
        self.result_receiver.lock().unwrap().try_recv().ok()
    }

    pub fn try_recv_load_result(&self) -> Option<Result<Vec<PageInfo>>> {
        self.load_result_receiver.lock().unwrap().try_recv().ok()
    }

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
