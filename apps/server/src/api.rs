//! HTTP API：音效库与文件夹、播放控制、麦克风、设备、设置、实时状态。
//! HTTP API: sound library and folders, playback control, mic, devices, settings, live
//! state.
//!
//! 所有阻塞操作（解码、文件操作、设备枚举、等待其它线程回复、系统对话框）都放到
//! `spawn_blocking`，不卡住 tokio 工作线程。
//! Every blocking operation (decoding, file operations, device enumeration, waiting
//! for other threads, native dialogs) runs in `spawn_blocking` so tokio workers are
//! never stalled.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use audio_cpal::CpalBackend;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use crossbeam_channel::Sender;
use platform_api::AudioBackend;
use serde::{Deserialize, Deserializer, Serialize};
use soundboard::{BoardCommand, LoadedSound, PlayMode, SoundId};

use crate::audio::{describe_output, AudioInfo, OutputKind, OutputRequest, PlayingSet};
use crate::hotkeys::HotkeyCommand;
use crate::library::{Entry, Library, ScanDiff};
use crate::loader::Player;
use crate::mic::{self, MicControl, MicRequest};
use crate::monitor::{MonitorControl, MonitorRequest, MonitorStatus};
use crate::remote::{qr_svg, RemoteServer, RemoteStatus};
use crate::settings::{is_hex_color, CloseAction, SettingsStore, ThemeMode};
use crate::virtual_mic::VirtualMicInfo;
use crate::{Hooks, WindowAction};

/// 导入文件大小上限（字节）。Max imported file size (bytes).
const MAX_IMPORT_BYTES: usize = 200 * 1024 * 1024;

/// API 错误：状态码 + 给用户看的消息。API error: status code + user-facing message.
type ApiError = (StatusCode, String);
type ApiResult<T> = Result<T, ApiError>;

fn internal(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, msg.into())
}

/// 在阻塞线程池运行。Run on the blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(internal)
}

/// 共享应用状态。Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// 播放与加载入口。Playback and loading entry point.
    pub player: Arc<Player>,
    /// 热键指令发送端（→ 热键线程）。Hotkey command sender (→ hotkey thread).
    pub hotkey_tx: Sender<HotkeyCommand>,
    /// 音效库（磁盘文件夹）。Sound library (a folder on disk).
    pub library: Arc<Library>,
    /// 麦克风参数、电平与设备状态。Mic parameters, level and device status.
    pub mic: Arc<MicControl>,
    /// 麦克风请求发送端（→ 麦克风线程）。Mic request sender (→ mic thread).
    pub mic_tx: Sender<MicRequest>,
    /// 输出请求发送端（→ 输出线程）。Output request sender (→ output thread).
    pub out_tx: Sender<OutputRequest>,
    /// 试听参数与状态。Monitor parameters and status.
    pub monitor: Arc<MonitorControl>,
    /// 试听请求发送端（→ 试听线程）。Monitor request sender (→ monitor thread).
    pub monitor_tx: Sender<MonitorRequest>,
    /// 当前输出设备（切换后更新）。Current output device (updated on switches).
    pub audio_info: Arc<std::sync::Mutex<AudioInfo>>,
    /// 正在播放的音效。Playing sounds.
    pub playing: Arc<PlayingSet>,
    /// 持久化设置。Persisted settings.
    pub settings: Arc<SettingsStore>,
    /// 数据目录。Data directory.
    pub data_dir: PathBuf,
    /// 实际监听端口。Actual listening port.
    pub port: u16,
    /// 手机遥控（局域网监听）。Phone remote (LAN listener).
    pub remote: Arc<RemoteServer>,
    /// Linux 虚拟麦克风状态（其它平台为 None）。Linux virtual mic status (None elsewhere).
    pub virtual_mic: Option<VirtualMicInfo>,
    /// 桌面外壳回调（浏览器版为空）。Desktop-shell callbacks (empty in the browser version).
    pub hooks: Hooks,
    /// 外壳请求弹出关闭询问。The shell asked for the close prompt.
    pub close_prompt: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    /// 当前输出设备信息。Linux 自动虚拟麦克风生效时，游戏里选的是虚拟麦克风。
    /// Current output info. With the Linux auto virtual mic, games pick that mic.
    fn output_info(&self) -> AudioInfo {
        let mut info = self.audio_info.lock().unwrap().clone();
        if let Some(vm) = self.virtual_mic.as_ref().filter(|v| v.active) {
            if self.settings.get().output_device.is_none() {
                info.output_kind = OutputKind::Virtual;
                info.game_mic = Some(vm.source.clone());
            }
        }
        info
    }

    /// 让热键线程绑定快捷键并等待结果。Ask the hotkey thread to bind and await the result.
    async fn bind_hotkey(&self, sound_id: SoundId, hotkey: Option<String>) -> Result<(), String> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.hotkey_tx
            .send(HotkeyCommand::Bind {
                sound_id,
                hotkey,
                reply: Some(reply),
            })
            .map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(3)))
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|_| {
                Err(tr!("热键线程无响应", "The hotkey thread is not responding").into())
            })
    }

    /// 绑定快捷键，不等结果（批量同步时用）。Bind without waiting (for bulk syncs).
    fn bind_hotkey_async(&self, sound_id: SoundId, hotkey: Option<String>) {
        let _ = self.hotkey_tx.send(HotkeyCommand::Bind {
            sound_id,
            hotkey,
            reply: None,
        });
    }

    /// 把扫描结果同步到音频线程与热键：移除的卸载并解绑，新增的绑定并预加载。
    /// Apply a scan diff to audio and hotkeys: unload + unbind removed, bind + preload added.
    fn apply_diff(&self, diff: &ScanDiff) {
        for &id in &diff.removed {
            self.bind_hotkey_async(id, None);
            self.player.unload(id);
        }
        for &id in &diff.added {
            if let Some(e) = self.library.entry(id) {
                if e.meta.hotkey.is_some() {
                    self.bind_hotkey_async(id, e.meta.hotkey.clone());
                }
            }
            self.player.preload(id);
        }
    }

    fn item(&self, e: &Entry) -> SoundItem {
        let rt = self.library.runtime(e.meta.id);
        SoundItem {
            id: e.meta.id,
            name: e.name().to_string(),
            folder: e.folder().to_string(),
            file_name: e.file_name().to_string(),
            looping: e.meta.mode.looping,
            exclusive: e.meta.mode.exclusive,
            gain_db: e.meta.gain_db,
            speed: e.meta.speed,
            hotkey: e.meta.hotkey.clone(),
            duration_secs: rt.duration_secs,
            loaded: rt.loaded,
            error: rt.error,
        }
    }

    fn library_view(&self) -> LibraryView {
        LibraryView {
            root: self.library.root().display().to_string(),
            folders: self.library.folders(),
            sounds: self
                .library
                .entries()
                .iter()
                .map(|e| self.item(e))
                .collect(),
        }
    }
}

