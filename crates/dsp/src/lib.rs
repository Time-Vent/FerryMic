//! dsp：定长（同帧数）处理器。
//!
//! 所有处理器实现 [`FixedProcessor`]，输入输出帧数相同。状态（延迟线、包络、
//! 相位等）由处理器自身持有，跨 buffer 保存，实时线程独占访问。
//!
//! 实时路径约束：`process` 不分配、不加锁、不做 IO、不 panic。

#![forbid(unsafe_code)]

mod biquad;
mod fixed;
mod gain;
mod limiter;

pub use biquad::{Biquad, BiquadKind, BiquadParams};
pub use fixed::FixedProcessor;
pub use gain::Gain;
pub use limiter::Limiter;
