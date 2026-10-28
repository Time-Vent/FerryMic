//! HTTP API：音效库、播放控制、麦克风、设备、设置、实时状态。
//! HTTP API: sound library, playback control, mic, devices, settings, live state.
//!
//! 所有阻塞操作（解码、设备枚举、等待其它线程回复）都放到 `spawn_blocking`，不卡住
//! tokio 工作线程。
//! Every blocking operation (decoding, device enumeration, waiting for other threads)
//! runs in `spawn_blocking` so tokio workers are never stalled.

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
use soundboard::{BoardCommand, LoadedSound, SoundId, SoundSpec, TriggerMode};

use crate::audio::{AudioInfo, PlayingSet};
use crate::hotkeys::HotkeyCommand;
use crate::library::{Library, SoundRuntime};
use crate::mic::{self, MicControl, MicRequest};
use crate::settings::SettingsStore;

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

/// 共享应用状态。Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// 命令发送端（→ 音频线程）。Command sender (→ audio thread).
    pub cmd_tx: Sender<BoardCommand>,
    /// 热键指令发送端（→ 热键线程）。Hotkey command sender (→ hotkey thread).
    pub hotkey_tx: Sender<HotkeyCommand>,
    /// 音效库（持久化）。Sound library (persisted).
    pub library: Arc<Library>,
    /// 麦克风参数、电平与设备状态。Mic parameters, level and device status.
    pub mic: Arc<MicControl>,
    /// 麦克风请求发送端（→ 麦克风线程）。Mic request sender (→ mic thread).
    pub mic_tx: Sender<MicRequest>,
    /// 实际打开的输出设备。Output device actually opened.
    pub audio_info: Arc<AudioInfo>,
    /// 正在播放的音效。Playing sounds.
    pub playing: Arc<PlayingSet>,
    /// 持久化设置。Persisted settings.
    pub settings: Arc<SettingsStore>,
    /// 数据目录。Data directory.
    pub data_dir: PathBuf,
    /// 实际监听端口。Actual listening port.
    pub port: u16,
    /// 是否正在局域网监听（手机遥控可用）。Whether listening on the LAN.
    pub lan_active: bool,
}

impl AppState {
    fn send(&self, cmd: BoardCommand) -> ApiResult<()> {
        self.cmd_tx.send(cmd).map_err(internal)
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
            .unwrap_or_else(|_| Err("热键线程无响应".into()))
    }

    fn item(&self, spec: &SoundSpec) -> SoundItem {
        SoundItem::new(spec, self.library.runtime(spec.id))
    }
}

/// 构建 API 路由（挂在 `/api` 下）。Build the API routes (mounted under `/api`).
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/app", get(get_app))
        .route("/settings", post(set_settings))
        .route("/live", get(get_live))
        .route("/devices", get(list_devices))
        .route("/mic", get(get_mic).post(set_mic))
        .route("/mic/device", post(select_mic))
        .route("/sounds", get(list_sounds).post(add_sound))
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
        .route("/sounds/:id/gain", post(set_gain))
        .route("/sounds/:id/speed", post(set_speed))
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
    data_dir: String,
    output: AudioInfo,
    port: u16,
    /// 设置中的手机遥控开关（重启后生效）。Phone-remote setting (applies on restart).
    lan_remote: bool,
    /// 当前是否已在局域网监听。Whether currently listening on the LAN.
    lan_active: bool,
    /// 手机访问地址（局域网监听时）。Phone URL (when listening on the LAN).
    lan_url: Option<String>,
}

fn app_info(state: &AppState) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.data_dir.display().to_string(),
        output: (*state.audio_info).clone(),
        port: state.port,
        lan_remote: state.settings.get().lan_remote,
        lan_active: state.lan_active,
        lan_url: state
            .lan_active
            .then(crate::lan_ip)
            .flatten()
            .map(|ip| format!("http://{ip}:{}", state.port)),
    }
}

async fn get_app(State(state): State<AppState>) -> Json<AppInfo> {
    Json(app_info(&state))
}

/// 修改设置（字段可选）。Update settings (fields optional).
#[derive(Debug, Deserialize)]
struct SettingsRequest {
    lan_remote: Option<bool>,
}

