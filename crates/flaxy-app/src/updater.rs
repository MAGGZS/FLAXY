//! Atualização pelo próprio app, sem rodar o instalador de novo.
//!
//! Fonte: a última release do GitHub (`REPO`). Ela precisa ter um asset
//! `FLAXY.exe` (gerado por `release.bat`). O fluxo:
//! 1. consulta `releases/latest` e compara a tag com a versão em execução;
//! 2. baixa o novo executável ao lado do atual (`FLAXY.exe.new`) mostrando o
//!    progresso;
//! 3. confere tamanho, SHA-256 (o GitHub publica o `digest` de cada asset) e o
//!    cabeçalho de executável do Windows;
//! 4. troca os arquivos: o Windows deixa renomear um .exe em uso, então o atual
//!    vira `FLAXY.exe.old` e o novo assume o nome. A próxima abertura apaga o
//!    `.old`;
//! 5. "Reiniciar" fecha a janela e `main` abre o executável novo.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const REPO: &str = "MAGGZS/FLAXY";
const ASSET: &str = "FLAXY.exe";
/// Argumento passado ao executável novo: espera a instância antiga fechar e
/// avisa que a atualização entrou.
pub const RELAUNCH_ARG: &str = "--updated";

/// Pedido de reinício: lido por `main` depois que a janela fecha.
static RELAUNCH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    url: String,
    size: u64,
    sha256: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate { latest: String },
    Available(Release),
    Downloading { version: String, done: u64, total: u64 },
    Installing { version: String },
    Ready { version: String },
    Failed(String),
}

#[derive(Clone, Default)]
pub struct Updater(Arc<Mutex<Phase>>);

impl Updater {
    pub fn phase(&self) -> Phase {
        self.0.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn set(&self, ctx: &egui::Context, p: Phase) {
        if let Ok(mut g) = self.0.lock() {
            *g = p;
        }
        ctx.request_repaint();
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase(), Phase::Checking | Phase::Downloading { .. } | Phase::Installing { .. })
    }

    /// Consulta a última release. Roda no runtime do backend.
    pub fn check(&self, ctx: &egui::Context) -> impl std::future::Future<Output = ()> + Send + 'static {
        let (me, ctx) = (self.clone(), ctx.clone());
        me.set(&ctx, Phase::Checking);
        async move {
            let p = match latest().await {
                Ok(Some(r)) if newer(&r.version, env!("CARGO_PKG_VERSION")) => Phase::Available(r),
                Ok(Some(r)) => Phase::UpToDate { latest: r.version },
                Ok(None) => Phase::Failed("Nenhuma versão publicada ainda no GitHub.".into()),
                Err(e) => Phase::Failed(e),
            };
            me.set(&ctx, p);
        }
    }

    /// Baixa, confere e instala a release. Roda no runtime do backend.
    pub fn install(&self, ctx: &egui::Context, r: Release) -> impl std::future::Future<Output = ()> + Send + 'static {
        let (me, ctx) = (self.clone(), ctx.clone());
        me.set(&ctx, Phase::Downloading { version: r.version.clone(), done: 0, total: r.size });
        async move {
            let p = match download_and_swap(&me, &ctx, &r).await {
                Ok(()) => {
                    tracing::info!("atualizado para {}", r.version);
                    Phase::Ready { version: r.version }
                }
                Err(e) => {
                    tracing::warn!("falha ao atualizar: {e}");
                    Phase::Failed(e)
                }
            };
            me.set(&ctx, p);
        }
    }
}

