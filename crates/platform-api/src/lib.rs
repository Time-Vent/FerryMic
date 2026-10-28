//! platform-api：平台能力 trait 契约层。
//!
//! 本 crate 只定义 trait 与错误类型，不包含任何平台实现。引擎只依赖这些
//! trait，不依赖具体平台 API、具体环形缓冲库或具体线程模型。
//!
//! 实时回调约束（对所有实现者强制）：
//! - 回调路径不分配、不加锁、不做 IO、不 panic。
//! - 跨线程状态通过无锁原语（`Atomic*`）共享。

#![forbid(unsafe_code)]

use audio_types::{DeviceInfo, StreamConfig};
use thiserror::Error;

/// 平台层统一错误类型。实时路径不得构造此错误（它会携带堆分配字符串）。
#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("stream config not supported by device {device}: {config:?}")]
    UnsupportedConfig {
        device: String,
        config: StreamConfig,
    },
    #[error("backend error: {0}")]
    Backend(String),
    #[error("stream is not running")]
    StreamNotRunning,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("loopback feedback loop detected: source == target")]
    FeedbackLoopDetected,
    #[error("not implemented on this platform: {0}")]
    NotSupported(String),
}

/// 音频后端抽象：枚举设备 + 打开输入/输出流。
///
/// 所有平台均由 `audio-cpal` 实现。输入/输出流的实际回调线程由后端决定，
/// 引擎只按逻辑职责（RealtimeCallback / 独立取数）组织，不假设同线程。
pub trait AudioBackend: Send {
    /// 枚举系统中可用的音频设备。
    fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError>;

    /// 打开输出流。`callback` 为实时回调，直接由后端在音频线程调用。
    /// Open an output stream. `callback` runs on the backend's audio thread.
    ///
    /// 返回一个句柄；句柄 drop 时停止并销毁流。
    /// Returns a handle; dropping it stops and destroys the stream.
    fn open_output<F>(
        &self,
        config: &StreamConfig,
        callback: F,
    ) -> Result<Box<dyn OutputStreamHandle>, PlatformError>
    where
        F: FnMut(&mut [f32]) + Send + 'static;

    /// 打开输入流。`callback` 为实时回调，直接由后端在音频线程调用。
    /// Open an input stream. `callback` runs on the backend's audio thread.
    fn open_input<F>(
        &self,
        config: &StreamConfig,
        callback: F,
    ) -> Result<Box<dyn InputStreamHandle>, PlatformError>
    where
        F: FnMut(&[f32]) + Send + 'static;
}

/// 输出流句柄。drop 时停止流。
/// Output stream handle. Stops the stream on drop.
///
/// 注意：**不加 `Send` 约束**。流句柄由平台后端创建与销毁，通常绑定创建线程，
/// 不跨线程传递（如 cpal 的 `Stream` 在某些平台有意为非 Send）。跨线程安全由
/// 真正需要跨线程的 trait（如 `AudioBackend`）保证。
/// Note: **no `Send` bound**. Stream handles are created/destroyed by the platform
/// backend and usually stay on one thread (cpal's `Stream` is deliberately !Send on
/// some platforms). Cross-thread safety is guaranteed by traits that actually need
/// it (e.g. `AudioBackend`).
pub trait OutputStreamHandle {
    /// 暂停数据流（保留设备占用）。Pause the stream (device stays occupied).
    fn pause(&mut self) -> Result<(), PlatformError>;
    /// 恢复数据流。Resume the stream.
    fn resume(&mut self) -> Result<(), PlatformError>;
}

/// 输入流句柄。drop 时停止流。
/// Input stream handle. Stops the stream on drop.
///
/// 同 [`OutputStreamHandle`]，不加 `Send` 约束。
/// Like [`OutputStreamHandle`], no `Send` bound.
pub trait InputStreamHandle {
    fn pause(&mut self) -> Result<(), PlatformError>;
    fn resume(&mut self) -> Result<(), PlatformError>;
}

/// 虚拟麦克风抽象。仅 macOS/Linux 第二阶段需要自行创建虚拟设备端点时实现。
/// Virtual microphone abstraction. Only implemented in phase 2 for macOS/Linux when
/// a virtual device endpoint must be created.
///
/// Windows 不实现此 trait（见 `SystemAudioLoopback` 与 `AudioBackend::open_output`）。
/// Windows does not implement this trait (see `SystemAudioLoopback` and
/// `AudioBackend::open_output`).
pub trait VirtualMic: Send {
    fn create(&mut self, config: &StreamConfig)
        -> Result<Box<dyn VirtualMicHandle>, PlatformError>;

    /// 由独立线程 drain SPSC 环后调用，**不在实时回调上执行**。
    /// Called after draining the SPSC ring on a dedicated thread, NOT on the realtime
    /// callback.
    fn write_pcm(&mut self, buf: &[f32]) -> Result<(), PlatformError>;

    fn destroy(&mut self);
}

/// 虚拟麦克风端点句柄。
/// Virtual microphone endpoint handle.
pub trait VirtualMicHandle {}

/// 系统声音 loopback 抽象。Windows 第一阶段实现；macOS/Linux 第一阶段不实现。
/// System audio loopback abstraction. Windows implements it in phase 1; macOS/Linux
/// do not in phase 1.
pub trait SystemAudioLoopback: Send {
    fn start(&mut self, config: &StreamConfig) -> Result<Box<dyn LoopbackHandle>, PlatformError>;
    fn stop(&mut self);
}

/// loopback 流句柄。
/// Loopback stream handle.
pub trait LoopbackHandle {}

/// 权限门槛抽象。各平台按需实现（macOS 麦克风/屏幕录制权限等）。
pub trait PermissionGate: Send {
    fn ensure_microphone(&self) -> Result<(), PlatformError>;
    fn ensure_system_audio(&self) -> Result<(), PlatformError>;
}
