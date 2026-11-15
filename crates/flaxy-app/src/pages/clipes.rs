//! Clipes de retenção: link (ou arquivo local) -> transcrição -> IA sugere
//! trechos -> você aprova e marca câmera/tela -> corte.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::captions::Style;
use flaxy_core::clips::{suggest_rects, ClipEdit, NewProject, Project, ProjectView, Rect as FRect, RenderRequest};
use flaxy_core::templates::ALLOWED_VIDEO;

use crate::app::{Page, Shared};
use crate::backend::{self, Res, Task};
use crate::editors::rect_editor::{self, RectEditor};
use crate::format::{date_time, timecode};
use crate::icons as ic;
use crate::media::{self, fit_rect, Player};
use crate::theme::*;
use crate::widgets::{self as w, Kind, Tone};

const RANGES: [(&str, f64, f64); 4] = [("15–30s", 15.0, 30.0), ("30–60s", 30.0, 60.0), ("60–90s", 60.0, 90.0), ("Automático", 15.0, 120.0)];
const LAYOUTS: [(&str, &str); 3] = [("vertical_camera", "Vertical · câmera"), ("vertical_split", "Vertical · tela + câmera"), ("horizontal", "Horizontal")];

fn stage(s: &str) -> &'static str {
    match s {
        "downloading" => "Baixando",
        "transcribing" => "Transcrevendo",
        "analyzing" => "Analisando retenção",
        "ready" => "Pronto para revisar",
        "error" => "Falhou",
        _ => "",
    }
}

fn busy(p: &Project) -> bool {
    matches!(p.status.as_str(), "downloading" | "transcribing" | "analyzing")
}

#[derive(Clone)]
struct Draft {
    approved: bool,
    layout: String,
    camera: FRect,
    screen: FRect,
}

pub struct Clipes {
    loaded: bool,
    url: String,
    range: usize,
    layout: String,
    projects: Vec<Project>,
    sel: Option<i64>,
    view: Option<ProjectView>,
    drafts: HashMap<i64, Draft>,
    styles: Vec<Style>,
    style_id: Option<i64>,
    player: Option<Player>,
    playing_clip: Option<i64>,
    marking: Option<RectEditor>,
    yt: crate::youtube::Cache,
    t_list: Option<Task<Res<Vec<Project>>>>,
    t_view: Option<Task<Res<ProjectView>>>,
    t_create: Option<Task<Res<ProjectView>>>,
    t_render: Option<Task<Res<ProjectView>>>,
    t_delete: Option<Task<Res<()>>>,
    t_retry: Option<Task<Res<ProjectView>>>,
    t_styles: Option<Task<Res<Vec<Style>>>>,
    last_poll: f64,
    confirm_delete: Option<Project>,
}

impl Default for Clipes {
    fn default() -> Self {
        Self {
            loaded: false,
            url: String::new(),
            range: 1,
            layout: "vertical_camera".into(),
            projects: vec![],
            sel: None,
            view: None,
            drafts: HashMap::new(),
            styles: vec![],
            style_id: None,
            player: None,
            playing_clip: None,
            marking: None,
            yt: Default::default(),
            t_list: None,
            t_view: None,
            t_create: None,
            t_render: None,
            t_delete: None,
            t_retry: None,
            t_styles: None,
            last_poll: 0.0,
            confirm_delete: None,
        }
    }
}

