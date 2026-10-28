//! Linux 虚拟麦克风：通过 PipeWire / PulseAudio 创建一对虚拟设备，无需第三方软件。
//! Linux virtual microphone: creates a pair of virtual devices through PipeWire /
//! PulseAudio, with no third-party software.
//!
//! ```text
//! SoundBridge 输出 ──▶ 虚拟输出 SoundBridge-Mix（null sink）
//!                         └─ monitor ──▶ 虚拟麦克风 SoundBridge-Microphone（remap source）
//!                                              └─▶ 游戏 / Discord 里选它当麦克风
//! ```
//!
//! - 用 `pactl` 加载模块（PipeWire 的 pipewire-pulse 与 PulseAudio 都支持），退出时卸载；
//!   上次异常退出遗留的模块会被接管并复用，不会越建越多。
//!   Modules are loaded with `pactl` (supported by both pipewire-pulse and PulseAudio)
//!   and unloaded on exit; modules left over by a crash are adopted and reused.
//! - cpal 在 Linux 走 ALSA，无法按名字选 PulseAudio 设备，所以通过环境变量
//!   `PULSE_SINK` / `PIPEWIRE_NODE` 让本进程的播放流进入虚拟输出（只影响播放，不影响麦克风）。
//!   cpal uses ALSA on Linux and can't pick PulseAudio devices by name, so the
//!   `PULSE_SINK` / `PIPEWIRE_NODE` environment variables route this process's playback
//!   into the virtual sink (playback only; the mic is unaffected).
//!
//! 解析 `pactl` 输出的函数与平台无关，便于在任何平台上测试。
//! The `pactl` output parsers are platform-independent so they can be tested anywhere.

// 非 Linux 平台只编译不调用（解析函数仍由测试覆盖）。
// Off Linux these items compile but aren't called (the parsers are still tested).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::Serialize;

/// 虚拟输出（null sink）名称。Virtual sink name.
pub const SINK_NAME: &str = "soundbridge_mix";
/// 虚拟麦克风（remap source）名称。Virtual mic (remap source) name.
pub const SOURCE_NAME: &str = "soundbridge_mic";
/// 游戏里看到的麦克风名称。The mic name shown in games.
pub const SOURCE_DESCRIPTION: &str = "SoundBridge-Microphone";

/// 虚拟麦克风状态（供设置页显示）。Virtual mic status (for the settings page).
#[derive(Debug, Clone, Serialize)]
pub struct VirtualMicInfo {
    /// 是否已创建。Whether created.
    pub active: bool,
    /// 游戏里应选择的麦克风名。The mic to pick in games.
    pub source: String,
    /// 失败原因或提示。Failure reason or hint.
    pub note: Option<String>,
}

/// 已创建的虚拟麦克风；drop 时卸载模块。A created virtual mic; unloads its modules on drop.
pub struct VirtualMic {
    modules: Vec<u32>,
}

impl VirtualMic {
    /// 创建（或接管已有的）虚拟麦克风，并把本进程的播放指向它。仅 Linux 可用。
    /// Create (or adopt) the virtual mic and route this process's playback to it.
    /// Linux only.
    #[cfg(target_os = "linux")]
    pub fn setup() -> Result<Self, String> {
        let modules_list = pactl(&["list", "short", "modules"])?;
        let mut modules = find_our_modules(&modules_list);
        if modules.is_empty() {
            let sink = pactl(&[
                "load-module",
                "module-null-sink",
                &format!("sink_name={SINK_NAME}"),
                "sink_properties=device.description=SoundBridge-Mix",
            ])?;
            modules.extend(parse_module_id(&sink));
            let source = pactl(&[
                "load-module",
                "module-remap-source",
                &format!("master={SINK_NAME}.monitor"),
                &format!("source_name={SOURCE_NAME}"),
                &format!("source_properties=device.description={SOURCE_DESCRIPTION}"),
            ])?;
            modules.extend(parse_module_id(&source));
        } else {
            log::info!("复用已有的虚拟麦克风 reusing existing virtual mic modules {modules:?}");
        }
        // 必须在打开音频流之前设置。Must be set before any audio stream is opened.
        std::env::set_var("PULSE_SINK", SINK_NAME);
        std::env::set_var("PIPEWIRE_NODE", SINK_NAME);
        log::info!("已创建虚拟麦克风 virtual mic ready: {SOURCE_DESCRIPTION}");
        Ok(Self { modules })
    }

    /// 非 Linux 平台不支持。Unsupported off Linux.
    #[cfg(not(target_os = "linux"))]
    pub fn setup() -> Result<Self, String> {
        Err(tr!(
            "仅 Linux 支持自动创建虚拟麦克风",
            "Creating a virtual mic automatically is Linux-only"
        )
        .into())
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        // 后加载的先卸载（remap source 依赖 null sink）。Unload in reverse order.
        for id in self.modules.iter().rev() {
            let _ = std::process::Command::new("pactl")
                .args(["unload-module", &id.to_string()])
                .status();
        }
    }
}

/// 运行 `pactl` 并返回标准输出。Run `pactl` and return stdout.
#[cfg(target_os = "linux")]
fn pactl(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| {
            trf!(
                "找不到 pactl（请安装 pipewire-pulse 或 pulseaudio-utils）：{e}",
                "pactl not found (install pipewire-pulse or pulseaudio-utils): {e}"
            )
        })?;
    if !out.status.success() {
        return Err(trf!(
            "pactl {} 失败：{}",
            "pactl {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 解析 `pactl load-module` 输出的模块号。Parse the module id printed by `pactl load-module`.
pub fn parse_module_id(stdout: &str) -> Option<u32> {
    stdout.trim().parse().ok()
}

/// 从 `pactl list short modules` 中找出本程序创建的模块号（按加载顺序）。
/// Find the module ids we created in `pactl list short modules` (in load order).
pub fn find_our_modules(list: &str) -> Vec<u32> {
    let mut ids: Vec<u32> = list
        .lines()
        .filter(|l| {
            l.contains(&format!("sink_name={SINK_NAME}"))
                || l.contains(&format!("source_name={SOURCE_NAME}"))
        })
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect();
    ids.sort_unstable();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_module_ids() {
        assert_eq!(parse_module_id("536870913\n"), Some(536870913));
        assert_eq!(
            parse_module_id("Failure: Module initialization failed"),
            None
        );
    }

    #[test]
    fn finds_leftover_modules() {
        let list = "\
22\tmodule-always-sink\t\t
536870914\tmodule-remap-source\tmaster=soundbridge_mix.monitor source_name=soundbridge_mic source_properties=device.description=SoundBridge-Microphone\t
536870913\tmodule-null-sink\tsink_name=soundbridge_mix sink_properties=device.description=SoundBridge-Mix\t
30\tmodule-null-sink\tsink_name=other\t";
        assert_eq!(find_our_modules(list), vec![536870913, 536870914]);
        assert!(find_our_modules("").is_empty());
    }
}
