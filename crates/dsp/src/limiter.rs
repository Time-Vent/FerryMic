//! Limiter（限幅）处理器：峰值限幅，带可调释放包络。

use crate::fixed::{param, FixedProcessor};
use crate::gain::{db_to_linear, linear_to_db};

/// 峰值限幅器。超过阈值时以瞬时增益压低的硬限幅，释放阶段按指数衰减恢复。
///
/// 第一阶段为简单硬限幅（无 attack），足够防止削波；Compressor 留待第二阶段。
pub struct Limiter {
    /// 阈值（线性幅度，0..=1）。
    threshold: f32,
    /// 每采样释放系数（<1，越大释放越慢）。
    release_coeff: f32,
    /// 当前增益包络（1 = 不衰减）。
    envelope: f32,
}

impl Limiter {
    pub fn new(threshold_db: f32, release_ms: f32, sample_rate: u32) -> Self {
        let mut l = Self {
            threshold: 1.0,
            release_coeff: 1.0,
            envelope: 1.0,
        };
        l.set_threshold_db(threshold_db);
        l.set_release_ms(release_ms, sample_rate);
        l
    }

    pub fn set_threshold_db(&mut self, db: f32) {
        let lin = db_to_linear(db);
        self.threshold = if lin.is_nan() {
            0.0
        } else {
            lin.clamp(0.0, 1.0)
        };
    }

    pub fn set_release_ms(&mut self, ms: f32, sample_rate: u32) {
        let ms = ms.max(1.0);
        let samples = (ms / 1000.0 * sample_rate as f32).max(1.0);
        // 指数释放：time constant 在 samples 帧内衰减到约 1/e。
        self.release_coeff = (-1.0_f32 / samples).exp();
    }
}

impl FixedProcessor for Limiter {
    fn process(&mut self, input: &[f32], output: &mut [f32], _frames: usize) {
        for (o, &i) in output.iter_mut().zip(input.iter()) {
            let abs = i.abs();
            // 若峰值超过阈值，压到阈值并收紧包络；否则释放。
            let target = if abs > self.threshold && abs > 0.0 {
                self.threshold / abs
            } else {
                1.0
            };
            if target < self.envelope {
                self.envelope = target; // 硬限幅，立即压低
            } else {
                // 释放：向 1 恢复
                self.envelope = 1.0 - (1.0 - self.envelope) * self.release_coeff;
            }
            *o = i * self.envelope;
        }
    }

    fn reset(&mut self) {
        self.envelope = 1.0;
    }

    fn set_param(&mut self, id: u32, value: f32) {
        if id == param::LIMITER_THRESHOLD_DB {
            self.set_threshold_db(value)
        }
    }

    fn name(&self) -> &'static str {
        "Limiter"
    }
}

/// 便捷：从当前阈值导出 dB（供 UI 读取）。
/// Convenience: derive dB from a linear threshold (for UI readback).
///
/// 预留：第一阶段无 UI，后续阶段读取限幅器阈值显示时使用。
/// Reserved: phase 1 has no UI; used later when reading limiter threshold for display.
#[allow(dead_code)]
pub fn limiter_threshold_db(threshold_linear: f32) -> f32 {
    linear_to_db(threshold_linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_clip_below_threshold() {
        let mut l = Limiter::new(-1.0, 50.0, 48_000);
        let input = [0.5, -0.5, 0.8];
        let mut out = [0.0; 3];
        l.process(&input, &mut out, 3);
        assert_eq!(out, input); // -1dB ≈ 0.891，0.8 不触发
    }

    #[test]
    fn clips_above_threshold() {
        let mut l = Limiter::new(-10.0, 50.0, 48_000); // ~0.316 阈值
        let input = [1.0, -1.0];
        let mut out = [0.0; 2];
        l.process(&input, &mut out, 2);
        // 峰值不可能超过阈值
        assert!(out[0].abs() <= 0.32, "out0={}", out[0]);
        assert!(out[1].abs() <= 0.32, "out1={}", out[1]);
    }

    #[test]
    fn reset_clears_envelope() {
        let mut l = Limiter::new(-20.0, 1000.0, 48_000);
        l.process(&[1.0], &mut [0.0], 1);
        assert!(l.envelope < 1.0);
        l.reset();
        assert_eq!(l.envelope, 1.0);
    }
}
