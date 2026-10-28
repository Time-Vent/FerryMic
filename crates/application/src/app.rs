//! App 装配：启动/停止完整混音链路。
//! App assembly: start/stop the complete mix chain.
//!
//! 线程拓扑（第一阶段）：
//! Thread topology (phase 1):
//! - 实时线程：cpal 输出回调 → 读文件环 + 麦克风环 → 混音 → 写输出缓冲。
//!   Real-time thread: cpal output callback → read file ring + mic ring → mix →
//!   write output buffer.
//! - 解码线程：文件源 fill → 声道转换 → 写文件环（SPSC）。
//!   Decode thread: file source fill → channel convert → write file ring (SPSC).
//! - 麦克风回调线程：cpal 输入回调 → 写麦克风环（SPSC）。
//!   Mic callback thread: cpal input callback → write mic ring (SPSC).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use audio_cpal::CpalBackend;
use audio_source::{SourceError, VariableRateSource, VarispeedSource};
use audio_types::StreamConfig;
use platform_api::{AudioBackend, InputStreamHandle, OutputStreamHandle};
use ringbuf::traits::{Consumer as _, Producer as _, Split as _};
use ringbuf::HeapRb;
use thiserror::Error;

use crate::mixer::Mixer;

/// App 配置。App configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// 引擎采样率。Engine sample rate.
    pub sample_rate: u32,
    /// 引擎声道数（输出与内部统一）。Engine channel count (output + internal).
    pub channels: u16,
    /// 每回调帧数。Frames per callback.
    pub buffer_frames: usize,
    /// 文件环容量（样点数）。File ring capacity (samples).
    pub file_ring_samples: usize,
    /// 麦克风环容量（样点数）。Mic ring capacity (samples).
    pub mic_ring_samples: usize,
    /// 是否循环播放文件。Whether to loop the file.
    pub loop_playback: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            buffer_frames: 512,
            file_ring_samples: 48_000 * 2 * 2, // 2 秒 × 2 声道。2s × 2ch.
            mic_ring_samples: 48_000 * 2,      // 1 秒 × 2 声道。1s × 2ch.
            loop_playback: true,
        }
    }
}