/// Marca o reinício e fecha a janela. `main` abre o executável novo depois que
/// a fila foi encerrada.
pub fn restart(ctx: &egui::Context) {
    RELAUNCH.store(true, Ordering::SeqCst);
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

/// O usuário desistiu de fechar (havia render em andamento).
pub fn cancel_restart() {
    RELAUNCH.store(false, Ordering::SeqCst);
}

pub fn relaunch_requested() -> bool {
    RELAUNCH.load(Ordering::SeqCst)
}

/// Abre o executável (já substituído) num processo novo.
pub fn spawn_new() {
    let Ok(exe) = std::env::current_exe() else { return };
    // `current_exe` pode apontar para o arquivo renomeado (.old): o novo fica
    // sempre no nome original.
    let exe = original_path(&exe);
    if let Err(e) = std::process::Command::new(&exe).arg(RELAUNCH_ARG).spawn() {
        tracing::error!("não foi possível reabrir {}: {e}", exe.display());
    }
}

fn original_path(exe: &Path) -> PathBuf {
    match exe.file_name().and_then(|n| n.to_str()) {
        Some(n) if n.ends_with(".old") => exe.with_file_name(n.trim_end_matches(".old")),
        _ => exe.to_path_buf(),
    }
}

/// Remove sobras de uma atualização anterior (o .exe antigo e downloads
/// interrompidos). Chamado na abertura.
pub fn cleanup() {
    let Ok(exe) = std::env::current_exe() else { return };
    for ext in ["old", "new"] {
        let p = sibling(&exe, ext);
        if p.exists() {
            // O processo antigo pode levar alguns instantes para soltar o arquivo.
            for _ in 0..20 {
                if std::fs::remove_file(&p).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

fn sibling(exe: &Path, ext: &str) -> PathBuf {
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or(ASSET);
    exe.with_file_name(format!("{name}.{ext}"))
}

// --- GitHub ---------------------------------------------------------------

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("FLAXY-updater/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

async fn latest() -> Result<Option<Release>, String> {
    // FLAXY_UPDATE_API troca a fonte (teste com um servidor local).
    let url = std::env::var("FLAXY_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let resp = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Sem conexão com o GitHub: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if resp.status() == reqwest::StatusCode::FORBIDDEN || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("O GitHub limitou as consultas. Tente de novo em alguns minutos.".into());
    }
    if !resp.status().is_success() {
        return Err(format!("O GitHub respondeu {}", resp.status()));
    }
    let gh: GhRelease = resp.json().await.map_err(|e| format!("Resposta inválida do GitHub: {e}"))?;
    if gh.draft {
        return Ok(None);
    }
    let version = gh.tag_name.trim_start_matches(['v', 'V']).to_string();
    let Some(a) = gh.assets.into_iter().find(|a| a.name.eq_ignore_ascii_case(ASSET)) else {
        return Err(format!("A versão {version} não tem o arquivo {ASSET} anexado."));
    };
    Ok(Some(Release {
        version,
        notes: gh.body.unwrap_or_default().trim().to_string(),
        url: a.browser_download_url,
        size: a.size,
        sha256: a.digest.and_then(|d| d.strip_prefix("sha256:").map(|h| h.to_ascii_lowercase())),
    }))
}

/// `a` é mais nova que `b`? Compara números separados por ponto ("2.10.0" > "2.9.3").
fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split(['.', '-', '+']).map(|p| p.parse().unwrap_or(0)).take(3).collect() };
    let (mut x, mut y) = (parse(a), parse(b));
    x.resize(3, 0);
    y.resize(3, 0);
    x > y
}

async fn download_and_swap(me: &Updater, ctx: &egui::Context, r: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = original_path(&exe);
    let new = sibling(&exe, "new");
    let old = sibling(&exe, "old");

    let mut resp = client()?
        .get(&r.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Falha no download: {e}"))?;
    let total = resp.content_length().unwrap_or(r.size).max(1);

    let mut file = tokio::fs::File::create(&new).await.map_err(|e| format!("Sem permissão para gravar na pasta do FLAXY: {e}"))?;
    let mut hash = Sha256::new();
    let mut done: u64 = 0;
    let mut last = std::time::Instant::now();
    loop {
        let chunk = match tokio::time::timeout(std::time::Duration::from_secs(60), resp.chunk()).await {
            Ok(Ok(Some(c))) => c,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                drop(file);
                let _ = tokio::fs::remove_file(&new).await;
                return Err(format!("Download interrompido: {e}"));
            }
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(&new).await;
                return Err("Download parado há 60 s. Verifique a conexão.".into());
            }
        };
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| format!("Falha ao gravar: {e}"))?;
        done += chunk.len() as u64;
        if last.elapsed().as_millis() > 80 {
            last = std::time::Instant::now();
            me.set(ctx, Phase::Downloading { version: r.version.clone(), done, total });
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())?;
    drop(file);
    me.set(ctx, Phase::Installing { version: r.version.clone() });

    let fail = |msg: String| async {
        let _ = tokio::fs::remove_file(&new).await;
        Err::<(), String>(msg)
    };
    if r.size > 0 && done != r.size {
        return fail(format!("Arquivo incompleto ({done} de {} bytes).", r.size)).await;
    }
    let got = format!("{:x}", hash.finalize());
    if let Some(want) = &r.sha256 {
        if &got != want {
            return fail("O arquivo baixado não confere com o publicado (SHA-256). Nada foi alterado.".into()).await;
        }
    }
    let head = tokio::fs::read(&new).await.map(|b| b.get(..2) == Some(b"MZ")).unwrap_or(false);
    if !head {
        return fail("O arquivo baixado não é um executável do Windows. Nada foi alterado.".into()).await;
    }

    // Troca: atual -> .old, novo -> nome original. Se a segunda falhar, desfaz.
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("Não foi possível substituir o executável: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("Não foi possível instalar o executável novo: {e}. A versão atual foi mantida."));
    }
    set_installed_version(&r.version);
    Ok(())
}

/// Atualiza a versão exibida em "Aplicativos instalados" do Windows (só se o
/// FLAXY foi instalado pelo instalador).
#[cfg(windows)]
fn set_installed_version(v: &str) {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FLAXY";
    let exists = std::process::Command::new("reg").args(["query", key]).creation_flags(NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false);
    if exists {
        let _ = std::process::Command::new("reg").args(["add", key, "/v", "DisplayVersion", "/t", "REG_SZ", "/d", v, "/f"]).creation_flags(NO_WINDOW).output();
    }
}

#[cfg(not(windows))]
fn set_installed_version(_: &str) {}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions() {
        assert!(newer("2.0.1", "2.0.0"));
        assert!(newer("2.10.0", "2.9.9"));
        assert!(newer("3", "2.9.9"));
        assert!(!newer("2.0.0", "2.0.0"));
        assert!(!newer("1.9.9", "2.0.0"));
    }
}
