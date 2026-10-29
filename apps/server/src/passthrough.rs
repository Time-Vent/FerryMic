//! 「不拦截」组合键：只旁听键盘 / 鼠标，按键照样传给前台程序（游戏）。
//! "Pass-through" combos: only observe the keyboard / mouse; keys still reach the
//! foreground program (the game).
//!
//! 系统热键（`RegisterHotKey`）是独占的：触发时前台窗口收不到那个键，而且只允许
//! Ctrl / Alt / Shift / Win 当修饰键。这里改用 Windows 底层钩子（`WH_KEYBOARD_LL` /
//! `WH_MOUSE_LL`）：每个按键先经过我们「看一眼」，再原样交给系统，所以「对讲键 + 1」
//! 可以同时让 FerryMic 放音效、让游戏切武器；任意键（包括鼠标侧键）都能参与组合。
//! System hotkeys (`RegisterHotKey`) are exclusive: the foreground window doesn't get
//! the key, and only Ctrl / Alt / Shift / Win may be modifiers. Instead we use Windows
//! low-level hooks (`WH_KEYBOARD_LL` / `WH_MOUSE_LL`): every key passes by us first and
//! then goes on unchanged, so "push-to-talk + 1" can play a sound in FerryMic and switch
//! weapons in the game at once; any key (mouse side buttons included) can be part of a
//! combo.
//!
//! 组合的写法：按下顺序的按键名用 `+` 连接，最后一个是「触发键」，前面的必须在按下
//! 触发键时处于按住状态，例如 `KeyV+Digit1`、`Mouse4+Digit2`、`Ctrl+KeyQ`。按键名与
//! 浏览器 `KeyboardEvent.code` 一致；左右修饰键合并为 `Ctrl` / `Shift` / `Alt` / `Super`；
//! 鼠标为 `MouseMiddle` / `Mouse4` / `Mouse5`。额外按住别的键（比如奔跑时的 Shift）不影响触发。
//! Combo format: key names in press order joined by `+`; the last one is the trigger
//! and the others must be held when it's pressed, e.g. `KeyV+Digit1`, `Mouse4+Digit2`,
//! `Ctrl+KeyQ`. Key names follow the browser's `KeyboardEvent.code`; left / right
//! modifiers merge into `Ctrl` / `Shift` / `Alt` / `Super`; mouse buttons are
//! `MouseMiddle` / `Mouse4` / `Mouse5`. Holding extra keys (e.g. Shift to sprint)
//! doesn't prevent a match.
//!
//! 钩子回调必须极快（它挡在全系统的输入前面）：只查表、发通道，不加锁等待、不分配。
//! 管理员权限运行的程序获得焦点时，Windows 不把它的输入交给普通权限的钩子。
//! The hook callback must be very fast (it sits in front of all system input): table
//! lookup and a channel send only, no waiting on locks, no allocation. While a program
//! running as administrator has focus, Windows doesn't pass its input to a
//! non-elevated hook.

use std::sync::RwLock;

use soundboard::SoundId;

/// 按键编号：0x00–0xFF 为 Windows 虚拟键码；0x100 起为合并后的修饰键与鼠标键。
/// Key id: 0x00–0xFF are Windows virtual-key codes; from 0x100 merged modifiers and
/// mouse buttons.
pub type KeyId = u16;

/// 组合的签名：排序后的按住键 + 触发键（与按下顺序无关）。
/// A combo signature: sorted held keys + trigger (independent of press order).
pub type Signature = (Vec<KeyId>, KeyId);

const KEY_CTRL: KeyId = 0x100;
const KEY_SHIFT: KeyId = 0x101;
const KEY_ALT: KeyId = 0x102;
const KEY_SUPER: KeyId = 0x103;
const KEY_MOUSE_MIDDLE: KeyId = 0x110;
const KEY_MOUSE4: KeyId = 0x111;
const KEY_MOUSE5: KeyId = 0x112;
/// 按键编号上限（按下状态表的大小）。Key id bound (size of the pressed table).
const KEY_COUNT: usize = 0x120;

/// 一个组合：按住 `held`，再按 `trigger`。A combo: hold `held`, then press `trigger`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub held: Vec<KeyId>,
    pub trigger: KeyId,
}

