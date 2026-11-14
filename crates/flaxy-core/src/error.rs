use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0} não encontrado")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("FFmpeg falhou: {0}")]
    Ffmpeg(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
