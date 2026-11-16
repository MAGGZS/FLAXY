//! Editor de um clipe de retenção: abre só o trecho escolhido.
//! - Início e fim ajustáveis (alças na linha do tempo, botões e atalhos I/O).
//! - Áreas livres (alças nos cantos e nas bordas), que podem trocar ao longo do
//!   clipe, com corte seco ou deslizando. Cada área preenche ou encaixa.
//! - Divisão ajustável nos modos tela + câmera.
//! - Cortes de dentro do clipe (à mão ou nas pausas da fala).
//! - Posição e tamanho da legenda só deste clipe, com trocas ao longo dele, e
//!   correção das palavras que a transcrição errou.
//! - Desfazer/refazer e atalhos de teclado.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::captions::{CaptionStyle, Style, Word};
use flaxy_core::watermarks::Watermark;
use flaxy_core::clips::{
    area_segments, caption_parts, kept_len, kept_ranges, pause_cuts, smooth_segments, AreaFit, ClipBg, Ease, Place, EASE_Y_MAX, EASE_Y_MIN, PLACE_SCALE_MIN, SMOOTH_MAX, SMOOTH_MIN, SMOOTH_SECS, AreaKey, CaptionAdjust, CaptionKey, Cut, Rect as FRect, Vertical, WordEdit,
};

use crate::editors::caption_preview;
use crate::format::timecode;
use crate::icons as ic;
use crate::media::{fit_rect, Player};
use crate::theme::*;
use crate::widgets::{self as w, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    Camera,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Grip {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Grip {
    const EDGES: [Grip; 8] = [Grip::NW, Grip::NE, Grip::SE, Grip::SW, Grip::N, Grip::E, Grip::S, Grip::W];

    fn has(self, c: char) -> bool {
        let s = match self {
            Grip::Move => "",
            Grip::N => "n",
            Grip::S => "s",
            Grip::E => "e",
            Grip::W => "w",
            Grip::NE => "ne",
            Grip::NW => "nw",
            Grip::SE => "se",
            Grip::SW => "sw",
        };
        s.contains(c)
    }

    fn point(self, r: Rect) -> Pos2 {
        let x = if self.has('w') { r.left() } else if self.has('e') { r.right() } else { r.center().x };
        let y = if self.has('n') { r.top() } else if self.has('s') { r.bottom() } else { r.center().y };
        egui::pos2(x, y)
    }

    fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match self {
            Grip::Move => Move,
            Grip::N | Grip::S => ResizeVertical,
            Grip::E | Grip::W => ResizeHorizontal,
            Grip::NE | Grip::SW => ResizeNeSw,
            Grip::NW | Grip::SE => ResizeNwSe,
        }
    }
}

/// Qual ponta do clipe está sendo arrastada na linha do tempo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum End {
    Start,
    Finish,
}

struct BoxDef {
    key: Key,
    label: &'static str,
    color: Color32,
    /// Largura/altura em px do espaço que a área ocupa na saída.
    slot: f32,
}

/// Trilho de ferramentas, painel da ferramenta e largura útil dentro dele.
const RAIL_W: f32 = 76.0;
const INSP_W: f32 = 332.0;
const SIDE_W: f32 = INSP_W - 40.0;
/// Coluna dos nomes das faixas da linha do tempo.
const GUTTER: f32 = 76.0;
/// Amarelo das marcas de edição (início/fim, travas, pontos da curva).
const MARK: Color32 = Color32::from_rgb(240, 190, 60);

/// Ferramenta aberta no painel da esquerda.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Frame,
    Motion,
    Trim,
    Caption,
    Text,
    Mark,
}

const TABS: [(Tab, &str, &str); 6] = [
    (Tab::Frame, ic::CROP, "Enquadrar"),
    (Tab::Motion, ic::LAYERS, "Trocas"),
    (Tab::Trim, ic::SCISSORS, "Trecho"),
    (Tab::Caption, ic::CAPTIONS, "Legenda"),
    (Tab::Text, ic::TYPE, "Texto"),
    (Tab::Mark, ic::IMAGE, "Marca"),
];

impl Tab {
    fn head(self) -> (&'static str, &'static str) {
        match self {
            Tab::Frame => ("Enquadrar", "Que parte do vídeo entra em cada espaço do clipe e onde ela fica"),
            Tab::Motion => ("Trocas de área", "Mude o enquadramento ao longo do clipe, com corte seco ou deslizando"),
            Tab::Trim => ("Trecho", "Início, fim e o que sai de dentro do clipe"),
            Tab::Caption => ("Legenda", "Modelo, lugar e tamanho da legenda"),
            Tab::Text => ("Texto da legenda", "Corrija ou apague palavras que a transcrição errou"),
            Tab::Mark => ("Marca d'água", "Textos translúcidos por cima do clipe"),
        }
    }
}

/// Espaço de cada área no quadro de saída `out`.
fn out_slots(out: Rect, layout: &str, split: f64) -> Vec<(Key, Rect)> {
    let split = split.clamp(0.2, 0.8) as f32;
    let cut = |y: f32| (Rect::from_min_max(out.min, egui::pos2(out.right(), y)), Rect::from_min_max(egui::pos2(out.left(), y), out.max));
    match layout {
        "vertical_split" => {
            let (a, b) = cut(out.top() + out.height() * split);
            vec![(Key::Screen, a), (Key::Camera, b)]
        }
        "vertical_split_cam" => {
            let (a, b) = cut(out.top() + out.height() * (1.0 - split));
            vec![(Key::Camera, a), (Key::Screen, b)]
        }
        "horizontal" => vec![],
        _ => vec![(Key::Camera, out)],
    }
}

/// Onde uma área encaixada de proporção `a` fica dentro de `dst` (igual ao
/// scale + overlay do corte).
fn placed_rect(dst: Rect, a: f32, p: Place) -> Rect {
    let full = fit_rect(dst, a);
    let size = full.size() * p.scale.clamp(PLACE_SCALE_MIN, 1.0) as f32;
    let free = dst.size() - size;
    Rect::from_min_size(dst.min + egui::vec2(free.x * p.x as f32, free.y * p.y as f32), size)
}

/// Proporção (largura/altura na tela) de um recorte `r` de uma fonte `src`.
fn area_aspect(r: FRect, src: f32) -> f32 {
    (r.w as f32 * src) / (r.h as f32).max(0.0001)
}

/// Grade 3×3 que escolhe o canto, a borda ou o centro do espaço.
fn anchor_pad(ui: &mut Ui, p: &mut Place) -> bool {
    let cell = 24.0;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(cell * 3.0 + 8.0, cell * 3.0 + 8.0), Sense::hover());
    ui.painter().rect(outer, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let mut changed = false;
    for row in 0..3 {
        for col in 0..3 {
            let (fx, fy) = (col as f64 / 2.0, row as f64 / 2.0);
            let r = Rect::from_min_size(outer.min + egui::vec2(4.0 + col as f32 * cell, 4.0 + row as f32 * cell), egui::vec2(cell, cell));
            let resp = ui.interact(r, ui.auto_id_with(("anchor", row, col)), Sense::click());
            let on = (p.x - fx).abs() < 0.01 && (p.y - fy).abs() < 0.01;
            if resp.hovered() {
                ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), RAISED);
            }
            if on {
                ui.painter().rect_filled(Rect::from_center_size(r.center(), egui::vec2(10.0, 10.0)), CornerRadius::same(2), ACCENT);
            } else {
                ui.painter().circle_filled(r.center(), 2.0, if resp.hovered() { TEXT_2 } else { TEXT_3 });
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                (p.x, p.y) = (fx, fy);
                changed = true;
            }
        }
    }
    // Posição livre (arrastada na prévia): marca onde a área está.
    let free = !(0..3).any(|i| (p.x - i as f64 / 2.0).abs() < 0.01) || !(0..3).any(|i| (p.y - i as f64 / 2.0).abs() < 0.01);
    if free {
        let c = outer.min + egui::vec2(4.0 + cell / 2.0 + p.x as f32 * cell * 2.0, 4.0 + cell / 2.0 + p.y as f32 * cell * 2.0);
        ui.painter().circle_stroke(c, 5.0, Stroke::new(1.5, ACCENT_INK));
    }
    changed
}

/// Nome de uma faixa da linha do tempo, na coluna da esquerda.
fn track_label(ui: &mut Ui, h: f32, text: &str) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(GUTTER, h), Sense::hover());
    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, text, body(T_META), TEXT_3);
}

/// Título de um grupo dentro do painel.
fn group(ui: &mut Ui, title: &str) {
    ui.label(egui::RichText::new(title).font(strong(T_SEC)).color(TEXT));
    ui.add_space(8.0);
}

/// Separador entre grupos do painel.
fn gap(ui: &mut Ui) {
    ui.add_space(12.0);
    w::line(ui);
    ui.add_space(12.0);
}

/// Espaços de cada área na saída 1080×1920; `split` = parte da tela.
fn boxes(layout: &str, split: f64) -> Vec<BoxDef> {
    let sh = (1920.0 * split.clamp(0.2, 0.8)) as f32;
    match layout {
        "vertical_split" => vec![
            BoxDef { key: Key::Screen, label: "Tela · em cima", color: TEXT, slot: 1080.0 / sh },
            BoxDef { key: Key::Camera, label: "Câmera · embaixo", color: ACCENT, slot: 1080.0 / (1920.0 - sh) },
        ],
        "vertical_split_cam" => vec![
            BoxDef { key: Key::Camera, label: "Câmera · em cima", color: ACCENT, slot: 1080.0 / (1920.0 - sh) },
            BoxDef { key: Key::Screen, label: "Tela · embaixo", color: TEXT, slot: 1080.0 / sh },
        ],
        "vertical_camera" => vec![BoxDef { key: Key::Camera, label: "Câmera", color: ACCENT, slot: 9.0 / 16.0 }],
        _ => vec![],
    }
}

/// Parte visível (em fração da fonte) quando a área `r` preenche um espaço de
/// proporção `slot`: o FFmpeg corta o excesso, centralizado.
fn visible(r: FRect, src_aspect: f32, slot: f32) -> FRect {
    let a = (r.w as f32 * src_aspect) / (r.h as f32).max(0.0001);
    let (mut x, mut y, mut w, mut h) = (r.x, r.y, r.w, r.h);
    if a > slot {
        let nw = w * (slot / a) as f64;
        x += (w - nw) / 2.0;
        w = nw;
    } else {
        let nh = h * (a / slot) as f64;
        y += (h - nh) / 2.0;
        h = nh;
    }
    FRect { x, y, w, h }
}

/// Tudo que se edita num clipe (e o que o editor devolve ao salvar). Também
/// é o que vai para a pilha de desfazer.
#[derive(Clone, PartialEq, Debug)]
pub struct Edit {
    pub layout: String,
    pub start: f64,
    pub end: f64,
    pub camera: FRect,
    pub screen: FRect,
    pub keys: Vec<AreaKey>,
    pub fit: AreaFit,
    pub caption: Option<CaptionAdjust>,
    pub cap_keys: Vec<CaptionKey>,
    pub split: f64,
    pub word_edits: Vec<WordEdit>,
    pub cuts: Vec<Cut>,
    pub smooth: bool,
    /// Duração do deslize entre áreas (segundos).
    pub smooth_secs: f64,
    /// Curva de velocidade do deslize (None = suave padrão).
    pub curve: Option<Ease>,
    /// Fundo das áreas em Encaixar.
    pub bg: ClipBg,
    /// Marca d'água da aba Modelos (None = sem marca).
    pub watermark_id: Option<i64>,
}

/// Curva que aparece no gráfico quando a escolhida é a suave padrão
/// (parecida com ela), para dar onde pegar e ajustar.
const SOFT_LIKE: Ease = Ease::new(0.6, 0.0, 0.4, 1.0);

/// Distância (px na tela) em que uma área gruda na trava do centro.
const SNAP_PX: f32 = 8.0;

/// Lembra se as travas estão ligadas entre uma abertura e outra do editor.
fn snap_id() -> egui::Id {
    egui::Id::unique("clip-modal-snap")
}

/// Lembra a ferramenta aberta entre uma abertura e outra.
fn tab_id() -> egui::Id {
    egui::Id::unique("clip-modal-tab")
}

pub enum Outcome {
    Open,
    Save(Edit),
    /// Salva e copia modo, áreas, divisão, encaixe e legenda para os outros clipes.
    SaveAll(Edit),
    Close,
}

