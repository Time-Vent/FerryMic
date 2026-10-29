//! FerryMic 服务库：音频（音效板 + 麦克风）+ 全局热键 + HTTP API + 内嵌前端。
//! FerryMic service library: audio (soundboard + mic) + global hotkeys + HTTP API
//! + embedded front-end.
//!
//! 两个宿主共用：
//! Shared by two hosts:
//! - `soundbridge-server`（`main.rs`）：命令行启动，用浏览器访问。
//!   `soundbridge-server` (`main.rs`): started from the command line, used in a browser.
//! - `soundbridge-app`（`apps/app`，Tauri）：桌面 App，窗口里加载同一套界面。
//!   `soundbridge-app` (`apps/app`, Tauri): desktop app showing the same UI in a window.
//!
//! 手机遥控：设置里开启后监听 `0.0.0.0`，手机浏览器打开 `http://<电脑IP>:<端口>`；
//! 默认只监听本机。
//! Phone remote: when enabled in settings it listens on `0.0.0.0` and a phone browser
//! opens `http://<PC-IP>:<port>`; by default it listens on localhost only.

#[macro_use]
pub mod i18n;
mod api;
mod audio;
mod hotkeys;
mod library;
mod loader;
mod mic;
mod monitor;
mod remote;
mod settings;
mod virtual_mic;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use crossbeam_channel::unbounded;
use soundboard::BoardCommand;

use api::AppState;
use library::Library;
use loader::Player;
use mic::MicControl;
use settings::SettingsStore;
use virtual_mic::{VirtualMic, VirtualMicInfo};

pub use api::open_in_browser;
pub use settings::{data_dir, is_web_url, CloseAction, Settings};

/// 文件名是否是支持的音频格式（按扩展名）。Whether a file name is a supported audio format
/// (by extension).
pub fn is_audio_file(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| library::is_audio_ext(ext))
}

/// 默认端口。Default port.
pub const DEFAULT_PORT: u16 = 8321;

/// 窗口操作（界面里的关闭询问选好后交给桌面外壳执行）。
/// A window action (chosen in the UI's close prompt, carried out by the desktop shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    /// 最小化到托盘。Minimize to the tray.
    Tray,
    /// 退出程序。Quit.
    Quit,
}

/// 设置改变后的回调。Callback after settings change.
pub type SettingsHook = Arc<dyn Fn(&Settings) + Send + Sync>;
/// 关闭询问选好后的回调。Callback once the close prompt is answered.
pub type WindowHook = Arc<dyn Fn(WindowAction) + Send + Sync>;
/// 在 App 窗口里打开网址的回调（网址, 标题）。Callback to open a URL in an app window (url, title).
pub type UrlHook = Arc<dyn Fn(String, String) + Send + Sync>;

/// 桌面外壳的回调（浏览器版没有）。Callbacks for the desktop shell (absent in the
/// browser version).
#[derive(Clone, Default)]
pub struct Hooks {
    /// 设置改变后调用（外壳据此更新图标颜色等）。Called after settings change (e.g. to
    /// re-tint the icon).
    pub on_settings: Option<SettingsHook>,
    /// 关闭询问选好后调用。Called once the close prompt is answered.
    pub on_window_action: Option<WindowHook>,
    /// 在 App 里的预览窗口打开网址；没有时用系统浏览器。Open a URL in the app's preview
    /// window; without it the system browser is used.
    pub on_open_url: Option<UrlHook>,
}

/// 外壳持有的控制句柄。A control handle held by the shell.
#[derive(Clone)]
pub struct Control {
    close_prompt: Arc<AtomicBool>,
    settings: Arc<SettingsStore>,
}

impl Control {
    /// 用户点了窗口关闭按钮：请界面弹出询问。返回 `true` 表示询问已在显示中
    /// （再点一次 = 直接退出，防止界面无响应时关不掉）。
    /// The user clicked the window's close button: ask the UI to prompt. Returns `true`
    /// if a prompt is already showing (a second click = quit, so an unresponsive UI
    /// can't trap the user).
    pub fn request_close(&self) -> bool {
        self.close_prompt.swap(true, Ordering::Relaxed)
    }

    /// 当前设置快照。A snapshot of the current settings.
    pub fn settings(&self) -> Settings {
        self.settings.get()
    }
}

/// 服务启动参数。Service start-up options.
#[derive(Clone)]
pub struct ServerConfig {
    /// 监听端口。Port to listen on.
    pub port: u16,
    /// 端口被占用时是否改用随机空闲端口（App 用；命令行版直接报错）。
    /// Fall back to a random free port when taken (app: yes; CLI: fail instead).
    pub port_fallback: bool,
    /// 输出设备名（包含匹配），`None` = 系统默认。Output device (containment match).
    pub output_device: Option<String>,
    /// 麦克风（包含匹配），优先于保存的选择。Mic (containment match), overrides saved.
    pub input_device: Option<String>,
    /// 启动时麦克风静音。Start with the mic muted.
    pub mic_muted: bool,
    /// 强制开/关局域网遥控（覆盖设置）。Force LAN remote on/off (overrides settings).
    pub lan_override: Option<bool>,
    /// 桌面外壳回调。Desktop-shell callbacks.
    pub hooks: Hooks,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            port_fallback: false,
            output_device: None,
            input_device: None,
            mic_muted: false,
            lan_override: None,
            hooks: Hooks::default(),
        }
    }
}

