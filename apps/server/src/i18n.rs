//! 界面语言：中文 / 英文。后端返回给界面的提示与错误按当前语言生成。
//! UI language: Chinese / English. Messages and errors returned to the UI are produced
//! in the current language.
//!
//! 设置里保存的是偏好（`system` / `zh-CN` / `en`），这里只记住解析后的结果（是否英文），
//! 用一个原子量，任何线程都能直接读。日志保持中英双语，不受影响。
//! Settings store the preference (`system` / `zh-CN` / `en`); here we only keep the
//! resolved result (English or not) in an atomic readable from any thread. Logs stay
//! bilingual and are unaffected.

use std::sync::atomic::{AtomicBool, Ordering};

/// 当前是否英文。Whether the current language is English.
static ENGLISH: AtomicBool = AtomicBool::new(false);

/// 支持的语言偏好。Supported language preferences.
pub const LANGUAGES: &[&str] = &["system", "zh-CN", "en"];

/// 把偏好解析成实际语言：`zh-CN` 或 `en`。`system` 看操作系统语言，中文系统用中文，
/// 其它一律英文。
/// Resolve a preference to an actual language: `zh-CN` or `en`. `system` follows the OS
/// language: Chinese for Chinese systems, English for everything else.
pub fn resolve(pref: &str) -> &'static str {
    match pref {
        "zh-CN" => "zh-CN",
        "en" => "en",
        _ => match sys_locale::get_locale() {
            Some(l) if l.to_ascii_lowercase().starts_with("zh") => "zh-CN",
            Some(_) => "en",
            None => "zh-CN",
        },
    }
}

/// 应用语言偏好，返回实际语言。Apply a language preference; returns the actual language.
pub fn apply(pref: &str) -> &'static str {
    let lang = resolve(pref);
    ENGLISH.store(lang == "en", Ordering::Relaxed);
    lang
}

/// 当前实际语言。The current actual language.
pub fn current() -> &'static str {
    if is_en() {
        "en"
    } else {
        "zh-CN"
    }
}

/// 当前是否英文。Whether the current language is English.
pub fn is_en() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// 按当前语言二选一（`&'static str`）。Pick by the current language (`&'static str`).
///
/// ```ignore
/// tr!("音效不存在", "Sound not found")
/// ```
#[macro_export]
macro_rules! tr {
    ($zh:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_en() {
            $en
        } else {
            $zh
        }
    };
}

/// 按当前语言格式化（`String`），参数写法同 `format!`。
/// Format in the current language (`String`); arguments as in `format!`.
///
/// ```ignore
/// trf!("无法打开「{name}」：{e}", "Can't open \"{name}\": {e}")
/// ```
#[macro_export]
macro_rules! trf {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_en() {
            format!($en $(, $arg)*)
        } else {
            format!($zh $(, $arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_resolve_directly() {
        assert_eq!(resolve("zh-CN"), "zh-CN");
        assert_eq!(resolve("en"), "en");
        assert!(["zh-CN", "en"].contains(&resolve("system")));
    }

    #[test]
    fn macros_pick_by_language() {
        let name = "a";
        apply("en");
        assert_eq!(tr!("中", "en"), "en");
        assert_eq!(trf!("中 {name}", "en {name}"), "en a");
        apply("zh-CN");
        assert_eq!(tr!("中", "en"), "中");
        assert_eq!(trf!("中 {}", "en {}", 1), "中 1");
    }
}
