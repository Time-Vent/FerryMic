//! 混音器：文件 + 麦克风两路，独立增益，软限幅后输出。
//! Mixer: file + mic tracks with independent gain, soft-limited before output.
//!
//! 实时回调线程独占访问；控制侧通过原子量下发增益/速率。
//! Owned by the real-time callback thread; control side pushes gain/rate via atomics.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// 混音器输入源枚举。Mixer input source enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerInput {
    File,
    Mic,
}

/// 实时安全的混音器。
/// Real-time-safe mixer.
///
/// 增益以 dB 通过 [`AtomicU32`]（f32 位模式）跨线程共享，实时侧读取并换算。
/// Gains are shared across threads as dB via `AtomicU32` (f32 bits); the real-time
/// side reads and converts.
pub struct Mixer {
    file_gain_db: Arc<AtomicU32>,
    mic_gain_db: Arc<AtomicU32>,
    file_speed: Arc<AtomicU32>,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            file_gain_db: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            mic_gain_db: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            file_speed: Arc::new(AtomicU32::new(1.0f32.to_bits())),
        }
    }

    /// 设置文件源增益（dB）。Set file source gain (dB).
    pub fn set_file_gain_db(&self, db: f32) {
        self.file_gain_db.store(db.to_bits(), Ordering::Relaxed);
    }

    /// 设置麦克风增益（dB）。Set mic gain (dB).
    pub fn set_mic_gain_db(&self, db: f32) {
        self.mic_gain_db.store(db.to_bits(), Ordering::Relaxed);
    }

    /// 设置文件源速率。Set file source rate.
    pub fn set_file_speed(&self, speed: f32) {
        let s = if speed.is_nan() || speed <= 0.0 {
            1.0
        } else {
            speed.clamp(0.0625, 16.0)
        };
        self.file_speed.store(s.to_bits(), Ordering::Relaxed);
    }

    /// 实时侧读取文件源线性增益。Real-time side reads file linear gain.
    #[inline]
    fn file_gain_lin(&self) -> f32 {
        let db = f32::from_bits(self.file_gain_db.load(Ordering::Relaxed));
        db_to_linear(db)
    }

    /// 实时侧读取麦克风线性增益。Real-time side reads mic linear gain.
    #[inline]
    fn mic_gain_lin(&self) -> f32 {
        let db = f32::from_bits(self.mic_gain_db.load(Ordering::Relaxed));
        db_to_linear(db)
    }

    /// 实时侧读取文件速率。Real-time side reads file rate.
    #[inline]
    pub fn file_speed(&self) -> f32 {
        f32::from_bits(self.file_speed.load(Ordering::Relaxed))
    }

    /// 混音：`file` + `mic` 相加，软限幅。长度 = frames × channels。
    /// Mix `file` + `mic`, soft-limited. Length = frames × channels.
    pub fn mix(&self, file: &[f32], mic: &[f32], out: &mut [f32]) {
        let fg = self.file_gain_lin();
        let mg = self.mic_gain_lin();
        for (i, o) in out.iter_mut().enumerate() {
            let s =
                file.get(i).copied().unwrap_or(0.0) * fg + mic.get(i).copied().unwrap_or(0.0) * mg;
            *o = s.tanh();
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

/// dB → 线性幅度（-120dB 以下静音）。dB → linear amplitude (mute below -120dB).
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 {
        return 0.0;
    }
    10.0_f32.powf(db / 20.0)
}

/// 线性幅度 → dB。Linear amplitude → dB.
///
/// 预留：供 UI 回读当前增益等使用。
/// Reserved: for UI readback of current gain etc.
#[allow(dead_code)]
#[inline]
pub fn linear_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_mix_sums_and_limits() {
        let m = Mixer::new();
        let file = [1.0, -1.0];
        let mic = [1.0, -1.0];
        let mut out = [0.0; 2];
        m.mix(&file, &mic, &mut out);
        // 1+1 经 tanh 限幅 ≈ 0.964
        assert!(out[0] > 0.9 && out[0] <= 1.0, "out0={}", out[0]);
        assert!(out[1] < -0.9 && out[1] >= -1.0, "out1={}", out[1]);
    }

    #[test]
    fn mute_silences_source() {
        let m = Mixer::new();
        m.set_mic_gain_db(-120.0);
        let file = [0.0];
        let mic = [0.9];
        let mut out = [0.0];
        m.mix(&file, &mic, &mut out);
        assert_eq!(out[0], 0.0);
    }
}
