//! LLM plugável para análise de retenção: Anthropic, OpenAI, Groq ou Ollama.
//! Todos devolvem um JSON validado contra o mesmo schema.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::ai::{http, AiSettings};
use crate::{Error, Result};

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Pede ao LLM um objeto JSON que segue `schema`.
pub async fn json_call(s: &AiSettings, system: &str, user: &str, schema: &Value) -> Result<Value> {
    let model = s.llm_model.trim();
    match s.llm_provider.as_str() {
        "anthropic" => anthropic(&s.keys.anthropic, if model.is_empty() { DEFAULT_ANTHROPIC_MODEL } else { model }, system, user, schema).await,
        "openai" => openai_like("https://api.openai.com/v1", &s.keys.openai, "OpenAI", need_model(model)?, system, user, schema).await,
        "groq" => groq(&s.keys.groq, model, system, user, schema).await,
        "ollama" => ollama(if s.ollama_url.trim().is_empty() { DEFAULT_OLLAMA_URL } else { s.ollama_url.trim() }, need_model(model)?, system, user, schema).await,
        "" => Err(Error::Invalid("Escolha o provedor de LLM na aba IA".into())),
        other => Err(Error::Invalid(format!("Provedor de LLM desconhecido: {other}"))),
    }
}

fn need_model(m: &str) -> Result<&str> {
    if m.is_empty() {
        Err(Error::Invalid("Informe o modelo do LLM na aba IA".into()))
    } else {
        Ok(m)
    }
}

fn parse_json(text: &str, name: &str) -> Result<Value> {
    // Tolera cercas ```json que alguns modelos abertos ainda colocam.
    let t = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    serde_json::from_str(t).map_err(|e| Error::Invalid(format!("{name} devolveu JSON inválido: {e}")))
}

async fn err_of(res: reqwest::Response, name: &str) -> Error {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().or(v["error"].as_str()).map(String::from))
        .unwrap_or_else(|| body.chars().take(240).collect());
    Error::Invalid(format!("{name} respondeu {status}: {msg}"))
}

/// Messages API com saída estruturada. Fallback server-side ligado: se o
/// modelo recusar por política, a API tenta o modelo padrão de reserva.
async fn anthropic(key: &str, model: &str, system: &str, user: &str, schema: &Value) -> Result<Value> {
    if key.is_empty() {
        return Err(Error::Invalid("Cadastre a chave da Anthropic na aba IA".into()));
    }
    let body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "output_config": {
            "effort": "medium",
            "format": { "type": "json_schema", "schema": schema }
        },
        "fallbacks": "default"
    });
    let res = http()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2026-10-21")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .json(&body)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("Anthropic: {e}")))?;
    if !res.status().is_success() {
        return Err(err_of(res, "Anthropic").await);
    }
    let v: Value = res.json().await.map_err(|e| Error::Invalid(format!("Anthropic: {e}")))?;
    match v["stop_reason"].as_str() {
        Some("refusal") => return Err(Error::Invalid("O modelo recusou analisar este vídeo".into())),
        Some("max_tokens") => return Err(Error::Invalid("Resposta do modelo cortada (vídeo longo demais)".into())),
        _ => {}
    }
    let text: String = v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    parse_json(&text, "Anthropic")
}

