//! audio-cpal：基于 [`cpal`] 的 [`platform_api::AudioBackend`] 实现。
//! audio-cpal: [`platform_api::AudioBackend`] implementation backed by [`cpal`].
//!
//! 覆盖所有平台（Windows WASAPI / macOS CoreAudio / Linux ALSA-PulseAudio-PipeWire）。
//! Covers all platforms (Windows WASAPI / macOS CoreAudio / Linux ALSA-PulseAudio-PipeWire).
//!
//! 实时回调约束：cpal 的回调即音频线程；回调体必须不分配/不加锁/不 panic。
//! Real-time constraint: cpal's callback IS the audio thread; the callback body must
//! not allocate / lock / panic.

#![forbid(unsafe_code)]

use std::sync::Mutex;

use audio_types::{DeviceId, DeviceInfo, StreamConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use platform_api::{AudioBackend, PlatformError};

/// cpal 后端：包装 `cpal::Host`。
/// cpal backend: wraps `cpal::Host`.
pub struct CpalBackend {
    host: cpal::Host,
}

impl CpalBackend {
    /// 使用默认主机创建。Create using the default host.
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    /// 用指定主机创建（测试/注入用）。Create with an explicit host (for tests/DI).
    pub fn with_host(host: cpal::Host) -> Self {
        Self { host }
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// 内部把 cpal 设备名 + 后端唯一标识组合成稳定 DeviceId。
/// Combine cpal device name + backend id into a stable DeviceId.
fn make_device_id(host: &cpal::Host, dev: &cpal::Device) -> DeviceId {
    let name = dev.name().unwrap_or_else(|_| "unnamed".to_string());
    DeviceId(format!("{}::{}", host.id().name(), name))
}

/// cpal 配置 → 我们的 StreamConfig（默认缓冲区为设备默认）。
/// cpal config → our StreamConfig (default buffer = device default).
fn from_cpal_config(c: &cpal::StreamConfig) -> StreamConfig {
    StreamConfig::with_buffer(c.sample_rate.0, c.channels, 0)
}

/// 实际采样率选取：请求值在支持范围内则用请求值，否则用最高支持值。
/// Pick the actual sample rate: use requested if in range, else the max supported.
fn pick_rate(requested: u32, min: u32, max: u32) -> u32 {
    if (min..=max).contains(&requested) {
        requested
    } else {
        max
    }
}

/// 缓冲区大小：0 → 设备默认，否则固定。Buffer size: 0 → default, else fixed.
fn to_buffer_size(frames: usize) -> cpal::BufferSize {
    if frames == 0 {
        cpal::BufferSize::Default
    } else {
        cpal::BufferSize::Fixed(frames as u32)
    }
}

impl AudioBackend for CpalBackend {
    fn enumerate_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        let mut out = Vec::new();
        let default_in = self.host.default_input_device();
        let default_out = self.host.default_output_device();

        let devices = self
            .host
            .devices()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;

        for dev in devices {
            let id = make_device_id(&self.host, &dev);
            let name = dev.name().unwrap_or_else(|_| "unnamed".to_string());
            let is_default_input = default_in
                .as_ref()
                .and_then(|d| d.name().ok())
                .map(|n| n == name)
                .unwrap_or(false);
            let is_default_output = default_out
                .as_ref()
                .and_then(|d| d.name().ok())
                .map(|n| n == name)
                .unwrap_or(false);

            let mut supported_input = Vec::new();
            if let Ok(cfgs) = dev.supported_input_configs() {
                for c in cfgs {
                    let c = c.with_max_sample_rate();
                    let cfg = c.config();
                    supported_input.push(from_cpal_config(&cfg));
                }
            }
            let mut supported_output = Vec::new();
            if let Ok(cfgs) = dev.supported_output_configs() {
                for c in cfgs {
                    let c = c.with_max_sample_rate();
                    let cfg = c.config();
                    supported_output.push(from_cpal_config(&cfg));
                }
            }

            out.push(DeviceInfo {
                id,
                name,
                is_default_input,
                is_default_output,
                supported_input,
                supported_output,
            });
        }
        Ok(out)
    }

    fn open_output<F>(
        &self,
        config: &StreamConfig,
        callback: F,
    ) -> Result<Box<dyn platform_api::OutputStreamHandle>, PlatformError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        // 找到默认输出设备（或首个可用输出设备）。
        // Find the default output device (or first available output device).
        let device = self
            .host
            .default_output_device()
            .or_else(|| self.host.output_devices().ok().and_then(|mut it| it.next()))
            .ok_or_else(|| PlatformError::DeviceNotFound("no output device".into()))?;

        self.build_output_on(device, config, callback)
    }

    fn open_input<F>(
        &self,
        config: &StreamConfig,
        callback: F,
    ) -> Result<Box<dyn platform_api::InputStreamHandle>, PlatformError>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let device = self
            .host
            .default_input_device()
            .or_else(|| self.host.input_devices().ok().and_then(|mut it| it.next()))
            .ok_or_else(|| PlatformError::DeviceNotFound("no input device".into()))?;

        let supported = device
            .supported_input_configs()
            .map_err(|e| PlatformError::Backend(e.to_string()))?
            .find(|c| {
                c.channels() == config.channels.0
                    && (c.min_sample_rate().0..=c.max_sample_rate().0)
                        .contains(&config.sample_rate.0)
            })
            .or_else(|| {
                device
                    .supported_input_configs()
                    .ok()
                    .and_then(|mut it| it.next())
            })
            .ok_or_else(|| PlatformError::UnsupportedConfig {
                device: device.name().unwrap_or_default(),
                config: *config,
            })?;

        let actual_rate = pick_rate(
            config.sample_rate.0,
            supported.min_sample_rate().0,
            supported.max_sample_rate().0,
        );

        let stream_config: cpal::StreamConfig = cpal::StreamConfig {
            channels: config.channels.0,
            sample_rate: cpal::SampleRate(actual_rate),
            buffer_size: to_buffer_size(config.buffer_frames.0),
        };

        let mut callback = callback;
        let stream = device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _info| callback(data),
                |err| log::error!("cpal input stream error: {err}"),
                None,
            )
            .map_err(|e| PlatformError::Backend(e.to_string()))?;

        stream
            .play()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;

        Ok(Box::new(CpalInputStreamHandle {
            inner: Mutex::new(Some(stream)),
        }))
    }
}

