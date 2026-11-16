//! Modelos: fundos (fundo fixo + área do vídeo) e estilos de legenda.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, CornerRadius, Pos2, RichText, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::captions::{CaptionStyle, Kind as CapKind, Style};
use flaxy_core::models::{ClipBackground, Template, TemplateInput};
use flaxy_core::templates::{self as tpls, ALLOWED_BG};

use crate::app::Shared;
use crate::backend::{self, Res, Task};
use crate::editors::caption_preview::{self, hex, to_hex};
use crate::editors::watermark_editor::WatermarkTab;
use crate::format::ratio_label;
use crate::icons as ic;
use crate::media;
use crate::theme::*;
use crate::widgets::{self as w, Kind};

const PRESETS: [(&str, i64, i64); 4] = [("9:16", 1080, 1920), ("4:5", 1080, 1350), ("1:1", 1080, 1080), ("16:9", 1920, 1080)];

#[derive(Clone, Copy, PartialEq, Default)]
enum Tab {
    #[default]
    Fundos,
    /// Fundos personalizados para a sobra das áreas em Encaixar nos clipes.
    ClipBgs,
    /// Textos translúcidos por cima dos clipes.
    Marcas,
    Legendas,
}

#[derive(Clone)]
struct Draft {
    id: Option<i64>,
    input: TemplateInput,
    /// Arquivo de fundo: existente (edição) ou escolhido agora (novo).
    file: Option<PathBuf>,
}

fn blank() -> TemplateInput {
    TemplateInput { overlay_y: 420, overlay_h: 1080, ..TemplateInput::default() }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Move,
    NW,
    SE,
}

#[derive(Default)]
pub struct Modelos {
    tab: Tab,
    loaded: bool,
    list: Vec<Template>,
    draft: Option<Draft>,
    drag: Option<(Drag, TemplateInput, Pos2)>,
    t_list: Option<Task<Res<Vec<Template>>>>,
    t_save: Option<Task<Res<()>>>,
    confirm: Option<Template>,
    // legendas
    styles: Vec<Style>,
    sel_style: Option<Style>,
    style_dirty: bool,
    t_styles: Option<Task<Res<(Vec<Style>, Option<i64>)>>>,
    confirm_style: bool,
    // fundos de clipe
    clip_bgs: Vec<ClipBackground>,
    t_bgs: Option<Task<Res<Vec<ClipBackground>>>>,
    confirm_bg: Option<ClipBackground>,
    /// Fundo com o nome em edição (id, texto).
    rename_bg: Option<(i64, String)>,
    marcas: WatermarkTab,
}

impl Modelos {
    fn load(&mut self, ui: &Ui, sh: &Shared) {
        let bk = sh.bk.clone();
        self.t_list = Some(sh.bk.spawn(ui.ctx(), async move { tpls::list(&bk.db).await.map_err(backend::e) }));
    }

    fn load_bgs(&mut self, ctx: &egui::Context, sh: &Shared) {
        let bk = sh.bk.clone();
        self.t_bgs = Some(sh.bk.spawn(ctx, async move { tpls::list_clip_bgs(&bk.db).await.map_err(backend::e) }));
    }

