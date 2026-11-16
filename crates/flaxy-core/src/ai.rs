//! Transcrição: Whisper local (whisper.cpp) ou APIs (Groq, OpenAI, Deepgram,
//! AssemblyAI). Configuração e chaves ficam na tabela `settings`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::captions::Word;
use crate::{media, Error, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub groq: String,
    pub openai: String,
    pub deepgram: String,
    pub assemblyai: String,
    pub anthropic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// "local" | "groq" | "openai" | "deepgram" | "assemblyai"
    pub transcriber: String,
    /// Código ISO ("pt", "en"...) ou vazio para detectar.
    pub language: String,
    /// Modelo do whisper.cpp ("large-v3-turbo", "small"...).
    pub local_model: String,
    /// LLM da análise de retenção: "anthropic" | "openai" | "groq" | "ollama" | "".
    pub llm_provider: String,
    pub llm_model: String,
    pub ollama_url: String,
    pub keys: Keys,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            transcriber: "local".into(),
            language: "pt".into(),
            local_model: "large-v3-turbo".into(),
            llm_provider: String::new(),
            llm_model: String::new(),
            ollama_url: String::new(),
            keys: Keys::default(),
        }
    }
}

impl AiSettings {
    /// Versão segura para o frontend: chaves viram "••••1234".
    pub fn masked(&self) -> Value {
        let m = |k: &str| {
            if k.is_empty() {
                String::new()
            } else {
                let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("••••{tail}")
            }
        };
        serde_json::json!({
            "transcriber": self.transcriber,
            "language": self.language,
            "local_model": self.local_model,
            "llm_provider": self.llm_provider,
            "llm_model": self.llm_model,
            "ollama_url": self.ollama_url,
            "keys": {
                "groq": m(&self.keys.groq),
                "openai": m(&self.keys.openai),
                "deepgram": m(&self.keys.deepgram),
                "assemblyai": m(&self.keys.assemblyai),
                "anthropic": m(&self.keys.anthropic),
            }
        })
    }
}

pub async fn load(db: &SqlitePool) -> Result<AiSettings> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'ai'")
        .fetch_optional(db)
        .await?;
    Ok(row.and_then(|r| serde_json::from_str(&r.0).ok()).unwrap_or_default())
}

/// Salva. Chave vazia ou mascarada ("••••") mantém a chave atual.
pub async fn save(db: &SqlitePool, mut new: AiSettings) -> Result<AiSettings> {
    let old = load(db).await?;
    let keep = |n: &mut String, o: &str| {
        if n.starts_with('•') {
            *n = o.to_string();
        }
        *n = n.trim().to_string();
    };
    keep(&mut new.keys.groq, &old.keys.groq);
    keep(&mut new.keys.openai, &old.keys.openai);
    keep(&mut new.keys.deepgram, &old.keys.deepgram);
    keep(&mut new.keys.assemblyai, &old.keys.assemblyai);
    keep(&mut new.keys.anthropic, &old.keys.anthropic);
    if !MODELS.iter().any(|(n, _)| *n == new.local_model) {
        new.local_model = "large-v3-turbo".into();
    }
    sqlx::query("INSERT INTO settings (key, value) VALUES ('ai', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(serde_json::to_string(&new)?)
        .execute(db)
        .await?;
    Ok(new)
}

// ---------------------------------------------------------- whisper local

/// Modelos oferecidos e tamanho aproximado em MB.
pub const MODELS: &[(&str, u64)] = &[
    ("large-v3-turbo", 1624),
    ("medium", 1533),
    ("small", 488),
    ("base", 148),
];

pub fn model_file(models_dir: &Path, name: &str) -> PathBuf {
    models_dir.join(format!("ggml-{name}.bin"))
}

pub fn model_url(name: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
}

static WHISPER: OnceLock<Option<PathBuf>> = OnceLock::new();

/// `whisper-cli` em FLAXY_WHISPER_DIR (empacotado) ou no PATH.
pub fn whisper_cli() -> Option<&'static Path> {
    WHISPER
        .get_or_init(|| {
            let names = if cfg!(windows) {
                ["whisper-cli.exe", "main.exe"]
            } else {
                ["whisper-cli", "main"]
            };
            if let Ok(dir) = std::env::var("FLAXY_WHISPER_DIR") {
                for n in names {
                    let p = Path::new(&dir).join(n);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .map(|d| d.join(names[0]))
                .find(|p| p.is_file())
        })
        .as_deref()
}

pub fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("flaxy/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .build()
            .expect("http client")
    })
}

/// Baixa um modelo com progresso (0..=100). Grava em `.part` e renomeia no fim.
pub async fn download_model<F: FnMut(u8)>(models_dir: &Path, name: &str, mut on: F) -> Result<PathBuf> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    if !MODELS.iter().any(|(n, _)| *n == name) {
        return Err(Error::Invalid(format!("Modelo desconhecido: {name}")));
    }
    tokio::fs::create_dir_all(models_dir).await?;
    let dest = model_file(models_dir, name);
    let part = dest.with_extension("part");
    let res = http()
        .get(model_url(name))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Error::Invalid(format!("Falha no download: {e}")))?;
    let total = res.content_length().unwrap_or(0);
    let mut file = tokio::io::BufWriter::new(tokio::fs::File::create(&part).await?);
    let mut got: u64 = 0;
    let mut last = 255u8;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Invalid(format!("Download interrompido: {e}")))?;
        file.write_all(&chunk).await?;
        got += chunk.len() as u64;
        if total > 0 {
            let pct = ((got * 100) / total).min(100) as u8;
            if pct != last {
                last = pct;
                on(pct);
            }
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, &dest).await?;
    Ok(dest)
}

