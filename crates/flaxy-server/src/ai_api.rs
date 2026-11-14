//! Rotas de IA: configurações, modelos do Whisper, transcrições e estilos de legenda.

use std::collections::HashMap;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use flaxy_core::ai::{self, AiSettings};
use flaxy_core::captions::{self, CaptionStyle, Word};
use serde::Deserialize;
use serde_json::json;

use crate::api::AppState;
use crate::error::{ApiError, ApiResult};

/// Estado dos downloads de modelo: nome -> progresso (0..100) ou erro.
#[derive(Default)]
pub struct Downloads(pub tokio::sync::Mutex<HashMap<String, Result<u8, String>>>);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ai/settings", get(get_settings).put(put_settings))
        .route("/ai/models", get(list_models))
        .route("/ai/models/{name}/download", post(download_model))
        .route("/transcripts", post(create_transcript))
        .route("/transcripts/{id}", get(get_transcript).put(put_transcript))
        .route("/caption-styles", get(list_styles).post(create_style))
        .route("/caption-styles/{id}", axum::routing::put(update_style).delete(delete_style))
}

async fn get_settings(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(ai::load(&st.db).await?.masked()))
}

async fn put_settings(State(st): State<AppState>, Json(s): Json<AiSettings>) -> ApiResult<impl IntoResponse> {
    Ok(Json(ai::save(&st.db, s).await?.masked()))
}

async fn list_models(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let dir = st.transcripts.models_dir();
    let dl = st.downloads.0.lock().await;
    let models: Vec<_> = ai::MODELS
        .iter()
        .map(|(name, mb)| {
            let installed = ai::model_file(&dir, name).is_file();
            let (progress, error) = match dl.get(*name) {
                Some(Ok(p)) if !installed => (Some(*p), None),
                Some(Err(e)) => (None, Some(e.clone())),
                _ => (None, None),
            };
            json!({ "name": name, "size_mb": mb, "installed": installed, "progress": progress, "error": error })
        })
        .collect();
    Ok(Json(json!({
        "whisper_cli": ai::whisper_cli().is_some(),
        "models": models,
    })))
}

async fn download_model(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult<impl IntoResponse> {
    if !ai::MODELS.iter().any(|(n, _)| *n == name) {
        return Err(ApiError::not_found("Modelo desconhecido"));
    }
    {
        let mut dl = st.downloads.0.lock().await;
        if matches!(dl.get(&name), Some(Ok(p)) if *p < 100) {
            return Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))));
        }
        dl.insert(name.clone(), Ok(0));
    }
    let dir = st.transcripts.models_dir();
    tokio::spawn(async move {
        let st2 = st.clone();
        let n2 = name.clone();
        let res = ai::download_model(&dir, &name, move |p| {
            let st3 = st2.clone();
            let n3 = n2.clone();
            tokio::spawn(async move {
                st3.downloads.0.lock().await.insert(n3, Ok(p));
            });
        })
        .await;
        let mut dl = st.downloads.0.lock().await;
        match res {
            Ok(_) => {
                dl.remove(&name);
            }
            Err(e) => {
                dl.insert(name, Err(e.to_string()));
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
struct NewTranscript {
    path: String,
}

async fn create_transcript(State(st): State<AppState>, Json(b): Json<NewTranscript>) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(st.transcripts.create(&b.path).await?)))
}

async fn get_transcript(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.transcripts.get(id).await?))
}

#[derive(Deserialize)]
struct EditWords {
    words: Vec<Word>,
}

async fn put_transcript(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, Json(b): Json<EditWords>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.transcripts.set_words(id, &b.words).await?))
}

async fn list_styles(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(captions::list(&st.db).await?))
}

#[derive(Deserialize)]
struct StyleBody {
    name: String,
    style: CaptionStyle,
}

async fn create_style(State(st): State<AppState>, Json(b): Json<StyleBody>) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(captions::create(&st.db, &b.name, &b.style).await?)))
}

async fn update_style(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, Json(b): Json<StyleBody>) -> ApiResult<impl IntoResponse> {
    Ok(Json(captions::update(&st.db, id, &b.name, &b.style).await?))
}

async fn delete_style(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    captions::delete(&st.db, id).await?;
    Ok(Json(json!({ "ok": true })))
}
