//! Marca câmera e tela sobre um quadro do vídeo. A sugestão vem do núcleo;
//! o usuário só corrige. Proporção travada evita que o recorte distorça.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind};
use flaxy_core::clips::Rect as FRect;

use crate::icons as ic;
use crate::media::{fit_rect, Player};
use crate::theme::*;
use crate::widgets::{self as w, Kind};

#[derive(Clone, Copy, PartialEq)]
enum Key {
    Camera,
    Screen,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Move,
    NW,
    SE,
}

struct BoxDef {
    key: Key,
    label: &'static str,
    color: Color32,
    /// Largura/altura em px de saída.
    ratio: f64,
}

fn boxes(layout: &str) -> Vec<BoxDef> {
    match layout {
        "vertical_split" => vec![
            BoxDef { key: Key::Screen, label: "Tela · em cima", color: TEXT, ratio: 1080.0 / 1152.0 },
            BoxDef { key: Key::Camera, label: "Câmera · embaixo", color: ACCENT, ratio: 1080.0 / 768.0 },
        ],
        "vertical_camera" => vec![BoxDef { key: Key::Camera, label: "Câmera", color: ACCENT, ratio: 9.0 / 16.0 }],
        _ => vec![],
    }
}

pub enum Outcome {
    Open,
    Save(FRect, FRect),
    Close,
}

pub struct RectEditor {
    pub clip_id: i64,
    layout: String,
    camera: FRect,
    screen: FRect,
    player: Player,
    lock: bool,
    drag: Option<(Key, Mode, FRect, Pos2)>,
}

impl RectEditor {
    pub fn new(clip_id: i64, src: &std::path::Path, at: f64, layout: &str, camera: FRect, screen: FRect) -> Self {
        Self { clip_id, layout: layout.to_string(), camera, screen, player: Player::new(src, at, (1280, 720)), lock: true, drag: None }
    }

    fn get(&self, k: Key) -> FRect {
        if k == Key::Camera { self.camera } else { self.screen }
    }

    fn set(&mut self, k: Key, r: FRect) {
        if k == Key::Camera { self.camera = r } else { self.screen = r }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        self.player.update(ctx);
        let mut out = Outcome::Open;
        let (open, _) = w::modal(ctx, "rect-editor", "Marcar câmera e tela", "A IA sugeriu as áreas. Arraste para corrigir", 1080.0, |ui| {
            w::pad(ui, |ui| {
                let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 560.0), Sense::hover());
                ui.painter().rect_filled(outer, CornerRadius::same(R), Color32::BLACK);
                let img = fit_rect(outer, self.player.aspect());
                if let Some(t) = self.player.texture() {
                    ui.painter().image(t.id(), img, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    ui.painter().text(outer.center(), Align2::CENTER_CENTER, "Carregando quadro", body(T_SEC), TEXT_3);
                }
                let src_ar = self.player.aspect() as f64;
                let defs = boxes(&self.layout);
                let to_screen = |r: FRect| {
                    Rect::from_min_size(
                        egui::pos2(img.left() + img.width() * r.x as f32, img.top() + img.height() * r.y as f32),
                        egui::vec2(img.width() * r.w as f32, img.height() * r.h as f32),
                    )
                };
                for b in &defs {
                    let r = to_screen(self.get(b.key));
                    let p = ui.painter();
                    p.rect(r, CornerRadius::ZERO, Color32::from_black_alpha(40), Stroke::new(2.0, b.color), StrokeKind::Inside);
                    let tag = p.layout_no_wrap(b.label.to_string(), mono(T_META), TEXT);
                    let tr = Rect::from_min_size(r.left_top() + egui::vec2(4.0, 4.0), tag.size() + egui::vec2(12.0, 4.0));
                    p.rect_filled(tr, CornerRadius::same(4), Color32::from_black_alpha(200));
                    p.galley(tr.min + egui::vec2(6.0, 2.0), tag, TEXT);
                    for c in [r.left_top(), r.right_bottom()] {
                        p.rect_filled(Rect::from_center_size(c, egui::vec2(12.0, 12.0)), CornerRadius::same(2), b.color);
                    }
                }

                let resp = ui.interact(img.expand(8.0), egui::Id::unique("rect-editor-stage"), Sense::drag());
                if resp.drag_started() {
                    if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                        // Alças primeiro, depois o corpo; a última caixa desenhada fica por cima.
                        self.drag = defs.iter().rev().find_map(|b| {
                            let r = to_screen(self.get(b.key));
                            if r.left_top().distance(pos) < 10.0 {
                                Some((b.key, Mode::NW, self.get(b.key), pos))
                            } else if r.right_bottom().distance(pos) < 10.0 {
                                Some((b.key, Mode::SE, self.get(b.key), pos))
                            } else {
                                r.contains(pos).then(|| (b.key, Mode::Move, self.get(b.key), pos))
                            }
                        });
                    }
                }
                if let (Some((k, mode, r0, from)), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
                    if resp.dragged() {
                        let ratio = defs.iter().find(|b| b.key == k).map(|b| b.ratio / src_ar);
                        let dx = ((pos.x - from.x) / img.width()) as f64;
                        let dy = ((pos.y - from.y) / img.height()) as f64;
                        let FRect { mut x, mut y, mut w, mut h } = r0;
                        match mode {
                            Mode::Move => {
                                x = (x + dx).clamp(0.0, 1.0 - w);
                                y = (y + dy).clamp(0.0, 1.0 - h);
                            }
                            Mode::SE | Mode::NW => {
                                if mode == Mode::SE {
                                    w = (w + dx).clamp(0.05, 1.0 - x);
                                    h = (h + dy).clamp(0.05, 1.0 - y);
                                } else {
                                    let nx = (x + dx).clamp(0.0, x + w - 0.05);
                                    let ny = (y + dy).clamp(0.0, y + h - 0.05);
                                    w += x - nx;
                                    h += y - ny;
                                    x = nx;
                                    y = ny;
                                }
                                if let (true, Some(fr)) = (self.lock, ratio) {
                                    h = w / fr;
                                    if y + h > 1.0 {
                                        h = 1.0 - y;
                                        w = h * fr;
                                    }
                                    if x + w > 1.0 {
                                        w = 1.0 - x;
                                        h = w / fr;
                                    }
                                }
                            }
                        }
                        self.set(k, FRect { x, y, w, h });
                    }
                }
                if resp.drag_stopped() {
                    self.drag = None;
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let (icon, txt) = if self.lock { (ic::LOCK, "Proporção travada") } else { (ic::LOCK_OPEN, "Proporção livre") };
                    if w::button(ui, Kind::Default, Some(icon), txt).clicked() {
                        self.lock = !self.lock;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if w::button(ui, Kind::Primary, None, "Aplicar").clicked() {
                            out = Outcome::Save(self.camera, self.screen);
                        }
                        if w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                            out = Outcome::Close;
                        }
                    });
                });
            });
        });
        if !open {
            return Outcome::Close;
        }
        out
    }
}
