//! Marcas d'água: textos translúcidos em posições escolhidas, criados na aba
//! Modelos e ligados por clipe. No corte viram uma legenda ASS fixa.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::{Error, Result};

/// Um texto da marca d'água. Posição = centro do texto em fração do quadro;
/// tamanho em px de uma saída com 1920 de altura.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WmText {
    pub text: String,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_size")]
    pub size: f64,
    /// 0 invisível .. 1 opaco.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "yes")]
    pub bold: bool,
    /// Contorno escuro fino, para ler em fundo claro.
    #[serde(default)]
    pub outline: bool,
}

fn default_size() -> f64 {
    64.0
}
fn default_opacity() -> f64 {
    0.35
}
fn default_color() -> String {
    "#ffffff".into()
}
fn default_font() -> String {
    "Arial".into()
}
fn yes() -> bool {
    true
}

pub const SIZE_MIN: f64 = 16.0;
pub const SIZE_MAX: f64 = 320.0;

impl WmText {
    pub fn new(text: &str, x: f64, y: f64) -> Self {
        Self { text: text.into(), x, y, size: default_size(), opacity: default_opacity(), color: default_color(), font: default_font(), bold: true, outline: false }
    }

    /// Valores dentro dos limites (posição no quadro, tamanho e opacidade válidos).
    pub fn sanitized(&self) -> Self {
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        Self {
            text: self.text.chars().take(200).collect(),
            x: f(self.x, 0.5).clamp(0.0, 1.0),
            y: f(self.y, 0.5).clamp(0.0, 1.0),
            size: f(self.size, default_size()).clamp(SIZE_MIN, SIZE_MAX),
            opacity: f(self.opacity, default_opacity()).clamp(0.0, 1.0),
            color: if self.color.trim_start_matches('#').len() == 6 { self.color.clone() } else { default_color() },
            font: if self.font.trim().is_empty() { default_font() } else { self.font.trim().to_string() },
            bold: self.bold,
            outline: self.outline,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct WatermarkRow {
    pub id: i64,
    pub name: String,
    pub items: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub row: WatermarkRow,
    pub items: Vec<WmText>,
}

fn parse(row: WatermarkRow) -> Watermark {
    let items = serde_json::from_str::<Vec<WmText>>(&row.items).unwrap_or_default().iter().map(WmText::sanitized).collect();
    Watermark { row, items }
}

fn encode(items: &[WmText]) -> Result<String> {
    let v: Vec<WmText> = items.iter().map(WmText::sanitized).filter(|t| !t.text.trim().is_empty()).collect();
    Ok(serde_json::to_string(&v)?)
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Watermark>> {
    let rows: Vec<WatermarkRow> = sqlx::query_as("SELECT * FROM watermarks ORDER BY created_at DESC, id DESC").fetch_all(db).await?;
    Ok(rows.into_iter().map(parse).collect())
}

pub async fn get(db: &SqlitePool, id: i64) -> Result<Watermark> {
    let row: WatermarkRow = sqlx::query_as("SELECT * FROM watermarks WHERE id=?").bind(id).fetch_optional(db).await?.ok_or(Error::NotFound("Marca d'água"))?;
    Ok(parse(row))
}

/// Cria (id None) ou atualiza a marca d'água; devolve o id.
pub async fn save(db: &SqlitePool, id: Option<i64>, name: &str, items: &[WmText]) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("Dê um nome para a marca d'água".into()));
    }
    let json = encode(items)?;
    match id {
        Some(id) => {
            sqlx::query("UPDATE watermarks SET name=?, items=? WHERE id=?").bind(name).bind(&json).bind(id).execute(db).await?;
            Ok(id)
        }
        None => {
            let (id,): (i64,) = sqlx::query_as("INSERT INTO watermarks (name, items) VALUES (?, ?) RETURNING id").bind(name).bind(&json).fetch_one(db).await?;
            Ok(id)
        }
    }
}

/// Apaga a marca; clipes que a usavam saem sem marca no próximo corte.
pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM watermarks WHERE id=?").bind(id).execute(db).await?;
    sqlx::query("UPDATE clip_items SET watermark_id=NULL WHERE watermark_id=?").bind(id).execute(db).await?;
    Ok(())
}

