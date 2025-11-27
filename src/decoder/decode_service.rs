use anyhow::Result;
use image::DynamicImage;
use log::{debug, error, info};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use std::{collections::VecDeque, sync::mpsc};

use crate::decoder::pdf::PdfDecoder;
use crate::decoder::{Decoder, PageInfo};

#[derive(Clone)]
pub struct DecodeTask {
    pub key: String,
    pub page_info: PageInfo,
    pub crop: i32,
    pub priority: Priority,
}

#[derive(Clone)]
pub enum Priority {
    Thumbnail = 0, // 最高优先级
    FullImage = 1, // 中优先级
    Cropped = 2,   // 低优先级
}

pub enum ControlMsg {
    LoadPdf { path: PathBuf },
    Shutdown,
}

pub struct DecodeService {
    control_tx: mpsc::Sender<ControlMsg>,
    request_tx: mpsc::Sender<DecodeTask>,
    result_rx: Option<mpsc::Receiver<Result<(String, DynamicImage)>>>,
    current_load_tx: Arc<Mutex<Option<mpsc::Sender<Vec<PageInfo>>>>>,
    _thread_handle: Option<thread::JoinHandle<()>>,
}

impl DecodeService {
    pub fn new() -> Self {
        let (control_tx, control_rx) = mpsc::channel();
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let current_load_tx = Arc::new(Mutex::new(None::<mpsc::Sender<Vec<PageInfo>>>));

        let current_load_tx_clone = Arc::clone(&current_load_tx);
        let thread = thread::spawn(move || {
            if let Err(err) = decode_thread_loop(control_rx, request_rx, result_tx, current_load_tx_clone) {
                error!("Decode thread error: {:?}", err);
            }
        });

        Self {
            control_tx,
            request_tx,
            result_rx: Some(result_rx),
            current_load_tx,
            _thread_handle: Some(thread),
        }
    }

    pub fn take_result_rx(&mut self) -> mpsc::Receiver<Result<(String, DynamicImage)>> {
        self.result_rx.take().unwrap()
    }

    pub fn load_pdf_with_tx(&self, path: &std::path::Path, tx: mpsc::Sender<Vec<PageInfo>>) -> Result<()> {
        *self.current_load_tx.lock().unwrap() = Some(tx);
        self.control_tx.send(ControlMsg::LoadPdf {
            path: path.to_path_buf(),
        });
        Ok(())
    }

    pub fn render_page(&self, decode_task: DecodeTask) {
        let _ = self.request_tx.send(decode_task);
    }

    pub fn shutdown(&mut self) {
        let _ = self.control_tx.send(ControlMsg::Shutdown);
        if let Some(handle) = self._thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DecodeService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn decode_thread_loop(
    control_rx: mpsc::Receiver<ControlMsg>,
    request_rx: mpsc::Receiver<DecodeTask>,
    result_tx: mpsc::Sender<Result<(String, DynamicImage)>>,
    current_load_tx: Arc<Mutex<Option<mpsc::Sender<Vec<PageInfo>>>>>,
) -> Result<()> {
    let mut decoder: Option<Rc<dyn Decoder>> = None;

    let mut prioritized_queue = VecDeque::new();

    loop {
        // Try to receive control message
        match control_rx.try_recv() {
            Ok(ControlMsg::LoadPdf { path }) => {
                info!("Loading PDF in decode thread: {:?}", path);
                let pdf_decoder = Rc::new(PdfDecoder::open(&path)?);
                let pages_info = pdf_decoder.get_all_pages()?;
                if let Some(tx) = current_load_tx.lock().unwrap().take() {
                    tx.send(pages_info)?;
                }
                decoder = Some(pdf_decoder);
                info!("PDF loaded successfully in decode thread");
            }
            Ok(ControlMsg::Shutdown) => {
                info!("Decode thread shutting down");
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => break,
        }

        // Try to receive decode task
        match request_rx.try_recv() {
            Ok(task) => {
                prioritized_queue.push_back(task);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => break,
        }

        // Process tasks if decoder is loaded
        if let Some(decoder_rc) = decoder.clone() {
            if let Some(task) = prioritized_queue.pop_front() {
                let start_time = Instant::now();
                let result = decoder_rc.render_page(&task.page_info, task.crop != 0);
                let duration = start_time.elapsed();
                match result {
                    Ok(image) => {
                        info!(
                            "[DecodeService] 页面 {} 异步渲染完成，耗时: {:?}",
                            task.page_info.index, duration
                        );
                        let _ = result_tx.send(Ok((task.key, image)));
                    }
                    Err(e) => {
                        debug!(
                            "[DecodeService] 页面 {} 渲染失败: {}",
                            task.page_info.index, e
                        );
                        let _ = result_tx.send(Err(e));
                    }
                }
            }
        }

        // Small sleep to prevent busy loop
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    Ok(())
}