async fn set_settings(
    State(state): State<AppState>,
    Json(req): Json<SettingsRequest>,
) -> Json<AppInfo> {
    if let Some(v) = req.lan_remote {
        state.settings.update(|s| s.lan_remote = v);
    }
    Json(app_info(&state))
}

/// 实时状态（UI 每 100 ms 轮询）：正在播放的音效 + 麦克风电平。
/// Live state (polled by the UI every 100 ms): playing sounds + mic level.
#[derive(Debug, Serialize)]
struct LiveState {
    playing: Vec<SoundId>,
    mic_level: f32,
    mic_connected: bool,
}

async fn get_live(State(state): State<AppState>) -> Json<LiveState> {
    Json(LiveState {
        playing: state.playing.ids(),
        mic_level: state.mic.level(),
        mic_connected: state.mic.status().connected,
    })
}

// ---------------------------------------------------------------------------
// 设备与麦克风 Devices and mic
// ---------------------------------------------------------------------------

/// 设备信息（供 UI 选择）。Device info (for UI selection).
#[derive(Debug, Serialize)]
struct DeviceItem {
    name: String,
    is_default: bool,
}

/// 设备列表：输出与输入，以及当前输出设备。Device lists plus the current output.
#[derive(Debug, Serialize)]
struct DevicesResponse {
    outputs: Vec<DeviceItem>,
    /// 可选的麦克风：已排除与输出构成反馈环的虚拟设备，以及立体声混音等回录设备。
    /// Selectable mics: excludes the virtual device that would loop with the output,
    /// and loopback devices such as Stereo Mix.
    inputs: Vec<DeviceItem>,
    current: AudioInfo,
}

/// 列出可用输入/输出设备（每次调用重新枚举，新接入的设备会出现）。
/// List available input/output devices (re-enumerated on every call, so newly
/// connected devices show up).
async fn list_devices(State(state): State<AppState>) -> ApiResult<Json<DevicesResponse>> {
    let devices = tokio::task::spawn_blocking(|| CpalBackend::new().enumerate_devices())
        .await
        .map_err(internal)?
        .map_err(internal)?;
    let output_name = &state.audio_info.output_device;

    let outputs = devices
        .iter()
        .filter(|d| !d.supported_output.is_empty())
        .map(|d| DeviceItem {
            name: d.name.clone(),
            is_default: d.is_default_output,
        })
        .collect();
    let inputs = devices
        .iter()
        .filter(|d| !d.supported_input.is_empty())
        .filter(|d| mic::input_risk(&d.name, output_name).is_none())
        .map(|d| DeviceItem {
            name: d.name.clone(),
            is_default: d.is_default_input,
        })
        .collect();

    Ok(Json(DevicesResponse {
        outputs,
        inputs,
        current: (*state.audio_info).clone(),
    }))
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
    let reply = tokio::task::spawn_blocking(move || reply_rx.recv_timeout(Duration::from_secs(10)))
        .await
        .map_err(internal)?;
    match reply {
        Ok(Ok(())) => Ok(Json(mic_response(&state))),
        Ok(Err(msg)) => Err(bad_request(msg)),
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            "切换麦克风超时 mic switch timed out".into(),
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
// 音效库 Sound library
// ---------------------------------------------------------------------------

/// 音效列表项。Sound list item.
#[derive(Debug, Serialize)]
struct SoundItem {
    id: SoundId,
    name: String,
    /// 音频文件名（不含目录）。Audio file name (without directory).
    file_name: String,
    trigger_mode: TriggerMode,
    gain_db: f32,
    speed: f32,
    hotkey: Option<String>,
    /// 时长（秒）。Duration (seconds).
    duration_secs: Option<f32>,
    /// 加载失败原因（文件丢失等）。Load failure reason (missing file etc.).
    error: Option<String>,
}

impl SoundItem {
    fn new(s: &SoundSpec, rt: SoundRuntime) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            file_name: s
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            trigger_mode: s.trigger_mode,
            gain_db: s.gain_db,
            speed: s.speed,
            hotkey: s.hotkey.clone(),
            duration_secs: rt.duration_secs,
            error: rt.error,
        }
    }
}

