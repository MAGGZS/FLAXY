//! Clipes de retenção: link -> yt-dlp -> transcrição -> LLM sugere trechos ->
//! usuário aprova e marca câmera/tela -> render por layout.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Semaphore;

use crate::ai::{self, TranscribeCtx};
use crate::captions::{self, Word};
use crate::compose::run_ffmpeg;
use crate::{llm, media, Config, Error, Result};

pub const LAYOUTS: &[&str] = &["vertical_camera", "vertical_split", "vertical_split_cam", "horizontal"];
const MAX_SUGGESTIONS: usize = 10;

/// Retângulo em fração 0..1 do quadro da fonte.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const FULL: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub fn clamped(self) -> Rect {
        let x = self.x.clamp(0.0, 0.98);
        let y = self.y.clamp(0.0, 0.98);
        Rect { x, y, w: self.w.clamp(0.02, 1.0 - x), h: self.h.clamp(0.02, 1.0 - y) }
    }

    fn crop(self) -> String {
        let r = self.clamped();
        format!("crop=iw*{:.5}:ih*{:.5}:iw*{:.5}:ih*{:.5}", r.w, r.h, r.x, r.y)
    }
}

/// Troca de área a partir do instante `t` (segundos da fonte). Corte seco:
/// a área nova vale até a próxima troca ou o fim do clipe.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct AreaKey {
    pub t: f64,
    pub camera: Rect,
    pub screen: Rect,
    /// Modo de recorte deste trecho. None = o modo do clipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Vertical>,
}

/// Modos verticais que um trecho pode usar (todos saem em 1080×1920).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Vertical {
    #[serde(rename = "vertical_camera")]
    Camera,
    #[serde(rename = "vertical_split")]
    Split,
    /// Câmera em cima, tela embaixo.
    #[serde(rename = "vertical_split_cam")]
    SplitCam,
}

impl Vertical {
    pub fn id(self) -> &'static str {
        match self {
            Vertical::Camera => "vertical_camera",
            Vertical::Split => "vertical_split",
            Vertical::SplitCam => "vertical_split_cam",
        }
    }

    pub fn from_id(s: &str) -> Option<Vertical> {
        match s {
            "vertical_camera" => Some(Vertical::Camera),
            "vertical_split" => Some(Vertical::Split),
            "vertical_split_cam" => Some(Vertical::SplitCam),
            _ => None,
        }
    }
}

/// Um trecho do clipe: tempos relativos ao começo do clipe, áreas e modo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seg {
    pub a: f64,
    pub b: f64,
    pub camera: Rect,
    pub screen: Rect,
    pub layout: &'static str,
}

fn layout_id(l: &str) -> &'static str {
    LAYOUTS.iter().copied().find(|x| *x == l).unwrap_or("vertical_camera")
}

/// Onde uma área encaixada fica dentro do seu espaço: `x`/`y` de 0 (esquerda,
/// topo) a 1 (direita, base), 0,5 = centro; `scale` 1 = o maior tamanho que
/// cabe inteiro, menor encolhe.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Place {
    #[serde(default = "half")]
    pub x: f64,
    #[serde(default = "half")]
    pub y: f64,
    #[serde(default = "one")]
    pub scale: f64,
}

fn half() -> f64 {
    0.5
}
fn one() -> f64 {
    1.0
}

pub const PLACE_SCALE_MIN: f64 = 0.3;

impl Default for Place {
    fn default() -> Self {
        Self::CENTER
    }
}

impl Place {
    pub const CENTER: Place = Place { x: 0.5, y: 0.5, scale: 1.0 };

    pub fn clamped(self) -> Self {
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        Self { x: f(self.x, 0.5).clamp(0.0, 1.0), y: f(self.y, 0.5).clamp(0.0, 1.0), scale: f(self.scale, 1.0).clamp(PLACE_SCALE_MIN, 1.0) }
    }

    pub fn is_center(self) -> bool {
        (self.x - 0.5).abs() < 1e-4 && (self.y - 0.5).abs() < 1e-4 && (self.scale - 1.0).abs() < 1e-4
    }
}

/// Como cada área ocupa o seu espaço no clipe.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct AreaFit {
    /// true = encaixar: a área aparece inteira e a sobra é o próprio vídeo
    /// desfocado. false = preencher, cortando o que passar.
    pub camera: bool,
    pub screen: bool,
    /// Posição e tamanho de cada área dentro do espaço, quando encaixada.
    #[serde(default)]
    pub cam_place: Place,
    #[serde(default)]
    pub scr_place: Place,
}

#[derive(Serialize, Deserialize, Default)]
struct Places {
    #[serde(default)]
    camera: Place,
    #[serde(default)]
    screen: Place,
}

impl AreaFit {
    pub fn bits(self) -> i64 {
        self.camera as i64 | (self.screen as i64) << 1
    }

    pub fn from_bits(b: i64) -> Self {
        Self { camera: b & 1 != 0, screen: b & 2 != 0, ..Default::default() }
    }

    /// Lê as posições gravadas na coluna `area_place` (JSON).
    pub fn with_places(mut self, json: Option<&str>) -> Self {
        if let Some(p) = json.and_then(|j| serde_json::from_str::<Places>(j).ok()) {
            self.cam_place = p.camera.clamped();
            self.scr_place = p.screen.clamped();
        }
        self
    }

    /// JSON das posições para a coluna `area_place` (None = as duas no centro).
    pub fn places_json(self) -> Option<String> {
        let (c, s) = (self.cam_place.clamped(), self.scr_place.clamped());
        if c.is_center() && s.is_center() {
            return None;
        }
        serde_json::to_string(&Places { camera: c, screen: s }).ok()
    }
}

/// Posição e tamanho da legenda só de um clipe, por cima do modelo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionAdjust {
    /// "top" | "center" | "bottom"
    pub position: String,
    /// Distância da borda em fração da altura (ignorada no centro).
    pub margin: f64,
    /// Tamanho em px de uma saída de 1920 de altura.
    pub size: f64,
}

impl CaptionAdjust {
    pub fn from_style(st: &captions::CaptionStyle) -> Self {
        Self { position: st.position.clone(), margin: st.margin, size: st.size }
    }

    pub fn apply(&self, st: &mut captions::CaptionStyle) {
        st.position = if ["top", "center", "bottom"].contains(&self.position.as_str()) { self.position.clone() } else { "bottom".into() };
        st.margin = self.margin.clamp(0.0, 0.45);
        st.size = self.size.clamp(16.0, 400.0);
    }
}

/// Trecho cortado de dentro do clipe (pausa, erro), em segundos da fonte.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Cut {
    pub a: f64,
    pub b: f64,
}

/// Correção de uma palavra da legenda só neste clipe. `t` = início da
/// palavra (segundos da fonte); texto vazio apaga a palavra.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordEdit {
    pub t: f64,
    pub text: String,
}

/// Parte da altura que a tela ocupa nos modos tela + câmera.
pub const DEFAULT_SPLIT: f64 = 0.6;

/// Trechos que ficam no clipe (segundos da fonte), tirando os cortes. Nunca
/// vazio: se tudo foi cortado, fica o clipe inteiro.
pub fn kept_ranges(start: f64, end: f64, cuts: &[Cut]) -> Vec<(f64, f64)> {
    let mut cs: Vec<Cut> = cuts
        .iter()
        .filter(|c| c.a.is_finite() && c.b.is_finite())
        .map(|c| Cut { a: c.a.min(c.b).max(start), b: c.a.max(c.b).min(end) })
        .filter(|c| c.b - c.a > 0.05)
        .collect();
    cs.sort_by(|x, y| x.a.total_cmp(&y.a));
    let mut out = Vec::new();
    let mut cur = start;
    for c in cs {
        if c.a > cur + 0.1 {
            out.push((cur, c.a));
        }
        cur = cur.max(c.b);
    }
    if end > cur + 0.1 {
        out.push((cur, end));
    }
    if out.is_empty() {
        out.push((start, end));
    }
    out
}

/// Instante no clipe final (sem os cortes) de um instante da fonte. Dentro
/// de um corte, vai para o começo do próximo trecho que fica.
pub fn out_time(t: f64, kept: &[(f64, f64)]) -> f64 {
    let mut acc = 0.0;
    for (a, b) in kept {
        if t <= *b {
            return acc + (t.max(*a) - a);
        }
        acc += b - a;
    }
    acc
}

/// Duração final do clipe, sem os cortes.
pub fn kept_len(kept: &[(f64, f64)]) -> f64 {
    kept.iter().map(|(a, b)| b - a).sum()
}

/// Cortes automáticos nas pausas: silêncios entre palavras maiores que
/// `min_gap`, deixando `pad` de respiro em cada lado.
pub fn pause_cuts(words: &[Word], start: f64, end: f64, min_gap: f64) -> Vec<Cut> {
    let pad = 0.15;
    let mut ws: Vec<&Word> = words.iter().filter(|w| w.end > start && w.start < end && !w.text.trim().is_empty()).collect();
    ws.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out = Vec::new();
    let mut prev = start;
    for w in ws.iter().chain(std::iter::once(&&Word { text: "x".into(), start: end, end })) {
        if w.start - prev > min_gap {
            let (a, b) = (prev + pad, w.start - pad);
            if b - a > 0.2 {
                out.push(Cut { a, b });
            }
        }
        prev = prev.max(w.end);
    }
    out
}

/// Palavras da legenda já no tempo do clipe final: correções aplicadas,
/// deslocadas pelo começo e sem o que caiu nos cortes.
pub fn clip_words(words: &[Word], edits: &[WordEdit], kept: &[(f64, f64)]) -> Vec<Word> {
    let mut out = Vec::new();
    for w in words {
        let text = match edits.iter().find(|e| (e.t - w.start).abs() < 0.02) {
            Some(e) => e.text.trim().to_string(),
            None => w.text.trim().to_string(),
        };
        if text.is_empty() {
            continue;
        }
        if let Some((a, b)) = kept.iter().find(|(a, b)| w.end > *a && w.start < *b) {
            let (s0, e0) = (w.start.max(*a), w.end.min(*b));
            out.push(Word { text, start: out_time(s0, kept), end: out_time(e0, kept) });
        }
    }
    out
}

/// Trechos finais do clipe: áreas (com transição suave, se pedida) cruzadas
/// com o que sobra dos cortes. Tempos relativos ao começo do clipe, na fonte.
#[allow(clippy::too_many_arguments)]
pub fn clip_plan(start: f64, end: f64, layout: &str, camera: Rect, screen: Rect, keys: &[AreaKey], cuts: &[Cut], smooth: bool, smooth_secs: f64, curve: Option<Ease>) -> Vec<Seg> {
    let segs = area_segments(start, end, layout, camera, screen, keys);
    let segs = if smooth { smooth_segments(&segs, smooth_secs, curve) } else { segs };
    let mut out = Vec::new();
    for (ka, kb) in kept_ranges(start, end, cuts) {
        let (ka, kb) = (ka - start, kb - start);
        for sg in &segs {
            let (a, b) = (sg.a.max(ka), sg.b.min(kb));
            if b - a > 0.02 {
                out.push(Seg { a, b, ..*sg });
            }
        }
    }
    out
}