/// 构建 API 路由（挂在 `/api` 下）。Build the API routes (mounted under `/api`).
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/app", get(get_app))
        .route("/settings", post(set_settings))
        .route("/window", post(window_action))
        .route("/pick-folder", post(pick_folder))
        .route("/live", get(get_live))
        .route("/devices", get(list_devices))
        .route("/output", post(select_output))
        .route("/monitor", get(get_monitor).post(set_monitor))
        .route("/remote", get(get_remote).post(set_remote))
        .route("/remote/qr.svg", get(remote_qr))
        .route("/mic", get(get_mic).post(set_mic))
        .route("/mic/device", post(select_mic))
        .route("/library", get(get_library))
        .route("/library/rescan", post(rescan_library))
        .route("/library/location", post(set_library_location))
        .route("/library/open", post(open_library))
        .route(
            "/folders",
            post(create_folder)
                .patch(rename_folder)
                .delete(delete_folder),
        )
        .route("/sounds", get(list_sounds))
        .route(
            "/sounds/import",
            post(import_sound).layer(DefaultBodyLimit::max(MAX_IMPORT_BYTES)),
        )
        .route(
            "/sounds/:id",
            axum::routing::patch(update_sound).delete(remove_sound),
        )
        .route("/sounds/:id/trigger", post(trigger_sound))
        .route("/sounds/:id/stop", post(stop_sound))
        .route("/sounds/:id/seek", post(seek_sound))
        .route("/stop-all", post(stop_all));
    Router::new().nest("/api", api).with_state(state)
}

// ---------------------------------------------------------------------------
// 应用信息与设置 App info and settings
// ---------------------------------------------------------------------------

/// 应用信息（设置页用）。App info (for the settings page).
#[derive(Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    /// `windows` / `linux` / `macos`。
    platform: &'static str,
    data_dir: String,
    library_dir: String,
    output: AudioInfo,
    port: u16,
    /// 手机遥控状态。Phone-remote status.
    remote: RemoteStatus,
    theme_mode: ThemeMode,
    accent: String,
    /// 是否在桌面 App 里运行（显示关闭行为等 App 专属设置）。Running inside the desktop app.
    is_app: bool,
    tint_app_icon: bool,
    close_action: CloseAction,
    /// 语言偏好（`system` / `zh-CN` / `en`）。Language preference.
    language: String,
    /// 实际使用的语言（`zh-CN` / `en`）。The language actually in use.
    language_effective: &'static str,
    auto_virtual_mic: bool,
    /// Linux 虚拟麦克风状态。Linux virtual mic status.
    virtual_mic: Option<VirtualMicInfo>,
}

