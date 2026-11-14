//! Rotas HTTP + WebSocket. Toda a API fica sob `/api`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use flaxy_core::jobs::Jobs;
use flaxy_core::models::{JobCreate, TemplateInput};
use flaxy_core::templates::{self as tpls, ALLOWED_AUDIO, ALLOWED_BG, ALLOWED_VIDEO};
use flaxy_core::{media, Config};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::io::AsyncWriteExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower::ServiceExt;

use crate::error::{ApiError, ApiResult};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub db: SqlitePool,
    pub jobs: Jobs,
    pub transcripts: flaxy_core::transcripts::Transcripts,
    pub clips: flaxy_core::clips::Clips,
    pub downloads: crate::ai_api::Downloads,
}

impl AppState {
    pub fn new(cfg: Config, db: SqlitePool, jobs: Jobs, transcripts: flaxy_core::transcripts::Transcripts, clips: flaxy_core::clips::Clips) -> Self {
        Self(Arc::new(Inner { cfg, db, jobs, transcripts, clips, downloads: Default::default() }))
    }
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/templates", get(list_templates).post(create_template))
        .route(
            "/templates/{id}",
            get(get_template).put(update_template).delete(delete_template),
        )
        .route("/templates/{id}/file", get(template_file))
        .route("/templates/{id}/duplicate", post(duplicate_template))
        .route("/upload", post(upload_videos))
        .route("/upload-audio", post(upload_audio))
        .route("/uploads", get(list_uploads))
        .route("/uploads/prune", post(prune_uploads))
        .route("/uploads/{name}", get(get_upload).delete(delete_upload))
        .route("/jobs", get(list_jobs).post(create_job))
        .route("/jobs/{id}", get(get_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/items/{item}/retry", post(retry_item))
        .route("/output", get(list_output))
        .route("/output/{name}", get(get_output).delete(delete_output))
        .route("/output-zip", get(output_zip))
        .route("/stats", get(stats))
        .merge(crate::ai_api::routes())
        .merge(crate::clips_api::routes())
        .route("/ws", get(ws))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_key))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::disable());

    let mut app = Router::new().nest("/api", api).with_state(state.clone());

    if let Some(dir) = &state.cfg.static_dir {
        // SPA: rota desconhecida cai no index.html.
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }

    let origins: Vec<HeaderValue> =
        state.cfg.cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::HeaderName::from_static("x-flaxy-key")])
        .allow_origin(if origins.is_empty() {
            AllowOrigin::any()
        } else {
            AllowOrigin::list(origins)
        });
    app.layer(cors)
}

// ---------------------------------------------------------------- infra

/// Segredo compartilhado opcional (`FLAXY_API_KEY`). Cabeçalho ou `?key=`,
/// porque `<video src>` não manda cabeçalho.
async fn require_key(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = st.cfg.api_key.as_deref() else {
        return next.run(req).await;
    };
    let header_ok = req
        .headers()
        .get("x-flaxy-key")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == expected);
    let query_ok = req.uri().query().is_some_and(|q| {
        q.split('&').any(|kv| kv.strip_prefix("key=").is_some_and(|v| v == expected))
    });
    if header_ok || query_ok {
        next.run(req).await
    } else {
        ApiError(StatusCode::UNAUTHORIZED, "Chave de acesso ausente ou inválida".into())
            .into_response()
    }
}

async fn health(State(st): State<AppState>) -> Json<Value> {
    let enc = media::encoder().await;
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "ffmpeg": media::ffmpeg().is_some(),
        "encoder": enc.name,
        "max_videos_per_job": st.cfg.max_videos_per_job,
        "max_upload_mb": st.cfg.max_upload_mb,
    }))
}

async fn ws(State(st): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let rx = st.jobs.subscribe();
    upgrade.on_upgrade(move |socket| ws_loop(socket, rx))
}

async fn ws_loop(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<flaxy_core::models::ProgressEvent>,
) {
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let text = serde_json::to_string(&ev).unwrap_or_default();
                    if socket.send(Message::Text(text.into())).await.is_err() { break; }
                }
                // Cliente lento: perdeu eventos, mas segue recebendo os próximos.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

// ------------------------------------------------------------ templates

async fn list_templates(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    Ok(Json(tpls::list(&st.db).await?))
}