impl Combo {
    /// 解析 `KeyV+Digit1` 这类写法。Parse a string like `KeyV+Digit1`.
    pub fn parse(s: &str) -> Result<Combo, String> {
        let mut keys = Vec::new();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            let id = key_id(part)
                .ok_or_else(|| trf!("无法识别的按键「{part}」", "Unrecognised key \"{part}\""))?;
            if keys.contains(&id) {
                return Err(trf!("按键「{part}」重复了", "Key \"{part}\" appears twice"));
            }
            keys.push(id);
        }
        let trigger = keys
            .pop()
            .ok_or_else(|| tr!("快捷键不能为空", "The hotkey can't be empty").to_string())?;
        if keys.len() > 3 {
            return Err(tr!("最多 4 个键的组合", "At most 4 keys per combo").into());
        }
        Ok(Combo {
            held: keys,
            trigger,
        })
    }

    /// 与顺序无关的签名，用于判断两个快捷键是否相同。
    /// An order-independent signature, for telling whether two hotkeys are the same.
    pub fn signature(&self) -> Signature {
        let mut held = self.held.clone();
        held.sort_unstable();
        (held, self.trigger)
    }

    /// 在给定的按下状态下，按下 `key` 是否触发这个组合。
    /// Whether pressing `key` triggers this combo given the pressed state.
    fn matches(&self, key: KeyId, pressed: &[bool; KEY_COUNT]) -> bool {
        self.trigger == key && self.held.iter().all(|&k| pressed[k as usize])
    }
}

/// 按键名 → 编号。Key name → id.
fn key_id(name: &str) -> Option<KeyId> {
    let simple = |vk: u16| Some(vk);
    if let Some(c) = name.strip_prefix("Key").filter(|c| c.len() == 1) {
        let ch = c.chars().next()?.to_ascii_uppercase();
        return ch.is_ascii_uppercase().then_some(ch as KeyId);
    }
    if let Some(d) = name.strip_prefix("Digit").filter(|d| d.len() == 1) {
        let ch = d.chars().next()?;
        return ch.is_ascii_digit().then_some(ch as KeyId);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1);
    }
    if let Some(d) = name
        .strip_prefix("Numpad")
        .and_then(|d| d.parse::<u16>().ok())
    {
        return (d <= 9).then_some(0x60 + d);
    }
    match name {
        "Ctrl" | "Control" | "ControlLeft" | "ControlRight" => simple(KEY_CTRL),
        "Shift" | "ShiftLeft" | "ShiftRight" => simple(KEY_SHIFT),
        "Alt" | "AltLeft" | "AltRight" => simple(KEY_ALT),
        "Super" | "Meta" | "MetaLeft" | "MetaRight" | "Win" => simple(KEY_SUPER),
        "MouseMiddle" => simple(KEY_MOUSE_MIDDLE),
        "Mouse4" => simple(KEY_MOUSE4),
        "Mouse5" => simple(KEY_MOUSE5),
        "Space" => simple(0x20),
        "Enter" | "NumpadEnter" => simple(0x0D),
        "Tab" => simple(0x09),
        "Backspace" => simple(0x08),
        "Escape" => simple(0x1B),
        "CapsLock" => simple(0x14),
        "Insert" => simple(0x2D),
        "Delete" => simple(0x2E),
        "Home" => simple(0x24),
        "End" => simple(0x23),
        "PageUp" => simple(0x21),
        "PageDown" => simple(0x22),
        "ArrowLeft" => simple(0x25),
        "ArrowUp" => simple(0x26),
        "ArrowRight" => simple(0x27),
        "ArrowDown" => simple(0x28),
        "Pause" => simple(0x13),
        "ScrollLock" => simple(0x91),
        "PrintScreen" => simple(0x2C),
        "NumpadMultiply" => simple(0x6A),
        "NumpadAdd" => simple(0x6B),
        "NumpadSubtract" => simple(0x6D),
        "NumpadDecimal" => simple(0x6E),
        "NumpadDivide" => simple(0x6F),
        "Minus" => simple(0xBD),
        "Equal" => simple(0xBB),
        "BracketLeft" => simple(0xDB),
        "BracketRight" => simple(0xDD),
        "Backslash" => simple(0xDC),
        "Semicolon" => simple(0xBA),
        "Quote" => simple(0xDE),
        "Backquote" => simple(0xC0),
        "Comma" => simple(0xBC),
        "Period" => simple(0xBE),
        "Slash" => simple(0xBF),
        _ => None,
    }
}

/// 钩子看到的虚拟键码 → 编号（左右修饰键合并）。Virtual-key code from the hook → id
/// (left / right modifiers merged).
#[cfg_attr(not(windows), allow(dead_code))]
fn key_from_vk(vk: u32) -> Option<KeyId> {
    Some(match vk {
        0x10 | 0xA0 | 0xA1 => KEY_SHIFT,
        0x11 | 0xA2 | 0xA3 => KEY_CTRL,
        0x12 | 0xA4 | 0xA5 => KEY_ALT,
        0x5B | 0x5C => KEY_SUPER,
        v if v < 0x100 => v as KeyId,
        _ => return None,
    })
}

