//! Prospecção: vídeos mais vistos da última semana sobre um assunto, por tipo
//! (em alta, shorts, podcasts, gameplays, notícias), e notícias relacionadas.
//!
//! Com uma chave da YouTube Data API a busca usa a API oficial (datas e
//! durações exatas). Sem chave usa o yt-dlp empacotado, com o filtro do próprio
//! YouTube "esta semana, por visualizações". Notícias vêm do RSS do Google Notícias.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::backend::Res;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cat {
    #[default]
    Alta,
    Shorts,
    Podcasts,
    Gameplays,
    Noticias,
}

impl Cat {
    pub const ALL: [Cat; 5] = [Cat::Alta, Cat::Shorts, Cat::Podcasts, Cat::Gameplays, Cat::Noticias];

    pub fn label(self) -> &'static str {
        match self {
            Cat::Alta => "Em alta",
            Cat::Shorts => "Shorts",
            Cat::Podcasts => "Podcasts",
            Cat::Gameplays => "Gameplays",
            Cat::Noticias => "Notícias",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Cat::Alta | Cat::Shorts => "",
            Cat::Podcasts => " podcast",
            Cat::Gameplays => " gameplay",
            Cat::Noticias => " notícias",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Video {
    pub id: String,
    pub url: String,
    pub title: String,
    pub channel: String,
    pub views: u64,
    pub duration: Option<f64>,
    pub published: Option<DateTime<Utc>>,
}

impl Video {
    pub fn is_short(&self) -> bool {
        self.url.contains("/shorts/") || self.duration.is_some_and(|d| d > 0.0 && d <= 180.0)
    }
}

#[derive(Clone, Debug)]
pub struct Article {
    pub title: String,
    pub source: String,
    pub link: String,
    pub published: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Api,
    YtDlp,
}

pub struct Results {
    pub query: String,
    pub videos: Vec<(Cat, Vec<Video>)>,
    pub news: Vec<Article>,
    pub source: Source,
    pub warnings: Vec<String>,
    pub at: DateTime<chrono::Local>,
}

impl Results {
    pub fn list(&self, c: Cat) -> &[Video] {
        self.videos.iter().find(|(k, _)| *k == c).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }
}

/// Filtros de busca do YouTube (parâmetro `sp`): ordenar por visualizações,
/// enviados nesta semana; e o mesmo com duração acima de 20 minutos.
const SP_WEEK_VIEWS: &str = "CAMSAggD";
const SP_WEEK_VIEWS_LONG: &str = "CAMSBAgDGAI%3D";
/// Esta semana + duração abaixo de 4 minutos (ordem por relevância: o YouTube
/// não devolve nada combinando curtos com ordem por visualizações).
const SP_WEEK_SHORT: &str = "EgQIAxgB";

pub async fn search(query: String, key: Option<String>) -> Res<Results> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err("Digite um assunto".into());
    }
    let news = news(&q);
    let (videos, source, mut warnings) = match key.filter(|k| !k.trim().is_empty()) {
        Some(k) => {
            let (v, w) = by_api(&q, k.trim()).await?;
            (v, Source::Api, w)
        }
        None => {
            let (v, w) = by_ytdlp(&q).await?;
            (v, Source::YtDlp, w)
        }
    };
    let news = match news.await {
        Ok(n) => n,
        Err(e) => {
            warnings.push(format!("Notícias: {e}"));
            Vec::new()
        }
    };
    Ok(Results { query: q, videos, news, source, warnings, at: chrono::Local::now() })
}

/// Minúsculas sem acento, para comparar palavras.
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// A ordem por visualizações do YouTube puxa vídeos que só tocam no assunto.
/// Fica o que cita alguma palavra da busca no título ou no canal.
fn relevant(v: &Video, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let hay = fold(&format!("{} {}", v.title, v.channel));
    words.iter().any(|w| hay.contains(w.as_str()))
}

fn rank(mut v: Vec<Video>, words: &[String]) -> Vec<Video> {
    let mut seen = HashSet::new();
    v.retain(|x| seen.insert(x.id.clone()) && relevant(x, words));
    v.sort_by(|a, b| b.views.cmp(&a.views));
    v.truncate(30);
    v
}

