//! 试听（监听）：把音效同时放一份到耳机 / 扬声器，让自己也能听到。
//! Monitoring: also play the sound effects on headphones / speakers so you can hear
//! them yourself.
//!
//! 主输出（发给游戏的虚拟音频线）在实时回调里混好音效后，先把「只有音效」的那份
//! 写进一个 SPSC 环（[`MonitorTap`]），再叠加麦克风。试听线程持有第二个输出流，
//! 回调（[`MonitorReader`]）从环里读出、按采样率比例插值、乘试听音量后播放。
//! 只放音效、不放自己的麦克风：听到自己说话的延迟回声会很难受。
//! The main output (the virtual cable sent to games) mixes the effects in its real-time
//! callback, writes the effects-only mix into an SPSC ring ([`MonitorTap`]), then adds
//! the mic. The monitor thread owns a second output stream whose callback
//! ([`MonitorReader`]) reads the ring, resamples by the rate ratio, applies the monitor
//! volume and plays it. Effects only, never your own mic: a delayed echo of your voice
//! is unpleasant.
//!
//! 与麦克风相同的约束：回调不分配、不加锁；换设备时新环经通道交给对方，旧环送回控制侧释放。
//! Same constraints as the mic: callbacks don't allocate or lock; on a device change
//! the new ring is handed over through a channel and the old one is freed on the control
//! side.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio_cpal::CpalBackend;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use platform_api::OutputStreamHandle;
use ringbuf::traits::{Consumer as _, Observer as _, Producer as _, Split as _};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;
use soundboard::db_to_linear;

use crate::audio::OutputKind;
use crate::settings::SettingsStore;

/// 环中传递的声道数（立体声）。Channels carried by the ring (stereo).
const RING_CH: usize = 2;
/// 抖动缓冲目标（毫秒）。Jitter-buffer target (ms).
const TARGET_MS: u32 = 20;
/// 超过目标这么多（毫秒）时丢弃积压。Excess (ms) beyond the target at which backlog is dropped.
const MAX_EXCESS_MS: u32 = 60;

/// 试听参数与状态（控制侧 ↔ 实时侧共享）。Monitor parameters and status.
pub struct MonitorControl {
    gain_db: AtomicU32,
    status: Mutex<MonitorStatus>,
}

/// 试听状态（供界面显示）。Monitor status (for the UI).
#[derive(Debug, Clone, Default, Serialize)]
pub struct MonitorStatus {
    /// 设置里是否开启。Enabled in settings.
    pub enabled: bool,
    /// 用户选择的设备；`None` = 系统默认播放设备。Chosen device; `None` = default playback.
    pub selection: Option<String>,
    /// 正在使用的设备。Device in use.
    pub device: Option<String>,
    /// 是否正在试听。Whether monitoring is active.
    pub active: bool,
    /// 未生效的原因。Why it isn't active.
    pub note: Option<String>,
    pub gain_db: f32,
}

impl MonitorControl {
    pub fn new(gain_db: f32) -> Self {
        Self {
            gain_db: AtomicU32::new(gain_db.to_bits()),
            status: Mutex::new(MonitorStatus::default()),
        }
    }

    pub fn gain_db(&self) -> f32 {
        f32::from_bits(self.gain_db.load(Ordering::Relaxed))
    }

    pub fn set_gain_db(&self, db: f32) {
        self.gain_db.store(db.to_bits(), Ordering::Relaxed);
    }

    pub fn status(&self) -> MonitorStatus {
        let mut s = self.status.lock().unwrap().clone();
        s.gain_db = self.gain_db();
        s
    }

    fn update(&self, f: impl FnOnce(&mut MonitorStatus)) {
        f(&mut self.status.lock().unwrap());
    }
}

