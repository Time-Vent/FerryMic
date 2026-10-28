//! audio-engine：音频引擎（逻辑职责组织，不硬编码物理线程）。
//! audio-engine: the audio engine (organized by logical roles; physical threads are
//! not hardcoded).
//!
//! 三个执行角色：
//! Three execution roles:
//! - [`ControlExecutor`]：应用线程——处理命令、设备切换、参数验证、插件管理。
//!   `ControlExecutor`: application thread — commands, device switching, param
//!   validation, plugin management.
//! - [`DecodeExecutor`]：独立线程——文件解码、变速、预加载，填充文件 SPSC 环。
//!   `DecodeExecutor`: dedicated thread — file decode, varispeed, preload, fills the
//!   file SPSC ring.
//! - [`RealtimeCallback`]：输出设备回调——消费命令(≤32)、取 PCM、混音/DSP、写出。
//!   `RealtimeCallback`: output device callback — consume ≤32 commands, pull PCM,
//!   mix/DSP, write out.
//!
//! 实时回调约束：不分配、不加锁、不做 IO、不 panic。
//! Real-time callback constraint: no allocation, no lock, no IO, no panic.

#![forbid(unsafe_code)]

mod clock;
mod engine;
mod executors;

pub use clock::{DriftConfig, DriftController};
pub use engine::{Engine, EngineConfig, EngineEvent};
pub use executors::{ControlExecutor, DecodeExecutor, RealtimeCallback};