fn app_info(state: &AppState) -> AppInfo {
    let s = state.settings.get();
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        data_dir: state.data_dir.display().to_string(),
        library_dir: state.library.root().display().to_string(),
        output: state.output_info(),
        port: state.port,
        remote: state.remote.status(),
        theme_mode: s.theme_mode,
        accent: s.accent,
        auto_virtual_mic: s.auto_virtual_mic,
        is_app: state.hooks.on_window_action.is_some(),
        tint_app_icon: s.tint_app_icon,
        close_action: s.close_action,
        language: s.language,
        language_effective: crate::i18n::current(),
        virtual_mic: state.virtual_mic.clone(),
    }
}

async fn get_app(State(state): State<AppState>) -> Json<AppInfo> {
    Json(app_info(&state))
}

/// 修改设置（字段可选）。Update settings (fields optional).
#[derive(Debug, Deserialize)]
struct SettingsRequest {
    theme_mode: Option<ThemeMode>,
    accent: Option<String>,
    auto_virtual_mic: Option<bool>,
    tint_app_icon: Option<bool>,
    close_action: Option<CloseAction>,
    language: Option<String>,
}

async fn set_settings(
    State(state): State<AppState>,
    Json(req): Json<SettingsRequest>,
) -> ApiResult<Json<AppInfo>> {
    if let Some(a) = &req.accent {
        if !is_hex_color(a) {
            return Err(bad_request(tr!(
                "颜色格式应为 #rrggbb",
                "Colour must be #rrggbb"
            )));
        }
    }
    if let Some(l) = &req.language {
        if !crate::i18n::LANGUAGES.contains(&l.as_str()) {
            return Err(bad_request(trf!(
                "不支持的语言 {l}",
                "Unsupported language {l}"
            )));
        }
        crate::i18n::apply(l);
    }
    state.settings.update(|s| {
        if let Some(v) = req.language {
            s.language = v;
        }
        if let Some(v) = req.theme_mode {
            s.theme_mode = v;
        }
        if let Some(v) = req.accent {
            s.accent = v.to_ascii_lowercase();
        }
        if let Some(v) = req.auto_virtual_mic {
            s.auto_virtual_mic = v;
        }
        if let Some(v) = req.tint_app_icon {
            s.tint_app_icon = v;
        }
        if let Some(v) = req.close_action {
            s.close_action = v;
        }
    });
    if let Some(hook) = &state.hooks.on_settings {
        hook(&state.settings.get());
    }
    Ok(Json(app_info(&state)))
}

/// 在电脑上弹出系统「选择文件夹」对话框（给设置页的「浏览…」按钮用）。
/// Show the native "pick folder" dialog on the PC (for the settings page's Browse button).
#[derive(Debug, Serialize)]
struct PickedFolder {
    path: Option<String>,
}

async fn pick_folder(State(state): State<AppState>) -> ApiResult<Json<PickedFolder>> {
    let start = state.library.root();
    let picked = blocking(move || {
        rfd::FileDialog::new()
            .set_title(tr!("选择音效库文件夹", "Choose the sound library folder"))
            .set_directory(start)
            .pick_folder()
    })
    .await?;
    Ok(Json(PickedFolder {
        path: picked.map(|p| p.display().to_string()),
    }))
}

/// 关闭询问的回答：`tray` 最小化到托盘、`quit` 退出、`cancel` 取消；`remember` 记住选择。
/// Answer to the close prompt: `tray`, `quit` or `cancel`; `remember` saves the choice.
#[derive(Debug, Deserialize)]
struct WindowRequest {
    action: String,
    #[serde(default)]
    remember: bool,
}