async fn list_sounds(State(state): State<AppState>) -> Json<Vec<SoundItem>> {
    Json(state.library.list().iter().map(|s| state.item(s)).collect())
}

/// 解码并加入音效库（解码失败不入库）。快捷键绑定失败时音效照常加入，但不带快捷键。
/// Decode and add to the library (not added if decoding fails). If the hotkey can't
/// be bound the sound is still added, without a hotkey.
async fn load_and_add(state: &AppState, mut spec: SoundSpec) -> ApiResult<SoundItem> {
    let loaded = tokio::task::spawn_blocking(move || LoadedSound::load(spec))
        .await
        .map_err(internal)?
        .map_err(|e| bad_request(format!("无法加载音频文件：{e}")))?;
    log::info!(
        "已加载音效 loaded sound {}: {:.1}s @ {} Hz",
        loaded.spec.name,
        loaded.duration_secs(),
        loaded.sample_rate()
    );
    spec = loaded.spec.clone();
    let id = spec.id;
    state.library.set_runtime(
        id,
        SoundRuntime {
            duration_secs: Some(loaded.duration_secs()),
            error: None,
        },
    );
    state.send(BoardCommand::AddSound(Box::new(loaded)))?;

    if spec.hotkey.is_some() {
        if let Err(e) = state.bind_hotkey(id, spec.hotkey.clone()).await {
            log::warn!("快捷键未绑定 hotkey not bound: {e}");
            spec.hotkey = None;
        }
    }
    state.library.insert(spec.clone());
    Ok(state.item(&spec))
}

/// 按本机路径添加音效（文件留在原处）。Add a sound by local path (file stays put).
#[derive(Debug, Deserialize)]
struct AddSoundRequest {
    name: String,
    file_path: String,
    trigger_mode: TriggerMode,
    #[serde(default)]
    gain_db: f32,
    #[serde(default = "default_speed")]
    speed: f32,
    #[serde(default)]
    hotkey: Option<String>,
}

fn default_speed() -> f32 {
    1.0
}

async fn add_sound(
    State(state): State<AppState>,
    Json(req): Json<AddSoundRequest>,
) -> ApiResult<Json<SoundItem>> {
    let spec = SoundSpec {
        id: state.library.alloc_id(),
        name: req.name,
        file_path: req.file_path.into(),
        trigger_mode: req.trigger_mode,
        gain_db: req.gain_db,
        speed: req.speed,
        hotkey: req.hotkey,
    };
    Ok(Json(load_and_add(&state, spec).await?))
}

