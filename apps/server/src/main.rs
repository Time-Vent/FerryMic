//! FerryMic 命令行入口：启动服务，用浏览器访问 `http://127.0.0.1:8321`。
//! FerryMic command-line entry: starts the service; open `http://127.0.0.1:8321`
//! in a browser.
//!
//! 桌面 App 版见 `apps/app`。配置项见 README 的环境变量说明。
//! The desktop app lives in `apps/app`. See the README for environment variables.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    soundbridge_server::run(soundbridge_server::ServerConfig::from_env(), |info| {
        log::info!("桌面访问 Desktop: {}", info.local_url);
    })
    .await
}