async fn get_template(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    Ok(Json(tpls::get(&st.db, id).await?))
}

/// Multipart: campo `file` (fundo) + `data` (JSON do TemplateInput).
async fn create_template(State(st): State<AppState>, mut mp: Multipart) -> ApiResult<impl IntoResponse> {
    let mut input = TemplateInput::default();
    let mut saved: Option<PathBuf> = None;
    let dir = st.cfg.templates_dir();
    let limit = st.cfg.max_upload_mb * 1024 * 1024;

    while let Some(field) = mp.next_field().await.map_err(|e| ApiError::bad(e.to_string()))? {
        match field.name() {
            Some("data") => {
                let text = field.text().await.map_err(|e| ApiError::bad(e.to_string()))?;
                input = serde_json::from_str(&text).map_err(|e| ApiError::bad(format!("data inválido: {e}")))?;
            }
            Some("file") => {
                let name = field.file_name().unwrap_or("fundo").to_string();
                let dest = dir.join(tpls::unique_filename(&name, ALLOWED_BG)?);
                save_field(field, &dest, limit).await?;
                saved = Some(dest);
            }
            _ => {}
        }
    }
    let Some(path) = saved else { return Err(ApiError::bad("Arquivo de fundo ausente")) };
    match tpls::create(&st.db, input, &path.to_string_lossy()).await {
        Ok(t) => Ok((StatusCode::CREATED, Json(t))),
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            Err(e.into())
        }
    }
}

async fn update_template(
    State(st): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    Json(data): Json<TemplateInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(tpls::update(&st.db, id, data).await?))
}