/// Interpolação entre duas áreas (0 = `p`, 1 = `q`).
pub fn lerp_rect(p: Rect, q: Rect, k: f64) -> Rect {
    let m = |x: f64, y: f64| x + (y - x) * k;
    Rect { x: m(p.x, q.x), y: m(p.y, q.y), w: m(p.w, q.w), h: m(p.h, q.h) }
}

/// Motivo gravado nos clipes criados à mão (o que separa eles das sugestões da IA).
pub const MANUAL_REASON: &str = "Adicionado à mão";

/// Duração padrão da transição suave entre duas áreas.
pub const SMOOTH_SECS: f64 = 0.6;
/// Faixa aceita para a duração do deslize (segundos).
pub const SMOOTH_MIN: f64 = 0.2;
pub const SMOOTH_MAX: f64 = 2.0;
/// Passos por segundo do deslize: um por quadro a 30 fps.
const SMOOTH_STEPS: f64 = 30.0;

/// Curva do deslize: começa e termina parada, sem tranco (smootherstep).
pub fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// Curva de velocidade do deslize, como a `cubic-bezier` do CSS: pontos de
/// controle (x1, y1) e (x2, y2) entre (0,0) e (1,1). `y` fora de 0..1 passa do
/// ponto e volta (efeito mola).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Ease {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// Limites de `y` dos pontos de controle (quanto a mola pode passar do ponto).
pub const EASE_Y_MIN: f64 = -0.6;
pub const EASE_Y_MAX: f64 = 1.6;

impl Ease {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Curvas prontas: (id, nome, curva). `None` = a suave padrão.
    pub const PRESETS: [(&'static str, &'static str, Option<Ease>); 8] = [
        ("suave", "Suave", None),
        ("linear", "Linear", Some(Ease::new(0.0, 0.0, 1.0, 1.0))),
        ("entrada", "Acelera", Some(Ease::new(0.55, 0.0, 1.0, 0.45))),
        ("saida", "Freia", Some(Ease::new(0.0, 0.55, 0.45, 1.0))),
        ("forte", "Forte", Some(Ease::new(0.85, 0.0, 0.15, 1.0))),
        ("rapido", "Rápido", Some(Ease::new(0.2, 0.9, 0.25, 1.0))),
        ("mola", "Mola", Some(Ease::new(0.34, 1.45, 0.64, 1.0))),
        ("elastico", "Vai e volta", Some(Ease::new(0.68, -0.5, 0.32, 1.5))),
    ];

    /// Mesma curva com os pontos dentro dos limites.
    pub fn clamped(self) -> Self {
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        Self {
            x1: f(self.x1, 0.0).clamp(0.0, 1.0),
            y1: f(self.y1, 0.0).clamp(EASE_Y_MIN, EASE_Y_MAX),
            x2: f(self.x2, 1.0).clamp(0.0, 1.0),
            y2: f(self.y2, 1.0).clamp(EASE_Y_MIN, EASE_Y_MAX),
        }
    }

    /// Progresso (0..1, podendo passar com mola) no instante `x` (0..1).
    pub fn at(self, x: f64) -> f64 {
        let e = self.clamped();
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let bz = |t: f64, p1: f64, p2: f64| {
            let u = 1.0 - t;
            3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
        };
        let dbz = |t: f64, p1: f64, p2: f64| {
            let u = 1.0 - t;
            3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
        };
        // Acha t com x(t) = x: Newton, e bisseção se a derivada sumir.
        let mut t = x;
        for _ in 0..8 {
            let err = bz(t, e.x1, e.x2) - x;
            if err.abs() < 1e-7 {
                return bz(t, e.y1, e.y2);
            }
            let d = dbz(t, e.x1, e.x2);
            if d.abs() < 1e-6 {
                break;
            }
            t = (t - err / d).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        t = x;
        for _ in 0..40 {
            let v = bz(t, e.x1, e.x2);
            if (v - x).abs() < 1e-7 {
                break;
            }
            if v < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
        bz(t, e.y1, e.y2)
    }
}

/// Progresso do deslize em `x` (0..1) com a curva escolhida (None = suave).
pub fn ease_with(curve: Option<Ease>, x: f64) -> f64 {
    match curve {
        Some(c) => c.at(x),
        None => ease(x),
    }
}

/// Troca corte seco por um deslize: nos primeiros `secs` de cada trecho a
/// área anda da anterior até a nova, quadro a quadro, no ritmo de `curve`.
/// Só entre trechos do mesmo modo (trocar de modo continua sendo corte seco).
pub fn smooth_segments(segs: &[Seg], secs: f64, curve: Option<Ease>) -> Vec<Seg> {
    let secs = if secs.is_finite() { secs.clamp(SMOOTH_MIN, SMOOTH_MAX) } else { SMOOTH_SECS };
    let mut out: Vec<Seg> = Vec::new();
    for (i, sg) in segs.iter().enumerate() {
        let prev = if i > 0 { Some(segs[i - 1]) } else { None };
        match prev.filter(|p| p.layout == sg.layout && sg.layout != "horizontal") {
            Some(p) => {
                let d = secs.min((sg.b - sg.a) / 2.0);
                let n = ((d * SMOOTH_STEPS).round() as usize).max(2);
                for j in 0..n {
                    let (a, b) = (sg.a + d * j as f64 / n as f64, sg.a + d * (j + 1) as f64 / n as f64);
                    let k = ease_with(curve, (j as f64 + 0.5) / n as f64);
                    // Com mola a área passa do ponto: não deixa sair do quadro.
                    let fix = |r: Rect| if (0.0..=1.0).contains(&k) { r } else { r.clamped() };
                    out.push(Seg { a, b, camera: fix(lerp_rect(p.camera, sg.camera, k)), screen: fix(lerp_rect(p.screen, sg.screen, k)), layout: sg.layout });
                }
                out.push(Seg { a: sg.a + d, ..*sg });
            }
            None => out.push(*sg),
        }
    }
    out
}

/// Troca de legenda a partir de `t` (segundos da fonte), como as de área:
/// dali em diante a legenda usa esta posição e tamanho.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionKey {
    pub t: f64,
    pub adjust: CaptionAdjust,
}

/// Ajuste de legenda que vale em `t` (segundos da fonte): o da última troca
/// até ali, ou o do clipe.
pub fn caption_at(base: Option<&CaptionAdjust>, keys: &[CaptionKey], start: f64, t: f64) -> Option<CaptionAdjust> {
    keys.iter()
        .filter(|k| k.t >= start && k.t <= t + 1e-6)
        .max_by(|a, b| a.t.total_cmp(&b.t))
        .map(|k| k.adjust.clone())
        .or_else(|| base.cloned())
}

/// Trechos da legenda: (início relativo ao clipe, ajuste). Mesmas regras das
/// áreas: trocas fora do clipe somem, trocas a menos de 0,2 s ficam com a última.
pub fn caption_parts(start: f64, end: f64, base: Option<CaptionAdjust>, keys: &[CaptionKey]) -> Vec<(f64, Option<CaptionAdjust>)> {
    let mut ks: Vec<&CaptionKey> = keys.iter().filter(|k| k.t.is_finite() && k.t >= start && end - k.t > 0.2).collect();
    ks.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut out = vec![(0.0, base)];
    for k in ks {
        let at = k.t - start;
        let last = out.last_mut().unwrap();
        if at - last.0 < 0.2 {
            last.1 = Some(k.adjust.clone());
        } else {
            out.push((at, Some(k.adjust.clone())));
        }
    }
    out
}

/// Trechos do clipe com a área e o modo de cada um, em segundos relativos ao
/// começo do clipe. Trocas fora do clipe são ignoradas; trocas a menos de
/// 0,2 s uma da outra (ou do começo) ficam com a última. No horizontal não
/// há troca de modo (sai em 16:9).
pub fn area_segments(start: f64, end: f64, layout: &str, camera: Rect, screen: Rect, keys: &[AreaKey]) -> Vec<Seg> {
    let base = layout_id(layout);
    let len = (end - start).max(0.0);
    let mut ks: Vec<AreaKey> = keys.iter().copied().filter(|k| k.t.is_finite() && k.t >= start && end - k.t > 0.2).collect();
    ks.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mode = |k: &AreaKey| if base == "horizontal" { base } else { k.layout.map(Vertical::id).unwrap_or(base) };
    let mut out = vec![Seg { a: 0.0, b: len, camera, screen, layout: base }];
    for k in ks {
        let at = k.t - start;
        let last = out.last_mut().unwrap();
        if at - last.a < 0.2 {
            last.camera = k.camera;
            last.screen = k.screen;
            last.layout = mode(&k);
        } else {
            last.b = at;
            out.push(Seg { a: at, b: len, camera: k.camera, screen: k.screen, layout: mode(&k) });
        }
    }
    out
}

/// Fundo atrás de uma área no modo Encaixar (a sobra em volta dela).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ClipBg {
    /// O próprio vídeo esticado e bem desfocado (o padrão).
    #[default]
    Blur,
    /// O próprio vídeo esticado, só um pouco desfocado.
    Soft,
    /// Sem vídeo atrás: preto.
    Black,
    /// Imagem ou vídeo escolhido (caminho do arquivo, da biblioteca de fundos).
    File(String),
}

impl ClipBg {
    /// Modos (id, nome) na ordem dos botões.
    pub const MODES: [(&'static str, &'static str); 4] = [("blur", "Desfocado"), ("soft", "Desfoque leve"), ("black", "Sem fundo"), ("file", "Arquivo")];

    pub fn mode(&self) -> &'static str {
        match self {
            ClipBg::Blur => "blur",
            ClipBg::Soft => "soft",
            ClipBg::Black => "black",
            ClipBg::File(_) => "file",
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self {
            ClipBg::File(p) => Some(p),
            _ => None,
        }
    }

    /// Lê o que foi salvo; "file" sem arquivo volta ao desfocado.
    pub fn from_parts(mode: &str, path: Option<&str>) -> Self {
        match (mode, path.filter(|p| !p.trim().is_empty())) {
            ("soft", _) => ClipBg::Soft,
            ("black", _) => ClipBg::Black,
            ("file", Some(p)) => ClipBg::File(p.to_string()),
            _ => ClipBg::Blur,
        }
    }

    /// O arquivo é imagem (entra repetindo o mesmo quadro).
    pub fn is_image(path: &str) -> bool {
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp")
    }
}

/// Estado do fundo enquanto o grafo é montado: o trecho atual (na entrada) e
/// os rótulos já pedidos do arquivo de fundo (entrada 1), um por área encaixada.
struct BgUse<'a> {
    bg: &'a ClipBg,
    a: f64,
    b: f64,
    labels: Vec<String>,
}

impl BgUse<'_> {
    fn label(&mut self) -> String {
        let l = format!("bgf{}", self.labels.len());
        self.labels.push(l.clone());
        l
    }
}

/// Expressão de posição do FFmpeg: `free` (a sobra, ex. `W-w`) vezes `f`.
fn pos_expr(free: &str, f: f64) -> String {
    if (f - 0.5).abs() < 1e-4 {
        format!("({free})/2")
    } else {
        format!("({free})*{f:.4}")
    }
}