// ---------------------------------------------------------- transcrição

/// Extrai o áudio em formato leve para transcrever.
async fn extract_audio(input: &Path, out: &Path, wav: bool) -> Result<()> {
    let ff = media::ffmpeg().ok_or_else(|| Error::Ffmpeg("ffmpeg não encontrado".into()))?;
    let mut cmd = crate::media::command(ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-ar", "16000"]);
    if wav {
        cmd.args(["-c:a", "pcm_s16le"]);
    } else {
        // ~0,35 MB/min: cabe no limite de 25 MB das APIs até ~70 min.
        cmd.args(["-c:a", "libmp3lame", "-b:a", "48k"]);
    }
    let out_s = cmd.arg(out).kill_on_drop(true).output().await?;
    if !out_s.status.success() {
        return Err(Error::Ffmpeg(String::from_utf8_lossy(&out_s.stderr).trim().to_string()));
    }
    Ok(())
}

pub struct TranscribeCtx<'a> {
    pub settings: &'a AiSettings,
    pub models_dir: &'a Path,
    pub work_dir: &'a Path,
}

/// Transcreve `input` com o provedor configurado e devolve palavras com tempo.
pub async fn transcribe(ctx: &TranscribeCtx<'_>, input: &Path) -> Result<Vec<Word>> {
    let s = ctx.settings;
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let local = s.transcriber == "local";
    let audio = ctx.work_dir.join(format!("asr_{tag}.{}", if local { "wav" } else { "mp3" }));
    extract_audio(input, &audio, local).await?;
    let result = match s.transcriber.as_str() {
        "local" => local_whisper(ctx, &audio).await,
        // Rodízio: cada Whisper do Groq tem a sua cota grátis. Se o turbo bater
        // no limite, o large-v3 transcreve.
        "groq" => match openai_like(&audio, "https://api.groq.com/openai/v1", &s.keys.groq, "whisper-large-v3-turbo", &s.language).await {
            Err(e) if e.to_string().contains(" 429 ") => {
                tracing::warn!("Groq whisper-large-v3-turbo no limite, usando whisper-large-v3: {e}");
                openai_like(&audio, "https://api.groq.com/openai/v1", &s.keys.groq, "whisper-large-v3", &s.language).await
            }
            r => r,
        },
        "openai" => openai_like(&audio, "https://api.openai.com/v1", &s.keys.openai, "whisper-1", &s.language).await,
        "deepgram" => deepgram(&audio, &s.keys.deepgram, &s.language).await,
        "assemblyai" => assemblyai(&audio, &s.keys.assemblyai, &s.language).await,
        other => Err(Error::Invalid(format!("Provedor desconhecido: {other}"))),
    };
    let _ = tokio::fs::remove_file(&audio).await;
    let mut words = result?;
    words.retain(|w| !w.text.trim().is_empty() && w.end >= w.start);
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(words)
}

fn need_key(k: &str, name: &str) -> Result<()> {
    if k.is_empty() {
        return Err(Error::Invalid(format!("Cadastre a chave da {name} na aba IA")));
    }
    Ok(())
}

async fn api_err(res: reqwest::Response, name: &str) -> Error {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v["error"]["message"].as_str().or(v["error"].as_str()).or(v["err_msg"].as_str()).map(String::from)
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    Error::Invalid(format!("{name} respondeu {status}: {msg}"))
}

async fn local_whisper(ctx: &TranscribeCtx<'_>, wav: &Path) -> Result<Vec<Word>> {
    let cli = whisper_cli().ok_or_else(|| {
        Error::Invalid("whisper-cli não encontrado. Reinstale o FLAXY ou escolha uma API na aba IA".into())
    })?;
    let model = model_file(ctx.models_dir, &ctx.settings.local_model);
    if !model.is_file() {
        return Err(Error::Invalid(format!(
            "Modelo {} não baixado. Baixe na aba IA",
            ctx.settings.local_model
        )));
    }
    let base = wav.with_extension("");
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let lang = if ctx.settings.language.is_empty() { "auto" } else { &ctx.settings.language };
    // -ml 1 + -sow: um segmento por palavra, com tempo próprio.
    let out = crate::media::command(cli)
        .arg("-m").arg(&model)
        .arg("-f").arg(wav)
        .args(["-l", lang, "-t", &threads.to_string(), "-ml", "1", "-sow", "-oj", "-np"])
        .arg("-of").arg(&base)
        .kill_on_drop(true)
        .output()
        .await?;
    let json_path = base.with_extension("json");
    if !out.status.success() || !json_path.is_file() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        return Err(Error::Invalid(format!("whisper-cli falhou: {tail}")));
    }
    let raw = tokio::fs::read(&json_path).await?;
    let _ = tokio::fs::remove_file(&json_path).await;
    let v: Value = serde_json::from_slice(&raw)?;
    let mut words = Vec::new();
    for seg in v["transcription"].as_array().into_iter().flatten() {
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();
        let a = seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
        let b = seg["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0;
        if text.is_empty() {
            continue;
        }
        // Pontuação solta vira parte da palavra anterior.
        if text.chars().all(|c| c.is_ascii_punctuation()) {
            if let Some(last) = words.last_mut() {
                let l: &mut Word = last;
                l.text.push_str(&text);
                l.end = b.max(l.end);
            }
            continue;
        }
        words.push(Word { text, start: a, end: b });
    }
    Ok(words)
}

