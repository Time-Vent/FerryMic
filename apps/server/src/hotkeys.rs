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
//! - 热键线程：创建 `GlobalHotKeyManager` 并在**同一线程**跑 Win32 消息循环（Windows 上
//!   `WM_HOTKEY` 只会投递到创建它的线程，没有消息循环热键就永远不会触发）；处理绑定请求，
//!   热键按下时向音频命令队列发 `Trigger`。
//!   Hotkey thread: creates the `GlobalHotKeyManager` and runs a Win32 message loop
//!   **on the same thread** (on Windows `WM_HOTKEY` is only delivered to the creating
//!   thread; without a message loop hotkeys never fire); handles bind requests and
//!   sends `Trigger` to the audio command queue on a hotkey press.
//! - 「不拦截」组合（按键照样传给游戏）走 [`crate::passthrough`] 的底层钩子，由钩子线程发现、
//!   转发线程触发；绑定表仍由这里统一管理，两种方式之间也检查冲突。
//!   Pass-through combos (keys still reach the game) use the low-level hooks in
//!   [`crate::passthrough`], detected on the hook thread and triggered by a forwarding
//!   thread; the binding table is still managed here, with conflicts checked across
//!   both kinds.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use soundboard::SoundId;

use crate::loader::Player;
use crate::passthrough::{self, Combo, Signature};

/// 热键线程指令。Hotkey thread command.
pub enum HotkeyCommand {
    /// 把音效绑定到快捷键（`None` = 解绑）。`passthrough` 为真时用「不拦截」组合
    /// （按键照样传给前台程序，见 [`crate::passthrough`]），否则用系统热键。
    /// 先注册新键成功后才释放旧键，失败时保留旧键。结果经 `reply` 返回（可为 `None`）。
    /// Bind a sound to a hotkey (`None` = unbind). With `passthrough` it's a
    /// pass-through combo (keys still reach the foreground program, see
    /// [`crate::passthrough`]), otherwise a system hotkey. The new key is registered
    /// before the old one is released, so a failure keeps the old key. The result goes
    /// to `reply` (optional).
    Bind {
        sound_id: SoundId,
        hotkey: Option<String>,
        passthrough: bool,
        reply: Option<Sender<Result<(), String>>>,
    },
}

