use anyhow::Result;
use log::{debug, info};
use std::path::{Path, PathBuf};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Instant, Duration};
use std::collections::VecDeque;
use std::fs;

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, Link, PageInfo, Rect};
use crate::ui::utils::generate_thumbnail_hash;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskType {
    Page = 0,
    Node = 1,
    Crop = 2,
}

pub trait DecodeCallback: Send + Sync {
    fn should_render(&self, page_index: usize) -> bool;
    fn on_completed(&self, result: DecodeResult);
    fn on_error(&self, page_index: usize);
}

pub type DecodeCallbackRef = Arc<dyn DecodeCallback>;

#[derive(Clone)]
pub struct RenderPage {
    pub key: String,
    pub page_info: PageInfo,
    pub crop: i32,
    pub task_type: TaskType,
    pub callback: Option<DecodeCallbackRef>,
}

impl std::fmt::Debug for RenderPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPage")
            .field("key", &self.key)
            .field("page_info", &self.page_info)
            .field("crop", &self.crop)
            .field("task_type", &self.task_type)
            .field("has_callback", &self.callback.is_some())
            .finish()
    }
}

pub enum DecodeTask {
    LoadDocument { path: PathBuf },
    RenderPages { pages: Vec<RenderPage> },
    GetOutline { response_tx: Sender<Result<Vec<crate::entity::OutlineItem>>> },
    GetPageText { page_index: usize, response_tx: Sender<Result<String>> },
    ExtractReflowData { start_page: usize, response_tx: Sender<Result<Vec<crate::entity::ReflowEntry>>> },
    Shutdown,
}

pub struct DecodeResult {
    pub key: String,
    pub page_info: PageInfo,
    pub image_data: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub links: Vec<Link>,
}

enum TaskHandled { Exit, Continue }