impl Clipes {
    fn load_list(&mut self, ui: &Ui, sh: &Shared) {
        let bk = sh.bk.clone();
        self.t_list = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.list().await.map_err(backend::e) }));
    }

    fn load_view(&mut self, ui: &Ui, sh: &Shared, id: i64) {
        let bk = sh.bk.clone();
        self.t_view = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.get(id).await.map_err(backend::e) }));
    }

    fn select(&mut self, ui: &Ui, sh: &Shared, id: i64) {
        if self.sel != Some(id) {
            self.sel = Some(id);
            self.view = None;
            self.drafts.clear();
            self.player = None;
            self.playing_clip = None;
            self.load_view(ui, sh, id);
        }
    }

    fn apply_view(&mut self, v: ProjectView) {
        for c in &v.clips {
            self.drafts.entry(c.row.id).or_insert_with(|| Draft {
                approved: c.row.approved || c.row.score >= 70,
                layout: c.row.layout.clone(),
                camera: c.camera,
                screen: c.screen,
            });
        }
        if self.player.is_none() {
            if let Some(src) = v.project.source_path.as_deref().filter(|_| v.project.status == "ready") {
                self.player = Some(Player::new(src, 0.5, (960, 540)));
            }
        }
        self.view = Some(v);
    }

    fn poll(&mut self, ui: &Ui, sh: &mut Shared) {
        if let Some(r) = self.t_list.as_ref().and_then(|t| t.take()) {
            self.t_list = None;
            match r {
                Ok(l) => {
                    self.projects = l;
                    if self.sel.is_none() {
                        if let Some(id) = self.projects.first().map(|p| p.id) {
                            self.select(ui, sh, id);
                        }
                    }
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_view.as_ref().and_then(|t| t.take()) {
            self.t_view = None;
            match r {
                Ok(v) if Some(v.project.id) == self.sel => self.apply_view(v),
                Ok(_) => {}
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_styles.as_ref().and_then(|t| t.take()) {
            self.t_styles = None;
            if let Ok(s) = r {
                self.styles = s;
            }
        }
        if let Some(r) = self.t_create.as_ref().and_then(|t| t.take()) {
            self.t_create = None;
            match r {
                Ok(v) => {
                    self.url.clear();
                    let id = v.project.id;
                    self.load_list(ui, sh);
                    self.select(ui, sh, id);
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_render.as_ref().and_then(|t| t.take()) {
            self.t_render = None;
            match r {
                Ok(v) => {
                    let n = v.clips.iter().filter(|c| c.row.status == "queued" || c.row.status == "processing").count();
                    sh.toasts.ok(format!("{n} clipes na fila"));
                    self.apply_view(v);
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_retry.as_ref().and_then(|t| t.take()) {
            self.t_retry = None;
            match r {
                Ok(v) => {
                    self.load_list(ui, sh);
                    if self.sel == Some(v.project.id) {
                        self.apply_view(v);
                    }
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_delete.as_ref().and_then(|t| t.take()) {
            self.t_delete = None;
            if let Err(e) = r {
                sh.toasts.err(e);
            }
            self.load_list(ui, sh);
        }
        // Atualiza enquanto há pipeline ou render em andamento.
        let working = self.projects.iter().any(busy)
            || self.view.as_ref().is_some_and(|v| busy(&v.project) || v.clips.iter().any(|c| c.row.status == "queued" || c.row.status == "processing"));
        if working {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            let now = ui.input(|i| i.time);
            if now - self.last_poll > 1.5 && self.t_list.is_none() && self.t_view.is_none() {
                self.last_poll = now;
                self.load_list(ui, sh);
                if let Some(id) = self.sel {
                    self.load_view(ui, sh, id);
                }
            }
        }
    }

    fn create(&mut self, ui: &Ui, sh: &Shared, url: String) {
        let (_, min, max) = RANGES[self.range];
        let bk = sh.bk.clone();
        let layout = self.layout.clone();
        self.t_create = Some(sh.bk.spawn(ui.ctx(), async move {
            bk.clips.create(NewProject { url, min_len: min, max_len: max, layout: Some(layout) }).await.map_err(backend::e)
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if let Some(u) = sh.clip_url.take() {
            self.url = u;
        }
        if !self.loaded {
            self.loaded = true;
            self.load_list(ui, sh);
            let bk = sh.bk.clone();
            self.t_styles = Some(sh.bk.spawn(ui.ctx(), async move { flaxy_core::captions::list(&bk.db).await.map_err(backend::e) }));
        }
        self.poll(ui, sh);
        if let Some(p) = self.player.as_mut() {
            p.update(ui.ctx());
            if !p.is_playing() {
                self.playing_clip = None;
            }
        }

        w::page_head(ui, "Clipes", "Cole um vídeo longo. A IA acha os trechos que prendem e você aprova antes de cortar", |_| {});

        let ytdlp = flaxy_core::clips::ytdlp().is_some();
        w::card_frame().show(ui, |ui| {
            w::pad(ui, |ui| {
                ui.horizontal(|ui| {
                    let bw = 420.0;
                    let field_w = ui.available_width() - bw - 8.0;
                    egui::Frame::NONE
                        .fill(RAISED)
                        .stroke(Stroke::new(1.0, LINE_2))
                        .corner_radius(CornerRadius::same(R))
                        .inner_margin(egui::Margin::symmetric(12, 0))
                        .show(ui, |ui| {
                            ui.set_width(field_w - 24.0);
                            ui.set_height(44.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(RichText::new(ic::LINK_2).color(TEXT_3));
                                let r = ui.add(
                                    egui::TextEdit::singleline(&mut self.url)
                                        .hint_text("https://www.youtube.com/watch?v=…")
                                        .frame(egui::Frame::NONE)
                                        .desired_width(f32::INFINITY)
                                        .font(body(T_BODY)),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.url.trim().is_empty() {
                                    let u = self.url.trim().to_string();
                                    self.create(ui, sh, u);
                                }
                            });
                        });
                    if w::button_sized(ui, Kind::Default, Some(ic::CLIPBOARD_PASTE), "Colar", 44.0).on_hover_text("Colar o link copiado (Ctrl+V)").clicked() {
                        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            Ok(t) if !t.trim().is_empty() => self.url = t.trim().to_string(),
                            _ => sh.toasts.err("A área de transferência não tem um link"),
                        }
                    }
                    if w::button_sized(ui, Kind::Default, Some(ic::FILM), "Arquivo do PC", 44.0).on_hover_text("Usar um vídeo longo que já está no computador").clicked() {
                        if let Some(f) = rfd::FileDialog::new().set_title("Escolher vídeo longo").add_filter("Vídeos", ALLOWED_VIDEO).pick_file() {
                            self.create(ui, sh, f.to_string_lossy().into_owned());
                        }
                    }
                    ui.add_enabled_ui(!self.url.trim().is_empty() && self.t_create.is_none() && ytdlp, |ui| {
                        if w::button_sized(ui, Kind::Primary, Some(ic::SCISSORS), "Encontrar clipes", 44.0).clicked() {
                            let u = self.url.trim().to_string();
                            self.create(ui, sh, u);
                        }
                    });
                });
                // Capa e título do vídeo colado, para conferir antes de começar.
                if let Some(id) = crate::youtube::video_id(&self.url) {
                    ui.add_space(12.0);
                    let info = self.yt.get(&sh.bk, ui.ctx(), &id);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let (r, _) = ui.allocate_exact_size(egui::vec2(160.0, 90.0), Sense::hover());
                        match info.as_ref().and_then(|i| i.thumb.clone()) {
                            Some(t) => {
                                ui.painter().rect_filled(r, CornerRadius::same(R), Color32::BLACK);
                                media::paint_image(ui, &t, r, true);
                                ui.painter().rect_stroke(r, CornerRadius::same(R), Stroke::new(1.0, LINE), StrokeKind::Inside);
                            }
                            None => {
                                ui.painter().rect_filled(r, CornerRadius::same(R), RAISED);
                                ui.painter().text(r.center(), Align2::CENTER_CENTER, ic::IMAGE, body(ICON), TEXT_3);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.add_space(12.0);
                            match &info {
                                None => {
                                    ui.label(w::muted("Buscando capa do vídeo"));
                                }
                                Some(i) if i.title.is_empty() => {
                                    ui.label(w::dim("Vídeo do YouTube"));
                                    ui.label(w::mono_text(&id, T_META, TEXT_3));
                                }
                                Some(i) => {
                                    ui.add(egui::Label::new(RichText::new(&i.title).font(strong(T_BODY)).color(TEXT)).truncate());
                                    ui.label(w::dim(&i.channel));
                                    ui.label(w::mono_text(&id, T_META, TEXT_3));
                                }
                            }
                        });
                    });
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        w::label(ui, "Duração de cada clipe");
                        let opts: Vec<(usize, &str)> = RANGES.iter().enumerate().map(|(i, r)| (i, r.0)).collect();
                        w::seg(ui, &mut self.range, &opts);
                    });
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        w::label(ui, "Layout padrão");
                        let opts: Vec<(String, &str)> = LAYOUTS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
                        w::seg(ui, &mut self.layout, &opts);
                    });
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        ui.add_space(24.0);
                        if ytdlp {
                            if ui.link(RichText::new("Transcrição e LLM configurados na aba IA").font(body(T_META))).clicked() {
                                sh.goto = Some(Page::Ia);
                            }
                        } else {
                            ui.label(RichText::new("yt-dlp não encontrado. Links não funcionam, arquivos do PC sim.").font(body(T_META)).color(ERR));
                        }
                    });
                });
            });
        });
        ui.add_space(12.0);

        let list_w = 290.0;
        let detail_w = ui.available_width() - list_w - 12.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            w::column(ui, list_w, |ui| {
                self.list(ui, sh);
            });
            w::column(ui, detail_w, |ui| {
                self.detail(ui, sh);
            });
        });
    }

    fn list(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let meta = self.projects.len().to_string();
        let mut pick = None;
        let mut del = None;
        w::card(ui, "Vídeos", &meta, |_| {}, |ui| {
            if self.projects.is_empty() {
                w::empty(ui, ic::SCISSORS, "Nenhum vídeo ainda");
                return;
            }
            for (i, p) in self.projects.iter().enumerate() {
                if i > 0 {
                    w::line(ui);
                }
                let on = self.sel == Some(p.id);
                let resp = ui.scope_builder(egui::UiBuilder::new().id_salt(("clip-proj", p.id)).sense(Sense::click()), |ui| egui::Frame::NONE.fill(if on { RAISED } else { Color32::TRANSPARENT }).inner_margin(egui::Margin { left: 20, right: 8, top: 12, bottom: 12 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (tr, _) = ui.allocate_exact_size(egui::vec2(72.0, 40.0), Sense::hover());
                        ui.painter().rect_filled(tr, CornerRadius::same(R_SM), RAISED);
                        let cover = match crate::youtube::video_id(&p.url) {
                            Some(id) => self.yt.get(&sh.bk, ui.ctx(), &id).and_then(|i| i.thumb),
                            None => p.source_path.as_deref().and_then(|src| sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(src), 1.0, (144, 80))),
                        };
                        if let Some(t) = cover {
                            let pp = ui.painter_at(tr);
                            let [tw, th] = t.size();
                            let sc = (tr.width() / tw as f32).max(tr.height() / th as f32);
                            pp.image(t.id(), Rect::from_center_size(tr.center(), egui::vec2(tw as f32 * sc, th as f32 * sc)), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        } else {
                            ui.painter().text(tr.center(), Align2::CENTER_CENTER, ic::FILM, body(ICON), TEXT_3);
                        }
                        ui.allocate_ui(egui::vec2(ui.available_width() - 34.0, 40.0), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                w::truncated(ui, RichText::new(if p.title.is_empty() { &p.url } else { &p.title }).font(body(T_SEC)).color(TEXT));
                                if busy(p) {
                                    ui.label(w::muted(if p.status == "downloading" { format!("{} {}%", stage(&p.status), p.progress) } else { stage(&p.status).to_string() }));
                                    w::progress(ui, if p.status == "downloading" { p.progress as f32 / 100.0 } else { 1.0 }, if p.status == "downloading" { Tone::Accent } else { Tone::Neutral }, None);
                                } else {
                                    let txt = format!("{} · {}", if p.status == "error" { "Falhou".to_string() } else { timecode(p.duration, false) }, date_time(&p.created_at));
                                    ui.label(w::mono_text(txt, T_META, if p.status == "error" { ERR } else { TEXT_3 }));
                                }
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_danger(ui, ic::TRASH_2, "Apagar").clicked() {
                                del = Some(p.clone());
                            }
                        });
                    });
                })).response;
                if resp.clicked() && del.is_none() {
                    pick = Some(p.id);
                }
            }
        });
        if let Some(id) = pick {
            self.select(ui, sh, id);
        }
        if del.is_some() {
            self.confirm_delete = del;
        }
    }

    fn detail(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let Some(v) = self.view.clone() else {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::SCISSORS, if self.sel.is_some() { "Carregando" } else { "Escolha um vídeo" });
            });
            return;
        };
        let p = &v.project;
        if busy(p) || p.status == "error" {
            // Capa: miniatura do YouTube ou, para arquivo local, um quadro do vídeo.
            let cover = match crate::youtube::video_id(&p.url) {
                Some(id) => self.yt.get(&sh.bk, ui.ctx(), &id).and_then(|i| i.thumb),
                None => p.source_path.as_deref().or(Some(p.url.as_str()).filter(|u| Path::new(u).is_file())).and_then(|src| sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(src), 1.0, (320, 180))),
            };
            w::card_frame().show(ui, |ui| {
                w::pad(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 20.0;
                        let (tr, _) = ui.allocate_exact_size(egui::vec2(192.0, 108.0), Sense::hover());
                        ui.painter().rect_filled(tr, CornerRadius::same(R), RAISED);
                        match cover {
                            Some(t) => {
                                media::paint_image(ui, &t, tr, true);
                                ui.painter().rect_stroke(tr, CornerRadius::same(R), Stroke::new(1.0, LINE), StrokeKind::Inside);
                            }
                            None => {
                                ui.painter().text(tr.center(), Align2::CENTER_CENTER, ic::IMAGE, body(ICON), TEXT_3);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(if p.title.is_empty() { &p.url } else { &p.title }).font(strong(T_BODY)).color(TEXT)).truncate());
                            ui.add_space(12.0);
                            if p.status == "error" {
                                ui.add(egui::Label::new(RichText::new(p.error_msg.clone().unwrap_or_default()).color(ERR)).wrap());
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let label = if self.t_retry.is_some() { "Tentando" } else { "Tentar de novo" };
                                    if ui.add_enabled_ui(self.t_retry.is_none(), |ui| w::button(ui, Kind::Primary, Some(ic::ROTATE_CCW), label)).inner.clicked() {
                                        let (bk, id) = (sh.bk.clone(), p.id);
                                        self.t_retry = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.retry(id).await.map_err(backend::e) }));
                                    }
                                    if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), "Apagar").clicked() {
                                        self.confirm_delete = Some(p.clone());
                                    }
                                });
                                return;
                            }
                            let order = ["downloading", "transcribing", "analyzing"];
                            let cur = order.iter().position(|s| *s == p.status).unwrap_or(0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                for (i, s) in order.iter().enumerate() {
                                    let c = if i < cur { OK } else if i == cur { ACCENT } else { RAISED_2 };
                                    w::status_dot(ui, c);
                                    let mut t = stage(s).to_string();
                                    if i == cur && *s == "downloading" {
                                        t = format!("{t} {}%", p.progress);
                                    }
                                    ui.label(RichText::new(t).color(if i > cur { TEXT_3 } else { TEXT }));
                                    ui.add_space(16.0);
                                }
                            });
                            if p.status == "downloading" {
                                ui.add_space(8.0);
                                w::progress(ui, p.progress as f32 / 100.0, Tone::Accent, Some(320.0));
                            }
                            ui.add_space(12.0);
                            ui.label(w::muted("Vídeos longos levam alguns minutos na transcrição local"));
                        });
                    });
                });
            });
            return;
        }

        // Prévia + mapa de retenção.
        let meta = format!("{} · {} sugestões", timecode(p.duration, false), v.clips.len());
        let mut seek_clip: Option<(i64, f64, f64)> = None;
        w::card(ui, &p.title, &meta, |_| {}, |ui| {
            w::pad(ui, |ui| {
                let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 360.0), Sense::hover());
                ui.painter().rect_filled(outer, CornerRadius::same(R), Color32::BLACK);
                if let Some(pl) = &self.player {
                    let img = fit_rect(outer, pl.aspect());
                    if let Some(t) = pl.texture() {
                        ui.painter().image(t.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.painter().text(outer.left_bottom() + egui::vec2(12.0, -12.0), Align2::LEFT_BOTTOM, timecode(pl.pos, true), mono(T_META), TEXT_2);
                }
                ui.add_space(12.0);
                // Mapa de retenção (momento da direção): régua, quadros do vídeo,
                // curva de retenção em cobalto e os trechos sugeridos.
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 104.0), Sense::click());
                let pt = ui.painter_at(rect);
                pt.rect(rect, CornerRadius::same(R), BG_DEEP, Stroke::new(1.0, LINE), StrokeKind::Inside);
                let inner = rect.shrink(1.0);
                w::ruler(ui, Rect::from_min_size(inner.min, egui::vec2(inner.width(), 16.0)), 12.0);
                let strip = Rect::from_min_max(egui::pos2(inner.left(), inner.top() + 16.0), egui::pos2(inner.right(), inner.top() + 56.0));
                let lane = Rect::from_min_max(egui::pos2(inner.left(), strip.bottom() + 4.0), egui::pos2(inner.right(), inner.bottom() - 8.0));
                let dur = p.duration.max(0.001);
                let x = |t: f64| inner.left() + inner.width() * (t / dur) as f32;
                if let Some(src) = p.source_path.as_deref() {
                    let cell_w = strip.height() * if p.height > 0 { p.width as f32 / p.height as f32 } else { 16.0 / 9.0 };
                    let n = ((strip.width() / cell_w).ceil() as usize).clamp(1, 20);
                    let cw = strip.width() / n as f32;
                    for i in 0..n {
                        let cell = Rect::from_min_size(egui::pos2(strip.left() + cw * i as f32, strip.top()), egui::vec2(cw, strip.height()));
                        let t = (i as f64 + 0.5) / n as f64 * dur;
                        match sh.thumbs.get(&sh.bk, ui.ctx(), std::path::Path::new(src), t, (160, 90)) {
                            Some(tex) => {
                                let [tw, th] = tex.size();
                                let sc = (cell.width() / tw as f32).max(cell.height() / th as f32);
                                ui.painter_at(cell.shrink2(egui::vec2(0.5, 0.0))).image(tex.id(), Rect::from_center_size(cell.center(), egui::vec2(tw as f32 * sc, th as f32 * sc)), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            None => {
                                pt.rect_filled(cell.shrink2(egui::vec2(0.5, 0.0)), CornerRadius::ZERO, RAISED);
                            }
                        }
                    }
                    pt.rect_filled(strip, CornerRadius::ZERO, Color32::from_black_alpha(90));
                }
                // Curva: nota da melhor sugestão em cada instante, suavizada.
                let samples = 240usize;
                let raw: Vec<f32> = (0..samples)
                    .map(|i| {
                        let t = (i as f64 + 0.5) / samples as f64 * dur;
                        v.clips.iter().filter(|c| t >= c.row.start && t <= c.row.end).map(|c| c.row.score as f32 / 100.0).fold(0.0, f32::max)
                    })
                    .collect();
                let smooth: Vec<f32> = (0..samples)
                    .map(|i| {
                        let a = i.saturating_sub(3);
                        let b = (i + 4).min(samples);
                        raw[a..b].iter().sum::<f32>() / (b - a) as f32
                    })
                    .collect();
                let curve_h = lane.height() - 8.0;
                let pts: Vec<egui::Pos2> = smooth
                    .iter()
                    .enumerate()
                    .map(|(i, v)| egui::pos2(lane.left() + lane.width() * (i as f32 + 0.5) / samples as f32, lane.bottom() - 8.0 - v * curve_h))
                    .collect();
                let mut area = pts.clone();
                area.push(egui::pos2(lane.right(), lane.bottom() - 8.0));
                area.push(egui::pos2(lane.left(), lane.bottom() - 8.0));
                for w2 in pts.windows(2) {
                    let base = lane.bottom() - 8.0;
                    pt.add(egui::Shape::convex_polygon(
                        vec![w2[0], w2[1], egui::pos2(w2[1].x, base), egui::pos2(w2[0].x, base)],
                        Color32::from_rgba_unmultiplied(59, 99, 214, 40),
                        Stroke::NONE,
                    ));
                }
                pt.add(egui::Shape::line(pts, Stroke::new(1.5, ACCENT)));
                // Trechos sugeridos: barra fina; aprovados em cobalto.
                for c in &v.clips {
                    let ap = self.drafts.get(&c.row.id).is_some_and(|d| d.approved);
                    let r = Rect::from_min_max(egui::pos2(x(c.row.start), lane.bottom() - 4.0), egui::pos2(x(c.row.end).max(x(c.row.start) + 2.0), lane.bottom()));
                    pt.rect_filled(r, CornerRadius::same(2), if ap { ACCENT } else { RAISED_2 });
                }
                if let Some(pl) = &self.player {
                    let px = x(pl.pos);
                    pt.vline(px, inner.y_range(), Stroke::new(1.0, TEXT));
                    pt.add(egui::Shape::convex_polygon(vec![egui::pos2(px - 4.0, inner.top()), egui::pos2(px + 4.0, inner.top()), egui::pos2(px, inner.top() + 6.0)], TEXT, Stroke::NONE));
                }
                if resp.clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() {
                        let t = ((pp.x - inner.left()) / inner.width()) as f64 * dur;
                        let hit = v.clips.iter().find(|c| t >= c.row.start && t <= c.row.end);
                        match hit {
                            Some(c) => seek_clip = Some((c.row.id, c.row.start, c.row.end)),
                            None => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.stop_at = None;
                                    pl.seek(t);
                                }
                            }
                        }
                    }
                }
            });
        });
        ui.add_space(12.0);

        let approved = v.clips.iter().filter(|c| self.drafts.get(&c.row.id).is_some_and(|d| d.approved)).count();
        let mut mark: Option<i64> = None;
        w::card(ui, "Sugestões da IA", &format!("{approved} aprovados"), |_| {}, |ui| {
            for (i, c) in v.clips.iter().enumerate() {
                if i > 0 {
                    w::line(ui);
                }
                let Some(d) = self.drafts.get_mut(&c.row.id) else { continue };
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 16)).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.set_width(ui.available_width());
                        if w::check(ui, d.approved, 20.0).on_hover_text(if d.approved { "Desaprovar" } else { "Aprovar" }).clicked() {
                            d.approved = !d.approved;
                        }
                        ui.add_space(8.0);
                        let (fr, _) = ui.allocate_exact_size(egui::vec2(112.0, 64.0), Sense::hover());
                        ui.painter().rect_filled(fr, CornerRadius::same(R_SM), RAISED);
                        if let Some(src) = p.source_path.as_deref() {
                            if let Some(t) = sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(src), c.row.start + 1.0, (224, 128)) {
                                let pp = ui.painter_at(fr);
                                let [tw, th] = t.size();
                                let sc = (fr.width() / tw as f32).max(fr.height() / th as f32);
                                pp.image(t.id(), Rect::from_center_size(fr.center(), egui::vec2(tw as f32 * sc, th as f32 * sc)), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_width(52.0);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new(c.row.score.to_string()).font(mono(T_KEY)).color(if i == 0 { ACCENT_INK } else { TEXT }));
                            ui.label(w::muted("retenção"));
                        });
                        let text_w = (ui.available_width() - 330.0).max(160.0);
                        w::column(ui, text_w, |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.label(RichText::new(&c.row.title).font(strong(T_BODY)).color(TEXT));
                                ui.add(egui::Label::new(w::dim(&c.row.reason)).wrap());
                                ui.label(w::mono_text(format!("{} → {} · {}s", timecode(c.row.start, true), timecode(c.row.end, true), (c.row.end - c.row.start).round()), T_META, TEXT_3));
                                match c.row.status.as_str() {
                                    "done" => {
                                        if let Some(out) = &c.row.output_path {
                                            ui.horizontal(|ui| {
                                                if w::button(ui, Kind::Default, Some(ic::PLAY), "Abrir clipe").clicked() {
                                                    backend::open_path(Path::new(out));
                                                }
                                                if w::button(ui, Kind::Ghost, Some(ic::FOLDER), "Mostrar na pasta").clicked() {
                                                    backend::reveal(Path::new(out));
                                                }
                                            });
                                        }
                                    }
                                    "error" => {
                                        ui.label(RichText::new(c.row.error_msg.clone().unwrap_or_default()).font(body(T_META)).color(ERR));
                                    }
                                    "queued" | "processing" => {
                                        w::progress(ui, c.row.progress as f32 / 100.0, Tone::Accent, Some(200.0));
                                    }
                                    _ => {}
                                }
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.add_enabled_ui(d.layout != "horizontal", |ui| {
                                if w::button(ui, Kind::Default, Some(ic::CROP), "Áreas").on_hover_text("Marcar câmera e tela").clicked() {
                                    mark = Some(c.row.id);
                                }
                            });
                            let playing = self.playing_clip == Some(c.row.id);
                            if w::icon_button(ui, if playing { ic::PAUSE } else { ic::PLAY }, "Assistir trecho").clicked() {
                                if playing {
                                    if let Some(pl) = self.player.as_mut() {
                                        pl.pause();
                                    }
                                } else {
                                    seek_clip = Some((c.row.id, c.row.start, c.row.end));
                                }
                            }
                            let cur = LAYOUTS.iter().find(|l| l.0 == d.layout).map(|l| l.1).unwrap_or("");
                            egui::ComboBox::from_id_salt(("layout", c.row.id)).width(200.0).selected_text(cur).show_ui(ui, |ui| {
                                for (k, l) in LAYOUTS {
                                    if ui.selectable_label(d.layout == k, l).clicked() && d.layout != k {
                                        d.layout = k.to_string();
                                        let (cam, scr) = suggest_rects(k, p.width, p.height);
                                        d.camera = cam;
                                        d.screen = scr;
                                    }
                                }
                            });
                        });
                    });
                });
            }
            w::line(ui);
            w::pad(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(w::dim("Legenda"));
                    let cur = self.style_id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.row.name.clone()).unwrap_or_else(|| "Sem legenda".into());
                    egui::ComboBox::from_id_salt("clip-style").width(220.0).selected_text(cur).show_ui(ui, |ui| {
                        if ui.selectable_label(self.style_id.is_none(), "Sem legenda").clicked() {
                            self.style_id = None;
                        }
                        for s in &self.styles {
                            if ui.selectable_label(self.style_id == Some(s.row.id), &s.row.name).clicked() {
                                self.style_id = Some(s.row.id);
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(approved > 0 && self.t_render.is_none(), |ui| {
                            let label = format!("Cortar {approved} {}", if approved == 1 { "clipe" } else { "clipes" });
                            if w::button_sized(ui, Kind::Primary, Some(ic::SCISSORS), &label, 36.0).clicked() {
                                let req = RenderRequest {
                                    caption_style_id: self.style_id,
                                    clips: v
                                        .clips
                                        .iter()
                                        .filter_map(|c| {
                                            let d = self.drafts.get(&c.row.id)?;
                                            Some(ClipEdit { id: c.row.id, approved: d.approved, layout: d.layout.clone(), camera: d.camera, screen: d.screen, start: None, end: None })
                                        })
                                        .collect(),
                                };
                                let (bk, id) = (sh.bk.clone(), p.id);
                                self.t_render = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.render(id, req).await.map_err(backend::e) }));
                            }
                        });
                    });
                });
            });
        });

        if let Some((id, a, b)) = seek_clip {
            if let Some(pl) = self.player.as_mut() {
                pl.seek(a);
                pl.stop_at = Some(b);
                pl.play();
                self.playing_clip = Some(id);
            }
        }
        if let Some(cid) = mark {
            if let (Some(d), Some(src), Some(c)) = (self.drafts.get(&cid), p.source_path.as_deref(), v.clips.iter().find(|c| c.row.id == cid)) {
                self.marking = Some(RectEditor::new(cid, &PathBuf::from(src), c.row.start + 0.5, &d.layout, d.camera, d.screen));
            }
        }
    }

    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        if let Some(ed) = self.marking.as_mut() {
            match ed.show(ctx) {
                rect_editor::Outcome::Open => {}
                rect_editor::Outcome::Close => self.marking = None,
                rect_editor::Outcome::Save(cam, scr) => {
                    if let Some(d) = self.drafts.get_mut(&ed.clip_id) {
                        d.camera = cam;
                        d.screen = scr;
                    }
                    self.marking = None;
                }
            }
        }
        if let Some(p) = self.confirm_delete.clone() {
            let title = if p.title.is_empty() { p.url.clone() } else { p.title.clone() };
            let (open, choice) = w::modal(ctx, "clip-delete", "Apagar vídeo?", "", 460.0, |ui| {
                let mut c = None;
                w::pad(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&title).color(TEXT)).wrap());
                    ui.label(w::dim("O vídeo baixado é apagado. Arquivos do seu PC e clipes já cortados ficam onde estão."));
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
                    let (bk, id) = (sh.bk.clone(), p.id);
                    self.t_delete = Some(sh.bk.spawn(ctx, async move { bk.clips.delete(id).await.map_err(backend::e) }));
                    if self.sel == Some(p.id) {
                        self.sel = None;
                        self.view = None;
                        self.player = None;
                    }
                    self.confirm_delete = None;
                }
                Some(false) => self.confirm_delete = None,
                None if !open => self.confirm_delete = None,
                None => {}
            }
        }
    }
}
