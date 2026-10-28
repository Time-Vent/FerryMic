//! 麦克风管理：运行中切换设备、断开检测、自动重连、跟随系统默认设备。
//! Microphone management: runtime device switching, disconnect detection,
//! auto-reconnect, and following the system default device.
//!
//! 麦克风来源经常变化（手机 mic 软件、蓝牙耳机随时连上/断开），因此输入流不在启动时
//! 固定，而由一个专用「麦克风线程」持有：
//! The mic source changes often (phone mic apps, Bluetooth headsets connecting and
//! dropping), so the input stream isn't fixed at startup; a dedicated "mic thread"
//! owns it:
//!
//! - Web API 通过 [`MicRequest`] 请求切换设备，线程打开新流成功后才替换旧流。
//!   The web API requests switches via [`MicRequest`]; the thread replaces the old
//!   stream only after the new one opens successfully.
//! - 每秒巡检一次：流出错（设备断开）则按原选择重连；选择为「跟随系统默认」时，
//!   默认设备变化即自动切换。
//!   Every second it checks: if the stream failed (device gone) it reconnects using
//!   the same selection; when following the system default, it switches as soon as
//!   the default device changes.
//! - 每次打开新流都新建一个 SPSC 环，把消费端经通道交给输出回调；输出回调把旧消费端
//!   送回本线程释放，实时线程不做大块内存释放。
//!   Each new stream gets a fresh SPSC ring whose consumer is handed to the output
//!   callback over a channel; the callback sends the old consumer back here to be
//!   freed, so the real-time thread never frees large buffers.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio_cpal::CpalBackend;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use platform_api::InputStreamHandle;
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;

use crate::settings::SettingsStore;

/// 巡检间隔（断开检测、默认设备变化检测）。
/// Watch interval (disconnect detection, default-device change detection).
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// 麦克风参数、电平与状态（控制侧 ↔ 实时侧共享）。
/// Mic parameters, level and status (shared between control and real-time sides).
///
/// 实时回调只读写原子量；`status` 带锁，只在控制侧访问。
/// The real-time callback only touches atomics; `status` is locked and only used on
/// the control side.
pub struct MicControl {
    gain_db: AtomicU32,
    muted: AtomicBool,
    /// 峰值电平（线性，带衰减），供 UI 电平表。Peak level (linear, decaying) for UI.
    level: AtomicU32,
    /// 当前设备状态（控制侧）。Current device status (control side).
    status: Mutex<MicStatus>,
}

impl MicControl {
    pub fn new(muted: bool, gain_db: f32) -> Self {
        Self {
            gain_db: AtomicU32::new(gain_db.to_bits()),
            muted: AtomicBool::new(muted),
            level: AtomicU32::new(0.0f32.to_bits()),
            status: Mutex::new(MicStatus::default()),
        }
    }

    pub fn gain_db(&self) -> f32 {
        f32::from_bits(self.gain_db.load(Ordering::Relaxed))
    }

