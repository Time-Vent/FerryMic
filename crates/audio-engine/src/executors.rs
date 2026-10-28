//! 三执行器角色定义。
//! Definition of the three executor roles.
//!
//! 这些是**逻辑角色**（职责边界），不是硬编码的线程。物理线程由平台适配器
//! （cpal callback）与装配层（application）决定。这样保持引擎的平台无关性。
//! These are **logical roles** (responsibility boundaries), not hardcoded threads.
//! Physical threads are decided by the platform adapter (cpal callback) and the
//! assembly layer (application), keeping the engine platform-agnostic.

use audio_types::EngineEvent;

/// 引擎控制命令（应用线程 → 引擎）。
/// Engine control commands (application thread → engine).
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// 设置混音参数：源 ID 与增益等。Set mix params for a source.
    SetSourceGain { source_id: u32, gain_db: f32 },
    /// 设置文件源的播放速率。Set file source playback rate.
    SetFileSpeed { source_id: u32, speed: f32 },
    /// 启动某源。Start a source.
    StartSource { source_id: u32 },
    /// 停止某源。Stop a source.
    StopSource { source_id: u32 },
    /// 切换输出设备（异步，需重建流）。Switch output device (async, rebuild stream).
    SwitchOutput { device_id: String },
    /// 停机。Shutdown.
    Shutdown,
}

/// 引擎对外上报的事件（引擎 → 应用线程）。
/// Events reported from the engine to the application thread.
#[derive(Debug, Clone)]
pub enum EngineEventReport {
    /// 引擎事件（欠载、漂移警告等）。Engine events (underrun, drift warning, etc.).
    Event(EngineEvent),
    /// 致命错误（引擎停止）。Fatal error (engine stopped).
    Fatal(String),
}

/// 控制执行器：处理命令、设备切换、参数验证、插件管理。
/// Control executor: handles commands, device switching, param validation, plugin
/// management.
///
/// 它是引擎的「前台」，一切非实时决策集中于此。
/// It is the engine's "front desk"; all non-realtime decisions concentrate here.
pub trait ControlExecutor: Send {
    /// 投递一条命令。Deliver a command.
    fn submit(&mut self, cmd: EngineCommand);

    /// 拉取并处理一批事件。Drain and handle a batch of events.
    fn poll_events(&mut self) -> Vec<EngineEventReport>;

    /// 请求停机。Request shutdown.
    fn shutdown(&mut self);
}

/// 解码执行器：文件解码、变速、预加载，填充文件 SPSC 环。
/// Decode executor: file decode, varispeed, preload, fills the file SPSC ring.
///
/// 独立于实时回调运行；其输出进入 SPSC 环供实时侧消费。
/// Runs independently of the real-time callback; its output enters an SPSC ring
/// consumed by the real-time side.
pub trait DecodeExecutor: Send {
    /// 以给定速率预填充 `frames` 帧到环。Pre-fill `frames` frames into the ring at rate.
    fn preload(&mut self, frames: usize, speed: f32) -> usize;
    /// 后台循环体：每次填充一批，直到 stop。Background loop body: fill a batch each time.
    fn tick(&mut self, batch_frames: usize) -> usize;
    /// 请求停止后台循环。Request the background loop to stop.
    fn stop(&mut self);
}

/// 实时回调：由输出设备回调驱动。
/// Real-time callback: driven by the output device callback.
///
/// 职责（每轮）：
/// Per-cycle responsibilities:
/// 1. 有限条数消费命令（≤32）。Consume a bounded number of commands (≤32).
/// 2. 从 SPSC 环取 PCM。Pull PCM from the SPSC ring.
/// 3. 混音/DSP。Mix/DSP.
/// 4. 写出到输出缓冲。Write to the output buffer.
pub trait RealtimeCallback: Send {
    /// 输出缓冲区帧数（每回调）。Frames per output callback.
    fn buffer_frames(&self) -> usize;
    /// 输出声道数。Output channel count.
    fn channels(&self) -> usize;

    /// 实时回调主体：处理 `frames` 帧，写入 `output`（交错）。
    /// Real-time callback body: process `frames` frames, write `output` (interleaved).
    ///
    /// 禁止分配/加锁/IO/panic。No alloc/lock/IO/panic.
    fn audio_callback(&mut self, output: &mut [f32]);
}
