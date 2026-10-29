//! 手机遥控：按需开关的局域网监听 + 访问码 + 二维码。
//! Phone remote: an on-demand LAN listener + access code + QR code.
//!
//! - 主服务只监听 `127.0.0.1`（本机 App / 浏览器）。打开手机遥控时，另起一个监听
//!   `0.0.0.0:<遥控端口>` 的服务（默认主端口 + 1），关闭时优雅停止，**不需要重启**。
//!   The main service listens on `127.0.0.1` only (local app / browser). Enabling phone
//!   remote starts a second listener on `0.0.0.0:<remote port>` (main port + 1 by
//!   default); disabling stops it gracefully — **no restart needed**.
//! - 局域网服务要求访问码：网址里的 `?t=<访问码>`（二维码里就是这个网址），第一次访问后
//!   写入 cookie，之后直接打开即可。没有访问码的请求返回 401。访问码保存在设置里，
//!   二维码重启后仍有效；可在设置里重新生成。
//!   The LAN service requires an access code: `?t=<code>` in the URL (which is what the
//!   QR code encodes); after the first visit it's stored in a cookie. Requests without
//!   it get 401. The code is saved in settings so the QR code survives restarts; it can
//!   be regenerated.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Router;
use serde::Serialize;
use tokio::sync::oneshot;

/// cookie 名。Cookie name.
const COOKIE: &str = "sb_token";

/// 只允许在电脑本机调用的接口（手机遥控不能改电脑的设置、开关遥控、操作文件管理器、关闭窗口）。
/// Endpoints allowed only on the PC itself (the phone can't change PC settings, toggle
/// the remote, drive the file manager or close the window).
const DESKTOP_ONLY: &[&str] = &[
    "/api/remote",
    "/api/settings",
    "/api/pick-folder",
    "/api/library/location",
    "/api/library/open",
    "/api/output",
    "/api/open-url",
    "/api/window",
];
// 注意：`/api/monitor` 不在这里——手机可以开关试听、调试听音量（只影响电脑主人自己耳机里的声音），
// 但不能换试听设备，由处理函数根据 [`FromPhone`] 拒绝。
// Note: `/api/monitor` isn't listed — phones may toggle the monitor and set its volume
// (only what the computer's owner hears in their headphones), but not change the
// monitor device; the handler refuses that based on [`FromPhone`].

/// 请求来自手机遥控（由 `require_token` 放进请求扩展，处理函数可据此区分）。
/// The request came through the phone remote (put into the request extensions by
/// `require_token`, so handlers can tell).
#[derive(Debug, Clone, Copy)]
pub struct FromPhone;

/// 遥控状态（供界面显示）。Remote status (for the UI).
#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    pub enabled: bool,
    /// 手机扫码 / 输入的完整网址（含访问码）。Full URL for the phone (with the code).
    pub url: Option<String>,
    pub port: Option<u16>,
    /// 未能开启的原因。Why it couldn't start.
    pub error: Option<String>,
}

/// 局域网服务管理器。LAN listener manager.
pub struct RemoteServer {
    token: Mutex<String>,
    preferred_port: u16,
    router: Mutex<Option<Router>>,
    running: Mutex<Option<(u16, oneshot::Sender<()>)>>,
    error: Mutex<Option<String>>,
}

impl RemoteServer {
    pub fn new(token: String, preferred_port: u16) -> Self {
        Self {
            token: Mutex::new(token),
            preferred_port,
            router: Mutex::new(None),
            running: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    /// 设置要对外提供的路由（与本机相同的界面与 API）。Set the router to serve.
    pub fn set_router(&self, router: Router) {
        *self.router.lock().unwrap() = Some(router);
    }

    pub fn token(&self) -> String {
        self.token.lock().unwrap().clone()
    }

    pub fn set_token(&self, token: String) {
        *self.token.lock().unwrap() = token;
    }

    pub fn status(&self) -> RemoteStatus {
        let running = self.running.lock().unwrap().as_ref().map(|(p, _)| *p);
        RemoteStatus {
            enabled: running.is_some(),
            url: running.and_then(|port| {
                crate::lan_ip().map(|ip| format!("http://{ip}:{port}/?t={}", self.token()))
            }),
            port: running,
            error: self.error.lock().unwrap().clone(),
        }
    }

    /// 开启（已开启则不变）。必须在 tokio 运行时内调用。Start (no-op if running).
    /// Must be called inside a tokio runtime.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.running.lock().unwrap().is_some() {
            return Ok(());
        }
        let router = self
            .router
            .lock()
            .unwrap()
            .clone()
            .ok_or(tr!("服务尚未就绪", "The service isn't ready yet"))?;
        let any: IpAddr = [0, 0, 0, 0].into();
        let listener =
            match tokio::net::TcpListener::bind(SocketAddr::new(any, self.preferred_port)).await {
                Ok(l) => l,
                Err(_) => tokio::net::TcpListener::bind(SocketAddr::new(any, 0))
                    .await
                    .map_err(|e| {
                        trf!(
                            "无法开启局域网监听：{e}",
                            "Can't listen on the local network: {e}"
                        )
                    })?,
            };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let app = router.layer(axum::middleware::from_fn_with_state(
            self.clone(),
            require_token,
        ));
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                })
                .await;
        });
        *self.running.lock().unwrap() = Some((port, stop_tx));
        *self.error.lock().unwrap() = None;
        log::info!("手机遥控已开启 phone remote on port {port}");
        Ok(())
    }

    /// 关闭（未开启则不变）。Stop (no-op if not running).
    pub fn stop(&self) {
        if let Some((_, stop)) = self.running.lock().unwrap().take() {
            let _ = stop.send(());
            log::info!("手机遥控已关闭 phone remote off");
        }
    }

    pub fn set_error(&self, e: Option<String>) {
        *self.error.lock().unwrap() = e;
    }
}

