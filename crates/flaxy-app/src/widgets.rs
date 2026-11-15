//! Peças de interface reaproveitadas: cartão, botões, controle segmentado,
//! barra de progresso, régua de tempo e avisos.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};

use crate::theme::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Default,
    Primary,
    Ghost,
    Danger,
}

/// Botão com ícone opcional. Altura 32; `button_sized` para 36/44.
pub fn button(ui: &mut Ui, kind: Kind, icon: Option<&str>, label: &str) -> Response {
    button_sized(ui, kind, icon, label, 32.0)
}

pub fn button_sized(ui: &mut Ui, kind: Kind, icon: Option<&str>, label: &str, h: f32) -> Response {
    let enabled = ui.is_enabled();
    let font = strong(T_BODY);
    let g_label = (!label.is_empty()).then(|| ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT));
    let g_icon = icon.map(|i| ui.painter().layout_no_wrap(i.to_string(), body(ICON), TEXT));
    // Ícone e texto separados por 8px fixos, 16px de respiro nas laterais.
    let content = g_label.as_ref().map(|g| g.size().x).unwrap_or(0.0)
        + g_icon.as_ref().map(|g| g.size().x).unwrap_or(0.0)
        + if g_label.is_some() && g_icon.is_some() { 8.0 } else { 0.0 };
    let w = if g_label.is_none() {
        h
    } else if ui.layout().horizontal_justify() {
        ui.available_width()
    } else {
        content + 32.0
    };
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = enabled && r.hovered();
        let (fill, stroke, color) = match kind {
            Kind::Primary if !enabled => (RAISED, Stroke::NONE, TEXT_3),
            Kind::Primary => (if hover { PRIMARY_HOVER } else { PRIMARY }, Stroke::NONE, ON_PRIMARY),
            Kind::Default => (if hover { RAISED_2 } else { RAISED }, Stroke::new(1.0, LINE_2), if enabled { TEXT } else { TEXT_3 }),
            Kind::Ghost => (if hover { RAISED } else { Color32::TRANSPARENT }, Stroke::NONE, if !enabled { TEXT_3 } else if hover { TEXT } else { TEXT_2 }),
            Kind::Danger => (if hover { RAISED } else { Color32::TRANSPARENT }, Stroke::NONE, if enabled { ERR } else { TEXT_3 }),
        };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(R), fill, stroke, StrokeKind::Inside);
        if kind == Kind::Default && enabled {
            p.hline(rect.x_range().shrink(R as f32), rect.top() + 1.0, Stroke::new(1.0, HIGHLIGHT));
        }
        let mut x = rect.center().x - content / 2.0;
        let has_label = g_label.is_some();
        if let Some(g) = g_icon {
            let icon_color = if kind == Kind::Ghost && !hover && has_label { TEXT_3 } else { color };
            let gw = g.size().x;
            p.galley_with_override_text_color(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, icon_color);
            x += gw + 8.0;
        }
        if let Some(g) = g_label {
            p.galley_with_override_text_color(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, color);
        }
        if r.has_focus() {
            p.rect_stroke(rect.expand(2.0), CornerRadius::same(R + 2), Stroke::new(1.5, ACCENT_INK), StrokeKind::Outside);
        }
    }
    r.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}

/// Botão quadrado só com ícone e dica.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    button_sized(ui, Kind::Ghost, Some(icon), "", 28.0).on_hover_text(tip)
}

pub fn icon_danger(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    button_sized(ui, Kind::Danger, Some(icon), "", 28.0).on_hover_text(tip)
}

/// Tecla de atalho desenhada como keycap.
pub fn keycap(ui: &mut Ui, key: &str) {
    let g = ui.painter().layout_no_wrap(key.to_string(), mono(T_META), TEXT_2);
    let size = egui::vec2((g.size().x + 12.0).max(20.0), 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(4), RAISED, Stroke::new(1.0, LINE_2), StrokeKind::Inside);
    p.hline(rect.x_range().shrink(4.0), rect.bottom() - 1.0, Stroke::new(1.0, BG_DEEP));
    p.galley(rect.center() - g.size() / 2.0, g, TEXT_2);
}

/// Cartão com cabeçalho (título + meta à direita + ações) e corpo.
pub fn card<R>(
    ui: &mut Ui,
    title: &str,
    meta: &str,
    actions: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        Frame::NONE.inner_margin(Margin { left: 20, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
            ui.set_min_height(32.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(strong(T_BODY)).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    actions(ui);
                    if !meta.is_empty() {
                        ui.label(RichText::new(meta).font(mono(T_META)).color(TEXT_3));
                    }
                });
            });
        });
        line(ui);
        body(ui)
    })
    .inner
}