/// Junta os resultados de cada categoria. Falha só se todas falharem.
fn collect(q: &str, parts: Vec<(Cat, Result<Vec<Video>, String>)>) -> Res<(Vec<(Cat, Vec<Video>)>, Vec<String>)> {
    let words: Vec<String> = fold(q).split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_string).collect();
    let mut warnings = Vec::new();
    let mut first_err = None;
    let mut out = Vec::new();
    let mut ok = 0;
    for (c, r) in parts {
        match r {
            Ok(v) => {
                ok += 1;
                out.push((c, rank(v, &words)));
            }
            Err(e) => {
                warnings.push(format!("{}: {e}", c.label()));
                first_err.get_or_insert(e);
                out.push((c, Vec::new()));
            }
        }
    }
    if ok == 0 {
        return Err(first_err.unwrap_or_else(|| "Nenhum resultado".into()));
    }
    Ok((out, warnings))
}

// ------------------------------------------------------------------ yt-dlp

async fn by_ytdlp(q: &str) -> Res<(Vec<(Cat, Vec<Video>)>, Vec<String>)> {
    let p = format!("{q}{}", Cat::Podcasts.suffix());
    let g = format!("{q}{}", Cat::Gameplays.suffix());
    let n = format!("{q}{}", Cat::Noticias.suffix());
    let s = format!("{q} shorts");
    let (alta, short, short2, pod, game, nws) = tokio::join!(
        ytdlp(q, SP_WEEK_VIEWS),
        ytdlp(q, SP_WEEK_SHORT),
        ytdlp(&s, SP_WEEK_VIEWS),
        ytdlp(&p, SP_WEEK_VIEWS_LONG),
        ytdlp(&g, SP_WEEK_VIEWS),
        ytdlp(&n, SP_WEEK_VIEWS),
    );
    // Shorts: o YouTube não filtra curtos por visualizações, então juntamos os
    // curtos de todas as buscas.
    let shorts: Result<Vec<Video>, String> = {
        let pool: Vec<Video> = [&alta, &short, &short2, &pod, &game, &nws]
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .flatten()
            .filter(|v| v.is_short())
            .cloned()
            .collect();
        match (&short, &short2) {
            (Err(e), Err(_)) if pool.is_empty() => Err(e.clone()),
            _ => Ok(pool),
        }
    };
    collect(q, vec![(Cat::Alta, alta), (Cat::Shorts, shorts), (Cat::Podcasts, pod), (Cat::Gameplays, game), (Cat::Noticias, nws)])
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

async fn ytdlp(q: &str, sp: &str) -> Result<Vec<Video>, String> {
    let bin = flaxy_core::clips::ytdlp().ok_or("yt-dlp não encontrado. Reinstale o FLAXY")?;
    let url = format!("https://www.youtube.com/results?search_query={}&sp={sp}", enc(q));
    let run = flaxy_core::media::command(bin)
        .args(["--flat-playlist", "-J", "--no-warnings", "--encoding", "utf-8", "--playlist-end", "40", "--"])
        .arg(&url)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .map_err(|_| "o YouTube demorou demais para responder".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| l.contains("ERROR")).unwrap_or("falha no yt-dlp");
        return Err(line.trim_start_matches("ERROR:").trim().to_string());
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("resposta inválida do yt-dlp: {e}"))?;
    Ok(v["entries"]
        .as_array()
        .map(|a| a.iter().filter_map(from_ytdlp).collect())
        .unwrap_or_default())
}

fn from_ytdlp(e: &Value) -> Option<Video> {
    let id = e["id"].as_str()?.to_string();
    if id.len() != 11 {
        return None; // canais e playlists no meio da busca
    }
    let url = e["url"].as_str().map(str::to_string).unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
    Some(Video {
        url,
        title: e["title"].as_str().unwrap_or_default().to_string(),
        channel: e["channel"].as_str().or(e["uploader"].as_str()).unwrap_or_default().to_string(),
        views: e["view_count"].as_u64().unwrap_or(0),
        duration: e["duration"].as_f64(),
        published: e["timestamp"].as_i64().and_then(|t| DateTime::from_timestamp(t, 0)),
        id,
    })
}

