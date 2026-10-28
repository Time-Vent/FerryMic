//! SoundBridge 桌面 App：Tauri 外壳 + 进程内的 SoundBridge 服务。
//! SoundBridge desktop app: Tauri shell + the SoundBridge service in-process.
//!
//! 启动流程：
//! Start-up:
//! 1. 单实例检查：已在运行则把旧窗口提到前台后退出（避免两份音频输出）。
//!    Single-instance check: if already running, focus that window and exit (avoids
//!    two audio outputs).
//! 2. 在专用线程上跑 tokio 运行时 + `soundbridge_server::run`（音频、热键、HTTP API）。
//!    Run a tokio runtime + `soundbridge_server::run` (audio, hotkeys, HTTP API) on a
//!    dedicated thread.
//! 3. 服务就绪后，用原生窗口打开 `http://127.0.0.1:<端口>`，界面与浏览器版完全相同；
//!    同时创建托盘图标。
//!    Once ready, open `http://127.0.0.1:<port>` in a native window (same UI as the
//!    browser version) and create the tray icon.
//!
//! 关闭窗口：按设置最小化到托盘 / 退出 / 询问。「询问」由界面里的对话框完成：外壳请服务
//! 端打开询问标记，界面轮询到后弹框，选好后经 HTTP 回调 [`WindowAction`]。
//! Closing the window: minimize to tray / quit / ask, per settings. "Ask" uses a
//! dialog in the UI: the shell raises a flag in the service, the UI picks it up while
//! polling and shows the dialog, and the answer comes back as a [`WindowAction`].
//!
//! 界面通过 HTTP 访问本机服务，不使用 Tauri IPC，因此无需开放任何 Tauri 权限。
//! The UI talks to the local service over HTTP, not Tauri IPC, so no Tauri
//! permissions need to be granted.

