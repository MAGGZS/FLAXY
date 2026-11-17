//! Prospecção: vídeos mais vistos da última semana sobre um assunto, por tipo
//! (em alta, shorts, podcasts, gameplays, notícias), e notícias relacionadas.
//!
//! Com uma chave da YouTube Data API a busca usa a API oficial (datas e
//! durações exatas). Sem chave usa o yt-dlp empacotado, com o filtro do próprio
//! YouTube "esta semana, por visualizações". Notícias vêm do RSS do Google Notícias.

use std::collections::{HashMap, HashSet};
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
    /// Id do canal (UC…), para descobrir o país do canal.
    pub channel_id: String,
    pub views: u64,
    pub duration: Option<f64>,
    pub published: Option<DateTime<Utc>>,
    /// Trecho da descrição (busca sem chave) ou descrição (API / yt-dlp).
    pub description: String,
    /// Veio de uma busca por relevância: o YouTube já decidiu que é do
    /// assunto (entende tema, não só palavra), então não passa pelo filtro.
    pub trusted: bool,
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

/// País da busca. Com a chave da API vira `regionCode`; sem chave a busca vai
/// direto ao YouTube com o país (`gl`). Em nenhum dos dois o YouTube filtra de
/// verdade (vídeo gigante de fora ganha em qualquer país), por isso o filtro
/// "só canais do país" olha o país que cada canal informa no YouTube.
#[derive(Debug, PartialEq, Eq)]
pub struct Region {
    pub code: &'static str,
    /// "do Brasil", "dos Estados Unidos": para o texto "Só canais …".
    pub of: &'static str,
    /// Nome do país em inglês, como aparece no "Sobre" do canal.
    country: &'static str,
    pub name: &'static str,
    /// Idioma da interface do YouTube na busca: os títulos vêm no original
    /// para quem é do país (o YouTube traduz título de fora para esse idioma).
    hl: &'static str,
    /// Idioma para a API (`relevanceLanguage`, ISO 639-1).
    lang: &'static str,
    /// Google Notícias: hl, gl, ceid.
    news: (&'static str, &'static str, &'static str),
}

pub const REGIONS: [Region; 8] = [
    Region { code: "BR", of: "do Brasil", country: "Brazil", name: "Brasil", hl: "pt", lang: "pt", news: ("pt-BR", "BR", "BR:pt-419") },
    Region { code: "PT", of: "de Portugal", country: "Portugal", name: "Portugal", hl: "pt-PT", lang: "pt", news: ("pt-PT", "PT", "PT:pt-150") },
    Region { code: "US", of: "dos Estados Unidos", country: "United States", name: "Estados Unidos", hl: "en", lang: "en", news: ("en-US", "US", "US:en") },
    Region { code: "GB", of: "do Reino Unido", country: "United Kingdom", name: "Reino Unido", hl: "en-GB", lang: "en", news: ("en-GB", "GB", "GB:en") },
    Region { code: "MX", of: "do México", country: "Mexico", name: "México", hl: "es-419", lang: "es", news: ("es-419", "MX", "MX:es-419") },
    Region { code: "AR", of: "da Argentina", country: "Argentina", name: "Argentina", hl: "es-419", lang: "es", news: ("es-419", "AR", "AR:es-419") },
    Region { code: "ES", of: "da Espanha", country: "Spain", name: "Espanha", hl: "es", lang: "es", news: ("es", "ES", "ES:es") },
    Region { code: "JP", of: "do Japão", country: "Japan", name: "Japão", hl: "ja", lang: "ja", news: ("ja", "JP", "JP:ja") },
];

pub fn region(code: &str) -> &'static Region {
    REGIONS.iter().find(|r| r.code == code).unwrap_or(&REGIONS[0])
}

/// Janela de publicação. São os mesmos períodos do filtro "Data de envio" do
/// YouTube, para a busca com e sem chave trazer a mesma coisa.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Period {
    Hora,
    Hoje,
    #[default]
    Semana,
    Mes,
    Ano,
}

impl Period {
    pub const ALL: [Period; 5] = [Period::Hora, Period::Hoje, Period::Semana, Period::Mes, Period::Ano];

