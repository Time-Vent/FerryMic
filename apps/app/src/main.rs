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
//! 3. 服务就绪后，用原生窗口打开 `http://127.0.0.1:<端口>`，界面与浏览器版完全相同。
//!    Once ready, open `http://127.0.0.1:<port>` in a native window; the UI is the same
//!    as the browser version.
//!
//! 界面通过 HTTP 访问本机服务，不使用 Tauri IPC，因此无需开放任何 Tauri 权限。
//! The UI talks to the local service over HTTP, not Tauri IPC, so no Tauri
//! permissions need to be granted.

// 发布版不弹控制台窗口。No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc;
use std::time::Duration;

use soundbridge_server::{ServerConfig, ServerInfo};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// 主窗口标签。Main window label.
const MAIN: &str = "main";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        // 第二次启动：聚焦已有窗口。Second launch: focus the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window(MAIN) {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let url = match start_server() {
                Ok(info) => WebviewUrl::External(info.local_url.parse()?),
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 失败 failed to run Tauri");
}

/// 在后台线程启动服务，等待其就绪并返回地址。
/// Start the service on a background thread, wait until ready, and return its address.
fn start_server() -> Result<ServerInfo, String> {
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