/// Dados do clipe para abrir o editor.
pub struct Init {
    pub clip_id: i64,
    pub title: String,
    pub src: std::path::PathBuf,
    pub edit: Edit,
    /// Modelos de legenda (aba Modelos) e o escolhido na tela de Clipes
    /// (None = sem legenda). Trocar aqui volta para a tela de Clipes ao salvar.
    pub styles: Vec<Style>,
    pub style_id: Option<i64>,
    /// Palavras da transcrição em volta do clipe (tempos da fonte).
    pub words: Vec<Word>,
    pub volume: f32,
    pub muted: bool,
    /// Fundos personalizados (aba Modelos › Fundos de clipe).
    pub backgrounds: Vec<flaxy_core::models::ClipBackground>,
    /// Marcas d'água (aba Modelos › Marcas d'água).
    pub watermarks: Vec<Watermark>,
}

pub struct ClipModal {
    pub clip_id: i64,
    title: String,
    s: Edit,
    committed: Edit,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    style: Option<CaptionStyle>,
    styles: Vec<Style>,
    style_id: Option<i64>,
    words: Vec<Word>,
    player: Player,
    drag: Option<(Key, Grip, FRect, Pos2)>,
    cap_drag: bool,
    end_drag: Option<End>,
    /// Faixa mostrada na linha do tempo (um pouco além do clipe).
    view: (f64, f64),
    /// Começo de um corte marcado, esperando o fim.
    cut_mark: Option<f64>,
    pause_gap: f64,
    /// Palavra em edição: início dela e o texto digitado.
    word_sel: Option<(f64, String)>,
    /// Travas no centro do quadro ligadas (Alt solta na hora).
    snap: bool,
    /// Travas em uso agora no arraste (vertical do centro, horizontal do centro).
    guides: (bool, bool),
    backgrounds: Vec<flaxy_core::models::ClipBackground>,
    watermarks: Vec<Watermark>,
    /// Ponto de controle da curva sendo arrastado (0 = primeiro, 1 = segundo).
    curve_drag: Option<usize>,
    bg_tex: Option<egui::TextureHandle>,
    tab: Tab,
    /// Área encaixada sendo arrastada na prévia: qual, retângulo no começo, ponto de partida.
    place_drag: Option<(Key, Rect, Pos2)>,
    /// Travas em uso no arraste da prévia (x, y), já em px da tela.
    place_guides: (Option<f32>, Option<f32>),
    /// Volume da reprodução (0..1) e mudo.
    volume: f32,
    muted: bool,
}

impl ClipModal {
    pub fn new(i: Init) -> Self {
        let mut player = Player::new(&i.src, i.edit.start, (960, 540)).with_audio();
        player.set_volume(if i.muted { 0.0 } else { i.volume });
        let mut s = i.edit;
        s.end = s.end.max(s.start + 1.0);
        s.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        s.cap_keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        if !s.split.is_finite() {
            s.split = flaxy_core::clips::DEFAULT_SPLIT;
        }
        s.smooth_secs = if s.smooth_secs.is_finite() { s.smooth_secs.clamp(SMOOTH_MIN, SMOOTH_MAX) } else { SMOOTH_SECS };
        let view = (s.start, s.end);
        Self {
            clip_id: i.clip_id,
            title: i.title,
            committed: s.clone(),
            s,
            undo: Vec::new(),
            redo: Vec::new(),
            style: i.style_id.and_then(|id| i.styles.iter().find(|s| s.row.id == id)).map(|s| s.style.clone()),
            styles: i.styles,
            style_id: i.style_id,
            words: i.words,
            player,
            drag: None,
            cap_drag: false,
            end_drag: None,
            view,
            cut_mark: None,
            pause_gap: 0.8,
            word_sel: None,
            snap: true,
            guides: (false, false),
            backgrounds: i.backgrounds,
            watermarks: i.watermarks,
            curve_drag: None,
            bg_tex: None,
            tab: Tab::Frame,
            place_drag: None,
            place_guides: (None, None),
            volume: i.volume,
            muted: i.muted,
        }
    }

    fn place_of(&self, k: Key) -> Place {
        if k == Key::Camera { self.s.fit.cam_place } else { self.s.fit.scr_place }
    }

    fn set_place(&mut self, k: Key, p: Place) {
        let p = p.clamped();
        if k == Key::Camera { self.s.fit.cam_place = p } else { self.s.fit.scr_place = p }
    }

    /// Instante da agulha, preso ao clipe (para editar áreas e legenda).
    fn now(&self) -> f64 {
        self.player.pos.clamp(self.s.start, self.s.end)
    }

    /// Duração do vídeo de origem (0 até o player ler).
    fn dur(&self) -> f64 {
        self.player.duration()
    }

    /// Troca que vale no instante `t` (a última que começou até ali, dentro do clipe).
    fn active(&self, t: f64) -> Option<usize> {
        self.s.keys.iter().rposition(|k| k.t <= t + 1e-6 && k.t >= self.s.start)
    }

    fn get(&self, k: Key, t: f64) -> FRect {
        let (c, s) = match self.active(t) {
            Some(i) => (self.s.keys[i].camera, self.s.keys[i].screen),
            None => (self.s.camera, self.s.screen),
        };
        if k == Key::Camera { c } else { s }
    }

    fn set(&mut self, k: Key, t: f64, r: FRect) {
        match (self.active(t), k) {
            (Some(i), Key::Camera) => self.s.keys[i].camera = r,
            (Some(i), Key::Screen) => self.s.keys[i].screen = r,
            (None, Key::Camera) => self.s.camera = r,
            (None, Key::Screen) => self.s.screen = r,
        }
    }

    /// Modo de recorte no instante `t` (o da troca ativa ou o do clipe).
    fn layout_at(&self, t: f64) -> String {
        if self.s.layout == "horizontal" {
            return self.s.layout.clone();
        }
        match self.active(t).and_then(|i| self.s.keys[i].layout) {
            Some(v) => v.id().to_string(),
            None => self.s.layout.clone(),
        }
    }

    fn set_layout_at(&mut self, t: f64, v: Vertical) {
        match self.active(t) {
            Some(i) => self.s.keys[i].layout = Some(v),
            None => self.s.layout = v.id().to_string(),
        }
    }

    /// Como o clipe sai no instante `t`: modo e áreas, já com o deslize.
    fn look_at(&self, t: f64) -> (String, FRect, FRect) {
        let segs = area_segments(self.s.start, self.s.end, &self.s.layout, self.s.camera, self.s.screen, &self.s.keys);
        let segs = if self.s.smooth { smooth_segments(&segs, self.s.smooth_secs, self.s.curve) } else { segs };
        let rel = t - self.s.start;
        match segs.iter().find(|s| rel >= s.a && rel < s.b).or(segs.last()) {
            Some(s) => (s.layout.to_string(), s.camera, s.screen),
            None => (self.s.layout.clone(), self.s.camera, self.s.screen),
        }
    }

    fn new_area_key(&mut self, t: f64) {
        let at = (t * 10.0).round() / 10.0;
        if self.s.keys.iter().any(|k| (k.t - at).abs() < 0.2) {
            return;
        }
        let k = AreaKey { t: at, camera: self.get(Key::Camera, t), screen: self.get(Key::Screen, t), layout: self.active(t).and_then(|i| self.s.keys[i].layout) };
        let i = self.s.keys.partition_point(|x| x.t < at);
        self.s.keys.insert(i, k);
    }

    /// Zoom numa área: `f` < 1 aproxima (recorte menor), > 1 afasta. Mantém o
    /// centro e a proporção, e não deixa a caixa sair do quadro.
    fn zoom(&mut self, k: Key, t: f64, f: f64) {
        let r = self.get(k, t);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let s = f.min(1.0 / r.w).min(1.0 / r.h);
        let (w, h) = ((r.w * s).max(0.03), (r.h * s).max(0.03));
        let x = (cx - w / 2.0).clamp(0.0, 1.0 - w);
        let y = (cy - h / 2.0).clamp(0.0, 1.0 - h);
        self.set(k, t, FRect { x, y, w, h });
    }

    /// Deixa a caixa no formato exato do espaço que ela ocupa no clipe (nada
    /// cortado, nada sobrando), com o mesmo centro e a maior área possível.
    fn fit_to_slot(&mut self, k: Key, t: f64, slot: f32) {
        let r = self.get(k, t);
        let src = self.player.aspect() as f64;
        let want = slot as f64 / src;
        let area = (r.w * r.h).max(0.0009);
        let mut h = (area / want).sqrt();
        let mut w = h * want;
        if w > 1.0 {
            w = 1.0;
            h = w / want;
        }
        if h > 1.0 {
            h = 1.0;
            w = h * want;
        }
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        self.set(k, t, FRect { x: (cx - w / 2.0).clamp(0.0, 1.0 - w), y: (cy - h / 2.0).clamp(0.0, 1.0 - h), w, h });
    }

    fn fit_of(&self, k: Key) -> bool {
        if k == Key::Camera { self.s.fit.camera } else { self.s.fit.screen }
    }

    /// Troca de legenda que vale no instante `t`.
    fn cap_active(&self, t: f64) -> Option<usize> {
        self.s.cap_keys.iter().rposition(|k| k.t <= t + 1e-6 && k.t >= self.s.start)
    }

    /// Ajuste da legenda no instante `t` (None = o do modelo).
    fn cap_at(&self, t: f64) -> Option<CaptionAdjust> {
        match self.cap_active(t) {
            Some(i) => Some(self.s.cap_keys[i].adjust.clone()),
            None => self.s.caption.clone(),
        }
    }

    /// Muda a legenda do trecho onde `t` está. None volta ao modelo.
    fn set_cap(&mut self, t: f64, a: Option<CaptionAdjust>) {
        match self.cap_active(t) {
            Some(i) => {
                let base = self.style.as_ref().map(CaptionAdjust::from_style);
                if let Some(v) = a.or(base) {
                    self.s.cap_keys[i].adjust = v;
                }
            }
            None => self.s.caption = a,
        }
    }

    fn style_now(&self) -> Option<CaptionStyle> {
        let mut st = self.style.clone()?;
        if let Some(a) = self.cap_at(self.now()) {
            a.apply(&mut st);
        }
        Some(st)
    }

    /// Palavras com as correções deste clipe (tempos da fonte).
    fn words_now(&self) -> Vec<Word> {
        self.words
            .iter()
            .filter_map(|w| match self.edit_of(w.start) {
                Some(e) if e.trim().is_empty() => None,
                Some(e) => Some(Word { text: e.to_string(), ..w.clone() }),
                None => Some(w.clone()),
            })
            .collect()
    }

    fn edit_of(&self, t: f64) -> Option<&str> {
        self.s.word_edits.iter().find(|e| (e.t - t).abs() < 0.02).map(|e| e.text.as_str())
    }

    fn set_word(&mut self, t: f64, text: Option<String>) {
        self.s.word_edits.retain(|e| (e.t - t).abs() >= 0.02);
        if let Some(text) = text {
            self.s.word_edits.push(WordEdit { t, text });
        }
    }

    /// Corte que contém o instante `t`.
    fn cut_at(&self, t: f64) -> Option<usize> {
        self.s.cuts.iter().position(|c| t >= c.a.min(c.b) && t < c.a.max(c.b))
    }

    fn add_cut(&mut self, a: f64, b: f64) {
        let (a, b) = (a.min(b).max(self.s.start), a.max(b).min(self.s.end));
        if b - a < 0.1 {
            return;
        }
        self.s.cuts.push(Cut { a, b });
        self.normalize_cuts();
    }

    /// Junta cortes que se encostam e ordena.
    fn normalize_cuts(&mut self) {
        let mut cs: Vec<Cut> = self.s.cuts.iter().map(|c| Cut { a: c.a.min(c.b), b: c.a.max(c.b) }).collect();
        cs.sort_by(|x, y| x.a.total_cmp(&y.a));
        let mut out: Vec<Cut> = Vec::new();
        for c in cs {
            match out.last_mut() {
                Some(l) if c.a <= l.b + 0.05 => l.b = l.b.max(c.b),
                _ => out.push(c),
            }
        }
        self.s.cuts = out;
    }

    fn set_start(&mut self, t: f64) {
        let d = self.dur();
        let max = self.s.end - 1.0;
        self.s.start = t.clamp(0.0, if d > 0.0 { max.min(d - 1.0) } else { max }).max(0.0);
    }

    fn set_end(&mut self, t: f64) {
        let d = self.dur();
        let e = t.max(self.s.start + 1.0);
        self.s.end = if d > 0.0 { e.min(d) } else { e };
    }

    // ------------------------------------------------------------ desfazer

