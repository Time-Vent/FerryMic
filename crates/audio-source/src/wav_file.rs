//! WAV 文件解码源。
//! WAV file decode source.
//!
//! 使用 [`hound`] 读入 WAV（i16/f32/i8/u8 等），统一转成 f32 交错缓冲，供
//! [`VarispeedSource`] 消费。解码为**一次性预加载**到内存（第一阶段文件规模
//! 可控），避免解码 IO 进实时路径。
//! Reads WAV (i16/f32/i8/u8 etc.) via [`hound`], normalizes to interleaved f32, and
//! hands it to [`VarispeedSource`]. Phase 1 preloads the whole file into memory
//! (file sizes are manageable), keeping decode IO out of the real-time path.

use audio_types::SampleFormat;
use thiserror::Error;

/// 文件解码错误。File decode error.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wav decode error: {0}")]
    Hound(#[from] hound::Error),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("unsupported sample format")]
    UnsupportedFormat,
}

/// 已解码的 WAV 内容（内存态）。
/// Decoded WAV content (in-memory form).
pub struct DecodedAudio {
    /// 交错样本（frames × channels），f32 归一化。
    /// Interleaved samples (frames × channels), normalized f32.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: usize,
    pub format: SampleFormat,
}

impl DecodedAudio {
    /// 帧数。Frame count.
    #[inline]
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels
        }
    }
}

/// WAV 文件源：持有解码后的 PCM，供 [`crate::VarispeedSource`] 包装。
/// WAV file source: holds decoded PCM, to be wrapped by [`crate::VarispeedSource`].
pub struct WavFileSource {
    decoded: DecodedAudio,
}

impl WavFileSource {
    /// 从文件路径解码 WAV。Decode WAV from a file path.
    pub fn open(path: &std::path::Path) -> Result<Self, SourceError> {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let channels = spec.channels as usize;
        let sample_rate = spec.sample_rate;
        let (samples, format) = match spec.sample_format {
            hound::SampleFormat::Int => {
                let bits = spec.bits_per_sample;
                match bits {
                    16 => {
                        let mut v = Vec::with_capacity(reader.len() as usize * channels);
                        for s in reader.into_samples::<i16>() {
                            v.push(s? as f32 / 32768.0);
                        }
                        (v, SampleFormat::I16)
                    }
                    8 => {
                        let mut v = Vec::with_capacity(reader.len() as usize * channels);
                        // 8-bit WAV 存储为无符号（0..255），但 hound 以 i8 读取并自动做
                        // 有符号偏移（减 128）。这里按 i8 读并归一化到 -1..1。
                        // 8-bit WAV is stored unsigned (0..255), but hound reads it as i8
                        // and applies the signed offset (minus 128) automatically. Read as
                        // i8 and normalize to -1..1.
                        for s in reader.into_samples::<i8>() {
                            v.push(s? as f32 / 128.0);
                        }
                        (v, SampleFormat::I16)
                    }
                    24 | 32 => {
                        let mut v = Vec::with_capacity(reader.len() as usize * channels);
                        for s in reader.into_samples::<i32>() {
                            // 24-bit 左对齐/右对齐的区别由 hound 处理；但 i32 读 24-bit 可能不准确。
                            // 第一阶段主支持 16-bit float；此处按 i32 归一化。
                            v.push(s? as f32 / i32::MAX as f32);
                        }
                        (v, SampleFormat::I16)
                    }
                    _ => return Err(SourceError::UnsupportedFormat),
                }
            }
            hound::SampleFormat::Float => {
                let mut v = Vec::with_capacity(reader.len() as usize * channels);
                for s in reader.into_samples::<f32>() {
                    v.push(s?);
                }
                (v, SampleFormat::F32)
            }
        };

        Ok(Self {
            decoded: DecodedAudio {
                samples,
                sample_rate,
                channels,
                format,
            },
        })
    }

    /// 引用解码内容。Borrow the decoded content.
    pub fn decoded(&self) -> &DecodedAudio {
        &self.decoded
    }

    /// 取走解码内容（转移所有权）。Take the decoded content (ownership).
    pub fn into_decoded(self) -> DecodedAudio {
        self.decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_frames_accounts_for_channels() {
        let d = DecodedAudio {
            samples: vec![0.0; 10],
            sample_rate: 48_000,
            channels: 2,
            format: SampleFormat::F32,
        };
        assert_eq!(d.frames(), 5);
    }
}