// 发布版不弹控制台窗口。No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use soundbridge_server::{
    CloseAction, Control, Hooks, ServerConfig, ServerInfo, Settings, WindowAction,
};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 主窗口标签。Main window label.
const MAIN: &str = "main";
/// 托盘图标 id。Tray icon id.
const TRAY: &str = "tray";
/// 音效资源预览窗口标签。Sound-site preview window label.
const BROWSE: &str = "browse";
/// 默认主题色的色相（图标原图的主色）。Hue of the default accent (the icon's base hue).
const BASE_HUE: f32 = 246.1;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        // 第二次启动：显示已有窗口。Second launch: show the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .setup(|app| {
            let handle = app.handle().clone();
            let hooks = Hooks {
                on_settings: Some(Arc::new({
                    let h = handle.clone();
                    move |s: &Settings| {
                        let (h2, accent, tint) = (h.clone(), s.accent.clone(), s.tint_app_icon);
                        let _ = h.run_on_main_thread(move || {
                            apply_icon(&h2, &accent, tint);
                            // 语言可能变了：重建托盘菜单。The language may have changed.
                            apply_tray_menu(&h2);
                        });
                    }
                })),
                on_window_action: Some(Arc::new({
                    let h = handle.clone();
                    move |a: WindowAction| {
                        let h2 = h.clone();
                        let _ = h.run_on_main_thread(move || match a {
                            WindowAction::Tray => hide_main(&h2),
                            WindowAction::Quit => h2.exit(0),
                        });
                    }
                })),
                on_open_url: Some(Arc::new({
                    let h = handle.clone();
                    move |url: String, title: String| {
                        // 建窗口会等主线程，放到单独线程里，不占用服务的线程。
                        // Building a window waits on the main thread; do it off the service threads.
                        let h = h.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = open_browse_window(&h, &url, &title) {
                                log::warn!("无法打开预览窗口 can't open the preview window: {e}");
                            }
                        });
                    }
                })),
            };

            let url = match start_server(hooks) {
                Ok(info) => {
                    app.manage(info.control.clone());
                    WebviewUrl::External(info.local_url.parse()?)
                }
                Err(e) => {
                    log::error!("服务启动失败 service failed to start: {e}");
                    // 显示内置的错误页。Show the bundled error page.
                    let msg: String = e
                        .bytes()
                        .map(|b| match b {
                            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                                (b as char).to_string()
                            }
                            _ => format!("%{b:02X}"),
                        })
                        .collect();
                    WebviewUrl::App(format!("index.html?error={msg}").into())
                }
            };
            WebviewWindowBuilder::new(app, MAIN, url)
                .title("SoundBridge")
                .inner_size(1080.0, 700.0)
                .min_inner_size(760.0, 520.0)
                .center()
                // 与界面背景同色，避免启动时白屏闪烁。Match the UI background to avoid a
                // white flash on start.
                .background_color(tauri::webview::Color(13, 14, 18, 255))
                // 关闭 Tauri 的文件拖放拦截，让网页自己处理拖入导入。
                // Disable Tauri's file-drop interception so the page handles drag-in import.
                .disable_drag_drop_handler()
                .build()?;

            build_tray(app.handle())?;
            if let Some(control) = app.try_state::<Control>() {
                let s = control.settings();
                apply_icon(app.handle(), &s.accent, s.tint_app_icon);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let app = window.app_handle();
            let Some(control) = app.try_state::<Control>() else {
                return; // 服务没起来：直接关闭。Service never started: just close.
            };
            match control.settings().close_action {
                CloseAction::Quit => app.exit(0),
                CloseAction::Tray => {
                    api.prevent_close();
                    hide_main(app);
                }
                CloseAction::Ask => {
                    // 第一次点：请界面弹框；询问还没回答时再点一次：直接退出。
                    // First click: ask the UI to prompt; clicking again while it's
                    // unanswered quits.
                    if control.request_close() {
                        app.exit(0);
                    } else {
                        api.prevent_close();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 失败 failed to run Tauri");
}

/// 显示并聚焦主窗口。Show and focus the main window.
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 隐藏主窗口（音效与快捷键继续工作，托盘里可再打开）。
/// Hide the main window (sounds and hotkeys keep working; reopen from the tray).
fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.hide();
    }
}

/// 创建托盘图标：单击打开窗口，菜单里有「打开」「退出」。
/// Create the tray icon: click to open the window; the menu has Open and Quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY)
        .icon(base_icon())
        .tooltip("SoundBridge")
        .menu(&tray_menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 音效资源网站的预览窗口：在 App 里浏览网站、试听，下载的音频文件直接存进音效库。
/// Preview window for sound sites: browse and listen inside the app; downloaded audio
/// files are saved straight into the sound library.
fn open_browse_window(app: &AppHandle, url: &str, title: &str) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{e}"))?;
    let title = if title.is_empty() {
        "SoundBridge".to_string()
    } else {
        format!("{title} - SoundBridge")
    };
    // 已打开：换网址并提到前台。Already open: navigate and bring to front.
    if let Some(w) = app.get_webview_window(BROWSE) {
        w.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = w.set_title(&title);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, BROWSE, WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1100.0, 760.0)
        .min_inner_size(480.0, 360.0)
        .center()
        .on_download(|webview, event| {
            // 音频文件改存到音效库的「下载」文件夹；其它文件照常进系统下载目录。
            // Audio files go to the library's "Downloads" folder; anything else goes to
            // the system downloads folder as usual.
            if let DownloadEvent::Requested { destination, .. } = event {
                let name = destination
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if soundbridge_server::is_audio_file(&name) {
                    if let Some(control) = webview.app_handle().try_state::<Control>() {
                        let dir = control.settings().library_root().join(download_folder());
                        if std::fs::create_dir_all(&dir).is_ok() {
                            *destination = unique_path(&dir, &name);
                            log::info!(
                                "下载到音效库 downloading into the library: {}",
                                destination.display()
                            );
                        }
                    }
                }
            }
            true
        })
        .build()
        .map_err(|e| e.to_string())?;
    // 新窗口也跟随主题色图标。The new window follows the tinted icon too.
    if let Some(control) = app.try_state::<Control>() {
        let s = control.settings();
        let (h, accent, tint) = (app.clone(), s.accent, s.tint_app_icon);
        let _ = app.run_on_main_thread(move || apply_icon(&h, &accent, tint));
    }
    Ok(())
}

/// 音效库里存放下载的文件夹名（按界面语言）。Library folder for downloads (UI language).
fn download_folder() -> &'static str {
    if soundbridge_server::i18n::is_en() {
        "Downloads"
    } else {
        "下载"
    }
}