/// 控制侧 → 试听线程的请求。Request from the control side to the monitor thread.
pub enum MonitorRequest {
    /// 开关 / 换设备（`None` = 系统默认）。结果经 `reply` 返回。
    /// Enable/disable or change device (`None` = default). Result via `reply`.
    Configure {
        enabled: bool,
        device: Option<String>,
        reply: Sender<Result<(), String>>,
    },
    /// 主输出变了：按新的采样率 / 设备重新打开（并检查是否与主输出相同）。
    /// The main output changed: reopen for the new rate / device (and check it differs).
    MainChanged {
        name: String,
        rate: u32,
        kind: OutputKind,
    },
}

/// 主输出回调一侧的端点（可克隆：重建主输出时新回调拿一份）。
/// Endpoints on the main-output side (cloneable for a rebuilt main callback).
#[derive(Clone)]
pub struct MonitorLink {
    prod_rx: Receiver<HeapProd<f32>>,
    retired_tx: Sender<HeapProd<f32>>,
}

/// 试听线程一侧的端点。The monitor thread's endpoints.
pub struct MonitorEnds {
    prod_tx: Sender<HeapProd<f32>>,
    retired_rx: Receiver<HeapProd<f32>>,
}

pub fn link_channels() -> (MonitorEnds, MonitorLink) {
    let (prod_tx, prod_rx) = bounded(4);
    let (retired_tx, retired_rx) = bounded(4);
    (
        MonitorEnds {
            prod_tx,
            retired_rx,
        },
        MonitorLink {
            prod_rx,
            retired_tx,
        },
    )
}

/// 主输出回调里的「分流」：把只有音效的混音写进试听环。
/// The tap in the main output callback: writes the effects-only mix into the monitor ring.
pub struct MonitorTap {
    link: MonitorLink,
    prod: Option<HeapProd<f32>>,
}

impl MonitorTap {
    pub fn new(link: MonitorLink) -> Self {
        Self { link, prod: None }
    }

    /// 实时侧：写入本次回调的音效混音（`data` 为 `channels` 声道交错）。
    /// Real-time side: write this callback's effects mix (`data` interleaved).
    pub fn tap(&mut self, data: &[f32], frames: usize, channels: usize) {
        // 换了试听设备：换上新环，旧环送回。New monitor device: swap rings.
        while let Ok(new) = self.link.prod_rx.try_recv() {
            if let Some(old) = self.prod.replace(new) {
                let _ = self.link.retired_tx.try_send(old);
            }
        }
        let Some(prod) = self.prod.as_mut() else {
            return;
        };
        // 空间不够就整块跳过（试听侧会重新缓冲）。Skip the block if there's no room.
        if prod.vacant_len() < frames * RING_CH || channels == 0 {
            return;
        }
        for f in 0..frames {
            let l = data[f * channels];
            let r = if channels > 1 {
                data[f * channels + 1]
            } else {
                l
            };
            let _ = prod.try_push(l);
            let _ = prod.try_push(r);
        }
    }
}

/// 试听回调：从环读立体声帧，按 `step = 主输出采样率 / 试听采样率` 线性插值。
/// Monitor callback: reads stereo frames, linearly interpolated with
/// `step = main rate / monitor rate`.
struct MonitorReader {
    cons: HeapCons<f32>,
    step: f64,
    pos: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    primed: bool,
    target: usize,
    max_excess: usize,
}

