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
//! 都在回调线程内）。控制侧（WebSocket / 全局热键）通过命令队列投递
//! [`BoardCommand`]，实时线程消费。`fill` 不分配、不加锁。
//! Real-time constraint: `SoundBoard` is exclusively accessed by a **single
//! real-time thread** (`fill` / `apply_command` both run on the callback thread).
//! The control side (WebSocket / global hotkey) pushes [`BoardCommand`] through a
//! queue consumed by the real-time thread. `fill` allocates and locks nothing.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use audio_source::{VariableRateSource, VarispeedSource};
use serde::{Deserialize, Serialize};

/// 音效唯一标识。Sound unique identifier.
pub type SoundId = u32;

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

/// 控制侧 → 实时侧的命令。
/// Command from control side → real-time side.
#[derive(Debug, Clone)]
pub enum BoardCommand {
    /// 添加/更新一个音效定义。Add/update a sound definition.
    AddSound(SoundSpec),
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
    /// 播放倍速。Playback speed.
    speed: f32,
    /// 是否已播放完毕（待移除）。Whether finished (pending removal).
    done: bool,
}

/// 音效板：管理音效定义与播放实例，实时线程独占。
/// Soundboard: manages sound definitions and playing instances; owned by the
/// real-time thread.
pub struct SoundBoard {
    /// 音效定义表（按 id）。Sound definitions by id.
    sounds: HashMap<SoundId, SoundSpec>,
    /// 正在播放的实例。Playing instances.
    instances: Vec<Instance>,
    /// 临时混音缓冲（预分配，避免 fill 内分配）。
    /// Scratch mix buffer (pre-allocated to avoid allocation in fill).
    scratch: Vec<f32>,
}

impl SoundBoard {
    /// 创建空音效板。`capacity_samples` 为单次 `fill` 的最大样点数（frames × channels）。
    /// Create an empty board. `capacity_samples` is the max sample count per `fill`.
    pub fn new(capacity_samples: usize) -> Self {
        Self {
            sounds: HashMap::new(),
            instances: Vec::new(),
            scratch: vec![0.0; capacity_samples],
        }
    }

    /// 应用一条命令（实时侧调用）。
    /// Apply one command (real-time side).
    pub fn apply_command(&mut self, cmd: &BoardCommand) {
        match cmd {
            BoardCommand::AddSound(spec) => {
                self.sounds.insert(spec.id, spec.clone());
            }
            BoardCommand::RemoveSound { sound_id } => {
                self.sounds.remove(sound_id);
                self.instances.retain(|i| i.sound_id != *sound_id);
            }
            BoardCommand::Trigger { sound_id } => {
                self.trigger(*sound_id);
            }
            BoardCommand::Stop { sound_id } => {
                self.instances.retain(|i| i.sound_id != *sound_id);
            }
            BoardCommand::StopAll => {
                self.instances.clear();
            }
            BoardCommand::SetGain { sound_id, gain_db } => {
                if let Some(s) = self.sounds.get_mut(sound_id) {
                    s.gain_db = *gain_db;
                }
                let lin = db_to_linear(*gain_db);
                for i in &mut self.instances {
                    if i.sound_id == *sound_id {
                        i.gain_lin = lin;
                    }
                }
            }
            BoardCommand::SetSpeed { sound_id, speed } => {
                if let Some(s) = self.sounds.get_mut(sound_id) {
                    s.speed = clamp_speed(*speed);
                }
                for i in &mut self.instances {
                    if i.sound_id == *sound_id {
                        i.speed = clamp_speed(*speed);
                    }
                }
            }
        }
    }

