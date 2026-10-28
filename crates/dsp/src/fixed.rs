//! `FixedProcessor` 接口与参数 ID 定义。
//! `FixedProcessor` trait and parameter ID definitions.

/// 定长处理器接口：输入输出帧数相同。
/// Fixed-length processor: input and output have the same frame count.
///
/// 实时线程调用。实现必须满足：
/// Called on the real-time thread. Implementations must satisfy:
/// - 不分配、不加锁、不做 IO、不 panic。No allocation, no lock, no IO, no panic.
/// - 状态跨 buffer 保存（延迟线、包络等）。State persists across buffers
///   (delay lines, envelopes, etc.).
/// - `set_param` 以有界方式生效（钳制到合法范围，不产生 NaN/Inf）。
///   `set_param` takes effect in a bounded way (clamped; no NaN/Inf).
pub trait FixedProcessor: Send {
    /// 处理 `frames` 帧；`input` 与 `output` 长度 = frames × channels。
    /// Process `frames` frames; `input` and `output` length = frames × channels.
    fn process(&mut self, input: &[f32], output: &mut [f32], frames: usize);

    /// 重置内部状态（延迟线清零、包络复位等）。用于设备切换或 seek。
    /// Reset internal state (clear delay lines, reset envelopes). Used on device
    /// switch or seek.
    fn reset(&mut self);

    /// 处理器引入的固定延迟（采样数）。默认 0。
    /// Fixed latency the processor introduces, in samples. Default 0.
    fn latency_samples(&self) -> usize {
        0
    }

    /// 设置命名参数。`value` 语义由具体处理器定义；实现必须钳制。
    /// Set a named parameter. `value` semantics are defined by each processor; the
    /// implementation must clamp.
    fn set_param(&mut self, id: u32, value: f32);

    /// 处理器名称（用于图配置与调试）。
    /// Processor name (for graph configuration and debugging).
    fn name(&self) -> &'static str;
}

/// 参数 ID 命名空间：`| 处理器类型(16bit) | 参数(16bit) |`。
/// Parameter ID namespace: `| processor kind(16bit) | parameter(16bit) |`.
///
/// 用 u32 表示，避免与具体处理器强绑定；引擎透传，不做类型化。
/// Represented as u32 to avoid strong coupling; the engine passes them through
/// without typing.
///
/// 部分常量尚未被第一阶段消费（供后续阶段使用），故加 `#[allow(dead_code)]`。
/// Some constants are not yet consumed in phase 1 (reserved for later), hence
/// `#[allow(dead_code)]`.
#[allow(dead_code)]
pub mod param {
    // Gain 处理器。Gain processor.
    pub const GAIN_DB: u32 = 0x0001_0001;
    // Limiter 处理器。Limiter processor.
    pub const LIMITER_THRESHOLD_DB: u32 = 0x0002_0001;
    pub const LIMITER_RELEASE_MS: u32 = 0x0002_0002;
    // Biquad 处理器。Biquad processor.
    pub const BIQUAD_TYPE: u32 = 0x0003_0001;
    pub const BIQUAD_FREQ_HZ: u32 = 0x0003_0002;
    pub const BIQUAD_Q: u32 = 0x0003_0003;
    pub const BIQUAD_GAIN_DB: u32 = 0x0003_0004;
}