async fn window_action(
    State(state): State<AppState>,
    Json(req): Json<WindowRequest>,
) -> ApiResult<StatusCode> {
    state
        .close_prompt
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let action = match req.action.as_str() {
        "tray" => WindowAction::Tray,
        "quit" => WindowAction::Quit,
        "cancel" => return Ok(StatusCode::NO_CONTENT),
        other => {
            return Err(bad_request(trf!(
                "未知操作 {other}",
                "Unknown action {other}"
            )))
        }
    };
    if req.remember {
        state.settings.update(|s| {
            s.close_action = match action {
                WindowAction::Tray => CloseAction::Tray,
                WindowAction::Quit => CloseAction::Quit,
            }
        });
    }
    if let Some(hook) = &state.hooks.on_window_action {
        hook(action);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 手机遥控开关（字段可选）：`enabled` 立即生效并记住；`regenerate` 重新生成访问码
/// （已扫码的手机需要重新扫）。
/// Phone-remote switch (fields optional): `enabled` takes effect immediately and is
/// remembered; `regenerate` makes a new access code (phones must re-scan).
#[derive(Debug, Deserialize)]
struct RemoteRequest {
    enabled: Option<bool>,
    #[serde(default)]
    regenerate: bool,
}

async fn get_remote(State(state): State<AppState>) -> Json<RemoteStatus> {
    Json(state.remote.status())
}

async fn set_remote(
    State(state): State<AppState>,
    Json(req): Json<RemoteRequest>,
) -> Json<RemoteStatus> {
    if req.regenerate {
        let t = crate::remote::new_token();
        state.remote.set_token(t.clone());
        state.settings.update(|s| s.remote_token = Some(t));
    }
    match req.enabled {
        Some(true) => match state.remote.start().await {
            Ok(()) => state.remote.set_error(None),
            Err(e) => state.remote.set_error(Some(e)),
        },
        Some(false) => state.remote.stop(),
        None => {}
    }
    if let Some(v) = req.enabled {
        state.settings.update(|s| s.lan_remote = v);
    }
    Json(state.remote.status())
}

/// 手机访问地址的二维码（SVG）；未开启时 404。QR code of the phone URL (SVG); 404 when off.
async fn remote_qr(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(url) = state.remote.status().url else {
        return (
            StatusCode::NOT_FOUND,
            tr!("手机遥控未开启", "Phone remote is off"),
        )
            .into_response();
    };
    match qr_svg(&url) {
        Ok(svg) => (
            [
                (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            svg,
        )
            .into_response(),
        Err(e) => internal(e).into_response(),
    }
}

/// 实时状态（UI 每 100 ms 轮询）：正在播放的音效及进度 + 麦克风电平。
/// Live state (polled by the UI every 100 ms): playing sounds with progress + mic level.
#[derive(Debug, Serialize)]
struct LiveState {
    playing: Vec<PlayingItem>,
    mic_level: f32,
    mic_connected: bool,
    /// 桌面 App 请求弹出「关闭窗口」询问。The desktop app wants the close prompt shown.
    close_prompt: bool,
}

#[derive(Debug, Serialize)]
struct PlayingItem {
    id: SoundId,
    /// 进度 0..1。Progress 0..1.
    progress: f32,
}

async fn get_live(State(state): State<AppState>) -> Json<LiveState> {
    Json(LiveState {
        playing: state
            .playing
            .snapshot()
            .into_iter()
            .map(|(id, progress)| PlayingItem { id, progress })
            .collect(),
        mic_level: state.mic.level(),
        mic_connected: state.mic.status().connected,
        close_prompt: state
            .close_prompt
            .load(std::sync::atomic::Ordering::Relaxed),
    })
}

// ---------------------------------------------------------------------------
// 设备与麦克风 Devices and mic
// ---------------------------------------------------------------------------

/// 输入设备（麦克风候选）。Input device (mic candidate).
#[derive(Debug, Serialize)]
struct InputItem {
    name: String,
    is_default: bool,
    /// 是否为虚拟设备（一般不当麦克风用）。Whether virtual (normally not used as a mic).
    #[serde(rename = "virtual")]
    is_virtual: bool,
}

/// 输出设备（混音发送目标）。Output device (where the mix is sent).
#[derive(Debug, Serialize)]
struct OutputItem {
    name: String,
    is_default: bool,
    /// `virtual`：虚拟音频线（游戏能收到）；`speaker`：扬声器 / 耳机（仅试听）。
    /// `virtual`: a virtual cable (games receive it); `speaker`: local preview only.
    kind: OutputKind,
    /// 选它时游戏里该选的麦克风。The mic games should pick when this is selected.
    game_mic: Option<String>,
}

/// 设备列表：输出与输入，以及当前输出设备。Device lists plus the current output.
#[derive(Debug, Serialize)]
struct DevicesResponse {
    outputs: Vec<OutputItem>,
    /// 可选的麦克风：已排除与当前输出构成反馈环的虚拟设备，以及立体声混音等回录设备。
    /// Selectable mics: excludes the virtual device that would loop with the current
    /// output, and loopback devices such as Stereo Mix.
    inputs: Vec<InputItem>,
    current: AudioInfo,
}

/// 列出可用输入/输出设备（每次调用重新枚举，新接入的设备会出现）。
/// List available input/output devices (re-enumerated on every call, so newly
/// connected devices show up).
async fn list_devices(State(state): State<AppState>) -> ApiResult<Json<DevicesResponse>> {
    let devices = blocking(|| CpalBackend::new().enumerate_devices())
        .await?
        .map_err(internal)?;
    let current = state.output_info();
    let input_names: Vec<String> = devices
        .iter()
        .filter(|d| !d.supported_input.is_empty())
        .map(|d| d.name.clone())
        .collect();

    let outputs = devices
        .iter()
        .filter(|d| !d.supported_output.is_empty())
        .map(|d| {
            let (kind, game_mic) = describe_output(&d.name, &input_names);
            OutputItem {
                name: d.name.clone(),
                is_default: d.is_default_output,
                kind,
                game_mic,
            }
        })
        .collect();
    let inputs = devices
        .iter()
        .filter(|d| !d.supported_input.is_empty())
        .filter(|d| mic::input_risk(&d.name, &current.output_device).is_none())
        .map(|d| InputItem {
            name: d.name.clone(),
            is_default: d.is_default_input,
            is_virtual: mic::is_virtual_device(&d.name),
        })
        .collect();

    Ok(Json(DevicesResponse {
        outputs,
        inputs,
        current,
    }))
}

/// 修改试听（字段可选）：开关、设备（`null` = 系统默认播放设备）、音量。
/// Update monitoring (fields optional): switch, device (`null` = default playback),
/// volume.
#[derive(Debug, Deserialize)]
struct MonitorPatch {
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    device: Option<Option<String>>,
    gain_db: Option<f32>,
}

async fn get_monitor(State(state): State<AppState>) -> Json<MonitorStatus> {
    Json(state.monitor.status())
}

async fn set_monitor(
    State(state): State<AppState>,
    Json(req): Json<MonitorPatch>,
) -> ApiResult<Json<MonitorStatus>> {
    if let Some(db) = req.gain_db.filter(|d| d.is_finite()) {
        let db = db.clamp(-40.0, 12.0);
        state.monitor.set_gain_db(db);
        state.settings.update(|s| s.monitor_gain_db = db);
    }
    if req.enabled.is_some() || req.device.is_some() {
        let current = state.monitor.status();
        let enabled = req.enabled.unwrap_or(current.enabled);
        let device = req
            .device
            .unwrap_or(current.selection)
            .filter(|d| !d.trim().is_empty());
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        state
            .monitor_tx
            .send(MonitorRequest::Configure {
                enabled,
                device,
                reply: reply_tx,
            })
            .map_err(internal)?;
        // 打不开（如与输出相同）不算错误：状态里带着原因。
        // Failing to open (e.g. same as output) isn't an error: the status carries the reason.
        let _ = blocking(move || reply_rx.recv_timeout(Duration::from_secs(10))).await?;
    }
    Ok(Json(state.monitor.status()))
}

/// 选择输出设备：`device` 为 `null` 表示系统默认。
/// Select the output device: `device: null` means the system default.
#[derive(Debug, Deserialize)]
struct SelectOutputRequest {
    device: Option<String>,
}

/// 运行中切换输出设备（不用重启）。成功后记住选择；失败返回 400 并恢复原设备。
/// Switch the output device at runtime (no restart). Remembered on success; on
/// failure 400 and the previous device is restored.
async fn select_output(
    State(state): State<AppState>,
    Json(req): Json<SelectOutputRequest>,
) -> ApiResult<Json<AudioInfo>> {
    let device = req.device.filter(|d| !d.trim().is_empty());
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    state
        .out_tx
        .send(OutputRequest::Select {
            device,
            reply: reply_tx,
        })
        .map_err(internal)?;
    let reply = blocking(move || reply_rx.recv_timeout(Duration::from_secs(15))).await?;
    match reply {
        Ok(Ok(_)) => Ok(Json(state.output_info())),
        Ok(Err(msg)) => Err(bad_request(msg)),
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            tr!("切换输出设备超时", "Switching the output device timed out").into(),
        )),
    }
}

/// 麦克风状态响应：设备状态 + 音量/静音/电平。
/// Mic status response: device status + volume / mute / level.
#[derive(Debug, Serialize)]
struct MicResponse {
    #[serde(flatten)]
    status: mic::MicStatus,
    gain_db: f32,
    muted: bool,
    /// 电平 0..1（增益后、静音前）。Level 0..1 (post-gain, pre-mute).
    level: f32,
}

fn mic_response(state: &AppState) -> MicResponse {
    MicResponse {
        status: state.mic.status(),
        gain_db: state.mic.gain_db(),
        muted: state.mic.muted(),
        level: state.mic.level(),
    }
}

async fn get_mic(State(state): State<AppState>) -> Json<MicResponse> {
    Json(mic_response(&state))
}

/// 选择麦克风：`device` 为 `null` 表示跟随系统默认。
/// Select a mic: `device: null` means follow the system default.
#[derive(Debug, Deserialize)]
struct SelectMicRequest {
    device: Option<String>,
}

/// 运行中切换麦克风。成功后记住选择；失败返回 400 并保留原设备。
/// Switch the mic at runtime. Remembered on success; on failure 400 and the current
/// device is kept.
async fn select_mic(
    State(state): State<AppState>,
    Json(req): Json<SelectMicRequest>,
) -> ApiResult<Json<MicResponse>> {
    let device = req.device.filter(|d| !d.trim().is_empty());
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    state
        .mic_tx
        .send(MicRequest::Select {
            device,
            reply: reply_tx,
        })
        .map_err(internal)?;
    let reply = blocking(move || reply_rx.recv_timeout(Duration::from_secs(10))).await?;
    match reply {
        Ok(Ok(())) => Ok(Json(mic_response(&state))),
        Ok(Err(msg)) => Err(bad_request(msg)),
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            tr!("切换麦克风超时", "Switching the microphone timed out").into(),
        )),
    }
}