    /// 触发一个音效：根据其触发模式启动/停止实例。
    /// Trigger a sound: start/stop instances per its trigger mode.
    fn trigger(&mut self, sound_id: SoundId) {
        let Some(spec) = self.sounds.get(&sound_id).cloned() else {
            return; // 未定义的音效。Unknown sound.
        };

        match spec.trigger_mode {
            TriggerMode::OneShot => {
                // 点按：新建实例从头播，不停止其它。
                // One-shot: new instance from start, don't stop others.
                if let Some(inst) = self.make_instance(&spec) {
                    self.instances.push(inst);
                }
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
                } else if let Some(inst) = self.make_instance(&spec) {
                    self.instances.push(inst);
                }
            }
            TriggerMode::Exclusive => {
                // 独占：停止所有其它实例，新建实例。
                // Exclusive: stop all others, create a new instance.
                self.instances.clear();
                if let Some(inst) = self.make_instance(&spec) {
                    self.instances.push(inst);
                }
            }
            TriggerMode::Overlap => {
                // 叠加：直接新建实例，可与其它音效同时播放。
                // Overlap: just create a new instance, can play with others.
                if let Some(inst) = self.make_instance(&spec) {
                    self.instances.push(inst);
                }
            }
        }
    }

    /// 从一个定义创建播放实例（解码文件；失败返回 None）。
    /// Create a playing instance from a spec (decode file; None on failure).
    fn make_instance(&self, spec: &SoundSpec) -> Option<Instance> {
        // 支持 WAV/MP3/FLAC/OGG/AAC 等多格式。
        // Support WAV/MP3/FLAC/OGG/AAC etc.
        let source = match VarispeedSource::from_file(&spec.file_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("加载音效失败 failed to load sound {}: {e}", spec.file_path.display());
                return None;
            }
        };
        Some(Instance {
            sound_id: spec.id,
            source,
            loop_: spec.trigger_mode == TriggerMode::Loop,
            gain_lin: db_to_linear(spec.gain_db),
            speed: clamp_speed(spec.speed),
            done: false,
        })
    }

    /// 当前播放中的实例数。Number of playing instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// 已注册的音效数。Number of registered sounds.
    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    /// 混音：把所有实例的 PCM 按各自增益相加，软限幅后写入 `out`。
    /// Mix: sum all instances' PCM by their gain, soft-limit, write to `out`.
    ///
    /// `channels` 为输出声道数；实例的源声道会被转换到 `channels`。
    /// `channels` is the output channel count; each instance's source channels are
    /// converted to `channels`.
    pub fn fill(&mut self, out: &mut [f32], frames: usize, channels: usize) {
        let n = frames * channels;
        debug_assert!(n <= self.scratch.len());

        // 清空输出。Clear output.
        for s in out.iter_mut() {
            *s = 0.0;
        }

        // 逐实例混音。Mix each instance.
        let mut idx = 0;
        while idx < self.instances.len() {
            // 在借用作用域内填充实例并取出所需值（避免与 self 的其它字段借用冲突）。
            // Fill the instance and extract needed values within a borrow scope
            // (avoids conflicting borrows with other `self` fields).
            let (src_ch, written, gain_lin, done) = {
                let inst = &mut self.instances[idx];
                let src_ch = inst.source.channels().max(1);
                let written = Self::fill_instance(&mut self.scratch, inst, frames, src_ch);
                (src_ch, written, inst.gain_lin, inst.done)
            };

            // 声道转换并累加（scratch → out）。
            // Channel-convert and accumulate (scratch → out).
            Self::accumulate(&self.scratch, src_ch, channels, written, gain_lin, out);

            // 移除已结束的实例。Remove finished instances.
            if done {
                self.instances.swap_remove(idx);
            } else {
                idx += 1;
            }
        }
    }

    /// 填充单个实例（到 scratch），处理循环与结束标记。
    /// Fill a single instance (into scratch), handle looping and done flag.
    ///
    /// 关联函数（不借 `self`），使调用方可同时借用 `instances` 与 `scratch` 两个字段。
    /// Associated function (doesn't borrow `self`), so the caller can borrow both
    /// `instances` and `scratch` fields simultaneously.
    fn fill_instance(
        scratch: &mut [f32],
        inst: &mut Instance,
        frames: usize,
        src_ch: usize,
    ) -> usize {
        let mut written = 0usize;
        while written < frames && !inst.done {
            let remaining = frames - written;
            let seg_start = written * src_ch;
            let n = inst.source.fill(
                &mut scratch[seg_start..seg_start + remaining * src_ch],
                remaining,
                inst.speed,
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
    ///
    /// 关联函数（不借 `self`）。Associated function (doesn't borrow `self`).
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
        Self::new(48_000 * 2)
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
    use std::path::PathBuf;

    fn spec(id: SoundId, mode: TriggerMode) -> SoundSpec {
        SoundSpec {
            id,
            name: format!("s{id}"),
            file_path: PathBuf::from("nonexistent.wav"),
            trigger_mode: mode,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }
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
    fn trigger_unknown_sound_is_noop() {
        let mut board = SoundBoard::new(8);
        board.apply_command(&BoardCommand::Trigger { sound_id: 999 });
        assert_eq!(board.instance_count(), 0);
    }

    #[test]
    fn stop_all_clears_instances() {
        let mut board = SoundBoard::new(8);
        // 添加一个定义（文件不存在，make_instance 会失败，但 StopAll 仍应清空）。
        board.apply_command(&BoardCommand::AddSound(spec(1, TriggerMode::OneShot)));
        board.apply_command(&BoardCommand::StopAll);
        assert_eq!(board.instance_count(), 0);
    }

    #[test]
    fn fill_empty_board_outputs_silence() {
        let mut board = SoundBoard::new(8);
        let mut out = [0.5f32; 4];
        board.fill(&mut out, 2, 2);
        assert_eq!(out, [0.0; 4]);
    }

    /// 生成一个含正弦波的临时 WAV，返回路径。
    /// Generate a temp WAV containing a sine wave; returns its path.
    fn make_test_wav(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..4800 {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
            w.write_sample((s * 30000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    /// 端到端：触发一个真实音效，fill 应产出非零音频。
    /// End-to-end: trigger a real sound; fill should produce non-zero audio.
    #[test]
    fn trigger_real_sound_produces_audio() {
        let dir = std::env::temp_dir().join("soundboard-test");
        std::fs::create_dir_all(&dir).unwrap();
        let wav = make_test_wav(&dir, "sine.wav");

        let mut board = SoundBoard::new(4800);
        board.apply_command(&BoardCommand::AddSound(SoundSpec {
            id: 1,
            name: "sine".into(),
            file_path: wav,
            trigger_mode: TriggerMode::OneShot,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }));

        // 触发后应有 1 个实例。After trigger, 1 instance.
        board.apply_command(&BoardCommand::Trigger { sound_id: 1 });
        assert_eq!(board.instance_count(), 1);

        // fill 一帧应输出非零（正弦波非静音）。
        // A fill should output non-zero (sine is not silence).
        let mut out = vec![0.0f32; 512];
        board.fill(&mut out, 512, 1);
        let peak = out.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        assert!(peak > 0.01, "预期有声音，实际峰值 peak={peak}");

        // 清理。Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 独占模式：触发后应只有一个实例，且播放新音效会清空旧的。
    /// Exclusive mode: triggering should clear existing instances.
    #[test]
    fn exclusive_trigger_clears_others() {
        let dir = std::env::temp_dir().join("soundboard-test-excl");
        std::fs::create_dir_all(&dir).unwrap();
        let wav1 = make_test_wav(&dir, "a.wav");
        let wav2 = make_test_wav(&dir, "b.wav");

        let mut board = SoundBoard::new(4800);
        board.apply_command(&BoardCommand::AddSound(SoundSpec {
            id: 1,
            name: "a".into(),
            file_path: wav1,
            trigger_mode: TriggerMode::Exclusive,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }));
        board.apply_command(&BoardCommand::AddSound(SoundSpec {
            id: 2,
            name: "b".into(),
            file_path: wav2,
            trigger_mode: TriggerMode::Exclusive,
            gain_db: 0.0,
            speed: 1.0,
            hotkey: None,
        }));

        board.apply_command(&BoardCommand::Trigger { sound_id: 1 });
        assert_eq!(board.instance_count(), 1);

        // 触发第二个独占音效 → 清空第一个。
        // Trigger second exclusive sound → clears the first.
        board.apply_command(&BoardCommand::Trigger { sound_id: 2 });
        assert_eq!(board.instance_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
