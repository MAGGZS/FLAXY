//! Aba Modelos › Marcas d'água: lista das marcas e editor visual. Cada marca
//! tem textos translúcidos que se arrastam no quadro 9:16 até o lugar certo.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::watermarks::{self as wms, Watermark, WmText, SIZE_MAX, SIZE_MIN};

use crate::app::Shared;
use crate::backend::{self, Res, Task};
use crate::editors::caption_preview::{hex, paint_watermark, to_hex, watermark_rect};
use crate::icons as ic;
use crate::theme::*;
use crate::widgets::{self as w, Kind};

/// Distância (px) em que o texto gruda nas travas do quadro.
const SNAP_PX: f32 = 8.0;

#[derive(Clone)]
struct Draft {
    id: Option<i64>,
    name: String,
    items: Vec<WmText>,
    sel: Option<usize>,
    /// Texto sendo arrastado e a distância do clique até o centro dele.
    drag: Option<(usize, egui::Vec2)>,
    /// Travas em uso no arraste (vertical do centro, horizontal do centro).
    guides: (bool, bool),
}

impl Draft {
    fn new() -> Self {
        Self { id: None, name: "Minha marca".into(), items: vec![WmText::new("@seucanal", 0.5, 0.12)], sel: Some(0), drag: None, guides: (false, false) }
    }

    fn from(wm: &Watermark) -> Self {
        Self { id: Some(wm.row.id), name: wm.row.name.clone(), items: wm.items.clone(), sel: (!wm.items.is_empty()).then_some(0), drag: None, guides: (false, false) }
    }
}

#[derive(Default)]
pub struct WatermarkTab {
    loaded: bool,
    list: Vec<Watermark>,
    draft: Option<Draft>,
    /// Lista nova e, depois de salvar, a marca que fica aberta.
    task: Option<Task<Res<(Vec<Watermark>, Option<i64>)>>>,
    confirm: Option<Watermark>,
}

impl WatermarkTab {
    fn reload(&mut self, ctx: &egui::Context, sh: &Shared, keep: Option<i64>) {
        let bk = sh.bk.clone();
        self.task = Some(sh.bk.spawn(ctx, async move { Ok((wms::list(&bk.db).await.map_err(backend::e)?, keep)) }));
    }

