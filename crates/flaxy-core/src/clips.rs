//! Clipes de retenção: link -> yt-dlp -> transcrição -> LLM sugere trechos ->
//! usuário aprova e marca câmera/tela -> render por layout.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Semaphore;

use crate::ai::{self, TranscribeCtx};
use crate::captions::{self, Word};
use crate::compose::run_ffmpeg;
use crate::{llm, media, Config, Error, Result};

pub const LAYOUTS: &[&str] = &["vertical_camera", "vertical_split", "horizontal"];
const MAX_SUGGESTIONS: usize = 10;

/// Retângulo em fração 0..1 do quadro da fonte.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const FULL: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    fn clamped(self) -> Rect {
        let x = self.x.clamp(0.0, 0.98);
        let y = self.y.clamp(0.0, 0.98);
        Rect { x, y, w: self.w.clamp(0.02, 1.0 - x), h: self.h.clamp(0.02, 1.0 - y) }
    }

    fn crop(self) -> String {
        let r = self.clamped();
        format!("crop=iw*{:.5}:ih*{:.5}:iw*{:.5}:ih*{:.5}", r.w, r.h, r.x, r.y)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Project {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub source_path: Option<String>,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub status: String,
    pub progress: i64,
    pub error_msg: Option<String>,
    pub transcript_id: Option<i64>,
    pub min_len: f64,
    pub max_len: f64,
    pub layout: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ClipRow {
    pub id: i64,
    pub project_id: i64,
    pub start: f64,
    pub end: f64,
    pub score: i64,
    pub title: String,
    pub reason: String,
    pub approved: bool,
    pub layout: String,
    #[serde(skip)]
    pub camera_rect: Option<String>,
    #[serde(skip)]
    pub screen_rect: Option<String>,
    pub caption_style_id: Option<i64>,
    pub status: String,
    pub progress: i64,
    pub output_path: Option<String>,
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    #[serde(flatten)]
    pub row: ClipRow,
    pub camera: Rect,
    pub screen: Rect,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProject {
    pub url: String,
    pub min_len: f64,
    pub max_len: f64,
    #[serde(default)]
    pub layout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipEdit {
    pub id: i64,
    pub approved: bool,
    pub layout: String,
    pub camera: Rect,
    pub screen: Rect,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderRequest {
    pub clips: Vec<ClipEdit>,
    #[serde(default)]
    pub caption_style_id: Option<i64>,
}

/// Sugestão inicial de câmera e tela (o usuário ajusta na tela de aprovação).
pub fn suggest_rects(layout: &str, w: i64, h: i64) -> (Rect, Rect) {
    let ar = if w > 0 && h > 0 { w as f64 / h as f64 } else { 16.0 / 9.0 };
    // Faixa 9:16 central: tamanho certo para "só câmera".
    let vw = ((9.0 / 16.0) / ar).min(1.0);
    match layout {
        // Webcam costuma ficar num canto inferior; tela é o quadro inteiro.
        "vertical_split" => (Rect { x: 0.68, y: 0.62, w: 0.3, h: 0.36 }, Rect::FULL),
        "horizontal" => (Rect::FULL, Rect::FULL),
        _ => (Rect { x: (1.0 - vw) / 2.0, y: 0.0, w: vw, h: 1.0 }, Rect::FULL),
    }
}

fn parse_rect(s: &Option<String>, fallback: Rect) -> Rect {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or(fallback)
}

static YTDLP: OnceLock<Option<PathBuf>> = OnceLock::new();

/// yt-dlp em FLAXY_YTDLP_DIR (empacotado) ou no PATH.
pub fn ytdlp() -> Option<&'static Path> {
    YTDLP
        .get_or_init(|| {
            let exe = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
            if let Ok(dir) = std::env::var("FLAXY_YTDLP_DIR") {
                let p = Path::new(&dir).join(exe);
                if p.is_file() {
                    return Some(p);
                }
            }
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file())
        })
        .as_deref()
}

#[derive(Clone)]
pub struct Clips {
    db: SqlitePool,
    cfg: Config,
    render_slot: Arc<Semaphore>,
}

impl Clips {
    pub async fn start(db: SqlitePool, cfg: Config) -> Result<Self> {
        let c = Self { db, cfg, render_slot: Arc::new(Semaphore::new(1)) };
        // Pipeline interrompido não tem como retomar no meio: marca erro.
        sqlx::query(
            "UPDATE clip_projects SET status='error', error_msg='Interrompido ao fechar o app. Tente de novo.' \
             WHERE status IN ('downloading','transcribing','analyzing')",
        )
        .execute(&c.db)
        .await?;
        sqlx::query("UPDATE clip_items SET status='queued', progress=0 WHERE status='processing'")
            .execute(&c.db)
            .await?;
        let pending: Vec<(i64,)> = sqlx::query_as("SELECT id FROM clip_items WHERE status='queued' ORDER BY id")
            .fetch_all(&c.db)
            .await?;
        for (id,) in pending {
            c.spawn_render(id);
        }
        Ok(c)
    }

    pub async fn create(&self, p: NewProject) -> Result<ProjectView> {
        let url = p.url.trim().trim_matches('"').to_string();
        let local = Path::new(&url).is_file();
        if !local && (!(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048) {
            return Err(Error::Invalid("Cole um link http(s) válido ou escolha um arquivo".into()));
        }
        if !local && ytdlp().is_none() {
            return Err(Error::Invalid("yt-dlp não encontrado. Reinstale o FLAXY".into()));
        }
        let min = p.min_len.clamp(5.0, 600.0);
        let max = p.max_len.clamp(min + 5.0, 900.0);
        let layout = p.layout.filter(|l| LAYOUTS.contains(&l.as_str())).unwrap_or_else(|| "vertical_camera".into());
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO clip_projects (url, min_len, max_len, layout) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(&url)
        .bind(min)
        .bind(max)
        .bind(&layout)
        .fetch_one(&self.db)
        .await?;
        self.run(id);
        self.get(id).await
    }

    /// Roda de novo um projeto que falhou. Reaproveita o vídeo baixado e a
    /// transcrição quando já existem.
    pub async fn retry(&self, id: i64) -> Result<ProjectView> {
        let p = self.get(id).await?.project;
        if p.status != "error" {
            return Err(Error::Invalid("Esse vídeo não falhou".into()));
        }
        sqlx::query("DELETE FROM clip_items WHERE project_id=?").bind(id).execute(&self.db).await?;
        sqlx::query("UPDATE clip_projects SET status='downloading', progress=0, error_msg=NULL WHERE id=?")
            .bind(id)
            .execute(&self.db)
            .await?;
        self.run(id);
        self.get(id).await
    }

    fn run(&self, id: i64) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.pipeline(id).await {
                tracing::warn!("projeto de clipes {id} falhou: {e}");
                let _ = sqlx::query("UPDATE clip_projects SET status='error', error_msg=? WHERE id=?")
                    .bind(e.to_string())
                    .bind(id)
                    .execute(&this.db)
                    .await;
            }
        });
    }

    pub async fn list(&self) -> Result<Vec<Project>> {
        Ok(sqlx::query_as("SELECT * FROM clip_projects ORDER BY id DESC LIMIT 100").fetch_all(&self.db).await?)
    }

    pub async fn get(&self, id: i64) -> Result<ProjectView> {
        let project: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?")
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound("Projeto de clipes"))?;
        let rows: Vec<ClipRow> = sqlx::query_as("SELECT * FROM clip_items WHERE project_id=? ORDER BY score DESC, id")
            .bind(id)
            .fetch_all(&self.db)
            .await?;
        let clips = rows
            .into_iter()
            .map(|row| {
                let (cam, scr) = suggest_rects(&row.layout, project.width, project.height);
                Clip { camera: parse_rect(&row.camera_rect, cam), screen: parse_rect(&row.screen_rect, scr), row }
            })
            .collect();
        Ok(ProjectView { project, clips })
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let p = self.get(id).await?;
        sqlx::query("DELETE FROM clip_projects WHERE id=?").bind(id).execute(&self.db).await?;
        // Só apaga o que o FLAXY baixou; arquivo local do usuário fica.
        if let Some(src) = p.project.source_path {
            if Path::new(&src).starts_with(self.cfg.uploads_dir()) {
                let _ = tokio::fs::remove_file(src).await;
            }
        }
        Ok(())
    }

    async fn set_status(&self, id: i64, status: &str, progress: i64) {
        let _ = sqlx::query("UPDATE clip_projects SET status=?, progress=? WHERE id=?")
            .bind(status)
            .bind(progress)
            .bind(id)
            .execute(&self.db)
            .await;
    }

    async fn pipeline(&self, id: i64) -> Result<()> {
        let p: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?").bind(id).fetch_one(&self.db).await?;

        // 1. Download (ou arquivo local escolhido pelo usuário)
        let kept = p.source_path.as_deref().map(PathBuf::from).filter(|s| s.is_file());
        let src = if let Some(s) = kept {
            s
        } else if Path::new(&p.url).is_file() {
            let title = Path::new(&p.url).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            sqlx::query("UPDATE clip_projects SET title=? WHERE id=?").bind(title).bind(id).execute(&self.db).await?;
            PathBuf::from(&p.url)
        } else {
            self.download(id, &p.url).await?
        };
        let probe = media::probe(&src).await;
        let (w, h) = media::dimensions(&src).await;
        sqlx::query("UPDATE clip_projects SET source_path=?, duration=?, width=?, height=? WHERE id=?")
            .bind(src.to_string_lossy().into_owned())
            .bind(probe.duration)
            .bind(w)
            .bind(h)
            .bind(id)
            .execute(&self.db)
            .await?;

        // 2. Transcrição (gravada em `transcripts` para servir de legenda depois)
        self.set_status(id, "transcribing", 0).await;
        let settings = ai::load(&self.db).await?;
        let models = self.cfg.storage_dir.join("models");
        let ctx = TranscribeCtx { settings: &settings, models_dir: &models, work_dir: &self.cfg.work_dir };
        let kept: Option<Vec<Word>> = match p.transcript_id {
            Some(tid) => sqlx::query_as::<_, (String,)>("SELECT words FROM transcripts WHERE id=?")
                .bind(tid)
                .fetch_optional(&self.db)
                .await?
                .and_then(|(w,)| serde_json::from_str(&w).ok())
                .filter(|w: &Vec<Word>| !w.is_empty()),
            None => None,
        };
        let words = if let Some(w) = kept {
            w
        } else {
            self.transcribe(id, &ctx, &src, &settings).await?
        };

        // 3. Análise de retenção
        self.set_status(id, "analyzing", 0).await;
        let suggestions = analyze(&settings, &words, p.min_len, p.max_len, probe.duration).await?;
        if suggestions.is_empty() {
            return Err(Error::Invalid("A IA não encontrou trechos dentro da duração escolhida".into()));
        }
        let (cam, scr) = suggest_rects(&p.layout, w, h);
        for s in suggestions {
            sqlx::query(
                "INSERT INTO clip_items (project_id, start, end, score, title, reason, layout, camera_rect, screen_rect)                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(s.start)
            .bind(s.end)
            .bind(s.score)
            .bind(&s.title)
            .bind(&s.reason)
            .bind(&p.layout)
            .bind(serde_json::to_string(&cam)?)
            .bind(serde_json::to_string(&scr)?)
            .execute(&self.db)
            .await?;
        }
        self.set_status(id, "ready", 100).await;
        Ok(())
    }

    async fn transcribe(&self, id: i64, ctx: &TranscribeCtx<'_>, src: &Path, settings: &ai::AiSettings) -> Result<Vec<Word>> {
        let words = ai::transcribe(ctx, src).await?;
        if words.is_empty() {
            return Err(Error::Invalid("Nenhuma fala encontrada no vídeo".into()));
        }
        let (tid,): (i64,) = sqlx::query_as(
            "INSERT INTO transcripts (source_path, provider, language, status, words) VALUES (?, ?, ?, 'done', ?) RETURNING id",
        )
        .bind(src.to_string_lossy().into_owned())
        .bind(&settings.transcriber)
        .bind(&settings.language)
        .bind(serde_json::to_string(&words)?)
        .fetch_one(&self.db)
        .await?;
        sqlx::query("UPDATE clip_projects SET transcript_id=? WHERE id=?").bind(tid).bind(id).execute(&self.db).await?;
        Ok(words)
    }

    async fn download(&self, id: i64, url: &str) -> Result<PathBuf> {
        let bin = ytdlp().ok_or_else(|| Error::Invalid("yt-dlp não encontrado".into()))?;
        let dir = self.cfg.uploads_dir();
        let template = dir.join(format!("clip_src_{id}.%(ext)s"));
        let mut cmd = crate::media::command(bin);
        cmd.args([
            "--no-playlist", "--newline", "--encoding", "utf-8", "--no-part", "--no-mtime",
            "-f", "bv*[height<=1080][ext=mp4]+ba[ext=m4a]/bv*[height<=1080]+ba/b[height<=1080]/b",
            "--merge-output-format", "mp4",
            "--progress-template", "download:flaxy %(progress._percent_str)s",
            "--print", "after_move:filepath",
            "--print", "before_dl:title:%(title)s",
            "-o",
        ])
        .arg(&template);
        if let Some(ff) = media::ffmpeg().and_then(|p| p.parent()) {
            cmd.arg("--ffmpeg-location").arg(ff);
        }
        // "--" impede que o link seja lido como opção.
        let mut child = cmd
            .arg("--")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stderr = child.stderr.take().expect("stderr");
        let err_task = tokio::spawn(async move {
            let mut lines = LossyLines::new(stderr);
            let mut last = Vec::new();
            while let Some(l) = lines.next().await {
                if l.contains("ERROR") || last.len() < 3 {
                    last.push(l);
                }
                if last.len() > 6 {
                    last.remove(0);
                }
            }
            last.join(" | ")
        });
        let mut out = LossyLines::new(child.stdout.take().expect("stdout"));
        let mut file: Option<String> = None;
        let mut last_pct = -1;
        while let Some(line) = out.next().await {
            if let Some(p) = line.strip_prefix("flaxy ") {
                if let Ok(v) = p.trim().trim_end_matches('%').trim().parse::<f64>() {
                    let pct = v as i64;
                    if pct != last_pct {
                        last_pct = pct;
                        self.set_status(id, "downloading", pct).await;
                    }
                }
            } else if let Some(t) = line.strip_prefix("title:") {
                let _ = sqlx::query("UPDATE clip_projects SET title=? WHERE id=?")
                    .bind(t.trim().chars().take(200).collect::<String>())
                    .bind(id)
                    .execute(&self.db)
                    .await;
            } else if !line.trim().is_empty() && Path::new(line.trim()).is_file() {
                file = Some(line.trim().to_string());
            }
        }
        let status = child.wait().await?;
        let err = err_task.await.unwrap_or_default();
        match file {
            Some(f) if status.success() => Ok(PathBuf::from(f)),
            _ => Err(Error::Invalid(format!("Falha no download: {err}"))),
        }
    }

    /// Grava as escolhas do usuário e enfileira os aprovados.
    pub async fn render(&self, project_id: i64, req: RenderRequest) -> Result<ProjectView> {
        let pv = self.get(project_id).await?;
        if pv.project.status != "ready" {
            return Err(Error::Invalid("Projeto ainda não está pronto".into()));
        }
        let dur = pv.project.duration;
        let mut queued = Vec::new();
        for e in &req.clips {
            let Some(cur) = pv.clips.iter().find(|c| c.row.id == e.id) else { continue };
            let layout = if LAYOUTS.contains(&e.layout.as_str()) { e.layout.clone() } else { cur.row.layout.clone() };
            let start = e.start.unwrap_or(cur.row.start).clamp(0.0, dur.max(0.0));
            let end = e.end.unwrap_or(cur.row.end).clamp(start + 1.0, dur.max(start + 1.0));
            let busy = cur.row.status == "queued" || cur.row.status == "processing";
            let enqueue = e.approved && !busy;
            sqlx::query(
                "UPDATE clip_items SET approved=?, layout=?, camera_rect=?, screen_rect=?, start=?, end=?, caption_style_id=?, \
                 status=CASE WHEN ? THEN 'queued' ELSE status END, progress=CASE WHEN ? THEN 0 ELSE progress END, \
                 error_msg=CASE WHEN ? THEN NULL ELSE error_msg END WHERE id=? AND project_id=?",
            )
            .bind(e.approved)
            .bind(&layout)
            .bind(serde_json::to_string(&e.camera.clamped())?)
            .bind(serde_json::to_string(&e.screen.clamped())?)
            .bind(start)
            .bind(end)
            .bind(req.caption_style_id)
            .bind(enqueue)
            .bind(enqueue)
            .bind(enqueue)
            .bind(e.id)
            .bind(project_id)
            .execute(&self.db)
            .await?;
            if enqueue {
                queued.push(e.id);
            }
        }
        for id in queued {
            self.spawn_render(id);
        }
        self.get(project_id).await
    }

    fn spawn_render(&self, clip_id: i64) {
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = this.render_slot.clone().acquire_owned().await;
            if let Err(e) = this.render_one(clip_id).await {
                tracing::warn!("clipe {clip_id} falhou: {e}");
                let _ = sqlx::query("UPDATE clip_items SET status='error', error_msg=? WHERE id=?")
                    .bind(e.to_string())
                    .bind(clip_id)
                    .execute(&this.db)
                    .await;
            }
        });
    }

    async fn render_one(&self, clip_id: i64) -> Result<()> {
        let row: ClipRow = sqlx::query_as("SELECT * FROM clip_items WHERE id=?")
            .bind(clip_id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound("Clipe"))?;
        if row.status != "queued" {
            return Ok(());
        }
        let p: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?").bind(row.project_id).fetch_one(&self.db).await?;
        let src = p.source_path.clone().ok_or_else(|| Error::Invalid("Vídeo de origem sumiu".into()))?;
        let ff = media::ffmpeg().ok_or_else(|| Error::Ffmpeg("ffmpeg não encontrado".into()))?;
        sqlx::query("UPDATE clip_items SET status='processing', progress=0 WHERE id=?").bind(clip_id).execute(&self.db).await?;

        let (cam0, scr0) = suggest_rects(&row.layout, p.width, p.height);
        let cam = parse_rect(&row.camera_rect, cam0);
        let scr = parse_rect(&row.screen_rect, scr0);
        let len = (row.end - row.start).max(1.0);
        let (ow, oh) = if row.layout == "horizontal" { (1920, 1080) } else { (1080, 1920) };

        // Legenda opcional: palavras da transcrição do projeto, deslocadas pelo início do clipe.
        let mut subs: Option<PathBuf> = None;
        if let (Some(style_id), Some(tid)) = (row.caption_style_id, p.transcript_id) {
            let style = captions::get(&self.db, style_id).await?;
            let (words,): (String,) = sqlx::query_as("SELECT words FROM transcripts WHERE id=?").bind(tid).fetch_one(&self.db).await?;
            let words: Vec<Word> = serde_json::from_str(&words)?;
            let shifted = captions::shift(&words, row.start, Some(row.end));
            let dir = self.cfg.work_dir.join("subs");
            tokio::fs::create_dir_all(&dir).await?;
            let path = dir.join(format!("clip_{clip_id}.ass"));
            tokio::fs::write(&path, captions::build_ass(&shifted, &style.style, ow, oh)).await?;
            subs = Some(path);
        }
        let ass = subs
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|n| format!(",ass={n}"))
            .unwrap_or_default();

        let graph = match row.layout.as_str() {
            "vertical_split" => {
                // 60% de cima para a tela, 40% de baixo para a câmera.
                let (th, bh) = (1152, 768);
                format!(
                    "[0:v]split=2[a][b];\
                     [a]{},scale={ow}:{th}:force_original_aspect_ratio=increase,crop={ow}:{th},setsar=1[top];\
                     [b]{},scale={ow}:{bh}:force_original_aspect_ratio=increase,crop={ow}:{bh},setsar=1[bot];\
                     [top][bot]vstack=inputs=2{ass},format=yuv420p[out]",
                    scr.crop(),
                    cam.crop()
                )
            }
            "horizontal" => format!(
                "[0:v]scale={ow}:{oh}:force_original_aspect_ratio=decrease,pad={ow}:{oh}:(ow-iw)/2:(oh-ih)/2:black,setsar=1{ass},format=yuv420p[out]"
            ),
            _ => format!(
                "[0:v]{},scale={ow}:{oh}:force_original_aspect_ratio=increase,crop={ow}:{oh},setsar=1{ass},format=yuv420p[out]",
                cam.crop()
            ),
        };

        let enc = media::encoder().await;
        let stem: String = row
            .title
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .take(48)
            .collect();
        let out = self.cfg.output_dir().join(format!(
            "clipe_{}_{}_{}.mp4",
            if stem.trim_matches('_').is_empty() { "sem_titulo".to_string() } else { stem },
            p.id,
            clip_id
        ));
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
        args.extend(["-ss".into(), format!("{:.3}", row.start), "-t".into(), format!("{len:.3}"), "-i".into(), src]);
        args.extend(["-filter_complex".into(), graph, "-map".into(), "[out]".into(), "-map".into(), "0:a?".into()]);
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), "-c:v".into(), enc.name.into()]);
        if let Some(pr) = enc.preset {
            args.extend(["-preset".into(), pr.into()]);
        }
        args.extend(["-b:v".into(), "10M".into(), "-movflags".into(), "+faststart".into(), out.to_string_lossy().into_owned()]);

        let db = self.db.clone();
        let log = self.cfg.logs_dir().join(format!("clip_{clip_id}.log"));
        let cwd = subs.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
        run_ffmpeg(ff, &args, cwd.as_deref(), len, Some(&log), move |pct| {
            let db = db.clone();
            tokio::spawn(async move {
                let _ = sqlx::query("UPDATE clip_items SET progress=? WHERE id=?").bind(pct).bind(clip_id).execute(&db).await;
            });
        })
        .await?;
        sqlx::query("UPDATE clip_items SET status='done', progress=100, output_path=? WHERE id=?")
            .bind(out.to_string_lossy().into_owned())
            .bind(clip_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

/// Linhas de um processo sem exigir UTF-8 válido. No Windows o yt-dlp pode
/// escrever na página de código do console; um acento no título não pode
/// derrubar o download.
struct LossyLines<R> {
    inner: BufReader<R>,
    buf: Vec<u8>,
}

impl<R: tokio::io::AsyncRead + Unpin> LossyLines<R> {
    fn new(r: R) -> Self {
        Self { inner: BufReader::new(r), buf: Vec::with_capacity(512) }
    }

    async fn next(&mut self) -> Option<String> {
        self.buf.clear();
        match self.inner.read_until(b'\n', &mut self.buf).await {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(String::from_utf8_lossy(&self.buf).trim_end_matches(['\r', '\n']).to_string()),
        }
    }
}

// ------------------------------------------------------------- análise

#[derive(Debug, Clone, Deserialize)]
pub struct Suggestion {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub reason: String,
}

/// Transcrição em linhas com tempo: "[12.3-18.9] texto". Corta por pausa ou frase.
pub fn timed_lines(words: &[Word]) -> String {
    let mut out = String::new();
    let mut line: Vec<&Word> = Vec::new();
    let flush = |line: &mut Vec<&Word>, out: &mut String| {
        if let (Some(a), Some(b)) = (line.first(), line.last()) {
            let text: Vec<&str> = line.iter().map(|w| w.text.as_str()).collect();
            out.push_str(&format!("[{:.1}-{:.1}] {}\n", a.start, b.end, text.join(" ")));
        }
        line.clear();
    };
    for w in words {
        let gap = line.last().map(|l| w.start - l.end).unwrap_or(0.0);
        if !line.is_empty() && (gap > 0.8 || line.len() >= 24) {
            flush(&mut line, &mut out);
        }
        line.push(w);
        if w.text.ends_with(['.', '!', '?']) {
            flush(&mut line, &mut out);
        }
    }
    flush(&mut line, &mut out);
    out
}

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["clips"],
        "properties": {
            "clips": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["start", "end", "title", "score", "reason"],
                    "properties": {
                        "start": { "type": "number" },
                        "end": { "type": "number" },
                        "title": { "type": "string" },
                        "score": { "type": "integer" },
                        "reason": { "type": "string" }
                    }
                }
            }
        }
    })
}

