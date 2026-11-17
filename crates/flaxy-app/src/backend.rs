//! Ponte entre a interface e o núcleo: runtime Tokio, fila, banco e tarefas
//! assíncronas cujo resultado a UI lê no próximo quadro.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use eframe::egui;
use flaxy_core::clips::Clips;
use flaxy_core::jobs::Jobs;
use flaxy_core::transcripts::Transcripts;
use flaxy_core::{db, Config};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::runtime::Handle;

/// Resultado de uma tarefa assíncrona. A UI chama `take` a cada quadro.
pub struct Task<T>(Arc<Mutex<Option<T>>>);

impl<T> Task<T> {
    pub fn take(&self) -> Option<T> {
        self.0.lock().ok()?.take()
    }
}

pub type Res<T> = Result<T, String>;

/// Preferências do app gravadas no banco (tabela `settings`, chave `app`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub output_dir: String,
    pub max_concurrent: usize,
    /// Navegador de onde o yt-dlp lê os cookies do YouTube (vazio = nenhum).
    pub yt_cookies_browser: String,
    /// Arquivo cookies.txt exportado (vale mais que o navegador).
    pub yt_cookies_file: String,
}

impl AppSettings {
    /// Passa os cookies do YouTube para o yt-dlp (vale já no próximo download).
    pub fn apply_yt_cookies(&self) {
        flaxy_core::clips::set_yt_cookies(flaxy_core::clips::YtCookies { browser: self.yt_cookies_browser.clone(), file: self.yt_cookies_file.clone() });
    }
}

#[derive(Default)]
pub struct Live {
    /// item_id -> (status, progresso) vindo da fila em tempo real.
    pub items: HashMap<i64, (String, i64)>,
}

pub struct Backend {
    pub cfg: Config,
    pub db: SqlitePool,
    pub jobs: Jobs,
    pub transcripts: Transcripts,
    pub clips: Clips,
    pub rt: Handle,
    pub live: Mutex<Live>,
    /// Muda quando um item termina, falha ou é cancelado: telas recarregam.
    pub jobs_rev: AtomicU64,
    pub encoder: Mutex<Option<String>>,
    /// Downloads de modelo do Whisper: nome -> progresso ou erro.
    pub downloads: Mutex<HashMap<String, Result<u8, String>>>,
    /// Limita quantos FFmpeg de miniatura rodam ao mesmo tempo.
    pub thumb_slots: Arc<tokio::sync::Semaphore>,
    pub settings: AppSettings,
}

pub async fn load_settings(db: &SqlitePool) -> AppSettings {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'app'")
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    row.and_then(|r| serde_json::from_str(&r.0).ok()).unwrap_or_default()
}

pub async fn save_settings(db: &SqlitePool, s: &AppSettings) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO settings (key, value) VALUES ('app', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(serde_json::to_string(s)?)
        .execute(db)
        .await?;
    Ok(())
}

impl Backend {
    pub async fn start(mut cfg: Config, rt: Handle) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.storage_dir).context("criando pasta de dados")?;
        let pool = db::connect(&cfg.db_path()).await.context("abrindo banco")?;
        let settings = load_settings(&pool).await;
        settings.apply_yt_cookies();
        if !settings.output_dir.trim().is_empty() {
            cfg.output_override = Some(PathBuf::from(settings.output_dir.trim()));
        }
        if settings.max_concurrent > 0 {
            cfg.max_concurrent = settings.max_concurrent.clamp(1, 8);
        }
        cfg.ensure_dirs().context("criando pastas de trabalho")?;
        cfg.storage_dir = flaxy_core::config::canonical(&cfg.storage_dir)?;
        cfg.work_dir = flaxy_core::config::canonical(&cfg.work_dir)?;
        if let Some(o) = &cfg.output_override {
            cfg.output_override = Some(flaxy_core::config::canonical(o)?);
        }

        let jobs = Jobs::start(pool.clone(), cfg.clone()).await?;
        flaxy_core::captions::seed(&pool).await?;
        let transcripts = Transcripts::start(pool.clone(), cfg.clone()).await?;
        let clips = Clips::start(pool.clone(), cfg.clone()).await?;

        let bk = Arc::new(Self {
            cfg,
            db: pool,
            jobs,
            transcripts,
            clips,
            rt,
            live: Mutex::new(Live::default()),
            jobs_rev: AtomicU64::new(1),
            encoder: Mutex::new(None),
            downloads: Mutex::new(HashMap::new()),
            thumb_slots: Arc::new(tokio::sync::Semaphore::new(3)),
            settings,
        });
        let b2 = bk.clone();
        tokio::spawn(async move {
            let e = flaxy_core::media::encoder().await;
            *b2.encoder.lock().unwrap() = Some(e.name.to_string());
        });
        Ok(bk)
    }

    /// Repassa os eventos da fila para a UI e pede um novo quadro.
    pub fn attach(self: &Arc<Self>, ctx: &egui::Context) {
        let mut rx = self.jobs.subscribe();
        let (bk, ctx) = (self.clone(), ctx.clone());
        self.rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        bk.live.lock().unwrap().items.insert(ev.item_id, (ev.status.clone(), ev.progress));
                        if ev.status != "processing" {
                            bk.jobs_rev.fetch_add(1, Ordering::Relaxed);
                        }
                        ctx.request_repaint();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    pub fn rev(&self) -> u64 {
        self.jobs_rev.load(Ordering::Relaxed)
    }

    pub fn bump(&self) {
        self.jobs_rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Roda `fut` no runtime e devolve o resultado na UI.
    pub fn spawn<T, F>(&self, ctx: &egui::Context, fut: F) -> Task<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let slot = Arc::new(Mutex::new(None));
        let (s2, ctx) = (slot.clone(), ctx.clone());
        self.rt.spawn(async move {
            let v = fut.await;
            if let Ok(mut g) = s2.lock() {
                *g = Some(v);
            }
            ctx.request_repaint();
        });
        Task(slot)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.transcripts.models_dir()
    }
}