    fn load_styles(&mut self, ctx: &egui::Context, sh: &Shared, pick: Option<i64>) {
        let bk = sh.bk.clone();
        self.t_styles = Some(sh.bk.spawn(ctx, async move { Ok((flaxy_core::captions::list(&bk.db).await.map_err(backend::e)?, pick)) }));
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if !self.loaded {
            self.loaded = true;
            self.load(ui, sh);
            self.load_styles(ui.ctx(), sh, None);
            self.load_bgs(ui.ctx(), sh);
        }
        if let Some(r) = self.t_bgs.as_ref().and_then(|t| t.take()) {
            self.t_bgs = None;
            match r {
                Ok(l) => self.clip_bgs = l,
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_list.as_ref().and_then(|t| t.take()) {
            self.t_list = None;
            match r {
                Ok(l) => self.list = l,
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_save.as_ref().and_then(|t| t.take()) {
            self.t_save = None;
            match r {
                Ok(()) => {
                    sh.toasts.ok("Modelo salvo");
                    self.draft = None;
                }
                Err(e) => sh.toasts.err(e),
            }
            self.load(ui, sh);
        }
        if let Some(r) = self.t_styles.as_ref().and_then(|t| t.take()) {
            self.t_styles = None;
            match r {
                Ok((l, pick)) => {
                    let want = pick.or(self.sel_style.as_ref().map(|s| s.row.id));
                    self.sel_style = want.and_then(|id| l.iter().find(|s| s.row.id == id).cloned()).or_else(|| l.first().cloned());
                    self.styles = l;
                    self.style_dirty = false;
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        let sub = match self.tab {
            Tab::Fundos => "Fundo fixo e a área onde cada vídeo entra",
            Tab::ClipBgs => "Imagens e vídeos para o fundo das áreas em Encaixar nos clipes",
            Tab::Marcas => "Textos translúcidos por cima dos clipes, nos lugares que você escolher",
            Tab::Legendas => "Como a legenda aparece no vídeo",
        };
        let mut new = false;
        let mut add_bg = false;
        w::page_head(ui, "Modelos", sub, |ui| {
            if self.tab == Tab::Fundos && w::button(ui, Kind::Primary, Some(ic::PLUS), "Novo modelo").clicked() {
                new = true;
            }
            if self.tab == Tab::ClipBgs {
                ui.add_enabled_ui(self.t_bgs.is_none(), |ui| {
                    if w::button(ui, Kind::Primary, Some(ic::IMAGE_PLUS), "Adicionar fundo").clicked() {
                        add_bg = true;
                    }
                });
            }
            if self.tab == Tab::Marcas {
                self.marcas.head(ui);
            }
            w::seg(ui, &mut self.tab, &[(Tab::Fundos, "Fundos"), (Tab::ClipBgs, "Fundos de clipe"), (Tab::Marcas, "Marcas d'água"), (Tab::Legendas, "Legendas")]);
        });
        if add_bg {
            if let Some(files) = rfd::FileDialog::new().set_title("Escolher fundos de clipe (imagem ou vídeo)").add_filter("Imagem ou vídeo", ALLOWED_BG).pick_files() {
                let bk = sh.bk.clone();
                self.t_bgs = Some(sh.bk.spawn(ui.ctx(), async move {
                    for f in files {
                        let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        tpls::add_clip_bg(&bk.db, &name, &f, &bk.cfg.templates_dir()).await.map_err(backend::e)?;
                    }
                    tpls::list_clip_bgs(&bk.db).await.map_err(backend::e)
                }));
            }
        }
        if new {
            if let Some(f) = rfd::FileDialog::new().set_title("Escolher fundo (imagem ou vídeo)").add_filter("Imagem ou vídeo", ALLOWED_BG).pick_file() {
                let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.draft = Some(Draft { id: None, input: TemplateInput { name, ..blank() }, file: Some(f) });
            }
        }

        match self.tab {
            Tab::Fundos => self.fundos(ui, sh),
            Tab::ClipBgs => self.clip_bgs(ui, sh),
            Tab::Marcas => self.marcas.ui(ui, sh),
            Tab::Legendas => self.legendas(ui, sh),
        }
    }

    /// Biblioteca de fundos de clipe: grade 9:16 com nome, tipo e apagar.
    fn clip_bgs(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.clip_bgs.is_empty() {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::IMAGE, "Nenhum fundo de clipe. Clique em Adicionar fundo e escolha imagens ou vídeos");
                ui.vertical_centered(|ui| {
                    ui.label(w::muted("Depois, no editor do clipe, ponha uma área em Encaixar e escolha Fundo do Encaixar › Arquivo"));
                });
                ui.add_space(24.0);
            });
            return;
        }
        let gap = 12.0;
        let avail = ui.available_width();
        let cols = ((avail + gap) / (150.0 + gap)).floor().max(1.0) as usize;
        let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = cw * 16.0 / 9.0;
        let mut del = None;
        let mut rename_done: Option<(i64, String)> = None;
        let list = self.clip_bgs.clone();
        for row in list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for b in row {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, th + 54.0), Sense::hover());
                    let p = ui.painter();
                    p.rect(rect, CornerRadius::same(R_LG), CARD, Stroke::new(1.0, LINE), StrokeKind::Inside);
                    let area = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(cw - 2.0, th));
                    p.rect_filled(area, CornerRadius { nw: R_LG, ne: R_LG, sw: 0, se: 0 }, Color32::BLACK);
                    let path = Path::new(&b.file_path);
                    match sh.thumbs.get(&sh.bk, ui.ctx(), path, 0.5, (360, 640)) {
                        Some(tex) => media::paint_image(ui, &tex, area, true),
                        None if !path.exists() => {
                            ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "Arquivo sumiu", body(T_META), ERR);
                        }
                        None => {}
                    }
                    let video = !flaxy_core::clips::ClipBg::is_image(&b.file_path);
                    let tag = if video { "vídeo" } else { "imagem" };
                    let g = ui.painter().layout_no_wrap(tag.to_string(), mono(T_META), TEXT);
                    let tr = egui::Rect::from_min_size(area.left_top() + egui::vec2(6.0, 6.0), g.size() + egui::vec2(10.0, 4.0));
                    ui.painter().rect_filled(tr, CornerRadius::same(4), Color32::from_black_alpha(190));
                    ui.painter().galley(tr.min + egui::vec2(5.0, 2.0), g, TEXT);
                    let foot = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, area.bottom() + 6.0), rect.max - egui::vec2(6.0, 6.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let editing = self.rename_bg.as_ref().filter(|r| r.0 == b.id).cloned();
                        match editing {
                            Some((id, mut text)) => {
                                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
                                if !r.has_focus() && !r.lost_focus() {
                                    r.request_focus();
                                }
                                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if r.lost_focus() || enter {
                                    rename_done = Some((id, text));
                                } else {
                                    self.rename_bg = Some((id, text));
                                }
                            }
                            None => {
                                ui.allocate_ui(egui::vec2(ui.available_width() - 32.0, 40.0), |ui| {
                                    let r = ui.add(egui::Label::new(RichText::new(&b.name).font(body(T_SEC)).color(TEXT)).truncate().sense(Sense::click()));
                                    if r.on_hover_text("Clique para renomear").clicked() {
                                        self.rename_bg = Some((b.id, b.name.clone()));
                                    }
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if w::icon_danger(ui, ic::TRASH_2, "Apagar").clicked() {
                                        del = Some(b.clone());
                                    }
                                });
                            }
                        }
                    });
                }
            });
            ui.add_space(gap - 8.0);
        }
        if let Some((id, name)) = rename_done {
            self.rename_bg = None;
            if self.clip_bgs.iter().any(|b| b.id == id && b.name != name.trim()) && !name.trim().is_empty() {
                let bk = sh.bk.clone();
                self.t_bgs = Some(sh.bk.spawn(ui.ctx(), async move {
                    tpls::rename_clip_bg(&bk.db, id, &name).await.map_err(backend::e)?;
                    tpls::list_clip_bgs(&bk.db).await.map_err(backend::e)
                }));
            }
        }
        if del.is_some() {
            self.confirm_bg = del;
        }
    }

    fn fundos(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let gap = 12.0;
        let editor_w = if self.draft.is_some() { 660.0 } else { 0.0 };
        let grid_w = ui.available_width() - editor_w - if editor_w > 0.0 { gap } else { 0.0 };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            w::column(ui, grid_w, |ui| {
                if self.list.is_empty() {
                    w::card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        w::empty(ui, ic::LAYOUT_TEMPLATE, "Nenhum modelo de fundo. Clique em Novo modelo");
                    });
                } else {
                    self.grid(ui, sh);
                }
            });
            if self.draft.is_some() {
                w::column(ui, editor_w, |ui| {
                    self.editor(ui, sh);
                });
            }
        });
    }

    fn grid(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let gap = 12.0;
        let avail = ui.available_width();
        let cols = ((avail + gap) / (172.0 + gap)).floor().max(1.0) as usize;
        let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = cw * 16.0 / 9.0;
        let mut edit = None;
        let mut dup = None;
        let mut del = None;
        for row in self.list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for t in row {
                    let on = self.draft.as_ref().and_then(|d| d.id) == Some(t.id);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, th + 54.0), Sense::hover());
                    let p = ui.painter();
                    p.rect(rect, CornerRadius::same(R_LG), CARD, Stroke::new(1.0, if on { ACCENT } else { LINE }), StrokeKind::Inside);
                    let area = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(cw - 2.0, th));
                    p.rect_filled(area, CornerRadius { nw: R_LG, ne: R_LG, sw: 0, se: 0 }, Color32::BLACK);
                    let r = media::fit_rect(area, t.output_w as f32 / t.output_h.max(1) as f32);
                    if let Some(tex) = sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(&t.file_path), 0.1, (360, 640)) {
                        media::paint_image(ui, &tex, r, true);
                    }
                    let ov = overlay_rect(r, &t_input(t));
                    ui.painter_at(r).rect(ov, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(59, 99, 214, 40), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
                    if ui.interact(area, egui::Id::unique(("tpl", t.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        edit = Some(t.clone());
                    }
                    let foot = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, area.bottom() + 6.0), rect.max - egui::vec2(6.0, 6.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.allocate_ui(egui::vec2(ui.available_width() - 62.0, 40.0), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                w::truncated(ui, RichText::new(&t.name).font(body(T_SEC)).color(TEXT));
                                ui.label(w::mono_text(format!("{} · {}×{}", ratio_label(t.output_w, t.output_h), t.output_w, t.output_h), T_META, TEXT_3));
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_danger(ui, ic::TRASH_2, "Apagar").clicked() {
                                del = Some(t.clone());
                            }
                            if w::icon_button(ui, ic::COPY, "Duplicar").clicked() {
                                dup = Some(t.id);
                            }
                        });
                    });
                }
            });
            ui.add_space(gap - 8.0);
        }
        if let Some(t) = edit {
            self.draft = Some(Draft { id: Some(t.id), input: t_input(&t), file: Some(PathBuf::from(&t.file_path)) });
        }
        if let Some(id) = dup {
            let bk = sh.bk.clone();
            self.t_save = Some(sh.bk.spawn(ui.ctx(), async move { tpls::duplicate(&bk.db, id, &bk.cfg.templates_dir()).await.map(|_| ()).map_err(backend::e) }));
        }
        if del.is_some() {
            self.confirm = del;
        }
    }

    fn editor(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let Some(mut d) = self.draft.clone() else { return };
        let title = if d.id.is_some() { "Editar modelo" } else { "Novo modelo" };
        let meta = format!("{}×{}", d.input.output_w, d.input.output_h);
        let mut close = false;
        let mut save = false;
        w::card(ui, title, &meta, |_| {}, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                w::column(ui, 250.0, |ui| {
                    w::pad(ui, |ui| {
                        let i = &d.input;
                        let aspect = i.output_w as f32 / i.output_h.max(1) as f32;
                        let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 380.0), Sense::hover());
                        let stage = media::fit_rect(outer, aspect);
                        ui.painter().rect(stage, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
                        if let Some(f) = &d.file {
                            if let Some(tex) = sh.thumbs.get(&sh.bk, ui.ctx(), f, 0.1, (540, 960)) {
                                media::paint_image(ui, &tex, stage, true);
                            }
                        }
                        let ov = overlay_rect(stage, i);
                        let p = ui.painter_at(stage);
                        p.rect(ov, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(59, 99, 214, 56), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
                        p.text(ov.left_top() + egui::vec2(4.0, 4.0), egui::Align2::LEFT_TOP, "vídeo", mono(T_META), TEXT);
                        for c in [ov.left_top(), ov.right_bottom()] {
                            p.rect_filled(egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)), CornerRadius::same(2), TEXT);
                        }
                        let resp = ui.interact(stage, egui::Id::unique("tpl-stage"), Sense::drag());
                        if resp.drag_started() {
                            if let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                                let mode = if ov.left_top().distance(pos) < 10.0 {
                                    Some(Drag::NW)
                                } else if ov.right_bottom().distance(pos) < 10.0 {
                                    Some(Drag::SE)
                                } else if ov.contains(pos) {
                                    Some(Drag::Move)
                                } else {
                                    None
                                };
                                self.drag = mode.map(|m| (m, d.input.clone(), pos));
                            }
                        }
                        if let (Some((mode, s, from)), Some(pos)) = (self.drag.clone(), resp.interact_pointer_pos()) {
                            if resp.dragged() {
                                let k = s.output_w as f32 / stage.width();
                                let dx = ((pos.x - from.x) * k).round() as i64;
                                let dy = ((pos.y - from.y) * k).round() as i64;
                                let (mut x, mut y, mut wd, mut ht) = (s.overlay_x, s.overlay_y, s.overlay_w, s.overlay_h);
                                match mode {
                                    Drag::Move => {
                                        x = (x + dx).clamp(0, s.output_w - wd);
                                        y = (y + dy).clamp(0, s.output_h - ht);
                                    }
                                    Drag::SE => {
                                        wd = (wd + dx).clamp(32, s.output_w - x);
                                        ht = (ht + dy).clamp(32, s.output_h - y);
                                    }
                                    Drag::NW => {
                                        let nx = (x + dx).clamp(0, x + wd - 32);
                                        let ny = (y + dy).clamp(0, y + ht - 32);
                                        wd += x - nx;
                                        ht += y - ny;
                                        x = nx;
                                        y = ny;
                                    }
                                }
                                d.input.overlay_x = x;
                                d.input.overlay_y = y;
                                d.input.overlay_w = wd & !1;
                                d.input.overlay_h = ht & !1;
                            }
                        }
                        if resp.drag_stopped() {
                            self.drag = None;
                        }
                        if d.id.is_none() {
                            ui.add_space(8.0);
                            if w::button(ui, Kind::Default, Some(ic::IMAGE_PLUS), "Trocar fundo").clicked() {
                                if let Some(f) = rfd::FileDialog::new().add_filter("Imagem ou vídeo", ALLOWED_BG).pick_file() {
                                    d.file = Some(f);
                                }
                            }
                        }
                    });
                });
                let r = ui.available_rect_before_wrap();
                ui.painter().vline(r.left(), r.y_range(), Stroke::new(1.0, LINE));
                ui.vertical(|ui| {
                    w::pad(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let i = &mut d.input;
                        w::label(ui, "Nome");
                        ui.add(egui::TextEdit::singleline(&mut i.name).desired_width(f32::INFINITY));
                        ui.add_space(8.0);
                        w::label(ui, "Proporção da saída");
                        let mut cur = (i.output_w, i.output_h);
                        let opts: Vec<((i64, i64), &str)> = PRESETS.iter().map(|(l, w, h)| ((*w, *h), *l)).collect();
                        if w::seg(ui, &mut cur, &opts) {
                            // Mantém a área proporcional ao trocar a saída.
                            let sx = cur.0 as f64 / i.output_w as f64;
                            let sy = cur.1 as f64 / i.output_h as f64;
                            i.overlay_x = (i.overlay_x as f64 * sx).round() as i64;
                            i.overlay_y = (i.overlay_y as f64 * sy).round() as i64;
                            i.overlay_w = ((i.overlay_w as f64 * sx).round() as i64) & !1;
                            i.overlay_h = ((i.overlay_h as f64 * sy).round() as i64) & !1;
                            i.output_w = cur.0;
                            i.output_h = cur.1;
                        }
                        ui.add_space(8.0);
                        w::label(ui, "Área do vídeo (px): x · y · largura · altura");
                        ui.horizontal(|ui| {
                            let (ow, oh) = (i.output_w, i.output_h);
                            ui.add(egui::DragValue::new(&mut i.overlay_x).range(0..=ow));
                            ui.add(egui::DragValue::new(&mut i.overlay_y).range(0..=oh));
                            ui.add(egui::DragValue::new(&mut i.overlay_w).range(32..=ow));
                            ui.add(egui::DragValue::new(&mut i.overlay_h).range(32..=oh));
                        });
                        ui.add_space(8.0);
                        w::label(ui, "Encaixe");
                        w::seg(ui, &mut i.fit_mode, &[("cover".to_string(), "Preencher (corta)"), ("contain".to_string(), "Caber (bordas)")]);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                w::label(ui, "Duração");
                                combo(ui, "dur", &mut i.duration_rule, &[("raw", "Do vídeo"), ("template", "Do fundo"), ("loop_template", "Fundo em loop")]);
                            });
                            ui.vertical(|ui| {
                                w::label(ui, "Áudio");
                                combo(ui, "aud", &mut i.audio_source, &[("raw", "Do vídeo"), ("template", "Do fundo"), ("both", "Mixar os dois")]);
                            });
                        });
                        if i.audio_source == "both" {
                            w::label_value(ui, "Volume do vídeo", &format!("{}%", (i.audio_mix_raw * 100.0).round()));
                            ui.add(egui::Slider::new(&mut i.audio_mix_raw, 0.0..=2.0).step_by(0.05).show_value(false));
                            w::label_value(ui, "Volume do fundo", &format!("{}%", (i.audio_mix_template * 100.0).round()));
                            ui.add(egui::Slider::new(&mut i.audio_mix_template, 0.0..=2.0).step_by(0.05).show_value(false));
                        }
                        ui.add_space(8.0);
                        w::label(ui, "Bitrate de vídeo");
                        combo(ui, "br", &mut i.video_bitrate, &[("4M", "4M"), ("6M", "6M"), ("8M", "8M"), ("12M", "12M"), ("16M", "16M")]);
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(self.t_save.is_none(), |ui| {
                                    if w::button(ui, Kind::Primary, Some(ic::SAVE), "Salvar modelo").clicked() {
                                        save = true;
                                    }
                                });
                                if w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                                    close = true;
                                }
                            });
                        });
                    });
                });
            });
        });
        if close {
            self.draft = None;
            return;
        }
        if save {
            if d.input.name.trim().is_empty() {
                sh.toasts.err("Dê um nome ao modelo");
            } else {
                let bk = sh.bk.clone();
                let dd = d.clone();
                self.t_save = Some(sh.bk.spawn(ui.ctx(), async move {
                    match dd.id {
                        Some(id) => tpls::update(&bk.db, id, dd.input).await.map(|_| ()).map_err(backend::e),
                        None => {
                            let src = dd.file.ok_or("Escolha o fundo do modelo")?;
                            let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let dest = bk.cfg.templates_dir().join(tpls::unique_filename(&name, ALLOWED_BG).map_err(backend::e)?);
                            tokio::fs::copy(&src, &dest).await.map_err(backend::e)?;
                            match tpls::create(&bk.db, dd.input, &dest.to_string_lossy()).await {
                                Ok(_) => Ok(()),
                                Err(e) => {
                                    let _ = tokio::fs::remove_file(&dest).await;
                                    Err(e.to_string())
                                }
                            }
                        }
                    }
                }));
            }
        }
        if self.draft.is_some() {
            self.draft = Some(d);
        }
    }

    // ------------------------------------------------------------- legendas

    fn legendas(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let Some(mut sel) = self.sel_style.clone() else {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::TYPE, "Nenhum estilo");
            });
            return;
        };
        let gap = 12.0;
        let list_w = 240.0;
        let prev_w = 330.0;
        let mid_w = ui.available_width() - list_w - prev_w - gap * 2.0;
        let mut pick = None;
        let mut dup = false;
        let mut save = false;
        let mut del = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            w::column(ui, list_w, |ui| {
                w::card(ui, "Estilos", "", |ui| {
                    if w::icon_button(ui, ic::PLUS, "Novo a partir deste").clicked() {
                        dup = true;
                    }
                }, |ui| {
                    for s in &self.styles {
                        let on = s.row.id == sel.row.id;
                        let resp = egui::Frame::NONE.fill(if on { RAISED } else { Color32::TRANSPARENT }).inner_margin(egui::Margin::symmetric(20, 8)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(&s.row.name).font(body(T_SEC)).color(TEXT));
                                if s.row.builtin {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(w::mono_text("pronto", T_META, TEXT_3));
                                    });
                                }
                            });
                        });
                        if ui.interact(resp.response.rect, egui::Id::unique(("style", s.row.id)), Sense::click()).clicked() {
                            pick = Some(s.clone());
                        }
                    }
                });
            });

            w::column(ui, mid_w, |ui| {
                w::card_frame().show(ui, |ui| {
                    ui.set_width(mid_w);
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!sel.row.builtin, |ui| {
                                if ui.add(egui::TextEdit::singleline(&mut sel.row.name).desired_width(260.0)).changed() {
                                    self.style_dirty = true;
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(self.style_dirty, |ui| {
                                    if w::button(ui, Kind::Primary, Some(ic::SAVE), if sel.row.builtin { "Salvar como meu" } else { "Salvar" }).clicked() {
                                        save = true;
                                    }
                                });
                                if !sel.row.builtin && w::icon_danger(ui, ic::TRASH_2, "Apagar").clicked() {
                                    del = true;
                                }
                                if w::icon_button(ui, ic::COPY, "Duplicar").clicked() {
                                    dup = true;
                                }
                            });
                        });
                    });
                    w::line(ui);
                    w::pad(ui, |ui| {
                        if style_form(ui, &mut sel.style, &sh.caption_fonts) {
                            self.style_dirty = true;
                        }
                    });
                });
            });

            w::column(ui, prev_w, |ui| {
                w::card(ui, "Prévia", "9:16", |_| {}, |ui| {
                    w::pad(ui, |ui| caption_preview::preview(ui, &sel.style, 500.0, &sh.caption_fonts));
                });
            });
        });

        if let Some(p) = pick {
            self.sel_style = Some(p);
            self.style_dirty = false;
            return;
        }
        let ctx = ui.ctx().clone();
        if dup || save {
            let bk = sh.bk.clone();
            let s = sel.clone();
            let as_new = dup || s.row.builtin;
            let name = if dup { format!("{} (cópia)", s.row.name) } else if s.row.builtin { format!("{} (meu)", s.row.name) } else { s.row.name.clone() };
            let (bk2, ctx2) = (sh.bk.clone(), ctx.clone());
            self.t_styles = Some(bk2.spawn(&ctx2, async move {
                let id = if as_new {
                    flaxy_core::captions::create(&bk.db, &name, &s.style).await.map_err(backend::e)?.row.id
                } else {
                    flaxy_core::captions::update(&bk.db, s.row.id, &name, &s.style).await.map_err(backend::e)?.row.id
                };
                Ok((flaxy_core::captions::list(&bk.db).await.map_err(backend::e)?, Some(id)))
            }));
            sh.toasts.ok("Estilo salvo");
        }
        if del {
            self.confirm_style = true;
        }
        self.sel_style = Some(sel);
    }

    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        self.marcas.overlays(ctx, sh);
        if let Some(t) = self.confirm.clone() {
            match confirm(ctx, "tpl-del", &format!("Apagar o modelo \"{}\"?", t.name), "O arquivo de fundo copiado para o FLAXY também é apagado") {
                Some(true) => {
                    let bk = sh.bk.clone();
                    self.t_save = Some(sh.bk.spawn(ctx, async move { tpls::delete(&bk.db, t.id, &bk.cfg.templates_dir()).await.map_err(backend::e) }));
                    if self.draft.as_ref().and_then(|d| d.id) == Some(t.id) {
                        self.draft = None;
                    }
                    sh.thumbs.forget(Path::new(&t.file_path));
                    self.confirm = None;
                }
                Some(false) => self.confirm = None,
                None => {}
            }
        }
        if let Some(b) = self.confirm_bg.clone() {
            match confirm(ctx, "clip-bg-del", &format!("Apagar o fundo \"{}\"?", b.name), "Clipes que usavam este fundo voltam ao vídeo desfocado no próximo corte") {
                Some(true) => {
                    let bk = sh.bk.clone();
                    self.t_bgs = Some(sh.bk.spawn(ctx, async move {
                        tpls::delete_clip_bg(&bk.db, b.id, &bk.cfg.templates_dir()).await.map_err(backend::e)?;
                        tpls::list_clip_bgs(&bk.db).await.map_err(backend::e)
                    }));
                    sh.thumbs.forget(Path::new(&b.file_path));
                    self.confirm_bg = None;
                }
                Some(false) => self.confirm_bg = None,
                None => {}
            }
        }
        if self.confirm_style {
            let Some(s) = self.sel_style.clone() else {
                self.confirm_style = false;
                return;
            };
            match confirm(ctx, "style-del", &format!("Apagar o estilo \"{}\"?", s.row.name), "Lotes já renderizados não mudam") {
                Some(true) => {
                    let bk = sh.bk.clone();
                    self.t_styles = Some(sh.bk.spawn(ctx, async move {
                        flaxy_core::captions::delete(&bk.db, s.row.id).await.map_err(backend::e)?;
                        Ok((flaxy_core::captions::list(&bk.db).await.map_err(backend::e)?, None))
                    }));
                    self.sel_style = None;
                    self.confirm_style = false;
                }
                Some(false) => self.confirm_style = false,
                None => {}
            }
        }
    }
}