const SYSTEM: &str = "Você é editor de vídeos curtos (Reels, Shorts, TikTok) especializado em retenção. \
Recebe a transcrição de um vídeo longo com tempos em segundos e escolhe os trechos com maior chance de \
prender quem assiste do começo ao fim, sem depender do resto do vídeo.\n\
Critérios: (1) gancho forte nos primeiros 3 segundos: pergunta, afirmação surpreendente, conflito ou promessa; \
(2) uma ideia completa com desfecho ou virada; (3) entendível sozinho, sem citar algo dito antes; \
(4) energia e emoção na fala.\n\
Regras: comece e termine nos limites das linhas da transcrição (nunca no meio de uma frase); \
respeite a faixa de duração pedida; trechos não podem se sobrepor; \
score de 0 a 100 é a chance de retenção; title é um título curto e chamativo no idioma do vídeo; \
reason é uma frase em português explicando por que o trecho prende.";

/// Pede os trechos ao LLM e ajusta os limites às palavras reais.
pub async fn analyze(s: &ai::AiSettings, words: &[Word], min: f64, max: f64, duration: f64) -> Result<Vec<Suggestion>> {
    let user = format!(
        "Duração de cada clipe: entre {min:.0} e {max:.0} segundos.\nSugira até {MAX_SUGGESTIONS} clipes, \
         do melhor para o pior.\n\nTranscrição:\n{}",
        timed_lines(words)
    );
    let v = llm::json_call(s, SYSTEM, &user, &schema()).await?;
    let raw: Vec<Suggestion> = serde_json::from_value(v["clips"].clone()).unwrap_or_default();
    Ok(refine(raw, words, min, max, duration))
}

