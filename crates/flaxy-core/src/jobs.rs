//! Lotes de renderização: persistência no SQLite e fila com concorrência limitada.
//!
//! Diferença para o backend antigo: jobs e itens ficam no banco, então
//! sobrevivem a reinícios. Itens que estavam em "processing" quando o processo
//! caiu voltam para a fila na inicialização.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::compose::{self, ComposeJob};
use crate::models::{Job, JobCreate, JobItem, JobRow, ProgressEvent, Template};
use crate::{Config, Error, Result};

#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}

struct Inner {
    db: SqlitePool,
    cfg: Config,
    events: broadcast::Sender<ProgressEvent>,
    queue: mpsc::UnboundedSender<i64>,
}

impl Jobs {
    /// Cria a fila e inicia o despachante. Retoma itens interrompidos.
    pub async fn start(db: SqlitePool, cfg: Config) -> Result<Self> {
        let (events, _) = broadcast::channel(1024);
        let (tx, rx) = mpsc::unbounded_channel();
        let jobs = Self { inner: Arc::new(Inner { db, cfg, events, queue: tx }) };

        sqlx::query("UPDATE job_items SET status='queued', progress=0 WHERE status='processing'")
            .execute(&jobs.inner.db)
            .await?;
        let pending: Vec<(i64,)> = sqlx::query_as(
            "SELECT DISTINCT job_id FROM job_items WHERE status='queued' ORDER BY job_id",
        )
        .fetch_all(&jobs.inner.db)
        .await?;

        tokio::spawn(dispatcher(jobs.clone(), rx));
        for (id,) in pending {
            jobs.enqueue(id);
        }
        Ok(jobs)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProgressEvent> {
        self.inner.events.subscribe()
    }

    fn enqueue(&self, job_id: i64) {
        let _ = self.inner.queue.send(job_id);
    }

    fn emit(&self, job_id: i64, item_id: i64, status: &str, progress: i64) {
        let _ = self.inner.events.send(ProgressEvent {
            job_id,
            item_id,
            status: status.to_string(),
            progress,
        });
    }

    pub async fn create(&self, data: JobCreate) -> Result<Job> {
        let cfg = &self.inner.cfg;
        if data.items.is_empty() {
            return Err(Error::Invalid("Nenhum vídeo no lote".into()));
        }
        if data.items.len() > cfg.max_videos_per_job {
            return Err(Error::Invalid(format!(
                "Máximo de {} vídeos por lote",
                cfg.max_videos_per_job
            )));
        }
        let tpl = self.template(data.template_id).await?;
        if !Path::new(&tpl.file_path).exists() {
            return Err(Error::Invalid(
                "O arquivo de fundo do template não está mais disponível".into(),
            ));
        }

        // App local: os vídeos são usados direto da pasta do usuário, sem cópia.
        let mut entries = Vec::with_capacity(data.items.len());
        for item in &data.items {
            let p = crate::config::canonical(std::path::Path::new(&item.path))
                .map_err(|_| Error::Invalid(format!("Arquivo não encontrado: {}", item.path)))?;
            let mut overrides = item.overrides.clone();
            if let Some(ap) = overrides.audio_path.as_deref() {
                let ap = crate::config::canonical(std::path::Path::new(ap))
                    .map_err(|_| Error::Invalid("Faixa de áudio não encontrada".into()))?;
                overrides.audio_path = Some(ap.to_string_lossy().into_owned());
            }
            let ov = if overrides.is_default() {
                None
            } else {
                Some(serde_json::to_string(&overrides)?)
            };
            entries.push((p, ov));
        }

        let mut tx = self.inner.db.begin().await?;
        let (job_id,): (i64,) =
            sqlx::query_as("INSERT INTO jobs (template_id) VALUES (?) RETURNING id")
                .bind(data.template_id)
                .fetch_one(&mut *tx)
                .await?;
        for (path, ov) in entries {
            sqlx::query("INSERT INTO job_items (job_id, input_path, overrides) VALUES (?, ?, ?)")
                .bind(job_id)
                .bind(path.to_string_lossy().into_owned())
                .bind(ov)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        self.enqueue(job_id);
        self.get(job_id).await
    }

    pub async fn get(&self, job_id: i64) -> Result<Job> {
        let job: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id = ?")
            .bind(job_id)
            .fetch_optional(&self.inner.db)
            .await?
            .ok_or(Error::NotFound("Job"))?;
        let items: Vec<JobItem> =
            sqlx::query_as("SELECT * FROM job_items WHERE job_id = ? ORDER BY id")
                .bind(job_id)
                .fetch_all(&self.inner.db)
                .await?;
        Ok(Job { job, items })
    }

    pub async fn list(&self, limit: i64) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> = sqlx::query_as("SELECT * FROM jobs ORDER BY id DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.inner.db)
            .await?;
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let min_id = rows.last().map(|r| r.id).unwrap_or(0);
        let items: Vec<JobItem> =
            sqlx::query_as("SELECT * FROM job_items WHERE job_id >= ? ORDER BY id")
                .bind(min_id)
                .fetch_all(&self.inner.db)
                .await?;
        Ok(rows
            .into_iter()
            .map(|job| {
                let its = items.iter().filter(|i| i.job_id == job.id).cloned().collect();
                Job { job, items: its }
            })
            .collect())
    }

    /// Cancela o lote. O item que já está no FFmpeg termina; os seguintes não começam.
    pub async fn cancel(&self, job_id: i64) -> Result<u64> {
        self.get(job_id).await?;
        sqlx::query("UPDATE jobs SET status='canceled', canceled=1 WHERE id=?")
            .bind(job_id)
            .execute(&self.inner.db)
            .await?;
        let r = sqlx::query(
            "UPDATE job_items SET status='canceled' WHERE job_id=? AND status='queued'",
        )
        .bind(job_id)
        .execute(&self.inner.db)
        .await?;
        Ok(r.rows_affected())
    }

    pub async fn retry(&self, job_id: i64, item_id: i64) -> Result<()> {
        let r = sqlx::query(
            "UPDATE job_items SET status='queued', progress=0, error_msg=NULL \
             WHERE id=? AND job_id=? AND status IN ('error','canceled','done')",
        )
        .bind(item_id)
        .bind(job_id)
        .execute(&self.inner.db)
        .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound("Item"));
        }
        sqlx::query("UPDATE jobs SET status='queued', canceled=0 WHERE id=?")
            .bind(job_id)
            .execute(&self.inner.db)
            .await?;
        self.enqueue(job_id);
        Ok(())
    }