/// 修改麦克风参数（字段可选）。Update mic parameters (fields optional).
#[derive(Debug, Deserialize)]
struct SetMicRequest {
    gain_db: Option<f32>,
    muted: Option<bool>,
}

async fn set_mic(
    State(state): State<AppState>,
    Json(req): Json<SetMicRequest>,
) -> Json<MicResponse> {
    if let Some(db) = req.gain_db.filter(|d| d.is_finite()) {
        let db = db.clamp(-60.0, 24.0);
        state.mic.set_gain_db(db);
        // 记住麦克风音量。Remember the mic volume.
        state.settings.update(|s| s.mic_gain_db = db);
    }
    if let Some(m) = req.muted {
        state.mic.set_muted(m);
    }
    Json(mic_response(&state))
}

// ---------------------------------------------------------------------------
// 音效库与文件夹 Library and folders
// ---------------------------------------------------------------------------

/// 音效列表项。Sound list item.
#[derive(Debug, Serialize)]
struct SoundItem {
    id: SoundId,
    /// 名称 = 文件名去掉扩展名。Name = file name without extension.
    name: String,
    /// 所在文件夹（相对音效库根目录，根目录为空串）。Folder (relative; "" = root).
    folder: String,
    file_name: String,
    /// 循环播放（否 = 单次）。Loop (false = once).
    looping: bool,
    /// 独占（否 = 叠加）。Exclusive (false = overlap).
    exclusive: bool,
    gain_db: f32,
    speed: f32,
    hotkey: Option<String>,
    /// 时长（秒），加载后才知道。Duration (seconds), known once loaded.
    duration_secs: Option<f32>,
    /// 是否已解码进内存。Whether decoded into memory.
    loaded: bool,
    /// 加载失败原因。Load failure reason.
    error: Option<String>,
}