/// Encaixa início/fim nas palavras, aplica a faixa de duração, remove
/// sobreposições e mantém os melhores.
pub fn refine(mut raw: Vec<Suggestion>, words: &[Word], min: f64, max: f64, duration: f64) -> Vec<Suggestion> {
    raw.sort_by(|a, b| b.score.cmp(&a.score));
    let mut out: Vec<Suggestion> = Vec::new();
    for mut s in raw {
        if !(s.start.is_finite() && s.end.is_finite()) || s.end <= s.start {
            continue;
        }
        // Início na palavra que começa mais perto; fim na palavra que termina mais perto.
        if let Some(w) = words.iter().min_by(|a, b| (a.start - s.start).abs().total_cmp(&(b.start - s.start).abs())) {
            s.start = w.start;
        }
        if let Some(w) = words.iter().min_by(|a, b| (a.end - s.end).abs().total_cmp(&(b.end - s.end).abs())) {
            s.end = w.end;
        }
        let len = s.end - s.start;
        // Tolerância de 15% para não descartar um bom trecho por poucos segundos.
        if len < min * 0.85 || len > max * 1.15 || s.start < 0.0 || (duration > 0.0 && s.end > duration + 0.5) {
            continue;
        }
        let overlaps = out.iter().any(|o| {
            let inter = (o.end.min(s.end) - o.start.max(s.start)).max(0.0);
            inter > 0.3 * len.min(o.end - o.start)
        });
        if overlaps {
            continue;
        }
        s.score = s.score.clamp(0, 100);
        s.title = s.title.trim().chars().take(120).collect();
        s.reason = s.reason.trim().chars().take(400).collect();
        out.push(s);
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<Word> {
        (0..200).map(|i| Word { text: format!("w{i}"), start: i as f64 * 0.5, end: i as f64 * 0.5 + 0.4 }).collect()
    }

    #[test]
    fn refine_snaps_and_filters() {
        let raw = vec![
            Suggestion { start: 10.1, end: 40.2, title: "a".into(), score: 90, reason: String::new() },
            Suggestion { start: 12.0, end: 41.0, title: "sobreposto".into(), score: 80, reason: String::new() },
            Suggestion { start: 50.0, end: 52.0, title: "curto".into(), score: 70, reason: String::new() },
            Suggestion { start: 60.0, end: 95.0, title: "b".into(), score: 140, reason: String::new() },
        ];
        let r = refine(raw, &words(), 30.0, 60.0, 100.0);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].score, 100);
        assert!((r[1].start - 10.0).abs() < 1e-9);
        assert!((r[1].end - 40.4).abs() < 1e-9);
    }

    #[test]
    fn rect_crop_is_clamped() {
        let r = Rect { x: 0.9, y: -1.0, w: 0.5, h: 2.0 };
        assert_eq!(r.crop(), "crop=iw*0.10000:ih*1.00000:iw*0.90000:ih*0.00000");
    }

    #[test]
    fn timed_lines_split_on_sentence() {
        let w = vec![
            Word { text: "Oi.".into(), start: 0.0, end: 0.3 },
            Word { text: "Tudo".into(), start: 0.4, end: 0.6 },
            Word { text: "bem?".into(), start: 0.6, end: 0.9 },
        ];
        assert_eq!(timed_lines(&w), "[0.0-0.3] Oi.\n[0.4-0.9] Tudo bem?\n");
    }
}