/// Converte erro do núcleo em texto para o aviso na tela.
pub fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

// ------------------------------------------------------------ consultas

#[derive(Default, Clone)]
pub struct Stats {
    pub by_status: HashMap<String, i64>,
    pub daily: Vec<(String, i64)>,
    pub templates: i64,
    pub disk_output: u64,
    pub disk_uploads: u64,
    pub disk_templates: u64,
}

pub async fn stats(bk: Arc<Backend>) -> Res<Stats> {
    let by_status: Vec<(String, i64)> = sqlx::query_as("SELECT status, COUNT(*) FROM job_items GROUP BY status")
        .fetch_all(&bk.db)
        .await
        .map_err(e)?;
    let daily: Vec<(String, i64)> = sqlx::query_as(
        "SELECT substr(j.created_at, 1, 10) d, COUNT(*) FROM job_items i \
         JOIN jobs j ON j.id = i.job_id \
         WHERE i.status = 'done' AND j.created_at >= strftime('%Y-%m-%d', 'now', '-6 days') \
         GROUP BY d ORDER BY d",
    )
    .fetch_all(&bk.db)
    .await
    .map_err(e)?;
    let (templates,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM templates").fetch_one(&bk.db).await.map_err(e)?;
    Ok(Stats {
        by_status: by_status.into_iter().collect(),
        daily,
        templates,
        disk_output: dir_size(&bk.cfg.output_dir()).await,
        disk_uploads: dir_size(&bk.cfg.uploads_dir()).await,
        disk_templates: dir_size(&bk.cfg.templates_dir()).await,
    })
}

pub async fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(en)) = rd.next_entry().await {
            if let Ok(m) = en.metadata().await {
                if m.is_file() {
                    total += m.len();
                }
            }
        }
    }
    total
}

#[derive(Clone)]
pub struct OutputFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: String,
}

pub async fn list_output(bk: Arc<Backend>) -> Res<Vec<OutputFile>> {
    let mut items = Vec::new();
    let mut rd = tokio::fs::read_dir(bk.cfg.output_dir()).await.map_err(e)?;
    while let Some(en) = rd.next_entry().await.map_err(e)? {
        let Ok(meta) = en.metadata().await else { continue };
        let name = en.file_name().to_string_lossy().into_owned();
        let video = matches!(
            Path::new(&name).extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(),
            Some("mp4" | "mov" | "webm" | "mkv")
        );
        if !meta.is_file() || !video {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        items.push(OutputFile { path: en.path(), name, size: meta.len(), modified });
    }
    items.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(items)
}

/// ZIP sem compressão: vídeo já é comprimido, deflate só gastaria CPU.
pub async fn write_zip(files: Vec<PathBuf>, dest: PathBuf) -> Res<usize> {
    tokio::task::spawn_blocking(move || -> std::io::Result<usize> {
        let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(&dest)?));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        let mut n = 0;
        for f in files.iter().filter(|f| f.is_file()) {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            zw.start_file(name, opts).map_err(std::io::Error::other)?;
            std::io::copy(&mut std::fs::File::open(f)?, &mut zw)?;
            n += 1;
        }
        zw.finish().map_err(std::io::Error::other)?;
        Ok(n)
    })
    .await
    .map_err(e)?
    .map_err(e)
}

// --------------------------------------------------------- sistema (Windows)

/// Abre o arquivo no programa padrão do Windows.
pub fn open_path(p: &Path) {
    let _ = std::process::Command::new("explorer").arg(p).spawn();
}

/// Abre o Explorador com o arquivo selecionado.
pub fn reveal(p: &Path) {
    if p.is_dir() {
        open_path(p);
        return;
    }
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(p.as_os_str());
    let _ = std::process::Command::new("explorer").arg(arg).spawn();
}