impl MonitorReader {
    fn new(cons: HeapCons<f32>, main_rate: u32, out_rate: u32) -> Self {
        Self {
            cons,
            step: main_rate as f64 / out_rate.max(1) as f64,
            pos: 1.0,
            prev: [0.0; 2],
            cur: [0.0; 2],
            primed: false,
            target: (main_rate * TARGET_MS / 1000) as usize,
            max_excess: (main_rate * MAX_EXCESS_MS / 1000) as usize,
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize, gain: f32) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        let frames = out.len() / channels.max(1);
        let avail = self.cons.occupied_len() / RING_CH;
        if avail > self.target + self.max_excess + frames {
            // 积压过多（时钟漂移）：丢到目标水位。Too much backlog: drop to target.
            self.cons.skip((avail - self.target) * RING_CH);
        }
        if !self.primed {
            if self.cons.occupied_len() / RING_CH < self.target + frames {
                return;
            }
            // 用第一帧同时作为前后两帧，避免开头插值到 0。
            // Seed both neighbours with the first frame so the start doesn't fade from 0.
            let mut first = [0.0f32; 2];
            self.cons.pop_slice(&mut first);
            self.prev = first;
            self.cur = first;
            self.pos = 0.0;
            self.primed = true;
        }
        for f in 0..frames {
            while self.pos >= 1.0 {
                let mut frame = [0.0f32; 2];
                if self.cons.pop_slice(&mut frame) < RING_CH {
                    // 欠载：静音并重新缓冲。Underrun: silence and re-buffer.
                    self.primed = false;
                    return;
                }
                self.prev = self.cur;
                self.cur = frame;
                self.pos -= 1.0;
            }
            let t = self.pos as f32;
            let l = (self.prev[0] + (self.cur[0] - self.prev[0]) * t) * gain;
            let r = (self.prev[1] + (self.cur[1] - self.prev[1]) * t) * gain;
            for c in 0..channels {
                out[f * channels + c] = if c % 2 == 0 { l } else { r }.clamp(-1.0, 1.0);
            }
            self.pos += self.step;
        }
    }
}

/// 启动试听线程。Start the monitor thread.
pub fn spawn_monitor_thread(
    ends: MonitorEnds,
    control: Arc<MonitorControl>,
    settings: Arc<SettingsStore>,
    main: (String, u32, OutputKind),
) -> Sender<MonitorRequest> {
    let (tx, rx) = bounded::<MonitorRequest>(8);
    let saved = settings.get();
    std::thread::Builder::new()
        .name("soundbridge-monitor".into())
        .spawn(move || {
            let mut t = MonitorThread {
                backend: CpalBackend::new(),
                ends,
                control,
                stream: None,
                enabled: saved.monitor_enabled,
                selection: saved.monitor_device,
                main_name: main.0,
                main_rate: main.1,
                main_kind: main.2,
            };
            let _ = t.apply(); // 原因已记在状态里。The reason is kept in the status.
            loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(MonitorRequest::Configure {
                        enabled,
                        device,
                        reply,
                    }) => {
                        t.enabled = enabled;
                        t.selection = device.clone();
                        let result = t.apply();
                        settings.update(|s| {
                            s.monitor_enabled = enabled;
                            s.monitor_device = device;
                        });
                        let _ = reply.send(result);
                    }
                    Ok(MonitorRequest::MainChanged { name, rate, kind }) => {
                        t.main_name = name;
                        t.main_rate = rate;
                        t.main_kind = kind;
                        let _ = t.apply();
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                while t.ends.retired_rx.try_recv().is_ok() {}
            }
        })
        .expect("spawn monitor thread");
    tx
}

struct MonitorThread {
    backend: CpalBackend,
    ends: MonitorEnds,
    control: Arc<MonitorControl>,
    stream: Option<Box<dyn OutputStreamHandle>>,
    enabled: bool,
    selection: Option<String>,
    main_name: String,
    main_rate: u32,
    main_kind: OutputKind,
}

impl MonitorThread {
    /// 按当前设置（重新）打开试听流；不该试听时关闭。返回无法开启的原因。
    /// (Re)open the monitor stream per the current settings, or close it. Returns why
    /// it couldn't be enabled.
    fn apply(&mut self) -> Result<(), String> {
        self.stream = None;
        let enabled = self.enabled;
        let selection = self.selection.clone();
        self.control.update(|s| {
            s.enabled = enabled;
            s.selection = selection;
            s.active = false;
            s.device = None;
            s.note = None;
        });
        if !self.enabled {
            return Ok(());
        }
        let result = self.open();
        if let Err(e) = &result {
            log::info!("试听未开启 monitor inactive: {e}");
            let note = e.clone();
            self.control.update(|s| s.note = Some(note));
        }
        result
    }