    pub fn set_gain_db(&self, db: f32) {
        self.gain_db.store(db.to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// 当前电平（0..1，增益后、静音前）。Current level (0..1, post-gain, pre-mute).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// 实时侧：用本次回调的峰值更新电平（快升慢降）。
    /// Real-time side: update the level with this callback's peak (fast attack, decay).
    pub fn update_level(&self, peak: f32) {
        let next = peak.max(self.level() * 0.9);
        self.level.store(next.to_bits(), Ordering::Relaxed);
    }

    /// 设备状态快照（控制侧）。Device status snapshot (control side).
    pub fn status(&self) -> MicStatus {
        self.status.lock().unwrap().clone()
    }

    fn update_status(&self, f: impl FnOnce(&mut MicStatus)) {
        f(&mut self.status.lock().unwrap());
    }
}

/// 麦克风设备状态。Mic device status.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MicStatus {
    /// 用户选择的设备名；`None` 表示跟随系统默认。
    /// Device name chosen by the user; `None` = follow the system default.
    pub selection: Option<String>,
    /// 正在使用的设备名。Device currently in use.
    pub device: Option<String>,
    /// 设备采样率。Device sample rate.
    pub sample_rate: Option<u32>,
    /// 是否正在采集（流已打开且未出错）。Whether capturing (stream open, no error).
    pub connected: bool,
    /// 未连接时的原因或提示。Reason or hint when not connected.
    pub note: Option<String>,
}

/// 控制侧 → 麦克风线程的请求。Request from the control side to the mic thread.
pub enum MicRequest {
    /// 选择设备（`None` = 自动：优先系统默认的硬件麦克风），结果经 `reply` 返回。
    /// Select a device (`None` = auto: prefer the system default hardware mic); the
    /// result comes back on `reply`.
    Select {
        device: Option<String>,
        reply: Sender<Result<(), String>>,
    },
    /// 输出设备变了：按新的输出名（反馈环检测）与采样率重新打开麦克风。
    /// The output device changed: reopen the mic for the new output name (loop
    /// detection) and sample rate.
    OutputChanged { output_name: String, out_rate: u32 },
}

/// 输出回调持有的麦克风端点：接收新环的消费端，送回旧消费端。可克隆：
/// 重建输出流时新的回调拿一份。
/// Mic endpoints held by the output callback: receives new ring consumers and sends
/// old ones back. Cloneable: a rebuilt output stream's callback takes a copy.
#[derive(Clone)]
pub struct MicLink {
    pub cons_rx: Receiver<HeapCons<f32>>,
    pub retired_tx: Sender<HeapCons<f32>>,
}

/// 麦克风线程一侧的通道端点。The mic thread's side of the channels.
pub struct MicEnds {
    cons_tx: Sender<HeapCons<f32>>,
    retired_rx: Receiver<HeapCons<f32>>,
}

/// 创建麦克风线程与输出回调之间的通道。Create the channels between the mic thread and
/// the output callback.
pub fn link_channels() -> (MicEnds, MicLink) {
    let (cons_tx, cons_rx) = bounded::<HeapCons<f32>>(4);
    let (retired_tx, retired_rx) = bounded::<HeapCons<f32>>(4);
    (
        MicEnds {
            cons_tx,
            retired_rx,
        },
        MicLink {
            cons_rx,
            retired_tx,
        },
    )
}

/// 启动麦克风线程，返回请求发送端。Start the mic thread; returns the request sender.
///
/// - `initial`：初始选择（`None` = 自动）。Initial selection (`None` = auto).
/// - `output_name`：输出设备名，用于反馈环检测。Output device name, for loop detection.
/// - `out_rate`：输出采样率，麦克风会被重采样到该速率。Output rate the mic is
///   resampled to.
pub fn spawn_mic_thread(
    ends: MicEnds,
    initial: Option<String>,
    output_name: String,
    out_rate: u32,
    mic: Arc<MicControl>,
    settings: Arc<SettingsStore>,
) -> Sender<MicRequest> {
    let (req_tx, req_rx) = bounded::<MicRequest>(8);
    let MicEnds {
        cons_tx,
        retired_rx,
    } = ends;

    std::thread::Builder::new()
        .name("soundbridge-mic".into())
        .spawn(move || {
            let mut t = MicThread {
                backend: CpalBackend::new(),
                output_name,
                out_rate,
                mic,
                cons_tx,
                stream: None,
                selection: initial,
                last_warning: None,
            };
            t.mic.update_status(|s| s.selection = t.selection.clone());
            let sel = t.selection.clone();
            if let Err(e) = t.open(sel) {
                // 与巡检重连时的提示一致。Same wording as the watcher's retry note.
                t.set_unavailable(trf!(
                    "等待麦克风连接… {e}",
                    "Waiting for the microphone… {e}"
                ));
            }

            loop {
                match req_rx.recv_timeout(WATCH_INTERVAL) {
                    Ok(MicRequest::Select { device, reply }) => {
                        let result = t.open(device.clone());
                        if result.is_ok() {
                            t.selection = device.clone();
                            t.mic.update_status(|s| s.selection = device.clone());
                            settings.update(|s| s.mic_device = device);
                        }
                        let _ = reply.send(result);
                    }
                    Ok(MicRequest::OutputChanged {
                        output_name,
                        out_rate,
                    }) => {
                        t.output_name = output_name;
                        t.out_rate = out_rate;
                        // 新的输出回调需要新环；可能因反馈环检测而打不开。
                        // The new output callback needs a new ring; the loop check may refuse.
                        t.stream = None;
                        let sel = t.selection.clone();
                        if let Err(e) = t.open(sel) {
                            t.set_unavailable(trf!(
                                "等待麦克风连接… {e}",
                                "Waiting for the microphone… {e}"
                            ));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => t.watch(),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                // 释放输出回调送回的旧环。Free old rings sent back by the output callback.
                while retired_rx.try_recv().is_ok() {}
            }
        })
        .expect("spawn mic thread");

    req_tx
}

/// 麦克风线程状态。Mic thread state.
struct MicThread {
    backend: CpalBackend,
    output_name: String,
    out_rate: u32,
    mic: Arc<MicControl>,
    /// 新环消费端 → 输出回调。New ring consumers → output callback.
    cons_tx: Sender<HeapCons<f32>>,
    /// 当前输入流及其「已出错」标记。Current input stream and its "failed" flag.
    stream: Option<(Box<dyn InputStreamHandle>, Arc<AtomicBool>)>,
    /// 用户选择（`None` = 跟随系统默认）。User selection (`None` = follow default).
    selection: Option<String>,
    /// 上次记录的警告，避免巡检时重复刷日志。Last logged warning, to avoid log spam.
    last_warning: Option<String>,
}

impl MicThread {
    /// 自动选择：系统默认录音设备是硬件麦克风就用它，否则用第一个硬件麦克风。
    /// 「硬件」= 不是虚拟音频线、不是回录设备、不与当前输出构成反馈环。
    /// Auto pick: the system default if it's a hardware mic, else the first hardware
    /// mic. "Hardware" = not a virtual cable, not a loopback device, no loop with the
    /// current output.
    fn auto_pick(&self) -> Option<String> {
        let usable = |n: &str| !is_virtual_device(n) && input_risk(n, &self.output_name).is_none();
        if let Ok(d) = self.backend.resolve_input(None) {
            if usable(d.name()) {
                return Some(d.name().to_string());
            }
        }
        self.backend
            .input_device_names()
            .into_iter()
            .find(|n| usable(n))
    }

    /// 打开 `device`（`None` = 自动选择）并替换当前流。失败时保留当前流。
    /// Open `device` (`None` = auto pick) and replace the current stream. On failure
    /// the current stream is kept.
    fn open(&mut self, device: Option<String>) -> Result<(), String> {
        let target = match &device {
            Some(d) => d.clone(),
            None => self.auto_pick().ok_or(tr!(
                "没有找到硬件麦克风（虚拟音频线不会被当作麦克风）",
                "No hardware microphone found (virtual cables aren't used as mics)"
            ))?,
        };
        let dev = self.backend.resolve_input(Some(&target)).map_err(|e| {
            trf!(
                "找不到麦克风「{target}」：{e}",
                "Microphone \"{target}\" not found: {e}"
            )
        })?;
        let name = dev.name().to_string();
        if let Some(risk) = input_risk(&name, &self.output_name) {
            return Err(risk.message(&name, &self.output_name));
        }

        let cfg = dev.config();
        let rate = cfg.sample_rate.0;
        // 麦克风环：单声道、输出采样率，容量 0.5 秒。
        // Mic ring: mono at the output rate, 0.5 s capacity.
        let (prod, cons) = HeapRb::<f32>::new((self.out_rate / 2) as usize).split();
        let mut capture = MicCapture::new(prod, cfg.channels.0 as usize, rate, self.out_rate);
        let failed = Arc::new(AtomicBool::new(false));
        let failed_cb = failed.clone();
        let handle = self
            .backend
            .build_input(
                dev,
                move |data: &[f32]| capture.push(data),
                move |_| failed_cb.store(true, Ordering::Relaxed),
            )
            .map_err(|e| {
                trf!(
                    "无法打开麦克风「{name}」：{e}",
                    "Can't open microphone \"{name}\": {e}"
                )
            })?;

        self.cons_tx.try_send(cons).map_err(|_| {
            tr!(
                "音频线程未响应，请稍后重试",
                "The audio thread is not responding; try again"
            )
            .to_string()
        })?;
        // 替换旧流（drop 即停止）。Replace the old stream (dropping stops it).
        self.stream = Some((handle, failed));
        self.last_warning = None;
        self.mic.update_status(|s| {
            s.device = Some(name.clone());
            s.sample_rate = Some(rate);
            s.connected = true;
            s.note = None;
        });
        log::info!("麦克风 microphone: {name} ({rate} Hz)");
        Ok(())
    }

    /// 标记为不可用并记录原因（同一原因只记一次日志）。
    /// Mark unavailable with a reason (each distinct reason is logged once).
    fn set_unavailable(&mut self, note: String) {
        if self.last_warning.as_deref() != Some(&note) {
            log::warn!("麦克风不可用 mic unavailable: {note}");
            self.last_warning = Some(note.clone());
        }
        self.mic.update_status(|s| {
            s.connected = false;
            s.note = Some(note);
        });
    }

    /// 巡检：断开则重连；跟随默认时检测默认设备变化。
    /// Watch: reconnect when disconnected; when following the default, detect changes.
    fn watch(&mut self) {
        let failed = self
            .stream
            .as_ref()
            .is_none_or(|(_, f)| f.load(Ordering::Relaxed));

        if failed {
            if self.stream.take().is_some() {
                log::warn!("麦克风已断开 mic disconnected, waiting to reconnect");
                self.mic.update_status(|s| s.device = None);
            }
            let sel = self.selection.clone();
            if let Err(e) = self.open(sel) {
                self.set_unavailable(trf!(
                    "等待麦克风连接… {e}",
                    "Waiting for the microphone… {e}"
                ));
            }
            return;
        }

        if self.selection.is_none() {
            // 自动模式：自动选出的设备变了（如连上手机 mic、改了系统默认）就切过去，
            // 切换失败则保留当前设备。
            // Auto mode: switch when the auto pick changes (phone mic connected, system
            // default changed); keep the current device on failure.
            let Some(pick) = self.auto_pick() else {
                return;
            };
            let current = self.mic.status().device;
            if current.as_deref() != Some(pick.as_str()) {
                if let Err(e) = self.open(None) {
                    if self.last_warning.as_deref() != Some(&e) {
                        log::warn!("未切换到新的默认麦克风 not switching to new default: {e}");
                        self.last_warning = Some(e);
                    }
                }
            }
        }
    }
}

/// 虚拟音频设备的名称关键词（小写）。只有命中这些关键词的设备才会被判为「同一条虚拟线的
/// 两端」，避免把同一块声卡的麦克风和扬声器（如 Realtek）误判为反馈环。
/// Name keywords (lowercase) of virtual audio devices. Only devices matching these are
/// treated as "two ends of one virtual cable", so a physical card's mic and speakers
/// (e.g. Realtek) aren't mistaken for a loop.
const VIRTUAL_DEVICE_HINTS: &[&str] = &[
    "virtual",     // VB-Audio Virtual Cable、Virtual Audio Cable (VAC) 等
    "cable",       // VB-CABLE A/B、Hi-Fi Cable
    "vb-audio",    // VB-Audio 全系列 / all VB-Audio products
    "voicemeeter", // VoiceMeeter
    "vaio",        // VoiceMeeter 的虚拟 I/O 驱动 / VoiceMeeter virtual I/O driver
    "blackhole",   // macOS BlackHole
    "soundflower", // macOS Soundflower
    "loopback",    // macOS Loopback 等 / Loopback etc.
];

/// 回录设备的名称关键词（小写）：这类「麦克风」录的是电脑正在播放的声音。
/// Name keywords (lowercase) of loopback capture devices: such "mics" record whatever
/// the computer is playing.
const LOOPBACK_INPUT_HINTS: &[&str] = &[
    "stereo mix",   // Windows 立体声混音（英文系统）/ Windows Stereo Mix
    "立体声混音",   // Windows 立体声混音（中文系统）/ Stereo Mix on Chinese Windows
    "what u hear",  // Creative 声卡 / Creative sound cards
    "wave out mix", // 部分声卡驱动 / some sound card drivers
    "monitor of ",  // Linux PulseAudio/PipeWire 输出监听源 / Linux output monitor source
];

/// 端点名中表示方向的词，比较「是否同一条虚拟线」时去掉。
/// Direction words in endpoint names, removed when comparing for "same cable".
const DIRECTION_WORDS: &[&str] = &[
    "input", "output", "in", "out", "playback", "capture", "record", "source", "sink",
];

/// 不能当麦克风用的输入设备类型。Kinds of input device that must not be used as the mic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRisk {
    /// 与输出是同一个虚拟音频设备的两端：输出会被录回自身，形成反馈环（ADR-0006）。
    /// The other end of the virtual device being output to: the output is recorded
    /// back into itself — a feedback loop (ADR-0006).
    FeedbackLoop,
    /// 回录设备（立体声混音等）：会把游戏等所有正在播放的声音录进去。
    /// A loopback capture device (Stereo Mix etc.): would record all playing sound,
    /// games included.
    SystemLoopback,
}

impl InputRisk {
    /// 给用户看的说明。User-facing explanation.
    pub fn message(self, input: &str, output: &str) -> String {
        match self {
            Self::FeedbackLoop => trf!(
                "「{input}」与输出「{output}」是同一个虚拟音频设备的两端，会形成反馈环，请选择真实麦克风",
                "\"{input}\" is the other end of the output \"{output}\" and would loop back; pick a real microphone"
            ),
            Self::SystemLoopback => trf!(
                "「{input}」是回录设备，会把电脑正在播放的所有声音（包括游戏）录进去，请选择真实麦克风",
                "\"{input}\" records everything the computer plays (games included); pick a real microphone"
            ),
        }
    }
}

/// 是否为虚拟音频设备（按名称关键词）。用于区分硬件麦克风，以及判断输出是虚拟线还是扬声器。
/// Whether a device is a virtual audio device (by name keywords). Used to tell hardware
/// mics apart and whether the output is a virtual cable or a speaker.
pub fn is_virtual_device(name: &str) -> bool {
    let n = name.to_lowercase();
    VIRTUAL_DEVICE_HINTS.iter().any(|h| n.contains(h))
}

/// 游戏里该选的麦克风：输出所在虚拟设备的另一端（在输入设备里找与输出构成反馈环的那个）。
/// 输出是普通扬声器时返回 `None`。
/// The mic a game should pick: the other end of the virtual device being output to
/// (the input that would loop with the output). `None` when outputting to a speaker.
pub fn game_mic_for(output: &str, inputs: &[String]) -> Option<String> {
    inputs
        .iter()
        .find(|i| input_risk(i, output) == Some(InputRisk::FeedbackLoop))
        .cloned()
}

/// 判断输入设备能否当麦克风用（基于设备名，适用于各家虚拟音频线与各平台命名）。
/// Decide whether an input device may be used as the mic (name-based; covers
/// virtual cables from different vendors and each platform's naming).
///
/// 规则 Rules：
/// 1. 名称含回录关键词（立体声混音、`Monitor of …`）→ [`InputRisk::SystemLoopback`]。
///    Name has a loopback hint (Stereo Mix, `Monitor of …`) → `SystemLoopback`.
/// 2. 与输出同名且是虚拟设备（如 VAC 的 `Line 1 (Virtual Audio Cable)`、macOS 的
///    `BlackHole 2ch`）→ [`InputRisk::FeedbackLoop`]。蓝牙耳机的麦克风与听筒也同名，
///    但不构成反馈环，所以要求是虚拟设备。
///    Same name as the output and virtual (VAC's `Line 1 (Virtual Audio Cable)`, macOS
///    `BlackHole 2ch`) → `FeedbackLoop`. Bluetooth headsets share one name for mic and
///    earpiece without looping, hence the virtual requirement.
/// 3. 括号内驱动名相同且是虚拟设备（如 `CABLE Input/Output (VB-Audio Virtual Cable)`、
///    `VoiceMeeter Input/Output (VB-Audio VoiceMeeter VAIO)`）→ `FeedbackLoop`。
///    Same driver name in parentheses and it's a virtual device → `FeedbackLoop`.
/// 4. 无括号时，去掉方向词后同名且是虚拟设备 → `FeedbackLoop`。
///    Without parentheses: same name after removing direction words, and virtual →
///    `FeedbackLoop`.
///
/// 名称启发式无法覆盖所有设备；命名特殊的虚拟线可能漏判。
/// Name heuristics can't cover every device; unusually named cables may slip through.
pub fn input_risk(input: &str, output: &str) -> Option<InputRisk> {
    let inp = input.trim().to_lowercase();
    let out = output.trim().to_lowercase();
    let is_virtual = |s: &str| VIRTUAL_DEVICE_HINTS.iter().any(|h| s.contains(h));

    if LOOPBACK_INPUT_HINTS.iter().any(|h| inp.contains(h)) {
        return Some(InputRisk::SystemLoopback);
    }
    if inp == out && is_virtual(&inp) {
        return Some(InputRisk::FeedbackLoop);
    }
    let same_device = match (driver_part(&inp), driver_part(&out)) {
        (Some(a), Some(b)) => a == b && is_virtual(a),
        (None, None) => {
            let (a, b) = (strip_direction(&inp), strip_direction(&out));
            !a.is_empty() && a == b && is_virtual(&a)
        }
        _ => false,
    };
    same_device.then_some(InputRisk::FeedbackLoop)
}

/// 取设备名中第一个 ` (` 之后的驱动名部分（Windows 命名格式「端点 (驱动)」）。
/// The driver part after the first ` (` (Windows naming: "endpoint (driver)").
fn driver_part(name: &str) -> Option<&str> {
    name.find(" (").map(|i| &name[i + 1..])
}

/// 去掉方向词后的名称（按空白分词）。Name with direction words removed (by whitespace).
fn strip_direction(name: &str) -> String {
    name.split_whitespace()
        .filter(|w| !DIRECTION_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 输入回调侧：下混为单声道，线性插值重采样到输出采样率，写入环。
/// Input-callback side: downmix to mono, linearly resample to the output rate, write
/// to the ring.
///
/// 蓝牙耳机通话模式常为 8/16 kHz 单声道，同样经此换算到输出采样率。
/// Bluetooth headsets in hands-free mode are often 8/16 kHz mono; they go through
/// the same conversion.
pub(crate) struct MicCapture {
    prod: HeapProd<f32>,
    in_ch: usize,
    /// 每个输出样点前进的输入帧数 = 输入采样率 / 输出采样率。
    /// Input frames advanced per output sample = input rate / output rate.
    step: f64,
    /// 下一个输出样点在 [prev, cur) 之间的位置。Next output position within [prev, cur).
    pos: f64,
    prev: f32,
}

impl MicCapture {
    pub(crate) fn new(prod: HeapProd<f32>, in_ch: usize, in_rate: u32, out_rate: u32) -> Self {
        Self {
            prod,
            in_ch: in_ch.max(1),
            step: in_rate as f64 / out_rate.max(1) as f64,
            pos: 0.0,
            prev: 0.0,
        }
    }

    pub(crate) fn push(&mut self, data: &[f32]) {
        for frame in data.chunks_exact(self.in_ch) {
            let cur = frame.iter().sum::<f32>() / self.in_ch as f32;
            while self.pos < 1.0 {
                let v = self.prev + (cur - self.prev) * self.pos as f32;
                // 环满说明输出侧停滞，丢弃即可。A full ring means output stalled; drop.
                let _ = self.prod.try_push(v);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = cur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Consumer as _, Observer as _};

    /// 断言（输入, 输出）组合判定为反馈环。Assert (input, output) is a feedback loop.
    fn assert_loop(input: &str, output: &str) {
        assert_eq!(
            input_risk(input, output),
            Some(InputRisk::FeedbackLoop),
            "{input} ← {output}"
        );
    }

    /// 断言（输入, 输出）组合可以使用。Assert (input, output) is allowed.
    fn assert_ok(input: &str, output: &str) {
        assert_eq!(input_risk(input, output), None, "{input} ← {output}");
    }

    #[test]
    fn detects_feedback_loop_across_vendors() {
        // VB-Audio Virtual Cable（含 16ch 端点）。VB-Audio Virtual Cable (incl. 16ch).
        let cable_out = "CABLE Output (VB-Audio Virtual Cable)";
        assert_loop(cable_out, "CABLE Input (VB-Audio Virtual Cable)");
        assert_loop(cable_out, "CABLE In 16ch (VB-Audio Virtual Cable)");
        // VB-CABLE A/B 与 Hi-Fi Cable。VB-CABLE A/B and Hi-Fi Cable.
        assert_loop(
            "CABLE-A Output (VB-Audio Cable A)",
            "CABLE-A Input (VB-Audio Cable A)",
        );
        assert_loop(
            "Hi-Fi Cable Output (VB-Audio Hi-Fi Cable)",
            "Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)",
        );
        // VoiceMeeter。VoiceMeeter.
        assert_loop(
            "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)",
            "VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)",
        );
        // Virtual Audio Cable (VAC)：两端同名。VAC: both ends share a name.
        assert_loop(
            "Line 1 (Virtual Audio Cable)",
            "Line 1 (Virtual Audio Cable)",
        );
        // macOS BlackHole：两端同名、无括号。macOS BlackHole: same name, no parens.
        assert_loop("BlackHole 2ch", "BlackHole 2ch");
        // 无括号的虚拟设备，名称只差方向词。No parens, names differ only by direction.
        assert_loop("SoundBridge Virtual Source", "SoundBridge Virtual Sink");
    }

    #[test]
    fn allows_real_mics_and_different_cables() {
        let cable_in = "CABLE Input (VB-Audio Virtual Cable)";
        // 真实麦克风 → 虚拟线。Real mic → virtual cable.
        assert_ok("Microphone (WO Mic Device)", cable_in);
        assert_ok("Headset (Bluetooth Hands-Free Audio)", cable_in);
        // 蓝牙耳机的麦克风与听筒同名，不是反馈环。
        // A Bluetooth headset's mic and earpiece share a name but don't loop.
        assert_ok("耳机 (AirPods - Find My)", "耳机 (AirPods - Find My)");
        // 同一块声卡的麦克风与扬声器不是反馈环。A card's own mic + speakers isn't a loop.
        assert_ok(
            "Microphone (Realtek(R) Audio)",
            "Speakers (Realtek(R) Audio)",
        );
        // macOS 内建设备去掉方向词后同名，但不是虚拟设备。
        // macOS built-ins match after stripping direction, but aren't virtual.
        assert_ok("Built-in Input", "Built-in Output");
        // 不同的两条虚拟线。Two different virtual cables.
        assert_ok(
            "CABLE-A Output (VB-Audio Cable A)",
            "CABLE-B Input (VB-Audio Cable B)",
        );
        // 虚拟线当麦克风、输出到扬声器：允许。Cable as mic, speakers as output: fine.
        assert_ok(
            "CABLE Output (VB-Audio Virtual Cable)",
            "LG ULTRAGEAR (NVIDIA High Definition Audio)",
        );
    }

    #[test]
    fn finds_game_mic_and_virtual_devices() {
        let inputs = vec![
            "Microphone (WO Mic Device)".to_string(),
            "CABLE Output (VB-Audio Virtual Cable)".to_string(),
        ];
        // 输出到虚拟线：游戏里选另一端。Output to a cable: the game picks the other end.
        assert_eq!(
            game_mic_for("CABLE Input (VB-Audio Virtual Cable)", &inputs).as_deref(),
            Some("CABLE Output (VB-Audio Virtual Cable)")
        );
        // 输出到扬声器：游戏收不到。Output to speakers: nothing for the game.
        assert_eq!(
            game_mic_for("LG ULTRAGEAR (NVIDIA High Definition Audio)", &inputs),
            None
        );
        // 硬件麦克风 vs 虚拟设备。Hardware mics vs virtual devices.
        assert!(is_virtual_device("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(!is_virtual_device("Microphone (WO Mic Device)"));
        assert!(!is_virtual_device("Headset (Bluetooth Hands-Free Audio)"));
        assert!(!is_virtual_device("Microphone (Realtek(R) Audio)"));
    }

    #[test]
    fn rejects_system_loopback_inputs() {
        for input in [
            "Stereo Mix (Realtek(R) Audio)",
            "立体声混音 (Realtek(R) Audio)",
            "What U Hear (Sound Blaster)",
            "Monitor of Built-in Audio Analog Stereo",
        ] {
            assert_eq!(
                input_risk(input, "CABLE Input (VB-Audio Virtual Cable)"),
                Some(InputRisk::SystemLoopback),
                "{input}"
            );
        }
    }

    #[test]
    fn capture_downmixes_stereo_to_mono() {
        let (prod, mut cons) = HeapRb::<f32>::new(64).split();
        let mut cap = MicCapture::new(prod, 2, 48_000, 48_000);
        cap.push(&[1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
        let mut out = [0.0f32; 8];
        let n = cons.pop_slice(&mut out);
        // 同采样率：每输入一帧输出一个样点（滞后一帧）。
        // Same rate: one output sample per input frame (one frame behind).
        assert_eq!(n, 3);
        assert_eq!(&out[..3], &[0.0, 0.5, 0.5]);
    }

    #[test]
    fn capture_resamples_44k_to_48k() {
        let (prod, cons) = HeapRb::<f32>::new(100_000).split();
        let mut cap = MicCapture::new(prod, 1, 44_100, 48_000);
        cap.push(&vec![0.25f32; 44_100]);
        // 1 秒输入 ≈ 1 秒输出（48000 个样点）。1 s in ≈ 1 s out (48000 samples).
        let n = cons.occupied_len();
        assert!((47_990..=48_010).contains(&n), "n={n}");
    }

    #[test]
    fn capture_resamples_bluetooth_16k_to_48k() {
        let (prod, cons) = HeapRb::<f32>::new(100_000).split();
        let mut cap = MicCapture::new(prod, 1, 16_000, 48_000);
        cap.push(&vec![0.25f32; 16_000]);
        // 蓝牙通话模式 16 kHz：1 秒输入 ≈ 48000 个输出样点。
        // Bluetooth hands-free 16 kHz: 1 s in ≈ 48000 output samples.
        let n = cons.occupied_len();
        assert!((47_990..=48_010).contains(&n), "n={n}");
    }
}