impl CpalBackend {
    /// 按设备名（模糊匹配）打开输出流。
    /// Open an output stream by device name (fuzzy match).
    ///
    /// `device_name` 与设备名做「包含」匹配；用于让用户选择声卡或虚拟音频线。
    /// `device_name` is matched by containment against device names; lets the user
    /// pick a speaker or a virtual audio cable.
    pub fn open_output_named<F>(
        &self,
        config: &StreamConfig,
        device_name: &str,
        callback: F,
    ) -> Result<Box<dyn platform_api::OutputStreamHandle>, PlatformError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let device = self
            .host
            .output_devices()
            .map_err(|e| PlatformError::Backend(e.to_string()))?
            .find(|d| {
                d.name()
                    .map(|n| n.to_lowercase().contains(&device_name.to_lowercase()))
                    .unwrap_or(false)
            })
            .ok_or_else(|| PlatformError::DeviceNotFound(device_name.to_string()))?;

        self.build_output_on(device, config, callback)
    }

    /// 在指定设备上构建输出流（内部核心逻辑，供 `open_output` / `open_output_named` 复用）。
    /// Build an output stream on a given device (internal core, shared by
    /// `open_output` / `open_output_named`).
    fn build_output_on<F>(
        &self,
        device: cpal::Device,
        config: &StreamConfig,
        callback: F,
    ) -> Result<Box<dyn platform_api::OutputStreamHandle>, PlatformError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        // 取一个兼容的支持配置。Pick a compatible supported config.
        let supported = device
            .supported_output_configs()
            .map_err(|e| PlatformError::Backend(e.to_string()))?
            .find(|c| {
                c.channels() == config.channels.0
                    && (c.min_sample_rate().0..=c.max_sample_rate().0)
                        .contains(&config.sample_rate.0)
            })
            .or_else(|| {
                device
                    .supported_output_configs()
                    .ok()
                    .and_then(|mut it| it.next())
            })
            .ok_or_else(|| PlatformError::UnsupportedConfig {
                device: device.name().unwrap_or_default(),
                config: *config,
            })?;

        let actual_rate = pick_rate(
            config.sample_rate.0,
            supported.min_sample_rate().0,
            supported.max_sample_rate().0,
        );

        let stream_config: cpal::StreamConfig = cpal::StreamConfig {
            channels: config.channels.0,
            sample_rate: cpal::SampleRate(actual_rate),
            buffer_size: to_buffer_size(config.buffer_frames.0),
        };

        let mut callback = callback;
        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _info| callback(data),
                |err| log::error!("cpal output stream error: {err}"),
                None,
            )
            .map_err(|e| PlatformError::Backend(e.to_string()))?;

        stream
            .play()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;

        Ok(Box::new(CpalOutputStreamHandle {
            inner: Mutex::new(Some(stream)),
        }))
    }
}

