//! 全局热键：系统级快捷键（游戏全屏也能触发）→ 触发音效。
//! Global hotkeys: system-wide shortcuts (work even in fullscreen games) → trigger
//! sounds.
//!
//! 依赖 [`global_hotkey`]（Windows / macOS / Linux X11）。快捷键字符串直接由
//! `HotKey::from_str` 解析（支持 "F1"、"Ctrl+Alt+1"、"shift+alt+KeyQ" 等）。
//! Backed by [`global_hotkey`] (Windows / macOS / Linux X11). Hotkey strings are
//! parsed by `HotKey::from_str` (supports "F1", "Ctrl+Alt+1", "shift+alt+KeyQ", etc).
//!
//! 线程拓扑：
//! Thread topology:
//! - 热键线程：GlobalHotKeyManager 事件循环，接收注册/注销指令，热键按下时向
//!   音频命令队列发 `Trigger`。
//!   Hotkey thread: GlobalHotKeyManager event loop, receives register/unregister
//!   commands, sends `Trigger` to the audio command queue on hotkey press.

use std::collections::HashMap;

use crossbeam_channel::{Receiver, Sender};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use soundboard::{BoardCommand, SoundId};

/// 热键线程指令。Hotkey thread commands.
pub enum HotkeyCommand {
    /// 注册：绑定某音效到某快捷键。Register: bind a sound to a hotkey.
    Register { sound_id: SoundId, hotkey: String },
    /// 注销某音效的所有热键。Unregister all hotkeys of a sound.
    Unregister { sound_id: SoundId },
}

/// 启动热键线程。Start the hotkey thread.
///
/// `cmd_rx` 接收注册/注销指令；`cmd_tx` 是音频命令队列（热键触发时发 `Trigger`）。
/// `cmd_rx` receives register/unregister commands; `cmd_tx` is the audio command
/// queue (receives `Trigger` on hotkey press).
pub fn spawn_hotkey_thread(cmd_rx: Receiver<HotkeyCommand>, cmd_tx: Sender<BoardCommand>) {
    std::thread::spawn(move || {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                log::error!("全局热键管理器初始化失败 failed to init hotkey manager: {e}");
                return;
            }
        };

        // hotkey_id → (HotKey, sound_id)。保存 HotKey 以便注销。
        // hotkey_id → (HotKey, sound_id). Keep HotKey so it can be unregistered.
        let mut map: HashMap<u32, (HotKey, SoundId)> = HashMap::new();

        loop {
            // 1. 处理注册/注销指令。1. Process register/unregister commands.
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    HotkeyCommand::Register { sound_id, hotkey } => {
                        match hotkey.parse::<HotKey>() {
                            Ok(hk) => {
                                let id = hk.id();
                                match manager.register(hk) {
                                    Ok(()) => {
                                        map.insert(id, (hk, sound_id));
                                        log::info!(
                                            "已注册热键 registered hotkey: {hotkey} → sound {sound_id}"
                                        );
                                    }
                                    Err(e) => {
                                        log::warn!("注册热键失败 failed to register {hotkey}: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("无法解析快捷键 cannot parse hotkey: {hotkey} ({e})");
                            }
                        }
                    }
                    HotkeyCommand::Unregister { sound_id } => {
                        let ids: Vec<u32> = map
                            .iter()
                            .filter(|(_, (_, s))| *s == sound_id)
                            .map(|(k, _)| *k)
                            .collect();
                        for id in ids {
                            if let Some((hk, _)) = map.remove(&id) {
                                let _ = manager.unregister(hk);
                            }
                        }
                    }
                }
            }

            // 2. 处理热键事件。2. Process hotkey events.
            if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == HotKeyState::Pressed {
                    if let Some((_, sound_id)) = map.get(&event.id) {
                        let _ = cmd_tx.send(BoardCommand::Trigger { sound_id: *sound_id });
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn parse_f1() {
        let hk: HotKey = "F1".parse().unwrap();
        assert_eq!(hk.key, Code::F1);
        assert!(hk.mods.is_empty());
    }

    #[test]
    fn parse_ctrl_alt_1() {
        let hk: HotKey = "Ctrl+Alt+1".parse().unwrap();
        assert!(hk.mods.contains(Modifiers::CONTROL));
        assert!(hk.mods.contains(Modifiers::ALT));
        assert_eq!(hk.key, Code::Digit1);
    }

    #[test]
    fn parse_invalid_returns_err() {
        assert!("".parse::<HotKey>().is_err());
        assert!("F25".parse::<HotKey>().is_err());
    }
}
