//! 用户设置持久化：麦克风选择与音量、手机遥控开关。
//! User settings persistence: mic choice and volume, phone-remote switch.
//!
//! 存在数据目录下的 `settings.json`（见 [`data_dir`]）。读失败用默认值，写失败只记警告，
//! 不影响运行。
//! Stored as `settings.json` in the data directory (see [`data_dir`]). Read failures
//! fall back to defaults; write failures only log a warning.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 数据目录（存放 `settings.json`，以及默认的音效库 `sounds/`）。
/// The data directory (holds `settings.json` and the default sound library `sounds/`).
///
/// - Windows：`%APPDATA%\SoundBridge`
/// - Linux：`$XDG_DATA_HOME/SoundBridge`，默认 `~/.local/share/SoundBridge`
/// - macOS：`~/Library/Application Support/SoundBridge`
///
/// 可用环境变量 `SOUNDBRIDGE_DATA_DIR` 覆盖（测试用）。
/// Overridable with `SOUNDBRIDGE_DATA_DIR` (for tests).
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SOUNDBRIDGE_DATA_DIR") {
        return PathBuf::from(p);
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local").join("share")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("SoundBridge")
}

/// 外观模式。Appearance mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// 跟随系统。Follow the system.
    #[default]
    System,
    Dark,
    Light,
}

/// 点窗口关闭按钮时的行为（仅桌面 App）。What the window's close button does (app only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseAction {
    /// 每次询问。Ask every time.
    #[default]
    Ask,
    /// 最小化到托盘（音效与快捷键继续可用）。Minimize to the tray (sounds and hotkeys keep working).
    Tray,
    /// 退出。Quit.
    Quit,
}

/// 默认主题色（紫）。Default accent colour (violet).
pub const DEFAULT_ACCENT: &str = "#7b6cff";

/// 持久化的设置项。Persisted settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 麦克风设备名；`None` 表示跟随系统默认。Mic device name; `None` = follow default.
    pub mic_device: Option<String>,
    /// 麦克风增益（dB）。Mic gain (dB).
    pub mic_gain_db: f32,
    /// 输出设备（混音发送到哪里，通常是虚拟音频线）；`None` = 系统默认。
    /// Output device (where the mix is sent, usually a virtual cable); `None` = default.
    pub output_device: Option<String>,
    /// 试听：音效同时放一份到耳机 / 扬声器，自己也能听到。Monitor: also play effects locally.
    pub monitor_enabled: bool,
    /// 试听设备；`None` = 系统默认播放设备。Monitor device; `None` = default playback.
    pub monitor_device: Option<String>,
    /// 试听音量（dB）。Monitor volume (dB).
    pub monitor_gain_db: f32,
    /// 手机遥控开关（开关立即生效，并记住到下次启动）。默认关闭。
    /// Phone-remote switch (takes effect immediately and is remembered). Off by default.
    pub lan_remote: bool,
    /// 手机遥控访问码（首次开启时生成）。Phone-remote access code (generated on first use).
    pub remote_token: Option<String>,
    /// 音效库根目录；`None` = 数据目录下的 `sounds/`。
    /// Sound library root; `None` = `sounds/` under the data directory.
    pub library_dir: Option<PathBuf>,
    /// 外观模式。Appearance mode.
    pub theme_mode: ThemeMode,
    /// 主题色（`#rrggbb`）。Accent colour (`#rrggbb`).
    pub accent: String,
    /// App 图标（任务栏 / 托盘）跟随主题色。App icon (taskbar / tray) follows the accent.
    pub tint_app_icon: bool,
    /// 关闭窗口时的行为。What closing the window does.
    pub close_action: CloseAction,
    /// 界面语言：`system`（跟随系统）/ `zh-CN` / `en`。
    /// UI language: `system` (follow the OS) / `zh-CN` / `en`.
    pub language: String,
    /// Linux：启动时自动创建虚拟麦克风（PipeWire / PulseAudio）。其它平台忽略。
    /// Linux: create a virtual microphone at start-up (PipeWire / PulseAudio). Ignored
    /// elsewhere.
    pub auto_virtual_mic: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mic_device: None,
            mic_gain_db: 0.0,
            output_device: None,
            monitor_enabled: true,
            monitor_device: None,
            monitor_gain_db: 0.0,
            lan_remote: false,
            remote_token: None,
            library_dir: None,
            theme_mode: ThemeMode::System,
            accent: DEFAULT_ACCENT.into(),
            tint_app_icon: true,
            close_action: CloseAction::Ask,
            language: "system".into(),
            auto_virtual_mic: true,
        }
    }
}

impl Settings {
    /// 实际使用的音效库根目录。The effective sound library root.
    pub fn library_root(&self) -> PathBuf {
        self.library_dir
            .clone()
            .unwrap_or_else(|| data_dir().join("sounds"))
    }
}

/// 校验 `#rrggbb` 颜色。Validate a `#rrggbb` colour.
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// 设置存储：内存副本 + 文件。Settings store: in-memory copy + file.
pub struct SettingsStore {
    path: PathBuf,
    data: Mutex<Settings>,
}

impl SettingsStore {
    /// 从默认位置加载。Load from the default location.
    pub fn load_default() -> Self {
        Self::load(default_path())
    }

    /// 从指定文件加载（不存在或损坏时用默认值）。
    /// Load from a file (defaults when missing or corrupt).
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            data: Mutex::new(data),
        }
    }

    /// 当前设置的副本。A copy of the current settings.
    pub fn get(&self) -> Settings {
        self.data.lock().unwrap().clone()
    }

    /// 修改并保存（内容不变时不写盘）。Modify and save (skips the write if unchanged).
    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        let mut data = self.data.lock().unwrap();
        let before = data.clone();
        f(&mut data);
        if *data == before {
            return;
        }
        let result = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| {
                std::fs::write(
                    &self.path,
                    serde_json::to_string_pretty(&*data).unwrap_or_default(),
                )
            });
        if let Err(e) = result {
            log::warn!(
                "保存设置失败 failed to save settings to {}: {e}",
                self.path.display()
            );
        }
    }
}

fn default_path() -> PathBuf {
    data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_file() {
        let path = std::env::temp_dir()
            .join("soundbridge-settings-test")
            .join("settings.json");
        let _ = std::fs::remove_file(&path);

        let store = SettingsStore::load(path.clone());
        assert_eq!(store.get(), Settings::default());
        store.update(|s| {
            s.mic_device = Some("Headset (Hands-Free)".into());
            s.mic_gain_db = 6.0;
        });

        let reloaded = SettingsStore::load(path.clone());
        assert_eq!(
            reloaded.get().mic_device.as_deref(),
            Some("Headset (Hands-Free)")
        );
        assert_eq!(reloaded.get().mic_gain_db, 6.0);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join("soundbridge-settings-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(SettingsStore::load(path).get(), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_settings_files_get_new_defaults() {
        // 旧版设置文件没有新字段：应补上默认值。Old files lack new fields: defaults apply.
        let s: Settings = serde_json::from_str(r#"{"mic_device":"X","mic_gain_db":3.0}"#).unwrap();
        assert_eq!(s.mic_device.as_deref(), Some("X"));
        assert_eq!(s.accent, DEFAULT_ACCENT);
        assert_eq!(s.theme_mode, ThemeMode::System);
        assert!(s.auto_virtual_mic);
    }

    #[test]
    fn validates_hex_colors() {
        assert!(is_hex_color("#7b6cff"));
        assert!(!is_hex_color("7b6cff"));
        assert!(!is_hex_color("#7b6cfz"));
        assert!(!is_hex_color("#fff"));
    }
}
