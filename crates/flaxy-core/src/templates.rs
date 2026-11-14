//! CRUD de templates. O arquivo de fundo fica em `storage/templates`.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::models::{Template, TemplateInput};
use crate::{Error, Result};

pub const ALLOWED_BG: &[&str] = &[
    "mp4", "mov", "webm", "mkv", "avi", "png", "jpg", "jpeg", "webp", "bmp",
];
pub const ALLOWED_VIDEO: &[&str] = &["mp4", "mov", "webm", "mkv", "avi", "m4v"];
pub const ALLOWED_AUDIO: &[&str] = &["mp3", "wav", "m4a", "aac", "ogg", "opus", "flac"];

/// Nome único e seguro: preserva o nome original e acrescenta sufixo aleatório,
/// para dois envios com o mesmo nome não se sobrescreverem.
pub fn unique_filename(original: &str, allowed: &[&str]) -> Result<String> {
    let p = Path::new(original);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !allowed.contains(&ext.as_str()) {
        return Err(Error::Invalid(format!(
            "Formato não suportado: {}",
            if ext.is_empty() { "sem extensão" } else { &ext }
        )));
    }
    let stem: String = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("arquivo")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(80)
        .collect();
    let stem = if stem.is_empty() { "arquivo".to_string() } else { stem };
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..8];
    Ok(format!("{stem}_{tag}.{ext}"))
}

/// Resolve `name` dentro de `dir`, barrando path traversal.
pub fn inside(dir: &Path, name: &str) -> Result<PathBuf> {
    let file = Path::new(name)
        .file_name()
        .ok_or_else(|| Error::Invalid("Nome de arquivo inválido".into()))?;
    Ok(dir.join(file))
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Template>> {
    Ok(sqlx::query_as("SELECT * FROM templates ORDER BY created_at DESC")
        .fetch_all(db)
        .await?)
}

pub async fn get(db: &SqlitePool, id: i64) -> Result<Template> {
    sqlx::query_as("SELECT * FROM templates WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Template"))
}

pub async fn create(db: &SqlitePool, data: TemplateInput, file_path: &str) -> Result<Template> {
    let d = data.sanitize();
    if d.name.trim().is_empty() {
        return Err(Error::Invalid("Nome do template é obrigatório".into()));
    }
    Ok(sqlx::query_as(
        "INSERT INTO templates (name, file_path, overlay_x, overlay_y, overlay_w, overlay_h, \
         fit_mode, output_w, output_h, output_format, video_bitrate, audio_source, \
         audio_mix_raw, audio_mix_template, duration_rule) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) RETURNING *",
    )
    .bind(d.name.trim())
    .bind(file_path)
    .bind(d.overlay_x)
    .bind(d.overlay_y)
    .bind(d.overlay_w)
    .bind(d.overlay_h)
    .bind(&d.fit_mode)
    .bind(d.output_w)
    .bind(d.output_h)
    .bind(&d.output_format)
    .bind(&d.video_bitrate)
    .bind(&d.audio_source)
    .bind(d.audio_mix_raw)
    .bind(d.audio_mix_template)
    .bind(&d.duration_rule)
    .fetch_one(db)
    .await?)
}

pub async fn update(db: &SqlitePool, id: i64, data: TemplateInput) -> Result<Template> {
    let d = data.sanitize();
    sqlx::query_as(
        "UPDATE templates SET name=?, overlay_x=?, overlay_y=?, overlay_w=?, overlay_h=?, \
         fit_mode=?, output_w=?, output_h=?, output_format=?, video_bitrate=?, audio_source=?, \
         audio_mix_raw=?, audio_mix_template=?, duration_rule=? WHERE id=? RETURNING *",
    )
    .bind(d.name.trim())
    .bind(d.overlay_x)
    .bind(d.overlay_y)
    .bind(d.overlay_w)
    .bind(d.overlay_h)
    .bind(&d.fit_mode)
    .bind(d.output_w)
    .bind(d.output_h)
    .bind(&d.output_format)
    .bind(&d.video_bitrate)
    .bind(&d.audio_source)
    .bind(d.audio_mix_raw)
    .bind(d.audio_mix_template)
    .bind(&d.duration_rule)
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(Error::NotFound("Template"))
}

/// Apaga o template e o arquivo de fundo, se nenhum outro template o usa.
pub async fn delete(db: &SqlitePool, id: i64, templates_dir: &Path) -> Result<()> {
    let tpl = get(db, id).await?;
    sqlx::query("DELETE FROM templates WHERE id=?").bind(id).execute(db).await?;
    let (others,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM templates WHERE file_path=?")
        .bind(&tpl.file_path)
        .fetch_one(db)
        .await?;
    let path = Path::new(&tpl.file_path);
    if others == 0 && path.starts_with(templates_dir) {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
}

pub async fn duplicate(db: &SqlitePool, id: i64, templates_dir: &Path) -> Result<Template> {
    let tpl = get(db, id).await?;
    let src = Path::new(&tpl.file_path);
    let new_path = if src.exists() {
        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("fundo.png");
        let dest = templates_dir.join(unique_filename(name, ALLOWED_BG)?);
        tokio::fs::copy(src, &dest).await?;
        dest.to_string_lossy().into_owned()
    } else {
        tpl.file_path.clone()
    };
    let input = TemplateInput {
        name: format!("{} (cópia)", tpl.name),
        overlay_x: tpl.overlay_x,
        overlay_y: tpl.overlay_y,
        overlay_w: tpl.overlay_w,
        overlay_h: tpl.overlay_h,
        fit_mode: tpl.fit_mode,
        output_w: tpl.output_w,
        output_h: tpl.output_h,
        output_format: tpl.output_format,
        video_bitrate: tpl.video_bitrate,
        audio_source: tpl.audio_source,
        audio_mix_raw: tpl.audio_mix_raw,
        audio_mix_template: tpl.audio_mix_template,
        duration_rule: tpl.duration_rule,
    };
    create(db, input, &new_path).await
}
