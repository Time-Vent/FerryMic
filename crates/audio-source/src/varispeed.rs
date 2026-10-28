//! Varispeed 变速源（线性插值重采样）。
//! Varispeed source (linear-interpolation resampling).
//!
//! 语义：第一阶段的「倍速」= varispeed，即**变速同时变调**（像磁带调速）。
//! 保音高的 time-stretch 留待第二阶段。
//! Semantics: phase-1 "speed" = varispeed, i.e. pitch changes together with speed
//! (like a tape). Pitch-preserving time-stretch is deferred to phase 2.
//!
//! 实现：以累加相位 `phase` 遍历源 PCM，`phase += speed` 每帧；对每个采样做
//! 线性插值。这是最简单、实时安全、无分配的变速重采样。
//! Implementation: walk source PCM with an accumulating `phase`, `phase += speed`
//! per frame; linearly interpolate each sample. This is the simplest real-time-safe,
//! allocation-free rate change.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::wav_file::DecodedAudio;
use crate::VariableRateSource;

/// varispeed 重采样源：包装一段已解码 PCM，按可变速率（含非整数）线性插值输出。
/// Varispeed source: wraps decoded PCM and outputs at a variable (possibly fractional)
/// rate via linear interpolation.
///
/// PCM 以 `Arc<[f32]>` 共享：`clone` 只增加引用计数、不复制音频、不分配，
/// 因此可在实时线程里为同一音效创建多个播放实例。克隆体与原件共享欠载计数。
/// PCM is shared as `Arc<[f32]>`: `clone` only bumps a refcount — no audio copy, no
/// allocation — so the real-time thread can spawn many instances of one sound.
/// Clones share the underrun counter with the original.
#[derive(Clone)]
pub struct VarispeedSource {
    samples: Arc<[f32]>,
    channels: usize,
    sample_rate: u32,
    /// 当前读取相位（采样索引，非整数）。Current read phase (sample index, fractional).
    phase: f64,
    /// 默认速率。Default rate.
    speed: f32,
    /// 欠载计数（数据耗尽即 +1），供引擎读取上报。
    /// Underrun counter (incremented when data is exhausted).
    underrun_count: Arc<AtomicU64>,
}

impl VarispeedSource {
    /// 用已解码 PCM 构造。Construct from decoded PCM.
    pub fn new(audio: DecodedAudio) -> Self {
        Self {
            samples: audio.samples.into(),
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            phase: 0.0,
            speed: 1.0,
            underrun_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 从 WAV 文件一站式构造（解码 + 包装）。
    /// One-shot construct from a WAV file (decode + wrap).
    pub fn from_wav(path: &std::path::Path) -> Result<Self, crate::wav_file::SourceError> {
        let file = crate::wav_file::WavFileSource::open(path)?;
        Ok(Self::new(file.into_decoded()))
    }

    /// 从任意支持的音频文件构造（WAV/MP3/FLAC/OGG/AAC 等）。
    /// Construct from any supported audio file (WAV/MP3/FLAC/OGG/AAC etc.).
    pub fn from_file(path: &std::path::Path) -> Result<Self, crate::wav_file::SourceError> {
        let decoded = crate::audio_file::decode_audio_file(path)?;
        Ok(Self::new(decoded))
    }

    /// 源总帧数。Total source frames.
    pub fn total_frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }

    /// 当前相位（采样索引）。Current phase.
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// 共享的欠载计数（克隆后供引擎侧读）。Shared underrun counter.
    pub fn underrun_counter(&self) -> Arc<AtomicU64> {
        self.underrun_count.clone()
    }

    /// 读取欠载计数当前值。Read current underrun count.
    pub fn underruns(&self) -> u64 {
        self.underrun_count.load(Ordering::Relaxed)
    }

    /// 线性插值读取单个样本（第 `idx` 采样、第 `ch` 声道，idx 可为非整数）。
    /// Linearly interpolate a single sample (sample `idx`, channel `ch`, idx fractional).
    #[inline]
    fn interp(&self, idx: f64, ch: usize) -> f32 {
        let total = self.total_frames();
        if total == 0 {
            return 0.0;
        }
        let i0 = idx.floor() as usize;
        let frac = (idx - idx.floor()) as f32;
        // 边界外补静音（末尾）。
        // Out-of-range (past the end) reads silence.
        if i0 >= total {
            return 0.0;
        }
        let base = i0 * self.channels + ch;
        let s0 = self.samples[base];
        if i0 + 1 >= total || frac == 0.0 {
            return s0;
        }
        let s1 = self.samples[base + self.channels];
        s0 + (s1 - s0) * frac
    }
}

impl VariableRateSource for VarispeedSource {
    fn fill(&mut self, out: &mut [f32], frames: usize, speed: f32) -> usize {
        // 速率钳制，防御 0/负/NaN。Clamp speed; guard against 0/negative/NaN.
        let speed = if speed.is_nan() || speed <= 0.0 {
            self.speed
        } else {
            speed.clamp(0.0625, 16.0)
        };

        let total = self.total_frames();
        let ch = self.channels;
        if ch == 0 {
            // 无声道：补静音并计入欠载。No channels: pad silence and count underrun.
            for o in out.iter_mut() {
                *o = 0.0;
            }
            self.underrun_count.fetch_add(1, Ordering::Relaxed);
            return frames;
        }

        let mut written = 0usize;
        for f in 0..frames {
            // 相位越界 → 数据耗尽，补静音。
            // Phase past the end → exhausted, pad silence.
            if self.phase as usize >= total {
                break;
            }
            for c in 0..ch {
                out[f * ch + c] = self.interp(self.phase, c);
            }
            self.phase += speed as f64;
            written += 1;
        }

        // 若未能写满请求帧数，补静音并计数。
        // If not all requested frames written, pad silence and count.
        if written < frames {
            for o in out[written * ch..].iter_mut() {
                *o = 0.0;
            }
        }
        if written == 0 {
            self.underrun_count.fetch_add(1, Ordering::Relaxed);
        }
        written
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.underrun_count.store(0, Ordering::Relaxed);
    }