/// 启动热键线程。Start the hotkey thread.
///
/// `cmd_rx` 接收绑定指令；热键按下时调用 `player.trigger`（未加载的音效会先加载）。
/// `cmd_rx` receives bind commands; a hotkey press calls `player.trigger` (unloaded
/// sounds are loaded first).
pub fn spawn_hotkey_thread(cmd_rx: Receiver<HotkeyCommand>, player: Arc<Player>) {
    // 「不拦截」组合由钩子线程发现，经通道交给这个转发线程去触发（钩子里不能做慢事）。
    // Pass-through combos are detected on the hook thread and handed over a channel to
    // this forwarding thread to trigger (the hook must not do slow work).
    let (combo_tx, combo_rx) = crossbeam_channel::unbounded::<SoundId>();
    {
        let player = player.clone();
        std::thread::Builder::new()
            .name("soundbridge-combo".into())
            .spawn(move || {
                for id in combo_rx {
                    player.trigger(id);
                }
            })
            .expect("spawn combo thread");
    }

    std::thread::Builder::new()
        .name("soundbridge-hotkeys".into())
        .spawn(move || {
            let manager = match GlobalHotKeyManager::new() {
                Ok(m) => m,
                Err(e) => {
                    log::error!("全局热键管理器初始化失败 failed to init hotkey manager: {e}");
                    // 仍然回复请求，避免 API 一直等待。Still answer requests so the API
                    // doesn't hang.
                    for cmd in cmd_rx {
                        let HotkeyCommand::Bind { reply, .. } = cmd;
                        if let Some(r) = reply {
                            let _ = r.send(Err(tr!(
                                "全局热键不可用",
                                "Global hotkeys are unavailable"
                            )
                            .into()));
                        }
                    }
                    return;
                }
            };
            let mut bindings = Bindings::new(combo_tx);

            loop {
                // 1. 处理绑定指令。1. Handle bind commands.
                while let Ok(cmd) = cmd_rx.try_recv() {
                    let HotkeyCommand::Bind {
                        sound_id,
                        hotkey,
                        passthrough,
                        reply,
                    } = cmd;
                    let result = bindings.bind(&manager, sound_id, hotkey.as_deref(), passthrough);
                    if let Err(e) = &result {
                        log::warn!("绑定热键失败 failed to bind hotkey for sound {sound_id}: {e}");
                    }
                    if let Some(r) = reply {
                        let _ = r.send(result);
                    }
                }

                // 2. 分发 Win32 消息（WM_HOTKEY 由此转成热键事件）。
                // 2. Dispatch Win32 messages (this turns WM_HOTKEY into hotkey events).
                pump_messages();

                // 3. 处理热键事件。3. Handle hotkey events.
                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state == HotKeyState::Pressed {
                        if let Some(sound_id) = bindings.sound_for(event.id) {
                            player.trigger(sound_id);
                        }
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
        .expect("spawn hotkey thread");
}

/// 一个音效的绑定。A sound's binding.
#[derive(Clone)]
enum Binding {
    /// 系统热键：独占，前台程序收不到。System hotkey: exclusive, the foreground program
    /// doesn't get it.
    System(HotKey),
    /// 不拦截组合：只旁听。Pass-through combo: observe only.
    Combo(Combo),
}

impl Binding {
    fn same_as(&self, other: &Binding) -> bool {
        match (self, other) {
            (Binding::System(a), Binding::System(b)) => a.id() == b.id(),
            (Binding::Combo(a), Binding::Combo(b)) => a.signature() == b.signature(),
            _ => false,
        }
    }
}

/// 热键绑定表。Hotkey binding table.
struct Bindings {
    /// sound_id → 绑定 + 与顺序无关的签名（用于两种方式之间的冲突检查）。
    /// sound_id → binding + order-independent signature (for conflicts across both kinds).
    by_sound: HashMap<SoundId, (Binding, Option<Signature>)>,
    /// 交给钩子的触发通道。Trigger channel handed to the hook.
    combo_tx: Sender<SoundId>,
}

impl Bindings {
    fn new(combo_tx: Sender<SoundId>) -> Self {
        Self {
            by_sound: HashMap::new(),
            combo_tx,
        }
    }

    fn sound_for(&self, hotkey_id: u32) -> Option<SoundId> {
        self.by_sound.iter().find_map(|(s, (b, _))| match b {
            Binding::System(hk) if hk.id() == hotkey_id => Some(*s),
            _ => None,
        })
    }

    fn bind(
        &mut self,
        manager: &GlobalHotKeyManager,
        sound_id: SoundId,
        hotkey: Option<&str>,
        passthrough: bool,
    ) -> Result<(), String> {
        let text = hotkey.map(str::trim).filter(|s| !s.is_empty());
        let new = match text {
            None => None,
            Some(s) if passthrough => Some(Binding::Combo(Combo::parse(s)?)),
            Some(s) => Some(Binding::System(s.parse::<HotKey>().map_err(|e| {
                trf!(
                    "无法识别快捷键「{s}」：{e}",
                    "Unrecognised hotkey \"{s}\": {e}"
                )
            })?)),
        };
        let old = self.by_sound.get(&sound_id).map(|(b, _)| b.clone());
        match (&new, &old) {
            (None, None) => return Ok(()),
            (Some(n), Some(o)) if n.same_as(o) => return Ok(()), // 没变化。Unchanged.
            _ => {}
        }

        // 与其它音效冲突（两种方式之间也算）。Conflicts with other sounds (across both kinds).
        let signature = text
            .and_then(|s| Combo::parse(s).ok())
            .map(|c| c.signature());
        if let Some(sig) = &signature {
            let other = self
                .by_sound
                .iter()
                .find(|(id, (_, s))| **id != sound_id && s.as_ref() == Some(sig))
                .map(|(id, _)| *id);
            if let Some(other) = other {
                return Err(trf!(
                    "该快捷键已被音效 #{other} 使用",
                    "This hotkey is already used by sound #{other}"
                ));
            }
        }

        // 先让新的生效，再释放旧的。Activate the new one first, then release the old one.
        match &new {
            Some(Binding::System(hk)) => manager.register(*hk).map_err(|e| {
                trf!(
                    "快捷键已被其他程序占用或不可用：{e}",
                    "The hotkey is taken by another program or unavailable: {e}"
                )
            })?,
            Some(Binding::Combo(_)) => passthrough::start(self.combo_tx.clone())?,
            None => {}
        }
        if let Some(Binding::System(hk)) = old {
            let _ = manager.unregister(hk);
        }
        match new {
            Some(b) => self.by_sound.insert(sound_id, (b, signature)),
            None => self.by_sound.remove(&sound_id),
        };
        self.publish_combos();
        Ok(())
    }

    /// 把当前所有「不拦截」组合交给钩子。Hand all current pass-through combos to the hook.
    fn publish_combos(&self) {
        let combos = self
            .by_sound
            .iter()
            .filter_map(|(id, (b, _))| match b {
                Binding::Combo(c) => Some((*id, c.clone())),
                Binding::System(_) => None,
            })
            .collect();
        passthrough::set_combos(combos);
    }
}

/// 分发本线程的所有待处理 Win32 消息（非阻塞）。
/// Dispatch all pending Win32 messages of this thread (non-blocking).
#[cfg(windows)]
fn pump_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    // SAFETY：MSG 是纯数据结构，全零是合法初值；三个 API 只读写这个本地变量，
    // 且只处理本线程（创建热键窗口的线程）的消息队列。
    // SAFETY: MSG is plain data and all-zero is a valid initial value; the three APIs
    // only touch this local and this thread's queue (the one owning the hotkey window).
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 非 Windows 平台由 global-hotkey 内部处理（Linux X11 自有线程；macOS 需主线程事件循环）。
/// On other platforms global-hotkey handles it (Linux X11 has its own thread; macOS
/// needs the main-thread event loop).
#[cfg(not(windows))]
fn pump_messages() {}

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