/// 整个音效库视图。The whole library view.
#[derive(Debug, Serialize)]
struct LibraryView {
    /// 根目录。Root directory.
    root: String,
    /// 所有文件夹（相对路径，已排序）。All folders (relative, sorted).
    folders: Vec<String>,
    sounds: Vec<SoundItem>,
}

async fn get_library(State(state): State<AppState>) -> ApiResult<Json<LibraryView>> {
    let s = state.clone();
    blocking(move || Json(s.library_view())).await
}

async fn list_sounds(State(state): State<AppState>) -> Json<Vec<SoundItem>> {
    Json(
        state
            .library
            .entries()
            .iter()
            .map(|e| state.item(e))
            .collect(),
    )
}

/// 重新扫描磁盘（在资源管理器里改过文件后）。Rescan the disk (after changes in a file manager).
async fn rescan_library(State(state): State<AppState>) -> ApiResult<Json<LibraryView>> {
    let s = state.clone();
    blocking(move || {
        let diff = s.library.rescan();
        s.apply_diff(&diff);
        Json(s.library_view())
    })
    .await
}

/// 修改音效库位置。Change the library location.
#[derive(Debug, Deserialize)]
struct LocationRequest {
    path: String,
    /// 把当前音效库的文件一起移过去。Move the current library's files along.
    #[serde(default)]
    move_contents: bool,
}