    pub fn id(self) -> &'static str {
        match self {
            Period::Hora => "hora",
            Period::Hoje => "hoje",
            Period::Semana => "semana",
            Period::Mes => "mes",
            Period::Ano => "ano",
        }
    }

    pub fn from_id(s: &str) -> Period {
        Period::ALL.into_iter().find(|p| p.id() == s).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Period::Hora => "Última hora",
            Period::Hoje => "Hoje",
            Period::Semana => "7 dias",
            Period::Mes => "30 dias",
            Period::Ano => "Este ano",
        }
    }

    /// Para frases como "Nada desta categoria nos últimos 7 dias".
    pub fn phrase(self) -> &'static str {
        match self {
            Period::Hora => "na última hora",
            Period::Hoje => "nas últimas 24 horas",
            Period::Semana => "nos últimos 7 dias",
            Period::Mes => "nos últimos 30 dias",
            Period::Ano => "no último ano",
        }
    }

    /// Código do filtro "Data de envio" dentro do parâmetro `sp` do YouTube.
    fn upload(self) -> u8 {
        match self {
            Period::Hora => 1,
            Period::Hoje => 2,
            Period::Semana => 3,
            Period::Mes => 4,
            Period::Ano => 5,
        }
    }

    fn span(self) -> chrono::Duration {
        match self {
            Period::Hora => chrono::Duration::hours(1),
            Period::Hoje => chrono::Duration::hours(24),
            Period::Semana => chrono::Duration::days(7),
            Period::Mes => chrono::Duration::days(30),
            Period::Ano => chrono::Duration::days(365),
        }
    }

    /// Operador `when:` do Google Notícias.
    fn news_when(self) -> &'static str {
        match self {
            Period::Hora => "1h",
            Period::Hoje => "1d",
            Period::Semana => "7d",
            Period::Mes => "30d",
            Period::Ano => "1y",
        }
    }
}

pub struct Results {
    pub query: String,
    pub region: &'static Region,
    pub period: Period,
    /// Só canais do país nesta busca.
    pub local_only: bool,
    /// Temas próximos que a IA sugeriu e que também foram pesquisados.
    pub related: Vec<String>,
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

/// Parâmetro `sp` da busca do YouTube: um protobuf em base64. Campo 1 = ordem
/// (3 = visualizações); campo 2 = filtros, com 1 = data de envio e 3 = duração
/// (1 = abaixo de 4 min, 2 = acima de 20 min).
/// Ex.: semana por visualizações = 08 03 12 02 08 03 = "CAMSAggD".
fn sp(by_views: bool, period: Period, duration: Option<u8>) -> String {
    let mut f = vec![0x08, period.upload()];
    if let Some(d) = duration {
        f.extend([0x18, d]);
    }
    let mut b = Vec::new();
    if by_views {
        b.extend([0x08, 0x03]);
    }
    b.extend([0x12, f.len() as u8]);
    b.extend(f);
    base64(&b)
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

pub async fn search(query: String, key: Option<String>, region: &'static Region, period: Period, local_only: bool, db: Option<sqlx::SqlitePool>) -> Res<Results> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err("Digite um assunto".into());
    }
    let news = news(&q, region, period);
    let key = key.filter(|k| !k.trim().is_empty());
    let main = async {
        match &key {
            Some(k) => (by_api(&q, k.trim(), region, period).await, Source::Api),
            None => (by_web(&q, region, period).await, Source::YtDlp),
        }
    };
    let ((mut parts, source), related) = tokio::join!(main, related_terms(db, &q, region));
    // Temas próximos (ciência -> biologia, física…): buscas extras por
    // relevância que entram em "Em alta".
    if !related.is_empty() {
        let extra = by_terms(&related, key.as_deref(), region, period).await;
        if let Some((_, alta)) = parts.iter_mut().find(|(c, _)| *c == Cat::Alta) {
            let cur = std::mem::replace(alta, Ok(Vec::new()));
            *alta = merge(cur, Ok(extra));
        }
    }
    // Só canais do país: o país vem do "Sobre" de cada canal. Canal que não
    // informa país fica de fora (o título não serve: o YouTube traduz).
    let countries = if local_only { channel_countries(&parts).await } else { HashMap::new() };
    let keep = |v: &Video| {
        if !local_only {
            return true;
        }
        matches!(countries.get(&v.channel_id), Some(Some(c)) if c == region.country)
    };
    let (videos, mut warnings) = collect(&q, &related, &keep, parts)?;
    let news = match news.await {
        Ok(n) => n,
        Err(e) => {
            warnings.push(format!("Notícias: {e}"));
            Vec::new()
        }
    };
    Ok(Results { query: q, region, period, local_only, related, videos, news, source, warnings, at: chrono::Local::now() })
}

/// Até 3 temas próximos do assunto, sugeridos pelo LLM da aba IA (o mesmo
/// da página Clipes). Sem LLM configurado ou com erro, nenhum: a busca segue
/// só com o assunto.
async fn related_terms(db: Option<sqlx::SqlitePool>, q: &str, region: &Region) -> Vec<String> {
    let Some(db) = db else { return Vec::new() };
    let Ok(settings) = flaxy_core::ai::load(&db).await else { return Vec::new() };
    if settings.llm_provider.is_empty() {
        return Vec::new();
    }
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "terms": { "type": "array", "items": { "type": "string" } } },
        "required": ["terms"],
        "additionalProperties": false
    });
    let system = "Você ajuda a pesquisar vídeos no YouTube. Responda só com o JSON pedido.";
    let user = format!(
        "Assunto pesquisado: \"{q}\". Idioma dos vídeos: {}.\n\
         Sugira 3 termos de busca curtos (1 a 3 palavras, nesse idioma) de temas próximos ou subtemas \
         que quem se interessa por esse assunto também assiste. Ex.: \"ciência\" -> biologia, física, astronomia. \
         Não repita o assunto.",
        region.lang
    );
    let call = flaxy_core::llm::json_call(&settings, system, &user, &schema);
    match tokio::time::timeout(Duration::from_secs(25), call).await {
        Ok(Ok(v)) => {
            let qf = fold(q);
            let mut out: Vec<String> = Vec::new();
            for t in v["terms"].as_array().into_iter().flatten().filter_map(|t| t.as_str()) {
                let t = t.trim().trim_matches('"').to_string();
                if !t.is_empty() && t.chars().count() <= 40 && fold(&t) != qf && !out.iter().any(|o| fold(o) == fold(&t)) {
                    out.push(t);
                }
            }
            out.truncate(3);
            out
        }
        Ok(Err(e)) => {
            tracing::warn!("prospecção: termos relacionados: {e}");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("prospecção: termos relacionados demoraram demais");
            Vec::new()
        }
    }
}

