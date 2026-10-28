//! application：装配层，把引擎、cpal 后端、音源组装成可启动/停止的完整应用。
//! application: assembly layer, wiring the engine, cpal backend, and audio sources
//! into a startable/stoppable application.
//!
//! 这一层负责「何时用哪个线程跑哪个执行器」的落地决策，但决策本身仍是逻辑性的：
//! This layer makes the "which thread runs which executor" decision concrete, yet
//! still logically:
//! - 控制执行器：调用方（CLI/UI）所在线程。Control executor: caller's thread (CLI/UI).
//! - 解码执行器：专用后台线程（文件解码 + 变速 + 预填充环）。Decode executor:
//!   dedicated background thread (file decode + varispeed + prefill ring).
//! - 实时回调：cpal 输出回调线程。Real-time callback: cpal output callback thread.

#![forbid(unsafe_code)]

mod app;
mod mixer;

pub use app::{App, AppConfig, AppError, RunHandle};
pub use mixer::{Mixer, MixerInput};
