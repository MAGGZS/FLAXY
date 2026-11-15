//! Editor de um vídeo do lote: recorte do quadro (fração 0..1 da fonte) e
//! corte no tempo (entrada/saída). Atalhos: Espaço toca/pausa, I entrada, O saída.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::models::{ItemOverrides, Template};

use crate::app::Shared;
use crate::format::timecode;
use crate::icons as ic;
use crate::media::{self, fit_rect, Player};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use crate::theme::*;
use crate::widgets::{self as w, Kind};

#[derive(Clone, Copy, PartialEq)]
enum Handle {
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

impl Handle {
    fn has(self, c: char) -> bool {
        let s = match self {
            Handle::Move => "",
            Handle::N => "n",
            Handle::S => "s",
            Handle::E => "e",
            Handle::W => "w",
            Handle::NE => "ne",
            Handle::NW => "nw",
            Handle::SE => "se",
            Handle::SW => "sw",
        };
        s.contains(c)
    }
    fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match self {
            Handle::Move => Move,
            Handle::N | Handle::S => ResizeVertical,
            Handle::E | Handle::W => ResizeHorizontal,
            Handle::NE | Handle::SW => ResizeNeSw,
            Handle::NW | Handle::SE => ResizeNwSe,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TrimDrag {
    In,
    Out,
    Seek,
}

pub enum Outcome {
    Open,
    Save(ItemOverrides),
    Close,
}

pub struct ClipEditor {
    pub name: String,
    o: ItemOverrides,
    player: Player,
    path: PathBuf,
    wave: Option<Vec<f32>>,
    wave_rx: Option<Receiver<Option<Vec<f32>>>>,
    lock: bool,
    /// Proporção (largura/altura) da área do vídeo no modelo.
    target: Option<f64>,
    duration_hint: f64,
    drag: Option<(Handle, ItemOverrides, Pos2)>,
    trim: Option<TrimDrag>,
}

const FULL: ItemOverrides = ItemOverrides {
    crop_x: 0.0,
    crop_y: 0.0,
    crop_w: 1.0,
    crop_h: 1.0,
    overlay_x: None,
    overlay_y: None,
    overlay_w: None,
    overlay_h: None,
    fit_mode: None,
    trim_start: None,
    trim_end: None,
    audio_path: None,
    audio_volume: None,
    original_volume: None,
    audio_offset: None,
    audio_fade: None,
    caption_style_id: None,
    transcript_id: None,
};

/// Recorte ou corte no tempo diferente do padrão.
pub fn is_adjusted(o: &ItemOverrides) -> bool {
    o.crop_x != 0.0 || o.crop_y != 0.0 || o.crop_w != 1.0 || o.crop_h != 1.0 || o.fit_mode.is_some() || o.trim_start.is_some() || o.trim_end.is_some()
}

impl ClipEditor {
    pub fn new(path: &std::path::Path, name: &str, duration: f64, value: &ItemOverrides, template: Option<&Template>) -> Self {
        let player = Player::new(path, value.trim_start.unwrap_or(0.0) + 0.05, (1280, 720));
        Self {
            name: name.to_string(),
            o: value.clone(),
            player,
            path: path.to_path_buf(),
            wave: None,
            wave_rx: Some(media::peaks(path, 480)),
            lock: true,
            target: template.map(|t| t.overlay_w as f64 / t.overlay_h.max(1) as f64),
            duration_hint: duration,
            drag: None,
            trim: None,
        }
    }

    fn dur(&self) -> f64 {
        let d = self.player.duration();
        if d > 0.0 { d } else { self.duration_hint }
    }

    fn tin(&self) -> f64 {
        self.o.trim_start.unwrap_or(0.0)
    }

    fn tout(&self) -> f64 {
        self.o.trim_end.unwrap_or(self.dur())
    }

    /// Proporção do modelo em fração da fonte.
    fn frac_ratio(&self) -> Option<f64> {
        let m = self.player.meta?;
        let t = self.target?;
        Some(t / (m.width as f64 / m.height.max(1) as f64))
    }

    fn set_in(&mut self, t: f64) {
        let v = t.clamp(0.0, (self.tout() - 0.1).max(0.0));
        self.o.trim_start = if v > 0.001 { Some(v) } else { None };
    }

    fn set_out(&mut self, t: f64) {
        let d = self.dur();
        let v = t.clamp(self.tin() + 0.1, d.max(self.tin() + 0.1));
        self.o.trim_end = if v >= d - 0.01 { None } else { Some(v) };
    }

    fn fit_template(&mut self) {
        let Some(fr) = self.frac_ratio() else { return };
        let (mut cw, mut ch) = (1.0, 1.0 / fr);
        if ch > 1.0 {
            ch = 1.0;
            cw = fr;
        }
        self.o.crop_x = (1.0 - cw) / 2.0;
        self.o.crop_y = (1.0 - ch) / 2.0;
        self.o.crop_w = cw;
        self.o.crop_h = ch;
    }

    fn toggle(&mut self) {
        if self.player.is_playing() {
            self.player.pause();
        } else {
            if self.player.pos < self.tin() || self.player.pos >= self.tout() - 0.05 {
                self.player.seek(self.tin());
            }
            self.player.stop_at = Some(self.tout());
            self.player.play();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, sh: &mut Shared) -> Outcome {
        self.player.update(ctx);
        if let Some(rx) = &self.wave_rx {
            if let Ok(w) = rx.try_recv() {
                self.wave = w;
                self.wave_rx = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if !ctx.egui_wants_keyboard_input() {
            let (space, i, o) = ctx.input(|inp| (inp.key_pressed(egui::Key::Space), inp.key_pressed(egui::Key::I), inp.key_pressed(egui::Key::O)));
            if space {
                self.toggle();
            }
            if i {
                self.set_in(self.player.pos);
            }
            if o {
                self.set_out(self.player.pos);
            }
        }
        let meta = format!("{} de {}", timecode(self.tout() - self.tin(), true), timecode(self.dur(), true));
        let name = self.name.clone();
        let mut out = Outcome::Open;
        let (open, _) = w::modal(ctx, "clip-editor", &name, &meta, 1180.0, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let left_w = ui.available_width() - 300.0;
                w::column(ui, left_w, |ui| {
                    w::pad(ui, |ui| {
                        self.stage(ui);
                        ui.add_space(12.0);
                        self.timeline(ui, sh);
                    });
                });
                let r = ui.available_rect_before_wrap();
                ui.painter().vline(r.left(), r.y_range(), Stroke::new(1.0, LINE));
                w::column(ui, 300.0, |ui| {
                    w::pad(ui, |ui| {
                        if let Some(o) = self.controls(ui) {
                            out = o;
                        }
                    });
                });
            });
        });
        if !open {
            self.player.pause();
            return Outcome::Close;
        }
        out
    }

    fn stage(&mut self, ui: &mut Ui) {
        let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 500.0), Sense::hover());
        ui.painter().rect_filled(outer, CornerRadius::same(R), Color32::BLACK);
        let img = fit_rect(outer, self.player.aspect());
        match self.player.texture() {
            Some(t) => {
                ui.painter().image(t.id(), img, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                let msg = if self.player.failed() { "Não foi possível ler este vídeo" } else { "Carregando quadro" };
                ui.painter().text(outer.center(), Align2::CENTER_CENTER, msg, body(T_SEC), TEXT_3);
                return;
            }
        }

        let o = &self.o;
        let crop = Rect::from_min_size(
            egui::pos2(img.left() + img.width() * o.crop_x as f32, img.top() + img.height() * o.crop_y as f32),
            egui::vec2(img.width() * o.crop_w as f32, img.height() * o.crop_h as f32),
        );
        let p = ui.painter_at(img);
        // Sombra fora do recorte.
        for r in [
            Rect::from_min_max(img.min, egui::pos2(img.right(), crop.top())),
            Rect::from_min_max(egui::pos2(img.left(), crop.bottom()), img.max),
            Rect::from_min_max(egui::pos2(img.left(), crop.top()), egui::pos2(crop.left(), crop.bottom())),
            Rect::from_min_max(egui::pos2(crop.right(), crop.top()), egui::pos2(img.right(), crop.bottom())),
        ] {
            p.rect_filled(r, CornerRadius::ZERO, SHADE);
        }
        p.rect_stroke(crop, CornerRadius::ZERO, Stroke::new(1.0, TEXT), StrokeKind::Inside);
        for k in 1..3 {
            let f = k as f32 / 3.0;
            p.vline(crop.left() + crop.width() * f, crop.y_range(), Stroke::new(1.0, Color32::from_white_alpha(60)));
            p.hline(crop.x_range(), crop.top() + crop.height() * f, Stroke::new(1.0, Color32::from_white_alpha(60)));
        }
        let handles = [
            (Handle::NW, crop.left_top()),
            (Handle::N, crop.center_top()),
            (Handle::NE, crop.right_top()),
            (Handle::E, crop.right_center()),
            (Handle::SE, crop.right_bottom()),
            (Handle::S, crop.center_bottom()),
            (Handle::SW, crop.left_bottom()),
            (Handle::W, crop.left_center()),
        ];
        let paint = ui.painter();
        for (_, c) in handles {
            paint.rect_filled(Rect::from_center_size(c, egui::vec2(10.0, 10.0)), CornerRadius::same(2), TEXT);
        }

        let resp = ui.interact(img.expand(8.0), egui::Id::unique("clip-editor-crop"), Sense::click_and_drag());
        let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
        let hit = |pos: Pos2| -> Option<Handle> {
            handles.iter().find(|(_, c)| c.distance(pos) < 10.0).map(|h| h.0).or_else(|| crop.contains(pos).then_some(Handle::Move))
        };
        if let Some((h, _, _)) = self.drag {
            ui.ctx().set_cursor_icon(h.cursor());
        } else if let Some(pos) = pointer {
            if let Some(h) = hit(pos) {
                ui.ctx().set_cursor_icon(h.cursor());
            }
        }
        if resp.drag_started() {
            if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                self.drag = hit(pos).map(|h| (h, self.o.clone(), pos));
            }
        }
        if resp.dragged() {
            if let (Some((h, start, from)), Some(pos)) = (self.drag.clone(), resp.interact_pointer_pos()) {
                let dx = ((pos.x - from.x) / img.width()) as f64;
                let dy = ((pos.y - from.y) / img.height()) as f64;
                let (mut x, mut y, mut cw, mut ch) = (start.crop_x, start.crop_y, start.crop_w, start.crop_h);
                if h == Handle::Move {
                    x = (x + dx).clamp(0.0, 1.0 - cw);
                    y = (y + dy).clamp(0.0, 1.0 - ch);
                } else {
                    if h.has('w') {
                        let nx = (x + dx).clamp(0.0, x + cw - 0.05);
                        cw += x - nx;
                        x = nx;
                    }
                    if h.has('e') {
                        cw = (cw + dx).clamp(0.05, 1.0 - x);
                    }
                    if h.has('n') {
                        let ny = (y + dy).clamp(0.0, y + ch - 0.05);
                        ch += y - ny;
                        y = ny;
                    }
                    if h.has('s') {
                        ch = (ch + dy).clamp(0.05, 1.0 - y);
                    }
                    if let (true, Some(fr)) = (self.lock, self.frac_ratio()) {
                        // O lado arrastado manda; o outro segue a proporção do modelo.
                        if h == Handle::N || h == Handle::S {
                            cw = ch * fr;
                        } else {
                            ch = cw / fr;
                        }
                        if cw > 1.0 - x {
                            cw = 1.0 - x;
                            ch = cw / fr;
                        }
                        if ch > 1.0 - y {
                            ch = 1.0 - y;
                            cw = ch * fr;
                        }
                    }
                }
                self.o.crop_x = x;
                self.o.crop_y = y;
                self.o.crop_w = cw;
                self.o.crop_h = ch;
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
    }

    /// Timeline (momento da direção): régua, quadros do vídeo e forma de onda,
    /// com o trecho mantido marcado em cobalto e alças de entrada e saída.
    fn timeline(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let dur = self.dur().max(0.001);
        let (tin, tout, pos) = (self.tin(), self.tout(), self.player.pos);
        ui.horizontal(|ui| {
            let icon = if self.player.is_playing() { ic::PAUSE } else { ic::PLAY };
            if w::button(ui, Kind::Default, Some(icon), &timecode(pos, true)).on_hover_text("Tocar ou pausar").clicked() {
                self.toggle();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(w::mono_text(format!("entrada {} · saída {}", timecode(tin, true), timecode(tout, true)), T_META, TEXT_3));
            });
        });
        ui.add_space(8.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), Sense::click_and_drag());
        let p = ui.painter_at(rect.expand(8.0));
        p.rect(rect, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        let ruler = Rect::from_min_size(inner.min, egui::vec2(inner.width(), 16.0));
        w::ruler(ui, ruler, 12.0);
        let strip = Rect::from_min_max(egui::pos2(inner.left(), ruler.bottom()), egui::pos2(inner.right(), ruler.bottom() + 48.0));
        let wave = Rect::from_min_max(egui::pos2(inner.left(), strip.bottom() + 4.0), egui::pos2(inner.right(), inner.bottom() - 4.0));
        let x = |t: f64| inner.left() + inner.width() * (t / dur) as f32;

        // Quadros do vídeo ao longo do tempo.
        let cell_w = strip.height() * self.player.aspect().clamp(0.5, 2.4);
        let n = ((strip.width() / cell_w).ceil() as usize).clamp(1, 24);
        let cw = strip.width() / n as f32;
        for i in 0..n {
            let cell = Rect::from_min_size(egui::pos2(strip.left() + cw * i as f32, strip.top()), egui::vec2(cw, strip.height()));
            let t = (i as f64 + 0.5) / n as f64 * dur;
            if let Some(tex) = sh.thumbs.get(&sh.bk, ui.ctx(), &self.path, t, (192, 108)) {
                let pp = ui.painter_at(cell.shrink2(egui::vec2(0.5, 0.0)));
                let [tw, th] = tex.size();
                let sc = (cell.width() / tw as f32).max(cell.height() / th as f32);
                pp.image(tex.id(), Rect::from_center_size(cell.center(), egui::vec2(tw as f32 * sc, th as f32 * sc)), Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.rect_filled(cell.shrink2(egui::vec2(0.5, 0.0)), CornerRadius::ZERO, RAISED);
            }
        }

        // Forma de onda: dentro do trecho em texto secundário, fora em apagado.
        if let Some(peaks) = &self.wave {
            let n = peaks.len().max(1);
            let bw = wave.width() / n as f32;
            let mid = wave.center().y;
            for (i, v) in peaks.iter().enumerate() {
                let t = (i as f64 + 0.5) / n as f64 * dur;
                let h = (v * wave.height()).max(1.0);
                let xx = wave.left() + bw * i as f32 + bw / 2.0;
                let c = if t >= tin && t <= tout { TEXT_2 } else { TEXT_3.gamma_multiply(0.5) };
                p.line_segment([egui::pos2(xx, mid - h / 2.0), egui::pos2(xx, mid + h / 2.0)], Stroke::new(bw.clamp(1.0, 2.0), c));
            }
        } else {
            p.hline(wave.x_range(), wave.center().y, Stroke::new(1.0, LINE_2));
        }

        // Fora do trecho fica escuro; o trecho ganha contorno cobalto.
        let body_y = strip.top()..=inner.bottom();
        for r in [
            Rect::from_x_y_ranges(inner.left()..=x(tin), body_y.clone()),
            Rect::from_x_y_ranges(x(tout)..=inner.right(), body_y.clone()),
        ] {
            if r.width() > 0.0 {
                p.rect_filled(r, CornerRadius::ZERO, SHADE);
            }
        }
        let kept = Rect::from_x_y_ranges(x(tin)..=x(tout), body_y.clone());
        p.rect_stroke(kept, CornerRadius::same(4), Stroke::new(2.0, ACCENT), StrokeKind::Inside);
        for t in [tin, tout] {
            let hr = Rect::from_center_size(egui::pos2(x(t), kept.center().y), egui::vec2(8.0, kept.height()));
            p.rect_filled(hr, CornerRadius::same(4), ACCENT);
            p.vline(hr.center().x, hr.center().y - 8.0..=hr.center().y + 8.0, Stroke::new(1.0, TEXT));
        }
        // Cursor de reprodução com marcador na régua.
        let px = x(pos);
        p.vline(px, inner.y_range(), Stroke::new(1.0, TEXT));
        p.add(egui::Shape::convex_polygon(vec![egui::pos2(px - 4.0, inner.top()), egui::pos2(px + 4.0, inner.top()), egui::pos2(px, inner.top() + 6.0)], TEXT, Stroke::NONE));

        let at = |px: f32| ((px - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64 * dur;
        if let Some(pp) = resp.hover_pos() {
            if (pp.x - x(tin)).abs() < 8.0 || (pp.x - x(tout)).abs() < 8.0 {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        }
        if resp.drag_started() || resp.clicked() {
            if let Some(pp) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                self.trim = Some(if (pp.x - x(tin)).abs() < 8.0 {
                    TrimDrag::In
                } else if (pp.x - x(tout)).abs() < 8.0 {
                    TrimDrag::Out
                } else {
                    TrimDrag::Seek
                });
            }
        }
        if let (Some(which), Some(pp)) = (self.trim, resp.interact_pointer_pos()) {
            if resp.dragged() || resp.clicked() || resp.drag_started() {
                let t = at(pp.x);
                match which {
                    TrimDrag::In => self.set_in(t),
                    TrimDrag::Out => self.set_out(t),
                    TrimDrag::Seek => {}
                }
                self.player.seek(t);
            }
        }
        if resp.drag_stopped() || resp.clicked() {
            self.trim = None;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(w::mono_text("00:00:00", T_META, TEXT_3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(w::mono_text(timecode(dur, false), T_META, TEXT_3));
            });
        });
    }

    fn controls(&mut self, ui: &mut Ui) -> Option<Outcome> {
        let mut out = None;
        ui.spacing_mut().item_spacing.y = 8.0;
        w::label(ui, "Corte no tempo");
        ui.horizontal(|ui| {
            if w::button(ui, Kind::Default, None, "Marcar entrada").clicked() {
                self.set_in(self.player.pos);
            }
            w::keycap(ui, "I");
        });
        ui.horizontal(|ui| {
            if w::button(ui, Kind::Default, None, "Marcar saída").clicked() {
                self.set_out(self.player.pos);
            }
            w::keycap(ui, "O");
        });
        ui.add_space(16.0);
        w::label(ui, "Recorte do quadro");
        let fr = self.frac_ratio();
        ui.add_enabled_ui(fr.is_some(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let (icon, txt) = if self.lock { (ic::LOCK, "Proporção travada") } else { (ic::LOCK_OPEN, "Proporção livre") };
                if w::button(ui, Kind::Default, Some(icon), txt).clicked() {
                    self.lock = !self.lock;
                }
                if w::button(ui, Kind::Default, None, "Proporção do modelo").clicked() {
                    self.fit_template();
                }
            });
        });
        if let Some(m) = self.player.meta {
            ui.label(w::mono_text(
                format!(
                    "{}×{} de {}×{}",
                    (self.o.crop_w * m.width as f64).round(),
                    (self.o.crop_h * m.height as f64).round(),
                    m.width,
                    m.height
                ),
                T_META,
                TEXT_3,
            ));
        }
        ui.add_space(16.0);
        w::label(ui, "Encaixe na área");
        let mut fit = self.o.fit_mode.clone();
        if w::seg(ui, &mut fit, &[(None, "Do modelo"), (Some("cover".to_string()), "Preencher"), (Some("contain".to_string()), "Caber")]) {
            self.o.fit_mode = fit;
        }
        ui.add_space(32.0);
        ui.vertical_centered_justified(|ui| {
            if w::button_sized(ui, Kind::Primary, None, "Aplicar", 36.0).clicked() {
                self.player.pause();
                out = Some(Outcome::Save(self.o.clone()));
            }
            if w::button(ui, Kind::Ghost, Some(ic::ROTATE_CCW), "Desfazer ajustes").clicked() {
                let keep = self.o.clone();
                self.o = ItemOverrides { audio_path: keep.audio_path, caption_style_id: keep.caption_style_id, transcript_id: keep.transcript_id, ..FULL };
            }
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            w::keycap(ui, "Espaço");
            ui.label(w::muted("toca"));
            ui.add_space(8.0);
            w::keycap(ui, "I");
            ui.label(w::muted("entrada"));
            ui.add_space(8.0);
            w::keycap(ui, "O");
            ui.label(w::muted("saída"));
        });
        out
    }
}