/// Busca por relevância de cada termo relacionado, tudo marcado como confiável.
async fn by_terms(terms: &[String], key: Option<&str>, region: &Region, period: Period) -> Vec<Video> {
    let mut set = Vec::new();
    for t in terms {
        set.push(async move {
            match key {
                Some(k) => {
                    let after = (Utc::now() - period.span()).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    api(k, t, None, &after, region).await
                }
                None => web(t, &sp(false, period, None), region.code, region.hl).await,
            }
        });
    }
    let mut out = Vec::new();
    for r in futures_join(set).await {
        if let Ok(v) = trust(r) {
            out.extend(v);
        }
    }
    out
}

/// Roda várias buscas juntas (sem depender do crate futures).
async fn futures_join<F: std::future::Future>(fs: Vec<F>) -> Vec<F::Output> {
    let mut out = Vec::with_capacity(fs.len());
    let mut fs: Vec<std::pin::Pin<Box<F>>> = fs.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = (0..fs.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, f) in fs.iter_mut().enumerate() {
            if done[i].is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => done[i] = Some(v),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending { std::task::Poll::Pending } else { std::task::Poll::Ready(()) }
    })
    .await;
    out.extend(done.into_iter().flatten());
    out
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
    if words.is_empty() || v.trusted {
        return true;
    }
    let hay = fold(&format!("{} {} {}", v.title, v.channel, v.description));
    words.iter().any(|w| hay.contains(w.as_str()))
}

/// Marca os vídeos de uma busca por relevância (ver `Video::trusted`).
fn trust(r: Result<Vec<Video>, String>) -> Result<Vec<Video>, String> {
    r.map(|v| v.into_iter().map(|x| Video { trusted: true, ..x }).collect())
}