/// Uma área recortada em W×H, de `[inp]` para `[out]`. Encaixada, fica onde
/// `place` diz dentro do espaço.
#[allow(clippy::too_many_arguments)]
fn box_chain(inp: &str, out: &str, r: Rect, fit: bool, place: Place, w: i64, h: i64, bg: &mut BgUse) -> String {
    if !fit {
        return format!("[{inp}]{},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1[{out}]", r.crop());
    }
    let place = place.clamped();
    let even = |v: f64| ((v / 2.0).round() as i64 * 2).max(2);
    let (fw, fh) = (even(w as f64 * place.scale), even(h as f64 * place.scale));
    let fg = format!("[{out}f]scale={fw}:{fh}:force_original_aspect_ratio=decrease:force_divisible_by=2[{out}fg]");
    let top = format!("[{out}bg][{out}fg]overlay={}:{},setsar=1[{out}]", pos_expr("W-w", place.x), pos_expr("H-h", place.y));
    let cover = format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
    match bg.bg {
        ClipBg::Black => format!(
            "[{inp}]{},scale={fw}:{fh}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={w}:{h}:{}:{}:black,setsar=1[{out}]",
            r.crop(),
            pos_expr("ow-iw", place.x),
            pos_expr("oh-ih", place.y)
        ),
        ClipBg::File(_) => {
            // O vídeo da área (esticado) só dá o ritmo dos quadros; o arquivo
            // cobre ele inteiro. Cortado no mesmo trecho, o fundo não reinicia.
            // O fundo ganha 1 s de sobra (tpad) e o overlay para junto com a
            // área (shortest): quadros do fundo fora do ritmo não viram quadro a mais.
            let l = bg.label();
            format!(
                "[{inp}]{},split=2[{out}f][{out}g];[{out}g]{cover},setsar=1[{out}cv];\
                 [{l}]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS,{cover},setsar=1,tpad=stop_mode=clone:stop_duration=1[{out}bi];\
                 [{out}cv][{out}bi]overlay=0:0:shortest=1[{out}bg];{fg};{top}",
                r.crop(),
                bg.a,
                bg.b
            )
        }
        ClipBg::Blur | ClipBg::Soft => {
            let blur = if *bg.bg == ClipBg::Soft { "boxblur=6:1" } else { "boxblur=24:2" };
            format!("[{inp}]{},split=2[{out}f][{out}g];[{out}g]{cover},{blur}[{out}bg];{fg};{top}", r.crop())
        }
    }
}

/// Recorte + escala de um layout, de `[inp]` para `[out]`, sempre ow×oh.
#[allow(clippy::too_many_arguments)]
fn layout_chain(layout: &str, inp: &str, out: &str, cam: Rect, scr: Rect, fit: AreaFit, split: f64, ow: i64, oh: i64, bg: &mut BgUse) -> String {
    // Altura (par) da parte da tela.
    let sh = (((oh as f64 * split.clamp(0.2, 0.8)) / 2.0).round() as i64) * 2;
    match layout {
        "vertical_split" => {
            // Tela em cima (`split` da altura), câmera embaixo.
            let (th, bh) = (sh, oh - sh);
            let t = box_chain(&format!("{out}a"), &format!("{out}t"), scr, fit.screen, fit.scr_place, ow, th, bg);
            let m = box_chain(&format!("{out}b"), &format!("{out}m"), cam, fit.camera, fit.cam_place, ow, bh, bg);
            format!("[{inp}]split=2[{out}a][{out}b];{t};{m};[{out}t][{out}m]vstack=inputs=2[{out}]")
        }
        "vertical_split_cam" => {
            // Invertido: câmera em cima, tela embaixo.
            let (th, bh) = (oh - sh, sh);
            let t = box_chain(&format!("{out}a"), &format!("{out}t"), cam, fit.camera, fit.cam_place, ow, th, bg);
            let m = box_chain(&format!("{out}b"), &format!("{out}m"), scr, fit.screen, fit.scr_place, ow, bh, bg);
            format!("[{inp}]split=2[{out}a][{out}b];{t};{m};[{out}t][{out}m]vstack=inputs=2[{out}]")
        }
        "horizontal" => format!("[{inp}]scale={ow}:{oh}:force_original_aspect_ratio=decrease,pad={ow}:{oh}:(ow-iw)/2:(oh-ih)/2:black,setsar=1[{out}]"),
        _ => box_chain(inp, out, cam, fit.camera, fit.cam_place, ow, oh, bg),
    }
}

/// Grafo completo do clipe, com o fundo padrão (vídeo desfocado).
#[allow(clippy::too_many_arguments)]
pub fn clip_graph(layout: &str, segments: &[Seg], fit: AreaFit, split: f64, ow: i64, oh: i64, ass: &str, len: f64) -> String {
    clip_graph_bg(layout, segments, fit, split, ow, oh, ass, len, &ClipBg::Blur)
}

/// Grafo completo do clipe. Com trocas de área ou cortes, cada trecho é
/// recortado à parte e os trechos são emendados (concat) antes da legenda,
/// que precisa da linha do tempo contínua. `len` = duração da entrada.
/// Com `ClipBg::File`, o arquivo de fundo tem que ser a entrada 1.
#[allow(clippy::too_many_arguments)]
pub fn clip_graph_bg(layout: &str, segments: &[Seg], fit: AreaFit, split: f64, ow: i64, oh: i64, ass: &str, len: f64, bg: &ClipBg) -> String {
    let mut bu = BgUse { bg, a: 0.0, b: len, labels: Vec::new() };
    let whole = segments.len() <= 1 && segments.first().is_none_or(|s| s.a <= 0.001 && s.b >= len - 0.001);
    let mut g = if whole {
        let s = segments.first().copied().unwrap_or(Seg { a: 0.0, b: 0.0, camera: Rect::FULL, screen: Rect::FULL, layout: layout_id(layout) });
        let l = if layout == "horizontal" { "horizontal" } else { s.layout };
        format!("{};[v]null{ass},format=yuv420p[out]", layout_chain(l, "0:v", "v", s.camera, s.screen, fit, split, ow, oh, &mut bu))
    } else {
        let n = segments.len();
        let mut g = format!("[0:v]split={n}{}", (0..n).map(|i| format!("[i{i}]")).collect::<String>());
        for (i, s) in segments.iter().enumerate() {
            g.push_str(&format!(";[i{i}]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[c{i}];", s.a, s.b));
            let l = if layout == "horizontal" { "horizontal" } else { s.layout };
            (bu.a, bu.b) = (s.a, s.b);
            g.push_str(&layout_chain(l, &format!("c{i}"), &format!("s{i}"), s.camera, s.screen, fit, split, ow, oh, &mut bu));
        }
        g.push(';');
        g.push_str(&(0..n).map(|i| format!("[s{i}]")).collect::<String>());
        // Trechos curtos (deslize) têm menos de 2 quadros: o concat encavala os
        // tempos e o encoder descartaria quadros, tirando o áudio de sincronia.
        // Refaz os tempos quadro a quadro.
        let retime = if segments.iter().any(|s| s.b - s.a < 0.3) { "setpts=N/FRAME_RATE/TB," } else { "" };
        g.push_str(&format!("concat=n={n}:v=1:a=0[v];[v]{retime}null{ass},format=yuv420p[out]"));
        g
    };
    // Arquivo de fundo: uma cópia da entrada 1 para cada área encaixada.
    if !bu.labels.is_empty() {
        let k = bu.labels.len();
        g = format!("[1:v]split={k}{};{g}", bu.labels.iter().map(|l| format!("[{l}]")).collect::<String>());
    }
    g
}

