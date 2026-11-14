use std::path::PathBuf;

/// Configuração lida do ambiente. Mesmos nomes de variável do backend Python,
/// para o `render.yaml` e os `.env` existentes continuarem valendo.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    /// Pasta persistente: banco e fundos dos templates.
    pub storage_dir: PathBuf,
    /// Pasta de trabalho: uploads, saídas e logs.
    pub work_dir: PathBuf,
    /// Frontend compilado, servido pelo próprio servidor. Opcional.
    pub static_dir: Option<PathBuf>,
    pub max_concurrent: usize,
    pub max_videos_per_job: usize,
    pub max_upload_mb: u64,
    pub api_key: Option<String>,
    pub cors_origins: Vec<String>,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    env(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        let storage_dir = env("FLAXY_STORAGE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("storage"));
        let work_dir = env("FLAXY_WORK_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("flaxy"));
        Self {
            host: env("FLAXY_HOST").unwrap_or_else(|| "0.0.0.0".into()),
            port: env("PORT")
                .or_else(|| env("FLAXY_PORT"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(8000),
            storage_dir,
            work_dir,
            static_dir: env("FLAXY_STATIC_DIR").map(PathBuf::from),
            max_concurrent: env_parse("FLAXY_MAX_CONCURRENT", 1usize).max(1),
            max_videos_per_job: env_parse("FLAXY_MAX_VIDEOS_PER_JOB", 100usize),
            max_upload_mb: env_parse("FLAXY_MAX_UPLOAD_MB", 2048u64),
            api_key: env("FLAXY_API_KEY"),
            cors_origins: env("CORS_ORIGINS")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
        }
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.storage_dir.join("templates")
    }
    pub fn uploads_dir(&self) -> PathBuf {
        self.work_dir.join("uploads")
    }
    pub fn output_dir(&self) -> PathBuf {
        self.work_dir.join("output")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.work_dir.join("logs")
    }
    pub fn db_path(&self) -> PathBuf {
        self.storage_dir.join("flaxy.db")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for d in [
            self.storage_dir.clone(),
            self.templates_dir(),
            self.uploads_dir(),
            self.output_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

/// `canonicalize` sem o prefixo verbatim do Windows (`\\?\`), que FFmpeg e o
/// frontend não precisam ver.
pub fn canonical(p: &std::path::Path) -> std::io::Result<PathBuf> {
    const VERBATIM: &str = concat!('\\', '\\', '?', '\\');
    let c = std::fs::canonicalize(p)?;
    let s = c.to_string_lossy();
    Ok(match s.strip_prefix(VERBATIM) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_has_no_verbatim_prefix() {
        let p = super::canonical(std::path::Path::new(".")).unwrap();
        assert!(!p.to_string_lossy().starts_with(concat!('\\', '\\', '?')));
        assert!(p.is_absolute());
    }
}