/// Texto do usuário não pode abrir tags ASS nem quebrar a linha do evento.
fn escape(s: &str) -> String {
    s.replace('\\', "\u{29F5}").replace('{', "(").replace('}', ")").replace(['\n', '\r'], " ")
}

/// `#rrggbb` para `&HBBGGRR&`.
fn bgr(hex: &str) -> String {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(255);
    format!("&H{:02X}{:02X}{:02X}&", p(4), p(2), p(0))
}

/// Legenda ASS com os textos parados do começo ao fim (`dur` segundos), num
/// quadro `w`×`h`.
pub fn build_ass(items: &[WmText], w: i64, h: i64, dur: f64) -> String {
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 2\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Wm,Arial,64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,1,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    let end = {
        let cs = (dur.max(0.1) * 100.0).ceil() as i64 + 100;
        format!("{}:{:02}:{:02}.{:02}", cs / 360000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
    };
    let scale = h as f64 / 1920.0;
    for t in items.iter().map(WmText::sanitized).filter(|t| !t.text.trim().is_empty()) {
        let a = ((1.0 - t.opacity) * 255.0).round() as u8;
        let bord = if t.outline { format!("\\bord{:.1}\\3c&H000000&\\3a&H{a:02X}&", (t.size * scale * 0.06).max(1.0)) } else { "\\bord0".into() };
        s.push_str(&format!(
            "Dialogue: 0,0:00:00.00,{end},Wm,,0,0,0,,{{\\an5\\pos({:.0},{:.0})\\fn{}\\fs{:.0}\\b{}\\1c{}\\1a&H{a:02X}&{bord}\\shad0}}{}\n",
            t.x * w as f64,
            t.y * h as f64,
            escape(&t.font),
            t.size * scale,
            t.bold as u8,
            bgr(&t.color),
            escape(t.text.trim())
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_places_translucent_text() {
        let mut t = WmText::new("@meucanal {x}", 0.5, 0.1);
        t.opacity = 0.4;
        t.color = "#ff8000".into();
        let ass = build_ass(&[t, WmText::new("   ", 0.2, 0.2)], 1080, 1920, 30.0);
        assert!(ass.contains("PlayResX: 1080"));
        assert!(ass.contains("\\pos(540,192)"));
        assert!(ass.contains("\\1c&H0080FF&"));
        assert!(ass.contains("\\1a&H99&"));
        assert!(ass.contains("@meucanal (x)"));
        assert_eq!(ass.matches("Dialogue:").count(), 1);
        // Horizontal: o tamanho acompanha a altura.
        let h = build_ass(&[WmText::new("a", 0.9, 0.9)], 1920, 1080, 5.0);
        assert!(h.contains("\\fs36") && h.contains("\\pos(1728,972)"));
    }

    /// ASS de exemplo para conferir no ffmpeg à mão.
    #[test]
    #[ignore]
    fn print_ass() {
        let mut a = WmText::new("@meucanal", 0.5, 0.08);
        a.size = 72.0;
        a.opacity = 0.45;
        let mut b = WmText::new("FLAXY", 0.82, 0.92);
        b.color = "#ffcc00".into();
        b.outline = true;
        b.opacity = 0.6;
        print!("{}", build_ass(&[a, b], 1080, 1920, 3.0));
    }

    #[test]
    fn sanitize_limits() {
        let t = WmText { x: 4.0, y: f64::NAN, size: 9999.0, opacity: -1.0, color: "xyz".into(), font: " ".into(), ..WmText::new("a", 0.0, 0.0) }.sanitized();
        assert_eq!((t.x, t.y, t.size, t.opacity), (1.0, 0.5, SIZE_MAX, 0.0));
        assert_eq!((t.color.as_str(), t.font.as_str()), ("#ffffff", "Arial"));
    }
}