    fn commit(&mut self) {
        if self.s != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, self.s.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn do_undo(&mut self) {
        self.commit();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.s, prev));
            self.committed = self.s.clone();
        }
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.s, next));
            self.committed = self.s.clone();
        }
    }

    fn seek(&mut self, t: f64) {
        self.player.pause();
        let d = self.dur();
        self.player.seek(if d > 0.0 { t.clamp(0.0, d) } else { t.max(0.0) });
    }

    /// Atalhos (só quando nenhum campo de texto tem o foco).
    fn shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if ctx.egui_wants_keyboard_input() {
            return false;
        }
        let mut toggle = false;
        let (undo, redo, l, r, shift, n, del, i, o, c, space, tk) = ctx.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND;
            let redo = i.consume_key(cmd | egui::Modifiers::SHIFT, egui::Key::Z) || i.consume_key(cmd, egui::Key::Y);
            let undo = i.consume_key(cmd, egui::Key::Z);
            (
                undo,
                redo,
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.shift,
                i.key_pressed(egui::Key::N),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::I),
                i.key_pressed(egui::Key::O),
                i.key_pressed(egui::Key::C),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::T),
            )
        });
        if tk {
            self.snap = !self.snap;
        }
        if undo {
            self.do_undo();
        }
        if redo {
            self.do_redo();
        }
        if l || r {
            let step = if shift { 1.0 } else { 1.0 / 30.0 };
            let t = self.player.pos + if r { step } else { -step };
            self.seek(t);
        }
        let t = self.now();
        if n && self.s.layout != "horizontal" {
            self.new_area_key(t);
        }
        if del {
            if let Some(k) = self.active(t) {
                self.s.keys.remove(k);
            }
        }
        if i {
            self.set_start(self.player.pos);
        }
        if o {
            self.set_end(self.player.pos);
        }
        if c {
            self.toggle_cut_mark();
        }
        if space {
            toggle = true;
        }
        toggle
    }

    fn toggle_cut_mark(&mut self) {
        let t = self.now();
        match self.cut_mark.take() {
            Some(a) => self.add_cut(a, t),
            None => self.cut_mark = Some(t),
        }
    }

    /// Lista nova da biblioteca de fundos (chega depois que o editor abriu).
    pub fn set_backgrounds(&mut self, b: Vec<flaxy_core::models::ClipBackground>) {
        self.backgrounds = b;
    }

    /// Lista nova de marcas d'água (chega depois que o editor abriu).
    pub fn set_watermarks(&mut self, w: Vec<Watermark>) {
        self.watermarks = w;
    }

    /// Textos da marca d'água escolhida, se ligada e ainda existir.
    fn wm_items(&self) -> Option<&[flaxy_core::watermarks::WmText]> {
        let id = self.s.watermark_id?;
        self.watermarks.iter().find(|w| w.row.id == id).map(|w| w.items.as_slice())
    }

    /// Checkbox da marca d'água e qual usar.
    fn watermark_panel(&mut self, ui: &mut Ui) {
        let mut on = self.s.watermark_id.is_some();
        let first = self.watermarks.first().map(|w| w.row.id);
        ui.add_enabled_ui(first.is_some() || on, |ui| {
            if ui.checkbox(&mut on, "Marca d'água").on_hover_text("Textos translúcidos por cima do clipe, nos lugares escolhidos em Modelos › Marcas d'água").changed() {
                self.s.watermark_id = if on { self.committed.watermark_id.filter(|id| self.watermarks.iter().any(|w| w.row.id == *id)).or(first) } else { None };
            }
        });
        if self.watermarks.is_empty() {
            ui.label(w::muted("Crie uma marca d'água em Modelos › Marcas d'água para usar aqui"));
            return;
        }
        if let Some(cur) = self.s.watermark_id {
            let name = self.watermarks.iter().find(|w| w.row.id == cur).map(|w| w.row.name.clone()).unwrap_or_else(|| "Marca apagada".into());
            let mut pick = None;
            egui::ComboBox::from_id_salt("clip-modal-wm").width(220.0).selected_text(name).show_ui(ui, |ui| {
                for w in &self.watermarks {
                    if ui.selectable_label(w.row.id == cur, &w.row.name).clicked() {
                        pick = Some(w.row.id);
                    }
                }
            });
            if let Some(id) = pick {
                self.s.watermark_id = Some(id);
            }
        }
    }

    /// Volume e mudo escolhidos no editor (para a tela de Clipes lembrar).
    pub fn volume(&self) -> (f32, bool) {
        (self.volume, self.muted)
    }

    /// Modelo de legenda escolhido no editor (None = sem legenda).
    pub fn style_id(&self) -> Option<i64> {
        self.style_id
    }

    /// Escolha do modelo de legenda (o mesmo da tela de Clipes).
    fn style_picker(&mut self, ui: &mut Ui) {
        let cur = self.style_id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.row.name.clone()).unwrap_or_else(|| "Sem legenda".into());
        let mut pick: Option<Option<i64>> = None;
        egui::ComboBox::from_id_salt("clip-modal-style").width(SIDE_W).selected_text(format!("{}  {cur}", ic::CAPTIONS)).show_ui(ui, |ui| {
            if ui.selectable_label(self.style_id.is_none(), "Sem legenda").clicked() {
                pick = Some(None);
            }
            for s in &self.styles {
                if ui.selectable_label(self.style_id == Some(s.row.id), &s.row.name).clicked() {
                    pick = Some(Some(s.row.id));
                }
            }
        });
        if let Some(id) = pick {
            self.style_id = id;
            self.style = id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.style.clone());
        }
        if self.styles.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::muted("Crie um modelo de legenda em Modelos para usar aqui")).wrap());
        }
    }

    /// Arquivo de fundo escolhido (para a tela de Clipes carregar a miniatura).
    pub fn bg_file(&self) -> Option<&str> {
        self.s.bg.path().filter(|p| !p.is_empty())
    }

    /// `bg_tex`: miniatura do arquivo de fundo, quando o fundo é um arquivo.
    pub fn show(&mut self, ctx: &egui::Context, fonts: &[String], bg_tex: Option<egui::TextureHandle>) -> Outcome {
        self.bg_tex = bg_tex;
        self.snap = ctx.data(|d| d.get_temp::<bool>(snap_id())).unwrap_or(self.snap);
        self.tab = ctx.data(|d| d.get_temp::<Tab>(tab_id())).unwrap_or(self.tab);
        let out = self.show_inner(ctx, fonts);
        ctx.data_mut(|d| {
            d.insert_temp(snap_id(), self.snap);
            d.insert_temp(tab_id(), self.tab);
        });
        out
    }

    fn show_inner(&mut self, ctx: &egui::Context, fonts: &[String]) -> Outcome {
        self.player.update(ctx);
        // Toca só o trecho, pulando os cortes; no fim, para e volta para o começo.
        if self.player.is_playing() {
            let pos = self.player.pos;
            if pos >= self.s.end - 0.05 {
                self.player.pause();
                self.player.seek(self.s.start);
            } else if let Some(c) = self.cut_at(pos).map(|i| self.s.cuts[i]) {
                self.player.seek(c.a.max(c.b));
                self.player.stop_at = Some(self.s.end);
                self.player.play();
            }
        }
        // Faixa da linha do tempo: o clipe com folga para os lados (parada
        // enquanto uma ponta é arrastada, para não fugir do mouse).
        if self.end_drag.is_none() {
            let len = self.s.end - self.s.start;
            let pad = (len * 0.15).max(3.0);
            let d = self.dur();
            self.view = ((self.s.start - pad).max(0.0), if d > 0.0 { (self.s.end + pad).min(d) } else { self.s.end + pad });
        }
        if !self.player.is_playing() && (self.player.pos < self.view.0 - 0.5 || self.player.pos > self.view.1 + 0.5) {
            self.player.seek(self.s.start);
        }
        let mut toggle = self.shortcuts(ctx);
        let mut out = Outcome::Open;
        let kept = kept_ranges(self.s.start, self.s.end, &self.s.cuts);
        let final_len = kept_len(&kept);
        let meta = format!("{} → {} · {}s", timecode(self.s.start, false), timecode(self.s.end, false), final_len.round());
        let title = format!("Editar clipe · {}", self.title);
        let screen_h = ctx.content_rect().height();
        let (open, _) = w::modal(ctx, "clip-modal", &title, &meta, 1640.0, |ui| {
            // Alturas fixas: cabeçalho 57, rodapé 57, parte de baixo (controles
            // + faixas); o resto vai para a área de trabalho.
            let tracks_h = 20.0 + 4.0 + 52.0 + if self.style.is_some() { 4.0 + 26.0 } else { 0.0 };
            let lower_h = 12.0 + 32.0 + 8.0 + tracks_h + 16.0;
            let body_h = (screen_h - 48.0 - 57.0 - 58.0).max(320.0);
            let work_h = (body_h - lower_h - 2.0).clamp(320.0, 820.0);
            egui::ScrollArea::vertical().id_salt("clip-modal-body").max_height(body_h).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    self.rail(ui, work_h);
                    let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, work_h), Sense::hover());
                    ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0, LINE));
                    self.inspector(ui, work_h);
                    let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, work_h), Sense::hover());
                    ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0, LINE));
                    if self.viewer(ui, work_h, fonts) {
                        toggle = true;
                    }
                });
                w::line(ui);
                egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 16 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    if self.transport(ui, final_len) {
                        toggle = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(GUTTER);
                        self.ruler(ui);
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        track_label(ui, 52.0, "Áreas");
                        self.timeline(ui);
                    });
                    if self.style.is_some() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            track_label(ui, 26.0, "Legenda");
                            self.cap_strip(ui);
                        });
                    }
                });
            });
            if toggle {
                if self.player.is_playing() {
                    self.player.pause();
                } else {
                    if self.player.pos >= self.s.end - 0.1 || self.player.pos < self.s.start {
                        self.player.seek(self.s.start);
                    }
                    self.player.stop_at = Some(self.s.end);
                    self.player.play();
                }
            }
            w::line(ui);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (k, what) in [("Espaço", "tocar"), ("I", "início"), ("O", "fim"), ("C", "corte"), ("N", "troca"), ("T", "travas")] {
                        w::keycap(ui, k);
                        ui.label(w::muted(what));
                        ui.add_space(6.0);
                    }
                    ui.label(w::muted("· mais atalhos")).on_hover_text("←/→ um quadro · Shift+←/→ um segundo · Del remove a troca · Alt solta a trava no arraste · Ctrl+Z desfaz · Ctrl+Y refaz");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if w::button(ui, Kind::Primary, Some(ic::CHECK), "Salvar").clicked() {
                            out = Outcome::Save(self.s.clone());
                        }
                        if w::button(ui, Kind::Default, None, "Salvar e aplicar aos outros")
                            .on_hover_text("Copia o modo, as áreas, a divisão, o encaixe com a posição, o deslize e a posição da legenda para os outros clipes deste vídeo. Início, fim, trocas, cortes e texto continuam só neste")
                            .clicked()
                        {
                            out = Outcome::SaveAll(self.s.clone());
                        }
                        if w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                            out = Outcome::Close;
                        }
                    });
                });
            });
        });
        // Fecha um passo de desfazer quando o mouse solta.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.commit();
        }
        if !open {
            return Outcome::Close;
        }
        out
    }

    /// Trilho de ferramentas à esquerda: ícone em cima, nome embaixo.
    fn rail(&mut self, ui: &mut Ui, h: f32) {
        ui.allocate_ui_with_layout(egui::vec2(RAIL_W, h), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.set_width(RAIL_W);
            ui.set_min_height(h);
            ui.add_space(8.0);
            for (tab, icon, name) in TABS {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(RAIL_W - 16.0, 56.0), Sense::click());
                let on = self.tab == tab;
                let p = ui.painter();
                if on {
                    p.rect_filled(r, CornerRadius::same(R), RAISED_2);
                    p.hline(r.x_range().shrink(R as f32), r.top() + 1.0, Stroke::new(1.0, HIGHLIGHT));
                } else if resp.hovered() {
                    p.rect_filled(r, CornerRadius::same(R), RAISED);
                }
                let (ic_c, tx_c) = if on { (TEXT, TEXT) } else if resp.hovered() { (TEXT_2, TEXT_2) } else { (TEXT_3, TEXT_2) };
                p.text(r.center_top() + egui::vec2(0.0, 18.0), Align2::CENTER_CENTER, icon, body(ICON), ic_c);
                p.text(r.center_top() + egui::vec2(0.0, 40.0), Align2::CENTER_CENTER, name, body(T_META), tx_c);
                // Ponto quando a ferramenta tem algo ligado neste clipe.
                let busy = match tab {
                    Tab::Motion => !self.s.keys.is_empty(),
                    Tab::Trim => !self.s.cuts.is_empty(),
                    Tab::Caption => self.s.caption.is_some() || !self.s.cap_keys.is_empty(),
                    Tab::Text => !self.s.word_edits.is_empty(),
                    Tab::Mark => self.s.watermark_id.is_some(),
                    Tab::Frame => false,
                };
                if busy {
                    p.circle_filled(r.right_top() + egui::vec2(-10.0, 10.0), 3.0, ACCENT_INK);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.tab = tab;
                }
                ui.add_space(4.0);
            }
        });
    }

    /// Painel da ferramenta aberta.
    fn inspector(&mut self, ui: &mut Ui, h: f32) {
        ui.allocate_ui_with_layout(egui::vec2(INSP_W, h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(INSP_W);
            ui.set_min_height(h);
            egui::ScrollArea::vertical().id_salt("clip-modal-insp").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 20 }).show(ui, |ui| {
                    ui.set_width(SIDE_W);
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                    let (title, sub) = self.tab.head();
                    ui.label(egui::RichText::new(title).font(strong(T_BODY)).color(TEXT));
                    ui.add(egui::Label::new(w::muted(sub)).wrap());
                    ui.add_space(16.0);
                    match self.tab {
                        Tab::Frame => self.frame_panel(ui),
                        Tab::Motion => self.motion_panel(ui),
                        Tab::Trim => self.trim_panel(ui),
                        Tab::Caption => self.caption_panel(ui),
                        Tab::Text => self.text_panel(ui, h - 120.0),
                        Tab::Mark => self.mark_panel(ui),
                    }
                });
            });
        });
    }

    /// Monitores: fonte com as áreas e saída 9:16. Devolve true num clique na fonte.
    fn viewer(&mut self, ui: &mut Ui, h: f32, fonts: &[String]) -> bool {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::ZERO, BG_DEEP);
        let inner = rect.shrink2(egui::vec2(20.0, 16.0));
        let mh = inner.height() - 24.0;
        let prev_w = (mh * 9.0 / 16.0).round();
        let stage_w = (inner.width() - prev_w - 20.0).max(240.0);
        let mut clicked = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Min)), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(20.0, 0.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.allocate_ui_with_layout(egui::vec2(stage_w, 24.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.label(w::mono_text("Fonte", T_META, TEXT_3));
                    if self.cut_at(self.player.pos).is_some() {
                        ui.label(w::mono_text("· trecho cortado, não entra no clipe", T_META, ERR));
                    }
                });
                clicked = self.stage(ui, stage_w, mh);
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.allocate_ui_with_layout(egui::vec2(prev_w, 24.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.label(w::mono_text("Saída 9:16", T_META, TEXT_3));
                });
                self.preview(ui, prev_w, mh, fonts);
            });
        });
        clicked
    }

    /// Tocar, tempo, travas e desfazer. Devolve true se pediu tocar/pausar.
    fn transport(&mut self, ui: &mut Ui, final_len: f64) -> bool {
        let mut toggle = false;
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let playing = self.player.is_playing();
            if w::button_sized(ui, Kind::Default, Some(if playing { ic::PAUSE } else { ic::PLAY }), "", 32.0).on_hover_text(if playing { "Pausar (espaço)" } else { "Tocar (espaço)" }).clicked() {
                toggle = true;
            }
            ui.add_space(4.0);
            let rel = self.now() - self.s.start;
            ui.label(w::mono_text(timecode(rel, true), T_BODY, TEXT));
            ui.label(w::mono_text(format!("/ {}", timecode(self.s.end - self.s.start, true)), T_BODY, TEXT_3));
            if final_len < self.s.end - self.s.start - 0.05 {
                ui.label(w::mono_text(format!("· final {} sem os cortes", timecode(final_len, false)), T_META, TEXT_3));
            }
            if let Some(a) = self.cut_mark {
                ui.label(w::mono_text(format!("· corte desde {}", timecode(a - self.s.start, false)), T_META, MARK));
            }
            // Volume logo depois do tempo, como no player da tela de Clipes.
            ui.add_space(8.0);
            let off = self.muted || self.volume <= 0.0;
            let icon = if off { ic::VOLUME_X } else if self.volume < 0.5 { ic::VOLUME_1 } else { ic::VOLUME_2 };
            let mut vol_changed = false;
            if w::icon_button(ui, icon, if off { "Ligar som" } else { "Sem som" }).clicked() {
                if self.volume <= 0.0 {
                    self.volume = 0.8;
                    self.muted = false;
                } else {
                    self.muted = !self.muted;
                }
                vol_changed = true;
            }
            if w::volume_bar(ui, &mut self.volume, self.muted) {
                if self.volume > 0.0 {
                    self.muted = false;
                }
                vol_changed = true;
            }
            if vol_changed {
                self.player.set_volume(if self.muted { 0.0 } else { self.volume });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_enabled_ui(!self.redo.is_empty(), |ui| {
                    if w::button(ui, Kind::Ghost, None, "Refazer").on_hover_text("Ctrl+Y").clicked() {
                        self.do_redo();
                    }
                });
                ui.add_enabled_ui(!self.undo.is_empty() || self.s != self.committed, |ui| {
                    if w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Desfazer").on_hover_text("Ctrl+Z").clicked() {
                        self.do_undo();
                    }
                });
                ui.add_space(8.0);
                let (icon, label) = if self.snap { (ic::LOCK, "Travas no centro") } else { (ic::LOCK_OPEN, "Travas desligadas") };
                if w::button(ui, Kind::Ghost, Some(icon), label)
                    .on_hover_text("As áreas grudam no centro e nas bordas ao mover ou esticar. Segure Alt para soltar na hora (T)")
                    .clicked()
                {
                    self.snap = !self.snap;
                }
            });
        });
        toggle
    }

    /// Régua de tempo em cima das faixas (segundos a partir do início do clipe).
    fn ruler(&mut self, ui: &mut Ui) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::click_and_drag());
        let inner = rect.shrink2(egui::vec2(10.0, 0.0));
        let p = ui.painter_at(rect);
        let (v0, v1) = self.view;
        let px_s = inner.width() as f64 / (v1 - v0).max(0.001);
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0].into_iter().find(|s| s * px_s >= 64.0).unwrap_or(300.0);
        let first = ((v0 - self.s.start) / step).floor() as i64;
        let last = ((v1 - self.s.start) / step).ceil() as i64;
        for i in first..=last {
            let rel = i as f64 * step;
            let t = self.s.start + rel;
            if t < v0 - 1e-6 || t > v1 + 1e-6 {
                continue;
            }
            let x = self.tx(inner, t);
            p.vline(x, rect.bottom() - 6.0..=rect.bottom(), Stroke::new(1.0, LINE_2));
            let half = self.tx(inner, t + step / 2.0);
            if t + step / 2.0 <= v1 {
                p.vline(half, rect.bottom() - 3.0..=rect.bottom(), Stroke::new(1.0, LINE_2));
            }
            let a = rel.abs();
            let label = if a < 60.0 { if step < 1.0 { format!("{a:.1}s") } else { format!("{a:.0}s") } } else { format!("{}:{:02}", (a / 60.0) as i64, (a % 60.0) as i64) };
            let label = if rel < -1e-6 { format!("−{label}") } else { label };
            p.text(egui::pos2(x + 4.0, rect.top() + 2.0), Align2::LEFT_TOP, label, mono(T_META - 1.0), if rel < -1e-6 || t > self.s.end { TEXT_3.gamma_multiply(0.6) } else { TEXT_3 });
        }
        let px = self.tx(inner, self.player.pos);
        p.add(egui::Shape::convex_polygon(vec![egui::pos2(px - 5.0, rect.bottom() - 7.0), egui::pos2(px + 5.0, rect.bottom() - 7.0), egui::pos2(px, rect.bottom())], TEXT, Stroke::NONE));
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
            let t = self.t_of(inner, pos.x);
            self.seek(t);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    /// Quadro da fonte com as áreas. Devolve true num clique simples (tocar/pausar).
    fn stage(&mut self, ui: &mut Ui, width: f32, height: f32) -> bool {
        let (outer, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        ui.painter().rect_filled(outer, CornerRadius::same(R), Color32::BLACK);
        let aspect = self.player.aspect();
        let img = fit_rect(outer, aspect);
        match self.player.texture() {
            Some(t) => {
                ui.painter().image(t.id(), img, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().text(outer.center(), Align2::CENTER_CENTER, "Carregando quadro", body(T_SEC), TEXT_3);
            }
        }
        let to_screen = |r: FRect| {
            Rect::from_min_size(
                egui::pos2(img.left() + img.width() * r.x as f32, img.top() + img.height() * r.y as f32),
                egui::vec2(img.width() * r.w as f32, img.height() * r.h as f32),
            )
        };
        let t = self.now();
        let defs = boxes(&self.layout_at(t), self.s.split);
        if defs.is_empty() {
            ui.painter().text(outer.center_bottom() - egui::vec2(0.0, 16.0), Align2::CENTER_BOTTOM, "Horizontal usa o vídeo inteiro", body(T_SEC), TEXT_2);
        }
        let p = ui.painter_at(outer);
        for b in &defs {
            let fr = self.get(b.key, t);
            let r = to_screen(fr);
            // Preencher: escurece o que vai ser cortado para caber no espaço.
            if !self.fit_of(b.key) {
                let v = to_screen(visible(fr, aspect, b.slot));
                let dim = Color32::from_black_alpha(150);
                p.rect_filled(Rect::from_min_max(r.min, egui::pos2(r.right(), v.top())), CornerRadius::ZERO, dim);
                p.rect_filled(Rect::from_min_max(egui::pos2(r.left(), v.bottom()), r.max), CornerRadius::ZERO, dim);
                p.rect_filled(Rect::from_min_max(egui::pos2(r.left(), v.top()), egui::pos2(v.left(), v.bottom())), CornerRadius::ZERO, dim);
                p.rect_filled(Rect::from_min_max(egui::pos2(v.right(), v.top()), egui::pos2(r.right(), v.bottom())), CornerRadius::ZERO, dim);
            }
            p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, b.color), StrokeKind::Inside);
            let mode = if self.fit_of(b.key) { "encaixar" } else { "preencher" };
            let tag = p.layout_no_wrap(format!("{} · {mode}", b.label), mono(T_META), TEXT);
            let tr = Rect::from_min_size(r.left_top() + egui::vec2(4.0, 4.0), tag.size() + egui::vec2(12.0, 4.0));
            p.rect_filled(tr, CornerRadius::same(4), Color32::from_black_alpha(200));
            p.galley(tr.min + egui::vec2(6.0, 2.0), tag, TEXT);
            for g in Grip::EDGES {
                p.rect_filled(Rect::from_center_size(g.point(r), egui::vec2(10.0, 10.0)), CornerRadius::same(2), b.color);
            }
        }

        let resp = ui.interact(outer, egui::Id::unique(("clip-modal-stage", self.clip_id)), Sense::click_and_drag());
        let hit = |pos: Pos2, me: &Self| -> Option<(Key, Grip, FRect)> {
            defs.iter().rev().find_map(|b| {
                let fr = me.get(b.key, t);
                let r = to_screen(fr);
                if let Some(g) = Grip::EDGES.iter().find(|g| g.point(r).distance(pos) < 10.0) {
                    return Some((b.key, *g, fr));
                }
                r.contains(pos).then_some((b.key, Grip::Move, fr))
            })
        };
        if let Some(pos) = resp.hover_pos() {
            if let Some((_, g, _)) = self.drag.map(|d| (d.0, d.1, d.2)).or_else(|| hit(pos, self)) {
                ui.ctx().set_cursor_icon(g.cursor());
            }
        }
        if resp.drag_started() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                self.drag = hit(pos, self).map(|(k, g, r)| (k, g, r, pos));
            }
        }
        if let (Some((k, g, r0, from)), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
            if resp.dragged() {
                let dx = ((pos.x - from.x) / img.width()) as f64;
                let dy = ((pos.y - from.y) / img.height()) as f64;
                let min = 0.03;
                // Travas: o centro (ao mover) ou a borda arrastada gruda no meio do quadro.
                let snap = self.snap && !ui.input(|i| i.modifiers.alt);
                let (tx, ty) = ((SNAP_PX / img.width()) as f64, (SNAP_PX / img.height()) as f64);
                let mut guides = (false, false);
                let FRect { mut x, mut y, mut w, mut h } = r0;
                if g == Grip::Move {
                    x = (x + dx).clamp(0.0, 1.0 - w);
                    y = (y + dy).clamp(0.0, 1.0 - h);
                    if snap && (x + w / 2.0 - 0.5).abs() < tx {
                        x = (0.5 - w / 2.0).clamp(0.0, 1.0 - w);
                        guides.0 = true;
                    }
                    if snap && (y + h / 2.0 - 0.5).abs() < ty {
                        y = (0.5 - h / 2.0).clamp(0.0, 1.0 - h);
                        guides.1 = true;
                    }
                } else {
                    if g.has('w') {
                        let mut nx = (x + dx).clamp(0.0, x + w - min);
                        if snap && (nx - 0.5).abs() < tx && x + w - 0.5 >= min {
                            nx = 0.5;
                            guides.0 = true;
                        }
                        w += x - nx;
                        x = nx;
                    }
                    if g.has('e') {
                        w = (w + dx).clamp(min, 1.0 - x);
                        if snap && (x + w - 0.5).abs() < tx && 0.5 - x >= min {
                            w = 0.5 - x;
                            guides.0 = true;
                        }
                    }
                    if g.has('n') {
                        let mut ny = (y + dy).clamp(0.0, y + h - min);
                        if snap && (ny - 0.5).abs() < ty && y + h - 0.5 >= min {
                            ny = 0.5;
                            guides.1 = true;
                        }
                        h += y - ny;
                        y = ny;
                    }
                    if g.has('s') {
                        h = (h + dy).clamp(min, 1.0 - y);
                        if snap && (y + h - 0.5).abs() < ty && 0.5 - y >= min {
                            h = 0.5 - y;
                            guides.1 = true;
                        }
                    }
                }
                self.guides = guides;
                self.set(k, t, FRect { x, y, w, h });
            }
        }
        if resp.drag_stopped() || self.drag.is_none() {
            self.drag = None;
            self.guides = (false, false);
        }
        // Travas do centro: marquinhas no meio de cada borda e uma cruz no
        // centro; viram linha inteira quando a área gruda nelas.
        if self.snap && !defs.is_empty() {
            let c = img.center();
            let on = Color32::from_rgb(240, 190, 60);
            let idle = Color32::from_white_alpha(90);
            let (gx, gy) = self.guides;
            let tick = |a: Pos2, b: Pos2, lit: bool| p.line_segment([a, b], Stroke::new(if lit { 2.0 } else { 1.5 }, if lit { on } else { idle }));
            tick(egui::pos2(c.x, img.top()), egui::pos2(c.x, img.top() + 8.0), gx);
            tick(egui::pos2(c.x, img.bottom() - 8.0), egui::pos2(c.x, img.bottom()), gx);
            tick(egui::pos2(img.left(), c.y), egui::pos2(img.left() + 8.0, c.y), gy);
            tick(egui::pos2(img.right() - 8.0, c.y), egui::pos2(img.right(), c.y), gy);
            tick(c - egui::vec2(5.0, 0.0), c + egui::vec2(5.0, 0.0), gy);
            tick(c - egui::vec2(0.0, 5.0), c + egui::vec2(0.0, 5.0), gx);
            if gx {
                p.vline(c.x, img.y_range(), Stroke::new(1.0, on));
            }
            if gy {
                p.hline(img.x_range(), c.y, Stroke::new(1.0, on));
            }
        }
        if let Some(pos) = resp.hover_pos() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                if let Some((k, _, _)) = hit(pos, self) {
                    // Proporcional ao giro: um "clique" da roda (~50 px) dá ~10% de zoom.
                    self.zoom(k, t, (-(wheel as f64) * 0.002).exp());
                    // Não deixa a roda rolar o modal junto.
                    ui.input_mut(|i| {
                        i.smooth_scroll_delta = egui::Vec2::ZERO;
                    });
                }
            }
        }
        resp.clicked()
    }

    /// Prévia 9:16 do clipe como vai sair. Arrastar uma área encaixada muda
    /// ela de lugar (roda do mouse muda o tamanho); arrastar a legenda muda a
    /// legenda de lugar.
    fn preview(&mut self, ui: &mut Ui, width: f32, height: f32, fonts: &[String]) {
        let (slot, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
        let t = self.now();
        let src = self.player.aspect();
        let tex = self.player.texture().map(|tx| (tx, src));
        let st = self.style_now();
        let (layout, cam, scr) = self.look_at(t);
        let words = self.words_now();
        clip_preview(ui, slot, tex, &layout, cam, scr, self.s.fit, self.s.split, &self.s.bg, self.bg_tex.as_ref(), self.wm_items(), st.as_ref(), fonts, &words, t);
        let out = if self.s.layout == "horizontal" { Rect::from_center_size(slot.center(), egui::vec2(slot.width(), slot.width() * 9.0 / 16.0)) } else { slot };

        // Áreas encaixadas: espaço e onde a área está dentro dele.
        let movable: Vec<(Key, Rect, Rect)> = out_slots(out, &layout, self.s.split)
            .into_iter()
            .filter(|(k, _)| self.fit_of(*k))
            .map(|(k, dst)| {
                let r = if k == Key::Camera { cam } else { scr };
                (k, dst, placed_rect(dst, area_aspect(r, src), self.place_of(k)))
            })
            .collect();
        // Faixa onde a legenda está (para pegar ela antes da área).
        let cap_band = self.style.as_ref().map(|base| {
            let adj = self.cap_at(t).unwrap_or_else(|| CaptionAdjust::from_style(base));
            let bh = (adj.size / 1920.0) as f32 * out.height() * 1.3;
            let top = match adj.position.as_str() {
                "top" => out.top() + out.height() * adj.margin as f32,
                "center" => out.center().y - bh / 2.0,
                _ => out.bottom() - out.height() * adj.margin as f32 - bh,
            };
            Rect::from_min_size(egui::pos2(out.left(), top), egui::vec2(out.width(), bh)).expand2(egui::vec2(0.0, 6.0))
        });
        let on_cap = |pos: Pos2| cap_band.is_some_and(|b| b.contains(pos));
        let on_area = |pos: Pos2| movable.iter().rev().find(|m| m.2.contains(pos)).copied();

        if let Some(pos) = resp.hover_pos() {
            if self.place_drag.is_some() || (!self.cap_drag && !on_cap(pos) && on_area(pos).is_some()) {
                ui.ctx().set_cursor_icon(if self.place_drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            } else if self.cap_drag || on_cap(pos) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }
        if resp.drag_started() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                if on_cap(pos) {
                    self.cap_drag = true;
                } else if let Some((k, _, pr)) = on_area(pos) {
                    self.place_drag = Some((k, pr, pos));
                }
            }
        }
        // Arraste da área: fração da sobra do espaço, com travas no começo, meio e fim.
        if let (Some((k, r0, from)), Some(pos)) = (self.place_drag, resp.interact_pointer_pos()) {
            if resp.dragged() {
                if let Some(&(_, dst, _)) = movable.iter().find(|m| m.0 == k) {
                    let snap = self.snap && !ui.input(|i| i.modifiers.alt);
                    let min = r0.min + (pos - from);
                    let mut p = self.place_of(k);
                    let mut guides = (None, None);
                    let axis = |off: f32, free: f32, cur: f64, lo: f32, size: f32, g: &mut Option<f32>| -> f64 {
                        if free < 1.0 {
                            return cur;
                        }
                        let mut f = (off / free).clamp(0.0, 1.0) as f64;
                        if snap {
                            for s in [0.0, 0.5, 1.0] {
                                if (off - free * s as f32).abs() < SNAP_PX {
                                    f = s;
                                    *g = Some(lo + free * s as f32 + size * s as f32);
                                }
                            }
                        }
                        f
                    };
                    p.x = axis(min.x - dst.left(), dst.width() - r0.width(), p.x, dst.left(), r0.width(), &mut guides.0);
                    p.y = axis(min.y - dst.top(), dst.height() - r0.height(), p.y, dst.top(), r0.height(), &mut guides.1);
                    self.place_guides = guides;
                    self.set_place(k, p);
                }
            }
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            self.place_drag = None;
            self.place_guides = (None, None);
        }
        // Roda do mouse sobre uma área encaixada: tamanho.
        if let Some(pos) = resp.hover_pos().filter(|p| !on_cap(*p)) {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if let (true, Some((k, _, _))) = (wheel != 0.0, on_area(pos)) {
                let mut p = self.place_of(k);
                p.scale = (p.scale * (wheel as f64 * 0.002).exp()).clamp(PLACE_SCALE_MIN, 1.0);
                self.set_place(k, p);
                ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        }
        // Contorno da área que dá para pegar e linhas das travas.
        let pt = ui.painter_at(out);
        let hot = self.place_drag.map(|d| d.0).or_else(|| resp.hover_pos().filter(|p| !on_cap(*p) && !self.cap_drag).and_then(on_area).map(|m| m.0));
        if let Some(k) = hot {
            if let Some(&(_, dst, pr)) = movable.iter().find(|m| m.0 == k) {
                pt.rect_stroke(dst, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Inside);
                pt.rect_stroke(pr, CornerRadius::ZERO, Stroke::new(1.5, ACCENT_INK), StrokeKind::Inside);
                if let Some(x) = self.place_guides.0 {
                    pt.vline(x, dst.y_range(), Stroke::new(1.0, MARK));
                }
                if let Some(y) = self.place_guides.1 {
                    pt.hline(dst.x_range(), y, Stroke::new(1.0, MARK));
                }
            }
        }
        if let Some(base) = self.style.as_ref() {
            if resp.drag_started() && self.place_drag.is_none() && !self.cap_drag && movable.is_empty() {
                self.cap_drag = true;
            }
            if let (true, Some(pos)) = (self.cap_drag && resp.dragged(), resp.interact_pointer_pos()) {
                let y = ((pos.y - out.top()) / out.height()).clamp(0.0, 1.0) as f64;
                let size = self.cap_at(t).map(|c| c.size).unwrap_or(base.size);
                let adj = if (0.44..=0.56).contains(&y) {
                    CaptionAdjust { position: "center".into(), margin: 0.0, size }
                } else if y < 0.5 {
                    CaptionAdjust { position: "top".into(), margin: y.min(0.45), size }
                } else {
                    CaptionAdjust { position: "bottom".into(), margin: (1.0 - y).min(0.45), size }
                };
                self.set_cap(t, Some(adj));
            }
            if resp.drag_stopped() {
                self.cap_drag = false;
            }
        }
        if !ui.input(|i| i.pointer.any_down()) {
            self.cap_drag = false;
        }
        let tip = match resp.hover_pos() {
            Some(p) if on_cap(p) => Some("Arraste para mudar a legenda de lugar"),
            Some(p) if on_area(p).is_some() => Some("Arraste para mudar a área de lugar. Roda do mouse muda o tamanho"),
            _ => None,
        };
        if let Some(tip) = tip {
            resp.on_hover_text(tip);
        }
    }

    /// x na tela de um instante da fonte, dentro da faixa mostrada.
    fn tx(&self, inner: Rect, t: f64) -> f32 {
        let (v0, v1) = self.view;
        inner.left() + inner.width() * ((t - v0) / (v1 - v0).max(0.001)).clamp(0.0, 1.0) as f32
    }

    fn t_of(&self, inner: Rect, x: f32) -> f64 {
        let (v0, v1) = self.view;
        v0 + ((x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64 * (v1 - v0)
    }

    /// Linha do tempo: o clipe (com folga dos lados), trechos de área,
    /// cortes, alças de início e fim e agulha.
    fn timeline(&mut self, ui: &mut Ui) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect(rect, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink2(egui::vec2(10.0, 6.0));
        let (xs, xe) = (self.tx(inner, self.s.start), self.tx(inner, self.s.end));
        let segs = area_segments(self.s.start, self.s.end, &self.s.layout, self.s.camera, self.s.screen, &self.s.keys);
        let cur = self.now() - self.s.start;
        for (i, s) in segs.iter().enumerate() {
            let on = cur >= s.a && cur < s.b;
            let r = Rect::from_min_max(egui::pos2(self.tx(inner, self.s.start + s.a) + 1.0, inner.top() + 10.0), egui::pos2(self.tx(inner, self.s.start + s.b) - 1.0, inner.bottom()));
            let fill = if on { Color32::from_rgba_unmultiplied(59, 99, 214, 90) } else if i % 2 == 0 { RAISED } else { RAISED_2 };
            p.rect_filled(r, CornerRadius::same(3), fill);
            let mode = match s.layout {
                "vertical_split" => " · tela + câmera",
                "vertical_split_cam" => " · câmera + tela",
                "vertical_camera" => " · câmera",
                _ => "",
            };
            p.with_clip_rect(r).text(r.left_center() + egui::vec2(6.0, 0.0), Align2::LEFT_CENTER, format!("Área {}{mode}", i + 1), mono(T_META), if on { TEXT } else { TEXT_3 });
            // Deslize: faixa no começo do trecho, do tamanho da duração escolhida.
            if self.s.smooth && i > 0 && segs[i - 1].layout == s.layout && s.layout != "horizontal" {
                let d = self.s.smooth_secs.min((s.b - s.a) / 2.0);
                let band = Rect::from_min_max(r.left_top(), egui::pos2(self.tx(inner, self.s.start + s.a + d), r.bottom()));
                p.rect_filled(band, CornerRadius::same(3), Color32::from_rgba_unmultiplied(59, 99, 214, 60));
            }
        }
        // Fora do clipe: só contexto, para poder esticar.
        let shade = Color32::from_black_alpha(140);
        p.rect_filled(Rect::from_min_max(egui::pos2(inner.left(), inner.top() + 10.0), egui::pos2(xs, inner.bottom())), CornerRadius::ZERO, shade);
        p.rect_filled(Rect::from_min_max(egui::pos2(xe, inner.top() + 10.0), egui::pos2(inner.right(), inner.bottom())), CornerRadius::ZERO, shade);
        // Cortes.
        for c in &self.s.cuts {
            let r = Rect::from_min_max(egui::pos2(self.tx(inner, c.a), inner.top() + 10.0), egui::pos2(self.tx(inner, c.b), inner.bottom()));
            p.rect_filled(r, CornerRadius::same(2), Color32::from_rgba_unmultiplied(200, 50, 50, 150));
            if r.width() > 34.0 {
                p.with_clip_rect(r).text(r.center(), Align2::CENTER_CENTER, "corte", mono(T_META), TEXT);
            }
        }
        if let Some(a) = self.cut_mark {
            let r = Rect::from_min_max(egui::pos2(self.tx(inner, a.min(self.now())), inner.top() + 10.0), egui::pos2(self.tx(inner, a.max(self.now())), inner.bottom()));
            p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(220, 120, 40, 90));
            p.vline(self.tx(inner, a), inner.y_range(), Stroke::new(1.5, Color32::from_rgb(230, 140, 50)));
        }
        for s in segs.iter().skip(1) {
            let px = self.tx(inner, self.s.start + s.a);
            p.add(egui::Shape::convex_polygon(
                vec![egui::pos2(px, inner.top()), egui::pos2(px + 5.0, inner.top() + 5.0), egui::pos2(px, inner.top() + 10.0), egui::pos2(px - 5.0, inner.top() + 5.0)],
                ACCENT,
                Stroke::NONE,
            ));
        }
        // Alças de início e fim.
        for x in [xs, xe] {
            let r = Rect::from_center_size(egui::pos2(x, rect.center().y + 4.0), egui::vec2(6.0, rect.height() - 14.0));
            p.rect_filled(r, CornerRadius::same(2), Color32::from_rgb(240, 190, 60));
        }
        let px = self.tx(inner, self.player.pos);
        p.vline(px, rect.y_range(), Stroke::new(1.5, TEXT));

        let near = |x: f32, pos: Pos2| (pos.x - x).abs() < 8.0;
        if let Some(pos) = resp.hover_pos() {
            if self.end_drag.is_some() || near(xs, pos) || near(xe, pos) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        if resp.drag_started() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                self.end_drag = if near(xs, pos) { Some(End::Start) } else if near(xe, pos) { Some(End::Finish) } else { None };
            }
        }
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
            let t = self.t_of(inner, pos.x);
            match self.end_drag {
                Some(End::Start) => {
                    self.set_start(t);
                    self.seek(self.s.start);
                }
                Some(End::Finish) => {
                    self.set_end(t);
                    self.seek(self.s.end - 0.05);
                }
                None => self.seek(t),
            }
        }
        if resp.drag_stopped() {
            self.end_drag = None;
        }
        resp.on_hover_text("Clique para ir ao ponto. Arraste as alças amarelas para mudar o início e o fim");
    }

    /// Faixa da legenda embaixo da linha do tempo: um trecho por troca.
    fn cap_strip(&mut self, ui: &mut Ui) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect(rect, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink2(egui::vec2(10.0, 4.0));
        let len = self.s.end - self.s.start;
        let parts = caption_parts(self.s.start, self.s.end, self.s.caption.clone(), &self.s.cap_keys);
        let cur = self.now() - self.s.start;
        for (i, (a, adj)) in parts.iter().enumerate() {
            let b = parts.get(i + 1).map(|p| p.0).unwrap_or(len);
            let on = cur >= *a && cur < b;
            let r = Rect::from_min_max(egui::pos2(self.tx(inner, self.s.start + a) + 1.0, inner.top()), egui::pos2(self.tx(inner, self.s.start + b) - 1.0, inner.bottom()));
            let fill = if on { Color32::from_rgba_unmultiplied(59, 99, 214, 70) } else if i % 2 == 0 { RAISED } else { RAISED_2 };
            p.rect_filled(r, CornerRadius::same(3), fill);
            let pos = match adj.as_ref().map(|a| a.position.as_str()) {
                Some("top") => " · em cima",
                Some("center") => " · meio",
                Some(_) => " · embaixo",
                None => " · do modelo",
            };
            p.with_clip_rect(r).text(r.left_center() + egui::vec2(6.0, 0.0), Align2::LEFT_CENTER, format!("Legenda {}{pos}", i + 1), mono(T_META), if on { TEXT } else { TEXT_3 });
        }
        p.vline(self.tx(inner, self.player.pos), rect.y_range(), Stroke::new(1.5, TEXT));
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
            let t = self.t_of(inner, pos.x);
            self.seek(t);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    /// Escolha da composição do trecho: três quadrinhos 9:16 desenhados.
    fn layout_tiles(&mut self, ui: &mut Ui, t: f64) {
        let cur = Vertical::from_id(&self.layout_at(t)).unwrap_or(Vertical::Camera);
        let opts = [(Vertical::Camera, "Câmera"), (Vertical::Split, "Tela em cima"), (Vertical::SplitCam, "Câmera em cima")];
        let tw = (SIDE_W - 16.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (v, name) in opts {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(tw, 84.0), Sense::click());
                let on = cur == v;
                let p = ui.painter();
                p.rect(r, CornerRadius::same(R), if on { RAISED_2 } else if resp.hovered() { RAISED } else { BG_DEEP }, Stroke::new(1.0, if on { ACCENT } else { LINE }), StrokeKind::Inside);
                let fr = Rect::from_center_size(r.center_top() + egui::vec2(0.0, 32.0), egui::vec2(27.0, 48.0));
                p.rect_stroke(fr, CornerRadius::same(3), Stroke::new(1.0, if on { TEXT_2 } else { TEXT_3 }), StrokeKind::Inside);
                let cam_c = if on { ACCENT } else { TEXT_3.gamma_multiply(0.5) };
                let scr_c = if on { TEXT_2.gamma_multiply(0.5) } else { TEXT_3.gamma_multiply(0.25) };
                let split = self.s.split.clamp(0.2, 0.8) as f32;
                let inner = fr.shrink(3.0);
                match v {
                    Vertical::Camera => {
                        p.rect_filled(inner, CornerRadius::same(1), cam_c);
                    }
                    Vertical::Split => {
                        let y = inner.top() + inner.height() * split;
                        p.rect_filled(Rect::from_min_max(inner.min, egui::pos2(inner.right(), y - 1.0)), CornerRadius::same(1), scr_c);
                        p.rect_filled(Rect::from_min_max(egui::pos2(inner.left(), y + 1.0), inner.max), CornerRadius::same(1), cam_c);
                    }
                    _ => {
                        let y = inner.top() + inner.height() * (1.0 - split);
                        p.rect_filled(Rect::from_min_max(inner.min, egui::pos2(inner.right(), y - 1.0)), CornerRadius::same(1), cam_c);
                        p.rect_filled(Rect::from_min_max(egui::pos2(inner.left(), y + 1.0), inner.max), CornerRadius::same(1), scr_c);
                    }
                }
                p.text(r.center_bottom() - egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, name, body(T_META), if on { TEXT } else { TEXT_2 });
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                    self.set_layout_at(t, v);
                }
            }
        });
    }

    /// Enquadrar: composição, divisão e, para cada área, preencher/encaixar,
    /// zoom, posição no espaço e fundo.
    fn frame_panel(&mut self, ui: &mut Ui) {
        let t = self.now();
        if self.s.layout == "horizontal" {
            ui.add(egui::Label::new(w::muted("No horizontal o clipe usa o vídeo inteiro, sem áreas. Troque o layout na lista de sugestões para usar áreas")).wrap());
            return;
        }
        group(ui, "Composição deste trecho");
        self.layout_tiles(ui, t);
        let lay = self.layout_at(t);
        if lay != "vertical_camera" {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(w::dim("Divisão"));
                let mut tela = self.s.split * 100.0;
                ui.spacing_mut().slider_width = 120.0;
                if ui.add(egui::Slider::new(&mut tela, 20.0..=80.0).step_by(5.0).show_value(false)).on_hover_text("Quanto da altura fica para a tela (vale para o clipe todo)").changed() {
                    self.s.split = tela / 100.0;
                }
                ui.label(w::mono_text(format!("{:.0} / {:.0}", tela, 100.0 - tela), T_META, TEXT_3));
            });
        }
        let defs = boxes(&lay, self.s.split);
        for b in &defs {
            gap(ui);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 16.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, b.color);
                ui.label(egui::RichText::new(b.label).font(strong(T_SEC)).color(TEXT));
            });
            ui.add_space(8.0);
            let mut v = self.fit_of(b.key);
            if w::seg(ui, &mut v, &[(false, "Preencher"), (true, "Encaixar")]) {
                if b.key == Key::Camera { self.s.fit.camera = v } else { self.s.fit.screen = v }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if w::button(ui, Kind::Default, None, "−").on_hover_text("Afastar: mostra mais do quadro").clicked() {
                    self.zoom(b.key, t, 1.0 / 0.9);
                }
                if w::button(ui, Kind::Default, None, "+").on_hover_text("Aproximar: recorta mais perto").clicked() {
                    self.zoom(b.key, t, 0.9);
                }
                ui.add_space(4.0);
                if w::button(ui, Kind::Ghost, None, "Ajustar").on_hover_text("Deixa a caixa no formato exato do espaço: nada cortado, nada sobrando").clicked() {
                    self.fit_to_slot(b.key, t, b.slot);
                }
                if w::button(ui, Kind::Ghost, None, "Inteiro").on_hover_text("A área vira o vídeo inteiro (use Encaixar para não cortar)").clicked() {
                    self.set(b.key, t, FRect::FULL);
                }
            });
            if self.fit_of(b.key) {
                ui.add_space(12.0);
                let mut p = self.place_of(b.key);
                let mut ch = false;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    ch |= anchor_pad(ui, &mut p);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(w::dim("Posição no espaço"));
                        ui.horizontal(|ui| {
                            ui.label(w::dim("Tamanho"));
                            let mut pct = p.scale * 100.0;
                            ui.spacing_mut().slider_width = 80.0;
                            if ui.add(egui::Slider::new(&mut pct, PLACE_SCALE_MIN * 100.0..=100.0).step_by(1.0).show_value(false)).changed() {
                                p.scale = pct / 100.0;
                                ch = true;
                            }
                            ui.label(w::mono_text(format!("{:.0}%", pct), T_META, TEXT_3));
                        });
                        let moved = !p.is_center();
                        ui.add_enabled_ui(moved, |ui| {
                            if w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Centralizar").clicked() {
                                p = Place::CENTER;
                                ch = true;
                            }
                        });
                    });
                });
                if ch {
                    self.set_place(b.key, p);
                }
            }
        }
        if defs.iter().any(|b| self.fit_of(b.key)) {
            gap(ui);
            self.bg_picker(ui, true);
            ui.add(egui::Label::new(w::muted("Na saída, arraste a área encaixada para mudar de lugar e use a roda do mouse para o tamanho")).wrap());
        } else {
            ui.add_space(12.0);
            ui.add(egui::Label::new(w::muted("Encaixar mostra a área inteira no espaço, com o fundo que você escolher na sobra, e deixa mudar a posição dela")).wrap());
        }
    }

    /// Trocas de área ao longo do clipe e como passam de uma para outra.
    fn motion_panel(&mut self, ui: &mut Ui) {
        let t = self.now();
        if self.s.layout == "horizontal" {
            ui.add(egui::Label::new(w::muted("No horizontal o clipe usa o vídeo inteiro, sem trocas de área")).wrap());
            return;
        }
        let act = self.active(t);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if w::button(ui, Kind::Default, Some(ic::PLUS), "Nova troca aqui").on_hover_text("A partir deste instante o clipe usa outra área (N). Pause onde a câmera mudou de lugar").clicked() {
                self.new_area_key(t);
            }
            ui.add_enabled_ui(act.is_some(), |ui| {
                if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Remover").on_hover_text("Este trecho volta a usar a área anterior (Del)").clicked() {
                    if let Some(i) = act {
                        self.s.keys.remove(i);
                    }
                }
            });
        });
        ui.add_space(8.0);
        let n = area_segments(self.s.start, self.s.end, &self.s.layout, self.s.camera, self.s.screen, &self.s.keys).len();
        ui.add(egui::Label::new(w::muted(if n > 1 {
            format!("{n} áreas neste clipe. As mudanças no Enquadrar valem no trecho onde a agulha está")
        } else {
            "Uma área para o clipe todo. Pause onde a câmera mudou de lugar e crie uma troca".to_string()
        }))
        .wrap());
        gap(ui);
        group(ui, "Passagem entre áreas");
        ui.add_enabled_ui(n > 1, |ui| {
            w::seg(ui, &mut self.s.smooth, &[(false, "Corte seco"), (true, "Deslizar")]);
        });
        if n > 1 && self.s.smooth {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(w::dim("Duração"));
                ui.spacing_mut().slider_width = 140.0;
                ui.add(egui::Slider::new(&mut self.s.smooth_secs, SMOOTH_MIN..=SMOOTH_MAX).step_by(0.1).show_value(false))
                    .on_hover_text("Quanto tempo a área leva para deslizar de uma posição para a outra");
                ui.label(w::mono_text(format!("{:.1} s", self.s.smooth_secs), T_META, TEXT_3));
            });
            gap(ui);
            self.curve_editor(ui);
        } else if n <= 1 {
            ui.add_space(4.0);
            ui.label(w::muted("Disponível com duas áreas ou mais"));
        }
    }

    /// Marca d'água do clipe.
    fn mark_panel(&mut self, ui: &mut Ui) {
        self.watermark_panel(ui);
        ui.add_space(12.0);
        ui.add(egui::Label::new(w::muted("Crie, posicione e mude a transparência dos textos em Modelos › Marcas d'água")).wrap());
    }

    /// Fundo atrás das áreas em Encaixar: vídeo desfocado, desfoque leve,
    /// nada (preto) ou um arquivo da biblioteca de fundos de clipe.
    fn bg_picker(&mut self, ui: &mut Ui, any_fit: bool) {
        group(ui, "Fundo da sobra");
        let mut mode = self.s.bg.mode();
        // Dois por linha: as quatro opções não cabem lado a lado no painel.
        let half = (ClipBg::MODES.len() + 1) / 2;
        let mut pick = false;
        for row in ClipBg::MODES.chunks(half) {
            pick |= w::seg(ui, &mut mode, row);
            ui.add_space(4.0);
        }
        if pick {
            self.s.bg = match mode {
                "file" => {
                    // Volta ao último arquivo usado, ou ao primeiro da biblioteca.
                    let last = self.committed.bg.path().filter(|p| !p.is_empty()).map(str::to_string);
                    ClipBg::File(last.or_else(|| self.backgrounds.first().map(|b| b.file_path.clone())).unwrap_or_default())
                }
                m => ClipBg::from_parts(m, None),
            };
        }
        if let ClipBg::File(cur) = &self.s.bg {
            ui.add_space(6.0);
            if self.backgrounds.is_empty() {
                ui.label(w::muted("Nenhum fundo na biblioteca. Adicione imagens ou vídeos em Modelos › Fundos de clipe"));
            } else {
                let name = self.backgrounds.iter().find(|b| &b.file_path == cur).map(|b| b.name.clone()).unwrap_or_else(|| "Escolher fundo".into());
                let mut pick = None;
                egui::ComboBox::from_id_salt("clip-modal-bg").width(SIDE_W).selected_text(name).show_ui(ui, |ui| {
                    for b in &self.backgrounds {
                        if ui.selectable_label(&b.file_path == cur, &b.name).clicked() {
                            pick = Some(b.file_path.clone());
                        }
                    }
                });
                if let Some(p) = pick {
                    self.s.bg = ClipBg::File(p);
                }
            }
        }
        if !any_fit {
            ui.add_space(2.0);
            ui.label(w::muted("O fundo aparece só nas áreas em Encaixar"));
        }
        ui.add_space(10.0);
    }

    /// Gráfico da curva de velocidade do deslize: tempo no eixo x, caminho
    /// percorrido no y. Dois pontos de controle arrastáveis, curvas prontas,
    /// valores exatos e uma bolinha que mostra o movimento.
    fn curve_editor(&mut self, ui: &mut Ui) {
        group(ui, "Curva do movimento");
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (_, name, c) in Ease::PRESETS {
                let on = self.s.curve == c;
                let txt = egui::RichText::new(name).size(T_META).color(if on { TEXT } else { TEXT_2 });
                if ui.add(egui::Button::new(txt).selected(on).small()).clicked() {
                    self.s.curve = c;
                }
            }
        });
        ui.add_space(6.0);
        let shown = self.s.curve.unwrap_or(SOFT_LIKE);
        let w_ = SIDE_W;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w_, 190.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect(rect, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let plot = rect.shrink2(egui::vec2(14.0, 10.0));
        let span = EASE_Y_MAX - EASE_Y_MIN;
        let to = |x: f64, y: f64| egui::pos2(plot.left() + plot.width() * x as f32, plot.bottom() - plot.height() * ((y - EASE_Y_MIN) / span) as f32);
        let from = |q: Pos2| (((q.x - plot.left()) / plot.width()) as f64, EASE_Y_MIN + ((plot.bottom() - q.y) / plot.height()) as f64 * span);
        // Quadro de 0 a 1 (onde a área começa e termina) com grade.
        let unit = Rect::from_two_pos(to(0.0, 0.0), to(1.0, 1.0));
        p.rect_filled(unit, CornerRadius::ZERO, Color32::from_white_alpha(6));
        for i in 1..4 {
            let f = i as f64 / 4.0;
            p.vline(to(f, 0.0).x, unit.y_range(), Stroke::new(1.0, Color32::from_white_alpha(10)));
            p.hline(unit.x_range(), to(0.0, f).y, Stroke::new(1.0, Color32::from_white_alpha(10)));
        }
        p.rect_stroke(unit, CornerRadius::ZERO, Stroke::new(1.0, LINE), StrokeKind::Inside);
        p.text(unit.left_top() + egui::vec2(4.0, 2.0), Align2::LEFT_TOP, "chegou", mono(T_META), TEXT_3);
        p.text(unit.right_bottom() - egui::vec2(4.0, 2.0), Align2::RIGHT_BOTTOM, "tempo →", mono(T_META), TEXT_3);
        // Curva (o que vale de verdade: a suave padrão quando nenhuma foi escolhida).
        let pts: Vec<Pos2> = (0..=64)
            .map(|i| {
                let x = i as f64 / 64.0;
                to(x, flaxy_core::clips::ease_with(self.s.curve, x))
            })
            .collect();
        let (c1, c2) = (to(shown.x1, shown.y1), to(shown.x2, shown.y2));
        let handle = Stroke::new(1.0, TEXT_3);
        p.line_segment([to(0.0, 0.0), c1], handle);
        p.line_segment([to(1.0, 1.0), c2], handle);
        p.add(egui::Shape::line(pts, Stroke::new(2.0, ACCENT)));
        p.circle_filled(to(0.0, 0.0), 3.0, TEXT_2);
        p.circle_filled(to(1.0, 1.0), 3.0, TEXT_2);
        // Bolinha andando na curva, no tempo do deslize, com uma pausa no fim.
        let secs = self.s.smooth_secs.max(0.1);
        let cycle = secs + 0.7;
        let ph = (ui.input(|i| i.time) % cycle) / secs;
        let x = ph.min(1.0);
        let y = flaxy_core::clips::ease_with(self.s.curve, x);
        p.circle_filled(to(x, y), 4.0, TEXT);
        ui.ctx().request_repaint();
        // Pontos de controle.
        let hover = resp.hover_pos();
        let near = |q: Pos2| -> Option<usize> {
            let (d1, d2) = (q.distance(c1), q.distance(c2));
            if d1.min(d2) > 14.0 {
                None
            } else if d1 <= d2 {
                Some(0)
            } else {
                Some(1)
            }
        };
        let gold = Color32::from_rgb(240, 190, 60);
        for (i, c) in [c1, c2].into_iter().enumerate() {
            let hot = self.curve_drag == Some(i) || hover.and_then(near) == Some(i);
            p.circle(c, if hot { 7.0 } else { 5.5 }, gold, Stroke::new(1.5, Color32::BLACK));
        }
        if self.curve_drag.is_some() || hover.and_then(near).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if resp.drag_started() {
            self.curve_drag = ui.input(|i| i.pointer.press_origin()).and_then(near);
        }
        if let (Some(i), Some(q)) = (self.curve_drag, resp.interact_pointer_pos().filter(|_| resp.dragged())) {
            let (x, y) = from(q);
            let mut e = shown;
            if i == 0 {
                (e.x1, e.y1) = (x, y);
            } else {
                (e.x2, e.y2) = (x, y);
            }
            // Arredonda para não salvar ruído de pixel.
            let r = |v: f64| (v * 100.0).round() / 100.0;
            let e = Ease::new(r(e.x1), r(e.y1), r(e.x2), r(e.y2)).clamped();
            self.s.curve = Some(e);
        }
        if resp.drag_stopped() {
            self.curve_drag = None;
        }
        // Faixa do movimento: a área indo de um lado ao outro com essa curva.
        ui.add_space(4.0);
        let (track, _) = ui.allocate_exact_size(egui::vec2(w_, 18.0), Sense::hover());
        let tp = ui.painter_at(track);
        tp.rect_filled(track, CornerRadius::same(4), BG_DEEP);
        let inner = track.shrink2(egui::vec2(12.0, 4.0));
        let lo = inner.left() + inner.width() * 0.15;
        let hi = inner.left() + inner.width() * 0.85;
        tp.vline(lo, inner.y_range(), Stroke::new(1.0, TEXT_3));
        tp.vline(hi, inner.y_range(), Stroke::new(1.0, TEXT_3));
        let bx = lo + (hi - lo) * y as f32;
        tp.rect_filled(Rect::from_center_size(egui::pos2(bx, inner.center().y), egui::vec2(16.0, inner.height())), CornerRadius::same(3), ACCENT);
        // Valores exatos.
        ui.add_space(6.0);
        let mut e = shown;
        let mut ch = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (label, v, lo, hi) in [("x1", &mut e.x1, 0.0, 1.0), ("y1", &mut e.y1, EASE_Y_MIN, EASE_Y_MAX), ("x2", &mut e.x2, 0.0, 1.0), ("y2", &mut e.y2, EASE_Y_MIN, EASE_Y_MAX)] {
                ui.label(w::mono_text(label, T_META, TEXT_3));
                ch |= ui.add(egui::DragValue::new(v).range(lo..=hi).speed(0.01).fixed_decimals(2)).changed();
            }
        });
        if ch {
            self.s.curve = Some(e.clamped());
        }
        ui.add_space(2.0);
        ui.label(w::muted("Arraste os pontos amarelos. Acima do quadro a área passa do ponto e volta (mola)"));
    }

    /// Início, fim e cortes de dentro do clipe.
    fn trim_panel(&mut self, ui: &mut Ui) {
        w::label(ui, "Trecho");
        let pos = self.player.pos;
        for (label, is_start) in [("Início", true), ("Fim", false)] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_sized(egui::vec2(46.0, 20.0), egui::Label::new(w::dim(label)));
                let v = if is_start { self.s.start } else { self.s.end };
                ui.add_sized(egui::vec2(74.0, 20.0), egui::Label::new(w::mono_text(timecode(v, false), T_META, TEXT_2)));
                let mut nv = None;
                if w::button(ui, Kind::Default, None, "−0,5").clicked() {
                    nv = Some(v - 0.5);
                }
                if w::button(ui, Kind::Default, None, "+0,5").clicked() {
                    nv = Some(v + 0.5);
                }
                if w::button(ui, Kind::Ghost, None, "Na agulha").on_hover_text(if is_start { "O clipe começa onde a agulha está (I)" } else { "O clipe termina onde a agulha está (O)" }).clicked() {
                    nv = Some(pos);
                }
                if let Some(t) = nv {
                    if is_start { self.set_start(t) } else { self.set_end(t) }
                }
            });
            ui.add_space(4.0);
        }
        ui.add_space(8.0);
        ui.label(w::dim("Cortes de dentro do clipe"));
        ui.add_space(4.0);
        let inside = self.cut_at(self.player.pos);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let label = if self.cut_mark.is_some() { "Fim do corte aqui" } else { "Início do corte aqui" };
            if w::button(ui, Kind::Default, Some(ic::SCISSORS), label).on_hover_text("Marque o começo e depois o fim do trecho que sai (C)").clicked() {
                self.toggle_cut_mark();
            }
            if self.cut_mark.is_some() && w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                self.cut_mark = None;
            }
            ui.add_enabled_ui(inside.is_some(), |ui| {
                if w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Desfazer corte").on_hover_text("Devolve o trecho cortado onde a agulha está").clicked() {
                    if let Some(i) = inside {
                        self.s.cuts.remove(i);
                    }
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(w::dim("Pausas acima de"));
            ui.add(egui::Slider::new(&mut self.pause_gap, 0.3..=2.0).step_by(0.1).show_value(false));
            ui.label(w::mono_text(format!("{:.1} s", self.pause_gap), T_META, TEXT_3));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let can = !self.words.is_empty();
            ui.add_enabled_ui(can, |ui| {
                if w::button(ui, Kind::Default, None, "Cortar pausas").on_hover_text(if can { "Tira os silêncios entre as falas, deixando um respiro curto" } else { "Precisa da transcrição do vídeo" }).clicked() {
                    let words = self.words_now();
                    let found = pause_cuts(&words, self.s.start, self.s.end, self.pause_gap);
                    self.s.cuts.extend(found);
                    self.normalize_cuts();
                }
            });
            ui.add_enabled_ui(!self.s.cuts.is_empty(), |ui| {
                if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Limpar cortes").clicked() {
                    self.s.cuts.clear();
                }
            });
        });
        ui.add_space(4.0);
        let kept = kept_ranges(self.s.start, self.s.end, &self.s.cuts);
        let cut = (self.s.end - self.s.start) - kept_len(&kept);
        ui.label(w::muted(if self.s.cuts.is_empty() {
            "Nenhum corte. Os cortes aparecem em vermelho na linha do tempo e são pulados ao tocar".to_string()
        } else {
            format!("{} cortes · {:.1} s a menos · clipe final {}", self.s.cuts.len(), cut, timecode(kept_len(&kept), false))
        }));
    }

    fn caption_panel(&mut self, ui: &mut Ui) {
        group(ui, "Modelo");
        self.style_picker(ui);
        let Some(base) = self.style.clone() else {
            ui.add_space(8.0);
            ui.add(egui::Label::new(w::muted("Escolha um modelo para pôr legenda no clipe. Vale para todos os clipes deste vídeo")).wrap());
            return;
        };
        ui.add(egui::Label::new(w::muted("Vale para todos os clipes deste vídeo")).wrap());
        gap(ui);
        let t = self.now();
        let cur = self.cap_at(t);
        let mut adj = cur.clone().unwrap_or_else(|| CaptionAdjust::from_style(&base));
        let mut ch = false;
        group(ui, "Posição");
        ch |= w::seg(ui, &mut adj.position, &[("top".to_string(), "Em cima"), ("center".to_string(), "Meio"), ("bottom".to_string(), "Embaixo")]);
        ui.add_space(12.0);
        let row = |ui: &mut Ui, name: &str, slider: egui::Slider, value: String, enabled: bool| -> bool {
            let mut c = false;
            ui.horizontal(|ui| {
                ui.add_sized(egui::vec2(64.0, 20.0), egui::Label::new(w::dim(name)));
                ui.spacing_mut().slider_width = SIDE_W - 64.0 - 56.0;
                ui.add_enabled_ui(enabled, |ui| c = ui.add(slider.show_value(false)).changed());
                ui.label(w::mono_text(value, T_META, TEXT_3));
            });
            c
        };
        let (m, s) = (adj.margin * 100.0, adj.size / base.size.max(1.0) * 100.0);
        let center = adj.position == "center";
        ch |= row(ui, "Distância", egui::Slider::new(&mut adj.margin, 0.0..=0.45).step_by(0.01), format!("{m:.0}%"), !center);
        ui.add_space(4.0);
        ch |= row(ui, "Tamanho", egui::Slider::new(&mut adj.size, 24.0..=240.0).step_by(2.0), format!("{s:.0}%"), true);
        if ch {
            self.set_cap(t, Some(adj));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(w::muted("Ou arraste a legenda na saída"));
            if cur.is_some_and(|c| c != CaptionAdjust::from_style(&base)) && w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Usar a do modelo").clicked() {
                self.set_cap(t, None);
            }
        });
        gap(ui);
        group(ui, "Trocas de legenda");
        let act = self.cap_active(t);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if w::button(ui, Kind::Default, Some(ic::PLUS), "Nova troca aqui").on_hover_text("A partir deste instante a legenda pode ficar em outro lugar ou tamanho").clicked() {
                let at = (t * 10.0).round() / 10.0;
                if !self.s.cap_keys.iter().any(|k| (k.t - at).abs() < 0.2) {
                    let adjust = self.cap_at(t).unwrap_or_else(|| CaptionAdjust::from_style(&base));
                    let i = self.s.cap_keys.partition_point(|x| x.t < at);
                    self.s.cap_keys.insert(i, CaptionKey { t: at, adjust });
                }
            }
            ui.add_enabled_ui(act.is_some(), |ui| {
                if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Remover").on_hover_text("Este trecho volta a usar a legenda anterior").clicked() {
                    if let Some(i) = act {
                        self.s.cap_keys.remove(i);
                    }
                }
            });
        });
        ui.add_space(8.0);
        let n = caption_parts(self.s.start, self.s.end, self.s.caption.clone(), &self.s.cap_keys).len();
        ui.add(egui::Label::new(w::muted(if n > 1 { format!("{n} trechos de legenda. Os ajustes valem no trecho onde a agulha está") } else { "A mesma legenda no clipe todo".to_string() })).wrap());
    }

    /// Palavras do clipe para corrigir o que a transcrição errou.
    fn text_panel(&mut self, ui: &mut Ui, max_h: f32) {
        if self.words.is_empty() {
            ui.label(w::muted("Sem transcrição para este vídeo"));
            return;
        }
        ui.add(egui::Label::new(w::muted("Clique numa palavra. Vale só neste clipe")).wrap());
        ui.add_space(8.0);
        let (a, b) = (self.s.start, self.s.end);
        let list: Vec<(f64, String, Option<String>)> =
            self.words.iter().filter(|w| w.end > a && w.start < b).map(|w| (w.start, w.text.clone(), self.edit_of(w.start).map(str::to_string))).collect();
        let pos = self.player.pos;
        let mut pick: Option<(f64, String)> = None;
        egui::ScrollArea::vertical().id_salt("clip-modal-words").max_height((max_h - 120.0).max(160.0)).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for (t, orig, edit) in &list {
                    let shown = edit.clone().unwrap_or_else(|| orig.clone());
                    let gone = edit.as_deref().is_some_and(|e| e.trim().is_empty());
                    let sel = self.word_sel.as_ref().is_some_and(|s| (s.0 - t).abs() < 0.02);
                    let now = pos >= *t && pos < t + 0.4;
                    let mut txt = egui::RichText::new(if gone { orig.clone() } else { shown.clone() }).size(T_SEC);
                    txt = if gone { txt.strikethrough().color(TEXT_3) } else if edit.is_some() { txt.color(ACCENT) } else if now { txt.color(TEXT) } else { txt.color(TEXT_2) };
                    let r = ui.add(egui::Button::new(txt).selected(sel || now).small());
                    if r.clicked() {
                        pick = Some((*t, if gone { orig.clone() } else { shown }));
                    }
                }
            });
        });
        if let Some((t, text)) = pick {
            self.word_sel = Some((t, text));
            self.seek(t);
        }
        if let Some((t, mut buf)) = self.word_sel.clone() {
            ui.add_space(6.0);
            let orig = self.words.iter().find(|w| (w.start - t).abs() < 0.02).map(|w| w.text.clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(120.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if w::button(ui, Kind::Default, Some(ic::CHECK), "Aplicar").clicked() || enter {
                    let v = buf.trim().to_string();
                    self.set_word(t, if v == orig.trim() { None } else { Some(v) });
                    self.word_sel = None;
                    return;
                }
                if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Apagar").on_hover_text("Tira esta palavra da legenda").clicked() {
                    self.set_word(t, Some(String::new()));
                    self.word_sel = None;
                    return;
                }
                if self.edit_of(t).is_some() && w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Original").clicked() {
                    self.set_word(t, None);
                    self.word_sel = None;
                    return;
                }
                self.word_sel = Some((t, buf.clone()));
            });
        }
    }
}