pub fn card_frame() -> Frame {
    Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(R_LG))
}

/// Área interna padrão de um cartão.
pub fn pad<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE.inner_margin(Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui)
    })
    .inner
}

pub fn line(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().hline(r.x_range(), y, Stroke::new(1.0, LINE));
    ui.add_space(1.0);
}

/// Rótulo de campo, acima do controle.
pub fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(body(T_SEC)).color(TEXT_2));
}

pub fn label_value(ui: &mut Ui, text: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(text).font(body(T_SEC)).color(TEXT_2));
        ui.label(RichText::new(value).font(mono(T_META)).color(TEXT));
    });
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(TEXT_2).font(body(T_SEC))
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(TEXT_3).font(body(T_META))
}

pub fn mono_text(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text.into()).font(mono(size)).color(color)
}

/// Controle segmentado. Devolve true se mudou. Ocupa só a largura das opções.
pub fn seg<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let font = strong(T_SEC);
    let galleys: Vec<_> = options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let total = widths.iter().sum::<f32>() + 2.0 * (options.len().saturating_sub(1)) as f32 + 8.0;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(total, 32.0), Sense::hover());
    let enabled = ui.is_enabled();
    ui.painter().rect(outer, CornerRadius::same(10), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let mut x = outer.left() + 4.0;
    let mut changed = false;
    for (i, (v, _)) in options.iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, outer.top() + 4.0), egui::vec2(widths[i], 24.0));
        x += widths[i] + 2.0;
        let r = ui.interact(rect, ui.auto_id_with(("seg", i)), Sense::click());
        let on = *value == *v;
        if on {
            ui.painter().rect_filled(rect, CornerRadius::same(R_SM), RAISED_2);
            ui.painter().hline(rect.x_range().shrink(R_SM as f32), rect.top() + 1.0, Stroke::new(1.0, HIGHLIGHT));
        } else if r.hovered() && enabled {
            ui.painter().rect_filled(rect, CornerRadius::same(R_SM), RAISED);
        }
        let color = if !enabled { TEXT_3 } else if on { TEXT } else { TEXT_2 };
        ui.painter().galley_with_override_text_color(rect.center() - galleys[i].size() / 2.0, galleys[i].clone(), color);
        if r.clicked() && enabled && !on {
            *value = v.clone();
            changed = true;
        }
        if enabled {
            let _ = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
    changed
}

#[derive(Clone, Copy)]
pub enum Tone {
    Accent,
    Done,
    Err,
    Neutral,
}

/// Barra fina de progresso (0..1).
pub fn progress(ui: &mut Ui, frac: f32, tone: Tone, width: Option<f32>) -> Response {
    let w = width.unwrap_or_else(|| ui.available_width());
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 4.0), Sense::hover());
    paint_progress(ui, rect, frac, tone);
    r
}

pub fn paint_progress(ui: &Ui, rect: egui::Rect, frac: f32, tone: Tone) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), RAISED);
    let c = match tone {
        Tone::Accent => ACCENT,
        Tone::Done => OK,
        Tone::Err => ERR,
        Tone::Neutral => TEXT_3,
    };
    let f = frac.clamp(0.0, 1.0);
    if f > 0.0 {
        let mut r = rect;
        r.set_width((rect.width() * f).max(4.0));
        p.rect_filled(r, CornerRadius::same(2), c);
    }
}

pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Estado vazio centralizado.
pub fn empty(ui: &mut Ui, icon: &str, text: &str) {
    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(RichText::new(icon).size(ICON).color(TEXT_3));
        ui.label(RichText::new(text).font(body(T_SEC)).color(TEXT_2));
    });
    ui.add_space(32.0);
}

/// Ticks de régua (assinatura visual) no topo de `rect`.
pub fn ruler(ui: &Ui, rect: egui::Rect, height: f32) {
    let p = ui.painter();
    let n = (rect.width() / 8.0).floor() as i32;
    for i in 0..=n {
        let x = rect.left() + i as f32 * 8.0;
        let h = if i % 10 == 0 { height } else if i % 5 == 0 { height * 0.66 } else { height * 0.4 };
        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + h)], Stroke::new(1.0, LINE_2));
    }
}