async fn delete_template(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    tpls::delete(&st.db, id, &st.cfg.templates_dir()).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn duplicate_template(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    let t = tpls::duplicate(&st.db, id, &st.cfg.templates_dir()).await?;
    Ok((StatusCode::CREATED, Json(t)))
}

async fn template_file(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, req: Request) -> ApiResult<Response> {
    let t = tpls::get(&st.db, id).await?;
    serve_file(Path::new(&t.file_path), req, None).await
}

// -------------------------------------------------------------- uploads

/// Grava cada vídeo em streaming, sem carregar o arquivo inteiro na memória.
async fn upload_videos(State(st): State<AppState>, mut mp: Multipart) -> ApiResult<impl IntoResponse> {
    let dir = st.cfg.uploads_dir();
    let limit = st.cfg.max_upload_mb * 1024 * 1024;
    let mut items = Vec::new();
    while let Some(field) = mp.next_field().await.map_err(|e| ApiError::bad(e.to_string()))? {
        if items.len() >= st.cfg.max_videos_per_job {
            return Err(ApiError::bad(format!(
                "Máximo de {} vídeos por envio",
                st.cfg.max_videos_per_job
            )));
        }
        let original = field.file_name().unwrap_or("video.mp4").to_string();
        let dest = dir.join(tpls::unique_filename(&original, ALLOWED_VIDEO)?);
        save_field(field, &dest, limit).await?;
        let probe = media::probe(&dest).await;
        items.push(json!({
            "path": dest.to_string_lossy(),
            "file": dest.file_name().and_then(|n| n.to_str()),
            "name": original,
            "duration": probe.duration,
            "has_audio": probe.has_audio,
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// Uma faixa de áudio (música/narração) para mixar nos vídeos do lote.
async fn upload_audio(State(st): State<AppState>, mut mp: Multipart) -> ApiResult<impl IntoResponse> {
    let limit = st.cfg.max_upload_mb * 1024 * 1024;
    let field = mp
        .next_field()
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?
        .ok_or_else(|| ApiError::bad("Arquivo de áudio ausente"))?;
    let original = field.file_name().unwrap_or("audio.mp3").to_string();
    let dest = st.cfg.uploads_dir().join(tpls::unique_filename(&original, ALLOWED_AUDIO)?);
    save_field(field, &dest, limit).await?;
    let probe = media::probe(&dest).await;
    if !probe.has_audio {
        let _ = tokio::fs::remove_file(&dest).await;
        return Err(ApiError::bad("O arquivo não tem faixa de áudio"));
    }
    Ok(Json(json!({
        "path": dest.to_string_lossy(),
        "file": dest.file_name().and_then(|n| n.to_str()),
        "name": original,
        "duration": probe.duration,
    })))
}

async fn list_uploads(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let mut items = Vec::new();
    let mut rd = tokio::fs::read_dir(st.cfg.uploads_dir()).await?;
    while let Some(e) = rd.next_entry().await? {
        let meta = e.metadata().await?;
        if meta.is_file() {
            items.push(json!({
                "name": e.file_name().to_string_lossy(),
                "path": e.path().to_string_lossy(),
                "size_bytes": meta.len(),
            }));
        }
    }
    Ok(Json(json!({ "items": items })))
}

async fn get_upload(State(st): State<AppState>, UrlPath(name): UrlPath<String>, req: Request) -> ApiResult<Response> {
    let p = tpls::inside(&st.cfg.uploads_dir(), &name)?;
    serve_file(&p, req, None).await
}

async fn delete_upload(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult<impl IntoResponse> {
    let p = tpls::inside(&st.cfg.uploads_dir(), &name)?;
    let _ = tokio::fs::remove_file(p).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct PruneBody {
    #[serde(default)]
    keep: Vec<String>,
}

/// Apaga uploads que não estão em `keep` nem em lotes pendentes.
async fn prune_uploads(State(st): State<AppState>, Json(body): Json<PruneBody>) -> ApiResult<impl IntoResponse> {
    let mut keep: std::collections::HashSet<String> = body
        .keep
        .iter()
        .filter_map(|n| Path::new(n).file_name().map(|f| f.to_string_lossy().into_owned()))
        .collect();
    for p in st.jobs.active_inputs().await? {
        if let Some(f) = Path::new(&p).file_name() {
            keep.insert(f.to_string_lossy().into_owned());
        }
    }
    let mut removed = 0;
    let mut rd = tokio::fs::read_dir(st.cfg.uploads_dir()).await?;
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.metadata().await?.is_file() && !keep.contains(&name) {
            let _ = tokio::fs::remove_file(e.path()).await;
            removed += 1;
        }
    }
    Ok(Json(json!({ "removed": removed })))
}

// ----------------------------------------------------------------- jobs

async fn create_job(State(st): State<AppState>, Json(data): Json<JobCreate>) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(st.jobs.create(data).await?)))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

async fn list_jobs(State(st): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.jobs.list(q.limit.unwrap_or(50).clamp(1, 500)).await?))
}

async fn get_job(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    Ok(Json(st.jobs.get(id).await?))
}

async fn cancel_job(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult<impl IntoResponse> {
    let n = st.jobs.cancel(id).await?;
    Ok(Json(json!({ "ok": true, "canceled_items": n })))
}

async fn retry_item(State(st): State<AppState>, UrlPath((id, item)): UrlPath<(i64, i64)>) -> ApiResult<impl IntoResponse> {
    st.jobs.retry(id, item).await?;
    Ok(Json(json!({ "ok": true })))
}

// --------------------------------------------------------------- output

async fn list_output(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let mut items = Vec::new();
    let mut rd = tokio::fs::read_dir(st.cfg.output_dir()).await?;
    while let Some(e) = rd.next_entry().await? {
        let meta = e.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
            .map(|d| d.to_rfc3339());
        items.push(json!({
            "name": e.file_name().to_string_lossy(),
            "size_bytes": meta.len(),
            "modified_at": modified,
        }));
    }
    items.sort_by(|a, b| b["modified_at"].as_str().cmp(&a["modified_at"].as_str()));
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
struct DownloadQuery {
    download: Option<bool>,
}

async fn get_output(
    State(st): State<AppState>,
    UrlPath(name): UrlPath<String>,
    Query(q): Query<DownloadQuery>,
    req: Request,
) -> ApiResult<Response> {
    let p = tpls::inside(&st.cfg.output_dir(), &name)?;
    let attach = q.download.unwrap_or(false).then_some(name.as_str());
    serve_file(&p, req, attach).await
}

async fn delete_output(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult<impl IntoResponse> {
    let p = tpls::inside(&st.cfg.output_dir(), &name)?;
    tokio::fs::remove_file(&p).await.map_err(|_| ApiError::not_found("Arquivo não encontrado"))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ZipQuery {
    filenames: String,
}

/// ZIP sem compressão: vídeo já é comprimido, deflate só gastaria CPU.
async fn output_zip(State(st): State<AppState>, Query(q): Query<ZipQuery>) -> ApiResult<Response> {
    let dir = st.cfg.output_dir();
    let mut files = Vec::new();
    for n in q.filenames.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let p = tpls::inside(&dir, n)?;
        if p.is_file() {
            files.push(p);
        }
    }
    if files.is_empty() {
        return Err(ApiError::not_found("Nenhum dos arquivos existe mais"));
    }
    let tmp = st.cfg.work_dir.join(format!("zip_{}.zip", uuid::Uuid::new_v4().simple()));
    let tmp2 = tmp.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(&tmp2)?));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            zw.start_file(name, opts).map_err(std::io::Error::other)?;
            std::io::copy(&mut std::fs::File::open(&f)?, &mut zw)?;
        }
        zw.finish().map_err(std::io::Error::other)?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;

    let file = tokio::fs::File::open(&tmp).await?;
    let len = file.metadata().await?.len();
    // No Windows o arquivo aberto não pode ser apagado; no Unix some do disco já.
    #[cfg(unix)]
    let _ = tokio::fs::remove_file(&tmp).await;
    #[cfg(not(unix))]
    {
        let t = tmp.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            let _ = tokio::fs::remove_file(t).await;
        });
    }
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M");
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, len)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"flaxy_{stamp}.zip\""),
        )
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
        .unwrap())
}