/// 上传导入音效（multipart：`file` 必填，`name` 可选）。文件复制到数据目录的 `sounds/`，
/// 桌面拖拽、文件选择、手机上传都走这里。
/// Import a sound by upload (multipart: `file` required, `name` optional). The file is
/// copied into `sounds/` in the data directory; desktop drag-and-drop, the file picker
/// and phone uploads all use this.
async fn import_sound(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Json<SoundItem>> {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut name: Option<String> = None;
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
            Some("name") => {
                name = Some(field.text().await.map_err(|e| bad_request(e.to_string()))?);
            }
            _ => {}
        }
    }
    let (file_name, bytes) = file.ok_or_else(|| bad_request("缺少文件 missing file"))?;

    let id = state.library.alloc_id();
    let path = state.library.import_path(id, &file_name);
    let dir = state.library.sounds_dir().to_path_buf();
    let write_path = path.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&write_path, bytes)
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;

    // 默认名称 = 文件名去掉扩展名。Default name = file name without extension.
    let display_name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            std::path::Path::new(&file_name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "音效".into())
        });
    let spec = SoundSpec {
        id,
        name: display_name,
        file_path: path.clone(),
        trigger_mode: TriggerMode::OneShot,
        gain_db: 0.0,
        speed: 1.0,
        hotkey: None,
    };
    match load_and_add(&state, spec).await {
        Ok(item) => Ok(Json(item)),
        Err(e) => {
            // 解码失败：删掉刚复制的文件。Decode failed: remove the copied file.
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// 反序列化「可能为 null 的可选字段」：缺省 = 不改，`null` = 清空。
/// Deserialize an optional nullable field: absent = unchanged, `null` = clear.
fn double_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

/// 编辑音效（字段可选）。`hotkey: null` 表示清除快捷键。
/// Edit a sound (fields optional). `hotkey: null` clears the hotkey.
#[derive(Debug, Deserialize)]
struct UpdateSoundRequest {
    name: Option<String>,
    trigger_mode: Option<TriggerMode>,
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
    let current = state
        .library
        .get(id)
        .ok_or((StatusCode::NOT_FOUND, "音效不存在".to_string()))?;

    // 先绑快捷键：失败则整个修改不生效。Bind the hotkey first: on failure nothing changes.
    let hotkey = req
        .hotkey
        .map(|h| h.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    if let Some(hk) = &hotkey {
        if *hk != current.hotkey {
            state
                .bind_hotkey(id, hk.clone())
                .await
                .map_err(bad_request)?;
        }
    }

    let name = req.name.map(|n| n.trim().to_string());
    if name.as_deref() == Some("") {
        return Err(bad_request("名称不能为空"));
    }
    if let Some(mode) = req.trigger_mode.filter(|m| *m != current.trigger_mode) {
        state.send(BoardCommand::SetMode { sound_id: id, mode })?;
    }
    let gain_db = req
        .gain_db
        .filter(|g| g.is_finite())
        .map(|g| g.clamp(-60.0, 24.0));
    if let Some(gain_db) = gain_db {
        state.send(BoardCommand::SetGain {
            sound_id: id,
            gain_db,
        })?;
    }
    let speed = req.speed.filter(|s| s.is_finite() && *s > 0.0);
    if let Some(speed) = speed {
        state.send(BoardCommand::SetSpeed {
            sound_id: id,
            speed,
        })?;
    }

    let spec = state
        .library
        .update(id, |s| {
            if let Some(n) = name {
                s.name = n;
            }
            if let Some(m) = req.trigger_mode {
                s.trigger_mode = m;
            }
            if let Some(g) = gain_db {
                s.gain_db = g;
            }
            if let Some(sp) = speed {
                s.speed = sp;
            }
            if let Some(hk) = hotkey {
                s.hotkey = hk;
            }
        })
        .ok_or((StatusCode::NOT_FOUND, "音效不存在".to_string()))?;
    Ok(Json(state.item(&spec)))
}

async fn remove_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> ApiResult<StatusCode> {
    state.send(BoardCommand::RemoveSound { sound_id: id })?;
    let _ = state.hotkey_tx.send(HotkeyCommand::Bind {
        sound_id: id,
        hotkey: None,
        reply: None,
    });
    state.library.remove(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// 播放控制 Playback control
// ---------------------------------------------------------------------------

async fn trigger_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> ApiResult<StatusCode> {
    state.send(BoardCommand::Trigger { sound_id: id })?;
    Ok(StatusCode::OK)
}

async fn stop_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> ApiResult<StatusCode> {
    state.send(BoardCommand::Stop { sound_id: id })?;
    Ok(StatusCode::OK)
}

async fn stop_all(State(state): State<AppState>) -> ApiResult<StatusCode> {
    state.send(BoardCommand::StopAll)?;
    Ok(StatusCode::OK)
}

/// 设置音效增益（兼容旧接口；新界面用 PATCH）。Set gain (legacy; the new UI uses PATCH).
#[derive(Debug, Deserialize)]
struct SetGainRequest {
    gain_db: f32,
}

async fn set_gain(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<SetGainRequest>,
) -> ApiResult<StatusCode> {
    state.send(BoardCommand::SetGain {
        sound_id: id,
        gain_db: req.gain_db,
    })?;
    state.library.update(id, |s| s.gain_db = req.gain_db);
    Ok(StatusCode::OK)
}

/// 设置音效变速（兼容旧接口；新界面用 PATCH）。Set speed (legacy; the new UI uses PATCH).
#[derive(Debug, Deserialize)]
struct SetSpeedRequest {
    speed: f32,
}

async fn set_speed(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<SetSpeedRequest>,
) -> ApiResult<StatusCode> {
    state.send(BoardCommand::SetSpeed {
        sound_id: id,
        speed: req.speed,
    })?;
    state.library.update(id, |s| s.speed = req.speed);
    Ok(StatusCode::OK)
}