    /// Botão do cabeçalho da página.
    pub fn head(&mut self, ui: &mut Ui) {
        if w::button(ui, Kind::Primary, Some(ic::PLUS), "Nova marca d'água").clicked() {
            self.draft = Some(Draft::new());
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if !self.loaded {
            self.loaded = true;
            self.reload(ui.ctx(), sh, None);
        }
        if let Some(r) = self.task.as_ref().and_then(|t| t.take()) {
            self.task = None;
            match r {
                Ok((l, keep)) => {
                    if let Some(id) = keep {
                        if let Some(wm) = l.iter().find(|w| w.row.id == id) {
                            let sel = self.draft.as_ref().and_then(|d| d.sel);
                            let mut d = Draft::from(wm);
                            d.sel = sel.filter(|i| *i < d.items.len()).or(d.sel);
                            self.draft = Some(d);
                        }
                        sh.toasts.ok("Marca d'água salva");
                    }
                    self.list = l;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        let gap = 12.0;
        let editor_w = if self.draft.is_some() { 700.0_f32.min(ui.available_width() * 0.62) } else { 0.0 };
        let list_w = ui.available_width() - editor_w - if editor_w > 0.0 { gap } else { 0.0 };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            w::column(ui, list_w, |ui| self.grid(ui, sh));
            if self.draft.is_some() {
                w::column(ui, editor_w, |ui| self.editor(ui, sh));
            }
        });
    }

    fn grid(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.list.is_empty() {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::LAYOUT_TEMPLATE, "Nenhuma marca d'água. Clique em Nova marca d'água");
                ui.vertical_centered(|ui| ui.label(w::muted("Depois ligue no editor do clipe, na caixa Marca d'água")));
                ui.add_space(24.0);
            });
            return;
        }
        let gap = 12.0;
        let avail = ui.available_width();
        let cols = ((avail + gap) / (140.0 + gap)).floor().max(1.0) as usize;
        let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = cw * 16.0 / 9.0;
        let mut open = None;
        for row in self.list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for wm in row {
                    let on = self.draft.as_ref().and_then(|d| d.id) == Some(wm.row.id);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, th + 40.0), Sense::click());
                    let p = ui.painter();
                    p.rect(rect, CornerRadius::same(R_LG), CARD, Stroke::new(1.0, if on { ACCENT } else { LINE }), StrokeKind::Inside);
                    let area = Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(cw - 2.0, th));
                    paint_backdrop(&ui.painter_at(area), area);
                    paint_watermark(&ui.painter_at(area), area, &wm.items, &sh.caption_fonts);
                    let name = Rect::from_min_max(egui::pos2(rect.left() + 12.0, area.bottom() + 4.0), rect.max - egui::vec2(8.0, 4.0));
                    ui.painter_at(name).text(name.left_center(), Align2::LEFT_CENTER, &wm.row.name, body(T_SEC), TEXT);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        open = Some(wm.clone());
                    }
                }
            });
            ui.add_space(gap - 8.0);
        }
        if let Some(wm) = open {
            self.draft = Some(Draft::from(&wm));
        }
    }

    fn editor(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let Some(mut d) = self.draft.clone() else { return };
        let title = if d.id.is_some() { "Editar marca d'água" } else { "Nova marca d'água" };
        let meta = format!("{} {}", d.items.len(), if d.items.len() == 1 { "texto" } else { "textos" });
        let (mut close, mut save, mut del) = (false, false, false);
        w::card(ui, title, &meta, |_| {}, |ui| {
            w::pad(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(w::dim("Nome"));
                    ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(260.0));
                });
                ui.add_space(10.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    ui.vertical(|ui| {
                        ui.set_width(270.0);
                        stage(ui, &mut d, &sh.caption_fonts);
                        ui.add_space(4.0);
                        ui.label(w::muted("Arraste os textos. Duplo clique num espaço vazio cria um texto ali. Alt solta das travas"));
                    });
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        if w::button(ui, Kind::Default, Some(ic::PLUS), "Adicionar texto").clicked() {
                            d.items.push(WmText::new("Texto", 0.5, 0.5));
                            d.sel = Some(d.items.len() - 1);
                        }
                        ui.add_space(8.0);
                        // Lista dos textos para escolher qual editar.
                        let mut pick = None;
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                            for (i, t) in d.items.iter().enumerate() {
                                let label: String = t.text.trim().chars().take(18).collect();
                                let label = if label.is_empty() { format!("Texto {}", i + 1) } else { label };
                                if ui.add(egui::Button::new(RichText::new(label).size(T_META)).selected(d.sel == Some(i)).small()).clicked() {
                                    pick = Some(i);
                                }
                            }
                        });
                        if pick.is_some() {
                            d.sel = pick;
                        }
                        ui.add_space(8.0);
                        match d.sel.filter(|i| *i < d.items.len()) {
                            Some(i) => {
                                if item_form(ui, &mut d.items[i], &sh.caption_fonts) {
                                    d.items.remove(i);
                                    d.sel = if d.items.is_empty() { None } else { Some(i.saturating_sub(1)) };
                                }
                            }
                            None => {
                                ui.label(w::muted("Clique num texto no quadro para editar"));
                            }
                        }
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.task.is_none(), |ui| {
                        if w::button(ui, Kind::Primary, Some(ic::CHECK), "Salvar").clicked() {
                            save = true;
                        }
                    });
                    if w::button(ui, Kind::Ghost, None, "Fechar").clicked() {
                        close = true;
                    }
                    if d.id.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_danger(ui, ic::TRASH_2, "Apagar marca d'água").clicked() {
                                del = true;
                            }
                        });
                    }
                });
            });
        });
        if save {
            let bk = sh.bk.clone();
            let (id, name, items) = (d.id, d.name.clone(), d.items.clone());
            self.task = Some(sh.bk.spawn(ui.ctx(), async move {
                let id = wms::save(&bk.db, id, &name, &items).await.map_err(backend::e)?;
                Ok((wms::list(&bk.db).await.map_err(backend::e)?, Some(id)))
            }));
        }
        if del {
            self.confirm = self.list.iter().find(|w| Some(w.row.id) == d.id).cloned();
        }
        self.draft = if close { None } else { Some(d) };
    }

    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let Some(wm) = self.confirm.clone() else { return };
        let (open, choice) = w::modal(ctx, "wm-del", &format!("Apagar a marca \"{}\"?", wm.row.name), "", 460.0, |ui| {
            let mut c = None;
            w::pad(ui, |ui| {
                ui.label(w::dim("Clipes que usavam esta marca saem sem marca d'água no próximo corte"));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::button(ui, Kind::Primary, Some(ic::TRASH_2), "Apagar").clicked() {
                        c = Some(true);
                    }
                    if w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                        c = Some(false);
                    }
                });
            });
            c
        });
        match choice.flatten() {
            Some(true) => {
                let bk = sh.bk.clone();
                let id = wm.row.id;
                self.task = Some(sh.bk.spawn(ctx, async move {
                    wms::delete(&bk.db, id).await.map_err(backend::e)?;
                    Ok((wms::list(&bk.db).await.map_err(backend::e)?, None))
                }));
                if self.draft.as_ref().and_then(|d| d.id) == Some(id) {
                    self.draft = None;
                }
                self.confirm = None;
            }
            Some(false) => self.confirm = None,
            None if !open => self.confirm = None,
            None => {}
        }
    }
}

