//! 音频输出：SoundBoard 混音 + 叠加麦克风 + cpal 输出。
//! Audio output: SoundBoard mix + microphone + cpal output.
//!
//! 线程拓扑：
//! Thread topology:
//! - 麦克风线程（见 [`crate::mic`]）：持有输入流，负责切换设备、断开重连。
//!   Mic thread (see [`crate::mic`]): owns the input stream, switches devices,
//!   reconnects.
//! - cpal 输入回调线程（实时）：麦克风 → 单声道 → 重采样 → 写环。
//!   cpal input callback thread (real-time): mic → mono → resample → write ring.
//! - cpal 输出回调线程（实时）：消费命令(≤32) → SoundBoard::fill → 叠加麦克风 → 写输出。
//!   cpal output callback thread (real-time): consume ≤32 commands →
//!   SoundBoard::fill → add mic → write output.
//!
//! 实时回调不分配、不加锁。麦克风参数（增益/静音）与电平表经原子量与控制侧共享；
//! 换麦克风时新环的消费端经无锁通道交给输出回调。
//! Real-time callbacks neither allocate nor lock. Mic parameters (gain/mute) and the
//! level meter are shared through atomics; on a mic switch the new ring's consumer
//! reaches the output callback over a lock-free channel.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use audio_cpal::CpalBackend;
use crossbeam_channel::{bounded, Receiver, Sender};
use platform_api::OutputStreamHandle;
use ringbuf::traits::{Consumer as _, Observer as _};
use ringbuf::HeapCons;
use soundboard::{db_to_linear, BoardCommand, SoundBoard, SoundId, MAX_INSTANCES};

use crate::loader::Player;
use crate::mic::{
    game_mic_for, is_virtual_device, link_channels, spawn_mic_thread, MicControl, MicLink,
    MicRequest,
};
use crate::monitor::{self, MonitorControl, MonitorLink, MonitorRequest, MonitorTap};
use crate::settings::SettingsStore;

/// 输出回调期望的缓冲帧数（设备可能给出不同长度）。
/// Requested output buffer frames (the device may deliver other lengths).
const OUTPUT_BUFFER_FRAMES: usize = 512;

/// 麦克风抖动缓冲目标（毫秒）：环中保持约这么多数据以吸收两个设备的回调节奏差。
/// Mic jitter-buffer target (ms): the ring holds about this much to absorb the
/// cadence difference between the two devices' callbacks.
const MIC_TARGET_MS: u32 = 15;

/// 环中数据超过目标这么多（毫秒）时丢弃多余部分，防止时钟漂移让延迟越积越大。
/// Excess (ms) beyond the target at which surplus is dropped, so clock drift can't
/// grow the latency unbounded.
const MIC_MAX_EXCESS_MS: u32 = 60;

/// 正在播放的音效及进度（实时侧每次回调写入，控制侧读取给 UI 高亮和画进度条）。
/// Playing sounds and their progress (written by the real-time side every callback,
/// read by the control side for UI highlighting and progress bars).
///
/// 固定 `MAX_INSTANCES` 个原子槽位：id（0 表示空，音效 id 从 1 开始）+ 进度（f32 位）。
/// 无锁、不分配。两个原子量分开写，读到的 id 与进度可能差一个回调，对 UI 无影响。
/// `MAX_INSTANCES` fixed atomic slots: id (0 = empty; ids start at 1) + progress (f32
/// bits). Lock- and allocation-free. The two atomics are written separately, so a
/// read may mix two callbacks — harmless for the UI.
pub struct PlayingSet {
    ids: [AtomicU32; MAX_INSTANCES],
    progress: [AtomicU32; MAX_INSTANCES],
}