pub struct DecodeService {
    task_sender: Sender<DecodeTask>,
    load_result_sender: Sender<Result<Vec<PageInfo>>>,
    load_result_receiver: Mutex<Receiver<Result<Vec<PageInfo>>>>,
    decode_thread: Option<JoinHandle<()>>,
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
        let (load_result_tx, load_result_rx) = unbounded::<Result<Vec<PageInfo>>>();
        let error_flag = Arc::new(AtomicBool::new(false));
        let error_flag_clone = Arc::clone(&error_flag);
        let load_result_tx_for_thread = load_result_tx.clone();
        let decode_thread = thread::spawn(move || {
            Self::decode_loop(task_rx, load_result_tx_for_thread, error_flag_clone);
        });
        Self {
            task_sender: task_tx,
            load_result_sender: load_result_tx,
            load_result_receiver: Mutex::new(load_result_rx),
            decode_thread: Some(decode_thread),
            error_occurred: error_flag,
        }
    }

    fn decode_loop(
        task_rx: Receiver<DecodeTask>,
        load_result_tx: Sender<Result<Vec<PageInfo>>>,
        error_flag: Arc<AtomicBool>,
    ) {
        let mut decoder: Option<Box<dyn Decoder>> = None;
        let mut page_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut node_queue: VecDeque<RenderPage> = VecDeque::new();
        let mut crop_queue: VecDeque<RenderPage> = VecDeque::new();

        loop {
            while let Ok(task) = task_rx.try_recv() {
                match Self::safe_handle_task(
                    task, &mut decoder, &mut page_queue, &mut node_queue,
                    &mut crop_queue, &load_result_tx, &error_flag,
                ) {
                    TaskHandled::Exit => return,
                    TaskHandled::Continue => {}
                }
            }

            let task = page_queue.pop_front()
                .or_else(|| node_queue.pop_front())
                .or_else(|| crop_queue.pop_front());

            if let Some(render_page) = task {
                let should_render = render_page.callback
                    .as_ref()
                    .map_or(true, |cb| cb.should_render(render_page.page_info.index));

                if !should_render {
                    continue;
                }

                let duration;
                let render_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(ref dec) = decoder {
                        let start = Instant::now();
                        match dec.render_page(&render_page.page_info, render_page.crop != 0) {
                            Ok((image_data, width, height)) => {
                                let links = dec.get_page_links(render_page.page_info.index)
                                    .unwrap_or_default();
                                Some((render_page.key.clone(), render_page.page_info.clone(),
                                      image_data, width, height, links, start.elapsed()))
                            }
                            Err(e) => {
                                info!("页面 {} 解码失败: {}", render_page.page_info.index, e);
                                None
                            }
                        }
                    } else { None }
                }));

                match render_result {
                    Ok(Some((key, page_info, image_data, width, height, links, dur))) => {
                        duration = dur;
                        info!("页面 {} 解码完成，耗时: {:?}, links: {}",
                            page_info.index, duration, links.len());
                        if let Some(ref cb) = render_page.callback {
                            cb.on_completed(DecodeResult {
                                key, page_info, image_data,
                                image_width: width, image_height: height, links,
                            });
                        }
                    }
                    Ok(None) => {
                        if let Some(ref cb) = render_page.callback {
                            cb.on_error(render_page.page_info.index);
                        }
                    }
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
                    }
                }
            } else {
                match task_rx.recv() {
                    Ok(task) => {
                        match Self::safe_handle_task(
                            task, &mut decoder, &mut page_queue, &mut node_queue,
                            &mut crop_queue, &load_result_tx, &error_flag,
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

    fn safe_handle_task(
        task: DecodeTask, decoder: &mut Option<Box<dyn Decoder>>,
        page_queue: &mut VecDeque<RenderPage>, node_queue: &mut VecDeque<RenderPage>,
        crop_queue: &mut VecDeque<RenderPage>,
        load_result_tx: &Sender<Result<Vec<PageInfo>>>, error_flag: &Arc<AtomicBool>,
    ) -> TaskHandled {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::handle_task(task, decoder, page_queue, node_queue, crop_queue, load_result_tx)
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
                TaskHandled::Continue
            }
        }
    }

    fn handle_task(
        task: DecodeTask, decoder: &mut Option<Box<dyn Decoder>>,
        page_queue: &mut VecDeque<RenderPage>, node_queue: &mut VecDeque<RenderPage>,
        crop_queue: &mut VecDeque<RenderPage>,
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
                    match page.task_type {
                        TaskType::Page => page_queue.push_back(page),
                        TaskType::Node => node_queue.push_back(page),
                        TaskType::Crop => crop_queue.push_back(page),
                    }
                }
                debug!("队列状态 - Page: {}, Node: {}, Crop: {}",
                    page_queue.len(), node_queue.len(), crop_queue.len());
                false
            }
            DecodeTask::GetOutline { response_tx } => {
                if let Some(ref dec) = decoder {
                    let _ = response_tx.send(dec.get_outline_items());
                } else {
                    let _ = response_tx.send(Ok(Vec::new()));
                }
                false
            }
            DecodeTask::GetPageText { page_index, response_tx } => {
                if let Some(ref dec) = decoder {
                    let _ = response_tx.send(dec.get_page_text(page_index));
                } else {
                    let _ = response_tx.send(Err(anyhow::anyhow!("No decoder")));
                }
                false
            }
            DecodeTask::ExtractReflowData { start_page, response_tx } => {
                if let Some(ref dec) = decoder {
                    let _ = response_tx.send(dec.get_reflow_from_page(start_page));
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
            .send(DecodeTask::LoadDocument { path: path.as_ref().to_path_buf() })
            .map_err(|e| anyhow::anyhow!("Failed to send load task: {}", e))
    }

    pub fn get_outline(&self) -> Result<Vec<crate::entity::OutlineItem>> {
        let (tx, rx) = unbounded();
        self.task_sender.send(DecodeTask::GetOutline { response_tx: tx })
            .map_err(|e| anyhow::anyhow!("Failed to send outline task: {}", e))?;
        rx.recv().map_err(|e| anyhow::anyhow!("{}", e))?
    }

    pub fn get_page_text(&self, page_index: usize) -> Result<String> {
        let (tx, rx) = unbounded();
        self.task_sender.send(DecodeTask::GetPageText { page_index, response_tx: tx })
            .map_err(|e| anyhow::anyhow!("Failed to send page text task: {}", e))?;
        rx.recv().map_err(|e| anyhow::anyhow!("{}", e))?
    }

    pub fn get_reflow_from_page(&self, start_page: usize) -> Result<Vec<crate::entity::ReflowEntry>> {
        let (tx, rx) = unbounded();
        self.task_sender.send(DecodeTask::ExtractReflowData { start_page, response_tx: tx })
            .map_err(|e| anyhow::anyhow!("Failed to send reflow task: {}", e))?;
        rx.recv().map_err(|e| anyhow::anyhow!("{}", e))?
    }

    pub fn render_pages(&self, pages: Vec<RenderPage>) {
        if !pages.is_empty() {
            let _ = self.task_sender.send(DecodeTask::RenderPages { pages });
        }
    }

    pub fn has_error(&self) -> bool {
        self.error_occurred.load(Ordering::Acquire)
    }

    pub fn clear_error(&self) {
        self.error_occurred.store(false, Ordering::Release);
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