/// Modal de confirmação. `Some(true)` confirma, `Some(false)` cancela.
fn confirm(ctx: &egui::Context, id: &str, title: &str, text: &str) -> Option<bool> {
    let (open, choice) = w::modal(ctx, id, title, "", 460.0, |ui| {
        let mut c = None;
        w::pad(ui, |ui| {
            ui.label(w::dim(text));
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
        Some(v) => Some(v),
        None if !open => Some(false),
        None => None,
    }
}

fn t_input(t: &Template) -> TemplateInput {
    TemplateInput {
        name: t.name.clone(),
        overlay_x: t.overlay_x,
        overlay_y: t.overlay_y,
        overlay_w: t.overlay_w,
        overlay_h: t.overlay_h,
        fit_mode: t.fit_mode.clone(),
        output_w: t.output_w,
        output_h: t.output_h,
        output_format: t.output_format.clone(),
        video_bitrate: t.video_bitrate.clone(),
        audio_source: t.audio_source.clone(),
        audio_mix_raw: t.audio_mix_raw,
        audio_mix_template: t.audio_mix_template,
        duration_rule: t.duration_rule.clone(),
    }
}

fn overlay_rect(r: egui::Rect, i: &TemplateInput) -> egui::Rect {
    let (ow, oh) = (i.output_w.max(1) as f32, i.output_h.max(1) as f32);
    egui::Rect::from_min_size(
        egui::pos2(r.left() + r.width() * i.overlay_x as f32 / ow, r.top() + r.height() * i.overlay_y as f32 / oh),
        egui::vec2(r.width() * i.overlay_w as f32 / ow, r.height() * i.overlay_h as f32 / oh),
    )
}

fn combo(ui: &mut Ui, id: &str, value: &mut String, opts: &[(&str, &str)]) -> bool {
    let cur = opts.iter().find(|o| o.0 == value).map(|o| o.1).unwrap_or(value.as_str()).to_string();
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(170.0).selected_text(cur).show_ui(ui, |ui| {
        for (k, l) in opts {
            if ui.selectable_label(value == k, *l).clicked() {
                *value = k.to_string();
                changed = true;
            }
        }
    });
    changed
}

fn color_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        w::label(ui, label);
        ui.horizontal(|ui| {
            let c = hex(value);
            let mut rgb = [c.r(), c.g(), c.b()];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                *value = to_hex(rgb);
                changed = true;
            }
            ui.label(w::mono_text(value.clone(), T_META, TEXT_3));
        });
    });
    changed
}