/// 局域网请求必须带访问码（网址参数或 cookie）。LAN requests need the code (URL or cookie).
async fn require_token(
    State(remote): State<Arc<RemoteServer>>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if req.method() != axum::http::Method::GET && DESKTOP_ONLY.iter().any(|p| path.starts_with(p)) {
        return (
            StatusCode::FORBIDDEN,
            tr!(
                "请在电脑上操作此项设置",
                "Change this setting on the computer"
            ),
        )
            .into_response();
    }
    let token = remote.token();
    let from_query = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("t=")))
        .map(|t| t == token)
        .unwrap_or(false);
    let from_cookie = req
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .any(|c| c.trim() == format!("{COOKIE}={token}"));

    if !from_query && !from_cookie {
        return (
            StatusCode::UNAUTHORIZED,
            Html(tr!(
                "<meta name=viewport content='width=device-width'>\
                 <body style='font-family:system-ui;padding:24px;background:#0d0e12;color:#eceef3'>\
                 <h3>需要访问码</h3><p>请在电脑上的 FerryMic 里打开「手机遥控」，用手机扫描二维码。</p>",
                "<meta name=viewport content='width=device-width'>\
                 <body style='font-family:system-ui;padding:24px;background:#0d0e12;color:#eceef3'>\
                 <h3>Access code required</h3><p>Turn on \"Phone remote\" in FerryMic on the computer \
                 and scan the QR code with your phone.</p>"
            )),
        )
            .into_response();
    }
    req.extensions_mut().insert(FromPhone);
    let mut res = next.run(req).await;
    if from_query && !from_cookie {
        // 记住访问码：之后不带参数也能打开（一年）。Remember the code for a year.
        if let Ok(v) = HeaderValue::from_str(&format!(
            "{COOKIE}={token}; Path=/; Max-Age=31536000; SameSite=Lax"
        )) {
            res.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    res
}

/// 生成随机访问码（10 位字母数字，来自系统随机种子的 SipHash）。
/// Generate a random access code (10 alphanumerics, SipHash keyed from OS randomness).
pub fn new_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut out = String::new();
    for i in 0..10u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        out.push(ALPHABET[(h.finish() % ALPHABET.len() as u64) as usize] as char);
    }
    out
}

/// 把网址画成二维码 SVG。Render a URL as a QR code SVG.
pub fn qr_svg(text: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("#111111"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手机请求：带访问码可以调试听并被标记为「来自手机」；电脑端专属接口仍被拒；没有访问码 401。
    /// Phone requests: with the code, the monitor endpoint is allowed and marked as
    /// "from phone"; desktop-only endpoints stay blocked; no code means 401.
    #[tokio::test]
    async fn phone_requests_are_marked_and_desktop_only_is_blocked() {
        use axum::body::Body;
        use axum::routing::post;
        use tower::ServiceExt;

        let remote = Arc::new(RemoteServer::new("abc123".into(), 0));
        let app = Router::new()
            .route(
                "/api/monitor",
                post(|phone: Option<axum::Extension<FromPhone>>| async move {
                    if phone.is_some() {
                        "phone"
                    } else {
                        "desktop"
                    }
                }),
            )
            .route("/api/output", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                remote.clone(),
                require_token,
            ));
        let req = |path: &str| axum::http::Request::post(path).body(Body::empty()).unwrap();

        let res = app
            .clone()
            .oneshot(req("/api/monitor?t=abc123"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"phone");

        let res = app
            .clone()
            .oneshot(req("/api/output?t=abc123"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res = app.oneshot(req("/api/monitor")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn tokens_are_random_and_well_formed() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 10);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }

    #[test]
    fn renders_qr_svg() {
        let svg = qr_svg("http://192.168.1.5:8322/?t=abc").unwrap();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"));
    }
}
