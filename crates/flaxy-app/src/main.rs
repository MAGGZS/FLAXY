//! FLAXY: app nativo (egui + GPU). Sem navegador nem WebView: a interface é
//! desenhada pela própria aplicação e fala com o núcleo Rust em processo.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod editors;
mod format;
mod icons;
mod media;
mod pages;
mod prospect;
mod theme;
mod widgets;
mod youtube;

use std::path::{Path, PathBuf};

use eframe::egui;

const APP_TITLE: &str = "FLAXY";

/// Programas externos (ffmpeg, ffprobe, yt-dlp, whisper-cli) ficam em `bin\`
/// ao lado do executável. Em desenvolvimento, procura `bin\` na raiz do repositório.
fn locate_tools() {
    let exe = std::env::current_exe().ok();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = exe.as_deref().and_then(Path::parent) {
        candidates.push(dir.join("bin"));
        for up in dir.ancestors().skip(1).take(4) {
            candidates.push(up.join("bin"));
        }
    }
    let Some(bin) = candidates.into_iter().find(|d| d.join("ffmpeg.exe").is_file() || d.join("yt-dlp.exe").is_file()) else {
        return;
    };
    for (var, dir, file) in [
        ("FLAXY_FFMPEG_DIR", bin.clone(), "ffmpeg.exe"),
        ("FLAXY_YTDLP_DIR", bin.clone(), "yt-dlp.exe"),
        ("FLAXY_WHISPER_DIR", bin.join("whisper"), "whisper-cli.exe"),
        ("FLAXY_WHISPER_DIR", bin.clone(), "whisper-cli.exe"),
    ] {
        if std::env::var_os(var).is_none() && dir.join(file).is_file() {
            // Ainda não há outras threads: seguro alterar o ambiente aqui.
            std::env::set_var(var, &dir);
        }
    }
}

/// Uma instância por vez: duas filas no mesmo banco renderizariam em dobro.
/// Se já existe, traz a janela aberta para frente e sai.
#[cfg(windows)]
fn single_instance() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let name = wide("Local\\FLAXY.single-instance");
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if !h.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            let title = wide(APP_TITLE);
            let w = FindWindowW(std::ptr::null(), title.as_ptr());
            if !w.is_null() {
                ShowWindow(w, SW_RESTORE);
                SetForegroundWindow(w);
            }
            return false;
        }
        // O handle nunca é fechado: o mutex vive até o processo terminar.
    }
    true
}

#[cfg(not(windows))]
fn single_instance() -> bool {
    true
}

fn init_logs(cfg: &flaxy_core::Config) {
    let dir = cfg.work_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("app.log"));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into());
    match file {
        Ok(f) => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(f)).try_init();
        }
        Err(_) => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        }
    }
}

fn fatal(msg: &str) {
    rfd::MessageDialog::new()
        .set_title(APP_TITLE)
        .set_description(msg)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

fn main() {
    // FLAXY_MULTI=1 permite uma segunda janela (teste com outra pasta de dados).
    if std::env::var_os("FLAXY_MULTI").is_none() && !single_instance() {
        return;
    }
    locate_tools();
    let cfg = flaxy_core::Config::desktop();
    init_logs(&cfg);

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("flaxy").build() {
        Ok(rt) => rt,
        Err(e) => return fatal(&format!("Não foi possível iniciar: {e}")),
    };
    let bk = match rt.block_on(backend::Backend::start(cfg, rt.handle().clone())) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("falha ao iniciar: {e:#}");
            return fatal(&format!("Não foi possível abrir os dados do FLAXY.\n\n{e:#}"));
        }
    };
    tracing::info!("FLAXY {} iniciado", env!("CARGO_PKG_VERSION"));

    let icon = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|i| {
        let i = i.to_rgba8();
        egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_TITLE)
        .with_inner_size([1360.0, 820.0])
        .with_min_inner_size([1100.0, 700.0])
        .with_app_id("app.flaxy");
    if let Some(i) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(i));
    }
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };

    let bk2 = bk.clone();
    let res = eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            let fonts = theme::install(&cc.egui_ctx);
            bk2.attach(&cc.egui_ctx);
            Ok(Box::new(app::App::new(bk2, fonts)))
        }),
    );
    if let Err(e) = res {
        fatal(&format!("Falha na janela: {e}"));
    }
    // Encerra a fila: FFmpeg em andamento é finalizado (kill_on_drop) e o item
    // volta para a fila na próxima abertura.
    drop(bk);
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
}