    fn set_speed(&mut self, speed: f32) {
        self.speed = if speed.is_nan() || speed <= 0.0 {
            1.0
        } else {
            speed.clamp(0.0625, 16.0)
        };
    }

    fn is_exhausted(&self) -> bool {
        self.phase as usize >= self.total_frames()
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav_file::DecodedAudio;
    use audio_types::SampleFormat;

    fn ramp(n: usize) -> DecodedAudio {
        DecodedAudio {
            samples: (0..n).map(|i| i as f32).collect(),
            sample_rate: 48_000,
            channels: 1,
            format: SampleFormat::F32,
        }
    }

    #[test]
    fn unity_speed_reproduces_samples() {
        let mut src = VarispeedSource::new(ramp(10));
        let mut out = vec![0.0; 5];
        let n = src.fill(&mut out, 5, 1.0);
        assert_eq!(n, 5);
        assert_eq!(&out[..5], &[0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn double_speed_skips_every_other() {
        let mut src = VarispeedSource::new(ramp(10));
        let mut out = vec![0.0; 4];
        let n = src.fill(&mut out, 4, 2.0);
        assert_eq!(n, 4);
        // phase 0,2,4,6 → 0,2,4,6
        assert_eq!(&out[..4], &[0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn clone_shares_pcm_but_plays_independently() {
        let mut a = VarispeedSource::new(ramp(10));
        let mut b = a.clone();
        let mut out = vec![0.0; 3];
        a.fill(&mut out, 3, 1.0);
        // b 的相位不受 a 影响，从头播放。b's phase is unaffected by a.
        b.fill(&mut out, 3, 1.0);
        assert_eq!(&out[..3], &[0.0, 1.0, 2.0]);
        assert!(Arc::ptr_eq(&a.samples, &b.samples));
    }

    #[test]
    fn half_speed_interpolates() {
        let mut src = VarispeedSource::new(ramp(10));
        let mut out = vec![0.0; 3];
        let n = src.fill(&mut out, 3, 0.5);
        assert_eq!(n, 3);
        // phase 0, 0.5, 1.0 → 0.0, 0.5, 1.0
        assert!((out[0] - 0.0).abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
        assert!((out[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn exhaustion_pads_silence_and_counts() {
        let mut src = VarispeedSource::new(ramp(3));
        let mut out = vec![0.0; 5];
        let n = src.fill(&mut out, 5, 1.0);
        assert_eq!(n, 3); // 只写出 3 帧
        assert_eq!(out[3], 0.0); // 后续补静音
        assert_eq!(out[4], 0.0);
        assert!(src.is_exhausted());
        // 再次 fill 触发 underrun 计数
        let mut out2 = vec![1.0; 2];
        src.fill(&mut out2, 2, 1.0);
        assert_eq!(src.underruns(), 1);
    }

    #[test]
    fn reset_restarts_from_beginning() {
        let mut src = VarispeedSource::new(ramp(5));
        let mut out = vec![0.0; 5];
        src.fill(&mut out, 5, 1.0);
        assert!(src.is_exhausted());
        src.reset();
        assert!(!src.is_exhausted());
        assert_eq!(src.phase(), 0.0);
    }
}