impl PlayingSet {
    pub fn new() -> Self {
        Self {
            ids: std::array::from_fn(|_| AtomicU32::new(0)),
            progress: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// 实时侧：写入本次回调后的播放实例。Real-time side: publish this callback's instances.
    fn publish(&self, board: &SoundBoard) {
        let mut n = 0;
        board.for_each_playing(|id, progress| {
            if n < MAX_INSTANCES {
                self.ids[n].store(id, Ordering::Relaxed);
                self.progress[n].store(progress.to_bits(), Ordering::Relaxed);
                n += 1;
            }
        });
        for slot in &self.ids[n..] {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// 控制侧：每个播放中的音效一项（同一音效多个实例时取最新的进度），按 id 排序。
    /// Control side: one entry per playing sound (the newest instance's progress when
    /// a sound has several), sorted by id.
    pub fn snapshot(&self) -> Vec<(SoundId, f32)> {
        let mut out: Vec<(SoundId, f32)> = Vec::new();
        for (id, progress) in self.ids.iter().zip(&self.progress) {
            let id = id.load(Ordering::Relaxed);
            if id == 0 {
                continue;
            }
            let p = f32::from_bits(progress.load(Ordering::Relaxed));
            // 槽位按启动先后排列，后写的覆盖前面的 = 取最新实例。
            // Slots are in start order; later ones overwrite = newest instance wins.
            match out.iter_mut().find(|(i, _)| *i == id) {
                Some(entry) => entry.1 = p,
                None => out.push((id, p)),
            }
        }
        out.sort_unstable_by_key(|(id, _)| *id);
        out
    }
}

impl Default for PlayingSet {
    fn default() -> Self {
        Self::new()
    }
}

/// 输出设备的类型：虚拟音频线（游戏能收到）还是扬声器 / 耳机（只能自己试听）。
/// Output kind: a virtual cable (games receive it) or a speaker / headphones (local
/// preview only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    Virtual,
    Speaker,
}

/// 输出设备信息（供 API 展示）。Output device info (for the API).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioInfo {
    pub output_device: String,
    pub output_rate: u32,
    pub output_channels: u16,
    pub output_kind: OutputKind,
    /// 游戏 / 语音软件里该选的麦克风（虚拟线的另一端）；扬声器时为 `None`。
    /// The mic games should pick (the cable's other end); `None` for speakers.
    pub game_mic: Option<String>,
}

/// 按输出设备名与当前输入设备列表推算类型与「游戏里该选的麦克风」。
/// Derive the kind and "mic to pick in games" from the output name and current inputs.
pub fn describe_output(name: &str, inputs: &[String]) -> (OutputKind, Option<String>) {
    let game_mic = game_mic_for(name, inputs);
    let kind = if game_mic.is_some() || is_virtual_device(name) {
        OutputKind::Virtual
    } else {
        OutputKind::Speaker
    };
    (kind, game_mic)
}

/// 控制侧 → 输出线程的请求。Request from the control side to the output thread.
pub enum OutputRequest {
    /// 切换输出设备（`None` = 系统默认）。失败时恢复原设备并返回错误。
    /// Switch the output device (`None` = system default). On failure the previous
    /// device is restored and an error returned.
    Select {
        device: Option<String>,
        reply: Sender<Result<AudioInfo, String>>,
    },
}

/// 音频运行时：设备信息、输出与麦克风请求通道、播放状态。输出流由输出线程持有。
/// Audio runtime: device info, output and mic request channels, playing state. The
/// output stream itself is owned by the output thread.
pub struct AudioRuntime {
    /// 当前输出设备（切换后更新）。Current output device (updated on switches).
    pub info: Arc<Mutex<AudioInfo>>,
    pub mic_tx: Sender<MicRequest>,
    pub out_tx: Sender<OutputRequest>,
    /// 试听请求发送端（→ 试听线程）。Monitor request sender (→ monitor thread).
    pub monitor_tx: Sender<MonitorRequest>,
    /// 正在播放的音效。Playing sounds.
    pub playing: Arc<PlayingSet>,
}

/// 启动音频：输出线程打开输出设备，再启动麦克风线程，开始混音。
/// Start audio: the output thread opens the output device, then the mic thread starts
/// and mixing begins.
///
/// `output_device`：输出设备名（包含匹配），`None` 用系统默认。`initial_mic`：初始
/// 麦克风选择（`None` = 自动）。麦克风不可用不致命：只输出音效，麦克风线程会持续等待。
/// `output_device`: output device name (containment match), `None` uses the system
/// default. `initial_mic`: initial mic selection (`None` = auto). A missing mic is not
/// fatal: sounds still play and the mic thread keeps waiting for a device.
pub fn start_audio(
    cmd_rx: Receiver<BoardCommand>,
    output_device: Option<String>,
    initial_mic: Option<String>,
    mic: Arc<MicControl>,
    monitor: Arc<MonitorControl>,
    settings: Arc<SettingsStore>,
    player: Arc<Player>,
) -> anyhow::Result<AudioRuntime> {
    let playing = Arc::new(PlayingSet::new());
    let (mic_ends, link) = link_channels();
    let (monitor_ends, monitor_link) = monitor::link_channels();
    let (out_tx, out_rx) = bounded::<OutputRequest>(4);
    let (ready_tx, ready_rx) = bounded::<Result<AudioInfo, String>>(1);
    let (notify_tx, notify_rx) = bounded::<(Sender<MicRequest>, Sender<MonitorRequest>)>(1);
    let info = Arc::new(Mutex::new(AudioInfo {
        output_device: String::new(),
        output_rate: 0,
        output_channels: 0,
        output_kind: OutputKind::Speaker,
        game_mic: None,
    }));

    let ctx = OutputCtx {
        backend: CpalBackend::new(),
        cmd_rx,
        link,
        monitor_link,
        mic: mic.clone(),
        playing: playing.clone(),
    };
    let thread_info = info.clone();
    let thread_settings = settings.clone();
    // 输出流句柄不能跨线程（非 Send），所以由这个线程创建、持有和替换。
    // Output stream handles aren't Send, so this thread creates, owns and replaces them.
    std::thread::Builder::new()
        .name("soundbridge-output".into())
        .spawn(move || {
            let mut stream = match ctx.open(output_device.as_deref()) {
                Ok((s, i)) => {
                    *thread_info.lock().unwrap() = i.clone();
                    let _ = ready_tx.send(Ok(i));
                    Some(s)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let Ok((mic_tx, monitor_tx)) = notify_rx.recv() else {
                return;
            };
            // 通道关闭（服务结束）时退出，流随之停止。Exit when the channel closes.
            for req in out_rx {
                let OutputRequest::Select { device, reply } = req;
                let previous = thread_info.lock().unwrap().output_device.clone();
                // 先停旧流，再排队重新装入音效，新流一开始就能按顺序收到。
                // Stop the old stream, then queue the sounds so the new one gets them in order.
                stream = None;
                player.reinstall_all();
                let result = match ctx.open(device.as_deref()) {
                    Ok((s, i)) => {
                        stream = Some(s);
                        thread_settings.update(|st| st.output_device = device.clone());
                        Ok(i)
                    }
                    Err(e) => {
                        // 恢复原设备（已排队的音效会被它收到）。Restore the previous device.
                        if let Ok((s, _)) = ctx.open(Some(&previous)) {
                            stream = Some(s);
                        }
                        Err(e)
                    }
                };
                if let Ok(i) = &result {
                    *thread_info.lock().unwrap() = i.clone();
                }
                // 新的主输出回调需要新的麦克风环与试听环。The new callback needs new rings.
                let current = thread_info.lock().unwrap().clone();
                let _ = mic_tx.send(MicRequest::OutputChanged {
                    output_name: current.output_device.clone(),
                    out_rate: current.output_rate,
                });
                let _ = monitor_tx.send(MonitorRequest::MainChanged {
                    name: current.output_device,
                    rate: current.output_rate,
                    kind: current.output_kind,
                });
                let _ = reply.send(result);
            }
            drop(stream);
        })
        .expect("spawn output thread");

    let first = ready_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("输出线程启动失败 output thread died"))?
        .map_err(|e| anyhow::anyhow!(e))?;
    let mic_tx = spawn_mic_thread(
        mic_ends,
        initial_mic,
        first.output_device.clone(),
        first.output_rate,
        mic,
        settings.clone(),
    );
    let monitor_tx = monitor::spawn_monitor_thread(
        monitor_ends,
        monitor,
        settings,
        (
            first.output_device.clone(),
            first.output_rate,
            first.output_kind,
        ),
    );
    let _ = notify_tx.send((mic_tx.clone(), monitor_tx.clone()));

    Ok(AudioRuntime {
        info,
        mic_tx,
        out_tx,
        monitor_tx,
        playing,
    })
}

/// 输出线程打开设备所需的上下文。Context the output thread needs to open a device.
struct OutputCtx {
    backend: CpalBackend,
    cmd_rx: Receiver<BoardCommand>,
    link: MicLink,
    monitor_link: MonitorLink,
    mic: Arc<MicControl>,
    playing: Arc<PlayingSet>,
}

impl OutputCtx {
    /// 打开输出设备并开始混音（新的音效板，按新设备的采样率）。
    /// Open an output device and start mixing (a fresh board at the device's rate).
    fn open(
        &self,
        device: Option<&str>,
    ) -> Result<(Box<dyn OutputStreamHandle>, AudioInfo), String> {
        let out = self.backend.resolve_output(device).map_err(|e| {
            let name = device.unwrap_or(tr!("默认", "default"));
            trf!(
                "找不到输出设备「{name}」：{e}",
                "Output device \"{name}\" not found: {e}"
            )
        })?;
        let out_cfg = out.config();
        let out_rate = out_cfg.sample_rate.0;
        let out_ch = out_cfg.channels.0 as usize;
        let output_name = out.name().to_string();

        let mut board = SoundBoard::new(4096, out_rate);
        let mut mic_reader = MicReader::new(self.link.clone(), out_rate);
        let mut monitor_tap = MonitorTap::new(self.monitor_link.clone());
        let cmd_rx = self.cmd_rx.clone();
        let playing = self.playing.clone();
        let mic = self.mic.clone();

        // 回调：消费命令 + 混音音效 + 叠加麦克风 + 限幅。
        // Callback: consume commands + mix sounds + add mic + clip.
        let callback = move |data: &mut [f32]| {
            // 1. 有界消费命令（≤32 条）。1. Bounded command consumption (≤32).
            for _ in 0..32 {
                match cmd_rx.try_recv() {
                    Ok(cmd) => board.apply_command(cmd),
                    Err(_) => break, // 队列空。Queue empty.
                }
            }

            // 2. 混音所有音效实例。2. Mix all sound instances.
            let frames = data.len() / out_ch;
            board.fill(data, frames, out_ch);
            playing.publish(&board);

            // 3. 只有音效的混音分一份给试听（在加麦克风之前）。
            // 3. Send the effects-only mix to the monitor (before adding the mic).
            monitor_tap.tap(data, frames, out_ch);

            // 4. 叠加麦克风。4. Add the microphone.
            mic_reader.mix_into(data, frames, out_ch, &mic);

            // 5. 硬限幅，避免叠加后超出满幅。5. Hard clip so the sum can't exceed full scale.
            for s in data.iter_mut() {
                *s = s.clamp(-1.0, 1.0);
            }
        };

        let stream = self
            .backend
            .build_output(out, OUTPUT_BUFFER_FRAMES, callback)
            .map_err(|e| {
                trf!(
                    "无法打开输出设备「{output_name}」：{e}",
                    "Can't open output device \"{output_name}\": {e}"
                )
            })?;
        let (output_kind, game_mic) =
            describe_output(&output_name, &self.backend.input_device_names());
        log::info!(
            "输出设备 output: {output_name} ({out_rate} Hz, {out_ch} ch, {output_kind:?}, game mic: {game_mic:?})"
        );
        Ok((
            stream,
            AudioInfo {
                output_device: output_name,
                output_rate: out_rate,
                output_channels: out_ch as u16,
                output_kind,
                game_mic,
            },
        ))
    }
}

/// 输出回调侧：从环读麦克风，带抖动缓冲与漂移裁剪，叠加到输出。
/// Output-callback side: read the mic from the ring with a jitter buffer and drift
/// trimming, and add it to the output.
struct MicReader {
    link: MicLink,
    /// 当前环的消费端；`None` 表示还没有麦克风。Current ring consumer; `None` = no mic yet.
    cons: Option<HeapCons<f32>>,
    /// 预分配读缓冲。Pre-allocated read buffer.
    buf: Vec<f32>,
    /// 是否已缓冲到目标水位（欠载或换设备后重新缓冲）。
    /// Whether buffered to target (re-buffers after an underrun or device switch).
    primed: bool,
    target: usize,
    max_excess: usize,
}

impl MicReader {
    fn new(link: MicLink, out_rate: u32) -> Self {
        Self {
            link,
            cons: None,
            buf: vec![0.0; 2048],
            primed: false,
            target: (out_rate * MIC_TARGET_MS / 1000) as usize,
            max_excess: (out_rate * MIC_MAX_EXCESS_MS / 1000) as usize,
        }
    }

    fn mix_into(&mut self, out: &mut [f32], frames: usize, channels: usize, mic: &MicControl) {
        // 换了麦克风：换上新环，旧环送回麦克风线程释放（通道满时才在此释放）。
        // Mic switched: take the new ring and send the old one back to the mic thread
        // to be freed (freed here only if that channel is full).
        while let Ok(new) = self.link.cons_rx.try_recv() {
            if let Some(old) = self.cons.replace(new) {
                let _ = self.link.retired_tx.try_send(old);
            }
            self.primed = false;
        }
        let Some(cons) = self.cons.as_mut() else {
            mic.update_level(0.0);
            return;
        };

        let occupied = cons.occupied_len();

        // 数据积压（输入时钟偏快）：丢弃多余，回到目标水位。
        // Backlog (input clock running fast): drop the surplus back to target.
        if occupied > frames + self.target + self.max_excess {
            cons.skip(occupied - frames - self.target);
        }

        // 未缓冲够：本次不出麦克风声，等待积累。Not buffered yet: stay silent, accumulate.
        if !self.primed {
            if cons.occupied_len() < frames + self.target {
                mic.update_level(0.0);
                return;
            }
            self.primed = true;
        }

        let gain = db_to_linear(mic.gain_db());
        let send = if mic.muted() { 0.0 } else { 1.0 };
        let mut peak = 0.0f32;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(self.buf.len());
            let got = cons.pop_slice(&mut self.buf[..n]);
            for (i, &s) in self.buf[..got].iter().enumerate() {
                let v = s * gain;
                peak = peak.max(v.abs());
                let o = v * send;
                for c in 0..channels {
                    out[(pos + i) * channels + c] += o;
                }
            }
            if got < n {
                // 欠载（或设备已断开）：重新缓冲。Underrun (or device gone): re-buffer.
                self.primed = false;
                break;
            }
            pos += n;
        }
        mic.update_level(peak.min(1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    use ringbuf::traits::{Producer as _, Split as _};
    use ringbuf::{HeapProd, HeapRb};

    /// 建一个 MicReader，并把一个新环交给它；返回（reader, 生产端, 旧环回收端）。
    /// Build a MicReader and hand it a fresh ring; returns (reader, producer,
    /// retired-ring receiver).
    fn reader_with_ring(cap: usize) -> (MicReader, HeapProd<f32>, Receiver<HeapCons<f32>>) {
        let (cons_tx, cons_rx) = bounded(4);
        let (retired_tx, retired_rx) = bounded(4);
        let reader = MicReader::new(
            MicLink {
                cons_rx,
                retired_tx,
            },
            48_000,
        );
        let (prod, cons) = HeapRb::<f32>::new(cap).split();
        cons_tx.send(cons).unwrap();
        // 保持发送端存活不是必须的：try_recv 读到已有数据即可。
        // Keeping the sender alive isn't required: try_recv still gets queued data.
        (reader, prod, retired_rx)
    }

    #[test]
    fn reader_without_mic_leaves_output_untouched() {
        let (_cons_tx, cons_rx) = bounded(4);
        let (retired_tx, _retired_rx) = bounded(4);
        let mut reader = MicReader::new(
            MicLink {
                cons_rx,
                retired_tx,
            },
            48_000,
        );
        let mic = MicControl::new(false, 0.0);
        let mut out = vec![0.3f32; 8];
        reader.mix_into(&mut out, 4, 2, &mic);
        assert_eq!(out, vec![0.3; 8]);
    }

    #[test]
    fn reader_waits_for_target_then_mixes_with_gain_and_mute() {
        let (mut reader, mut prod, _retired) = reader_with_ring(10_000);
        let mic = MicControl::new(false, 0.0);
        let target = reader.target;

        // 数据不足目标水位：输出不变。Below target: output untouched.
        prod.push_slice(&[0.5f32; 4]);
        let mut out = vec![0.0f32; 8];
        reader.mix_into(&mut out, 4, 2, &mic);
        assert_eq!(out, vec![0.0; 8]);

        // 补足后：叠加到每个声道。Once filled: added to every channel.
        prod.push_slice(&vec![0.5f32; target + 4]);
        reader.mix_into(&mut out, 4, 2, &mic);
        assert_eq!(out, vec![0.5; 8]);
        assert!((mic.level() - 0.5).abs() < 1e-6);

        // 静音：不叠加，但电平表仍显示输入。Muted: nothing added, meter still shows input.
        mic.set_muted(true);
        let mut out = vec![0.0f32; 8];
        reader.mix_into(&mut out, 4, 2, &mic);
        assert_eq!(out, vec![0.0; 8]);
        assert!(mic.level() > 0.4);
    }

    #[test]
    fn reader_trims_backlog() {
        let (mut reader, mut prod, _retired) = reader_with_ring(48_000);
        let mic = MicControl::new(false, 0.0);
        prod.push_slice(&vec![0.1f32; 20_000]);
        let mut out = vec![0.0f32; 512];
        reader.mix_into(&mut out, 512, 1, &mic);
        // 积压被裁到目标水位附近。Backlog trimmed to around the target.
        assert_eq!(reader.cons.as_ref().unwrap().occupied_len(), reader.target);
    }

    #[test]
    fn switching_ring_retires_old_one_and_rebuffers() {
        let (cons_tx, cons_rx) = bounded(4);
        let (retired_tx, retired_rx) = bounded(4);
        let mut reader = MicReader::new(
            MicLink {
                cons_rx,
                retired_tx,
            },
            48_000,
        );
        let mic = MicControl::new(false, 0.0);
        let target = reader.target;

        let (mut prod_a, cons_a) = HeapRb::<f32>::new(10_000).split();
        cons_tx.send(cons_a).unwrap();
        prod_a.push_slice(&vec![0.2f32; target + 4]);
        let mut out = vec![0.0f32; 4];
        reader.mix_into(&mut out, 4, 1, &mic);
        assert_eq!(out, vec![0.2; 4]);

        // 换新设备：旧环被送回，新环需重新缓冲。New device: old ring returned, new one
        // re-buffers.
        let (mut prod_b, cons_b) = HeapRb::<f32>::new(10_000).split();
        cons_tx.send(cons_b).unwrap();
        prod_b.push_slice(&[0.7f32; 4]);
        let mut out = vec![0.0f32; 4];
        reader.mix_into(&mut out, 4, 1, &mic);
        assert_eq!(out, vec![0.0; 4]);
        assert!(retired_rx.try_recv().is_ok());

        prod_b.push_slice(&vec![0.7f32; target]);
        reader.mix_into(&mut out, 4, 1, &mic);
        assert_eq!(out, vec![0.7; 4]);
    }
}