/// Áudio só dos trechos que ficam (tempos relativos à entrada), em `[aout]`.
pub fn audio_graph(kept_rel: &[(f64, f64)]) -> String {
    let n = kept_rel.len();
    let mut g = format!("[0:a]asplit={n}{}", (0..n).map(|i| format!("[ai{i}]")).collect::<String>());
    for (i, (a, b)) in kept_rel.iter().enumerate() {
        g.push_str(&format!(";[ai{i}]atrim=start={a:.3}:end={b:.3},asetpts=PTS-STARTPTS[ac{i}]"));
    }
    g.push(';');
    g.push_str(&(0..n).map(|i| format!("[ac{i}]")).collect::<String>());
    g.push_str(&format!("concat=n={n}:v=0:a=1[aout]"));
    g
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Project {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub source_path: Option<String>,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub status: String,
    pub progress: i64,
    pub error_msg: Option<String>,
    pub transcript_id: Option<i64>,
    pub min_len: f64,
    pub max_len: f64,
    pub layout: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ClipRow {
    pub id: i64,
    pub project_id: i64,
    pub start: f64,
    pub end: f64,
    pub score: i64,
    pub title: String,
    pub reason: String,
    pub approved: bool,
    pub layout: String,
    #[serde(skip)]
    pub camera_rect: Option<String>,
    #[serde(skip)]
    pub screen_rect: Option<String>,
    pub caption_style_id: Option<i64>,
    pub status: String,
    pub progress: i64,
    pub output_path: Option<String>,
    pub error_msg: Option<String>,
    #[serde(skip)]
    pub area_keys: Option<String>,
    #[serde(skip)]
    pub area_fit: i64,
    #[serde(skip)]
    pub caption_adjust: Option<String>,
    #[serde(skip)]
    pub caption_keys: Option<String>,
    pub split: f64,
    #[serde(skip)]
    pub word_edits: Option<String>,
    #[serde(skip)]
    pub cuts: Option<String>,
    pub smooth: bool,
    pub smooth_secs: f64,
    #[serde(skip)]
    pub smooth_curve: Option<String>,
    pub bg_mode: String,
    pub bg_path: Option<String>,
    pub watermark_id: Option<i64>,
    /// Posição das áreas encaixadas (JSON de `Places`).
    #[serde(skip)]
    pub area_place: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    #[serde(flatten)]
    pub row: ClipRow,
    pub camera: Rect,
    pub screen: Rect,
    pub keys: Vec<AreaKey>,
    pub fit: AreaFit,
    pub caption: Option<CaptionAdjust>,
    pub cap_keys: Vec<CaptionKey>,
    pub word_edits: Vec<WordEdit>,
    pub cuts: Vec<Cut>,
    pub curve: Option<Ease>,
    #[serde(skip)]
    pub bg: ClipBg,
}

fn parse_json<T: serde::de::DeserializeOwned + Default>(s: &Option<String>) -> T {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default()
}

fn parse_curve(s: &Option<String>) -> Option<Ease> {
    s.as_deref().and_then(|v| serde_json::from_str::<Ease>(v).ok()).map(Ease::clamped)
}

fn parse_cap_keys(s: &Option<String>) -> Vec<CaptionKey> {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default()
}

fn parse_keys(s: &Option<String>) -> Vec<AreaKey> {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default()
}

fn parse_caption(s: &Option<String>) -> Option<CaptionAdjust> {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok())
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProject {
    pub url: String,
    pub min_len: f64,
    pub max_len: f64,
    #[serde(default)]
    pub layout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipEdit {
    pub id: i64,
    pub approved: bool,
    pub layout: String,
    pub camera: Rect,
    pub screen: Rect,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
    #[serde(default)]
    pub keys: Vec<AreaKey>,
    #[serde(default)]
    pub fit: AreaFit,
    #[serde(default)]
    pub caption: Option<CaptionAdjust>,
    #[serde(default)]
    pub cap_keys: Vec<CaptionKey>,
    #[serde(default = "default_split")]
    pub split: f64,
    #[serde(default)]
    pub word_edits: Vec<WordEdit>,
    #[serde(default)]
    pub cuts: Vec<Cut>,
    #[serde(default)]
    pub smooth: bool,
    #[serde(default = "default_smooth_secs")]
    pub smooth_secs: f64,
    #[serde(default)]
    pub curve: Option<Ease>,
    /// "blur" | "soft" | "black" | "file"
    #[serde(default = "default_bg_mode")]
    pub bg_mode: String,
    #[serde(default)]
    pub bg_path: Option<String>,
    /// Marca d'água da aba Modelos (None = sem marca).
    #[serde(default)]
    pub watermark_id: Option<i64>,
}

fn default_bg_mode() -> String {
    "blur".into()
}

fn default_split() -> f64 {
    DEFAULT_SPLIT
}

fn default_smooth_secs() -> f64 {
    SMOOTH_SECS
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderRequest {
    pub clips: Vec<ClipEdit>,
    #[serde(default)]
    pub caption_style_id: Option<i64>,
}

/// Sugestão inicial de câmera e tela (o usuário ajusta na tela de aprovação).
pub fn suggest_rects(layout: &str, w: i64, h: i64) -> (Rect, Rect) {
    let ar = if w > 0 && h > 0 { w as f64 / h as f64 } else { 16.0 / 9.0 };
    // Faixa 9:16 central: tamanho certo para "só câmera".
    let vw = ((9.0 / 16.0) / ar).min(1.0);
    match layout {
        // Webcam costuma ficar num canto inferior; tela é o quadro inteiro.
        "vertical_split" | "vertical_split_cam" => (Rect { x: 0.68, y: 0.62, w: 0.3, h: 0.36 }, Rect::FULL),
        "horizontal" => (Rect::FULL, Rect::FULL),
        _ => (Rect { x: (1.0 - vw) / 2.0, y: 0.0, w: vw, h: 1.0 }, Rect::FULL),
    }
}

fn parse_rect(s: &Option<String>, fallback: Rect) -> Rect {
    s.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or(fallback)
}

static YTDLP: OnceLock<Option<PathBuf>> = OnceLock::new();

/// yt-dlp em FLAXY_YTDLP_DIR (empacotado) ou no PATH.
pub fn ytdlp() -> Option<&'static Path> {
    YTDLP
        .get_or_init(|| {
            let exe = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
            if let Ok(dir) = std::env::var("FLAXY_YTDLP_DIR") {
                let p = Path::new(&dir).join(exe);
                if p.is_file() {
                    return Some(p);
                }
            }
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file())
        })
        .as_deref()
}

#[derive(Clone)]
pub struct Clips {
    db: SqlitePool,
    cfg: Config,
    render_slot: Arc<Semaphore>,
}

impl Clips {
    pub async fn start(db: SqlitePool, cfg: Config) -> Result<Self> {
        let c = Self { db, cfg, render_slot: Arc::new(Semaphore::new(1)) };
        // Pipeline interrompido não tem como retomar no meio: marca erro.
        sqlx::query(
            "UPDATE clip_projects SET status='error', error_msg='Interrompido ao fechar o app. Tente de novo.' \
             WHERE status IN ('downloading','transcribing','analyzing')",
        )
        .execute(&c.db)
        .await?;
        sqlx::query("UPDATE clip_items SET status='queued', progress=0 WHERE status='processing'")
            .execute(&c.db)
            .await?;
        let pending: Vec<(i64,)> = sqlx::query_as("SELECT id FROM clip_items WHERE status='queued' ORDER BY id")
            .fetch_all(&c.db)
            .await?;
        for (id,) in pending {
            c.spawn_render(id);
        }
        // Bruto que não deu para apagar antes (arquivo preso): tenta de novo.
        let done: Vec<(i64,)> = sqlx::query_as("SELECT id FROM clip_projects WHERE source_path IS NOT NULL AND status='ready'")
            .fetch_all(&c.db)
            .await?;
        let me = c.clone();
        tokio::spawn(async move {
            for (id,) in done {
                me.prune_source(id).await;
            }
        });
        Ok(c)
    }

    /// Apaga o vídeo bruto baixado depois que os clipes aprovados foram
    /// cortados, para um vídeo de horas não ficar ocupando o disco. Só quando
    /// nada está na fila, nenhum corte falhou (o bruto fica para tentar de
    /// novo) e pelo menos um clipe saiu. Arquivo local escolhido pelo usuário
    /// nunca é apagado. Dá para baixar de novo com `restore_source`.
    async fn prune_source(&self, project_id: i64) {
        let Ok(Some(p)) = sqlx::query_as::<_, Project>("SELECT * FROM clip_projects WHERE id=?").bind(project_id).fetch_optional(&self.db).await else { return };
        let Some(src) = p.source_path.clone() else { return };
        if p.status != "ready" || !Path::new(&src).starts_with(self.cfg.uploads_dir()) {
            return;
        }
        let counts: std::result::Result<(i64, i64), sqlx::Error> = sqlx::query_as(
            "SELECT COALESCE(SUM(status IN ('queued','processing','error')), 0), COALESCE(SUM(status='done'), 0) FROM clip_items WHERE project_id=?",
        )
        .bind(project_id)
        .fetch_one(&self.db)
        .await;
        let Ok((busy, done)) = counts else { return };
        if busy > 0 || done == 0 {
            return;
        }
        // No Windows o arquivo pode estar aberto pela prévia: tenta algumas vezes.
        for i in 0..6 {
            match tokio::fs::remove_file(&src).await {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) if i == 5 => {
                    tracing::warn!("não deu para apagar o bruto {src}: {e}");
                    return;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
            }
        }
        tracing::info!("bruto apagado depois do corte: {src}");
        let _ = sqlx::query("UPDATE clip_projects SET source_path=NULL WHERE id=?").bind(project_id).execute(&self.db).await;
    }

    /// Clipe escolhido à mão na linha do tempo (já aprovado).
    pub async fn add_clip(&self, project_id: i64, start: f64, end: f64) -> Result<ProjectView> {
        let p = self.get(project_id).await?.project;
        if p.status != "ready" {
            return Err(Error::Invalid("Projeto ainda não está pronto".into()));
        }
        let dur = p.duration.max(1.0);
        let start = start.clamp(0.0, (dur - 1.0).max(0.0));
        let end = end.clamp(start + 1.0, dur.max(start + 1.0));
        let (cam, scr) = suggest_rects(&p.layout, p.width, p.height);
        sqlx::query(
            "INSERT INTO clip_items (project_id, start, end, score, title, reason, approved, layout, camera_rect, screen_rect) \
             VALUES (?, ?, ?, 0, ?, ?, 1, ?, ?, ?)",
        )
        .bind(project_id)
        .bind(start)
        .bind(end)
        .bind(format!("Clipe manual {}", {
            let t = start.max(0.0) as i64;
            format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
        }))
        .bind(MANUAL_REASON)
        .bind(&p.layout)
        .bind(serde_json::to_string(&cam)?)
        .bind(serde_json::to_string(&scr)?)
        .execute(&self.db)
        .await?;
        self.get(project_id).await
    }

    /// Apaga um clipe da lista (o arquivo cortado, se houver, fica).
    pub async fn remove_clip(&self, project_id: i64, clip_id: i64) -> Result<ProjectView> {
        sqlx::query("DELETE FROM clip_items WHERE id=? AND project_id=? AND status NOT IN ('queued','processing')")
            .bind(clip_id)
            .bind(project_id)
            .execute(&self.db)
            .await?;
        self.get(project_id).await
    }

    /// Tira da lista todas as sugestões da IA que ainda não foram cortadas,
    /// para o usuário marcar os próprios clipes. Clipes feitos à mão e os já
    /// cortados (ou cortando) ficam.
    pub async fn clear_suggestions(&self, project_id: i64) -> Result<ProjectView> {
        sqlx::query(&format!(
            "DELETE FROM clip_items WHERE project_id=? AND reason IS NOT '{MANUAL_REASON}' AND status NOT IN ('queued','processing','done')"
        ))
        .bind(project_id)
        .execute(&self.db)
        .await?;
        self.get(project_id).await
    }

    /// Baixa de novo o vídeo bruto apagado, para cortar ou editar mais clipes.
    /// As sugestões e edições continuam como estavam.
    pub async fn restore_source(&self, id: i64) -> Result<ProjectView> {
        let p = self.get(id).await?.project;
        if p.source_path.as_deref().is_some_and(|s| Path::new(s).is_file()) {
            return self.get(id).await;
        }
        if p.status != "ready" {
            return Err(Error::Invalid("Esse vídeo ainda está sendo preparado".into()));
        }
        self.set_status(id, "downloading", 0).await;
        let this = self.clone();
        tokio::spawn(async move {
            let res = async {
                let src = if Path::new(&p.url).is_file() { PathBuf::from(&p.url) } else { this.download(id, &p.url).await? };
                sqlx::query("UPDATE clip_projects SET source_path=?, status='ready', progress=100 WHERE id=?")
                    .bind(src.to_string_lossy().into_owned())
                    .bind(id)
                    .execute(&this.db)
                    .await?;
                Ok::<_, Error>(())
            }
            .await;
            if let Err(e) = res {
                tracing::warn!("baixar o bruto de novo ({id}) falhou: {e}");
                // Volta para pronto: os clipes cortados continuam valendo.
                let _ = sqlx::query("UPDATE clip_projects SET status='ready', progress=100 WHERE id=?").bind(id).execute(&this.db).await;
            }
        });
        self.get(id).await
    }

    pub async fn create(&self, p: NewProject) -> Result<ProjectView> {
        let url = p.url.trim().trim_matches('"').to_string();
        let local = Path::new(&url).is_file();
        if !local && (!(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048) {
            return Err(Error::Invalid("Cole um link http(s) válido ou escolha um arquivo".into()));
        }
        if !local && ytdlp().is_none() {
            return Err(Error::Invalid("yt-dlp não encontrado. Reinstale o FLAXY".into()));
        }
        let min = p.min_len.clamp(5.0, 600.0);
        let max = p.max_len.clamp(min + 5.0, 900.0);
        let layout = p.layout.filter(|l| LAYOUTS.contains(&l.as_str())).unwrap_or_else(|| "vertical_camera".into());
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO clip_projects (url, min_len, max_len, layout) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(&url)
        .bind(min)
        .bind(max)
        .bind(&layout)
        .fetch_one(&self.db)
        .await?;
        self.run(id);
        self.get(id).await
    }

    /// Roda de novo um projeto que falhou. Reaproveita o vídeo baixado e a
    /// transcrição quando já existem.
    pub async fn retry(&self, id: i64) -> Result<ProjectView> {
        let p = self.get(id).await?.project;
        if p.status != "error" {
            return Err(Error::Invalid("Esse vídeo não falhou".into()));
        }
        sqlx::query("DELETE FROM clip_items WHERE project_id=?").bind(id).execute(&self.db).await?;
        sqlx::query("UPDATE clip_projects SET status='downloading', progress=0, error_msg=NULL WHERE id=?")
            .bind(id)
            .execute(&self.db)
            .await?;
        self.run(id);
        self.get(id).await
    }

    fn run(&self, id: i64) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.pipeline(id).await {
                tracing::warn!("projeto de clipes {id} falhou: {e}");
                let _ = sqlx::query("UPDATE clip_projects SET status='error', error_msg=? WHERE id=?")
                    .bind(e.to_string())
                    .bind(id)
                    .execute(&this.db)
                    .await;
            }
        });
    }

    pub async fn list(&self) -> Result<Vec<Project>> {
        Ok(sqlx::query_as("SELECT * FROM clip_projects ORDER BY id DESC LIMIT 100").fetch_all(&self.db).await?)
    }

    pub async fn get(&self, id: i64) -> Result<ProjectView> {
        let project: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?")
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound("Projeto de clipes"))?;
        let rows: Vec<ClipRow> = sqlx::query_as("SELECT * FROM clip_items WHERE project_id=? ORDER BY score DESC, id")
            .bind(id)
            .fetch_all(&self.db)
            .await?;
        let clips = rows
            .into_iter()
            .map(|row| {
                let (cam, scr) = suggest_rects(&row.layout, project.width, project.height);
                Clip {
                    camera: parse_rect(&row.camera_rect, cam),
                    screen: parse_rect(&row.screen_rect, scr),
                    keys: parse_keys(&row.area_keys),
                    fit: AreaFit::from_bits(row.area_fit).with_places(row.area_place.as_deref()),
                    caption: parse_caption(&row.caption_adjust),
                    cap_keys: parse_cap_keys(&row.caption_keys),
                    word_edits: parse_json(&row.word_edits),
                    cuts: parse_json(&row.cuts),
                    curve: parse_curve(&row.smooth_curve),
                    bg: ClipBg::from_parts(&row.bg_mode, row.bg_path.as_deref()),
                    row,
                }
            })
            .collect();
        Ok(ProjectView { project, clips })
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let p = self.get(id).await?;
        sqlx::query("DELETE FROM clip_projects WHERE id=?").bind(id).execute(&self.db).await?;
        // Só apaga o que o FLAXY baixou; arquivo local do usuário fica.
        if let Some(src) = p.project.source_path {
            if Path::new(&src).starts_with(self.cfg.uploads_dir()) {
                let _ = tokio::fs::remove_file(src).await;
            }
        }
        Ok(())
    }

    async fn set_status(&self, id: i64, status: &str, progress: i64) {
        let _ = sqlx::query("UPDATE clip_projects SET status=?, progress=? WHERE id=?")
            .bind(status)
            .bind(progress)
            .bind(id)
            .execute(&self.db)
            .await;
    }

    async fn pipeline(&self, id: i64) -> Result<()> {
        let p: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?").bind(id).fetch_one(&self.db).await?;

        // 1. Download (ou arquivo local escolhido pelo usuário)
        let kept = p.source_path.as_deref().map(PathBuf::from).filter(|s| s.is_file());
        let src = if let Some(s) = kept {
            s
        } else if Path::new(&p.url).is_file() {
            let title = Path::new(&p.url).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            sqlx::query("UPDATE clip_projects SET title=? WHERE id=?").bind(title).bind(id).execute(&self.db).await?;
            PathBuf::from(&p.url)
        } else {
            self.download(id, &p.url).await?
        };
        let probe = media::probe(&src).await;
        let (w, h) = media::dimensions(&src).await;
        sqlx::query("UPDATE clip_projects SET source_path=?, duration=?, width=?, height=? WHERE id=?")
            .bind(src.to_string_lossy().into_owned())
            .bind(probe.duration)
            .bind(w)
            .bind(h)
            .bind(id)
            .execute(&self.db)
            .await?;

        // 2. Transcrição (gravada em `transcripts` para servir de legenda depois)
        self.set_status(id, "transcribing", 0).await;
        let settings = ai::load(&self.db).await?;
        let models = self.cfg.storage_dir.join("models");
        let ctx = TranscribeCtx { settings: &settings, models_dir: &models, work_dir: &self.cfg.work_dir };
        let kept: Option<Vec<Word>> = match p.transcript_id {
            Some(tid) => sqlx::query_as::<_, (String,)>("SELECT words FROM transcripts WHERE id=?")
                .bind(tid)
                .fetch_optional(&self.db)
                .await?
                .and_then(|(w,)| serde_json::from_str(&w).ok())
                .filter(|w: &Vec<Word>| !w.is_empty()),
            None => None,
        };
        let words = if let Some(w) = kept {
            w
        } else {
            self.transcribe(id, &ctx, &src, &settings).await?
        };

        // 3. Análise de retenção
        self.set_status(id, "analyzing", 0).await;
        let suggestions = analyze(&settings, &words, p.min_len, p.max_len, probe.duration).await?;
        if suggestions.is_empty() {
            return Err(Error::Invalid("A IA não encontrou trechos dentro da duração escolhida".into()));
        }
        let (cam, scr) = suggest_rects(&p.layout, w, h);
        for s in suggestions {
            sqlx::query(
                "INSERT INTO clip_items (project_id, start, end, score, title, reason, layout, camera_rect, screen_rect)                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(s.start)
            .bind(s.end)
            .bind(s.score)
            .bind(&s.title)
            .bind(&s.reason)
            .bind(&p.layout)
            .bind(serde_json::to_string(&cam)?)
            .bind(serde_json::to_string(&scr)?)
            .execute(&self.db)
            .await?;
        }
        self.set_status(id, "ready", 100).await;
        Ok(())
    }

    async fn transcribe(&self, id: i64, ctx: &TranscribeCtx<'_>, src: &Path, settings: &ai::AiSettings) -> Result<Vec<Word>> {
        let words = ai::transcribe(ctx, src).await?;
        if words.is_empty() {
            return Err(Error::Invalid("Nenhuma fala encontrada no vídeo".into()));
        }
        let (tid,): (i64,) = sqlx::query_as(
            "INSERT INTO transcripts (source_path, provider, language, status, words) VALUES (?, ?, ?, 'done', ?) RETURNING id",
        )
        .bind(src.to_string_lossy().into_owned())
        .bind(&settings.transcriber)
        .bind(&settings.language)
        .bind(serde_json::to_string(&words)?)
        .fetch_one(&self.db)
        .await?;
        sqlx::query("UPDATE clip_projects SET transcript_id=? WHERE id=?").bind(tid).bind(id).execute(&self.db).await?;
        Ok(words)
    }

    async fn download(&self, id: i64, url: &str) -> Result<PathBuf> {
        let bin = ytdlp().ok_or_else(|| Error::Invalid("yt-dlp não encontrado".into()))?;
        let dir = self.cfg.uploads_dir();
        let template = dir.join(format!("clip_src_{id}.%(ext)s"));
        let mut cmd = crate::media::command(bin);
        cmd.args([
            "--no-playlist", "--newline", "--encoding", "utf-8", "--no-part", "--no-mtime",
            "-f", "bv*[height<=1080][ext=mp4]+ba[ext=m4a]/bv*[height<=1080]+ba/b[height<=1080]/b",
            "--merge-output-format", "mp4",
            "--progress-template", "download:flaxy %(progress._percent_str)s",
            "--print", "after_move:filepath",
            "--print", "before_dl:title:%(title)s",
            "-o",
        ])
        .arg(&template);
        if let Some(ff) = media::ffmpeg().and_then(|p| p.parent()) {
            cmd.arg("--ffmpeg-location").arg(ff);
        }
        // "--" impede que o link seja lido como opção.
        let mut child = cmd
            .arg("--")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stderr = child.stderr.take().expect("stderr");
        let err_task = tokio::spawn(async move {
            let mut lines = LossyLines::new(stderr);
            let mut last = Vec::new();
            while let Some(l) = lines.next().await {
                if l.contains("ERROR") || last.len() < 3 {
                    last.push(l);
                }
                if last.len() > 6 {
                    last.remove(0);
                }
            }
            last.join(" | ")
        });
        let mut out = LossyLines::new(child.stdout.take().expect("stdout"));
        let mut file: Option<String> = None;
        let mut last_pct = -1;
        while let Some(line) = out.next().await {
            if let Some(p) = line.strip_prefix("flaxy ") {
                if let Ok(v) = p.trim().trim_end_matches('%').trim().parse::<f64>() {
                    let pct = v as i64;
                    if pct != last_pct {
                        last_pct = pct;
                        self.set_status(id, "downloading", pct).await;
                    }
                }
            } else if let Some(t) = line.strip_prefix("title:") {
                let _ = sqlx::query("UPDATE clip_projects SET title=? WHERE id=?")
                    .bind(t.trim().chars().take(200).collect::<String>())
                    .bind(id)
                    .execute(&self.db)
                    .await;
            } else if !line.trim().is_empty() && Path::new(line.trim()).is_file() {
                file = Some(line.trim().to_string());
            }
        }
        let status = child.wait().await?;
        let err = err_task.await.unwrap_or_default();
        match file {
            Some(f) if status.success() => Ok(PathBuf::from(f)),
            _ => Err(Error::Invalid(format!("Falha no download: {err}"))),
        }
    }

    /// Grava as escolhas do usuário e enfileira os aprovados.
    pub async fn render(&self, project_id: i64, req: RenderRequest) -> Result<ProjectView> {
        let pv = self.get(project_id).await?;
        if pv.project.status != "ready" {
            return Err(Error::Invalid("Projeto ainda não está pronto".into()));
        }
        let has_src = pv.project.source_path.as_deref().is_some_and(|s| Path::new(s).is_file());
        if !has_src && req.clips.iter().any(|e| e.approved) {
            return Err(Error::Invalid("O vídeo bruto foi apagado depois do corte para liberar espaço. Use \"Baixar de novo\" para cortar mais".into()));
        }
        let dur = pv.project.duration;
        let mut queued = Vec::new();
        for e in &req.clips {
            let Some(cur) = pv.clips.iter().find(|c| c.row.id == e.id) else { continue };
            let layout = if LAYOUTS.contains(&e.layout.as_str()) { e.layout.clone() } else { cur.row.layout.clone() };
            let start = e.start.unwrap_or(cur.row.start).clamp(0.0, dur.max(0.0));
            let end = e.end.unwrap_or(cur.row.end).clamp(start + 1.0, dur.max(start + 1.0));
            let busy = cur.row.status == "queued" || cur.row.status == "processing";
            let enqueue = e.approved && !busy;
            let keys: Vec<AreaKey> = e
                .keys
                .iter()
                .filter(|k| k.t.is_finite())
                .map(|k| AreaKey { t: k.t.clamp(0.0, dur.max(0.0)), camera: k.camera.clamped(), screen: k.screen.clamped(), layout: k.layout })
                .collect();
            let cuts: Vec<Cut> = e.cuts.iter().copied().filter(|c| c.a.is_finite() && c.b.is_finite() && c.b > c.a).collect();
            let bg = ClipBg::from_parts(&e.bg_mode, e.bg_path.as_deref());
            let cap_keys: Vec<CaptionKey> = e.cap_keys.iter().filter(|k| k.t.is_finite()).map(|k| CaptionKey { t: k.t.clamp(0.0, dur.max(0.0)), adjust: k.adjust.clone() }).collect();
            sqlx::query(
                "UPDATE clip_items SET approved=?, layout=?, camera_rect=?, screen_rect=?, start=?, end=?, caption_style_id=?, \
                 area_keys=?, area_fit=?, caption_adjust=?, caption_keys=?, split=?, word_edits=?, cuts=?, smooth=?, smooth_secs=?, smooth_curve=?, bg_mode=?, bg_path=?, watermark_id=?, area_place=?, \
                 status=CASE WHEN ? THEN 'queued' ELSE status END, progress=CASE WHEN ? THEN 0 ELSE progress END, \
                 error_msg=CASE WHEN ? THEN NULL ELSE error_msg END WHERE id=? AND project_id=?",
            )
            .bind(e.approved)
            .bind(&layout)
            .bind(serde_json::to_string(&e.camera.clamped())?)
            .bind(serde_json::to_string(&e.screen.clamped())?)
            .bind(start)
            .bind(end)
            .bind(req.caption_style_id)
            .bind(if keys.is_empty() { None } else { Some(serde_json::to_string(&keys)?) })
            .bind(e.fit.bits())
            .bind(e.caption.as_ref().map(serde_json::to_string).transpose()?)
            .bind(if cap_keys.is_empty() { None } else { Some(serde_json::to_string(&cap_keys)?) })
            .bind(if e.split.is_finite() { e.split.clamp(0.2, 0.8) } else { DEFAULT_SPLIT })
            .bind(if e.word_edits.is_empty() { None } else { Some(serde_json::to_string(&e.word_edits)?) })
            .bind(if cuts.is_empty() { None } else { Some(serde_json::to_string(&cuts)?) })
            .bind(e.smooth)
            .bind(if e.smooth_secs.is_finite() { e.smooth_secs.clamp(SMOOTH_MIN, SMOOTH_MAX) } else { SMOOTH_SECS })
            .bind(e.curve.map(|c| serde_json::to_string(&c.clamped())).transpose()?)
            .bind(bg.mode())
            .bind(bg.path().map(str::to_string))
            .bind(e.watermark_id)
            .bind(e.fit.places_json())
            .bind(enqueue)
            .bind(enqueue)
            .bind(enqueue)
            .bind(e.id)
            .bind(project_id)
            .execute(&self.db)
            .await?;
            if enqueue {
                queued.push(e.id);
            }
        }
        for id in queued {
            self.spawn_render(id);
        }
        self.get(project_id).await
    }

    fn spawn_render(&self, clip_id: i64) {
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = this.render_slot.clone().acquire_owned().await;
            if let Err(e) = this.render_one(clip_id).await {
                tracing::warn!("clipe {clip_id} falhou: {e}");
                let _ = sqlx::query("UPDATE clip_items SET status='error', error_msg=? WHERE id=?")
                    .bind(e.to_string())
                    .bind(clip_id)
                    .execute(&this.db)
                    .await;
            }
            drop(_permit);
            if let Ok((pid,)) = sqlx::query_as::<_, (i64,)>("SELECT project_id FROM clip_items WHERE id=?").bind(clip_id).fetch_one(&this.db).await {
                this.prune_source(pid).await;
            }
        });
    }

    async fn render_one(&self, clip_id: i64) -> Result<()> {
        let row: ClipRow = sqlx::query_as("SELECT * FROM clip_items WHERE id=?")
            .bind(clip_id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(Error::NotFound("Clipe"))?;
        if row.status != "queued" {
            return Ok(());
        }
        let p: Project = sqlx::query_as("SELECT * FROM clip_projects WHERE id=?").bind(row.project_id).fetch_one(&self.db).await?;
        let src = p.source_path.clone().ok_or_else(|| Error::Invalid("Vídeo de origem sumiu".into()))?;
        let ff = media::ffmpeg().ok_or_else(|| Error::Ffmpeg("ffmpeg não encontrado".into()))?;
        sqlx::query("UPDATE clip_items SET status='processing', progress=0 WHERE id=?").bind(clip_id).execute(&self.db).await?;

        let (cam0, scr0) = suggest_rects(&row.layout, p.width, p.height);
        let cam = parse_rect(&row.camera_rect, cam0);
        let scr = parse_rect(&row.screen_rect, scr0);
        let len = (row.end - row.start).max(1.0);
        let cuts: Vec<Cut> = parse_json(&row.cuts);
        let kept = kept_ranges(row.start, row.end, &cuts);
        let out_len = kept_len(&kept).max(0.5);
        let (ow, oh) = if row.layout == "horizontal" { (1920, 1080) } else { (1080, 1920) };

        // Legenda opcional: palavras da transcrição do projeto, deslocadas pelo início do clipe.
        let mut subs: Option<PathBuf> = None;
        if let (Some(style_id), Some(tid)) = (row.caption_style_id, p.transcript_id) {
            let style = captions::get(&self.db, style_id).await?;
            // Um estilo por trecho (trocas de posição/tamanho ao longo do clipe).
            let parts: Vec<(f64, captions::CaptionStyle)> = caption_parts(row.start, row.end, parse_caption(&row.caption_adjust), &parse_cap_keys(&row.caption_keys))
                .into_iter()
                .map(|(a, adj)| {
                    let mut st = style.style.clone();
                    if let Some(adj) = adj {
                        adj.apply(&mut st);
                    }
                    (out_time(row.start + a, &kept), st)
                })
                .collect();
            let (words,): (String,) = sqlx::query_as("SELECT words FROM transcripts WHERE id=?").bind(tid).fetch_one(&self.db).await?;
            let words: Vec<Word> = serde_json::from_str(&words)?;
            let edits: Vec<WordEdit> = parse_json(&row.word_edits);
            let shifted = clip_words(&words, &edits, &kept);
            let dir = self.cfg.work_dir.join("subs");
            tokio::fs::create_dir_all(&dir).await?;
            let path = dir.join(format!("clip_{clip_id}.ass"));
            tokio::fs::write(&path, captions::build_ass_parts(&shifted, &parts, ow, oh)).await?;
            subs = Some(path);
        }
        // Marca d'água: outra legenda ASS, parada, por baixo da legenda falada.
        let mut wm: Option<PathBuf> = None;
        if let Some(wid) = row.watermark_id {
            if let Ok(w) = crate::watermarks::get(&self.db, wid).await {
                if w.items.iter().any(|t| !t.text.trim().is_empty()) {
                    let dir = self.cfg.work_dir.join("subs");
                    tokio::fs::create_dir_all(&dir).await?;
                    let path = dir.join(format!("wm_{clip_id}.ass"));
                    tokio::fs::write(&path, crate::watermarks::build_ass(&w.items, ow, oh, out_len)).await?;
                    wm = Some(path);
                }
            }
        }
        let ass: String = [&wm, &subs]
            .iter()
            .filter_map(|p| p.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()))
            .map(|n| format!(",ass={n}"))
            .collect();

        let segments = clip_plan(row.start, row.end, &row.layout, cam, scr, &parse_keys(&row.area_keys), &cuts, row.smooth, row.smooth_secs, parse_curve(&row.smooth_curve));
        // Fundo das áreas encaixadas; arquivo que sumiu volta ao desfocado.
        let bg = match ClipBg::from_parts(&row.bg_mode, row.bg_path.as_deref()) {
            ClipBg::File(f) if !Path::new(&f).exists() => ClipBg::Blur,
            b => b,
        };
        let mut graph = clip_graph_bg(&row.layout, &segments, AreaFit::from_bits(row.area_fit).with_places(row.area_place.as_deref()), row.split, ow, oh, &ass, len, &bg);
        // O grafo só lê a entrada 1 quando alguma área encaixa sobre o arquivo.
        let bg_input = bg.path().filter(|_| graph.starts_with("[1:v]")).map(str::to_string);
        // Com cortes, o áudio também perde os trechos cortados.
        let cut_audio = kept.len() > 1 || kept.first().is_some_and(|(a, b)| *a > row.start + 0.01 || *b < row.end - 0.01);
        let audio_map = if cut_audio && media::probe(Path::new(&src)).await.has_audio {
            let rel: Vec<(f64, f64)> = kept.iter().map(|(a, b)| (a - row.start, b - row.start)).collect();
            graph.push(';');
            graph.push_str(&audio_graph(&rel));
            "[aout]"
        } else {
            "0:a?"
        };

        let enc = media::encoder().await;
        let stem: String = row
            .title
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .take(48)
            .collect();
        let out = self.cfg.output_dir().join(format!(
            "clipe_{}_{}_{}.mp4",
            if stem.trim_matches('_').is_empty() { "sem_titulo".to_string() } else { stem },
            p.id,
            clip_id
        ));
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
        args.extend(["-ss".into(), format!("{:.3}", row.start), "-t".into(), format!("{len:.3}"), "-i".into(), src]);
        if let Some(f) = bg_input {
            // Imagem vira um vídeo do mesmo quadro; vídeo curto repete até o fim.
            if ClipBg::is_image(&f) {
                args.extend(["-loop".into(), "1".into(), "-framerate".into(), "30".into()]);
            } else {
                args.extend(["-stream_loop".into(), "-1".into()]);
            }
            args.extend(["-t".into(), format!("{len:.3}"), "-i".into(), f]);
        }
        args.extend(["-filter_complex".into(), graph, "-map".into(), "[out]".into(), "-map".into(), audio_map.into()]);
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), "-c:v".into(), enc.name.into()]);
        if let Some(pr) = enc.preset {
            args.extend(["-preset".into(), pr.into()]);
        }
        args.extend(["-b:v".into(), "10M".into(), "-movflags".into(), "+faststart".into(), out.to_string_lossy().into_owned()]);

        let db = self.db.clone();
        let log = self.cfg.logs_dir().join(format!("clip_{clip_id}.log"));
        let cwd = subs.as_ref().or(wm.as_ref()).and_then(|p| p.parent()).map(Path::to_path_buf);
        run_ffmpeg(ff, &args, cwd.as_deref(), out_len, Some(&log), move |pct| {
            let db = db.clone();
            tokio::spawn(async move {
                let _ = sqlx::query("UPDATE clip_items SET progress=? WHERE id=?").bind(pct).bind(clip_id).execute(&db).await;
            });
        })
        .await?;
        sqlx::query("UPDATE clip_items SET status='done', progress=100, output_path=? WHERE id=?")
            .bind(out.to_string_lossy().into_owned())
            .bind(clip_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

/// Linhas de um processo sem exigir UTF-8 válido. No Windows o yt-dlp pode
/// escrever na página de código do console; um acento no título não pode
/// derrubar o download.
struct LossyLines<R> {
    inner: BufReader<R>,
    buf: Vec<u8>,
}

impl<R: tokio::io::AsyncRead + Unpin> LossyLines<R> {
    fn new(r: R) -> Self {
        Self { inner: BufReader::new(r), buf: Vec::with_capacity(512) }
    }

    async fn next(&mut self) -> Option<String> {
        self.buf.clear();
        match self.inner.read_until(b'\n', &mut self.buf).await {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(String::from_utf8_lossy(&self.buf).trim_end_matches(['\r', '\n']).to_string()),
        }
    }
}

// ------------------------------------------------------------- análise

#[derive(Debug, Clone, Deserialize)]
pub struct Suggestion {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub reason: String,
}

/// Transcrição em linhas com tempo: "[12.3-18.9] texto". Corta por pausa ou frase.
pub fn timed_lines(words: &[Word]) -> String {
    let mut out = String::new();
    let mut line: Vec<&Word> = Vec::new();
    let flush = |line: &mut Vec<&Word>, out: &mut String| {
        if let (Some(a), Some(b)) = (line.first(), line.last()) {
            let text: Vec<&str> = line.iter().map(|w| w.text.as_str()).collect();
            out.push_str(&format!("[{:.1}-{:.1}] {}\n", a.start, b.end, text.join(" ")));
        }
        line.clear();
    };
    for w in words {
        let gap = line.last().map(|l| w.start - l.end).unwrap_or(0.0);
        if !line.is_empty() && (gap > 0.8 || line.len() >= 24) {
            flush(&mut line, &mut out);
        }
        line.push(w);
        if w.text.ends_with(['.', '!', '?']) {
            flush(&mut line, &mut out);
        }
    }
    flush(&mut line, &mut out);
    out
}

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["clips"],
        "properties": {
            "clips": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["start", "end", "title", "score", "reason"],
                    "properties": {
                        "start": { "type": "number" },
                        "end": { "type": "number" },
                        "title": { "type": "string" },
                        "score": { "type": "integer" },
                        "reason": { "type": "string" }
                    }
                }
            }
        }
    })
}