/// 已解析的设备：设备本身 + 其原生（共享模式）配置。
/// A resolved device: the device plus its native (shared-mode) config.
///
/// 先解析、再按 [`ResolvedDevice::config`] 准备回调状态（采样率、声道数），
/// 最后用 [`CpalBackend::build_output`] / [`CpalBackend::build_input`] 建流。
/// Resolve first, prepare callback state from [`ResolvedDevice::config`] (rate,
/// channels), then open the stream with [`CpalBackend::build_output`] /
/// [`CpalBackend::build_input`].
pub struct ResolvedDevice {
    device: cpal::Device,
    name: String,
    config: StreamConfig,
}

impl ResolvedDevice {
    /// 设备名。Device name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 设备原生采样率与声道数（`buffer_frames` 为 0）。
    /// The device's native rate and channel count (`buffer_frames` is 0).
    pub fn config(&self) -> StreamConfig {
        self.config
    }
}

impl CpalBackend {
    /// 解析输出设备：`name` 为 `Some` 时按名称「包含」匹配（不区分大小写），否则用默认设备。
    /// Resolve an output device: `Some(name)` matches by case-insensitive containment,
    /// `None` uses the default device.
    pub fn resolve_output(&self, name: Option<&str>) -> Result<ResolvedDevice, PlatformError> {
        let device = match name {
            Some(n) => find_by_name(
                self.host
                    .output_devices()
                    .map_err(|e| PlatformError::Backend(e.to_string()))?,
                n,
            ),
            None => self.host.default_output_device(),
        }
        .ok_or_else(|| {
            PlatformError::DeviceNotFound(name.unwrap_or("default output").to_string())
        })?;
        let cfg = device
            .default_output_config()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(resolved(device, &cfg))
    }

    /// 解析输入设备（麦克风），规则同 [`Self::resolve_output`]。
    /// Resolve an input device (microphone); same rules as [`Self::resolve_output`].
    pub fn resolve_input(&self, name: Option<&str>) -> Result<ResolvedDevice, PlatformError> {
        let device = match name {
            Some(n) => find_by_name(
                self.host
                    .input_devices()
                    .map_err(|e| PlatformError::Backend(e.to_string()))?,
                n,
            ),
            None => self.host.default_input_device(),
        }
        .ok_or_else(|| {
            PlatformError::DeviceNotFound(name.unwrap_or("default input").to_string())
        })?;
        let cfg = device
            .default_input_config()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(resolved(device, &cfg))
    }