    /// Caminhos de entrada de itens ainda pendentes (não podem ser apagados).
    pub async fn active_inputs(&self) -> Result<Vec<String>> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT input_path, overrides FROM job_items WHERE status IN ('queued','processing')",
        )
        .fetch_all(&self.inner.db)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for (input, ov) in rows {
            out.push(input);
            // A faixa de áudio de um lote pendente também não pode sumir.
            let audio = ov
                .and_then(|s| serde_json::from_str::<crate::models::ItemOverrides>(&s).ok())
                .and_then(|o| o.audio_path);
            out.extend(audio);
        }
        Ok(out)
    }

    async fn template(&self, id: i64) -> Result<Template> {
        sqlx::query_as("SELECT * FROM templates WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.inner.db)
            .await?
            .ok_or(Error::NotFound("Template"))
    }

    async fn is_canceled(&self, job_id: i64) -> bool {
        sqlx::query_as::<_, (bool,)>("SELECT canceled FROM jobs WHERE id=?")
            .bind(job_id)
            .fetch_optional(&self.inner.db)
            .await
            .ok()
            .flatten()
            .map(|r| r.0)
            .unwrap_or(true)
    }

    async fn set_item(&self, item: &JobItem, status: &str, progress: i64) {
        let _ = sqlx::query("UPDATE job_items SET status=?, progress=? WHERE id=?")
            .bind(status)
            .bind(progress)
            .bind(item.id)
            .execute(&self.inner.db)
            .await;
        self.emit(item.job_id, item.id, status, progress);
    }

    /// Processa os itens pendentes de um lote, respeitando o limite global.
    async fn run_job(&self, job_id: i64, slots: Arc<Semaphore>) {
        let _ = sqlx::query("UPDATE jobs SET status='processing' WHERE id=?")
            .bind(job_id)
            .execute(&self.inner.db)
            .await;
        let Ok(job) = self.get(job_id).await else { return };
        let Ok(tpl) = self.template(job.job.template_id).await else {
            let _ = sqlx::query("UPDATE jobs SET status='error' WHERE id=?")
                .bind(job_id)
                .execute(&self.inner.db)
                .await;
            return;
        };
        let tpl = Arc::new(tpl);

        let mut handles = Vec::new();
        for item in job.items.into_iter().filter(|i| i.status == "queued") {
            let permit = slots.clone().acquire_owned().await.expect("semaphore");
            if self.is_canceled(job_id).await {
                break;
            }
            let this = self.clone();
            let tpl = tpl.clone();
            handles.push(tokio::spawn(async move {
                this.process_item(&item, &tpl).await;
                drop(permit);
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        let final_status = if self.is_canceled(job_id).await {
            "canceled"
        } else {
            let (bad,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM job_items WHERE job_id=? AND status != 'done'",
            )
            .bind(job_id)
            .fetch_one(&self.inner.db)
            .await
            .unwrap_or((1,));
            if bad == 0 { "done" } else { "error" }
        };
        let _ = sqlx::query("UPDATE jobs SET status=? WHERE id=?")
            .bind(final_status)
            .bind(job_id)
            .execute(&self.inner.db)
            .await;
    }

    async fn fail(&self, item: &JobItem, msg: &str, log: Option<&str>) {
        let _ = sqlx::query("UPDATE job_items SET status='error', error_msg=?, log_path=? WHERE id=?")
            .bind(msg)
            .bind(log)
            .bind(item.id)
            .execute(&self.inner.db)
            .await;
        self.emit(item.job_id, item.id, "error", 0);
    }

    /// Gera o .ass do item (palavras deslocadas pelo trim). Sem legenda: None.
    async fn write_subtitles(&self, item: &JobItem, tpl: &Template, o: &crate::models::ItemOverrides) -> Result<Option<PathBuf>> {
        let (Some(style_id), Some(tid)) = (o.caption_style_id, o.transcript_id) else {
            return Ok(None);
        };
        let style = crate::captions::get(&self.inner.db, style_id).await?;
        let (status, words): (String, String) =
            sqlx::query_as("SELECT status, words FROM transcripts WHERE id = ?")
                .bind(tid)
                .fetch_optional(&self.inner.db)
                .await?
                .ok_or(Error::NotFound("Transcrição"))?;
        if status != "done" {
            return Err(Error::Invalid("Transcrição ainda não terminou".into()));
        }
        let words: Vec<crate::captions::Word> = serde_json::from_str(&words)?;
        let words = crate::captions::shift(&words, o.trim_start.unwrap_or(0.0), o.trim_end);
        let ass = crate::captions::build_ass(&words, &style.style, tpl.output_w, tpl.output_h);
        let dir = self.inner.cfg.work_dir.join("subs");
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("item_{}.ass", item.id));
        tokio::fs::write(&path, ass).await?;
        Ok(Some(path))
    }

    async fn process_item(&self, item: &JobItem, tpl: &Template) {
        let cfg = &self.inner.cfg;
        self.set_item(item, "processing", 0).await;

        let stem = safe_stem(Path::new(&item.input_path));
        let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let out: PathBuf =
            cfg.output_dir().join(format!("{stem}_{stamp}_{}.{}", item.id, tpl.output_format));
        let log = cfg.logs_dir().join(format!("job_{}_item_{}.log", item.job_id, item.id));
        let overrides = item.overrides();
        let subtitles = match self.write_subtitles(item, tpl, &overrides).await {
            Ok(s) => s,
            Err(e) => {
                self.fail(item, &e.to_string(), None).await;
                return;
            }
        };

        let db = self.inner.db.clone();
        let events = self.inner.events.clone();
        let (job_id, item_id) = (item.job_id, item.id);
        let result = compose::compose(
            ComposeJob {
                template: tpl,
                input: Path::new(&item.input_path),
                output: &out,
                log: Some(log.clone()),
                overrides: &overrides,
                subtitles: subtitles.clone(),
            },
            move |pct| {
                let _ = events.send(ProgressEvent {
                    job_id,
                    item_id,
                    status: "processing".into(),
                    progress: pct,
                });
                let db = db.clone();
                tokio::spawn(async move {
                    let _ = sqlx::query("UPDATE job_items SET progress=? WHERE id=?")
                        .bind(pct)
                        .bind(item_id)
                        .execute(&db)
                        .await;
                });
            },
        )
        .await;

        let log_s = log.to_string_lossy().into_owned();
        match result {
            Ok(()) => {
                let _ = sqlx::query(
                    "UPDATE job_items SET status='done', progress=100, output_path=?, log_path=? WHERE id=?",
                )
                .bind(out.to_string_lossy().into_owned())
                .bind(&log_s)
                .bind(item.id)
                .execute(&self.inner.db)
                .await;
                self.emit(item.job_id, item.id, "done", 100);
            }
            Err(e) => {
                tracing::warn!("item {} falhou: {e}", item.id);
                let _ = tokio::fs::remove_file(&out).await;
                let _ = sqlx::query(
                    "UPDATE job_items SET status='error', error_msg=?, log_path=? WHERE id=?",
                )
                .bind(e.to_string())
                .bind(&log_s)
                .bind(item.id)
                .execute(&self.inner.db)
                .await;
                self.emit(item.job_id, item.id, "error", 0);
            }
        }
    }
}

/// Um único semáforo global limita renders simultâneos entre todos os lotes.
async fn dispatcher(jobs: Jobs, mut rx: mpsc::UnboundedReceiver<i64>) {
    let slots = Arc::new(Semaphore::new(jobs.inner.cfg.max_concurrent));
    let running = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
    while let Some(job_id) = rx.recv().await {
        // Evita dois laços sobre o mesmo lote (ex.: retry durante o processamento).
        if !running.lock().await.insert(job_id) {
            continue;
        }
        let (jobs, slots, running) = (jobs.clone(), slots.clone(), running.clone());
        tokio::spawn(async move {
            loop {
                jobs.run_job(job_id, slots.clone()).await;
                // Um retry pode ter chegado enquanto o laço rodava.
                let (pending,): (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM job_items WHERE job_id=? AND status='queued'",
                )
                .bind(job_id)
                .fetch_one(&jobs.inner.db)
                .await
                .unwrap_or((0,));
                if pending == 0 || jobs.is_canceled(job_id).await {
                    break;
                }
            }
            running.lock().await.remove(&job_id);
        });
    }
}

pub fn safe_stem(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let clean: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect();
    if clean.is_empty() { "video".into() } else { clean }
}