fn rank(mut v: Vec<Video>, words: &[String], keep: &dyn Fn(&Video) -> bool) -> Vec<Video> {
    // Repetido entre buscas: fica a cópia "confiável", se houver.
    v.sort_by_key(|x| !x.trusted);
    let mut seen = HashSet::new();
    v.retain(|x| seen.insert(x.id.clone()) && relevant(x, words) && keep(x));
    v.sort_by(|a, b| b.views.cmp(&a.views));
    v.truncate(30);
    v
}

/// Junta os resultados de cada categoria. Falha só se todas falharem.
type Parts = Vec<(Cat, Result<Vec<Video>, String>)>;

fn collect(q: &str, related: &[String], keep: &dyn Fn(&Video) -> bool, parts: Parts) -> Res<(Vec<(Cat, Vec<Video>)>, Vec<String>)> {
    let words: Vec<String> = std::iter::once(q)
        .chain(related.iter().map(String::as_str))
        .flat_map(|t| fold(t).split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_string).collect::<Vec<_>>())
        .collect();
    let mut warnings = Vec::new();
    let mut first_err = None;
    let mut out = Vec::new();
    let mut ok = 0;
    for (c, r) in parts {
        match r {
            Ok(v) => {
                ok += 1;
                out.push((c, rank(v, &words, keep)));
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

// ------------------------------------------------------------------ YouTube sem chave

/// Junta duas buscas da mesma categoria; falha só se as duas falharem.
fn merge(a: Result<Vec<Video>, String>, b: Result<Vec<Video>, String>) -> Result<Vec<Video>, String> {
    match (a, b) {
        (Ok(mut x), Ok(y)) => {
            x.extend(y);
            Ok(x)
        }
        (Ok(x), Err(_)) | (Err(_), Ok(x)) => Ok(x),
        (Err(e), Err(_)) => Err(e),
    }
}

async fn by_web(q: &str, region: &Region, period: Period) -> Parts {
    let p = format!("{q}{}", Cat::Podcasts.suffix());
    let g = format!("{q}{}", Cat::Gameplays.suffix());
    let n = format!("{q}{}", Cat::Noticias.suffix());
    let s = format!("{q} shorts");
    let views = sp(true, period, None);
    let long = sp(true, period, Some(2));
    // Ordem por relevância traz mais vídeos do país do que a por visualizações
    // (que puxa os gigantes de fora); as duas são somadas e reordenadas aqui.
    let rel = sp(false, period, None);
    let long_rel = sp(false, period, Some(2));
    // Curtos com ordem por relevância: o YouTube não devolve nada combinando
    // curtos com ordem por visualizações.
    let short_sp = sp(false, period, Some(1));
    let (gl, hl) = (region.code, region.hl);
    let (alta, alta2, short, short2, pod, pod2, game, game2, nws, nws2) = tokio::join!(
        web(q, &views, gl, hl),
        web(q, &rel, gl, hl),
        web(q, &short_sp, gl, hl),
        web(&s, &views, gl, hl),
        web(&p, &long, gl, hl),
        web(&p, &long_rel, gl, hl),
        web(&g, &views, gl, hl),
        web(&g, &rel, gl, hl),
        web(&n, &views, gl, hl),
        web(&n, &rel, gl, hl),
    );
    let (short, alta2, pod2, game2, nws2) = (trust(short), trust(alta2), trust(pod2), trust(game2), trust(nws2));
    let (alta, pod, game, nws) = (merge(alta, alta2), merge(pod, pod2), merge(game, game2), merge(nws, nws2));
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
    vec![(Cat::Alta, alta), (Cat::Shorts, shorts), (Cat::Podcasts, pod), (Cat::Gameplays, game), (Cat::Noticias, nws)]
}

const WEB_CLIENT: &str = "2.20251101.00.00";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// País de cada canal já consultado nesta sessão do app (None = o canal não informa).
static COUNTRIES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Option<String>>>> = std::sync::OnceLock::new();

/// País dos canais de todos os vídeos, 16 consultas por vez. Falha de rede
/// não entra no cache (tenta de novo na próxima busca).
async fn channel_countries(parts: &Parts) -> HashMap<String, Option<String>> {
    let cache = COUNTRIES.get_or_init(Default::default);
    let ids: HashSet<String> = parts.iter().filter_map(|(_, r)| r.as_ref().ok()).flatten().map(|v| v.channel_id.clone()).filter(|c| !c.is_empty()).collect();
    let missing: Vec<String> = {
        let c = cache.lock().unwrap();
        ids.iter().filter(|id| !c.contains_key(*id)).cloned().collect()
    };
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    let mut set = tokio::task::JoinSet::new();
    for id in missing {
        let sem = sem.clone();
        set.spawn(async move {
            let _p = sem.acquire_owned().await;
            let c = channel_country(&id).await;
            (id, c)
        });
    }
    while let Some(r) = set.join_next().await {
        match r {
            Ok((id, Ok(c))) => {
                cache.lock().unwrap().insert(id, c);
            }
            Ok((id, Err(e))) => tracing::warn!("prospecção: país do canal {id}: {e}"),
            Err(_) => {}
        }
    }
    let c = cache.lock().unwrap();
    ids.into_iter().filter_map(|id| c.get(&id).cloned().map(|v| (id, v))).collect()
}

/// Token do painel "Sobre" de um canal (o mesmo que a página do canal manda
/// ao clicar em "mais"): protobuf {80226972: {2: id, 3: params}} em base64.
fn about_token(channel_id: &str) -> String {
    let uuid = b"00000000-0000-0000-0000-000000000000";
    let mut inner = vec![0xf2, 0x06, uuid.len() as u8 + 7, 0x1a, uuid.len() as u8 + 5, 0x9a, 0x01, uuid.len() as u8 + 2, 0x0a, uuid.len() as u8];
    inner.extend_from_slice(uuid);
    let params = b64url(&inner).replace('=', "%3D");
    let mut msg = vec![0x12, channel_id.len() as u8];
    msg.extend_from_slice(channel_id.as_bytes());
    msg.extend([0x1a, params.len() as u8]);
    msg.extend_from_slice(params.as_bytes());
    let mut out = vec![0xe2, 0xa9, 0x85, 0xb2, 0x02, msg.len() as u8];
    out.extend(msg);
    b64url(&out)
}

fn b64url(b: &[u8]) -> String {
    base64(b).replace('+', "-").replace('/', "_")
}

async fn channel_country(channel_id: &str) -> Result<Option<String>, String> {
    if channel_id.len() > 64 {
        return Ok(None);
    }
    let body = serde_json::json!({
        "context": { "client": { "clientName": "WEB", "clientVersion": WEB_CLIENT, "hl": "en", "gl": "US" } },
        "continuation": about_token(channel_id),
    });
    let v: Value = flaxy_core::ai::http()
        .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
        .header("User-Agent", UA)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(find_str(&v, "country").filter(|c| !c.is_empty()))
}

fn find_str(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(m) => m.get(key).and_then(Value::as_str).map(str::to_string).or_else(|| m.values().find_map(|x| find_str(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_str(x, key)),
        _ => None,
    }
}

/// Uma busca: direto no YouTube com o país; se falhar, pelo yt-dlp.
async fn web(q: &str, sp: &str, gl: &str, hl: &str) -> Result<Vec<Video>, String> {
    match innertube(q, sp, gl, hl).await {
        Ok(v) => Ok(v),
        Err(e) => {
            tracing::warn!("prospecção: busca direta falhou ({e}), usando yt-dlp");
            ytdlp(q, sp, hl).await
        }
    }
}

/// Busca pela API interna do site do YouTube (a mesma que a página usa), com
/// o país em `gl` e o idioma em `hl` (títulos dos canais do país no original).
/// Lê duas páginas (~40 vídeos).
async fn innertube(q: &str, sp: &str, gl: &str, hl: &str) -> Result<Vec<Video>, String> {
    let http = flaxy_core::ai::http();
    let ctx = serde_json::json!({ "client": { "clientName": "WEB", "clientVersion": WEB_CLIENT, "hl": hl, "gl": gl } });
    let mut body = serde_json::json!({ "context": ctx, "query": q, "params": sp });
    let mut out = Vec::new();
    for _ in 0..2 {
        let v: Value = http
            .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
            .header("User-Agent", UA)
            .json(&body)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("sem conexão com o YouTube ({e})"))?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| format!("resposta inválida do YouTube: {e}"))?;
        let mut token = None;
        walk(&v, &mut out, &mut token);
        match token {
            Some(t) => body = serde_json::json!({ "context": ctx, "continuation": t }),
            None => break,
        }
    }
    if out.is_empty() {
        return Err("o YouTube não devolveu vídeos".into());
    }
    Ok(out)
}

/// Procura `videoRenderer` (vídeos) e o token da próxima página em qualquer
/// profundidade da resposta.
fn walk(v: &Value, out: &mut Vec<Video>, token: &mut Option<String>) {
    match v {
        Value::Object(m) => {
            if let Some(r) = m.get("videoRenderer") {
                if let Some(x) = from_renderer(r) {
                    out.push(x);
                }
                return;
            }
            if let Some(t) = m.get("continuationCommand").and_then(|c| c["token"].as_str()) {
                token.get_or_insert_with(|| t.to_string());
            }
            for x in m.values() {
                walk(x, out, token);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk(x, out, token);
            }
        }
        _ => {}
    }
}

fn runs(v: &Value) -> String {
    v["simpleText"].as_str().map(str::to_string).unwrap_or_else(|| v["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).collect()).unwrap_or_default())
}

fn from_renderer(r: &Value) -> Option<Video> {
    let id = r["videoId"].as_str()?.to_string();
    let views = runs(&r["viewCountText"]).chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
    let duration = r["lengthText"]["simpleText"].as_str().and_then(clock);
    let path = r["navigationEndpoint"]["commandMetadata"]["webCommandMetadata"]["url"].as_str().unwrap_or_default();
    let url = if path.starts_with("/shorts/") { format!("https://www.youtube.com{path}") } else { format!("https://www.youtube.com/watch?v={id}") };
    Some(Video {
        url,
        title: runs(&r["title"]),
        channel: runs(&r["ownerText"]),
        channel_id: r["ownerText"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str().unwrap_or_default().to_string(),
        views,
        duration,
        published: r["publishedTimeText"]["simpleText"].as_str().and_then(ago_en),
        description: r["detailedMetadataSnippets"].as_array().map(|a| a.iter().map(|x| runs(&x["snippetText"])).collect::<Vec<_>>().join(" ")).unwrap_or_default(),
        trusted: false,
        id,
    })
}

/// "1:02:03" -> 3723 segundos.
fn clock(s: &str) -> Option<f64> {
    s.split(':').try_fold(0.0, |acc, p| p.trim().parse::<f64>().ok().map(|n| acc * 60.0 + n))
}

/// "3 days ago" / "há 3 dias" -> instante aproximado.
fn ago_en(s: &str) -> Option<DateTime<Utc>> {
    let low = s.to_lowercase();
    let w: Vec<&str> = low.split_whitespace().collect();
    let i = w.iter().position(|x| x.parse::<i64>().is_ok())?;
    let n: i64 = w[i].parse().ok()?;
    let unit = w.get(i + 1)?;
    let secs = [
        (["second", "segundo"], 1),
        (["minute", "minuto"], 60),
        (["hour", "hora"], 3600),
        (["day", "dia"], 86400),
        (["week", "semana"], 7 * 86400),
        (["month", "mes"], 30 * 86400),
        (["year", "ano"], 365 * 86400),
    ]
    .iter()
    .find(|(names, _)| names.iter().any(|n| fold(unit).starts_with(n)))?
    .1;
    Some(Utc::now() - chrono::Duration::seconds(n * secs))
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

async fn ytdlp(q: &str, sp: &str, hl: &str) -> Result<Vec<Video>, String> {
    let bin = flaxy_core::clips::ytdlp().ok_or("yt-dlp não encontrado. Reinstale o FLAXY")?;
    let url = format!("https://www.youtube.com/results?search_query={}&sp={}", enc(q), enc(sp));
    let lang = format!("youtube:lang={hl}");
    let run = flaxy_core::media::command(bin)
        .args(flaxy_core::clips::ytdlp_args())
        .args(["--flat-playlist", "-J", "--no-warnings", "--encoding", "utf-8", "--playlist-end", "40", "--extractor-args", &lang, "--"])
        .arg(&url)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .map_err(|_| "o YouTube demorou demais para responder".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if let Some(h) = flaxy_core::clips::ytdlp_hint(&err) {
            return Err(h.to_string());
        }
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
        channel_id: e["channel_id"].as_str().unwrap_or_default().to_string(),
        views: e["view_count"].as_u64().unwrap_or(0),
        duration: e["duration"].as_f64(),
        published: e["timestamp"].as_i64().and_then(|t| DateTime::from_timestamp(t, 0)),
        description: e["description"].as_str().unwrap_or_default().to_string(),
        trusted: false,
        id,
    })
}

// ------------------------------------------------------------------ YouTube Data API

async fn by_api(q: &str, key: &str, region: &Region, period: Period) -> Parts {
    let after = (Utc::now() - period.span()).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let p = format!("{q}{}", Cat::Podcasts.suffix());
    let g = format!("{q}{}", Cat::Gameplays.suffix());
    let n = format!("{q}{}", Cat::Noticias.suffix());
    let r = region;
    let (alta, short, pod, game, nws) = tokio::join!(
        api(key, q, None, &after, r),
        api(key, q, Some("short"), &after, r),
        api(key, &p, Some("long"), &after, r),
        api(key, &g, None, &after, r),
        api(key, &n, None, &after, r),
    );
    let short = short.map(|v| v.into_iter().filter(Video::is_short).collect());
    vec![(Cat::Alta, alta), (Cat::Shorts, short), (Cat::Podcasts, pod), (Cat::Gameplays, game), (Cat::Noticias, nws)]
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

async fn api(key: &str, q: &str, duration: Option<&str>, after: &str, region: &Region) -> Result<Vec<Video>, String> {
    let http = flaxy_core::ai::http();
    let mut params: Vec<(&str, &str)> = vec![
        ("part", "id"),
        ("type", "video"),
        ("order", "viewCount"),
        ("maxResults", "30"),
        ("regionCode", region.code),
        ("relevanceLanguage", region.lang),
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
                        channel_id: i["snippet"]["channelId"].as_str().unwrap_or_default().to_string(),
                        views: i["statistics"]["viewCount"].as_str().and_then(|v| v.parse().ok()).unwrap_or(0),
                        duration: i["contentDetails"]["duration"].as_str().and_then(iso_duration),
                        published: i["snippet"]["publishedAt"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc)),
                        description: i["snippet"]["description"].as_str().unwrap_or_default().to_string(),
                        trusted: false,
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

async fn news(q: &str, region: &Region, period: Period) -> Result<Vec<Article>, String> {
    let http = flaxy_core::ai::http();
    let (hl, gl, ceid) = region.news;
    let body = http
        .get("https://news.google.com/rss/search")
        .query(&[("q", format!("{q} when:{}", period.news_when()).as_str()), ("hl", hl), ("gl", gl), ("ceid", ceid)])
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

    #[test]
    fn sp_codes() {
        // Os valores fixos que o app usava antes, copiados do próprio YouTube.
        assert_eq!(sp(true, Period::Semana, None), "CAMSAggD");
        assert_eq!(sp(true, Period::Semana, Some(2)), "CAMSBAgDGAI=");
        assert_eq!(sp(false, Period::Semana, Some(1)), "EgQIAxgB");
        assert_eq!(sp(true, Period::Hoje, None), "CAMSAggC");
    }
}

#[cfg(test)]
mod live {
    /// Busca real (rede + yt-dlp): `cargo test -p flaxy-app live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn search_live() {
        let r = super::search(std::env::var("Q").unwrap_or("minecraft".into()), None, &super::REGIONS[0], super::Period::Hoje, true, None).await.expect("busca");
        for (c, v) in &r.videos {
            println!("{:?}: {} vídeos, topo {:?}", c, v.len(), v.first().map(|x| (x.views, x.duration, &x.title)));
        }
        println!("notícias: {} {:?}", r.news.len(), r.news.first().map(|a| (&a.source, &a.title)));
        println!("avisos: {:?}", r.warnings);
        assert!(!r.list(super::Cat::Alta).is_empty());
    }
}

#[cfg(test)]
mod web_tests {
    use super::{ago_en, clock};

    #[test]
    fn parsers() {
        assert_eq!(clock("1:02:03"), Some(3723.0));
        assert_eq!(clock("54:54"), Some(3294.0));
        let t = ago_en("Streamed 5 hours ago").unwrap();
        assert!((chrono::Utc::now() - t).num_minutes() >= 299);
        let t = ago_en("há 2 dias").unwrap();
        assert!((chrono::Utc::now() - t).num_hours() >= 47);
        assert!(ago_en("Premieres soon").is_none());
    }
}