/// Caixa de seleção quadrada (aprovar, selecionar).
pub fn check(ui: &mut Ui, on: bool, size: f32) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, CornerRadius::same(R_SM), ACCENT);
        p.text(rect.center(), Align2::CENTER_CENTER, crate::icons::CHECK, body(size * 0.7), TEXT);
    } else {
        p.rect(rect, CornerRadius::same(R_SM), Color32::from_black_alpha(140), Stroke::new(1.0, if r.hovered() { TEXT_3 } else { LINE_2 }), StrokeKind::Inside);
    }
    if r.has_focus() {
        p.rect_stroke(rect.expand(2.0), CornerRadius::same(R), Stroke::new(1.5, ACCENT_INK), StrokeKind::Outside);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rádio redondo.
pub fn radio(ui: &mut Ui, on: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    let p = ui.painter();
    p.circle_stroke(rect.center(), 7.5, Stroke::new(1.0, if on { ACCENT } else { TEXT_3 }));
    if on {
        p.circle_filled(rect.center(), 4.0, ACCENT);
    }
    r
}

/// Título da página com subtítulo e ações à direita.
pub fn page_head(ui: &mut Ui, title: &str, sub: &str, actions: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(title).font(strong(T_FIG)).color(TEXT));
            if !sub.is_empty() {
                ui.label(RichText::new(sub).font(body(T_BODY)).color(TEXT_2));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), actions);
    });
    ui.add_space(24.0);
}

/// Texto que corta com reticências em uma linha.
pub fn truncated(ui: &mut Ui, text: impl Into<RichText>) -> Response {
    ui.add(egui::Label::new(text.into()).truncate())
}

// ------------------------------------------------------------------ avisos

#[derive(Clone)]
pub struct Toast {
    pub ok: bool,
    pub text: String,
    pub until: f64,
}

#[derive(Default)]
pub struct Toasts(pub Vec<Toast>);

impl Toasts {
    pub fn ok(&mut self, t: impl Into<String>) {
        self.push(true, t.into());
    }
    pub fn err(&mut self, t: impl Into<String>) {
        self.push(false, t.into());
    }
    fn push(&mut self, ok: bool, text: String) {
        self.0.push(Toast { ok, text, until: f64::NAN });
        if self.0.len() > 4 {
            self.0.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        for t in &mut self.0 {
            if t.until.is_nan() {
                t.until = now + if t.ok { 3.5 } else { 7.0 };
            }
        }
        self.0.retain(|t| t.until > now);
        if self.0.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-24.0, -24.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                let mut close = None;
                for (i, t) in self.0.iter().enumerate() {
                    Frame::NONE
                        .fill(RAISED)
                        .stroke(Stroke::new(1.0, LINE_2))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.horizontal(|ui| {
                                status_dot(ui, if t.ok { OK } else { ERR });
                                ui.add(egui::Label::new(RichText::new(&t.text).color(TEXT)).wrap());
                                if ui.add(egui::Label::new(RichText::new(crate::icons::X).color(TEXT_3)).sense(Sense::click())).clicked() {
                                    close = Some(i);
                                }
                            });
                        });
                    ui.add_space(8.0);
                }
                if let Some(i) = close {
                    self.0.remove(i);
                }
            });
    }
}

/// Janela modal com cabeçalho. Devolve false quando o usuário fecha.
pub fn modal<R>(ctx: &egui::Context, id: &str, title: &str, meta: &str, width: f32, body: impl FnOnce(&mut Ui) -> R) -> (bool, Option<R>) {
    let mut open = true;
    let screen = ctx.content_rect();
    let w = width.min(screen.width() - 48.0);
    let resp = egui::Modal::new(egui::Id::unique(id))
        .backdrop_color(Color32::from_black_alpha(170))
        .frame(card_frame().fill(CARD).stroke(Stroke::new(1.0, LINE_2)).corner_radius(CornerRadius::same(R_WIN)))
        .show(ctx, |ui| {
            ui.set_width(w);
            ui.set_max_height(screen.height() - 48.0);
            Frame::NONE.inner_margin(Margin { left: 20, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    truncated(ui, RichText::new(title).font(strong(T_BODY)).color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, crate::icons::X, "Fechar (Esc)").clicked() {
                            open = false;
                        }
                        if !meta.is_empty() {
                            ui.label(mono_text(meta, T_META, TEXT_3));
                        }
                    });
                });
            });
            line(ui);
            body(ui)
        });
    if resp.should_close() {
        open = false;
    }
    (open, Some(resp.inner))
}

/// Coluna de largura fixa dentro de uma linha horizontal (layout vertical).
pub fn column<R>(ui: &mut Ui, width: f32, body: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(egui::vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        body(ui)
    })
    .inner
}

/// Texto alinhado à esquerda numa largura fixa (colunas de tabela).
pub fn fixed(ui: &mut Ui, width: f32, text: impl Into<egui::WidgetText>) {
    ui.allocate_ui_with_layout(egui::vec2(width, 24.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(text).truncate());
    });
}
