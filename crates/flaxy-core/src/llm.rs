//! LLM plugável para análise de retenção: Anthropic, OpenAI, Groq ou Ollama.
//! Todos devolvem um JSON validado contra o mesmo schema.

use std::time::Duration;

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
        "groq" => openai_like("https://api.groq.com/openai/v1", &s.keys.groq, "Groq", need_model(model)?, system, user, schema).await,
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

/// OpenAI e Groq: Chat Completions com `json_schema` (OpenAI) ou `json_object` (Groq).
async fn openai_like(base: &str, key: &str, name: &str, model: &str, system: &str, user: &str, schema: &Value) -> Result<Value> {
    if key.is_empty() {
        return Err(Error::Invalid(format!("Cadastre a chave da {name} na aba IA")));
    }
    let format = if name == "OpenAI" {
        json!({ "type": "json_schema", "json_schema": { "name": "clips", "schema": schema, "strict": true } })
    } else {
        json!({ "type": "json_object" })
    };
    let system = format!("{system}\n\nResponda só com JSON que siga este schema:\n{schema}");
    let res = http()
        .post(format!("{base}/chat/completions"))
        .bearer_auth(key)
        .json(&json!({
            "model": model,
            "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
            "response_format": format
        }))
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
    if !res.status().is_success() {
        return Err(err_of(res, name).await);
    }
    let v: Value = res.json().await.map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
    parse_json(v["choices"][0]["message"]["content"].as_str().unwrap_or(""), name)
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