/// Formulário de um estilo de legenda. Devolve true se algo mudou.
fn style_form(ui: &mut Ui, s: &mut CaptionStyle, fonts: &[String]) -> bool {
    let mut ch = false;
    ui.spacing_mut().item_spacing.y = 6.0;
    w::label(ui, "Tipo");
    ch |= w::seg(ui, &mut s.kind, &[(CapKind::Highlight, "Palavra destacada"), (CapKind::Word, "Palavra por palavra"), (CapKind::Classic, "Clássica"), (CapKind::Box, "Caixa colorida")]);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            w::label(ui, "Fonte");
            let mut list: Vec<String> = fonts.to_vec();
            if !list.contains(&s.font) {
                list.insert(0, s.font.clone());
            }
            egui::ComboBox::from_id_salt("font").width(200.0).selected_text(&s.font).show_ui(ui, |ui| {
                for f in &list {
                    if ui.selectable_label(&s.font == f, f).clicked() {
                        s.font = f.clone();
                        ch = true;
                    }
                }
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            w::label_value(ui, "Tamanho", &format!("{}px", s.size));
            ch |= ui.add(egui::Slider::new(&mut s.size, 32.0..=180.0).step_by(2.0).show_value(false)).changed();
        });
    });
    ui.horizontal(|ui| {
        if w::button(ui, if s.bold { Kind::Primary } else { Kind::Default }, None, "Negrito").clicked() {
            s.bold = !s.bold;
            ch = true;
        }
        if w::button(ui, if s.uppercase { Kind::Primary } else { Kind::Default }, None, "Caixa alta").clicked() {
            s.uppercase = !s.uppercase;
            ch = true;
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ch |= color_field(ui, "Texto", &mut s.color);
        ui.add_space(16.0);
        let hl = if s.kind == CapKind::Box { "Caixa" } else { "Destaque" };
        ch |= color_field(ui, hl, &mut s.highlight);
        ui.add_space(16.0);
        ch |= color_field(ui, "Contorno", &mut s.outline_color);
    });
    w::label_value(ui, "Contorno", &format!("{}px", s.outline));
    ch |= ui.add(egui::Slider::new(&mut s.outline, 0.0..=16.0).step_by(1.0).show_value(false)).changed();
    if s.kind == CapKind::Classic {
        ui.horizontal(|ui| {
            ch |= color_field(ui, "Cor do fundo", &mut s.back_color);
            ui.add_space(16.0);
            ui.vertical(|ui| {
                w::label_value(ui, "Opacidade", &format!("{}%", (s.back_opacity * 100.0).round()));
                ch |= ui.add(egui::Slider::new(&mut s.back_opacity, 0.0..=1.0).step_by(0.05).show_value(false)).changed();
            });
        });
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            w::label(ui, "Posição");
            ch |= w::seg(ui, &mut s.position, &[("top".to_string(), "Topo"), ("center".to_string(), "Centro"), ("bottom".to_string(), "Base")]);
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            w::label_value(ui, "Distância da borda", &format!("{}%", (s.margin * 100.0).round()));
            ui.add_enabled_ui(s.position != "center", |ui| {
                ch |= ui.add(egui::Slider::new(&mut s.margin, 0.0..=0.45).step_by(0.01).show_value(false)).changed();
            });
        });
    });
    if s.kind != CapKind::Word {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                w::label_value(ui, "Palavras por vez", &s.max_words.to_string());
                ch |= ui.add(egui::Slider::new(&mut s.max_words, 1..=10).show_value(false)).changed();
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                w::label_value(ui, "Caracteres por linha", &s.max_chars.to_string());
                ch |= ui.add(egui::Slider::new(&mut s.max_chars, 8..=60).show_value(false)).changed();
            });
        });
    }
    ch
}
