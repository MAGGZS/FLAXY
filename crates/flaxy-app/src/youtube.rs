//! Capa e título de vídeos do YouTube, para reconhecer cada projeto de clipes
//! de relance. Busca a miniatura pública (i.ytimg.com) e o oEmbed do YouTube.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::backend::{Backend, Task};

/// Id de 11 caracteres de um link do YouTube (watch, youtu.be, shorts, embed, live).
pub fn video_id(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = u.split("://").nth(1).unwrap_or(u);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let candidate = if host == "youtu.be" {
        path.split(['?', '&', '#', '/']).next().map(str::to_string)
    } else if host == "youtube.com" || host == "youtube-nocookie.com" {
        if let Some(q) = path.strip_prefix("watch").and_then(|p| p.split_once('?')).map(|(_, q)| q) {
            q.split('&').find_map(|kv| kv.strip_prefix("v=")).map(|v| v.split('#').next().unwrap_or(v).to_string())
        } else {
            ["shorts/", "embed/", "live/", "v/"]
                .iter()
                .find_map(|pre| path.strip_prefix(pre))
                .and_then(|p| p.split(['?', '&', '#', '/']).next())
                .map(str::to_string)
        }
    } else {
        None
    };
    candidate.filter(|id| id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

#[derive(Clone)]
pub struct Info {
    pub thumb: Option<TextureHandle>,
    pub title: String,
    pub channel: String,
}

type Fetched = (Option<ColorImage>, String, String);

enum Entry {
    Loading(Task<Fetched>),
    Ready(Info),
}

#[derive(Default)]
pub struct Cache {
    map: HashMap<String, Entry>,
}

async fn fetch(id: String) -> Fetched {
    let http = flaxy_core::ai::http();
    // mqdefault é 16:9 sem as faixas pretas do hqdefault.
    let img = async {
        let bytes = http.get(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
    };
    let meta = async {
        let url = format!("https://www.youtube.com/oembed?format=json&url=https://www.youtube.com/watch?v={id}");
        let v: serde_json::Value = http.get(url).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
        Some((v["title"].as_str().unwrap_or_default().to_string(), v["author_name"].as_str().unwrap_or_default().to_string()))
    };
    let (img, meta) = tokio::join!(img, meta);
    let (title, channel) = meta.unwrap_or_default();
    (img, title, channel)
}

impl Cache {
    /// `None` enquanto carrega. Falha de rede devolve `Info` vazio (sem capa).
    pub fn get(&mut self, bk: &Arc<Backend>, ctx: &egui::Context, id: &str) -> Option<Info> {
        let entry = self.map.entry(id.to_string()).or_insert_with(|| Entry::Loading(bk.spawn(ctx, fetch(id.to_string()))));
        if let Entry::Loading(t) = entry {
            if let Some((img, title, channel)) = t.take() {
                let thumb = img.map(|i| ctx.load_texture(format!("yt:{id}"), i, TextureOptions::LINEAR));
                *entry = Entry::Ready(Info { thumb, title, channel });
            }
        }
        match entry {
            Entry::Ready(i) => Some(i.clone()),
            Entry::Loading(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::video_id;
    #[test]
    fn ids() {
        assert_eq!(video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("https://youtu.be/dQw4w9WgXcQ?si=x").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("youtube.com/shorts/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id("https://vimeo.com/123"), None);
    }
}

/// Só a miniatura (sem oEmbed), para listas longas como a da Prospecção.
#[derive(Default)]
pub struct ThumbCache {
    map: HashMap<String, Thumb>,
}

enum Thumb {
    Loading(Task<Option<ColorImage>>),
    Ready(Option<TextureHandle>),
}

async fn fetch_thumb(id: String) -> Option<ColorImage> {
    let http = flaxy_core::ai::http();
    let bytes = http.get(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

impl ThumbCache {
    /// `None` enquanto carrega ou se o download falhou.
    pub fn get(&mut self, bk: &Arc<Backend>, ctx: &egui::Context, id: &str) -> Option<TextureHandle> {
        let entry = self.map.entry(id.to_string()).or_insert_with(|| Thumb::Loading(bk.spawn(ctx, fetch_thumb(id.to_string()))));
        if let Thumb::Loading(t) = entry {
            if let Some(img) = t.take() {
                *entry = Thumb::Ready(img.map(|i| ctx.load_texture(format!("ytt:{id}"), i, TextureOptions::LINEAR)));
            }
        }
        match entry {
            Thumb::Ready(t) => t.clone(),
            Thumb::Loading(_) => None,
        }
    }
}
