//! audio-types：SoundBridge 的纯数据类型层。
//!
//! 本 crate 只包含纯数据（`Copy`/`Clone`/`Send`/`Sync`），不依赖平台 API、
//! 不依赖具体线程模型。它是所有模块之间契约的基石。

#![forbid(unsafe_code)]

use std::fmt;

/// 采样率（Hz）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SampleRate(pub u32);

/// 声道数。1 = 单声道，2 = 立体声。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelCount(pub u16);

/// 帧数（一帧 = 每个声道一个采样）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameCount(pub usize);

impl SampleRate {
    /// 以 f32 表示的采样率，用于重采样等计算。
    #[inline]
    pub fn as_f32(self) -> f32 {
        self.0 as f32
    }
}

impl ChannelCount {
    #[inline]
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl FrameCount {
    #[inline]
    pub fn as_usize(self) -> usize {
        self.0
    }
}

/// 采样格式。SoundBridge 第一阶段的引擎内部统一为 f32（-1.0..=1.0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// 32-bit float，引擎内部规范格式。
    F32,
    /// 16-bit PCM，文件解码输入 / 声卡输出时可能涉及。
    I16,
}

/// 音频流配置：采样率、声道、帧缓冲区大小。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamConfig {
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    /// 每次回调期望的缓冲区帧数。0 表示由后端选择默认值。
    pub buffer_frames: FrameCount,
}

impl StreamConfig {
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate: SampleRate(sample_rate),
            channels: ChannelCount(channels),
            buffer_frames: FrameCount(0),
        }
    }

    pub const fn with_buffer(sample_rate: u32, channels: u16, buffer_frames: usize) -> Self {
        Self {
            sample_rate: SampleRate(sample_rate),
            channels: ChannelCount(channels),
            buffer_frames: FrameCount(buffer_frames),
        }
    }

    /// 立体声 48 kHz 32 采样默认配置。
    pub const DEFAULT: Self = Self::with_buffer(48_000, 2, 0);
}

/// 设备唯一标识（跨平台不透明字符串）。由平台适配器生成，引擎只透传、比较。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 设备信息描述。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    /// 是否为默认输入设备。
    pub is_default_input: bool,
    /// 是否为默认输出设备。
    pub is_default_output: bool,
    /// 该设备支持的输入流配置。
    pub supported_input: Vec<StreamConfig>,
    /// 该设备支持的输出流配置。
    pub supported_output: Vec<StreamConfig>,
}

/// 设备用途分类，供 UI 与引擎区分候选设备。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Input,
    Output,
    Duplex,
}

/// 引擎事件：实时路径以无锁方式上报给 Control 侧。
///
/// 这些事件是纯数据，可跨线程通过无锁队列传输，不携带任何句柄或锁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineEvent {
    /// 文件音源欠载（补静音）。携带累计欠载次数。
    Underrun(u64),
    /// 重采样漂移超出软性阈值（水位漂移报警）。携带当前水位与目标水位。
    DriftWarning { current: u32, target: u32 },
    /// 输入设备被移除。
    InputDeviceRemoved,
    /// 输出设备被移除。
    OutputDeviceRemoved,
    /// 实时路径发生了非致命处理异常（如格式不匹配导致的重置）。
    Recovered,
}

/// 音量/倍速等参数的合法范围标识，供参数验证层统一使用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gain(pub f32);

impl Gain {
    pub const UNITY: Gain = Gain(1.0);
    pub const MUTE: Gain = Gain(0.0);

    /// 将增益钳制到合理范围（避免 NaN/Inf 与爆炸增益）。
    #[inline]
    pub fn clamped(self) -> Self {
        if self.0.is_nan() {
            return Gain(0.0);
        }
        Gain(self.0.clamp(0.0, 64.0))
    }
}

/// 播放速（>0；1.0 为原速）。第一阶段为 varispeed 语义（变速变调）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySpeed(pub f32);

impl PlaySpeed {
    pub const NORMAL: PlaySpeed = PlaySpeed(1.0);

    #[inline]
    pub fn clamped(self) -> Self {
        if self.0.is_nan() || self.0 <= 0.0 {
            return PlaySpeed(1.0);
        }
        PlaySpeed(self.0.clamp(0.0625, 16.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_clamps_nan_to_mute() {
        assert_eq!(Gain(f32::NAN).clamped(), Gain(0.0));
    }

    #[test]
    fn gain_clamps_negative_to_zero() {
        assert_eq!(Gain(-3.0).clamped(), Gain(0.0));
    }

    #[test]
    fn play_speed_clamps_to_normal_on_invalid() {
        assert_eq!(PlaySpeed(0.0).clamped(), PlaySpeed::NORMAL);
        assert_eq!(PlaySpeed(-1.0).clamped(), PlaySpeed::NORMAL);
        assert_eq!(PlaySpeed(f32::NAN).clamped(), PlaySpeed::NORMAL);
    }
}