async fn set_library_location(
    State(state): State<AppState>,
    Json(req): Json<LocationRequest>,
) -> ApiResult<Json<LibraryView>> {
    let path = PathBuf::from(req.path.trim());
    if !path.is_absolute() {
        return Err(bad_request(tr!(
            "请填写完整路径，如 D:\\音效",
            "Enter a full path, e.g. D:\\Sounds"
        )));
    }
    let s = state.clone();
    blocking(move || {
        // 先卸载并解绑旧库的所有音效，再切换，最后按新库绑定与预加载。
        // Unload and unbind everything from the old library, switch, then bind and
        // preload the new one.
        let old: Vec<SoundId> = s.library.entries().iter().map(|e| e.meta.id).collect();
        s.library
            .set_root(path.clone(), req.move_contents)
            .map_err(bad_request)?;
        for id in old {
            s.bind_hotkey_async(id, None);
        }
        s.player.unload_all();
        s.settings.update(|st| st.library_dir = Some(path));
        let added: Vec<SoundId> = s.library.entries().iter().map(|e| e.meta.id).collect();
        s.apply_diff(&ScanDiff {
            added,
            removed: Vec::new(),
        });
        Ok(Json(s.library_view()))
    })
    .await?
}

/// 在系统文件管理器中打开音效库。Open the library in the system file manager.
async fn open_library(State(state): State<AppState>) -> ApiResult<StatusCode> {
    let root = state.library.root();
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(&root)
        .spawn()
        .map_err(|e| internal(trf!("无法打开文件夹：{e}", "Can't open the folder: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct FolderRequest {
    path: String,
}

async fn create_folder(
    State(state): State<AppState>,
    Json(req): Json<FolderRequest>,
) -> ApiResult<Json<LibraryView>> {
    let s = state.clone();
    blocking(move || {
        s.library.create_folder(&req.path).map_err(bad_request)?;
        Ok(Json(s.library_view()))
    })
    .await?
}

/// 重命名 / 移动文件夹：`to` 为新的完整相对路径。Rename / move: `to` is the new full path.
#[derive(Debug, Deserialize)]
struct RenameFolderRequest {
    from: String,
    to: String,
}

async fn rename_folder(
    State(state): State<AppState>,
    Json(req): Json<RenameFolderRequest>,
) -> ApiResult<Json<LibraryView>> {
    let s = state.clone();
    blocking(move || {
        s.library
            .rename_folder(&req.from, &req.to)
            .map_err(bad_request)?;
        Ok(Json(s.library_view()))
    })
    .await?
}

async fn delete_folder(
    State(state): State<AppState>,
    Json(req): Json<FolderRequest>,
) -> ApiResult<Json<LibraryView>> {
    let s = state.clone();
    blocking(move || {
        s.library.delete_folder(&req.path).map_err(bad_request)?;
        Ok(Json(s.library_view()))
    })
    .await?
}

/// 上传导入音效（multipart：`file` 必填，`folder` 可选）。文件写进音效库对应文件夹并
/// 立即解码校验；桌面拖拽、文件选择、手机上传都走这里。
/// Import a sound by upload (multipart: `file` required, `folder` optional). The file is
/// written into the library folder and decoded right away to validate it; desktop
/// drag-and-drop, the file picker and phone uploads all use this.
async fn import_sound(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Json<SoundItem>> {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut folder = String::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(e.to_string()))?
    {
        match field.name() {
            Some("file") => {
                let file_name = field.file_name().unwrap_or("sound").to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| bad_request(e.to_string()))?;
                file = Some((file_name, bytes.to_vec()));
            }
            Some("folder") => {
                folder = field.text().await.map_err(|e| bad_request(e.to_string()))?;
            }
            _ => {}
        }
    }
    let (file_name, bytes) = file.ok_or_else(|| bad_request(tr!("缺少文件", "Missing file")))?;

    let s = state.clone();
    blocking(move || {
        let id = s
            .library
            .import(&folder, &file_name, &bytes)
            .map_err(bad_request)?;
        let spec = s
            .library
            .spec(id)
            .ok_or_else(|| internal(tr!("导入后未找到音效", "Sound not found after import")))?;
        match LoadedSound::load(spec) {
            Ok(loaded) => {
                log::info!(
                    "已导入音效 imported {}: {:.1}s @ {} Hz",
                    loaded.spec.name,
                    loaded.duration_secs(),
                    loaded.sample_rate()
                );
                s.player.install(loaded);
                let e = s
                    .library
                    .entry(id)
                    .ok_or_else(|| internal(tr!("音效不存在", "Sound not found")))?;
                Ok(Json(s.item(&e)))
            }
            Err(e) => {
                // 解码失败：删掉刚写入的文件。Decode failed: remove the written file.
                let _ = s.library.delete_sound(id);
                Err(bad_request(trf!(
                    "无法读取音频文件：{e}",
                    "Can't read the audio file: {e}"
                )))
            }
        }
    })
    .await?
}

/// 反序列化「可能为 null 的可选字段」：缺省 = 不改，`null` = 清空。
/// Deserialize an optional nullable field: absent = unchanged, `null` = clear.
fn double_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

/// 编辑音效（字段可选）。`name` 会重命名文件，`folder` 会移动文件，`hotkey: null` 清除快捷键。
/// Edit a sound (fields optional). `name` renames the file, `folder` moves it,
/// `hotkey: null` clears the hotkey.
#[derive(Debug, Deserialize)]
struct UpdateSoundRequest {
    name: Option<String>,
    folder: Option<String>,
    looping: Option<bool>,
    exclusive: Option<bool>,
    gain_db: Option<f32>,
    speed: Option<f32>,
    #[serde(default, deserialize_with = "double_option")]
    hotkey: Option<Option<String>>,
}

async fn update_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<UpdateSoundRequest>,
) -> ApiResult<Json<SoundItem>> {
    let current = state.library.entry(id).ok_or((
        StatusCode::NOT_FOUND,
        tr!("音效不存在", "Sound not found").to_string(),
    ))?;

    // 先绑快捷键：失败则整个修改不生效。Bind the hotkey first: on failure nothing changes.
    let hotkey = req
        .hotkey
        .map(|h| h.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    if let Some(hk) = &hotkey {
        if *hk != current.meta.hotkey {
            state
                .bind_hotkey(id, hk.clone())
                .await
                .map_err(bad_request)?;
        }
    }

    let s = state.clone();
    blocking(move || {
        // 文件操作：重命名、移动。File operations: rename, move.
        if let Some(name) = req.name.as_deref().map(str::trim) {
            if name.is_empty() {
                return Err(bad_request(tr!("名称不能为空", "Name can't be empty")));
            }
            if name != current.name() {
                s.library.rename_sound(id, name).map_err(bad_request)?;
            }
        }
        if let Some(folder) = &req.folder {
            s.library.move_sound(id, folder).map_err(bad_request)?;
        }

        // 播放参数：同时更新音频线程（已加载时）与音效库。
        // Playback parameters: update the audio thread (if loaded) and the library.
        let mode = PlayMode {
            looping: req.looping.unwrap_or(current.meta.mode.looping),
            exclusive: req.exclusive.unwrap_or(current.meta.mode.exclusive),
        };
        if mode != current.meta.mode {
            s.player.send(BoardCommand::SetMode { sound_id: id, mode });
        }
        let gain_db = req
            .gain_db
            .filter(|g| g.is_finite())
            .map(|g| g.clamp(-60.0, 24.0));
        if let Some(gain_db) = gain_db {
            s.player.send(BoardCommand::SetGain {
                sound_id: id,
                gain_db,
            });
        }
        let speed = req
            .speed
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| v.clamp(0.25, 4.0));
        if let Some(speed) = speed {
            s.player.send(BoardCommand::SetSpeed {
                sound_id: id,
                speed,
            });
        }
        s.library.update_meta(id, |m| {
            m.mode = mode;
            if let Some(g) = gain_db {
                m.gain_db = g;
            }
            if let Some(v) = speed {
                m.speed = v;
            }
            if let Some(hk) = hotkey {
                m.hotkey = hk;
            }
        });
        let e = s.library.entry(id).ok_or((
            StatusCode::NOT_FOUND,
            tr!("音效不存在", "Sound not found").to_string(),
        ))?;
        Ok(Json(s.item(&e)))
    })
    .await?
}

/// 删除音效（文件移到回收站）。Delete a sound (the file goes to the recycle bin).
async fn remove_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> ApiResult<StatusCode> {
    state.bind_hotkey_async(id, None);
    state.player.unload(id);
    let s = state.clone();
    blocking(move || s.library.delete_sound(id).map_err(bad_request)).await??;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 播放控制 Playback control
// ---------------------------------------------------------------------------

async fn trigger_sound(State(state): State<AppState>, Path(id): Path<SoundId>) -> StatusCode {
    state.player.trigger(id);
    StatusCode::OK
}

async fn stop_sound(State(state): State<AppState>, Path(id): Path<SoundId>) -> StatusCode {
    state.player.send(BoardCommand::Stop { sound_id: id });
    StatusCode::OK
}

/// 拖动进度条：跳到 `position`（0..1）。Progress-bar drag: seek to `position` (0..1).
#[derive(Debug, Deserialize)]
struct SeekRequest {
    position: f32,
}

async fn seek_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<SeekRequest>,
) -> StatusCode {
    state.player.send(BoardCommand::Seek {
        sound_id: id,
        position: req.position,
    });
    StatusCode::OK
}

async fn stop_all(State(state): State<AppState>) -> StatusCode {
    state.player.send(BoardCommand::StopAll);
    StatusCode::OK
}
