//! Estilos de legenda e geração do arquivo ASS queimado no vídeo (libass).
//!
//! Tempos das palavras vêm em segundos da FONTE. Na geração eles são
//! deslocados pelo trim, para casar com a linha do tempo da saída.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Frase curta; a palavra falada muda de cor.
    Highlight,
    /// Uma palavra por vez, com pop.
    Word,
    /// Frase inteira, fundo semitransparente.
    Classic,
    /// Frase curta; a palavra falada ganha caixa colorida.
    Box,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CaptionStyle {
    pub kind: Kind,
    pub font: String,
    /// Tamanho em px de uma saída de 1920 de altura; escala com a saída real.
    pub size: f64,
    pub bold: bool,
    pub uppercase: bool,
    /// Cores em `#rrggbb`.
    pub color: String,
    pub highlight: String,
    pub outline_color: String,
    pub outline: f64,
    pub shadow: f64,
    /// Fundo da clássica, `#rrggbb` + opacidade 0..1.
    pub back_color: String,
    pub back_opacity: f64,
    /// "bottom" | "center" | "top"
    pub position: String,
    /// Distância da borda, em fração da altura da saída.
    pub margin: f64,
    pub max_words: usize,
    pub max_chars: usize,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            kind: Kind::Highlight,
            font: "Arial".into(),
            size: 84.0,
            bold: true,
            uppercase: true,
            color: "#ffffff".into(),
            highlight: "#facc15".into(),
            outline_color: "#000000".into(),
            outline: 6.0,
            shadow: 0.0,
            back_color: "#000000".into(),
            back_opacity: 0.55,
            position: "bottom".into(),
            margin: 0.2,
            max_words: 3,
            max_chars: 22,
        }
    }
}

