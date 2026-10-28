//! SoundBridge 服务库：音频（音效板 + 麦克风）+ 全局热键 + HTTP API + 内嵌前端。
//! SoundBridge service library: audio (soundboard + mic) + global hotkeys + HTTP API
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

mod api;
mod audio;
mod hotkeys;
mod library;
mod mic;
mod settings;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use crossbeam_channel::unbounded;
use soundboard::{BoardCommand, LoadedSound};

use api::AppState;
use library::{Library, SoundRuntime};
use mic::MicControl;
use settings::SettingsStore;

pub use settings::data_dir;

/// 默认端口。Default port.
pub const DEFAULT_PORT: u16 = 8321;

/// 服务启动参数。Service start-up options.
#[derive(Debug, Clone)]
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
        }
    }
}

/// 服务已就绪的信息。Info once the service is ready.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// 实际端口。Actual port.
    pub port: u16,
    /// 本机访问地址。Local URL.
    pub local_url: String,
}

/// 启动并运行服务，直到出错。`on_ready` 在开始监听后调用一次。
/// Start and run the service until it fails. `on_ready` is called once listening.
///
/// 返回的 future 持有音频流句柄（非 `Send`），请在专用线程上用 `block_on` 运行。
/// The future holds audio stream handles (not `Send`); run it with `block_on` on a
/// dedicated thread.
pub async fn run(config: ServerConfig, on_ready: impl FnOnce(ServerInfo)) -> anyhow::Result<()> {
    let data_dir = settings::data_dir();
    let settings = Arc::new(SettingsStore::load_default());
    let saved = settings.get();
    let library = Arc::new(Library::load(&data_dir));

    // 命令通道：Web API / 热键 → 音频线程。Command channel: web API / hotkeys → audio.
    let (cmd_tx, cmd_rx) = unbounded::<BoardCommand>();

    // 初始麦克风：参数优先，否则上次的选择，都没有则跟随系统默认。
    // Initial mic: explicit option first, else the last choice, else system default.
    let initial_mic = config.input_device.clone().or(saved.mic_device.clone());
    let mic = Arc::new(MicControl::new(config.mic_muted, saved.mic_gain_db));
    let audio_rt = audio::start_audio(
        cmd_rx,
        config.output_device.as_deref(),
        initial_mic,
        mic.clone(),
        settings.clone(),
    )?;

    let (hotkey_tx, hotkey_rx) = unbounded::<hotkeys::HotkeyCommand>();
    hotkeys::spawn_hotkey_thread(hotkey_rx, cmd_tx.clone());

    // 加载音效库：逐个解码并交给音频线程，绑定快捷键。文件丢失的保留在列表里并标出错误。
    // Load the library: decode each sound, hand it to audio, bind hotkeys. Sounds whose
    // files are missing stay listed with an error.
    for spec in library.list() {
        let id = spec.id;
        let hotkey = spec.hotkey.clone();
        match LoadedSound::load(spec) {
            Ok(loaded) => {
                library.set_runtime(
                    id,
                    SoundRuntime {
                        duration_secs: Some(loaded.duration_secs()),
                        error: None,
                    },
                );
                let _ = cmd_tx.send(BoardCommand::AddSound(Box::new(loaded)));
                if hotkey.is_some() {
                    let _ = hotkey_tx.send(hotkeys::HotkeyCommand::Bind {
                        sound_id: id,
                        hotkey,
                        reply: None,
                    });
                }
            }
            Err(e) => {
                log::warn!("音效 #{id} 加载失败 failed to load sound: {e}");
                library.set_runtime(
                    id,
                    SoundRuntime {
                        duration_secs: None,
                        error: Some(format!("无法加载音频文件：{e}")),
                    },
                );
            }
        }
    }

    // 监听地址：开启手机遥控时为 0.0.0.0，否则只监听本机。
    // Listen address: 0.0.0.0 with phone remote on, otherwise localhost only.
    let lan = config.lan_override.unwrap_or(saved.lan_remote);
    let ip: IpAddr = if lan {
        [0, 0, 0, 0].into()
    } else {
        [127, 0, 0, 1].into()
    };
    let listener = match tokio::net::TcpListener::bind(SocketAddr::new(ip, config.port)).await {
        Ok(l) => l,
        Err(e) if config.port_fallback => {
            log::warn!("端口 {} 被占用，改用随机端口 port taken: {e}", config.port);
            tokio::net::TcpListener::bind(SocketAddr::new(ip, 0)).await?
        }
        Err(e) => return Err(e.into()),
    };
    let port = listener.local_addr()?.port();

    let state = AppState {
        cmd_tx,
        hotkey_tx,
        library,
        mic,
        mic_tx: audio_rt.mic_tx.clone(),
        audio_info: Arc::new(audio_rt.info.clone()),
        playing: audio_rt.playing.clone(),
        settings,
        data_dir,
        port,
        lan_active: lan,
    };
    let app = api::router(state).fallback(static_handler);

    let local_url = format!("http://127.0.0.1:{port}");
    log::info!("SoundBridge 已启动 listening on {local_url}");
    if lan {
        log::info!("手机遥控已开启 phone remote: http://<电脑IP>:{port}");
    }
    on_ready(ServerInfo { port, local_url });

    axum::serve(listener, app).await?;
    // 音频运行时活到服务结束。Keep the audio runtime alive until the server ends.
    drop(audio_rt);
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
