//! Rotas de clipes de retenção.

use axum::extract::{Path as UrlPath, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use flaxy_core::clips::{NewProject, RenderRequest};
use serde_json::json;

use crate::api::{serve_file, AppState};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/clips", get(list).post(create))
        .route("/clips/tools", get(tools))
        .route("/clips/{id}", get(get_one).delete(delete))
        .route("/clips/{id}/render", post(render))
        .route("/clips/{id}/source", get(source))
}

async fn tools() -> impl IntoResponse {
    Json(json!({ "ytdlp": flaxy_core::clips::ytdlp().is_some() }))
}

async fn list(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.clips.list().await?))
}

async fn create(State(st): State<AppState>, Json(b): Json<NewProject>) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(st.clips.create(b).await?)))
}

async fn get_one(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.clips.get(id).await?))
}

async fn delete(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    st.clips.delete(id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn render(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, Json(b): Json<RenderRequest>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.clips.render(id, b).await?))
}

/// Vídeo de origem baixado, para a prévia e a marcação de câmera/tela.
async fn source(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, req: Request) -> ApiResult<Response> {
    let p = st.clips.get(id).await?;
    let src = p.project.source_path.ok_or_else(|| ApiError::not_found("Vídeo ainda não baixado"))?;
    serve_file(std::path::Path::new(&src), req, None).await
}