// ------------------------------------------------------------------ YouTube Data API

async fn by_api(q: &str, key: &str) -> Res<(Vec<(Cat, Vec<Video>)>, Vec<String>)> {
    let after = (Utc::now() - chrono::Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let p = format!("{q}{}", Cat::Podcasts.suffix());
    let g = format!("{q}{}", Cat::Gameplays.suffix());
    let n = format!("{q}{}", Cat::Noticias.suffix());
    let (alta, short, pod, game, nws) = tokio::join!(
        api(key, q, None, &after),
        api(key, q, Some("short"), &after),
        api(key, &p, Some("long"), &after),
        api(key, &g, None, &after),
        api(key, &n, None, &after),
    );
    let short = short.map(|v| v.into_iter().filter(Video::is_short).collect());
    collect(q, vec![(Cat::Alta, alta), (Cat::Shorts, short), (Cat::Podcasts, pod), (Cat::Gameplays, game), (Cat::Noticias, nws)])
}

async fn api_json(req: reqwest::RequestBuilder) -> Result<Value, String> {
    let r = req.timeout(Duration::from_secs(30)).send().await.map_err(|e| format!("sem conexão com o YouTube ({e})"))?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    if !ok {
        let msg = v["error"]["message"].as_str().unwrap_or("erro da API do YouTube");
        return Err(strip_html(msg));
    }
    Ok(v)
}

async fn api(key: &str, q: &str, duration: Option<&str>, after: &str) -> Result<Vec<Video>, String> {
    let http = flaxy_core::ai::http();
    let mut params: Vec<(&str, &str)> = vec![
        ("part", "id"),
        ("type", "video"),
        ("order", "viewCount"),
        ("maxResults", "30"),
        ("relevanceLanguage", "pt"),
        ("q", q),
        ("publishedAfter", after),
        ("key", key),
    ];
    if let Some(d) = duration {
        params.push(("videoDuration", d));
    }
    let s = api_json(http.get("https://www.googleapis.com/youtube/v3/search").query(&params)).await?;
    let ids: Vec<&str> = s["items"].as_array().map(|a| a.iter().filter_map(|i| i["id"]["videoId"].as_str()).collect()).unwrap_or_default();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let joined = ids.join(",");
    let d = api_json(http.get("https://www.googleapis.com/youtube/v3/videos").query(&[("part", "snippet,statistics,contentDetails"), ("id", joined.as_str()), ("key", key)])).await?;
    Ok(d["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    let id = i["id"].as_str()?.to_string();
                    Some(Video {
                        url: format!("https://www.youtube.com/watch?v={id}"),
                        title: i["snippet"]["title"].as_str().unwrap_or_default().to_string(),
                        channel: i["snippet"]["channelTitle"].as_str().unwrap_or_default().to_string(),
                        views: i["statistics"]["viewCount"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
                        duration: i["contentDetails"]["duration"].as_str().and_then(iso_duration),
                        published: i["snippet"]["publishedAt"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc)),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// "PT1H2M3S" -> 3723 segundos.
fn iso_duration(s: &str) -> Option<f64> {
    let s = s.strip_prefix('P')?;
    let (days, time) = s.split_once('T').unwrap_or((s, ""));
    let mut total = 0.0;
    let mut num = String::new();
    for c in days.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else if c == 'D' {
            total += num.parse::<f64>().ok()? * 86400.0;
            num.clear();
        }
    }
    for c in time.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let n: f64 = num.parse().ok()?;
            num.clear();
            total += n * match c {
                'H' => 3600.0,
                'M' => 60.0,
                'S' => 1.0,
                _ => return None,
            };
        }
    }
    Some(total)
}

// ------------------------------------------------------------------ Google Notícias

async fn news(q: &str) -> Result<Vec<Article>, String> {
    let http = flaxy_core::ai::http();
    let body = http
        .get("https://news.google.com/rss/search")
        .query(&[("q", format!("{q} when:7d").as_str()), ("hl", "pt-BR"), ("gl", "BR"), ("ceid", "BR:pt-419")])
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "sem conexão com o Google Notícias".to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_rss(&body))
}

fn parse_rss(body: &str) -> Vec<Article> {
    body.split("<item>")
        .skip(1)
        .filter_map(|item| {
            let source = tag(item, "source").unwrap_or_default();
            let mut title = tag(item, "title")?;
            if !source.is_empty() {
                if let Some(t) = title.strip_suffix(&format!(" - {source}")) {
                    title = t.to_string();
                }
            }
            Some(Article {
                title,
                source,
                link: tag(item, "link")?,
                published: tag(item, "pubDate").and_then(|d| DateTime::parse_from_rfc2822(&d).ok()).map(|d| d.with_timezone(&Utc)),
            })
        })
        .take(24)
        .collect()
}

/// Conteúdo de `<name ...>…</name>`, sem CDATA e com entidades decodificadas.
fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let mut from = 0;
    let start = loop {
        let i = from + s[from..].find(&open)?;
        let next = s[i + open.len()..].chars().next()?;
        if next == '>' || next == ' ' {
            break i;
        }
        from = i + open.len();
    };
    let body_start = start + s[start..].find('>')? + 1;
    let end = body_start + s[body_start..].find(&format!("</{name}>"))?;
    let raw = s[body_start..end].trim();
    let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(raw);
    Some(entities(raw))
}

fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';').filter(|j| *j <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    entities(&out)
}

