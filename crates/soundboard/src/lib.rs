//! soundboard：音效板核心。
//! soundboard: soundboard core.
//!
//! 管理多个音效源，支持「触发语义 DIY」——每个音效可独立配置触发模式
//! （点按 / 循环 / 独占 / 叠加）、音量、变速。用于「在麦克风中随时放出音效」的
//! 场景（游戏整活、会议、直播）。
//! Manages multiple sound sources with DIY trigger semantics — each sound can
//! independently configure its trigger mode (one-shot / loop / exclusive / overlap),
//! volume, and speed. For the "play sound effects into the mic anytime" use case
//! (gaming, meetings, streaming).
//!
//! 实时安全约束：`SoundBoard` 由**单个实时线程**独占访问（`fill` / `apply_command`
//! 都在回调线程内）。控制侧（Web API / 全局热键）通过命令队列投递
//! [`BoardCommand`]，实时线程消费。文件解码在控制侧完成（[`LoadedSound::load`]），
//! 实时侧触发音效只克隆共享 PCM（引用计数），不做 IO、不解码、不分配。
//! Real-time constraint: `SoundBoard` is exclusively accessed by a **single
//! real-time thread** (`fill` / `apply_command` both run on the callback thread).
//! The control side (web API / global hotkey) pushes [`BoardCommand`] through a
//! queue consumed by the real-time thread. Files are decoded on the control side
//! ([`LoadedSound::load`]); triggering on the real-time side only clones the shared
//! PCM (a refcount bump) — no IO, no decoding, no allocation.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use audio_source::{SourceError, VariableRateSource, VarispeedSource};
use serde::{Deserialize, Serialize};

/// 音效唯一标识。Sound unique identifier.
pub type SoundId = u32;

/// 同时播放的实例上限（预分配，满了替换最早的实例）。
/// Max simultaneous instances (pre-allocated; when full, the oldest is replaced).
pub const MAX_INSTANCES: usize = 32;

/// 预留的音效定义容量，避免 `AddSound` 时哈希表扩容。
/// Reserved definition capacity, so `AddSound` doesn't grow the hash map.
const RESERVED_SOUNDS: usize = 256;

/// 触发模式（用户可 DIY）。
/// Trigger mode (user-configurable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    /// 点按：每按一次从头播放一遍，播完自动停止。
    /// One-shot: each press plays once from the start, then stops.
    OneShot,
    /// 循环：按一下开始循环，再按停止。
    /// Loop: one press starts looping, press again to stop.
    Loop,
    /// 独占：播放时停止其它所有音效，同一时刻只播这一个。
    /// Exclusive: stops all other sounds when triggered; only this one plays.
    Exclusive,
    /// 叠加：可与其它音效同时播放。
    /// Overlap: can play simultaneously with other sounds.
    Overlap,
}

/// 一个音效的定义（控制侧配置，可跨线程共享）。
/// A sound's definition (control-side config, shareable across threads).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundSpec {
    pub id: SoundId,
    /// 显示名称。Display name.
    pub name: String,
    /// 音频文件路径。Audio file path.
    pub file_path: std::path::PathBuf,
    /// 触发模式。Trigger mode.
    pub trigger_mode: TriggerMode,
    /// 增益（dB）。Gain in dB.
    pub gain_db: f32,
    /// 播放倍速（varispeed，变速变调）。Playback speed (varispeed).
    pub speed: f32,
    /// 快捷键（如 "F1"、"Ctrl+Alt+1"），None 表示未绑定。
    /// Hotkey (e.g. "F1", "Ctrl+Alt+1"); None means unbound.
    pub hotkey: Option<String>,
}

/// 已解码的音效：定义 + 共享 PCM。在控制侧构造，再交给实时侧。
/// A decoded sound: definition + shared PCM. Built on the control side, then handed
/// to the real-time side.
pub struct LoadedSound {
    pub spec: SoundSpec,
    source: VarispeedSource,
}

