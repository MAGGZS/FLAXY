//! Descoberta de FFmpeg/ffprobe e do encoder de GPU. Feita uma vez por processo.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct Encoder {
    pub name: &'static str,
    pub preset: Option<&'static str>,
}

static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
static FFPROBE: OnceLock<Option<PathBuf>> = OnceLock::new();
static ENCODER: tokio::sync::OnceCell<Encoder> = tokio::sync::OnceCell::const_new();

/// `Command` que não abre janela de console no Windows (app gráfico).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    if let Ok(dir) = std::env::var("FLAXY_FFMPEG_DIR") {
        let p = Path::new(&dir).join(&exe);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|p| p.is_file())
}

pub fn ffmpeg() -> Option<&'static Path> {
    FFMPEG.get_or_init(|| which("ffmpeg")).as_deref()
}

pub fn ffprobe() -> Option<&'static Path> {
    FFPROBE.get_or_init(|| which("ffprobe")).as_deref()
}

/// Escolhe o encoder H.264 mais rápido que realmente funciona nesta máquina.
///
/// Listar `-encoders` não basta: o NVENC aparece em qualquer build "full",
/// mesmo sem placa NVIDIA. Cada candidato é testado com um encode curto.
pub async fn encoder() -> &'static Encoder {
    ENCODER
        .get_or_init(|| async {
            let candidates = [
                Encoder { name: "h264_nvenc", preset: Some("p4") },
                Encoder { name: "h264_qsv", preset: Some("faster") },
                Encoder { name: "h264_amf", preset: None },
            ];
            if let Some(ff) = ffmpeg() {
                for c in candidates {
                    let ok = command(ff)
                        .args([
                            "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                            "color=black:s=256x256:d=0.1", "-frames:v", "1", "-c:v", c.name,
                            "-f", "null", "-",
                        ])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .kill_on_drop(true)
                        .status()
                        .await
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if ok {
                        tracing::info!("encoder de GPU: {}", c.name);
                        return c;
                    }
                }
            }
            tracing::info!("encoder: libx264 (CPU)");
            Encoder { name: "libx264", preset: Some("veryfast") }
        })
        .await
}

#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub duration: f64,
    pub has_audio: bool,
}

/// Duração e presença de áudio numa única chamada ao ffprobe.
pub async fn probe(path: &Path) -> Probe {
    let Some(fp) = ffprobe() else { return Probe::default() };
    let out = command(fp)
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await;
    let Ok(out) = out else { return Probe::default() };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let has_audio = v["streams"]
        .as_array()
        .map(|a| a.iter().any(|s| s["codec_type"] == "audio"))
        .unwrap_or(false);
    Probe { duration, has_audio }
}

/// Largura e altura do primeiro stream de vídeo (0,0 se não der para ler).
pub async fn dimensions(path: &Path) -> (i64, i64) {
    let Some(fp) = ffprobe() else { return (0, 0) };
    let Ok(out) = command(fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "json"])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
    else {
        return (0, 0);
    };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let s = &v["streams"][0];
    (s["width"].as_i64().unwrap_or(0), s["height"].as_i64().unwrap_or(0))
}

pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp")
    )
}