/// Limite de tokens de saída por minuto que o Groq informa no erro 429
/// ("... output tokens per minute (OTPM): Limit 1000, Requested 1333 ...").
fn otpm_limit(msg: &str) -> Option<u32> {
    let i = msg.find("(OTPM)")?;
    let rest = &msg[i..];
    let j = rest.find("Limit")? + "Limit".len();
    rest[j..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// Por que uma chamada falhou: `Skip` = problema deste modelo (limite, modelo
/// sem suporte, resposta ruim), vale tentar outro; `Fatal` = problema geral
/// (chave inválida, sem internet), não adianta trocar de modelo.
enum Fail {
    Skip { msg: String, cool: u64 },
    Fatal(Error),
}

impl From<Fail> for Error {
    fn from(f: Fail) -> Self {
        match f {
            Fail::Skip { msg, .. } => Error::Invalid(msg),
            Fail::Fatal(e) => e,
        }
    }
}

/// OpenAI: Chat Completions com `json_schema`, um modelo só.
async fn openai_like(base: &str, key: &str, name: &str, model: &str, system: &str, user: &str, schema: &Value) -> Result<Value> {
    if key.is_empty() {
        return Err(Error::Invalid(format!("Cadastre a chave da {name} na aba IA")));
    }
    Ok(chat_once(base, key, name, model, system, user, schema).await?)
}

/// Uma chamada de Chat Completions. No Groq gratuito o limite de saída por
/// minuto pode ser menor que o padrão do modelo e o pedido é recusado antes de
/// rodar: aí tenta de novo uma vez com a saída limitada abaixo do teto e
/// pedindo respostas curtas.
async fn chat_once(base: &str, key: &str, name: &str, model: &str, system: &str, user: &str, schema: &Value) -> std::result::Result<Value, Fail> {
    let format = if name == "OpenAI" {
        json!({ "type": "json_schema", "json_schema": { "name": "clips", "schema": schema, "strict": true } })
    } else {
        json!({ "type": "json_object" })
    };
    let system = format!("{system}\n\nResponda só com JSON que siga este schema:\n{schema}");
    let mut cap: Option<u32> = None;
    loop {
        let sys = match cap {
            Some(c) => format!(
                "{system}\n\nLimite apertado de resposta (cerca de {c} tokens): sugira no máximo {} clipes, títulos curtos e o motivo com até 15 palavras.",
                (c / 160).clamp(2, 10)
            ),
            None => system.clone(),
        };
        let mut body = json!({
            "model": model,
            "messages": [{ "role": "system", "content": sys }, { "role": "user", "content": user }],
            "response_format": format
        });
        if let Some(c) = cap {
            body["max_completion_tokens"] = json!(c);
        }
        let res = http()
            .post(format!("{base}/chat/completions"))
            .bearer_auth(key)
            .json(&body)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| Fail::Fatal(Error::Invalid(format!("{name}: {e}"))))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            let msg = err_of(res, name).await.to_string();
            if status == 429 && cap.is_none() {
                if let Some(limit) = otpm_limit(&msg).filter(|l| *l >= 300) {
                    cap = Some(limit - limit / 10);
                    tracing::warn!("{name} {model}: limite de saída por minuto {limit}, tentando com {cap:?}");
                    continue;
                }
            }
            let cool = match status {
                401 | 403 => return Err(Fail::Fatal(Error::Invalid(msg))),
                429 if msg.contains("per day") || msg.contains("(TPD)") || msg.contains("(RPD)") => 1800,
                429 | 413 => 65,
                500..=599 => 30,
                // 400/404/422: modelo desativado, sem modo JSON etc.
                _ => 3600,
            };
            return Err(Fail::Skip { msg, cool });
        }
        let v: Value = res.json().await.map_err(|e| Fail::Skip { msg: format!("{name}: {e}"), cool: 0 })?;
        if v["choices"][0]["finish_reason"] == "length" {
            return Err(Fail::Skip { msg: format!("{name} {model}: a resposta foi cortada pelo limite de tokens"), cool: 0 });
        }
        return parse_json(v["choices"][0]["message"]["content"].as_str().unwrap_or(""), name).map_err(|e| Fail::Skip { msg: e.to_string(), cool: 0 });
    }
}

const GROQ: &str = "https://api.groq.com/openai/v1";

/// Modelos de chat da chave (lista do próprio Groq, guardada por 1 hora) e
/// até quando cada modelo fica de lado depois de bater no limite.
#[derive(Default)]
struct GroqState {
    models: Option<(Instant, Vec<String>)>,
    cool: HashMap<String, Instant>,
}

static GROQ_STATE: OnceLock<Mutex<GroqState>> = OnceLock::new();

fn groq_state() -> std::sync::MutexGuard<'static, GroqState> {
    GROQ_STATE.get_or_init(Default::default).lock().unwrap()
}

/// Modelos de texto ativos da chave, os de contexto maior primeiro. Fora:
/// transcrição, voz e moderação.
async fn groq_models(key: &str) -> Vec<String> {
    if let Some((at, m)) = groq_state().models.as_ref() {
        if at.elapsed() < Duration::from_secs(3600) {
            return m.clone();
        }
    }
    let got = async {
        let v: Value = http().get(format!("{GROQ}/models")).bearer_auth(key).timeout(Duration::from_secs(20)).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
        let mut list: Vec<(i64, String)> = v["data"]
            .as_array()?
            .iter()
            .filter(|m| m["active"].as_bool() != Some(false))
            .filter_map(|m| Some((m["context_window"].as_i64().unwrap_or(0), m["id"].as_str()?.to_string())))
            .filter(|(_, id)| !["whisper", "tts", "guard", "orpheus", "playai", "distil"].iter().any(|x| id.to_lowercase().contains(x)))
            .collect();
        list.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        Some(list.into_iter().map(|(_, id)| id).collect::<Vec<_>>())
    }
    .await
    .unwrap_or_default();
    if !got.is_empty() {
        groq_state().models = Some((Instant::now(), got.clone()));
    }
    got
}