/// OpenAI e Groq usam a mesma API de transcrição.
async fn openai_like(audio: &Path, base: &str, key: &str, model: &str, lang: &str) -> Result<Vec<Word>> {
    let name = if base.contains("groq") { "Groq" } else { "OpenAI" };
    need_key(key, name)?;
    let bytes = tokio::fs::read(audio).await?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(bytes).file_name("audio.mp3").mime_str("audio/mpeg").unwrap())
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "word");
    if !lang.is_empty() {
        form = form.text("language", lang.to_string());
    }
    let res = http()
        .post(format!("{base}/audio/transcriptions"))
        .bearer_auth(key)
        .multipart(form)
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
    if !res.status().is_success() {
        return Err(api_err(res, name).await);
    }
    let v: Value = res.json().await.map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
    Ok(v["words"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| Word {
            text: w["word"].as_str().unwrap_or("").trim().to_string(),
            start: w["start"].as_f64().unwrap_or(0.0),
            end: w["end"].as_f64().unwrap_or(0.0),
        })
        .collect())
}

async fn deepgram(audio: &Path, key: &str, lang: &str) -> Result<Vec<Word>> {
    need_key(key, "Deepgram")?;
    let bytes = tokio::fs::read(audio).await?;
    let lang_q = match lang {
        "" => "detect_language=true".to_string(),
        "pt" => "language=pt-BR".to_string(),
        l => format!("language={l}"),
    };
    let res = http()
        .post(format!("https://api.deepgram.com/v1/listen?model=nova-3&smart_format=true&punctuate=true&{lang_q}"))
        .header("Authorization", format!("Token {key}"))
        .header("Content-Type", "audio/mpeg")
        .body(bytes)
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("Deepgram: {e}")))?;
    if !res.status().is_success() {
        return Err(api_err(res, "Deepgram").await);
    }
    let v: Value = res.json().await.map_err(|e| Error::Invalid(format!("Deepgram: {e}")))?;
    Ok(v["results"]["channels"][0]["alternatives"][0]["words"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| Word {
            text: w["punctuated_word"].as_str().or(w["word"].as_str()).unwrap_or("").to_string(),
            start: w["start"].as_f64().unwrap_or(0.0),
            end: w["end"].as_f64().unwrap_or(0.0),
        })
        .collect())
}

async fn assemblyai(audio: &Path, key: &str, lang: &str) -> Result<Vec<Word>> {
    need_key(key, "AssemblyAI")?;
    let base = "https://api.assemblyai.com/v2";
    let bytes = tokio::fs::read(audio).await?;
    let up = http()
        .post(format!("{base}/upload"))
        .header("authorization", key)
        .body(bytes)
        .timeout(Duration::from_secs(900))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?;
    if !up.status().is_success() {
        return Err(api_err(up, "AssemblyAI").await);
    }
    let up: Value = up.json().await.map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?;
    let mut body = serde_json::json!({ "audio_url": up["upload_url"], "punctuate": true });
    if lang.is_empty() {
        body["language_detection"] = true.into();
    } else {
        body["language_code"] = lang.into();
    }
    let job = http()
        .post(format!("{base}/transcript"))
        .header("authorization", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?;
    if !job.status().is_success() {
        return Err(api_err(job, "AssemblyAI").await);
    }
    let job: Value = job.json().await.map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?;
    let id = job["id"].as_str().unwrap_or_default().to_string();
    for _ in 0..900 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let r: Value = http()
            .get(format!("{base}/transcript/{id}"))
            .header("authorization", key)
            .send()
            .await
            .map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?
            .json()
            .await
            .map_err(|e| Error::Invalid(format!("AssemblyAI: {e}")))?;
        match r["status"].as_str() {
            Some("completed") => {
                return Ok(r["words"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|w| Word {
                        text: w["text"].as_str().unwrap_or("").to_string(),
                        start: w["start"].as_f64().unwrap_or(0.0) / 1000.0,
                        end: w["end"].as_f64().unwrap_or(0.0) / 1000.0,
                    })
                    .collect())
            }
            Some("error") => {
                return Err(Error::Invalid(format!(
                    "AssemblyAI: {}",
                    r["error"].as_str().unwrap_or("falhou")
                )))
            }
            _ => {}
        }
    }
    Err(Error::Invalid("AssemblyAI: tempo esgotado".into()))
}