/// 当前所有「不拦截」组合（钩子线程读，热键线程写）。
/// All current pass-through combos (read by the hook thread, written by the hotkey thread).
static COMBOS: RwLock<Vec<(SoundId, Combo)>> = RwLock::new(Vec::new());

/// 设置全部组合（覆盖）。Replace all combos.
pub fn set_combos(combos: Vec<(SoundId, Combo)>) {
    *COMBOS.write().unwrap() = combos;
}

/// 按下 `key` 时应触发的音效（写入 `out`，不分配）。Sounds to trigger when `key` is
/// pressed (written into `out`, no allocation).
#[cfg_attr(not(windows), allow(dead_code))]
fn matching(key: KeyId, pressed: &[bool; KEY_COUNT], out: &mut dyn FnMut(SoundId)) {
    // 拿不到读锁（正在改绑定）就跳过这一下，绝不在钩子里等。
    // If the read lock is busy (bindings being changed), skip this press; never wait in the hook.
    if let Ok(combos) = COMBOS.try_read() {
        for (id, combo) in combos.iter() {
            if combo.matches(key, pressed) {
                out(*id);
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    //! Windows 底层钩子。Windows low-level hooks.

    use std::cell::RefCell;
    use std::sync::OnceLock;

    use crossbeam_channel::Sender;
    use soundboard::SoundId;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
        WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
        WM_XBUTTONUP,
    };

    use super::{
        key_from_vk, matching, KeyId, KEY_COUNT, KEY_MOUSE4, KEY_MOUSE5, KEY_MOUSE_MIDDLE,
    };

    /// 触发请求发往这里（由转发线程调用 `Player::trigger`）。
    /// Trigger requests go here (a forwarding thread calls `Player::trigger`).
    static SENDER: OnceLock<Sender<SoundId>> = OnceLock::new();
    /// 钩子线程是否已启动。Whether the hook thread has started.
    static STARTED: OnceLock<()> = OnceLock::new();

    thread_local! {
        /// 当前按住的键（只在钩子线程访问）。Keys currently held (hook thread only).
        static PRESSED: RefCell<[bool; KEY_COUNT]> = const { RefCell::new([false; KEY_COUNT]) };
    }

    /// 处理一次按下 / 松开。按住不放时系统会重复发「按下」，只有第一次算。
    /// Handle one press / release. Holding a key repeats "down"; only the first counts.
    fn on_key(key: KeyId, down: bool) {
        PRESSED.with(|p| {
            let mut pressed = p.borrow_mut();
            let slot = &mut pressed[key as usize];
            if down {
                if *slot {
                    return; // 自动重复。Auto-repeat.
                }
                *slot = true;
                if let Some(tx) = SENDER.get() {
                    matching(key, &pressed, &mut |id| {
                        let _ = tx.try_send(id);
                    });
                }
            } else {
                *slot = false;
            }
        });
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY：HC_ACTION 时 lparam 指向系统提供的 KBDLLHOOKSTRUCT，回调期间有效。
            // SAFETY: with HC_ACTION, lparam points to a system-provided KBDLLHOOKSTRUCT,
            // valid for the duration of the callback.
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if down || up {
                if let Some(key) = key_from_vk(info.vkCode) {
                    on_key(key, down);
                }
            }
        }
        // 永远交给下一个钩子：不拦截。Always pass on: never swallow.
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let msg = wparam as u32;
            // 鼠标移动最频繁，先按消息类型过滤，尽快返回。
            // Mouse moves are the most frequent; filter by message first and return fast.
            let event = match msg {
                WM_MBUTTONDOWN => Some((KEY_MOUSE_MIDDLE, true)),
                WM_MBUTTONUP => Some((KEY_MOUSE_MIDDLE, false)),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    // SAFETY：同上，lparam 指向 MSLLHOOKSTRUCT。SAFETY: as above, MSLLHOOKSTRUCT.
                    let info = &*(lparam as *const MSLLHOOKSTRUCT);
                    let key = if (info.mouseData >> 16) == 2 {
                        KEY_MOUSE5
                    } else {
                        KEY_MOUSE4
                    };
                    Some((key, msg == WM_XBUTTONDOWN))
                }
                _ => None,
            };
            if let Some((key, down)) = event {
                on_key(key, down);
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// 启动钩子线程（只启动一次）。Start the hook thread (once).
    pub fn start(tx: Sender<SoundId>) -> Result<(), String> {
        if STARTED.get().is_some() {
            return Ok(());
        }
        let _ = SENDER.set(tx);
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
        std::thread::Builder::new()
            .name("soundbridge-combo-hook".into())
            .spawn(move || {
                // SAFETY：在本线程安装钩子并用 GetMessageW 阻塞分发消息（底层钩子的回调由
                // 安装它的线程在取消息时执行，所以这里必须一直取消息）。MSG 全零是合法初值。
                // SAFETY: install the hooks on this thread and dispatch messages with a
                // blocking GetMessageW (low-level hook callbacks run on the installing
                // thread while it retrieves messages, so it must keep doing so). An
                // all-zero MSG is a valid initial value.
                unsafe {
                    let module = GetModuleHandleW(std::ptr::null());
                    let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
                    if kb.is_null() || mouse.is_null() {
                        let _ = ready_tx.send(Err(tr!(
                            "无法安装键盘 / 鼠标钩子",
                            "Can't install the keyboard / mouse hooks"
                        )
                        .into()));
                        return;
                    }
                    let _ = ready_tx.send(Ok(()));
                    log::info!("不拦截组合键已启用 pass-through combos active");
                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let result = ready_rx
            .recv()
            .unwrap_or_else(|_| Err("hook thread died".into()));
        if result.is_ok() {
            let _ = STARTED.set(());
        }
        result
    }
}

#[cfg(not(windows))]
mod imp {
    use crossbeam_channel::Sender;
    use soundboard::SoundId;

    pub fn start(_tx: Sender<SoundId>) -> Result<(), String> {
        Err(tr!(
            "「不拦截」组合键目前只支持 Windows",
            "Pass-through combos are only supported on Windows for now"
        )
        .into())
    }
}

/// 启动钩子（只在第一次有「不拦截」组合时调用）。触发请求发往 `tx`。
/// Start the hooks (called when the first pass-through combo appears). Trigger
/// requests go to `tx`.
pub use imp::start;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_holds_and_mouse_buttons() {
        let c = Combo::parse("KeyV+Digit1").unwrap();
        assert_eq!((c.held, c.trigger), (vec![b'V' as KeyId], b'1' as KeyId));
        let c = Combo::parse("Mouse4+F13").unwrap();
        assert_eq!((c.held, c.trigger), (vec![KEY_MOUSE4], 0x7C));
        let c = Combo::parse("ControlLeft+Alt+KeyQ").unwrap();
        assert_eq!(c.held, vec![KEY_CTRL, KEY_ALT]);
    }

    #[test]
    fn rejects_bad_combos() {
        assert!(Combo::parse("").is_err());
        assert!(Combo::parse("KeyV+Nope").is_err());
        assert!(Combo::parse("KeyV+KeyV").is_err());
        assert!(Combo::parse("F25").is_err());
        assert!(Combo::parse("KeyA+KeyB+KeyC+KeyD+KeyE").is_err());
    }

    #[test]
    fn signature_ignores_hold_order() {
        let a = Combo::parse("Ctrl+Alt+KeyQ").unwrap();
        let b = Combo::parse("Alt+Ctrl+KeyQ").unwrap();
        assert_eq!(a.signature(), b.signature());
        assert_ne!(
            a.signature(),
            Combo::parse("Ctrl+KeyQ").unwrap().signature()
        );
    }

    #[test]
    fn matches_only_with_held_keys_down() {
        let c = Combo::parse("KeyV+Digit1").unwrap();
        let mut pressed = [false; KEY_COUNT];
        assert!(!c.matches(b'1' as KeyId, &pressed)); // 没按住 V。V not held.
        pressed[b'V' as usize] = true;
        assert!(c.matches(b'1' as KeyId, &pressed));
        pressed[KEY_SHIFT as usize] = true; // 额外按住 Shift 也行。Extra Shift is fine.
        assert!(c.matches(b'1' as KeyId, &pressed));
        assert!(!c.matches(b'2' as KeyId, &pressed));
    }

    #[test]
    fn hook_codes_merge_left_and_right() {
        assert_eq!(key_from_vk(0xA2), Some(KEY_CTRL));
        assert_eq!(key_from_vk(0xA3), Some(KEY_CTRL));
        assert_eq!(key_from_vk(0xA1), Some(KEY_SHIFT));
        assert_eq!(key_from_vk(b'V' as u32), key_id("KeyV"));
    }
}
