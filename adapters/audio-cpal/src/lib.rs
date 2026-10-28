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

use std::sync::{Arc, Mutex};

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

        // 取一个兼容的支持配置。
        // Pick a compatible supported config.
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

        stream.play().map_err(|e| PlatformError::Backend(e.to_string()))?;

        Ok(Box::new(CpalOutputStreamHandle {
            inner: Arc::new(Mutex::new(Some(stream))),
        }))
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

        stream.play().map_err(|e| PlatformError::Backend(e.to_string()))?;

        Ok(Box::new(CpalInputStreamHandle {
            inner: Arc::new(Mutex::new(Some(stream))),
        }))
    }
}

/// 输出流句柄（drop 时停止流）。Output stream handle (stops on drop).
pub struct CpalOutputStreamHandle {
    inner: Arc<Mutex<Option<cpal::Stream>>>,
}

impl platform_api::OutputStreamHandle for CpalOutputStreamHandle {
    fn pause(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.pause().map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.play().map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }
}

/// 输入流句柄（drop 时停止流）。Input stream handle (stops on drop).
pub struct CpalInputStreamHandle {
    inner: Arc<Mutex<Option<cpal::Stream>>>,
}

impl platform_api::InputStreamHandle for CpalInputStreamHandle {
    fn pause(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.pause().map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlatformError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PlatformError::Backend("lock poisoned".into()))?;
        if let Some(s) = guard.as_ref() {
            s.play().map_err(|e| PlatformError::Backend(e.to_string()))?;
        }
        Ok(())
    }
}