impl LoadedSound {
    /// 解码 `spec.file_path`（WAV/MP3/FLAC/OGG/AAC 等）。会做文件 IO 与解码，
    /// **不要在实时线程调用**。
    /// Decode `spec.file_path` (WAV/MP3/FLAC/OGG/AAC etc.). Does file IO and
    /// decoding — **never call on the real-time thread**.
    pub fn load(spec: SoundSpec) -> Result<Self, SourceError> {
        let source = VarispeedSource::from_file(&spec.file_path)?;
        Ok(Self { spec, source })
    }

    /// 源文件采样率。Source file sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    /// 源文件时长（秒）。Source file duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.source.total_frames() as f32 / self.source.sample_rate().max(1) as f32
    }
}

impl std::fmt::Debug for LoadedSound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedSound")
            .field("spec", &self.spec)
            .field("sample_rate", &self.source.sample_rate())
            .field("frames", &self.source.total_frames())
            .finish()
    }
}

/// 控制侧 → 实时侧的命令。
/// Command from control side → real-time side.
#[derive(Debug)]
pub enum BoardCommand {
    /// 添加/更新一个已解码的音效。Add/update a decoded sound.
    AddSound(Box<LoadedSound>),
    /// 移除一个音效。Remove a sound.
    RemoveSound { sound_id: SoundId },
    /// 触发一个音效（按触发模式启动/停止）。
    /// Trigger a sound (start/stop per its trigger mode).
    Trigger { sound_id: SoundId },
    /// 停止一个音效的所有实例。
    /// Stop all instances of a sound.
    Stop { sound_id: SoundId },
    /// 停止所有音效。Stop all sounds.
    StopAll,
    /// 设置某音效增益。Set a sound's gain.
    SetGain { sound_id: SoundId, gain_db: f32 },
    /// 设置某音效变速。Set a sound's speed.
    SetSpeed { sound_id: SoundId, speed: f32 },
    /// 设置某音效的触发模式（会停止它正在播放的实例）。
    /// Set a sound's trigger mode (stops its playing instances).
    SetMode {
        sound_id: SoundId,
        mode: TriggerMode,
    },
}

/// 正在播放的音效实例。
/// A playing sound instance.
struct Instance {
    sound_id: SoundId,
    source: VarispeedSource,
    /// 是否循环。Whether to loop.
    loop_: bool,
    /// 线性增益。Linear gain.
    gain_lin: f32,
    /// 播放倍速（用户设置）。Playback speed (user setting).
    speed: f32,
    /// 采样率换算比 = 源采样率 / 输出采样率。
    /// Sample-rate ratio = source rate / output rate.
    rate_ratio: f32,
    /// 是否已播放完毕（待移除）。Whether finished (pending removal).
    done: bool,
}

/// 音效板：管理音效定义与播放实例，实时线程独占。
/// Soundboard: manages sound definitions and playing instances; owned by the
/// real-time thread.
pub struct SoundBoard {
    /// 音效定义表（按 id）。Sound definitions by id.
    sounds: HashMap<SoundId, LoadedSound>,
    /// 正在播放的实例（容量固定为 `MAX_INSTANCES`）。
    /// Playing instances (capacity fixed at `MAX_INSTANCES`).
    instances: Vec<Instance>,
    /// 临时混音缓冲（预分配，避免 fill 内分配）。
    /// Scratch mix buffer (pre-allocated to avoid allocation in fill).
    scratch: Vec<f32>,
    /// 输出采样率。Output sample rate.
    output_rate: u32,
}

impl SoundBoard {
    /// 创建空音效板。`capacity_samples` 为 scratch 大小（样点数）；`fill` 请求更多时
    /// 自动分块处理。`output_rate` 为输出设备采样率。
    /// Create an empty board. `capacity_samples` sizes the scratch buffer; larger
    /// `fill` requests are processed in chunks. `output_rate` is the output device rate.
    pub fn new(capacity_samples: usize, output_rate: u32) -> Self {
        Self {
            sounds: HashMap::with_capacity(RESERVED_SOUNDS),
            instances: Vec::with_capacity(MAX_INSTANCES),
            scratch: vec![0.0; capacity_samples.max(64)],
            output_rate: output_rate.max(1),
        }
    }

