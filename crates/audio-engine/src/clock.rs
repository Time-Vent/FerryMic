//! 时钟架构与水位 PI 漂移控制。
//! Clock architecture and water-level PI drift control.
//!
//! 主时钟 = 输出设备回调（见架构文档 §6.1）。
//! Master clock = the output device callback (see architecture §6.1).
//!
//! 各异步源（麦克风、loopback、文件解码）经独立异步重采样向主时钟对齐。
//! Each async source (mic, loopback, file decode) aligns to the master clock via
//! independent async resampling.
//!
//! 本模块实现 [`DriftController`]：以环形缓冲**填充水位**为控制量，PI 输出限幅
//! ±1000 ppm，经低通平滑后作用于重采样比率，避免可闻变调。
//! This module implements `DriftController`: controlled by the ring-buffer **fill
//! level**, PI output clamped to ±1000 ppm, smoothed by a first-order low-pass to
//! avoid audible pitch change.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// PI 漂移控制器配置。PI drift controller configuration.
#[derive(Debug, Clone, Copy)]
pub struct DriftConfig {
    /// 目标水位（采样数）。Target fill level (samples).
    pub target_fill: u32,
    /// 比例增益。Proportional gain.
    pub kp: f32,
    /// 积分增益。Integral gain.
    pub ki: f32,
    /// 输出限幅（比率偏差，ppm）。Output clamp (ratio deviation, ppm).
    pub max_ppm: f32,
    /// 低通平滑系数（0..1；越小越平滑，响应越慢）。
    /// Low-pass smoothing coefficient (0..1; smaller = smoother, slower).
    pub smoothing: f32,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            target_fill: 4096,
            kp: 0.001,
            ki: 0.000_05,
            max_ppm: 1000.0,
            smoothing: 0.1,
        }
    }
}

/// 漂移控制器：非实时线程周期调用 `update`，实时侧只读 `ratio`。
/// Drift controller: non-realtime thread calls `update` periodically; the real-time
/// side only reads `ratio`.
pub struct DriftController {
    config: DriftConfig,
    /// 积分项。Integral term.
    integral: f32,
    /// 当前平滑后的比率（f32 位模式共享）。Current smoothed ratio (f32 bits shared).
    ratio_bits: Arc<AtomicU32>,
    /// 当前平滑比率（本地副本，用于增量平滑）。Local smoothed ratio.
    smoothed: f32,
}

impl DriftController {
    pub fn new(config: DriftConfig) -> Self {
        Self {
            config,
            integral: 0.0,
            ratio_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            smoothed: 1.0,
        }
    }

    /// 非实时线程周期调用：读环形缓冲水位，更新 PI，写 ratio_bits。
    /// Called periodically by a non-realtime thread: read fill level, update PI,
    /// write ratio_bits.
    pub fn update(&mut self, current_fill: u32) {
        let cfg = self.config;
        let err = cfg.target_fill as i64 - current_fill as i64; // 目标 - 当前（正值 = 欠水，需加快）
        let err = err as f32;

        // 积分项累计并做简单的反向饱和。
        // Accumulate integral with simple anti-windup.
        self.integral += cfg.ki * err;
        let max_int = cfg.max_ppm / cfg.ki.max(1e-9);
        self.integral = self.integral.clamp(-max_int, max_int);

        // 原始比率：>1 加快消费（水位低时），<1 放慢。
        // Raw ratio: >1 consumes faster (when fill is low), <1 slower.
        let raw = 1.0 + (cfg.kp * err + self.integral) / 1_000_000.0;
        let raw = raw.clamp(1.0 - cfg.max_ppm / 1_000_000.0, 1.0 + cfg.max_ppm / 1_000_000.0);

        // 一阶低通平滑。First-order low-pass smoothing.
        self.smoothed = self.smoothed * (1.0 - cfg.smoothing) + raw * cfg.smoothing;

        self.ratio_bits.store(self.smoothed.to_bits(), Ordering::Relaxed);
    }

    /// 实时侧只读当前重采样比率。
    /// Real-time side reads the current resample ratio (read-only).
    pub fn ratio(&self) -> f32 {
        f32::from_bits(self.ratio_bits.load(Ordering::Relaxed))
    }

    /// 共享的比率位（供跨线程无锁共享）。
    /// Shared ratio bits (for lock-free sharing across threads).
    pub fn ratio_shared(&self) -> Arc<AtomicU32> {
        self.ratio_bits.clone()
    }

    /// 复位积分与平滑到初始值（1.0 比率）。
    /// Reset integral and smoothing to initial (ratio 1.0).
    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.smoothed = 1.0;
        self.ratio_bits.store(1.0f32.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_fill_raises_ratio() {
        let mut c = DriftController::new(DriftConfig {
            target_fill: 4096,
            kp: 0.01,
            ki: 0.0,
            ..Default::default()
        });
        // 水位远低于目标 → 比率应 >1
        for _ in 0..100 {
            c.update(1000);
        }
        assert!(c.ratio() > 1.0, "ratio={}", c.ratio());
    }

    #[test]
    fn high_fill_lowers_ratio() {
        let mut c = DriftController::new(DriftConfig {
            target_fill: 4096,
            kp: 0.01,
            ki: 0.0,
            ..Default::default()
        });
        for _ in 0..100 {
            c.update(8000);
        }
        assert!(c.ratio() < 1.0, "ratio={}", c.ratio());
    }

    #[test]
    fn ratio_clamped_within_ppm() {
        let mut c = DriftController::new(DriftConfig {
            target_fill: 4096,
            kp: 100.0,
            ki: 0.0,
            max_ppm: 1000.0,
            ..Default::default()
        });
        for _ in 0..100 {
            c.update(0); // 极端欠水
        }
        let r = c.ratio();
        assert!(r <= 1.001 + 1e-4, "ratio={}", r);
        assert!(r >= 0.999 - 1e-4, "ratio={}", r);
    }
}