/// App 错误。App error.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("platform error: {0}")]
    Platform(#[from] platform_api::PlatformError),
    #[error("source error: {0}")]
    Source(#[from] SourceError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("already running")]
    AlreadyRunning,
    #[error("no file source loaded")]
    NoFileSource,
}

/// 运行句柄：持有停止标志、后台线程与流句柄；drop 时自动停止并释放。
/// Run handle: holds the stop flag, background thread, and stream handles; stops and
/// releases on drop.
pub struct RunHandle {
    stop: Arc<AtomicBool>,
    decode_thread: Option<JoinHandle<()>>,
    _output: Option<Box<dyn OutputStreamHandle>>,
    _input: Option<Box<dyn InputStreamHandle>>,
}

impl RunHandle {
    /// 停止（幂等）。Stop (idempotent).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.decode_thread.take() {
            let _ = t.join();
        }
        // drop 流句柄以停止音频流。
        // Drop stream handles to stop the audio streams.
        self._output.take();
        self._input.take();
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 应用主体：持有后端、混音器、文件源，负责启动/停止。
/// Application body: holds backend, mixer, file source; starts/stops the chain.
pub struct App {
    backend: CpalBackend,
    config: AppConfig,
    mixer: Arc<Mixer>,
    file_source: Option<VarispeedSource>,
    running: bool,
}

impl App {
    /// 创建应用。Create the app.
    pub fn new(config: AppConfig) -> Self {
        Self {
            backend: CpalBackend::new(),
            config,
            mixer: Arc::new(Mixer::new()),
            file_source: None,
            running: false,
        }
    }

    /// 混音器共享引用（供控制侧调增益/速率）。
    /// Shared mixer reference (for the control side to adjust gain/rate).
    pub fn mixer(&self) -> Arc<Mixer> {
        self.mixer.clone()
    }

    /// 加载文件源（WAV）。Load a file source (WAV).
    pub fn load_file(&mut self, path: &std::path::Path) -> Result<(), AppError> {
        self.file_source = Some(VarispeedSource::from_wav(path)?);
        Ok(())
    }

    /// 枚举设备。Enumerate devices.
    pub fn devices(&self) -> Result<Vec<audio_types::DeviceInfo>, AppError> {
        Ok(self.backend.enumerate_devices()?)
    }

    /// 设置文件速率。Set file speed.
    pub fn set_file_speed(&self, speed: f32) {
        self.mixer.set_file_speed(speed);
    }

    /// 设置文件增益（dB）。Set file gain (dB).
    pub fn set_file_gain_db(&self, db: f32) {
        self.mixer.set_file_gain_db(db);
    }

    /// 设置麦克风增益（dB）。Set mic gain (dB).
    pub fn set_mic_gain_db(&self, db: f32) {
        self.mixer.set_mic_gain_db(db);
    }

    /// 启动完整链路（输出 + 输入 + 解码线程）。
    /// Start the full chain (output + input + decode thread).
    pub fn start(&mut self) -> Result<RunHandle, AppError> {
        if self.running {
            return Err(AppError::AlreadyRunning);
        }

        let cfg = self.config.clone();
        let channels = cfg.channels as usize;
        let frames = cfg.buffer_frames;
        let out_samples = frames * channels;

        let out_config = StreamConfig::with_buffer(cfg.sample_rate, cfg.channels, frames);
        let in_config = StreamConfig::with_buffer(cfg.sample_rate, cfg.channels, frames);

        // 文件环：解码线程写，实时线程读。
        // File ring: decode thread writes, real-time thread reads.
        let file_ring = HeapRb::<f32>::new(cfg.file_ring_samples);
        let (mut file_prod, mut file_cons) = file_ring.split();

        // 麦克风环：麦克风回调写，实时线程读。
        // Mic ring: mic callback writes, real-time thread reads.
        let mic_ring = HeapRb::<f32>::new(cfg.mic_ring_samples);
        let (mut mic_prod, mut mic_cons) = mic_ring.split();

        let mixer = self.mixer.clone();
        let stop = Arc::new(AtomicBool::new(false));

        // 输出流：实时回调（捕获文件/麦克风 consumer + mixer）。
        // Output stream: real-time callback (captures file/mic consumers + mixer).
        let out_mixer = mixer.clone();
        let mut out_buf = vec![0.0f32; out_samples];
        let mut file_buf = vec![0.0f32; out_samples];
        let mut mic_buf = vec![0.0f32; out_samples];

        let output = self.backend.open_output(&out_config, move |data: &mut [f32]| {
            // 1. 从环取 PCM（文件、麦克风），不足补静音。
            // 1. Pull PCM from rings (file, mic); pad silence when short.
            let n_file = file_cons.pop_slice(&mut file_buf);
            for s in file_buf[n_file..].iter_mut() {
                *s = 0.0;
            }
            let n_mic = mic_cons.pop_slice(&mut mic_buf);
            for s in mic_buf[n_mic..].iter_mut() {
                *s = 0.0;
            }

            // 2. 混音（含软限幅）。2. Mix (with soft limiting).
            out_mixer.mix(&file_buf, &mic_buf, &mut out_buf);

            // 3. 写出。3. Write out.
            let n = data.len().min(out_buf.len());
            data[..n].copy_from_slice(&out_buf[..n]);
        })?;

        // 输入流：麦克风回调写环。
        // Input stream: mic callback writes ring.
        let input = self.backend.open_input(&in_config, move |data: &[f32]| {
            let _ = mic_prod.push_slice(data);
        })?;

        // 解码线程：文件源 fill → 声道转换 → 写文件环。
        // Decode thread: file source fill → channel convert → write file ring.
        let mut file_source = self.file_source.take().ok_or(AppError::NoFileSource)?;

        let decode_mixer = mixer.clone();
        let decode_stop = stop.clone();
        let loop_playback = cfg.loop_playback;

        let decode_thread = std::thread::spawn(move || {
            let mut src_buf = vec![0.0f32; frames * channels];
            let mut conv_buf = vec![0.0f32; frames * channels];
            let src_channels = file_source.channels().max(1);

            while !decode_stop.load(Ordering::Acquire) {
                let speed = decode_mixer.file_speed();
                let n = file_source.fill(&mut src_buf, frames, speed);

                // 声道转换：源声道 → 引擎声道。
                // Channel convert: source → engine channels.
                convert_channels(&src_buf[..n * src_channels], src_channels, &mut conv_buf, channels);

                let _ = file_prod.push_slice(&conv_buf[..n * channels]);

                // 文件耗尽：循环则 reset，否则休眠让出 CPU。
                // Exhausted: loop → reset; else sleep to yield CPU.
                if file_source.is_exhausted() {
                    if loop_playback {
                        file_source.reset();
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
        });

        self.running = true;

        Ok(RunHandle {
            stop,
            decode_thread: Some(decode_thread),
            _output: Some(output),
            _input: Some(input),
        })
    }
}

/// 声道转换：把 `src_channels` 的交错样本转成 `dst_channels` 交错样本。
/// Channel conversion: interleaved `src_channels` → interleaved `dst_channels`.
///
/// 规则：mono→多声道复制；多声道→mono 取平均；同声道直接拷贝；其它取首 N 声道。
/// Rules: mono→multi duplicates; multi→mono averages; same count copies; otherwise
/// take the first N channels.
fn convert_channels(src: &[f32], src_ch: usize, dst: &mut [f32], dst_ch: usize) {
    let frames = if src_ch == 0 { 0 } else { src.len() / src_ch };

    if src_ch == dst_ch {
        dst[..src.len()].copy_from_slice(src);
        return;
    }

    for f in 0..frames {
        if dst_ch == 1 {
            // 多声道 → 单声道：平均。Multi → mono: average.
            let mut sum = 0.0;
            for c in 0..src_ch {
                sum += src[f * src_ch + c];
            }
            dst[f] = sum / src_ch as f32;
        } else if src_ch == 1 {
            // 单声道 → 多声道：复制。Mono → multi: duplicate.
            let v = src[f];
            for c in 0..dst_ch {
                dst[f * dst_ch + c] = v;
            }
        } else {
            // 其它：取交集（min 声道），多余声道补 0。
            // Otherwise: take the intersection (min channels), pad extra with 0.
            let take = src_ch.min(dst_ch);
            for c in 0..take {
                dst[f * dst_ch + c] = src[f * src_ch + c];
            }
            for c in take..dst_ch {
                dst[f * dst_ch + c] = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单声道 → 立体声应复制到左右声道。
    /// Mono → stereo should duplicate to both channels.
    #[test]
    fn convert_mono_to_stereo_duplicates() {
        let src = [0.5, -0.5];
        let mut dst = [0.0; 4];
        convert_channels(&src, 1, &mut dst, 2);
        assert_eq!(dst, [0.5, 0.5, -0.5, -0.5]);
    }

    /// 立体声 → 单声道应取平均。
    /// Stereo → mono should average.
    #[test]
    fn convert_stereo_to_mono_averages() {
        let src = [1.0, 0.0, 0.0, 0.0]; // 2 帧，帧0=(1,0) 帧1=(0,0)。2 frames.
        let mut dst = [0.0; 2];
        convert_channels(&src, 2, &mut dst, 1);
        assert_eq!(dst, [0.5, 0.0]);
    }
}