    /// 应用一条命令（实时侧调用）。
    /// Apply one command (real-time side).
    pub fn apply_command(&mut self, cmd: BoardCommand) {
        match cmd {
            BoardCommand::AddSound(sound) => {
                let id = sound.spec.id;
                self.sounds.insert(id, *sound);
            }
            BoardCommand::RemoveSound { sound_id } => {
                self.sounds.remove(&sound_id);
                self.instances.retain(|i| i.sound_id != sound_id);
            }
            BoardCommand::Trigger { sound_id } => {
                self.trigger(sound_id);
            }
            BoardCommand::Stop { sound_id } => {
                self.instances.retain(|i| i.sound_id != sound_id);
            }
            BoardCommand::StopAll => {
                self.instances.clear();
            }
            BoardCommand::SetGain { sound_id, gain_db } => {
                if let Some(s) = self.sounds.get_mut(&sound_id) {
                    s.spec.gain_db = gain_db;
                }
                let lin = db_to_linear(gain_db);
                for i in &mut self.instances {
                    if i.sound_id == sound_id {
                        i.gain_lin = lin;
                    }
                }
            }
            BoardCommand::SetSpeed { sound_id, speed } => {
                if let Some(s) = self.sounds.get_mut(&sound_id) {
                    s.spec.speed = clamp_speed(speed);
                }
                for i in &mut self.instances {
                    if i.sound_id == sound_id {
                        i.speed = clamp_speed(speed);
                    }
                }
            }
            BoardCommand::SetMode { sound_id, mode } => {
                if let Some(s) = self.sounds.get_mut(&sound_id) {
                    s.spec.trigger_mode = mode;
                }
                // 旧实例按旧模式播放（如循环），直接停掉避免语义混乱。
                // Old instances follow the old mode (e.g. looping); stop them.
                self.instances.retain(|i| i.sound_id != sound_id);
            }
        }
    }

    /// 触发一个音效：根据其触发模式启动/停止实例。
    /// Trigger a sound: start/stop instances per its trigger mode.
    fn trigger(&mut self, sound_id: SoundId) {
        let Some(sound) = self.sounds.get(&sound_id) else {
            return; // 未定义的音效。Unknown sound.
        };
        let mode = sound.spec.trigger_mode;

        match mode {
            TriggerMode::OneShot | TriggerMode::Overlap => {
                // 点按 / 叠加：新建实例从头播，不停止其它。
                // One-shot / overlap: new instance from start, don't stop others.
            }
            TriggerMode::Loop => {
                // 循环：已有循环实例则停止，否则新建循环实例。
                // Loop: stop if already looping, else create a loop instance.
                let already = self
                    .instances
                    .iter()
                    .any(|i| i.sound_id == sound_id && i.loop_);
                if already {
                    self.instances.retain(|i| i.sound_id != sound_id);
                    return;
                }
            }
            TriggerMode::Exclusive => {
                // 独占：停止所有其它实例。Exclusive: stop all others.
                self.instances.clear();
            }
        }

        let inst = Instance {
            sound_id,
            // 只克隆 Arc，不复制 PCM、不分配。Arc clone only — no copy, no alloc.
            source: sound.source.clone(),
            loop_: mode == TriggerMode::Loop,
            gain_lin: db_to_linear(sound.spec.gain_db),
            speed: clamp_speed(sound.spec.speed),
            rate_ratio: sound.source.sample_rate().max(1) as f32 / self.output_rate as f32,
            done: false,
        };

        // 容量已满：替换最早的实例（不扩容）。Full: replace the oldest (no growth).
        if self.instances.len() == MAX_INSTANCES {
            self.instances.remove(0);
        }
        self.instances.push(inst);
    }

    /// 当前播放中的实例数。Number of playing instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// 已注册的音效数。Number of registered sounds.
    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    /// 依次回调每个播放实例的音效 id（同一音效可能出现多次）。不分配，可在实时线程调用。
    /// Call `f` with the sound id of every playing instance (a sound may appear more
    /// than once). Allocation-free; safe on the real-time thread.
    pub fn for_each_playing(&self, mut f: impl FnMut(SoundId)) {
        for i in &self.instances {
            f(i.sound_id);
        }
    }