    fn open(&mut self) -> Result<(), String> {
        if self.main_kind == OutputKind::Speaker {
            return Err(tr!(
                "输出已经是扬声器 / 耳机，不需要试听",
                "The output is already speakers / headphones; no monitor needed"
            )
            .into());
        }
        let dev = self
            .backend
            .resolve_output(self.selection.as_deref())
            .map_err(|e| trf!("找不到试听设备：{e}", "Monitor device not found: {e}"))?;
        let name = dev.name().to_string();
        if name == self.main_name {
            return Err(tr!(
                "试听设备与输出设备相同，已跳过（否则会重复播放）",
                "The monitor device is the output device; skipped (it would play twice)"
            )
            .into());
        }
        let cfg = dev.config();
        let out_rate = cfg.sample_rate.0;
        let out_ch = cfg.channels.0 as usize;
        // 环：主输出采样率的立体声，0.5 秒。Ring: stereo at the main rate, 0.5 s.
        let (prod, cons) = HeapRb::<f32>::new((self.main_rate as usize / 2) * RING_CH).split();
        let mut reader = MonitorReader::new(cons, self.main_rate, out_rate);
        let control = self.control.clone();
        let stream = self
            .backend
            .build_output(dev, 512, move |data: &mut [f32]| {
                reader.render(data, out_ch, db_to_linear(control.gain_db()));
            })
            .map_err(|e| {
                trf!(
                    "无法打开试听设备「{name}」：{e}",
                    "Can't open monitor device \"{name}\": {e}"
                )
            })?;
        self.ends
            .prod_tx
            .try_send(prod)
            .map_err(|_| tr!("音频线程未响应", "The audio thread is not responding").to_string())?;
        self.stream = Some(stream);
        let device = name.clone();
        self.control.update(|s| {
            s.active = true;
            s.device = Some(device);
        });
        log::info!("试听 monitor: {name} ({out_rate} Hz)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_writes_stereo_and_reader_plays_it_back() {
        let (ends, link) = link_channels();
        let mut tap = MonitorTap::new(link);
        let (prod, cons) = HeapRb::<f32>::new(48_000).split();
        ends.prod_tx.send(prod).unwrap();

        // 主输出单声道 0.5：试听收到 L=R=0.5。Main mono 0.5 → monitor gets L=R=0.5.
        let main = vec![0.5f32; 2000];
        tap.tap(&main, 2000, 1);
        let mut reader = MonitorReader::new(cons, 48_000, 48_000);
        let mut out = vec![0.0f32; 256 * 2];
        reader.render(&mut out, 2, 1.0);
        assert!(
            out.iter().all(|&v| (v - 0.5).abs() < 1e-6),
            "{:?}",
            &out[..4]
        );

        // 音量 -6 dB 生效。Gain applies.
        reader.render(&mut out, 2, 0.5);
        assert!((out[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn reader_resamples_to_device_rate() {
        let (prod, cons) = HeapRb::<f32>::new(200_000).split();
        let mut prod = prod;
        // 0.125 秒 48 kHz 立体声（不超过积压上限）。0.125 s of 48 kHz stereo (under the cap).
        prod.push_slice(&vec![0.1f32; 6000 * 2]);
        let mut reader = MonitorReader::new(cons, 48_000, 44_100);
        let mut out = vec![0.0f32; 4410 * 2];
        reader.render(&mut out, 2, 1.0);
        // 44.1 kHz 设备取 0.1 秒约消耗 4800 帧 48 kHz 数据。
        // 0.1 s on a 44.1 kHz device consumes ≈ 4800 frames of 48 kHz data.
        assert!(out.iter().all(|&v| (v - 0.1).abs() < 1e-6));
        let left = reader.cons.occupied_len() / RING_CH;
        assert!((1150..=1250).contains(&left), "left={left}");
    }
}