const SYSTEM: &str = "Você é editor de vídeos curtos (Reels, Shorts, TikTok) especializado em retenção. \
Recebe a transcrição de um vídeo longo com tempos em segundos e escolhe os trechos com maior chance de \
prender quem assiste do começo ao fim, sem depender do resto do vídeo.\n\
Critérios: (1) gancho forte nos primeiros 3 segundos: pergunta, afirmação surpreendente, conflito ou promessa; \
(2) uma ideia completa com desfecho ou virada; (3) entendível sozinho, sem citar algo dito antes; \
(4) energia e emoção na fala.\n\
Regras: comece e termine nos limites das linhas da transcrição (nunca no meio de uma frase); \
respeite a faixa de duração pedida; trechos não podem se sobrepor; \
score de 0 a 100 é a chance de retenção; title é um título curto e chamativo no idioma do vídeo; \
reason é uma frase em português explicando por que o trecho prende.";

/// Pede os trechos ao LLM e ajusta os limites às palavras reais.
pub async fn analyze(s: &ai::AiSettings, words: &[Word], min: f64, max: f64, duration: f64) -> Result<Vec<Suggestion>> {
    let user = format!(
        "Duração de cada clipe: entre {min:.0} e {max:.0} segundos.\nSugira até {MAX_SUGGESTIONS} clipes, \
         do melhor para o pior.\n\nTranscrição:\n{}",
        timed_lines(words)
    );
    let v = llm::json_call(s, SYSTEM, &user, &schema()).await?;
    let raw: Vec<Suggestion> = serde_json::from_value(v["clips"].clone()).unwrap_or_default();
    Ok(refine(raw, words, min, max, duration))
}