    /// 混音：把所有实例的 PCM 按各自增益相加后写入 `out`（覆盖原内容）。
    /// Mix: sum all instances' PCM by their gain and write to `out` (overwrites).
    ///
    /// `channels` 为输出声道数；实例的源声道会被转换到 `channels`。
    /// `channels` is the output channel count; each instance's source channels are
    /// converted to `channels`.
    pub fn fill(&mut self, out: &mut [f32], frames: usize, channels: usize) {
        let frames = frames.min(out.len() / channels.max(1));

        // 清空输出。Clear output.
        for s in out.iter_mut() {
            *s = 0.0;
        }

        // 逐实例混音；scratch 不够大时分块。Mix each instance, chunked by scratch size.
        let mut idx = 0;
        while idx < self.instances.len() {
            let inst = &mut self.instances[idx];
            let src_ch = inst.source.channels().max(1);
            let chunk_frames = self.scratch.len() / src_ch;
            if chunk_frames == 0 {
                // 声道数超出 scratch，无法处理。Too many channels for scratch.
                inst.done = true;
            }

            let mut pos = 0;
            while pos < frames && !inst.done {
                let n = chunk_frames.min(frames - pos);
                let written = Self::fill_instance(&mut self.scratch, inst, n, src_ch);
                Self::accumulate(
                    &self.scratch,
                    src_ch,
                    channels,
                    written,
                    inst.gain_lin,
                    &mut out[pos * channels..],
                );
                pos += n;
            }

            // 移除已结束的实例。Remove finished instances.
            if inst.done {
                self.instances.swap_remove(idx);
            } else {
                idx += 1;
            }
        }
    }

    /// 填充单个实例（到 scratch），处理循环与结束标记。
    /// Fill a single instance (into scratch), handle looping and done flag.
    fn fill_instance(
        scratch: &mut [f32],
        inst: &mut Instance,
        frames: usize,
        src_ch: usize,
    ) -> usize {
        // 实际步进 = 用户倍速 × 采样率换算比（44.1k 文件在 48k 输出上不再变调）。
        // Step = user speed × rate ratio (a 44.1k file no longer shifts pitch at 48k).
        let step = inst.speed * inst.rate_ratio;
        let mut written = 0usize;
        while written < frames && !inst.done {
            let remaining = frames - written;
            let seg_start = written * src_ch;
            let n = inst.source.fill(
                &mut scratch[seg_start..seg_start + remaining * src_ch],
                remaining,
                step,
            );
            written += n;

            if inst.source.is_exhausted() {
                if inst.loop_ {
                    inst.source.reset(); // 循环：从头继续。Loop: restart.
                } else {
                    inst.done = true; // 非循环：结束。Non-loop: finished.
                    break;
                }
            }
            if n == 0 {
                // 防御：无进展则退出，避免死循环。
                // Guard: no progress, break to avoid infinite loop.
                inst.done = true;
                break;
            }
        }
        // 未写满的部分补静音。Pad silence for the unfilled part.
        for s in scratch[written * src_ch..frames * src_ch].iter_mut() {
            *s = 0.0;
        }
        written
    }

    /// 把 scratch 中该实例的输出（src_ch 声道）累加到 out（dst_ch 声道）。
    /// Accumulate this instance's output (src_ch) into out (dst_ch).
    fn accumulate(
        scratch: &[f32],
        src_ch: usize,
        dst_ch: usize,
        written: usize,
        gain_lin: f32,
        out: &mut [f32],
    ) {
        // 只累加实际写出的帧（written）。Only accumulate actually written frames.
        for f in 0..written {
            for c in 0..dst_ch {
                let src_c = if src_ch == 1 { 0 } else { c.min(src_ch - 1) };
                let sample = scratch[f * src_ch + src_c] * gain_lin;
                out[f * dst_ch + c] += sample;
            }
        }
    }
}

impl Default for SoundBoard {
    fn default() -> Self {
        Self::new(48_000 * 2, 48_000)
    }
}