impl ServerConfig {
    /// 从环境变量读取（见 README）。Read from environment variables (see README).
    pub fn from_env() -> Self {
        let flag = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| matches!(v.trim(), "1" | "true" | "yes"))
        };
        Self {
            port: std::env::var("SOUNDBRIDGE_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            port_fallback: false,
            output_device: std::env::var("SOUNDBRIDGE_OUTPUT_DEVICE").ok(),
            input_device: std::env::var("SOUNDBRIDGE_INPUT_DEVICE").ok(),
            mic_muted: flag("SOUNDBRIDGE_MIC_MUTED").unwrap_or(false),
            lan_override: flag("SOUNDBRIDGE_LAN"),
            hooks: Hooks::default(),
        }
    }
}

/// 服务已就绪的信息。Info once the service is ready.
#[derive(Clone)]
pub struct ServerInfo {
    /// 实际端口。Actual port.
    pub port: u16,
    /// 本机访问地址。Local URL.
    pub local_url: String,
    /// 外壳用的控制句柄。Control handle for the shell.
    pub control: Control,
}

/// 启动并运行服务，直到出错。`on_ready` 在开始监听后调用一次。
/// Start and run the service until it fails. `on_ready` is called once listening.
///
/// 返回的 future 持有音频流句柄（非 `Send`），请在专用线程上用 `block_on` 运行。
/// The future holds audio stream handles (not `Send`); run it with `block_on` on a
/// dedicated thread.
pub async fn run(config: ServerConfig, on_ready: impl FnOnce(ServerInfo)) -> anyhow::Result<()> {
    // 改名迁移：SoundBridge → FerryMic 数据目录。Rename migration of the data directory.
    let migrated = settings::migrate_legacy_data_dir();
    let data_dir = settings::data_dir();
    let settings = Arc::new(SettingsStore::load_default());
    if let Some((old, new)) = &migrated {
        // 音效库若指定在旧目录里，跟着换到新目录。A library set inside the old directory
        // follows it to the new one.
        settings.update(|s| {
            if let Some(dir) = &s.library_dir {
                if let Some(p) = settings::rebase_path(dir, old, new) {
                    s.library_dir = Some(p);
                }
            }
        });
    }
    let saved = settings.get();
    i18n::apply(&saved.language);

    // 输出设备：参数（环境变量）优先，否则上次的选择，都没有则系统默认。
    // Output device: explicit option (env) first, else the last choice, else default.
    let output_device = config.output_device.clone().or(saved.output_device.clone());

    // Linux：未指定输出设备时自动创建虚拟麦克风（必须在打开音频流之前）。
    // Linux: create the virtual mic when no output device is given (before any stream).
    let (virtual_mic, virtual_mic_info) =
        if cfg!(target_os = "linux") && output_device.is_none() && saved.auto_virtual_mic {
            match VirtualMic::setup() {
                Ok(v) => (
                    Some(v),
                    Some(VirtualMicInfo {
                        active: true,
                        source: virtual_mic::SOURCE_DESCRIPTION.into(),
                        note: None,
                    }),
                ),
                Err(e) => {
                    log::warn!("虚拟麦克风创建失败 virtual mic setup failed: {e}");
                    (
                        None,
                        Some(VirtualMicInfo {
                            active: false,
                            source: virtual_mic::SOURCE_DESCRIPTION.into(),
                            note: Some(e),
                        }),
                    )
                }
            }
        } else {
            (None, None)
        };

    // 音效库：先迁移旧版数据，再打开根目录。Library: migrate old data, then open the root.
    let root = saved.library_root();
    library::migrate_v1(&data_dir, &root);
    let library = Arc::new(Library::open_with(root, saved.sound_defaults.clamped()));

    // 命令通道：加载线程 / Web API / 热键 → 音频线程。
    // Command channel: loader / web API / hotkeys → audio.
    let (cmd_tx, cmd_rx) = unbounded::<BoardCommand>();

    // 加载线程（切换输出设备时要靠它重新装入音效，所以先于音频创建）。
    // Loader thread (created before audio: output switches rely on it to re-install sounds).
    let player = Player::new(cmd_tx, library.clone());

    // 初始麦克风：参数优先，否则上次的选择，都没有则自动选择硬件麦克风。
    // Initial mic: explicit option first, else the last choice, else auto (hardware mic).
    let initial_mic = config.input_device.clone().or(saved.mic_device.clone());
    let mic = Arc::new(MicControl::new(config.mic_muted, saved.mic_gain_db));
    let monitor = Arc::new(monitor::MonitorControl::new(saved.monitor_gain_db));
    let audio_rt = audio::start_audio(
        cmd_rx,
        output_device,
        initial_mic,
        mic.clone(),
        monitor.clone(),
        settings.clone(),
        player.clone(),
    )?;

    // 热键线程。Hotkey thread.
    let (hotkey_tx, hotkey_rx) = unbounded::<hotkeys::HotkeyCommand>();
    hotkeys::spawn_hotkey_thread(hotkey_rx, player.clone());

    // 绑定快捷键并排队预加载（后台进行，不阻塞启动）。
    // Bind hotkeys and queue preloads (in the background; start-up isn't blocked).
    for e in library.entries() {
        if e.meta.hotkey.is_some() {
            let _ = hotkey_tx.send(hotkeys::HotkeyCommand::Bind {
                sound_id: e.meta.id,
                hotkey: e.meta.hotkey.clone(),
                reply: None,
            });
        }
        player.preload(e.meta.id);
    }

    // 主服务只监听本机；手机遥控由单独的局域网监听负责（可随时开关）。
    // The main service listens on localhost only; the phone remote has its own LAN
    // listener (switchable at any time).
    let ip: IpAddr = [127, 0, 0, 1].into();
    let listener = match tokio::net::TcpListener::bind(SocketAddr::new(ip, config.port)).await {
        Ok(l) => l,
        Err(e) if config.port_fallback => {
            log::warn!("端口 {} 被占用，改用随机端口 port taken: {e}", config.port);
            tokio::net::TcpListener::bind(SocketAddr::new(ip, 0)).await?
        }
        Err(e) => return Err(e.into()),
    };
    let port = listener.local_addr()?.port();

    // 手机遥控：访问码保存在设置里（首次生成），端口默认为主端口 + 1。
    // Phone remote: the access code lives in settings (generated once); port = main + 1.
    let token = saved.remote_token.clone().unwrap_or_else(|| {
        let t = remote::new_token();
        settings.update(|s| s.remote_token = Some(t.clone()));
        t
    });
    let remote = Arc::new(remote::RemoteServer::new(token, port.saturating_add(1)));
    let close_prompt = Arc::new(AtomicBool::new(false));

    let state_settings = settings.clone();
    let state = AppState {
        player,
        hotkey_tx,
        library,
        mic,
        mic_tx: audio_rt.mic_tx.clone(),
        out_tx: audio_rt.out_tx.clone(),
        monitor,
        monitor_tx: audio_rt.monitor_tx.clone(),
        audio_info: audio_rt.info.clone(),
        playing: audio_rt.playing.clone(),
        settings,
        data_dir,
        port,
        remote: remote.clone(),
        virtual_mic: virtual_mic_info,
        hooks: config.hooks.clone(),
        close_prompt: close_prompt.clone(),
    };
    let app = api::router(state).fallback(static_handler);
    remote.set_router(app.clone());
    if config.lan_override.unwrap_or(saved.lan_remote) {
        if let Err(e) = remote.start().await {
            log::warn!("手机遥控开启失败 phone remote failed: {e}");
            remote.set_error(Some(e));
        }
    }

    let local_url = format!("http://127.0.0.1:{port}");
    log::info!("FerryMic 已启动 listening on {local_url}");
    on_ready(ServerInfo {
        port,
        local_url,
        control: Control {
            close_prompt,
            settings: state_settings,
        },
    });

    axum::serve(listener, app).await?;
    // 音频运行时与虚拟麦克风活到服务结束（虚拟麦克风 drop 时卸载）。
    // Keep the audio runtime and virtual mic alive until the end (the mic unloads on drop).
    drop(audio_rt);
    drop(virtual_mic);
    Ok(())
}

/// 内嵌的前端文件（`apps/web/dist`）。Embedded front-end files (`apps/web/dist`).
#[derive(rust_embed::RustEmbed)]
#[folder = "../web/dist/"]
struct WebAssets;

/// 静态文件：找不到的路径回退到 `index.html`。index.html 禁止缓存，避免升级后看到旧界面。
/// Static files: unknown paths fall back to `index.html`, which is never cached so an
/// upgrade can't show a stale UI.
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (path, file) = match WebAssets::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (path, f),
        None => match WebAssets::get("index.html") {
            Some(f) => ("index.html", f),
            None => return (StatusCode::NOT_FOUND, "front-end not built").into_response(),
        },
    };
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        file.data,
    )
        .into_response()
}

/// 本机局域网 IP（用于显示手机访问地址）。只查询路由表，不发送任何数据包。
/// The machine's LAN IP (to show the phone URL). Only consults the routing table; no
/// packet is sent.
pub fn lan_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?; // TEST-NET-1，不可路由 / non-routable
    Some(socket.local_addr().ok()?.ip())
}
