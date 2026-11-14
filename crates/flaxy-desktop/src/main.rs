//! FLAXY desktop: janela nativa (Tauri) com o servidor embutido.
//!
//! Na partida o app sobe o mesmo servidor do modo nuvem em 127.0.0.1, numa
//! porta livre, e injeta o endereço na janela. A interface fala HTTP com ele
//! exatamente como faria com um servidor remoto.

// Sem janela de terminal no build de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use flaxy_core::Config;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

struct Shutdown(std::sync::Mutex<Option<oneshot::Sender<()>>>);

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            // FFmpeg empacotado no instalador tem prioridade sobre o do PATH.
            if std::env::var("FLAXY_FFMPEG_DIR").is_err() {
                if let Ok(res) = app.path().resource_dir() {
                    let dir = res.join("ffmpeg");
                    if dir.join("ffmpeg.exe").is_file() {
                        std::env::set_var("FLAXY_FFMPEG_DIR", &dir);
                    }
                }
            }

            if std::env::var("FLAXY_WHISPER_DIR").is_err() {
                if let Ok(res) = app.path().resource_dir() {
                    let dir = res.join("whisper");
                    if dir.join("whisper-cli.exe").is_file() {
                        std::env::set_var("FLAXY_WHISPER_DIR", &dir);
                    }
                }
            }

            if std::env::var("FLAXY_YTDLP_DIR").is_err() {
                if let Ok(res) = app.path().resource_dir() {
                    let dir = res.join("yt-dlp");
                    if dir.join("yt-dlp.exe").is_file() {
                        std::env::set_var("FLAXY_YTDLP_DIR", &dir);
                    }
                }
            }

            // Dados persistentes em %APPDATA%\app.flaxy.desktop
            let data = app.path().app_data_dir()?;
            let mut cfg = Config::from_env();
            if std::env::var("FLAXY_STORAGE_DIR").is_err() {
                cfg.storage_dir = data.join("storage");
            }
            if std::env::var("FLAXY_WORK_DIR").is_err() {
                // No desktop os vídeos prontos ficam: nada de pasta temporária.
                cfg.work_dir = data.join("work");
            }
            cfg.host = "127.0.0.1".into();
            cfg.static_dir = None;

            let (tx, rx) = oneshot::channel::<()>();
            let port = tauri::async_runtime::block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                let router = flaxy_server::build(cfg).await?;
                tauri::async_runtime::spawn(async move {
                    let stop = async {
                        let _ = rx.await;
                    };
                    if let Err(e) = flaxy_server::serve(listener, router, stop).await {
                        tracing::error!("servidor embutido parou: {e}");
                    }
                });
                anyhow::Ok(port)
            })?;
            app.manage(Shutdown(std::sync::Mutex::new(Some(tx))));
            tracing::info!("servidor embutido em 127.0.0.1:{port}");

            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("FLAXY")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1080.0, 680.0)
                .center()
                .background_color(tauri::webview::Color(19, 20, 22, 255))
                .initialization_script(&format!(
                    "window.__FLAXY_API__ = 'http://127.0.0.1:{port}';"
                ))
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("falha ao iniciar o FLAXY")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(tx) = app.state::<Shutdown>().0.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
}
