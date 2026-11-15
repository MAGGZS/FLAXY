//! Fila de transcrições. Uma por vez: o Whisper local já usa todos os núcleos,
//! e as APIs têm limite de taxa.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::ai::{self, TranscribeCtx};
use crate::captions::Word;
use crate::{Config, Error, Result};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TranscriptRow {
    pub id: i64,
    pub source_path: String,
    pub provider: String,
    pub language: Option<String>,
    pub status: String,
    pub error_msg: Option<String>,
    #[serde(skip)]
    pub words: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    #[serde(flatten)]
    pub row: TranscriptRow,
    pub words: Vec<Word>,
}

impl From<TranscriptRow> for Transcript {
    fn from(row: TranscriptRow) -> Self {
        let words = serde_json::from_str(&row.words).unwrap_or_default();
        Transcript { row, words }
    }
}

#[derive(Clone)]
pub struct Transcripts {
    db: SqlitePool,
    cfg: Config,
    slot: Arc<Semaphore>,
}

impl Transcripts {
    pub async fn start(db: SqlitePool, cfg: Config) -> Result<Self> {
        let t = Self { db, cfg, slot: Arc::new(Semaphore::new(1)) };
        // Retoma o que o processo deixou pela metade.
        let pending: Vec<(i64,)> =
            sqlx::query_as("SELECT id FROM transcripts WHERE status IN ('queued','processing') ORDER BY id")
                .fetch_all(&t.db)
                .await?;
        for (id,) in pending {
            t.spawn(id);
        }
        Ok(t)
    }

    pub fn models_dir(&self) -> std::path::PathBuf {
        self.cfg.storage_dir.join("models")
    }

    /// Enfileira a transcrição de um arquivo local.
    pub async fn create(&self, path: &str) -> Result<Transcript> {
        let p = crate::config::canonical(Path::new(path))
            .map_err(|_| Error::Invalid("Arquivo não encontrado".into()))?;
        let s = ai::load(&self.db).await?;
        let row: TranscriptRow = sqlx::query_as(
            "INSERT INTO transcripts (source_path, provider, language) VALUES (?, ?, ?) RETURNING *",
        )
        .bind(p.to_string_lossy().into_owned())
        .bind(&s.transcriber)
        .bind(&s.language)
        .fetch_one(&self.db)
        .await?;
        self.spawn(row.id);
        Ok(row.into())
    }

    pub async fn get(&self, id: i64) -> Result<Transcript> {
        let row: TranscriptRow = sqlx::query_as("SELECT * FROM transcripts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound("Transcrição"))?;
        Ok(row.into())
    }

    /// Palavras editadas (ou digitadas) pelo usuário substituem as da IA.
    pub async fn set_words(&self, id: i64, words: &[Word]) -> Result<Transcript> {
        sqlx::query("UPDATE transcripts SET words = ?, status = 'done', error_msg = NULL WHERE id = ? AND status != 'processing'")
            .bind(serde_json::to_string(words)?)
            .bind(id)
            .execute(&self.db)
            .await?;
        self.get(id).await
    }

    fn spawn(&self, id: i64) {
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = this.slot.clone().acquire_owned().await;
            this.run(id).await;
        });
    }

    async fn run(&self, id: i64) {
        let Ok(t) = self.get(id).await else { return };
        let _ = sqlx::query("UPDATE transcripts SET status='processing' WHERE id=?")
            .bind(id)
            .execute(&self.db)
            .await;
        let result = async {
            let settings = ai::load(&self.db).await?;
            let models = self.models_dir();
            let ctx = TranscribeCtx { settings: &settings, models_dir: &models, work_dir: &self.cfg.work_dir };
            ai::transcribe(&ctx, Path::new(&t.row.source_path)).await
        }
        .await;
        let q = match result {
            Ok(words) => sqlx::query("UPDATE transcripts SET status='done', words=?, error_msg=NULL WHERE id=?")
                .bind(serde_json::to_string(&words).unwrap_or_else(|_| "[]".into()))
                .bind(id),
            Err(e) => {
                tracing::warn!("transcrição {id} falhou: {e}");
                sqlx::query("UPDATE transcripts SET status='error', error_msg=? WHERE id=?")
                    .bind(e.to_string())
                    .bind(id)
            }
        };
        let _ = q.execute(&self.db).await;
    }
}
