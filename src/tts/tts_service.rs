use anyhow::Result;
use log::{debug, info};
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::collections::VecDeque;
use std::process;
use regex::Regex;

pub enum TtsTask {
    SpeakText {
        text: String,
    },
    Stop,
    SetVoice {
        voice: String,
    },
    Shutdown,
}

struct TtsState {
    task_rx: Receiver<TtsTask>,
    speech_queue: VecDeque<String>,
    current_voice: String,
    rate: f32,
    volume: f32,
    is_speaking: bool,
}

pub struct TtsService {
    task_sender: Sender<TtsTask>,
    thread_handle: Option<JoinHandle<()>>,
}

impl TtsService {
    pub fn new() -> Self {
        let (task_tx, task_rx) = unbounded::<TtsTask>();

        let thread_handle = thread::spawn(move || {
            Self::tts_loop(task_rx);
        });

        Self {
            task_sender: task_tx,
            thread_handle: Some(thread_handle),
        }
    }

    fn tts_loop(task_rx: Receiver<TtsTask>) {
        let mut state = TtsState {
            task_rx,
            speech_queue: VecDeque::new(),
            current_voice: "Mei-Jia".to_string(),
            rate: 0.6,
            volume: 0.8,
            is_speaking: false,
        };

        loop {
            while let Ok(task) = state.task_rx.try_recv() {
                if Self::handle_task(task, &mut state) {
                    return;
                }
            }

            if let Some(text) = state.speech_queue.pop_front() {
                if let Err(e) = Self::execute_speech(&text, &state.current_voice, state.rate) {
                    info!("[TtsService] TTS 朗读失败: {}", e);
                }
                continue;
            }

            match state.task_rx.recv() {
                Ok(task) => {
                    if Self::handle_task(task, &mut state) {
                        break;
                    }
                }
                Err(_) => {
                    info!("[TtsService] Task channel closed");
                    break;
                }
            }
        }
    }

    fn handle_task(task: TtsTask, state: &mut TtsState) -> bool {
        match task {
            TtsTask::SpeakText { text } => {
                debug!("[TtsService] 收到朗读任务: {}", text);
                state.speech_queue.push_back(text);
                false
            }
            TtsTask::Stop => {
                info!("[TtsService] 停止朗读，清空队列");
                state.speech_queue.clear();
                false
            }
            TtsTask::SetVoice { voice } => {
                info!("[TtsService] 设置语音: {}", voice);
                state.current_voice = voice;
                false
            }
            TtsTask::Shutdown => {
                info!("[TtsService] Shutting down TTS thread");
                true
            }
        }
    }

    fn execute_speech(text: &str, voice: &str, rate: f32) -> Result<()> {
        let text_variants = vec![
            Self::clean_text_for_tts(text),
            text.replace("--", "").replace("-", ""),  
            Self::extract_meaningful_text(text),
            "跳过无法朗读的内容".to_string(),  
        ];

        let rate_value = (rate * 400.0).clamp(100.0, 500.0) as i32;

        for (i, variant) in text_variants.iter().enumerate() {
            if variant.is_empty() {
                continue;
            }

            info!("[TtsService] Trying text variant {}: {}", i, variant);

            let status = if cfg!(target_os = "macos") {
                process::Command::new("say")
                    .args(["-v", voice, "-r", &rate_value.to_string(), variant])
                    .status()
            } else if cfg!(target_os = "windows") {
                let escaped_text = variant
                    .replace("\\", "\\\\")
                    .replace("'", "''")
                    .replace("\"", "`\"")
                    .replace("$", "`$");

                process::Command::new("powershell")
                    .args([
                        "-Command",
                        &format!("Add-Type -AssemblyName System.Speech; $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; $synth.SelectVoice('{}'); $synth.Rate = {}; $synth.Volume = {}; $synth.Speak('{}'); $synth.Dispose()", voice, 0, 80, escaped_text)
                    ])
                    .status()
            } else {
                return Err(anyhow::anyhow!("Unsupported platform"));
            };

            match status {
                Ok(s) if s.success() => {
                    info!("[TtsService] Successfully spoke with variant {}", i);
                    return Ok(());
                }
                Ok(s) => {
                    info!("[TtsService] Variant {} failed with code: {}", i, s.code().unwrap_or(-1));
                    continue;
                }
                Err(e) => {
                    info!("[TtsService] Variant {} failed to start: {}", i, e);
                    continue;
                }
            }
        }

        Err(anyhow::anyhow!("All TTS variants failed"))
    }

    fn clean_text_for_tts(text: &str) -> String {
        let re_long_dashes = Regex::new(r"-{3,}").unwrap();
        let re_long_equals = Regex::new(r"={3,}").unwrap();
        let re_long_asterisks = Regex::new(r"\*{3,}").unwrap();
        let re_long_hashes = Regex::new(r"#{3,}").unwrap();
        let re_long_underscores = Regex::new(r"_{3,}").unwrap();
        let re_full_brackets = Regex::new(r"（[^）]*）").unwrap();
        let re_half_brackets = Regex::new(r"\([^)]*\)").unwrap();
        let re_multiple_spaces = Regex::new(r"\s{2,}").unwrap();

        let cleaned = re_long_dashes.replace_all(text, "");
        let cleaned = re_long_equals.replace_all(&cleaned, "");
        let cleaned = re_long_asterisks.replace_all(&cleaned, "");
        let cleaned = re_long_hashes.replace_all(&cleaned, "");
        let cleaned = re_long_underscores.replace_all(&cleaned, "");
        let cleaned = cleaned.replace("---", "")  // Remove long dashes
            .replace("--", "")   // Remove double dashes
            .replace("—", "")    // Remove em dash
            .replace("–", "")    // Remove en dash
            .replace("…", "")    // Remove ellipsis
            .replace("　", " ")   // Full width space to half
            .replace("，", ",")   // Full comma to half
            .replace("。", ".")   // Full period to half
            .replace("；", ";")   // Full semicolon to half
            .replace("：", ":")   // Full colon to half
            .replace("？", "?")   // Full question to half
            .replace("！", "!");   // Full exclamation to half
        let cleaned = re_full_brackets.replace_all(&cleaned, "");
        let cleaned = re_half_brackets.replace_all(&cleaned, "");
        let cleaned = re_multiple_spaces.replace_all(&cleaned, " ");
        cleaned.trim().to_string()
    }

    fn extract_meaningful_text(text: &str) -> String {
        text.lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && line.len() > 2)
            .filter(|line| !Regex::new(r"^-+$|^=+$|^\*+$|^#+|^_+$").unwrap().is_match(line))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn speak_text(&self, text: String) {
        let _ = self.task_sender.send(TtsTask::SpeakText { text });
    }

    pub fn stop_speaking(&self) {
        let _ = self.task_sender.send(TtsTask::Stop);
    }

    pub fn set_voice(&self, voice: String) {
        let _ = self.task_sender.send(TtsTask::SetVoice { voice });
    }

    pub fn destroy(&mut self) {
        info!("[TtsService] Destroying TTS service");
        let _ = self.task_sender.send(TtsTask::Shutdown);
    }
}

impl Drop for TtsService {
    fn drop(&mut self) {
        self.destroy();
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Default for TtsService {
    fn default() -> Self {
        Self::new()
    }
}