/// dB → 线性幅度（-120dB 以下静音）。dB → linear amplitude (mute below -120dB).
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// 钳制倍速到合法范围。Clamp speed to a valid range.
#[inline]
fn clamp_speed(speed: f32) -> f32 {
    if speed.is_nan() || speed <= 0.0 {
        1.0
    } else {
        speed.clamp(0.0625, 16.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn spec(id: SoundId, path: PathBuf, mode: TriggerMode) -> SoundSpec {
        SoundSpec {
            id,
            name: format!("s{id}"),
            file_path: path,
            trigger_mode: mode,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }
    }

    fn add(board: &mut SoundBoard, id: SoundId, path: &Path, mode: TriggerMode) {
        let loaded = LoadedSound::load(spec(id, path.to_path_buf(), mode)).unwrap();
        board.apply_command(BoardCommand::AddSound(Box::new(loaded)));
    }

    #[test]
    fn db_linear_mute() {
        assert_eq!(db_to_linear(-120.0), 0.0);
        assert_eq!(db_to_linear(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn clamp_speed_invalid() {
        assert_eq!(clamp_speed(0.0), 1.0);
        assert_eq!(clamp_speed(f32::NAN), 1.0);
        assert_eq!(clamp_speed(100.0), 16.0);
    }

    #[test]
    fn load_missing_file_is_error() {
        let r = LoadedSound::load(spec(
            1,
            PathBuf::from("nonexistent.wav"),
            TriggerMode::OneShot,
        ));
        assert!(r.is_err());
    }

    #[test]
    fn trigger_unknown_sound_is_noop() {
        let mut board = SoundBoard::new(8, 48_000);
        board.apply_command(BoardCommand::Trigger { sound_id: 999 });
        assert_eq!(board.instance_count(), 0);
    }

    #[test]
    fn fill_empty_board_outputs_silence() {
        let mut board = SoundBoard::new(8, 48_000);
        let mut out = [0.5f32; 4];
        board.fill(&mut out, 2, 2);
        assert_eq!(out, [0.0; 4]);
    }

    /// 生成一个含正弦波的临时 WAV，返回路径。
    /// Generate a temp WAV containing a sine wave; returns its path.
    fn make_test_wav(dir: &Path, name: &str, sample_rate: u32, frames: usize) -> PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..frames {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin();
            w.write_sample((s * 30000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 端到端：触发一个真实音效，fill 应产出非零音频。
    /// End-to-end: trigger a real sound; fill should produce non-zero audio.
    #[test]
    fn trigger_real_sound_produces_audio() {
        let dir = temp_dir("soundboard-test");
        let wav = make_test_wav(&dir, "sine.wav", 48_000, 4800);

        let mut board = SoundBoard::new(4800, 48_000);
        add(&mut board, 1, &wav, TriggerMode::OneShot);

        // 触发后应有 1 个实例。After trigger, 1 instance.
        board.apply_command(BoardCommand::Trigger { sound_id: 1 });
        assert_eq!(board.instance_count(), 1);

        // fill 一帧应输出非零（正弦波非静音）。
        // A fill should output non-zero (sine is not silence).
        let mut out = vec![0.0f32; 512];
        board.fill(&mut out, 512, 1);
        let peak = out.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        assert!(peak > 0.01, "预期有声音，实际峰值 peak={peak}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 独占模式：触发后应只有一个实例，且播放新音效会清空旧的。
    /// Exclusive mode: triggering should clear existing instances.
    #[test]
    fn exclusive_trigger_clears_others() {
        let dir = temp_dir("soundboard-test-excl");
        let wav1 = make_test_wav(&dir, "a.wav", 48_000, 4800);
        let wav2 = make_test_wav(&dir, "b.wav", 48_000, 4800);

        let mut board = SoundBoard::new(4800, 48_000);
        add(&mut board, 1, &wav1, TriggerMode::Exclusive);
        add(&mut board, 2, &wav2, TriggerMode::Exclusive);

        board.apply_command(BoardCommand::Trigger { sound_id: 1 });
        assert_eq!(board.instance_count(), 1);

        // 触发第二个独占音效 → 清空第一个。
        // Trigger second exclusive sound → clears the first.
        board.apply_command(BoardCommand::Trigger { sound_id: 2 });
        assert_eq!(board.instance_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 改触发模式会停止旧实例，之后按新模式播放；for_each_playing 反映当前实例。
    /// Changing the mode stops old instances, then plays with the new mode;
    /// for_each_playing reflects current instances.
    #[test]
    fn set_mode_stops_instances_and_applies_new_mode() {
        let dir = temp_dir("soundboard-test-mode");
        let wav = make_test_wav(&dir, "s.wav", 48_000, 4800);

        let mut board = SoundBoard::new(4800, 48_000);
        add(&mut board, 7, &wav, TriggerMode::Loop);
        board.apply_command(BoardCommand::Trigger { sound_id: 7 });
        let mut ids = Vec::new();
        board.for_each_playing(|id| ids.push(id));
        assert_eq!(ids, vec![7]);

        board.apply_command(BoardCommand::SetMode {
            sound_id: 7,
            mode: TriggerMode::Overlap,
        });
        assert_eq!(board.instance_count(), 0);

        // 叠加模式：连按两次有两个实例（循环模式下第二次会停止）。
        // Overlap: two presses give two instances (Loop would stop on the second).
        board.apply_command(BoardCommand::Trigger { sound_id: 7 });
        board.apply_command(BoardCommand::Trigger { sound_id: 7 });
        assert_eq!(board.instance_count(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 实例数达到上限后，再触发替换最早的实例，不超过上限。
    /// At the instance cap, further triggers replace the oldest without exceeding it.
    #[test]
    fn instance_count_is_capped() {
        let dir = temp_dir("soundboard-test-cap");
        let wav = make_test_wav(&dir, "s.wav", 48_000, 4800);

        let mut board = SoundBoard::new(4800, 48_000);
        add(&mut board, 1, &wav, TriggerMode::Overlap);
        for _ in 0..MAX_INSTANCES + 5 {
            board.apply_command(BoardCommand::Trigger { sound_id: 1 });
        }
        assert_eq!(board.instance_count(), MAX_INSTANCES);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 44.1k 文件在 48k 输出上应按原时长播放（不变速不变调）。
    /// A 44.1k file on a 48k output plays at its original duration (no pitch shift).
    #[test]
    fn source_rate_is_converted_to_output_rate() {
        let dir = temp_dir("soundboard-test-rate");
        // 0.1 秒的 44.1k 文件。A 0.1 s file at 44.1k.
        let wav = make_test_wav(&dir, "s.wav", 44_100, 4410);

        let mut board = SoundBoard::new(4800, 48_000);
        add(&mut board, 1, &wav, TriggerMode::OneShot);
        board.apply_command(BoardCommand::Trigger { sound_id: 1 });

        // 48k 下 0.1 秒 = 4800 帧：4790 帧时仍在播，4810 帧时已结束。
        // 0.1 s at 48k = 4800 frames: still playing at 4790, finished by 4810.
        let mut out = vec![0.0f32; 4790];
        board.fill(&mut out, 4790, 1);
        assert_eq!(board.instance_count(), 1);
        let mut out = vec![0.0f32; 20];
        board.fill(&mut out, 20, 1);
        assert_eq!(board.instance_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单次 fill 超过 scratch 容量时分块处理，不越界。
    /// A fill larger than scratch is processed in chunks without going out of bounds.
    #[test]
    fn fill_larger_than_scratch_is_chunked() {
        let dir = temp_dir("soundboard-test-chunk");
        let wav = make_test_wav(&dir, "s.wav", 48_000, 4800);

        let mut board = SoundBoard::new(64, 48_000);
        add(&mut board, 1, &wav, TriggerMode::OneShot);
        board.apply_command(BoardCommand::Trigger { sound_id: 1 });

        let mut out = vec![0.0f32; 2048 * 2];
        board.fill(&mut out, 2048, 2);
        // 最后一帧也应有声音（不是只写了前 64 个样点）。
        // The tail must have audio too (not just the first 64 samples).
        let tail_peak = out[4000..].iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        assert!(tail_peak > 0.01, "tail peak={tail_peak}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
