//! audio-source：音频源抽象。
//! audio-source: audio source abstraction.
//!
//! 提供两类源：
//! Two kinds of source:
//! 1. [`VariableRateSource`] —— 变速源（文件解码 + varispeed 重采样）。
//!    1. `VariableRateSource` — variable-rate source (file decode + varispeed resample).
//! 2. [`InputSource`] —— 输入采集源（麦克风等），经 SPSC 环与异步重采样汇入混音。
//!    2. `InputSource` — input capture source (mic etc.), fed via SPSC ring + async
//!       resample into the mix.
//!
//! 变速属于「源」的职责，不属于 [`dsp::FixedProcessor`]；重采样改变帧数。
//! Speed/pitch change belongs to the source, not `dsp::FixedProcessor`; resampling
//! changes frame counts.

#![forbid(unsafe_code)]

mod varispeed;
pub mod wav_file;

pub use varispeed::VarispeedSource;
pub use wav_file::{DecodedAudio, SourceError, WavFileSource};

use audio_types::EngineEvent;

/// 变速源接口：请求填充 `frames` 帧，内部按 `speed` 消耗输入。
/// Variable-rate source: request `frames` frames; internally consumes input at `speed`.
///
/// 返回实际写入的帧数；欠载时补静音、通过 `underrun_count` 上报，不阻塞。
/// Returns actual frames written; on underrun it pads silence, counts it via
/// `underrun_count`, and never blocks.
pub trait VariableRateSource: Send {
    /// 填充 `frames` 帧到 `out`（交错：frames × channels）。返回实际写入帧数。
    /// Fill `frames` frames into `out` (interleaved). Returns frames actually written.
    ///
    /// `speed`：播放速率（>0；1.0 = 原速）。第一阶段为 varispeed（变速变调）。
    /// `speed`: playback rate (>0; 1.0 = original). Phase 1 is varispeed (speed and
    /// pitch change together).
    fn fill(&mut self, out: &mut [f32], frames: usize, speed: f32) -> usize;

    /// 重置到起始位置（seek 到 0）。
    /// Reset to the start (seek to 0).
    fn reset(&mut self);

    /// 设置默认速率（后续 `fill` 可传入的速率覆盖此值）。
    /// Set a default rate (the `speed` passed to `fill` overrides it).
    fn set_speed(&mut self, speed: f32);

    /// 是否已播放到末尾（数据耗尽）。供引擎决定是否补静音/停止。
    /// Whether the source has reached the end (data exhausted).
    fn is_exhausted(&self) -> bool;

    /// 声道数。Channel count.
    fn channels(&self) -> usize;

    /// 采样率（引擎侧据此做对齐重采样）。Sample rate.
    fn sample_rate(&self) -> u32;
}

/// 输入采集源：由平台输入回调持续灌入 PCM，供混音点读取。
/// Input capture source: continuously fed by a platform input callback, read by the
/// mix point.
///
/// 实时路径通过 SPSC 环写入，消费侧做水位漂移补偿 + 异步重采样。
/// The real-time path writes through an SPSC ring; the consumer applies drift
/// compensation + async resampling.
pub trait InputSource: Send {
    /// 从源读取 `frames` 帧到 `out`。若数据不足，补静音并返回实际帧数。
    /// Read up to `frames` frames into `out`. Pads silence when short and returns
    /// actual frames.
    fn read(&mut self, out: &mut [f32], frames: usize) -> usize;

    /// 启动/暂停采集。Start/pause capture.
    fn start(&mut self);
    fn stop(&mut self);

    /// 声道数。Channel count.
    fn channels(&self) -> usize;

    /// 采样率。Sample rate.
    fn sample_rate(&self) -> u32;

    /// 非实时侧消费事件（欠载等）。Drain events on the non-realtime side.
    fn drain_events(&mut self) -> Vec<EngineEvent>;
}
