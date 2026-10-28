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

/// 数据目录：`%APPDATA%\SoundBridge`，可用环境变量 `SOUNDBRIDGE_DATA_DIR` 覆盖（测试用）。
/// 存放 `settings.json`、`library.json` 和导入的音效文件 `sounds/`。
/// Data directory: `%APPDATA%\SoundBridge`, overridable with `SOUNDBRIDGE_DATA_DIR`
/// (for tests). Holds `settings.json`, `library.json` and imported sounds in `sounds/`.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SOUNDBRIDGE_DATA_DIR") {
        return PathBuf::from(p);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("SoundBridge")
}

/// 持久化的设置项。Persisted settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 麦克风设备名；`None` 表示跟随系统默认。Mic device name; `None` = follow default.
    pub mic_device: Option<String>,
    /// 麦克风增益（dB）。Mic gain (dB).
    pub mic_gain_db: f32,
    /// 是否允许手机在局域网遥控（监听 0.0.0.0）。默认关闭，只监听本机。重启后生效。
    /// Allow phone remote over the LAN (listen on 0.0.0.0). Off by default (localhost
    /// only). Takes effect after restart.
    pub lan_remote: bool,
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
}
