//! Servidor HTTP do FLAXY como biblioteca.
//!
//! Usado pelo binário `flaxy-server`, que serve a API e o frontend no navegador.

mod ai_api;
mod api;
mod clips_api;
mod error;

use anyhow::Context;
use flaxy_core::{db, jobs::Jobs, Config};
use tokio::net::TcpListener;

/// Prepara pastas, banco e fila, e devolve o `Router` pronto para servir.
pub async fn build(mut cfg: Config) -> anyhow::Result<axum::Router> {
    cfg.ensure_dirs().context("criando pastas de trabalho")?;
    // Caminhos absolutos: a checagem de "dentro da pasta" compara prefixos.
    cfg.storage_dir = flaxy_core::config::canonical(&cfg.storage_dir)?;
    cfg.work_dir = flaxy_core::config::canonical(&cfg.work_dir)?;

    if flaxy_core::media::ffmpeg().is_none() {
        tracing::warn!("FFmpeg não encontrado no PATH: o processamento vai falhar");
    }
    let pool = db::connect(&cfg.db_path()).await.context("abrindo banco")?;
    let jobs = Jobs::start(pool.clone(), cfg.clone()).await?;
    flaxy_core::captions::seed(&pool).await?;
    let transcripts = flaxy_core::transcripts::Transcripts::start(pool.clone(), cfg.clone()).await?;
    let clips = flaxy_core::clips::Clips::start(pool.clone(), cfg.clone()).await?;
    // Detecta o encoder já na partida, não no primeiro vídeo.
    tokio::spawn(flaxy_core::media::encoder());
    Ok(api::router(api::AppState::new(cfg, pool, jobs, transcripts, clips)))
}

/// Serve até `shutdown` completar.
pub async fn serve(
    listener: TcpListener,
    app: axum::Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, app).with_graceful_shutdown(shutdown).await?;
    Ok(())
}
