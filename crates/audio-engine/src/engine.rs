//! 引擎核心装配：把源、混音、DSP 图、命令队列串成一个可运行实体。
//! Engine core assembly: wires sources, mix, DSP graph, and command queue into a
//! runnable entity.
//!
//! 这是第一阶段「跑通路」的核心：文件(变速) + 麦克风 → 独立增益 → 混音 → DSP
//! → 输出。实时回调只做有界工作（消费 ≤32 命令、取 PCM、混音、写出）。
//! This is the phase-1 "walking path" core: file (varispeed) + mic → independent
//! gain → mix → DSP → output. The real-time callback does only bounded work
//! (consume ≤32 commands, pull PCM, mix, write out).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use audio_types::{EngineEvent as CoreEngineEvent, Gain, PlaySpeed, SampleRate};

use crate::clock::DriftConfig;
use crate::executors::EngineCommand;

/// 引擎配置。Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 引擎内部主时钟采样率。Master clock sample rate inside the engine.
    pub sample_rate: SampleRate,
    /// 输出声道数。Output channel count.
    pub channels: usize,
    /// 每回调帧数（输出缓冲）。Frames per callback (output buffer).
    pub buffer_frames: usize,
    /// 文件 SPSC 环容量（帧）。File SPSC ring capacity (frames).
    pub file_ring_frames: usize,
    /// 麦克风 SPSC 环容量（帧）。Mic SPSC ring capacity (frames).
    pub mic_ring_frames: usize,
    /// 漂移控制配置。Drift control configuration.
    pub drift: DriftConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate(48_000),
            channels: 2,
            buffer_frames: 512,
            file_ring_frames: 48_000 * 2, // 2 秒缓冲。2s buffer.
            mic_ring_frames: 48_000,      // 1 秒缓冲。1s buffer.
            drift: DriftConfig::default(),
        }
    }
}

/// 源 ID 常量（第一阶段固定两个源）。Source ID constants (two fixed sources).
pub mod source_id {
    /// 文件源。File source.
    pub const FILE: u32 = 1;
    /// 麦克风源。Mic source.
    pub const MIC: u32 = 2;
}

/// 混音参数：每个源的独立增益与文件源速率。
/// Mix parameters: per-source gain and file source rate.
#[derive(Debug, Clone)]
pub struct MixParams {
    pub file_gain: Gain,
    pub mic_gain: Gain,
    pub file_speed: PlaySpeed,
}

impl Default for MixParams {
    fn default() -> Self {
        Self {
            file_gain: Gain::UNITY,
            mic_gain: Gain::UNITY,
            file_speed: PlaySpeed::NORMAL,
        }
    }
}

/// 引擎运行态：持有实时回调所需的全部可变状态。
/// Engine runtime state: holds all mutable state the real-time callback needs.
///
/// 此结构由**单个实时线程独占**访问（audio_callback），控制侧通过无锁命令
/// 队列与原子量下发参数。
/// This struct is exclusively accessed by a **single real-time thread**
/// (audio_callback); the control side pushes params via lock-free command queue
/// and atomics.
pub struct Engine {
    config: EngineConfig,
    /// 混音参数（实时线程独享的可变副本，命令侧通过原子量/队列更新）。
    /// Mix params (mutable copy owned by the real-time thread).
    mix: MixParams,
    /// 各源增益的线性值（用于混音，避免每帧算 dB）。
    /// Linear gains per source (used in the mix, avoids per-frame dB math).
    file_gain_lin: f32,
    mic_gain_lin: f32,
    /// 文件源当前速率（实时侧使用的副本）。
    /// Current file rate (copy used by the real-time side).
    file_speed: f32,
    /// 是否运行中。Whether running.
    running: Arc<AtomicBool>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            mix: MixParams::default(),
            file_gain_lin: 1.0,
            mic_gain_lin: 1.0,
            file_speed: 1.0,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 引擎配置访问器（供装配层读取缓冲帧数、声道等）。
    /// Engine config accessor (for the assembly layer to read buffer size, channels).
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// 应用一条命令（实时侧调用，必须在回调线程内）。
    /// Apply one command (real-time side, must be inside the callback thread).
    pub fn apply_command(&mut self, cmd: &EngineCommand) {
        match cmd {
            EngineCommand::SetSourceGain { source_id, gain_db } => {
                let lin = db_to_linear(*gain_db);
                if *source_id == source_id::FILE {
                    self.mix.file_gain = Gain(*gain_db);
                    self.file_gain_lin = lin;
                } else if *source_id == source_id::MIC {
                    self.mix.mic_gain = Gain(*gain_db);
                    self.mic_gain_lin = lin;
                }
            }
            EngineCommand::SetFileSpeed { speed, .. } => {
                self.mix.file_speed = PlaySpeed(*speed);
                self.file_speed = PlaySpeed(*speed).clamped().0;
            }
            _ => {
                // Start/Stop/Switch/Shutdown 由装配层处理（涉及流生命周期）。
                // Start/Stop/Switch/Shutdown are handled by the assembly layer
                // (they involve stream lifecycle).
            }
        }
    }

    /// 混音：将文件、麦克风两路 PCM 按各自增益相加，并做软限幅防削波。
    /// Mix: sum file and mic PCM by their gains, with soft limiting to avoid clipping.
    ///
    /// 输入输出均为交错。`file`/`mic` 长度 = frames × channels。
    /// Both inputs and output are interleaved; `file`/`mic` length = frames × channels.
    pub fn mix(&self, file: &[f32], mic: &[f32], out: &mut [f32], frames: usize, channels: usize) {
        let _ = (frames, channels);
        for (i, o) in out.iter_mut().enumerate() {
            let mut s = file.get(i).copied().unwrap_or(0.0) * self.file_gain_lin
                + mic.get(i).copied().unwrap_or(0.0) * self.mic_gain_lin;
            // 软限幅：tanh 防削波，比硬 clip 更平滑。
            // Soft limiting: tanh avoids hard clipping and sounds smoother.
            s = s.tanh();
            *o = s;
        }
    }

    /// 是否运行中。Whether the engine is running.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// 设置运行标志（装配层在启动/停止流时调用）。
    /// Set the running flag (called by the assembly layer on stream start/stop).
    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Release);
    }
}

/// dB → 线性幅度（引擎内部用，与 dsp::gain 保持一致）。
/// dB → linear amplitude (kept consistent with `dsp::gain`).
#[inline]
fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 {
        return 0.0; // -120dB 以下视为静音。Below -120dB treat as mute.
    }
    10.0_f32.powf(db / 20.0)
}

/// 便捷：从引擎事件重建核心事件类型（消除命名歧义）。
/// Re-export the core event type for convenience (avoids name ambiguity).
pub type EngineEvent = CoreEngineEvent;