/// Desenha o clipe como ele sai do corte: recorte da câmera (e da tela) no
/// quadro 9:16, ou o vídeo inteiro no horizontal, com a legenda por cima.
/// Encaixar: a área inteira no centro, sobre ela mesma esticada e escurecida
/// (no corte real a sobra fica desfocada). `split` = parte da tela.
#[allow(clippy::too_many_arguments)]
pub fn clip_preview(
    ui: &Ui,
    slot: Rect,
    tex: Option<(&egui::TextureHandle, f32)>,
    layout: &str,
    cam: FRect,
    scr: FRect,
    fit: AreaFit,
    split: f64,
    bg: &ClipBg,
    bg_tex: Option<&egui::TextureHandle>,
    wm: Option<&[flaxy_core::watermarks::WmText]>,
    style: Option<&CaptionStyle>,
    fonts: &[String],
    words: &[Word],
    t: f64,
) {
    let out = if layout == "horizontal" { Rect::from_center_size(slot.center(), egui::vec2(slot.width(), slot.width() * 9.0 / 16.0)) } else { slot };
    ui.painter().rect_filled(slot, CornerRadius::same(R), BG_DEEP);
    ui.painter().rect_filled(out, CornerRadius::same(R), Color32::BLACK);
    let pt = ui.painter_at(out);
    if let Some((tex, aspect)) = tex {
        let place = |dst: Rect, r: FRect, fit: bool, at: Place| {
            let pt = ui.painter_at(dst);
            let v = visible(r, aspect, dst.width() / dst.height());
            let uv = |r: FRect| Rect::from_min_size(egui::pos2(r.x as f32, r.y as f32), egui::vec2(r.w as f32, r.h as f32));
            if fit {
                // Fundo: a prévia não desfoca; escurece mais quanto mais forte o desfoque.
                match (bg, bg_tex) {
                    (ClipBg::Blur, _) => {
                        pt.image(tex.id(), dst, uv(v), Color32::from_gray(70));
                    }
                    (ClipBg::Soft, _) => {
                        pt.image(tex.id(), dst, uv(v), Color32::from_gray(140));
                    }
                    (ClipBg::File(_), Some(b)) => crate::media::paint_image(ui, b, dst, true),
                    _ => {
                        pt.rect_filled(dst, CornerRadius::ZERO, Color32::BLACK);
                    }
                }
                pt.image(tex.id(), placed_rect(dst, area_aspect(r, aspect), at), uv(r), Color32::WHITE);
            } else {
                pt.image(tex.id(), dst, uv(v), Color32::WHITE);
            }
        };
        if layout == "horizontal" {
            pt.image(tex.id(), fit_rect(out, aspect), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        for (k, dst) in out_slots(out, layout, split) {
            match k {
                Key::Camera => place(dst, cam, fit.camera, fit.cam_place),
                Key::Screen => place(dst, scr, fit.screen, fit.scr_place),
            }
        }
    }
    if let Some(items) = wm {
        caption_preview::paint_watermark(&pt, out, items, fonts);
    }
    if let Some(st) = style {
        caption_preview::paint_at(&pt, out, st, fonts, words, t);
    }
    ui.painter().rect_stroke(slot, CornerRadius::same(R), Stroke::new(1.0, LINE), StrokeKind::Inside);
}
