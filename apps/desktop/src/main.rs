//! SoundBridge 桌面 CLI 演示壳。
//! SoundBridge desktop CLI demo shell.
//!
//! 第一阶段：无 GUI 的命令行壳，验证「文件(变速) + 麦克风 → 混音 → 声卡输出」
//! 端到端链路，并提供「离线渲染」以便不依赖硬件即可听到处理结果。
//! Phase 1: a command-line shell with no GUI, verifying the end-to-end
//! "file (varispeed) + mic → mix → speaker output" chain, plus an "offline render"
//! mode to hear the processed result without hardware.
//!
//! ```text
//! soundbridge --list                                    列出设备 list devices
//! soundbridge <file.wav> --render <out.wav> --speed 1.5  离线渲染 offline render
//! soundbridge <file.wav> [--speed 1.0] [--file-gain 0]   实时播放 real-time play
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use application::{App, AppConfig};
use anyhow::{Context, Result};
use audio_source::{VariableRateSource, VarispeedSource};

/// 命令行参数。Command-line arguments.
#[derive(Debug, Default)]
struct Args {
    file: Option<PathBuf>,
    render_to: Option<PathBuf>,
    speed: f32,
    file_gain: f32,
    mic_gain: f32,
    list: bool,
}

impl Args {
    /// 解析命令行参数。Parse command-line arguments.
    fn parse() -> Self {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut a = Args::default();

        let mut i = 0;
        while i < raw.len() {
            match raw[i].as_str() {
                "--list" => a.list = true,
                "--render" => {
                    a.render_to = raw.get(i + 1).map(PathBuf::from);
                    i += 1;
                }
                "--speed" => {
                    a.speed = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
                    i += 1;
                }
                "--file-gain" => {
                    a.file_gain = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    i += 1;
                }
                "--mic-gain" => {
                    a.mic_gain = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    i += 1;
                }
                other => {
                    // 第一个非选项参数视为输入文件。
                    // The first non-option argument is treated as the input file.
                    if a.file.is_none() && !other.starts_with('-') {
                        a.file = Some(PathBuf::from(other));
                    }
                }
            }
            i += 1;
        }
        a
    }
}

/// 主入口：解析参数，分派到 list / render / play。
/// Main entry: parse args, dispatch to list / render / play.
fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    if args.list {
        return list_devices();
    }

    let Some(file) = args.file.as_deref() else {
        print_usage();
        return Ok(());
    };

    // 离线渲染：不依赖音频硬件，把处理结果写成 WAV。
    // Offline render: no audio hardware needed; writes the processed result to WAV.
    if let Some(out) = args.render_to.as_deref() {
        return render_file(file, out, args.speed, args.file_gain);
    }

    // 实时播放：文件 + 麦克风 → 混音 → 声卡输出。
    // Real-time play: file + mic → mix → speaker output.
    let mut app = App::new(AppConfig::default());
    app.load_file(file)
        .with_context(|| format!("加载文件失败 failed to load: {}", file.display()))?;

    app.set_file_speed(args.speed);
    app.set_file_gain_db(args.file_gain);
    app.set_mic_gain_db(args.mic_gain);

    log::info!(
        "启动混音链路 start mix chain: file={} speed={} file_gain={}dB mic_gain={}dB",
        file.display(),
        args.speed,
        args.file_gain,
        args.mic_gain
    );

    let mut handle = app.start()?;
    log::info!("已启动 running。按 Ctrl-C 停止 press Ctrl-C to stop.");

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("无法设置 Ctrl-C 处理 failed to set Ctrl-C handler")?;

    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    handle.stop();
    log::info!("已停止 stopped.");
    Ok(())
}

/// 离线渲染：文件 → 变速(varispeed) → 增益 → 输出 WAV。
/// Offline render: file → varispeed → gain → output WAV.
fn render_file(input: &Path, output: &Path, speed: f32, gain_db: f32) -> Result<()> {
    let mut src = VarispeedSource::from_wav(input)
        .with_context(|| format!("加载文件失败 failed to load: {}", input.display()))?;

    let channels = src.channels().max(1) as u16;
    let sample_rate = src.sample_rate();
    let gain = db_to_linear(gain_db);

    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output, spec)
        .with_context(|| format!("创建输出失败 failed to create: {}", output.display()))?;

    let chunk = 1024usize;
    let mut buf = vec![0.0f32; chunk * channels as usize];

    loop {
        let n = src.fill(&mut buf, chunk, speed);
        if n == 0 {
            break; // 数据耗尽。Exhausted.
        }
        // 应用增益并钳制到 [-1, 1]，写 i16。
        // Apply gain, clamp to [-1, 1], write i16.
        for &s in &buf[..n * channels as usize] {
            let v = (s * gain).clamp(-1.0, 1.0);
            writer.write_sample((v * 32767.0) as i16)?;
        }
        if src.is_exhausted() {
            break;
        }
    }

    writer.finalize().context("写入 WAV 失败 failed to finalize")?;
    log::info!(
        "渲染完成 rendered: {} -> {} (speed={}, gain={}dB)",
        input.display(),
        output.display(),
        speed,
        gain_db
    );
    Ok(())
}

/// dB → 线性幅度。dB → linear amplitude.
fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// 列出设备。List devices.
fn list_devices() -> Result<()> {
    let app = App::new(AppConfig::default());
    let devices = app.devices()?;
    println!("=== 音频设备 Audio devices ===");
    for d in devices {
        let mut tags = Vec::new();
        if d.is_default_input {
            tags.push("默认输入 default-in");
        }
        if d.is_default_output {
            tags.push("默认输出 default-out");
        }
        let tag = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("- {}{}", d.name, tag);
        if !d.supported_output.is_empty() {
            let rates: Vec<String> = d
                .supported_output
                .iter()
                .map(|c| format!("{}Hz/{}ch", c.sample_rate.0, c.channels.0))
                .collect();
            println!("    输出 output: {}", rates.join(", "));
        }
        if !d.supported_input.is_empty() {
            let rates: Vec<String> = d
                .supported_input
                .iter()
                .map(|c| format!("{}Hz/{}ch", c.sample_rate.0, c.channels.0))
                .collect();
            println!("    输入 input: {}", rates.join(", "));
        }
    }
    Ok(())
}

/// 打印用法。Print usage.
fn print_usage() {
    println!(
        "SoundBridge 桌面演示 Desktop demo\n\
         用法 Usage:\n\
         \x20 soundbridge --list                              列出设备 list devices\n\
         \x20 soundbridge <in.wav> --render <out.wav> [--speed 1.5] [--file-gain 0]\n\
         \x20                                               离线渲染 offline render\n\
         \x20 soundbridge <in.wav> [--speed 1.0] [--file-gain 0] [--mic-gain 0]\n\
         \x20                                               实时播放文件+麦克风混音 play file+mic"
    );
}