/// Fundo neutro com faixas claras e escuras, para ver a transparência.
fn paint_backdrop(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgb(40, 44, 52));
    let n = 6;
    for i in 0..n {
        if i % 2 == 0 {
            let y0 = r.top() + r.height() * i as f32 / n as f32;
            p.rect_filled(Rect::from_min_max(egui::pos2(r.left(), y0), egui::pos2(r.right(), y0 + r.height() / n as f32)), CornerRadius::ZERO, Color32::from_rgb(150, 160, 175));
        }
    }
}

/// Quadro 9:16 com os textos: clicar seleciona, arrastar move (com travas no
/// centro e nos terços), duplo clique no vazio cria um texto.
fn stage(ui: &mut Ui, d: &mut Draft, fonts: &[String]) {
    let size = egui::vec2(270.0, 480.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let p = ui.painter_at(r);
    paint_backdrop(&p, r);
    paint_watermark(&p, r, &d.items, fonts);
    // Caixa dos textos (o selecionado em destaque).
    let rects: Vec<Rect> = d.items.iter().map(|t| watermark_rect(&p, r, t, fonts).expand(4.0)).collect();
    for (i, b) in rects.iter().enumerate() {
        let sel = d.sel == Some(i);
        p.rect_stroke(*b, CornerRadius::same(3), Stroke::new(if sel { 1.5 } else { 1.0 }, if sel { ACCENT } else { Color32::from_white_alpha(60) }), StrokeKind::Outside);
    }
    let hit = |q: Pos2| rects.iter().enumerate().rev().find(|(_, b)| b.contains(q)).map(|(i, _)| i);
    if let Some(q) = resp.hover_pos() {
        if d.drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if hit(q).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    let center = |t: &WmText| egui::pos2(r.left() + r.width() * t.x as f32, r.top() + r.height() * t.y as f32);
    if resp.drag_started() {
        if let Some(q) = ui.input(|i| i.pointer.press_origin()) {
            d.drag = hit(q).map(|i| (i, center(&d.items[i]) - q));
            if let Some((i, _)) = d.drag {
                d.sel = Some(i);
            }
        }
    }
    if let (Some((i, off)), Some(q)) = (d.drag, resp.interact_pointer_pos().filter(|_| resp.dragged())) {
        let c = q + off;
        let mut x = ((c.x - r.left()) / r.width()).clamp(0.0, 1.0);
        let mut y = ((c.y - r.top()) / r.height()).clamp(0.0, 1.0);
        let mut guides = (false, false);
        if !ui.input(|i| i.modifiers.alt) {
            let (tx, ty) = (SNAP_PX / r.width(), SNAP_PX / r.height());
            for g in [0.5_f32, 1.0 / 3.0, 2.0 / 3.0] {
                if (x - g).abs() < tx {
                    x = g;
                    guides.0 = g == 0.5;
                }
            }
            for g in [0.5_f32, 0.1, 0.9] {
                if (y - g).abs() < ty {
                    y = g;
                    guides.1 = g == 0.5;
                }
            }
        }
        d.guides = guides;
        d.items[i].x = x as f64;
        d.items[i].y = y as f64;
    }
    if resp.drag_stopped() {
        d.drag = None;
        d.guides = (false, false);
    }
    if resp.clicked() {
        if let Some(q) = resp.interact_pointer_pos() {
            d.sel = hit(q).or(d.sel);
        }
    }
    if resp.double_clicked() {
        if let Some(q) = resp.interact_pointer_pos().filter(|q| hit(*q).is_none()) {
            d.items.push(WmText::new("Texto", ((q.x - r.left()) / r.width()) as f64, ((q.y - r.top()) / r.height()) as f64));
            d.sel = Some(d.items.len() - 1);
        }
    }
    // Travas: marquinhas sempre, linha inteira quando gruda no centro.
    let gold = Color32::from_rgb(240, 190, 60);
    let c = r.center();
    p.line_segment([egui::pos2(c.x, r.top()), egui::pos2(c.x, r.top() + 8.0)], Stroke::new(1.5, Color32::from_white_alpha(120)));
    p.line_segment([egui::pos2(c.x, r.bottom() - 8.0), egui::pos2(c.x, r.bottom())], Stroke::new(1.5, Color32::from_white_alpha(120)));
    p.line_segment([egui::pos2(r.left(), c.y), egui::pos2(r.left() + 8.0, c.y)], Stroke::new(1.5, Color32::from_white_alpha(120)));
    p.line_segment([egui::pos2(r.right() - 8.0, c.y), egui::pos2(r.right(), c.y)], Stroke::new(1.5, Color32::from_white_alpha(120)));
    if d.guides.0 {
        p.vline(c.x, r.y_range(), Stroke::new(1.0, gold));
    }
    if d.guides.1 {
        p.hline(r.x_range(), c.y, Stroke::new(1.0, gold));
    }
    ui.painter().rect_stroke(r, CornerRadius::same(R), Stroke::new(1.0, LINE), StrokeKind::Inside);
}

/// Campos de um texto. Devolve true quando pedem para remover.
fn item_form(ui: &mut Ui, t: &mut WmText, fonts: &[String]) -> bool {
    let mut remove = false;
    ui.spacing_mut().item_spacing.y = 6.0;
    w::label(ui, "Texto");
    ui.add(egui::TextEdit::singleline(&mut t.text).desired_width(ui.available_width() - 8.0));
    ui.horizontal(|ui| {
        ui.add_sized(egui::vec2(70.0, 20.0), egui::Label::new(w::dim("Fonte")));
        let cur = t.font.clone();
        egui::ComboBox::from_id_salt("wm-font").width(170.0).selected_text(cur).show_ui(ui, |ui| {
            for f in fonts {
                if ui.selectable_label(&t.font == f, f).clicked() {
                    t.font = f.clone();
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_sized(egui::vec2(70.0, 20.0), egui::Label::new(w::dim("Tamanho")));
        ui.add(egui::Slider::new(&mut t.size, SIZE_MIN..=SIZE_MAX).step_by(2.0).show_value(false));
        ui.label(w::mono_text(format!("{:.0}", t.size), T_META, TEXT_3));
    });
    ui.horizontal(|ui| {
        ui.add_sized(egui::vec2(70.0, 20.0), egui::Label::new(w::dim("Opacidade")));
        let mut pct = t.opacity * 100.0;
        if ui.add(egui::Slider::new(&mut pct, 5.0..=100.0).step_by(5.0).show_value(false)).changed() {
            t.opacity = pct / 100.0;
        }
        ui.label(w::mono_text(format!("{:.0}%", t.opacity * 100.0), T_META, TEXT_3));
    });
    ui.horizontal(|ui| {
        ui.add_sized(egui::vec2(70.0, 20.0), egui::Label::new(w::dim("Cor")));
        let c = hex(&t.color);
        let mut rgb = [c.r(), c.g(), c.b()];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            t.color = to_hex(rgb);
        }
        ui.checkbox(&mut t.bold, "Negrito");
        ui.checkbox(&mut t.outline, "Contorno");
    });
    ui.add_space(4.0);
    // Posições prontas: 3×3 (cantos, bordas e centro).
    ui.label(w::dim("Posição rápida"));
    egui::Grid::new("wm-pos").spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
        for (row, y) in [0.08, 0.5, 0.92].into_iter().enumerate() {
            for (col, x) in [0.2, 0.5, 0.8].into_iter().enumerate() {
                let on = (t.x - x).abs() < 0.01 && (t.y - y).abs() < 0.01;
                let glyph = ["↖", "↑", "↗", "←", "•", "→", "↙", "↓", "↘"][row * 3 + col];
                if ui.add(egui::Button::new(glyph).selected(on).min_size(egui::vec2(28.0, 22.0))).clicked() {
                    (t.x, t.y) = (x, y);
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Remover texto").clicked() {
        remove = true;
    }
    remove
}
