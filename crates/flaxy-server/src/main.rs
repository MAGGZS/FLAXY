//! Binário do servidor (nuvem / rede local). O app desktop usa a mesma biblioteca.

use std::net::SocketAddr;

use flaxy_core::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = Config::from_env();
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let app = flaxy_server::build(cfg).await?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("FLAXY {} em http://{addr}", env!("CARGO_PKG_VERSION"));
    flaxy_server::serve(listener, app, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}
