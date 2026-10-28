//! Gain（增益）处理器：无状态乘法，带平滑以消除 zipper 噪声。

use crate::fixed::{param, FixedProcessor};

/// 带一阶平滑的增益处理器。目标增益以每采样固定斜率逼近，避免瞬时跳变。
pub struct Gain {
    /// 目标线性增益。
    target: f32,
    /// 当前生效增益（对每个声道统一）。
    current: f32,
    /// 每采样逼近斜率；0 表示立即生效。
    step: f32,
}

impl Gain {
    pub fn new(gain_db: f32) -> Self {
        let mut g = Self {
            target: 1.0,
            current: 1.0,
            step: 0.0,
        };
        g.set_gain_db(gain_db, 0);
        g
    }

    /// 设置增益。`smoothing_samples` 为平滑窗口（采样数）；0 为立即生效。
    pub fn set_gain_db(&mut self, gain_db: f32, smoothing_samples: usize) {
        let lin = db_to_linear(gain_db);
        let lin = if lin.is_nan() {
            0.0
        } else {
            lin.clamp(0.0, 64.0)
        };
        self.target = lin;
        if smoothing_samples > 0 {
            self.step = (self.target - self.current) / smoothing_samples as f32;
        } else {
            self.current = lin;
            self.step = 0.0;
        }
    }

    #[inline]
    fn advance(&mut self) {
        if self.step != 0.0 {
            if (self.target - self.current).abs() <= self.step.abs() {
                self.current = self.target;
                self.step = 0.0;
            } else {
                self.current += self.step;
            }
        }
    }
}

impl FixedProcessor for Gain {
    fn process(&mut self, input: &[f32], output: &mut [f32], _frames: usize) {
        for (o, &i) in output.iter_mut().zip(input.iter()) {
            *o = i * self.current;
            self.advance();
        }
    }

    fn reset(&mut self) {
        // 增益无状态，但保持增益值；不做任何事避免破坏配置。
    }

    fn set_param(&mut self, id: u32, value: f32) {
        if id == param::GAIN_DB {
            self.set_gain_db(value, 64);
        }
    }

    fn name(&self) -> &'static str {
        "Gain"
    }
}

/// dB 转线性幅度。
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// 线性幅度转 dB。
/// Linear amplitude → dB.
///
/// 预留：供 UI 显示与后续阶段（如 limiter 阈值回读）使用。
/// Reserved: for UI display and later phases (e.g. limiter threshold readback).
#[allow(dead_code)]
#[inline]
pub fn linear_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::FixedProcessor;

    #[test]
    fn unity_gain_is_noop() {
        let mut g = Gain::new(0.0);
        let input = [0.5, -0.5, 0.25];
        let mut out = [0.0; 3];
        g.process(&input, &mut out, 3);
        assert_eq!(out, input);
    }

    #[test]
    fn muting_sets_zero() {
        let mut g = Gain::new(f32::NEG_INFINITY);
        let input = [0.5, -0.5];
        let mut out = [0.0; 2];
        g.process(&input, &mut out, 2);
        assert_eq!(out, [0.0, 0.0]);
    }

    #[test]
    fn db_linear_roundtrip() {
        assert!((db_to_linear(6.0) - 1.995_262).abs() < 1e-4);
        assert!((linear_to_db(2.0) - 6.0206).abs() < 1e-3);
    }
}
