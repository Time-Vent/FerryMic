//! 音频线程：SoundBoard + cpal 输出，消费命令队列。
//! Audio thread: SoundBoard + cpal output, consuming the command queue.
//!
//! 线程拓扑：
//! Thread topology:
//! - cpal 输出回调线程（实时）：消费命令(≤32) → SoundBoard::fill → 写输出。
//!   cpal output callback thread (real-time): consume ≤32 commands → SoundBoard::fill
//!   → write output.
//! - 本模块创建的流句柄由调用方持有（drop 即停止）。
//!   The stream handle created here is held by the caller (dropped to stop).

use std::sync::Arc;

use audio_cpal::CpalBackend;
use audio_types::StreamConfig;
use crossbeam_channel::Receiver;
use platform_api::{AudioBackend, OutputStreamHandle};
use soundboard::{BoardCommand, SoundBoard};

/// 启动音频线程：创建 SoundBoard + cpal 输出流。
/// Start the audio thread: create SoundBoard + cpal output stream.
///
/// `output_device`：可选输出设备名（模糊匹配）；`None` 用默认输出设备。
/// `output_device`: optional output device name (fuzzy match); `None` uses default.
///
/// 返回输出流句柄（持有即播放，drop 即停止）。
/// Returns the output stream handle (playing while held, stops on drop).
pub fn spawn_audio_thread(
    cmd_rx: Receiver<BoardCommand>,
    output_device: Option<&str>,
) -> anyhow::Result<Box<dyn OutputStreamHandle>> {
    let backend = CpalBackend::new();

    let sample_rate = 48_000u32;
    let channels = 2u16;
    let frames = 512usize;
    let config = StreamConfig::with_buffer(sample_rate, channels, frames);

    // SoundBoard 的 scratch 容量：足够容纳单次 fill 的最大样点数。
    // 用 frames × 8 留余量（文件最多按 8 声道假设，实际 mono/stereo 常见）。
    // SoundBoard scratch capacity: enough for the max samples per fill.
    // frames × 8 leaves headroom (files up to 8ch assumed; mono/stereo common).
    let mut board = SoundBoard::new(frames * 8);

    let channels_usize = channels as usize;

    // 回调：消费命令 + 混音 + 写出。
    // Callback: consume commands + mix + write out.
    let callback = move |data: &mut [f32]| {
        // 1. 有界消费命令（≤32 条）。1. Bounded command consumption (≤32).
        let mut n = 0;
        while n < 32 {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    board.apply_command(&cmd);
                    n += 1;
                }
                Err(_) => break, // 队列空。Queue empty.
            }
        }

        // 2. 混音所有实例到输出。2. Mix all instances into output.
        let frames_actual = data.len() / channels_usize;
        board.fill(data, frames_actual, channels_usize);
    };

    let handle = match output_device {
        Some(name) => backend.open_output_named(&config, name, callback)?,
        None => backend.open_output(&config, callback)?,
    };

    Ok(handle)
}

/// 音频线程句柄：持有流句柄，drop 即停止音频。
/// Audio thread handle: holds the stream handle; drop stops audio.
pub struct AudioHandle {
    _stream: Arc<Box<dyn OutputStreamHandle>>,
}

impl AudioHandle {
    pub fn new(stream: Box<dyn OutputStreamHandle>) -> Self {
        Self {
            _stream: Arc::new(stream),
        }
    }
}
