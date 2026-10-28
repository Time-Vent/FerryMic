//! SoundBridge 服务入口：axum Web 服务 + 音频线程。
//! SoundBridge server entry: axum web service + audio thread.
//!
//! 提供：
//! Provides:
//! - 静态文件（Web 前端，桌面浏览器 / 手机浏览器均可访问）。
//!   Static files (web front-end, accessible from desktop or phone browser).
//! - REST API：列出/添加/删除音效、触发音效、调音量/变速。
//!   REST API: list/add/remove sounds, trigger, adjust gain/speed.
//!
//! 手机遥控：手机浏览器打开 `http://<电脑IP>:8321`，通过 API 触发音效，
//! 音频在**电脑**上处理并输出到声卡/虚拟音频线。
//! Phone remote: phone browser opens `http://<PC-IP>:8321`, triggers sounds via API;
//! audio is processed on the **PC** and output to speaker / virtual cable.

mod audio;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use crossbeam_channel::{unbounded, Sender};
use serde::{Deserialize, Serialize};
use soundboard::{BoardCommand, SoundId, SoundSpec, TriggerMode};
use tower_http::services::ServeDir;

/// 共享应用状态。Shared application state.
#[derive(Clone)]
struct AppState {
    /// 命令发送端（→ 音频线程）。Command sender (→ audio thread).
    cmd_tx: Sender<BoardCommand>,
    /// 音效列表（供 API 读取）。Sound list (for API reads).
    sounds: Arc<Mutex<Vec<SoundSpec>>>,
    /// 音效 ID 自增。Auto-incrementing sound id.
    next_id: Arc<AtomicU32>,
}

/// 添加音效的请求体。Request body for adding a sound.
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

/// 音效列表响应（含 id 供后续操作）。Sound list response (with id for later ops).
#[derive(Debug, Serialize)]
struct SoundItem {
    id: SoundId,
    name: String,
    trigger_mode: TriggerMode,
    gain_db: f32,
    speed: f32,
    hotkey: Option<String>,
}

impl From<&SoundSpec> for SoundItem {
    fn from(s: &SoundSpec) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            trigger_mode: s.trigger_mode,
            gain_db: s.gain_db,
            speed: s.speed,
            hotkey: s.hotkey.clone(),
        }
    }
}

/// 健康检查。Health check.
async fn health() -> &'static str {
    "ok"
}

/// 列出所有音效。List all sounds.
async fn list_sounds(State(state): State<AppState>) -> Json<Vec<SoundItem>> {
    let sounds = state.sounds.lock().unwrap();
    let items: Vec<SoundItem> = sounds.iter().map(SoundItem::from).collect();
    Json(items)
}

/// 添加音效。Add a sound.
async fn add_sound(
    State(state): State<AppState>,
    Json(req): Json<AddSoundRequest>,
) -> Result<Json<SoundItem>, (StatusCode, String)> {
    let id = state.next_id.fetch_add(1, Ordering::Relaxed);

    let spec = SoundSpec {
        id,
        name: req.name,
        file_path: req.file_path.into(),
        trigger_mode: req.trigger_mode,
        gain_db: req.gain_db,
        speed: req.speed,
        hotkey: req.hotkey,
    };

    // 发命令给音频线程 + 记录到列表。
    // Send command to audio thread + record in list.
    if let Err(e) = state.cmd_tx.send(BoardCommand::AddSound(spec.clone())) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    state.sounds.lock().unwrap().push(spec.clone());

    Ok(Json(SoundItem::from(&spec)))
}

/// 删除音效。Remove a sound.
async fn remove_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = state.cmd_tx.send(BoardCommand::RemoveSound { sound_id: id }) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    state.sounds.lock().unwrap().retain(|s| s.id != id);
    Ok(StatusCode::NO_CONTENT)
}

/// 触发音效。Trigger a sound.
async fn trigger_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = state.cmd_tx.send(BoardCommand::Trigger { sound_id: id }) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    Ok(StatusCode::OK)
}

/// 停止音效。Stop a sound.
async fn stop_sound(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = state.cmd_tx.send(BoardCommand::Stop { sound_id: id }) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    Ok(StatusCode::OK)
}

/// 设置音效增益。Set a sound's gain.
#[derive(Debug, Deserialize)]
struct SetGainRequest {
    gain_db: f32,
}

async fn set_gain(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<SetGainRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = state.cmd_tx.send(BoardCommand::SetGain {
        sound_id: id,
        gain_db: req.gain_db,
    }) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    if let Some(s) = state.sounds.lock().unwrap().iter_mut().find(|s| s.id == id) {
        s.gain_db = req.gain_db;
    }
    Ok(StatusCode::OK)
}

/// 设置音效变速。Set a sound's speed.
#[derive(Debug, Deserialize)]
struct SetSpeedRequest {
    speed: f32,
}

async fn set_speed(
    State(state): State<AppState>,
    Path(id): Path<SoundId>,
    Json(req): Json<SetSpeedRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Err(e) = state.cmd_tx.send(BoardCommand::SetSpeed {
        sound_id: id,
        speed: req.speed,
    }) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("发送命令失败: {e}")));
    }
    if let Some(s) = state.sounds.lock().unwrap().iter_mut().find(|s| s.id == id) {
        s.speed = req.speed;
    }
    Ok(StatusCode::OK)
}

/// 构建路由。Build the router.
fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/sounds", get(list_sounds).post(add_sound))
        .route(
            "/sounds/:id",
            delete(remove_sound),
        )
        .route("/sounds/:id/trigger", post(trigger_sound))
        .route("/sounds/:id/stop", post(stop_sound))
        .route("/sounds/:id/gain", post(set_gain))
        .route("/sounds/:id/speed", post(set_speed));

    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new("apps/web/dist"))
        .with_state(state)
}

/// 主入口。Main entry.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 命令通道：Web 服务 → 音频线程。
    // Command channel: web service → audio thread.
    let (cmd_tx, cmd_rx) = unbounded::<BoardCommand>();

    // 启动音频线程，句柄保持存活到进程结束。
    // Start audio thread; hold the handle until process exit.
    let stream = audio::spawn_audio_thread(cmd_rx)?;
    let _audio = audio::AudioHandle::new(stream);

    let state = AppState {
        cmd_tx,
        sounds: Arc::new(Mutex::new(Vec::new())),
        next_id: Arc::new(AtomicU32::new(1)),
    };

    let app = router(state);

    let port = std::env::var("SOUNDBRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8321u16);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    log::info!("SoundBridge 服务已启动 listening on http://{addr}");
    log::info!("桌面访问 Desktop: http://127.0.0.1:{port}");
    log::info!("手机访问 Phone (同一局域网 same LAN): http://<电脑IP>:{port}");

    axum::serve(listener, app).await?;
    Ok(())
}
