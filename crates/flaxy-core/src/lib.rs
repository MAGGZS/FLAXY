//! Núcleo do FLAXY: domínio, banco, FFmpeg e fila de renderização.
//!
//! O servidor web e o app desktop usam este crate do mesmo jeito. Nada aqui
//! conhece HTTP.

pub mod ai;
pub mod captions;
pub mod compose;
pub mod config;
pub mod db;
pub mod error;
pub mod clips;
pub mod jobs;
pub mod llm;
pub mod media;
pub mod models;
pub mod templates;
pub mod transcripts;

pub use config::Config;
pub use error::{Error, Result};