/// 不覆盖已有文件：重名时加「 (2)」「 (3)」…。Never overwrite: append " (2)", " (3)"… on clashes.
fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|n| {
            dir.join(if ext.is_empty() {
                format!("{stem} ({n})")
            } else {
                format!("{stem} ({n}).{ext}")
            })
        })
        .find(|p| !p.exists())
        .expect("有限个文件 finitely many files")
}

/// 托盘菜单（按当前界面语言）。The tray menu (in the current UI language).
fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let en = soundbridge_server::i18n::is_en();
    let open_text = if en {
        "Open SoundBridge"
    } else {
        "打开 SoundBridge"
    };
    let quit_text = if en { "Quit" } else { "退出" };
    let open = MenuItem::with_id(app, "open", open_text, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
}

/// 按当前语言重建托盘菜单。Rebuild the tray menu in the current language.
fn apply_tray_menu(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// 原始图标（紫蓝渐变 + 白色声波）。The original icon (violet-blue gradient + white bars).
fn base_icon() -> Image<'static> {
    tauri::include_image!("icons/icon.png")
}

/// 按主题色给窗口与托盘图标着色（关闭此选项时用原图）。
/// Tint the window and tray icons with the accent (the original when disabled).
fn apply_icon(app: &AppHandle, accent: &str, tint: bool) {
    let icon = if tint {
        tinted_icon(accent).unwrap_or_else(base_icon)
    } else {
        base_icon()
    };
    // 主窗口与预览窗口都换。Both the main and the preview window.
    for w in app.webview_windows().values() {
        let _ = w.set_icon(icon.clone());
    }
    if let Some(tray) = app.tray_by_id(TRAY) {
        let _ = tray.set_icon(Some(icon));
    }
}

/// 把图标的色相整体旋转到主题色（白色、透明像素不变），保留原图的渐变。
/// Rotate the icon's hue to the accent (white and transparent pixels unchanged),
/// keeping the original gradient.
fn tinted_icon(accent: &str) -> Option<Image<'static>> {
    let target = hex_hue(accent)?;
    let delta = target - BASE_HUE;
    let base = base_icon();
    let mut rgba = base.rgba().to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            continue;
        }
        let (h, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        if s < 0.08 {
            continue; // 白色声波柱。The white bars.
        }
        let (r, g, b) = hsl_to_rgb((h + delta).rem_euclid(360.0), s, l);
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
    Some(Image::new_owned(rgba, base.width(), base.height()))
}

/// `#rrggbb` 的色相（度）。Hue of `#rrggbb` in degrees.
fn hex_hue(hex: &str) -> Option<f32> {
    let n = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
    let (h, _, _) = rgb_to_hsl((n >> 16) as u8, (n >> 8) as u8, n as u8);
    Some(h)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        let c = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (c * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (f(0.0), f(8.0), f(4.0))
}

/// 在后台线程启动服务，等待其就绪并返回地址与控制句柄。
/// Start the service on a background thread, wait until ready, and return its address
/// and control handle.
fn start_server(hooks: Hooks) -> Result<ServerInfo, String> {
    let (tx, rx) = mpsc::channel::<Result<ServerInfo, String>>();
    std::thread::Builder::new()
        .name("soundbridge-service".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let mut config = ServerConfig::from_env();
            // 默认端口被占用（如浏览器版在运行）时改用随机端口。
            // Use a random port if the default is taken (e.g. the browser version runs).
            config.port_fallback = true;
            config.hooks = hooks;
            let ready_tx = tx.clone();
            let result = rt.block_on(soundbridge_server::run(config, move |info| {
                let _ = ready_tx.send(Ok(info));
            }));
            if let Err(e) = result {
                let _ = tx.send(Err(format!("{e:#}")));
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(20))
        .map_err(|_| "服务启动超时 service start timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_round_trip() {
        let (h, s, l) = rgb_to_hsl(0x7b, 0x6c, 0xff);
        assert!((h - BASE_HUE).abs() < 0.5, "h={h}");
        assert_eq!(hsl_to_rgb(h, s, l), (0x7b, 0x6c, 0xff));
        assert!((hex_hue("#22c55e").unwrap() - 142.1).abs() < 1.0);
    }
}