/// Os quatro estilos que vêm prontos.
pub fn presets() -> Vec<(&'static str, CaptionStyle)> {
    let base = CaptionStyle::default();
    vec![
        ("Palavra destacada", base.clone()),
        (
            "Palavra por palavra",
            CaptionStyle {
                kind: Kind::Word,
                size: 120.0,
                position: "center".into(),
                margin: 0.0,
                max_words: 1,
                ..base.clone()
            },
        ),
        (
            "Clássica",
            CaptionStyle {
                kind: Kind::Classic,
                size: 62.0,
                uppercase: false,
                bold: false,
                outline: 0.0,
                margin: 0.08,
                max_words: 8,
                max_chars: 42,
                ..base.clone()
            },
        ),
        (
            "Caixa colorida",
            CaptionStyle {
                kind: Kind::Box,
                highlight: "#3b63d6".into(),
                outline: 4.0,
                ..base
            },
        ),
    ]
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StyleRow {
    pub id: i64,
    pub name: String,
    #[serde(skip)]
    pub data: String,
    pub builtin: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Style {
    #[serde(flatten)]
    pub row: StyleRow,
    pub style: CaptionStyle,
}

impl From<StyleRow> for Style {
    fn from(row: StyleRow) -> Self {
        let style = serde_json::from_str(&row.data).unwrap_or_default();
        Style { row, style }
    }
}

/// Grava os estilos prontos na primeira execução.
pub async fn seed(db: &SqlitePool) -> Result<()> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM caption_styles WHERE builtin = 1")
        .fetch_one(db)
        .await?;
    if n > 0 {
        return Ok(());
    }
    for (name, st) in presets() {
        sqlx::query("INSERT INTO caption_styles (name, data, builtin) VALUES (?, ?, 1)")
            .bind(name)
            .bind(serde_json::to_string(&st)?)
            .execute(db)
            .await?;
    }
    Ok(())
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Style>> {
    let rows: Vec<StyleRow> = sqlx::query_as("SELECT * FROM caption_styles ORDER BY builtin DESC, id")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(Style::from).collect())
}

pub async fn get(db: &SqlitePool, id: i64) -> Result<Style> {
    let row: StyleRow = sqlx::query_as("SELECT * FROM caption_styles WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::NotFound("Estilo de legenda"))?;
    Ok(row.into())
}

pub async fn create(db: &SqlitePool, name: &str, st: &CaptionStyle) -> Result<Style> {
    if name.trim().is_empty() {
        return Err(Error::Invalid("Nome do estilo é obrigatório".into()));
    }
    let row: StyleRow =
        sqlx::query_as("INSERT INTO caption_styles (name, data) VALUES (?, ?) RETURNING *")
            .bind(name.trim())
            .bind(serde_json::to_string(st)?)
            .fetch_one(db)
            .await?;
    Ok(row.into())
}

pub async fn update(db: &SqlitePool, id: i64, name: &str, st: &CaptionStyle) -> Result<Style> {
    let row: StyleRow =
        sqlx::query_as("UPDATE caption_styles SET name = ?, data = ? WHERE id = ? RETURNING *")
            .bind(name.trim())
            .bind(serde_json::to_string(st)?)
            .bind(id)
            .fetch_optional(db)
            .await?
            .ok_or(Error::NotFound("Estilo de legenda"))?;
    Ok(row.into())
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM caption_styles WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}

// ------------------------------------------------------------------ ASS

/// `#rrggbb` + alpha (0 opaco .. 255 transparente) para `&HAABBGGRR`.
fn ass_color(hex: &str, alpha: u8) -> String {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(255);
    format!("&H{alpha:02X}{:02X}{:02X}{:02X}", p(4), p(2), p(0))
}

fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as i64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

/// Texto do usuário não pode abrir tags ASS nem quebrar a linha do evento.
fn escape(s: &str) -> String {
    s.replace('\\', "\u{29F5}").replace('{', "(").replace('}', ")").replace('\n', " ")
}

/// Agrupa palavras em frases curtas, respeitando limite de palavras, de
/// caracteres e pausas longas.
fn chunks(words: &[Word], st: &CaptionStyle) -> Vec<Vec<Word>> {
    let max_words = st.max_words.max(1);
    let mut out: Vec<Vec<Word>> = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let mut chars = 0;
    for w in words {
        let len = w.text.chars().count();
        let gap = cur.last().map(|l: &Word| w.start - l.end).unwrap_or(0.0);
        let ends_sentence = cur
            .last()
            .is_some_and(|l| l.text.ends_with(['.', '!', '?']));
        if !cur.is_empty()
            && (cur.len() >= max_words || chars + len + 1 > st.max_chars || gap > 0.6 || ends_sentence)
        {
            out.push(std::mem::take(&mut cur));
            chars = 0;
        }
        chars += len + 1;
        cur.push(w.clone());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Palavras com tempo na saída: desloca pelo trim e descarta o que ficou fora.
pub fn shift(words: &[Word], trim_start: f64, trim_end: Option<f64>) -> Vec<Word> {
    let end = trim_end.unwrap_or(f64::INFINITY);
    words
        .iter()
        .filter(|w| w.end > trim_start && w.start < end && !w.text.trim().is_empty())
        .map(|w| Word {
            text: w.text.trim().to_string(),
            start: (w.start.max(trim_start) - trim_start),
            end: (w.end.min(end) - trim_start),
        })
        .collect()
}

/// Gera o ASS completo para uma saída `w`×`h`.
pub fn build_ass(words: &[Word], st: &CaptionStyle, w: i64, h: i64) -> String {
    build_ass_parts(words, &[(0.0, st.clone())], w, h)
}

/// Linhas de estilo (Main/Back/Hl + sufixo) de um trecho.
fn style_lines(st: &CaptionStyle, w: i64, h: i64, sfx: &str) -> String {
    let scale = h as f64 / 1920.0;
    let size = (st.size * scale).round().max(8.0);
    let outline = st.outline * scale;
    let shadow = st.shadow * scale;
    let (align, margin_v) = match st.position.as_str() {
        "top" => (8, (st.margin * h as f64).round()),
        "center" => (5, 0.0),
        _ => (2, (st.margin * h as f64).round()),
    };
    let margin_h = (w as f64 * 0.06).round();
    let bold = if st.bold { -1 } else { 0 };
    let primary = ass_color(&st.color, 0);
    let outline_c = ass_color(&st.outline_color, 0);
    let back_alpha = ((1.0 - st.back_opacity.clamp(0.0, 1.0)) * 255.0).round() as u8;
    let back = ass_color(&st.back_color, back_alpha);
    let hl = ass_color(&st.highlight, 0);
    // Style "Main": texto normal. "Back"/"Hl": BorderStyle 3 = caixa opaca.
    format!(
        "Style: Main{sfx},{font},{size},{primary},{primary},{outline_c},{shadow_c},{bold},0,0,0,100,100,0,0,1,{outline:.1},{shadow:.1},{align},{margin_h},{margin_h},{margin_v},1\n\
         Style: Back{sfx},{font},{size},{primary},{primary},{back},{back},{bold},0,0,0,100,100,0,0,3,{pad:.1},0,{align},{margin_h},{margin_h},{margin_v},1\n\
         Style: Hl{sfx},{font},{size},{primary},{primary},{hl},{hl},{bold},0,0,0,100,100,0,0,3,{pad:.1},0,{align},{margin_h},{margin_h},{margin_v},1\n",
        font = st.font,
        shadow_c = ass_color("#000000", 96),
        pad = (size * 0.18).max(4.0),
    )
}

/// ASS com a legenda trocando de lugar/tamanho ao longo do clipe: cada trecho
/// `(início em s, estilo)` vale até o próximo. Uma frase fica no trecho onde
/// sua primeira palavra começa (nunca é partida no meio).
pub fn build_ass_parts(words: &[Word], parts: &[(f64, CaptionStyle)], w: i64, h: i64) -> String {
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\n\
         ScaledBorderAndShadow: yes\nYCbCr Matrix: TV.709\n\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, \
         Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, \
         Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
    );
    let sfx = |i: usize| if i == 0 { String::new() } else { i.to_string() };
    for (i, (_, st)) in parts.iter().enumerate() {
        s.push_str(&style_lines(st, w, h, &sfx(i)));
    }
    s.push_str("\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    for (i, (a, st)) in parts.iter().enumerate() {
        let next = parts.get(i + 1).map(|p| p.0).unwrap_or(f64::INFINITY);
        let first = i == 0;
        let mine: Vec<Word> = words.iter().filter(|x| (first || x.start >= *a) && x.start < next).cloned().collect();
        let sx = sfx(i);
        let primary = ass_color(&st.color, 0);
        let hl = ass_color(&st.highlight, 0);
        let text_of = |w: &Word| {
            let t = escape(&w.text);
            if st.uppercase { t.to_uppercase() } else { t }
        };
        let mut ev = |layer: u8, a: f64, b: f64, style: &str, text: String| {
            if b - a < 0.01 {
                return;
            }
            s.push_str(&format!(
                "Dialogue: {layer},{},{},{style}{sx},,0,0,0,,{text}\n",
                ass_time(a),
                ass_time(b)
            ));
        };

        match st.kind {
            Kind::Word => {
                for w in &mine {
                    let pop = r"{\fscx80\fscy80\t(0,90,\fscx100\fscy100)}";
                    ev(0, w.start, w.end.max(w.start + 0.12), "Main", format!("{pop}{}", text_of(w)));
                }
            }
            Kind::Classic => {
                for c in chunks(&mine, st) {
                    let line: Vec<String> = c.iter().map(&text_of).collect();
                    let (a, b) = (c[0].start, c.last().unwrap().end);
                    ev(0, a, b, "Back", line.join(" "));
                }
            }
            Kind::Highlight | Kind::Box => {
                for c in chunks(&mine, st) {
                    let n = c.len();
                    for i in 0..n {
                        let a = c[i].start;
                        // Mantém a frase na tela até a próxima palavra (sem piscar).
                        let b = if i + 1 < n { c[i + 1].start } else { c[i].end };
                        if st.kind == Kind::Highlight {
                            let line: Vec<String> = c
                                .iter()
                                .enumerate()
                                .map(|(j, w)| {
                                    if j == i {
                                        format!(r"{{\c{hl}}}{}{{\c{primary}}}", text_of(w))
                                    } else {
                                        text_of(w)
                                    }
                                })
                                .collect();
                            ev(0, a, b, "Main", line.join(" "));
                        } else {
                            // Camada 0: frase normal. Camada 1: mesma frase com só a
                            // palavra ativa visível, em estilo de caixa. Mesmo texto,
                            // mesma posição: a caixa cai exatamente sobre a palavra.
                            let plain: Vec<String> = c.iter().map(&text_of).collect();
                            ev(0, a, b, "Main", plain.join(" "));
                            let boxed: Vec<String> = c
                                .iter()
                                .enumerate()
                                .map(|(j, w)| {
                                    if j == i {
                                        format!(r"{{\alpha&H00&}}{}{{\alpha&HFF&}}", text_of(w))
                                    } else {
                                        text_of(w)
                                    }
                                })
                                .collect();
                            ev(1, a, b, "Hl", format!(r"{{\alpha&HFF&}}{}", boxed.join(" ")));
                        }
                    }
                }
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, a: f64, b: f64) -> Word {
        Word { text: t.into(), start: a, end: b }
    }

    #[test]
    fn colors_are_bgr() {
        assert_eq!(ass_color("#ff8000", 0), "&H000080FF");
    }

    #[test]
    fn shift_follows_trim() {
        let words = vec![w("a", 0.0, 0.5), w("b", 1.0, 1.5), w("c", 3.0, 3.5)];
        let s = shift(&words, 0.8, Some(3.2));
        assert_eq!(s.len(), 2);
        assert!((s[0].start - 0.2).abs() < 1e-9);
        assert!((s[1].end - 2.4).abs() < 1e-9);
    }

    #[test]
    fn highlight_marks_active_word() {
        let words = vec![w("oi", 0.0, 0.4), w("tudo", 0.4, 0.8), w("bem", 0.8, 1.2)];
        let ass = build_ass(&words, &CaptionStyle::default(), 1080, 1920);
        assert_eq!(ass.matches("Dialogue:").count(), 3);
        assert!(ass.contains(r"{\c&H0015CCFA}TUDO{\c&H00FFFFFF}"));
    }

    #[test]
    fn parts_switch_style() {
        let words = vec![w("oi", 0.0, 0.4), w("tudo", 0.4, 0.8), w("bem", 2.0, 2.4)];
        let a = CaptionStyle { kind: Kind::Word, ..CaptionStyle::default() };
        let b = CaptionStyle { position: "top".into(), ..a.clone() };
        let ass = build_ass_parts(&words, &[(0.0, a), (1.5, b)], 1080, 1920);
        assert!(ass.contains("Style: Main1,"));
        assert_eq!(ass.matches(",Main,").count(), 2);
        assert_eq!(ass.matches(",Main1,").count(), 1);
    }

    #[test]
    fn escape_blocks_tags() {
        let words = vec![w("{\\b1}x", 0.0, 1.0)];
        let ass = build_ass(&words, &presets()[1].1, 1080, 1920);
        assert!(!ass.contains(r"{\b1}"));
    }
}