// ------------------------------------------------------------------ formatos

/// 5.384.585 -> "5,4 mi"; 83.200 -> "83 mil".
pub fn compact(n: u64) -> String {
    let f = n as f64;
    if n >= 1_000_000_000 {
        format!("{:.1} bi", f / 1e9).replace('.', ",")
    } else if n >= 10_000_000 {
        format!("{:.0} mi", f / 1e6)
    } else if n >= 1_000_000 {
        format!("{:.1} mi", f / 1e6).replace('.', ",")
    } else if n >= 1_000 {
        format!("{:.0} mil", f / 1e3)
    } else {
        n.to_string()
    }
}

/// "há 3 h", "há 2 dias".
pub fn ago(t: DateTime<Utc>) -> String {
    let s = (Utc::now() - t).num_seconds().max(0);
    match s {
        0..=3599 => format!("há {} min", (s / 60).max(1)),
        3600..=86399 => format!("há {} h", s / 3600),
        86400..=172799 => "ontem".into(),
        _ => format!("há {} dias", s / 86400),
    }
}

pub fn duration(s: f64) -> String {
    let s = s.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso() {
        assert_eq!(iso_duration("PT1H2M3S"), Some(3723.0));
        assert_eq!(iso_duration("PT45S"), Some(45.0));
        assert_eq!(iso_duration("P1DT1M"), Some(86460.0));
    }

    #[test]
    fn rss() {
        let x = r#"<rss><channel><title>x</title><item><title>Copa: Brasil vence &amp; avança - g1</title><link>https://n/1</link><pubDate>Sun, 15 Nov 2026 00:42:44 GMT</pubDate><source url="https://g1">g1</source></item></channel></rss>"#;
        let a = parse_rss(x);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].title, "Copa: Brasil vence & avança");
        assert_eq!(a[0].source, "g1");
        assert!(a[0].published.is_some());
    }

    #[test]
    fn formats() {
        assert_eq!(compact(5_384_585), "5,4 mi");
        assert_eq!(compact(83_200), "83 mil");
        assert_eq!(compact(12_300_000), "12 mi");
        assert_eq!(enc("café & cia"), "caf%C3%A9+%26+cia");
    }
}

#[cfg(test)]
mod live {
    /// Busca real (rede + yt-dlp): `cargo test -p flaxy-app live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn search_live() {
        let r = super::search("minecraft".into(), None).await.expect("busca");
        for (c, v) in &r.videos {
            println!("{:?}: {} vídeos, topo {:?}", c, v.len(), v.first().map(|x| (x.views, x.duration, &x.title)));
        }
        println!("notícias: {} {:?}", r.news.len(), r.news.first().map(|a| (&a.source, &a.title)));
        println!("avisos: {:?}", r.warnings);
        assert!(!r.list(super::Cat::Alta).is_empty());
    }
}