/// Ordem de tentativa: o modelo escolhido na aba IA, depois os outros da chave.
fn groq_order(chosen: &str, models: Vec<String>) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    if !chosen.is_empty() {
        order.push(chosen.to_string());
    }
    for m in models {
        if !order.contains(&m) {
            order.push(m);
        }
    }
    order
}

/// Groq com rodízio: começa no modelo escolhido na aba IA e, se ele bater no
/// limite (cada modelo do Groq tem a sua cota grátis) ou não servir, passa
/// para o próximo modelo da chave. Modelo que bateu no limite fica de lado até
/// a cota dele voltar.
async fn groq(key: &str, chosen: &str, system: &str, user: &str, schema: &Value) -> Result<Value> {
    if key.is_empty() {
        return Err(Error::Invalid("Cadastre a chave da Groq na aba IA".into()));
    }
    let order = groq_order(chosen, groq_models(key).await);
    if order.is_empty() {
        return Err(Error::Invalid("Informe o modelo do LLM na aba IA".into()));
    }
    let mut tried = 0;
    let mut wait: Option<u64> = None;
    for m in &order {
        let until = groq_state().cool.get(m).copied();
        if let Some(u) = until.filter(|u| *u > Instant::now()) {
            let left = u.duration_since(Instant::now()).as_secs();
            wait = Some(wait.map_or(left, |w| w.min(left)));
            continue;
        }
        tried += 1;
        match chat_once(GROQ, key, "Groq", m, system, user, schema).await {
            Ok(v) => {
                if m != chosen {
                    tracing::info!("Groq: análise feita com {m} ({chosen} no limite ou indisponível)");
                }
                return Ok(v);
            }
            Err(Fail::Fatal(e)) => return Err(e),
            Err(Fail::Skip { msg, cool }) => {
                tracing::warn!("Groq {m}: {msg}");
                if cool > 0 {
                    groq_state().cool.insert(m.clone(), Instant::now() + Duration::from_secs(cool));
                    wait = Some(wait.map_or(cool, |x| x.min(cool)));
                }
            }
        }
    }
    let when = match wait {
        Some(s) if s < 120 => format!("em {s} segundos"),
        Some(s) => format!("em {} minutos", s.div_ceil(60)),
        None => "daqui a pouco".into(),
    };
    Err(Error::Invalid(format!(
        "Todos os {} modelos do Groq da sua chave estão no limite agora ({tried} tentados). Tente de novo {when}",
        order.len()
    )))
}

#[cfg(test)]
mod tests {
    #[test]
    fn otpm() {
        let m = "Groq respondeu 429 Too Many Requests: Request too large for model `qwen/qwen3.8-27b` in organization `org_x` service tier `on_demand` on output tokens per minute (OTPM): Limit 1000, Requested 1333. The request's expected output";
        assert_eq!(super::otpm_limit(m), Some(1000));
        assert_eq!(super::otpm_limit("Rate limit reached on tokens per minute (TPM): Limit 6000"), None);
    }

    #[test]
    fn groq_order_starts_with_chosen() {
        let o = super::groq_order("b", vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(o, vec!["b", "a", "c"]);
        assert_eq!(super::groq_order("", vec!["a".into()]), vec!["a"]);
    }
}

async fn ollama(base: &str, model: &str, system: &str, user: &str, schema: &Value) -> Result<Value> {
    let res = http()
        .post(format!("{}/api/chat", base.trim_end_matches('/')))
        .json(&json!({
            "model": model,
            "stream": false,
            "format": schema,
            "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }]
        }))
        .timeout(Duration::from_secs(1800))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("Ollama em {base}: {e}")))?;
    if !res.status().is_success() {
        return Err(err_of(res, "Ollama").await);
    }
    let v: Value = res.json().await.map_err(|e| Error::Invalid(format!("Ollama: {e}")))?;
    parse_json(v["message"]["content"].as_str().unwrap_or(""), "Ollama")
}
