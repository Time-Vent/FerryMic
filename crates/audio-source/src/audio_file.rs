//! 通用音频文件解码：WAV 用 [`hound`]，MP3/FLAC/OGG/AAC 等用 [`symphonia`]。
//! Universal audio file decode: WAV via [`hound`], MP3/FLAC/OGG/AAC etc. via
//! [`symphonia`].
//!
//! 根据文件扩展名选择解码器；所有格式统一输出为 f32 交错 PCM。
//! The decoder is chosen by file extension; all formats output interleaved f32 PCM.

use crate::wav_file::{DecodedAudio, SourceError};
use audio_types::SampleFormat;
use std::path::Path;

/// 解码任意支持的音频文件（WAV/MP3/FLAC/OGG/AAC 等）。
/// Decode any supported audio file (WAV/MP3/FLAC/OGG/AAC etc.).
pub fn decode_audio_file(path: &Path) -> Result<DecodedAudio, SourceError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    // WAV 用 hound（简单可靠）。WAV uses hound (simple and reliable).
    if ext == "wav" {
        return crate::wav_file::WavFileSource::open(path).map(|f| f.into_decoded());
    }

    // 其他格式用 symphonia。Other formats use symphonia.
    decode_with_symphonia(path)
}

/// 用 symphonia 解码非 WAV 格式。Decode non-WAV formats with symphonia.
fn decode_with_symphonia(path: &Path) -> Result<DecodedAudio, SourceError> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    // 打开文件并探测格式。Open file and probe format.
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| SourceError::Decode(format!("无法识别格式 unknown format: {e}")))?;

    let mut format = probed.format;

    // 找第一个音频轨道。Find the first audio track.
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| SourceError::Decode("无音频轨道 no audio track".into()))?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(1);

    // 创建解码器。Create the decoder.
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| SourceError::Decode(format!("无法创建解码器 no decoder: {e}")))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    // 循环读包并解码。Loop over packets and decode.
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break, // 到达文件末尾。Reached end of file.
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue, // 跳过坏包。Skip corrupt packets.
        };

        // 首次分配 SampleBuffer（容量按第一个包）。Allocate SampleBuffer on first packet.
        if sample_buf.is_none() {
            let spec = *decoded.spec();
            let cap = decoded.capacity() as u64;
            sample_buf = Some(SampleBuffer::<f32>::new(cap, spec));
        }

        // 转成交错 f32 并追加。Convert to interleaved f32 and append.
        let sb = sample_buf.as_mut().unwrap();
        sb.copy_interleaved_ref(decoded);
        samples.extend_from_slice(sb.samples());
    }

    if samples.is_empty() {
        return Err(SourceError::Decode("未解码到任何样本 decoded no samples".into()));
    }

    Ok(DecodedAudio {
        samples,
        sample_rate,
        channels,
        format: SampleFormat::F32,
    })
}