    /// 以设备原生配置打开输出流。`buffer_frames` 为 0 表示设备默认缓冲区。
    /// 回调收到的帧数可能与 `buffer_frames` 不同，回调须能处理任意长度。
    /// Open an output stream at the device's native config. `buffer_frames` 0 means
    /// the device default. The callback may receive a different frame count than
    /// `buffer_frames` and must handle any length.
    pub fn build_output<F>(
        &self,
        dev: ResolvedDevice,
        buffer_frames: usize,
        callback: F,
    ) -> Result<Box<dyn platform_api::OutputStreamHandle>, PlatformError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let stream_config = cpal::StreamConfig {
            channels: dev.config.channels.0,
            sample_rate: cpal::SampleRate(dev.config.sample_rate.0),
            buffer_size: to_buffer_size(buffer_frames),
        };
        let mut callback = callback;
        let stream = dev
            .device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _info| callback(data),
                |err| log::error!("cpal output stream error: {err}"),
                None,
            )
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        stream
            .play()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(Box::new(CpalOutputStreamHandle {
            inner: Mutex::new(Some(stream)),
        }))
    }

    /// 以设备原生配置打开输入流（缓冲区用设备默认值）。
    /// `on_error` 在流出错时调用（如蓝牙耳机断开、设备被拔出）。
    /// Open an input stream at the device's native config (default buffer size).
    /// `on_error` is called when the stream fails (e.g. a Bluetooth headset
    /// disconnects or the device is unplugged).
    pub fn build_input<F, E>(
        &self,
        dev: ResolvedDevice,
        callback: F,
        mut on_error: E,
    ) -> Result<Box<dyn platform_api::InputStreamHandle>, PlatformError>
    where
        F: FnMut(&[f32]) + Send + 'static,
        E: FnMut(String) + Send + 'static,
    {
        let stream_config = cpal::StreamConfig {
            channels: dev.config.channels.0,
            sample_rate: cpal::SampleRate(dev.config.sample_rate.0),
            buffer_size: cpal::BufferSize::Default,
        };
        let mut callback = callback;
        let stream = dev
            .device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _info| callback(data),
                move |err| {
                    log::error!("cpal input stream error: {err}");
                    on_error(err.to_string());
                },
                None,
            )
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        stream
            .play()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(Box::new(CpalInputStreamHandle {
            inner: Mutex::new(Some(stream)),
        }))
    }
}

/// 按名称「包含」匹配设备（不区分大小写）。Find a device by case-insensitive containment.
fn find_by_name(
    mut devices: impl Iterator<Item = cpal::Device>,
    name: &str,
) -> Option<cpal::Device> {
    let needle = name.to_lowercase();
    devices.find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains(&needle))
            .unwrap_or(false)
    })
}

/// 组装 [`ResolvedDevice`]。Build a [`ResolvedDevice`].
fn resolved(device: cpal::Device, cfg: &cpal::SupportedStreamConfig) -> ResolvedDevice {
    let name = device.name().unwrap_or_else(|_| "unnamed".to_string());
    ResolvedDevice {
        device,
        name,
        config: StreamConfig::with_buffer(cfg.sample_rate().0, cfg.channels(), 0),
    }
}

/// 输出流句柄（drop 时停止流）。Output stream handle (stops on drop).
pub struct CpalOutputStreamHandle {
    inner: Mutex<Option<cpal::Stream>>,
}

impl platform_api::OutputStreamHandle for CpalOutputStreamHandle {
    fn pause(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.pause()
                .map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.play()
                .map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }
}

/// 输入流句柄（drop 时停止流）。Input stream handle (stops on drop).
pub struct CpalInputStreamHandle {
    inner: Mutex<Option<cpal::Stream>>,
}

impl platform_api::InputStreamHandle for CpalInputStreamHandle {
    fn pause(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.pause()
                .map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.play()
                .map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }
}