/// Encaixa início/fim nas palavras, aplica a faixa de duração, remove
/// sobreposições e mantém os melhores.
pub fn refine(mut raw: Vec<Suggestion>, words: &[Word], min: f64, max: f64, duration: f64) -> Vec<Suggestion> {
    raw.sort_by(|a, b| b.score.cmp(&a.score));
    let mut out: Vec<Suggestion> = Vec::new();
    for mut s in raw {
        if !(s.start.is_finite() && s.end.is_finite()) || s.end <= s.start {
            continue;
        }
        // Início na palavra que começa mais perto; fim na palavra que termina mais perto.
        if let Some(w) = words.iter().min_by(|a, b| (a.start - s.start).abs().total_cmp(&(b.start - s.start).abs())) {
            s.start = w.start;
        }
        if let Some(w) = words.iter().min_by(|a, b| (a.end - s.end).abs().total_cmp(&(b.end - s.end).abs())) {
            s.end = w.end;
        }
        let len = s.end - s.start;
        // Tolerância de 15% para não descartar um bom trecho por poucos segundos.
        if len < min * 0.85 || len > max * 1.15 || s.start < 0.0 || (duration > 0.0 && s.end > duration + 0.5) {
            continue;
        }
        let overlaps = out.iter().any(|o| {
            let inter = (o.end.min(s.end) - o.start.max(s.start)).max(0.0);
            inter > 0.3 * len.min(o.end - o.start)
        });
        if overlaps {
            continue;
        }
        s.score = s.score.clamp(0, 100);
        s.title = s.title.trim().chars().take(120).collect();
        s.reason = s.reason.trim().chars().take(400).collect();
        out.push(s);
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 };
    const B: Rect = Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 };

    fn k(t: f64, c: Rect, s: Rect) -> AreaKey {
        AreaKey { t, camera: c, screen: s, layout: None }
    }

    #[test]
    fn split_cam_puts_camera_on_top() {
        let segs = area_segments(0.0, 10.0, "vertical_split_cam", A, B, &[]);
        let g = clip_graph("vertical_split_cam", &segs, AreaFit::default(), DEFAULT_SPLIT, 1080, 1920, "", 10.0);
        println!("{g}");
        // Câmera (A, x=0) em 1080x768 em cima, tela (B, x=0.5) em 1080x1152 embaixo.
        let cam = g.find("iw*0.00000,scale=1080:768").or_else(|| g.find("crop=iw*0.50000:ih*1.00000:iw*0.00000")).unwrap();
        let scr = g.find("iw*0.50000:ih*0.00000,scale=1080:1152").unwrap();
        assert!(cam < scr);
    }

    #[test]
    fn cuts_remap_time_and_words() {
        let cuts = [Cut { a: 12.0, b: 15.0 }, Cut { a: 25.0, b: 40.0 }];
        let kept = kept_ranges(10.0, 30.0, &cuts);
        assert_eq!(kept, vec![(10.0, 12.0), (15.0, 25.0)]);
        assert_eq!(kept_len(&kept), 12.0);
        assert_eq!(out_time(16.0, &kept), 3.0);
        assert_eq!(out_time(13.0, &kept), 2.0);
        let ws = vec![
            Word { text: "a".into(), start: 11.0, end: 11.5 },
            Word { text: "b".into(), start: 13.0, end: 13.5 },
            Word { text: "c".into(), start: 16.0, end: 16.5 },
        ];
        let out = clip_words(&ws, &[WordEdit { t: 16.0, text: "C!".into() }, WordEdit { t: 11.0, text: "".into() }], &kept);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].text.as_str(), out[0].start), ("C!", 3.0));
        // Grafo: trechos cortados + áudio.
        let plan = clip_plan(10.0, 30.0, "vertical_camera", A, B, &[], &cuts, false, SMOOTH_SECS, None);
        assert_eq!(plan.len(), 2);
        let g = clip_graph("vertical_camera", &plan, AreaFit::default(), DEFAULT_SPLIT, 1080, 1920, "", 20.0);
        assert!(g.contains("trim=start=5.000:end=15.000") && g.contains("concat=n=2"));
        assert!(audio_graph(&[(0.0, 2.0), (5.0, 15.0)]).contains("concat=n=2:v=0:a=1[aout]"));
    }

    #[test]
    fn pauses_become_cuts() {
        let ws = vec![
            Word { text: "a".into(), start: 0.2, end: 1.0 },
            Word { text: "b".into(), start: 3.0, end: 3.5 },
            Word { text: "c".into(), start: 3.6, end: 4.0 },
        ];
        let c = pause_cuts(&ws, 0.0, 6.0, 0.7);
        assert_eq!(c.len(), 2);
        assert!((c[0].a - 1.15).abs() < 1e-9 && (c[0].b - 2.85).abs() < 1e-9);
        assert!((c[1].a - 4.15).abs() < 1e-9);
    }

    #[test]
    fn ease_curves() {
        let lin = Ease::new(0.0, 0.0, 1.0, 1.0);
        for x in [0.1, 0.25, 0.5, 0.9] {
            assert!((lin.at(x) - x).abs() < 1e-5);
        }
        // CSS ease-in-out em 0,5 fica no meio; freia mais rápido que acelera.
        let io = Ease::new(0.42, 0.0, 0.58, 1.0);
        assert!((io.at(0.5) - 0.5).abs() < 1e-5);
        let (acc, brk) = (Ease::PRESETS[2].2.unwrap(), Ease::PRESETS[3].2.unwrap());
        assert!(acc.at(0.3) < 0.3 && brk.at(0.3) > 0.3);
        // Mola passa do ponto e termina nele.
        let mola = Ease::PRESETS[6].2.unwrap();
        assert!((0..100).any(|i| mola.at(i as f64 / 100.0) > 1.0));
        assert_eq!(mola.at(1.0), 1.0);
        // Com mola a área não sai do quadro.
        let segs = area_segments(0.0, 10.0, "vertical_camera", A, Rect::FULL, &[k(5.0, Rect { x: 0.7, y: 0.0, w: 0.3, h: 1.0 }, Rect::FULL)]);
        for s in smooth_segments(&segs, 1.0, Some(mola)) {
            assert!(s.camera.x >= 0.0 && s.camera.x + s.camera.w <= 1.0 + 1e-9);
        }
        assert_eq!(parse_curve(&Some(serde_json::to_string(&mola).unwrap())), Some(mola));
        assert_eq!(parse_curve(&None), None);
    }

    /// Grafo real com fundo em arquivo e deslize com mola, para rodar no ffmpeg à mão.
    #[test]
    #[ignore]
    fn print_bg_graph() {
        let segs = area_segments(0.0, 4.0, "vertical_split", A, Rect::FULL, &[k(2.0, B, Rect::FULL)]);
        let segs = smooth_segments(&segs, 0.8, Ease::PRESETS[6].2);
        let fit = AreaFit { camera: true, screen: true, ..Default::default() };
        println!("GRAPH {}", clip_graph_bg("vertical_split", &segs, fit, 0.5, 1080, 1920, "", 4.0, &ClipBg::File("x".into())));
    }

    #[test]
    fn fit_backgrounds() {
        let segs = area_segments(0.0, 10.0, "vertical_split", A, Rect::FULL, &[k(5.0, B, Rect::FULL)]);
        let fit = AreaFit { camera: true, screen: true, ..Default::default() };
        let blur = clip_graph_bg("vertical_split", &segs, fit, 0.5, 1080, 1920, "", 10.0, &ClipBg::Blur);
        assert!(blur.contains("boxblur=24:2") && !blur.contains("[1:v]"));
        let soft = clip_graph_bg("vertical_split", &segs, fit, 0.5, 1080, 1920, "", 10.0, &ClipBg::Soft);
        assert!(soft.contains("boxblur=6:1"));
        let black = clip_graph_bg("vertical_split", &segs, fit, 0.5, 1080, 1920, "", 10.0, &ClipBg::Black);
        assert!(black.contains(":black") && !black.contains("boxblur"));
        // Arquivo: 2 trechos × 2 áreas encaixadas = 4 cópias da entrada 1, cada
        // uma cortada no seu trecho.
        let file = clip_graph_bg("vertical_split", &segs, fit, 0.5, 1080, 1920, "", 10.0, &ClipBg::File("bg.png".into()));
        assert!(file.starts_with("[1:v]split=4[bgf0][bgf1][bgf2][bgf3];"));
        assert!(file.contains("[bgf2]trim=start=5.000:end=10.000"));
        assert_eq!(file.matches("shortest=1").count(), 4);
        // Sem área encaixada o arquivo nem entra.
        let none = clip_graph_bg("vertical_split", &segs, AreaFit::default(), 0.5, 1080, 1920, "", 10.0, &ClipBg::File("bg.png".into()));
        assert!(!none.contains("[1:v]"));
        assert_eq!(ClipBg::from_parts("file", None), ClipBg::Blur);
        assert_eq!(ClipBg::from_parts("file", Some("x.mp4")).path(), Some("x.mp4"));
        assert!(ClipBg::is_image("C:/a/B.PNG") && !ClipBg::is_image("b.mp4"));
    }

    #[test]
    fn smooth_slides_between_areas() {
        let segs = area_segments(0.0, 10.0, "vertical_camera", A, Rect::FULL, &[k(5.0, B, Rect::FULL)]);
        let sm = smooth_segments(&segs, SMOOTH_SECS, None);
        assert!(sm.len() > 3);
        assert_eq!(sm[0].camera, A);
        assert_eq!(sm.last().unwrap().camera, B);
        let mid = sm[1 + (sm.len() - 2) / 2].camera;
        assert!(mid.x > 0.1 && mid.x < 0.4);
        // Duração ajustável: 1,5 s anda até 6,5 s, quadro a quadro, sem tranco nas pontas.
        let long = smooth_segments(&segs, 1.5, None);
        assert!((long.last().unwrap().a - 6.5).abs() < 1e-9);
        assert_eq!(long.len() - 2, 45);
        assert!(ease(0.0) == 0.0 && ease(1.0) == 1.0 && (ease(0.5) - 0.5).abs() < 1e-12);
        assert!(ease(0.05) < 0.01);
        // Divisão ajustável: tela com 50%.
        let segs = area_segments(0.0, 5.0, "vertical_split", A, Rect::FULL, &[]);
        let g = clip_graph("vertical_split", &segs, AreaFit::default(), 0.5, 1080, 1920, "", 5.0);
        assert!(g.contains("scale=1080:960"));
    }

    #[test]
    fn caption_parts_follow_keys() {
        let adj = |p: &str| CaptionAdjust { position: p.into(), margin: 0.1, size: 80.0 };
        let keys = vec![
            CaptionKey { t: 15.0, adjust: adj("top") },
            CaptionKey { t: 5.0, adjust: adj("center") },
            CaptionKey { t: 40.0, adjust: adj("top") },
        ];
        let p = caption_parts(10.0, 30.0, None, &keys);
        assert_eq!(p.len(), 2);
        assert!(p[0].1.is_none());
        assert_eq!((p[1].0, p[1].1.as_ref().unwrap().position.as_str()), (5.0, "top"));
        assert_eq!(caption_at(None, &keys, 10.0, 20.0).unwrap().position, "top");
        assert!(caption_at(None, &keys, 10.0, 12.0).is_none());
    }

    #[test]
    fn segments_cut_at_keys() {
        // Clipe 100..130; trocas em 110 e 120, uma antes e uma depois do clipe.
        let keys = [k(120.0, A, A), k(110.0, B, B), k(90.0, B, B), k(140.0, B, B)];
        let s = area_segments(100.0, 130.0, "vertical_camera", A, Rect::FULL, &keys);
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].a, s[0].b, s[0].camera), (0.0, 10.0, A));
        assert_eq!((s[1].a, s[1].b, s[1].camera), (10.0, 20.0, B));
        assert_eq!((s[2].a, s[2].b, s[2].camera), (20.0, 30.0, A));
        // Troca no começo do clipe substitui a área inicial.
        let s = area_segments(100.0, 130.0, "vertical_camera", A, A, &[k(100.0, B, B)]);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].a, s[0].b, s[0].camera, s[0].screen), (0.0, 30.0, B, B));
    }

    #[test]
    fn segment_layouts() {
        let mut split = k(10.0, B, A);
        split.layout = Some(Vertical::Split);
        let s = area_segments(0.0, 20.0, "vertical_camera", A, Rect::FULL, &[split]);
        assert_eq!((s[0].layout, s[1].layout), ("vertical_camera", "vertical_split"));
        let g = clip_graph("vertical_camera", &s, AreaFit::default(), DEFAULT_SPLIT, 1080, 1920, "", 20.0);
        assert!(g.contains("vstack") && g.contains("concat=n=2"));
        // Horizontal ignora o modo do trecho.
        let s = area_segments(0.0, 20.0, "horizontal", A, Rect::FULL, &[split]);
        assert!(s.iter().all(|x| x.layout == "horizontal"));
        // Chave antiga (sem "layout") ainda lê.
        let old: AreaKey = serde_json::from_str(r#"{"t":1,"camera":{"x":0,"y":0,"w":1,"h":1},"screen":{"x":0,"y":0,"w":1,"h":1}}"#).unwrap();
        assert_eq!(old.layout, None);
    }

    #[test]
    fn graph_concat_and_fit() {
        let segs = area_segments(0.0, 20.0, "vertical_split", A, Rect::FULL, &[k(10.0, B, Rect::FULL)]);
        let g = clip_graph("vertical_split", &segs, AreaFit { camera: true, screen: false, ..Default::default() }, DEFAULT_SPLIT, 1080, 1920, ",ass=x.ass", 20.0);
        assert!(g.contains("concat=n=2:v=1:a=0[v]"));
        assert!(g.contains("trim=start=10.000:end=20.000"));
        assert!(g.contains("boxblur"));
        assert!(g.ends_with("[v]null,ass=x.ass,format=yuv420p[out]"));
        let g1 = clip_graph("vertical_camera", &segs[..1], AreaFit::default(), DEFAULT_SPLIT, 1080, 1920, "", 10.0);
        assert!(!g1.contains("concat") && !g1.contains("boxblur"));
        assert_eq!(AreaFit::from_bits(AreaFit { camera: true, screen: true, ..Default::default() }.bits()), AreaFit { camera: true, screen: true, ..Default::default() });
    }

    #[test]
    fn fit_place_moves_and_scales() {
        let segs = area_segments(0.0, 10.0, "vertical_camera", A, Rect::FULL, &[]);
        let top = Place { x: 0.5, y: 0.0, scale: 0.5 };
        let fit = AreaFit { camera: true, cam_place: top, ..Default::default() };
        let g = clip_graph("vertical_camera", &segs, fit, DEFAULT_SPLIT, 1080, 1920, "", 10.0);
        assert!(g.contains("scale=540:960:force_original_aspect_ratio=decrease"), "{g}");
        assert!(g.contains("overlay=(W-w)/2:(H-h)*0.0000"), "{g}");
        let black = clip_graph_bg("vertical_camera", &segs, fit, DEFAULT_SPLIT, 1080, 1920, "", 10.0, &ClipBg::Black);
        assert!(black.contains("pad=1080:1920:(ow-iw)/2:(oh-ih)*0.0000:black"), "{black}");
        // Centro continua com a expressão antiga.
        let c = clip_graph("vertical_camera", &segs, AreaFit { camera: true, ..Default::default() }, DEFAULT_SPLIT, 1080, 1920, "", 10.0);
        assert!(c.contains("overlay=(W-w)/2:(H-h)/2"));
        // Gravação: centro não grava nada; o resto volta igual.
        assert_eq!(AreaFit::default().places_json(), None);
        let back = AreaFit::from_bits(fit.bits()).with_places(fit.places_json().as_deref());
        assert_eq!(back, fit);
        assert_eq!(Place { x: 9.0, y: f64::NAN, scale: 0.0 }.clamped(), Place { x: 1.0, y: 0.5, scale: PLACE_SCALE_MIN });
    }

    #[test]
    #[ignore]
    fn print_full_graph() {
        let cuts = [Cut { a: 2.0, b: 3.0 }];
        let plan = clip_plan(0.0, 8.0, "vertical_split", A, Rect::FULL, &[k(4.0, B, Rect::FULL)], &cuts, true, SMOOTH_SECS, None);
        let kept = kept_ranges(0.0, 8.0, &cuts);
        println!("GRAPH {};{}", clip_graph("vertical_split", &plan, AreaFit::default(), 0.5, 1080, 1920, "", 8.0), audio_graph(&kept));
    }

    #[test]
    #[ignore]
    fn print_graph() {
        let mut split = k(3.0, B, A);
        split.layout = Some(Vertical::Split);
        let segs = area_segments(0.0, 6.0, "vertical_camera", A, Rect::FULL, &[split]);
        println!("{}", clip_graph("vertical_camera", &segs, AreaFit { camera: true, screen: false, ..Default::default() }, DEFAULT_SPLIT, 1080, 1920, "", 6.0));
        let segs = area_segments(0.0, 6.0, "vertical_split", A, Rect::FULL, &[k(3.0, B, A)]);
        println!("{}", clip_graph("vertical_split", &segs, AreaFit { camera: true, screen: false, ..Default::default() }, DEFAULT_SPLIT, 1080, 1920, "", 6.0));
    }

    fn words() -> Vec<Word> {
        (0..200).map(|i| Word { text: format!("w{i}"), start: i as f64 * 0.5, end: i as f64 * 0.5 + 0.4 }).collect()
    }

    #[test]
    fn refine_snaps_and_filters() {
        let raw = vec![
            Suggestion { start: 10.1, end: 40.2, title: "a".into(), score: 90, reason: String::new() },
            Suggestion { start: 12.0, end: 41.0, title: "sobreposto".into(), score: 80, reason: String::new() },
            Suggestion { start: 50.0, end: 52.0, title: "curto".into(), score: 70, reason: String::new() },
            Suggestion { start: 60.0, end: 95.0, title: "b".into(), score: 140, reason: String::new() },
        ];
        let r = refine(raw, &words(), 30.0, 60.0, 100.0);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].score, 100);
        assert!((r[1].start - 10.0).abs() < 1e-9);
        assert!((r[1].end - 40.4).abs() < 1e-9);
    }

    #[test]
    fn rect_crop_is_clamped() {
        let r = Rect { x: 0.9, y: -1.0, w: 0.5, h: 2.0 };
        assert_eq!(r.crop(), "crop=iw*0.10000:ih*1.00000:iw*0.90000:ih*0.00000");
    }

    #[test]
    fn timed_lines_split_on_sentence() {
        let w = vec![
            Word { text: "Oi.".into(), start: 0.0, end: 0.3 },
            Word { text: "Tudo".into(), start: 0.4, end: 0.6 },
            Word { text: "bem?".into(), start: 0.6, end: 0.9 },
        ];
        assert_eq!(timed_lines(&w), "[0.0-0.3] Oi.\n[0.4-0.9] Tudo bem?\n");
    }
}