// ---------------------------------------------------------------- stats

/// Números do painel inicial, todos vindos do banco e do disco.
async fn stats(State(st): State<AppState>) -> ApiResult<impl IntoResponse> {
    let by_status: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM job_items GROUP BY status")
            .fetch_all(&st.db)
            .await
            .map_err(flaxy_core::Error::from)?;
    let daily: Vec<(String, i64)> = sqlx::query_as(
        "SELECT substr(j.created_at, 1, 10) d, COUNT(*) FROM job_items i \
         JOIN jobs j ON j.id = i.job_id \
         WHERE i.status = 'done' AND j.created_at >= strftime('%Y-%m-%d', 'now', '-6 days') \
         GROUP BY d ORDER BY d",
    )
    .fetch_all(&st.db)
    .await
    .map_err(flaxy_core::Error::from)?;
    let (templates,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM templates")
        .fetch_one(&st.db)
        .await
        .map_err(flaxy_core::Error::from)?;

    let mut sizes = serde_json::Map::new();
    for (k, d) in [
        ("uploads", st.cfg.uploads_dir()),
        ("output", st.cfg.output_dir()),
        ("templates", st.cfg.templates_dir()),
    ] {
        sizes.insert(k.into(), json!(dir_size(&d).await));
    }

    Ok(Json(json!({
        "items_by_status": by_status.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "done_last_7_days": daily.into_iter().map(|(d, n)| json!({"date": d, "count": n})).collect::<Vec<_>>(),
        "templates": templates,
        "disk_bytes": sizes,
    })))
}

async fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if let Ok(m) = e.metadata().await {
                if m.is_file() {
                    total += m.len();
                }
            }
        }
    }
    total
}

// -------------------------------------------------------------- helpers

async fn save_field(
    mut field: axum::extract::multipart::Field<'_>,
    dest: &Path,
    limit: u64,
) -> ApiResult<()> {
    let mut out = tokio::io::BufWriter::with_capacity(1 << 20, tokio::fs::File::create(dest).await?);
    let mut written: u64 = 0;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                drop(out);
                let _ = tokio::fs::remove_file(dest).await;
                return Err(ApiError::bad(e.to_string()));
            }
        };
        written += chunk.len() as u64;
        if limit > 0 && written > limit {
            drop(out);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("Arquivo passa do limite de {} MB", limit / 1024 / 1024),
            ));
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(())
}

/// Serve arquivo com suporte a Range (necessário para o `<video>` buscar).
pub async fn serve_file(path: &Path, req: Request, attachment: Option<&str>) -> ApiResult<Response> {
    if !path.is_file() {
        return Err(ApiError::not_found("Arquivo não encontrado"));
    }
    let mut res = ServeFile::new(path)
        .oneshot(req)
        .await
        .unwrap_or_else(|never| match never {})
        .map(Body::new);
    if let Some(name) = attachment {
        let safe: String = name.chars().filter(|c| *c != '"' && !c.is_control()).collect();
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe}\"")) {
            res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(res)
}
