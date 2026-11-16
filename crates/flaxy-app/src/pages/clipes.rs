//! Clipes de retenção: link (ou arquivo local) -> transcrição -> IA sugere
//! trechos -> você aprova e marca câmera/tela -> corte.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::captions::{Style, Word};
use flaxy_core::clips::{caption_at, suggest_rects, AreaFit, ClipEdit, NewProject, Project, ProjectView, Rect as FRect, RenderRequest};
use flaxy_core::models::ClipBackground;
use flaxy_core::templates::ALLOWED_VIDEO;
use flaxy_core::watermarks::Watermark;

use crate::app::{Page, Shared};
use crate::backend::{self, Res, Task};
use crate::editors::clip_modal::{Edit, self, clip_preview, ClipModal};
use crate::format::{date_time, timecode};
use crate::icons as ic;
use crate::media::{self, fit_rect, Player};
use crate::theme::*;
use crate::widgets::{self as w, Kind, Tone};

const RANGES: [(&str, f64, f64); 4] = [("15–30s", 15.0, 30.0), ("30–60s", 30.0, 60.0), ("60–90s", 60.0, 90.0), ("Automático", 15.0, 120.0)];
const LAYOUTS: [(&str, &str); 4] = [
    ("vertical_camera", "Vertical · câmera"),
    ("vertical_split", "Vertical · tela em cima"),
    ("vertical_split_cam", "Vertical · câmera em cima"),
    ("horizontal", "Horizontal"),
];

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

/// Sugestão da IA que ainda pode sair da lista (não foi feita à mão nem cortada).
fn is_ai_pending(c: &flaxy_core::clips::Clip) -> bool {
    c.row.reason != flaxy_core::clips::MANUAL_REASON && !matches!(c.row.status.as_str(), "queued" | "processing" | "done")
}

fn busy(p: &Project) -> bool {
    matches!(p.status.as_str(), "downloading" | "transcribing" | "analyzing")
}

#[derive(Clone)]
struct Draft {
    approved: bool,
    e: Edit,
}

impl Draft {
    /// Áreas e modo de recorte que valem no instante `t` (segundos da fonte).
    fn areas_at(&self, t: f64) -> (FRect, FRect, String) {
        let e = &self.e;
        let k = e.keys.iter().filter(|k| k.t >= e.start && k.t <= t).max_by(|a, b| a.t.total_cmp(&b.t));
        let layout = match k.and_then(|k| k.layout) {
            Some(v) if e.layout != "horizontal" => v.id().to_string(),
            _ => e.layout.clone(),
        };
        match k {
            Some(k) => (k.camera, k.screen, layout),
            None => (e.camera, e.screen, layout),
        }
    }

    /// Algo foi mexido no editor (para marcar o botão "Editar").
    fn edited(&self, row_start: f64, row_end: f64) -> bool {
        let e = &self.e;
        !e.keys.is_empty()
            || e.caption.is_some()
            || !e.cap_keys.is_empty()
            || e.fit != AreaFit::default()
            || !e.cuts.is_empty()
            || !e.word_edits.is_empty()
            || e.smooth
            || (e.smooth_secs - flaxy_core::clips::SMOOTH_SECS).abs() > 1e-6
            || e.curve.is_some()
            || e.bg != flaxy_core::clips::ClipBg::Blur
            || e.watermark_id.is_some()
            || (e.split - flaxy_core::clips::DEFAULT_SPLIT).abs() > 1e-6
            || (e.start - row_start).abs() > 0.01
            || (e.end - row_end).abs() > 0.01
    }
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
    editing: Option<ClipModal>,
    yt: crate::youtube::Cache,
    t_list: Option<Task<Res<Vec<Project>>>>,
    t_view: Option<Task<Res<ProjectView>>>,
    t_create: Option<Task<Res<ProjectView>>>,
    t_render: Option<Task<Res<ProjectView>>>,
    t_delete: Option<Task<Res<()>>>,
    t_retry: Option<Task<Res<ProjectView>>>,
    /// Clipe adicionado/removido à mão.
    t_clip: Option<Task<Res<ProjectView>>>,
    /// Ponta de clipe sendo arrastada no mapa de retenção (clipe, é o início?).
    ret_drag: Option<(i64, bool)>,
    /// Clipe inteiro sendo arrastado no mapa (clipe, distância do clique até o início).
    ret_move: Option<(i64, f64)>,
    /// Trecho novo sendo desenhado no mapa (início, fim).
    ret_new: Option<(f64, f64)>,
    t_styles: Option<Task<Res<Vec<Style>>>>,
    backgrounds: Vec<ClipBackground>,
    watermarks: Vec<Watermark>,
    /// Fundos de clipe e marcas d'água (aba Modelos), juntos.
    t_bgs: Option<Task<Res<(Vec<ClipBackground>, Vec<Watermark>)>>>,
    last_poll: f64,
    confirm_delete: Option<Project>,
    /// Pedido de remover as sugestões da IA (id do projeto), esperando confirmação.
    confirm_clear: Option<i64>,
    /// Aviso da IA (projeto, mensagem): análise falhou, mas a linha do tempo abre.
    ai_warn: Option<(i64, String)>,
    /// Projetos cujo aviso da IA já foi visto (não volta antes do banco atualizar).
    ai_seen: std::collections::HashSet<i64>,
    /// Análise de retenção rodando de novo com a linha do tempo aberta.
    t_analyze: Option<Task<Res<ProjectView>>>,
    /// Trecho mostrado na prévia 9:16.
    focus: Option<i64>,
    volume: f32,
    muted: bool,
    /// Palavras da transcrição do projeto aberto (id da transcrição, palavras).
    words: Option<(i64, Vec<Word>)>,
    t_words: Option<Task<Res<(i64, Vec<Word>)>>>,
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
            editing: None,
            yt: Default::default(),
            t_list: None,
            t_view: None,
            t_create: None,
            t_render: None,
            t_delete: None,
            t_retry: None,
            t_clip: None,
            ret_drag: None,
            ret_move: None,
            ret_new: None,
            t_styles: None,
            backgrounds: vec![],
            watermarks: vec![],
            t_bgs: None,
            last_poll: 0.0,
            confirm_delete: None,
            confirm_clear: None,
            ai_warn: None,
            ai_seen: Default::default(),
            t_analyze: None,
            focus: None,
            volume: 0.8,
            muted: false,
            words: None,
            t_words: None,
        }
    }
}

impl Clipes {
    /// Biblioteca de fundos de clipe (aba Modelos), para o editor.
    fn load_bgs(&mut self, ctx: &egui::Context, sh: &Shared) {
        let bk = sh.bk.clone();
        self.t_bgs = Some(sh.bk.spawn(ctx, async move {
            Ok((
                flaxy_core::templates::list_clip_bgs(&bk.db).await.map_err(backend::e)?,
                flaxy_core::watermarks::list(&bk.db).await.map_err(backend::e)?,
            ))
        }));
    }

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
            self.focus = None;
            self.load_view(ui, sh, id);
        }
    }

    fn apply_view(&mut self, v: ProjectView) {
        for c in &v.clips {
            self.drafts.entry(c.row.id).or_insert_with(|| Draft {
                approved: c.row.approved || c.row.score >= 70,
                e: Edit {
                    layout: c.row.layout.clone(),
                    start: c.row.start,
                    end: c.row.end,
                    camera: c.camera,
                    screen: c.screen,
                    keys: c.keys.clone(),
                    fit: c.fit,
                    caption: c.caption.clone(),
                    cap_keys: c.cap_keys.clone(),
                    split: c.row.split,
                    word_edits: c.word_edits.clone(),
                    cuts: c.cuts.clone(),
                    smooth: c.row.smooth,
                    smooth_secs: c.row.smooth_secs,
                    curve: c.curve,
                    bg: c.bg.clone(),
                    watermark_id: c.row.watermark_id,
                },
            });
        }
        // Bruto apagado depois do corte: solta a prévia.
        if v.project.source_path.is_none() {
            self.player = None;
            self.playing_clip = None;
        }
        if self.player.is_none() {
            if let Some(src) = v.project.source_path.as_deref().filter(|_| v.project.status == "ready") {
                self.player = Some(Player::new(src, 0.5, (960, 540)).with_audio());
            }
        }
        if v.project.status == "ready" && self.ai_warn.is_none() && !self.ai_seen.contains(&v.project.id) {
            if let Some(m) = v.project.error_msg.clone().filter(|m| !m.is_empty()) {
                self.ai_warn = Some((v.project.id, m));
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
        if let Some(r) = self.t_bgs.as_ref().and_then(|t| t.take()) {
            self.t_bgs = None;
            if let Ok((b, wm)) = r {
                if let Some(ed) = self.editing.as_mut() {
                    ed.set_backgrounds(b.clone());
                    ed.set_watermarks(wm.clone());
                }
                self.backgrounds = b;
                self.watermarks = wm;
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
        if let Some(r) = self.t_words.as_ref().and_then(|t| t.take()) {
            self.t_words = None;
            if let Ok(w) = r {
                self.words = Some(w);
            }
        }
        if let Some(r) = self.t_clip.as_ref().and_then(|t| t.take()) {
            self.t_clip = None;
            match r {
                Ok(v) => {
                    let new = v.clips.iter().map(|c| c.row.id).filter(|id| !self.drafts.contains_key(id)).max();
                    self.apply_view(v);
                    if let Some(id) = new {
                        self.focus = Some(id);
                        sh.toasts.ok("Clipe adicionado e aprovado. Arraste as pontas no mapa ou abra o editor para ajustar");
                    }
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
        if let Some(r) = self.t_analyze.as_ref().and_then(|t| t.take()) {
            self.t_analyze = None;
            match r {
                Ok(v) => {
                    // Sugestões antigas da IA saíram: esquece as edições delas.
                    let ids: std::collections::HashSet<i64> = v.clips.iter().map(|c| c.row.id).collect();
                    if self.view.as_ref().is_some_and(|o| o.project.id == v.project.id) {
                        if let Some(o) = self.view.as_ref() {
                            for c in o.clips.iter().filter(|c| !ids.contains(&c.row.id)) {
                                self.drafts.remove(&c.row.id);
                                if self.focus == Some(c.row.id) {
                                    self.focus = None;
                                }
                            }
                        }
                    }
                    let n = v.clips.iter().filter(|c| is_ai_pending(c)).count();
                    if self.sel == Some(v.project.id) {
                        self.apply_view(v);
                    }
                    sh.toasts.ok(format!("Análise de retenção pronta: {n} sugestões da IA"));
                }
                Err(e) => {
                    if let Some(pid) = self.sel {
                        self.ai_warn = Some((pid, e.to_string()));
                    }
                }
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
            self.load_bgs(ui.ctx(), sh);
        }
        self.poll(ui, sh);
        if let Some(p) = self.player.as_mut() {
            p.set_volume(if self.muted { 0.0 } else { self.volume });
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
        let mut mark: Option<i64> = None;
        w::card(ui, &p.title, &meta, |_| {}, |ui| {
            w::pad(ui, |ui| {
                if p.source_path.is_none() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(w::muted("O vídeo bruto foi apagado depois do corte para liberar espaço. Os clipes cortados continuam na pasta de saída"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if self.t_retry.is_some() { "Baixando" } else { "Baixar de novo" };
                            if ui
                                .add_enabled_ui(self.t_retry.is_none(), |ui| w::button(ui, Kind::Default, Some(ic::DOWNLOAD), label))
                                .inner
                                .on_hover_text("Baixa o vídeo outra vez para assistir, editar ou cortar mais clipes. As sugestões e edições continuam")
                                .clicked()
                            {
                                let (bk, id) = (sh.bk.clone(), p.id);
                                self.t_retry = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.restore_source(id).await.map_err(backend::e) }));
                            }
                        });
                    });
                    ui.add_space(10.0);
                }
                if let Some(tid) = p.transcript_id {
                    if self.words.as_ref().map(|w| w.0) != Some(tid) && self.t_words.is_none() {
                        let bk = sh.bk.clone();
                        self.t_words = Some(sh.bk.spawn(ui.ctx(), async move {
                            let (w,): (String,) = sqlx::query_as("SELECT words FROM transcripts WHERE id=?").bind(tid).fetch_one(&bk.db).await.map_err(backend::e)?;
                            Ok((tid, serde_json::from_str(&w).map_err(backend::e)?))
                        }));
                    }
                }
                let mut toggle = ui.input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().egui_wants_keyboard_input() && self.editing.is_none() && self.confirm_delete.is_none();
                const VID_H: f32 = 384.0;
                const PREV_W: f32 = 216.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let left_w = (ui.available_width() - PREV_W - 16.0).max(240.0);
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        let (outer, vr) = ui.allocate_exact_size(egui::vec2(left_w, VID_H), Sense::click());
                        ui.painter().rect_filled(outer, CornerRadius::same(R), Color32::BLACK);
                        if let Some(t) = self.player.as_ref().and_then(|pl| pl.texture().map(|t| (t, pl.aspect()))) {
                            ui.painter_at(outer).image(t.0.id(), fit_rect(outer, t.1), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        if vr.clicked() {
                            toggle = true;
                        }
                        ui.add_space(8.0);
                        // Controles: reproduzir/pausar, tempo, volume.
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let (playing, pos) = self.player.as_ref().map(|pl| (pl.is_playing(), pl.pos)).unwrap_or((false, 0.0));
                            if w::icon_button(ui, if playing { ic::PAUSE } else { ic::PLAY }, if playing { "Pausar (espaço)" } else { "Reproduzir (espaço)" }).clicked() {
                                toggle = true;
                            }
                            ui.label(w::mono_text(format!("{} / {}", timecode(pos, true), timecode(p.duration, true)), T_META, TEXT_2));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if w::volume_bar(ui, &mut self.volume, self.muted) && self.volume > 0.0 {
                                    self.muted = false;
                                }
                                let off = self.muted || self.volume <= 0.0;
                                let icon = if off { ic::VOLUME_X } else if self.volume < 0.5 { ic::VOLUME_1 } else { ic::VOLUME_2 };
                                if w::icon_button(ui, icon, if off { "Ligar som" } else { "Sem som" }).clicked() {
                                    if self.volume <= 0.0 {
                                        self.volume = 0.8;
                                        self.muted = false;
                                    } else {
                                        self.muted = !self.muted;
                                    }
                                }
                            });
                        });
                    });
                    // Prévia do clipe como vai sair, com a legenda escolhida.
                    ui.vertical(|ui| {
                        ui.set_width(PREV_W);
                        let (slot, _) = ui.allocate_exact_size(egui::vec2(PREV_W, VID_H), Sense::hover());
                        let focus = self.focus.and_then(|id| v.clips.iter().find(|c| c.row.id == id)).or(v.clips.first());
                        let pos = self.player.as_ref().map(|pl| pl.pos).unwrap_or(0.0);
                        let draft = focus.and_then(|c| self.drafts.get(&c.row.id).map(|d| (c, d)));
                        let (layout, cam, scr, fit, adj, split, bg, wm_id) = match draft {
                            Some((_, d)) => {
                                let (cam, scr, layout) = d.areas_at(pos);
                                (layout, cam, scr, d.e.fit, caption_at(d.e.caption.as_ref(), &d.e.cap_keys, d.e.start, pos), d.e.split, d.e.bg.clone(), d.e.watermark_id)
                            }
                            None => {
                                let (c, s) = suggest_rects(&p.layout, p.width, p.height);
                                (p.layout.clone(), c, s, AreaFit::default(), None, flaxy_core::clips::DEFAULT_SPLIT, Default::default(), None)
                            }
                        };
                        let wm = wm_id.and_then(|id| self.watermarks.iter().find(|w| w.row.id == id)).map(|w| w.items.as_slice());
                        let bg_tex = bg.path().filter(|f| !f.is_empty()).and_then(|f| sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(f), 0.5, (360, 640)));
                        let mut style = self.style_id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.style.clone());
                        if let (Some(st), Some(a)) = (style.as_mut(), adj.as_ref()) {
                            a.apply(st);
                        }
                        let words = self.words.as_ref().filter(|w| Some(w.0) == p.transcript_id).map(|w| w.1.as_slice()).unwrap_or(&[]);
                        let tex = self.player.as_ref().and_then(|pl| pl.texture().map(|t| (t, pl.aspect())));
                        clip_preview(ui, slot, tex, &layout, cam, scr, fit, split, &bg, bg_tex.as_ref(), wm, style.as_ref(), &sh.caption_fonts, words, pos);
                        ui.add_space(8.0);
                        let cur = self.style_id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.row.name.clone()).unwrap_or_else(|| "Sem legenda".into());
                        egui::ComboBox::from_id_salt("clip-style").width(PREV_W).selected_text(format!("{}  {cur}", ic::CAPTIONS)).show_ui(ui, |ui| {
                            if ui.selectable_label(self.style_id.is_none(), "Sem legenda").clicked() {
                                self.style_id = None;
                            }
                            for s in &self.styles {
                                if ui.selectable_label(self.style_id == Some(s.row.id), &s.row.name).clicked() {
                                    self.style_id = Some(s.row.id);
                                }
                            }
                        });
                    });
                });
                if toggle {
                    if let Some(pl) = self.player.as_mut() {
                        if pl.is_playing() {
                            pl.pause();
                        } else {
                            if pl.stop_at.is_some_and(|e| pl.pos >= e - 0.1) {
                                pl.stop_at = None;
                            }
                            pl.play();
                        }
                    }
                }
                ui.add_space(12.0);
                // Mapa de retenção (momento da direção): régua, quadros do vídeo,
                // curva de retenção em cobalto e os trechos sugeridos.
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 104.0), Sense::click_and_drag());
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
                // Trechos sugeridos: barra fina; aprovados em cobalto. Passar o
                // mouse destaca o trecho, que abre o editor do clipe no clique.
                let t_at = |px: f32| (((px - inner.left()) / inner.width()) as f64 * dur).clamp(0.0, dur);
                let span = |c: &flaxy_core::clips::Clip, me: &Self| me.drafts.get(&c.row.id).map(|d| (d.e.start, d.e.end)).unwrap_or((c.row.start, c.row.end));
                let hover_t = resp.hover_pos().filter(|pp| inner.contains(*pp)).map(|pp| t_at(pp.x));
                // Ponta de clipe perto do mouse (para esticar ou encolher).
                let edge_at = |pp: egui::Pos2, me: &Self| -> Option<(i64, bool)> {
                    if pp.y < strip.top() {
                        return None;
                    }
                    v.clips.iter().find_map(|c| {
                        let (a, b) = span(c, me);
                        if (pp.x - x(a)).abs() < 6.0 {
                            Some((c.row.id, true))
                        } else if (pp.x - x(b)).abs() < 6.0 {
                            Some((c.row.id, false))
                        } else {
                            None
                        }
                    })
                };
                // Clipe sob o mouse (corpo, não a ponta).
                let body_at = |pp: egui::Pos2, me: &Self| -> Option<i64> {
                    if pp.y < strip.top() {
                        return None;
                    }
                    let t = t_at(pp.x);
                    // O selecionado ganha quando há clipes encavalados.
                    let hit = |c: &&flaxy_core::clips::Clip| {
                        let (a, b) = span(c, me);
                        t >= a && t <= b
                    };
                    v.clips.iter().filter(hit).find(|c| me.focus == Some(c.row.id)).or_else(|| v.clips.iter().find(hit)).map(|c| c.row.id)
                };
                let busy_id = self.ret_drag.map(|d| d.0).or(self.ret_move.map(|m| m.0));
                let hover_id = busy_id.or_else(|| resp.hover_pos().filter(|pp| inner.contains(*pp)).and_then(|pp| body_at(pp, self)));
                let mut hovered_clip = None;
                for c in &v.clips {
                    let (a, b) = span(c, self);
                    let ap = self.drafts.get(&c.row.id).is_some_and(|d| d.approved);
                    let over = hover_id == Some(c.row.id);
                    let sel = self.focus == Some(c.row.id);
                    if over {
                        hovered_clip = Some(c);
                    }
                    if over || sel {
                        let band = Rect::from_min_max(egui::pos2(x(a), strip.top()), egui::pos2(x(b), lane.bottom()));
                        pt.rect_filled(band, CornerRadius::ZERO, Color32::from_white_alpha(if sel { 22 } else { 14 }));
                        pt.rect_stroke(band, CornerRadius::ZERO, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { TEXT } else { ACCENT }), StrokeKind::Inside);
                        // Alças das pontas.
                        for ex in [x(a), x(b)] {
                            pt.rect_filled(Rect::from_center_size(egui::pos2(ex, (strip.top() + lane.bottom()) / 2.0), egui::vec2(4.0, 22.0)), CornerRadius::same(2), Color32::from_rgb(240, 190, 60));
                        }
                    }
                    let h = if over || sel { 8.0 } else { 4.0 };
                    let r = Rect::from_min_max(egui::pos2(x(a), lane.bottom() - h), egui::pos2(x(b).max(x(a) + 2.0), lane.bottom()));
                    pt.rect_filled(r, CornerRadius::same(2), if ap { ACCENT } else { RAISED_2 });
                }
                if let Some((a, b)) = self.ret_new {
                    let band = Rect::from_min_max(egui::pos2(x(a.min(b)), strip.top()), egui::pos2(x(a.max(b)), lane.bottom()));
                    pt.rect_filled(band, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(240, 190, 60, 50));
                    pt.rect_stroke(band, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(240, 190, 60)), StrokeKind::Inside);
                }
                let on_edge = resp.hover_pos().and_then(|pp| edge_at(pp, self));
                if self.ret_drag.is_some() || (on_edge.is_some() && self.ret_move.is_none()) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                } else if self.ret_move.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if let Some(c) = hovered_clip.filter(|_| self.ret_new.is_none()) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("ret-tip"), egui::PopupAnchor::Pointer).show(|ui| {
                        ui.label(RichText::new(&c.row.title).color(TEXT));
                        ui.label(w::muted("Clique para selecionar · duplo clique para editar · Del tira da lista. Arraste para mover, ou as pontas para ajustar"));
                    });
                } else if hover_t.is_some() && self.ret_new.is_none() {
                    egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("ret-tip2"), egui::PopupAnchor::Pointer).show(|ui| {
                        ui.label(w::muted("Clique para ir ao ponto. Arraste num espaço vazio para criar um clipe"));
                    });
                }
                // Arrastar: ponta de um clipe ou trecho novo.
                if resp.drag_started() {
                    if let Some(pp) = ui.input(|i| i.pointer.press_origin()) {
                        match (edge_at(pp, self), body_at(pp, self)) {
                            (Some(e), _) => {
                                self.ret_drag = Some(e);
                                self.focus = Some(e.0);
                            }
                            (None, Some(id)) => {
                                if let Some(d) = self.drafts.get(&id) {
                                    self.ret_move = Some((id, t_at(pp.x) - d.e.start));
                                    self.focus = Some(id);
                                }
                            }
                            (None, None) if pp.y >= strip.top() => {
                                let t = t_at(pp.x);
                                self.ret_new = Some((t, t));
                            }
                            _ => {}
                        }
                    }
                }
                if let (true, Some(pp)) = (resp.dragged(), resp.interact_pointer_pos()) {
                    let t = t_at(pp.x);
                    if let Some((id, off)) = self.ret_move {
                        // Move o clipe inteiro, levando junto trocas, legendas e cortes.
                        if let Some(d) = self.drafts.get_mut(&id) {
                            let len = d.e.end - d.e.start;
                            let start = (t - off).clamp(0.0, (dur - len).max(0.0));
                            let dt = start - d.e.start;
                            if dt.abs() > 1e-9 {
                                d.e.start += dt;
                                d.e.end += dt;
                                for k in &mut d.e.keys {
                                    k.t += dt;
                                }
                                for k in &mut d.e.cap_keys {
                                    k.t += dt;
                                }
                                for c in &mut d.e.cuts {
                                    c.a += dt;
                                    c.b += dt;
                                }
                            }
                            if let Some(pl) = self.player.as_mut() {
                                pl.pause();
                                pl.stop_at = None;
                                pl.seek(d.e.start);
                            }
                        }
                    } else if let Some((id, is_start)) = self.ret_drag {
                        if let Some(d) = self.drafts.get_mut(&id) {
                            if is_start {
                                d.e.start = t.min(d.e.end - 1.0).max(0.0);
                            } else {
                                d.e.end = t.max(d.e.start + 1.0).min(dur);
                            }
                            let at = if is_start { d.e.start } else { d.e.end };
                            if let Some(pl) = self.player.as_mut() {
                                pl.pause();
                                pl.seek(at);
                            }
                        }
                    } else if let Some((a, _)) = self.ret_new {
                        self.ret_new = Some((a, t));
                    }
                }
                if resp.drag_stopped() {
                    self.ret_drag = None;
                    self.ret_move = None;
                    if let Some((a, b)) = self.ret_new.take() {
                        if (a - b).abs() >= 1.0 && self.t_clip.is_none() {
                            let (bk, id, s0, s1) = (sh.bk.clone(), p.id, a.min(b), a.max(b));
                            self.t_clip = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.add_clip(id, s0, s1).await.map_err(backend::e) }));
                        }
                    }
                }
                if let Some(pl) = &self.player {
                    let px = x(pl.pos);
                    pt.vline(px, inner.y_range(), Stroke::new(1.0, TEXT));
                    pt.add(egui::Shape::convex_polygon(vec![egui::pos2(px - 4.0, inner.top()), egui::pos2(px + 4.0, inner.top()), egui::pos2(px, inner.top() + 6.0)], TEXT, Stroke::NONE));
                }
                // Clique seleciona o clipe (e leva a agulha ao começo dele);
                // duplo clique abre o editor; clique no vazio só move a agulha.
                if resp.clicked() || resp.double_clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() {
                        let t = t_at(pp.x);
                        match body_at(pp, self) {
                            Some(id) if resp.double_clicked() => mark = Some(id),
                            Some(id) => {
                                self.focus = Some(id);
                                if let (Some(pl), Some(d)) = (self.player.as_mut(), self.drafts.get(&id)) {
                                    pl.pause();
                                    pl.stop_at = None;
                                    pl.seek(d.e.start);
                                }
                            }
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
        let mut add_here = false;
        let mut remove: Option<i64> = None;
        let ai_left = v.clips.iter().filter(|c| is_ai_pending(c)).count();
        let mut ask_clear = false;
        let mut analyze = false;
        w::card(ui, "Clipes", &format!("{} · {approved} aprovados", v.clips.len()), |ui| {
            ui.add_enabled_ui(self.t_analyze.is_none() && self.t_clip.is_none() && p.transcript_id.is_some(), |ui| {
                let label = if self.t_analyze.is_some() { "Analisando retenção" } else { "Analisar retenção" };
                if w::button(ui, Kind::Default, Some(ic::TRENDING_UP), label)
                    .on_hover_text("A IA procura de novo os melhores trechos, sem sair desta tela. Troca as sugestões da IA ainda não cortadas; clipes feitos à mão e os já cortados ficam")
                    .clicked()
                {
                    analyze = true;
                }
            });
            ui.add_enabled_ui(self.t_clip.is_none() && p.source_path.is_some(), |ui| {
                if w::button(ui, Kind::Default, Some(ic::PLUS), "Adicionar clipe aqui").on_hover_text("Cria um clipe a partir da agulha. Também dá para arrastar num espaço vazio do mapa").clicked() {
                    add_here = true;
                }
            });
            ui.add_enabled_ui(self.t_clip.is_none() && ai_left > 0, |ui| {
                if w::button(ui, Kind::Ghost, Some(ic::TRASH_2), &format!("Remover sugestões da IA ({ai_left})"))
                    .on_hover_text("Tira da lista e do mapa as sugestões da IA que ainda não foram cortadas, para você marcar os seus clipes. Os feitos à mão ficam")
                    .clicked()
                {
                    ask_clear = true;
                }
            });
        }, |ui| {
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
                                ui.label(w::mono_text(format!("{} → {} · {}s", timecode(d.e.start, true), timecode(d.e.end, true), (d.e.end - d.e.start).round()), T_META, TEXT_3));
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
                            let edited = d.edited(c.row.start, c.row.end);
                            let busy_clip = c.row.status == "queued" || c.row.status == "processing";
                            if !busy_clip && w::icon_button(ui, ic::TRASH_2, "Tirar este clipe da lista").clicked() {
                                remove = Some(c.row.id);
                            }
                            if w::button(ui, Kind::Default, Some(ic::CROP), if edited { "Editar ·" } else { "Editar" }).on_hover_text("Áreas, trocas de câmera e legenda deste clipe").clicked() {
                                mark = Some(c.row.id);
                            }
                            let playing = self.playing_clip == Some(c.row.id);
                            if w::icon_button(ui, if playing { ic::PAUSE } else { ic::PLAY }, "Assistir trecho").clicked() {
                                if playing {
                                    if let Some(pl) = self.player.as_mut() {
                                        pl.pause();
                                    }
                                } else {
                                    seek_clip = Some((c.row.id, d.e.start, d.e.end));
                                }
                            }
                            let cur = LAYOUTS.iter().find(|l| l.0 == d.e.layout).map(|l| l.1).unwrap_or("");
                            egui::ComboBox::from_id_salt(("layout", c.row.id)).width(200.0).selected_text(cur).show_ui(ui, |ui| {
                                for (k, l) in LAYOUTS {
                                    if ui.selectable_label(d.e.layout == k, l).clicked() && d.e.layout != k {
                                        d.e.layout = k.to_string();
                                        let (cam, scr) = suggest_rects(k, p.width, p.height);
                                        d.e.camera = cam;
                                        d.e.screen = scr;
                                        d.e.keys.clear();
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
                    let cur = self.style_id.and_then(|id| self.styles.iter().find(|s| s.row.id == id)).map(|s| s.row.name.clone()).unwrap_or_else(|| "sem legenda".into());
                    ui.label(w::muted(format!("Legenda: {cur}")));
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
                                            Some(ClipEdit {
                                                id: c.row.id,
                                                approved: d.approved,
                                                layout: d.e.layout.clone(),
                                                camera: d.e.camera,
                                                screen: d.e.screen,
                                                start: Some(d.e.start),
                                                end: Some(d.e.end),
                                                keys: d.e.keys.clone(),
                                                fit: d.e.fit,
                                                caption: d.e.caption.clone(),
                                                cap_keys: d.e.cap_keys.clone(),
                                                split: d.e.split,
                                                word_edits: d.e.word_edits.clone(),
                                                cuts: d.e.cuts.clone(),
                                                smooth: d.e.smooth,
                                                smooth_secs: d.e.smooth_secs,
                                                curve: d.e.curve,
                                                bg_mode: d.e.bg.mode().to_string(),
                                                bg_path: d.e.bg.path().map(str::to_string),
                                                watermark_id: d.e.watermark_id,
                                            })
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

        if ask_clear {
            self.confirm_clear = Some(p.id);
        }
        if analyze {
            let (bk, id) = (sh.bk.clone(), p.id);
            self.t_analyze = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.reanalyze(id).await.map_err(backend::e) }));
        }
        // Del com um clipe selecionado no mapa: tira ele da lista.
        if remove.is_none()
            && self.editing.is_none()
            && self.confirm_clear.is_none()
            && self.confirm_delete.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            remove = self.focus.filter(|id| v.clips.iter().any(|c| c.row.id == *id && c.row.status != "queued" && c.row.status != "processing"));
            if remove.is_some() {
                self.focus = None;
            }
        }
        if let (Some(cid), None) = (remove, self.t_clip.as_ref()) {
            self.drafts.remove(&cid);
            let (bk, id) = (sh.bk.clone(), p.id);
            self.t_clip = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.remove_clip(id, cid).await.map_err(backend::e) }));
        }
        if add_here {
            let pos = self.player.as_ref().map(|pl| pl.pos).unwrap_or(0.0);
            let len = ((p.min_len + p.max_len) / 2.0).clamp(10.0, 120.0);
            let (bk, id) = (sh.bk.clone(), p.id);
            self.t_clip = Some(sh.bk.spawn(ui.ctx(), async move { bk.clips.add_clip(id, pos, pos + len).await.map_err(backend::e) }));
        }
        if let Some((id, a, b)) = seek_clip {
            self.focus = Some(id);
            if let Some(pl) = self.player.as_mut() {
                pl.seek(a);
                pl.stop_at = Some(b);
                pl.play();
                self.playing_clip = Some(id);
            }
        }
        // Desenvolvimento: FLAXY_EDIT_CLIP=1 abre o editor do primeiro clipe uma vez.
        let mark = mark.or_else(|| {
            let id = egui::Id::unique("dev-edit-clip");
            let done = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            if done || std::env::var_os("FLAXY_EDIT_CLIP").is_none() || self.editing.is_some() {
                return None;
            }
            let c = v.clips.first().filter(|c| self.drafts.contains_key(&c.row.id))?;
            ui.ctx().data_mut(|d| d.insert_temp(id, true));
            Some(c.row.id)
        });
        if let Some(cid) = mark {
            if let (Some(d), Some(src), Some(c)) = (self.drafts.get(&cid), p.source_path.as_deref(), v.clips.iter().find(|c| c.row.id == cid)) {
                if let Some(pl) = self.player.as_mut() {
                    pl.pause();
                }
                self.focus = Some(cid);
                let words = self
                    .words
                    .as_ref()
                    .filter(|w| Some(w.0) == p.transcript_id)
                    .map(|w| w.1.iter().filter(|x| x.end >= d.e.start - 90.0 && x.start <= d.e.end + 90.0).cloned().collect())
                    .unwrap_or_default();
                self.editing = Some(ClipModal::new(clip_modal::Init {
                    clip_id: cid,
                    title: c.row.title.clone(),
                    src: PathBuf::from(src),
                    edit: d.e.clone(),
                    styles: self.styles.clone(),
                    style_id: self.style_id,
                    words,
                    volume: self.volume,
                    muted: self.muted,
                    backgrounds: self.backgrounds.clone(),
                    watermarks: self.watermarks.clone(),
                }));
                // Pega fundos adicionados na aba Modelos desde a última vez.
                self.load_bgs(ui.ctx(), sh);
            }
        }
    }

    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        if let Some(ed) = self.editing.as_mut() {
            let bg_tex = ed.bg_file().and_then(|f| sh.thumbs.get(&sh.bk, ctx, Path::new(f), 0.5, (540, 960)));
            let res = ed.show(ctx, &sh.caption_fonts, bg_tex);
            // O volume escolhido no editor vale também no player da tela.
            let (vol, muted) = ed.volume();
            if (vol, muted) != (self.volume, self.muted) {
                self.volume = vol;
                self.muted = muted;
                if let Some(p) = self.player.as_mut() {
                    p.set_volume(if muted { 0.0 } else { vol });
                }
            }
            match res {
                clip_modal::Outcome::Open => {}
                clip_modal::Outcome::Close => self.editing = None,
                clip_modal::Outcome::Save(e) => {
                    self.style_id = ed.style_id();
                    if let Some(d) = self.drafts.get_mut(&ed.clip_id) {
                        d.e = e;
                    }
                    self.editing = None;
                    sh.toasts.ok("Edição do clipe salva. Vale no próximo corte");
                }
                clip_modal::Outcome::SaveAll(e) => {
                    self.style_id = ed.style_id();
                    let me = ed.clip_id;
                    let n = self.drafts.len().saturating_sub(1);
                    for (id, d) in self.drafts.iter_mut() {
                        if *id == me {
                            d.e = e.clone();
                        } else {
                            if d.e.layout != e.layout {
                                d.e.keys.clear();
                            }
                            d.e.layout = e.layout.clone();
                            d.e.camera = e.camera;
                            d.e.screen = e.screen;
                            d.e.fit = e.fit;
                            d.e.split = e.split;
                            d.e.smooth = e.smooth;
                            d.e.smooth_secs = e.smooth_secs;
                            d.e.curve = e.curve;
                            d.e.bg = e.bg.clone();
                            d.e.watermark_id = e.watermark_id;
                            d.e.caption = e.caption.clone();
                        }
                    }
                    self.editing = None;
                    sh.toasts.ok(format!("Edição salva e aplicada a mais {n} clipes. Vale no próximo corte"));
                }
            }
        }
        if let Some((pid, msg)) = self.ai_warn.clone() {
            let (open, ok) = w::modal(ctx, "clip-ai-warn", "A IA não conseguiu analisar", "", 500.0, |ui| {
                let mut c = false;
                w::pad(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&msg).color(ERR)).wrap());
                    ui.add_space(12.0);
                    ui.add(egui::Label::new(w::dim("O vídeo e a transcrição estão prontos. Marque os clipes à mão arrastando no mapa de retenção, ou clique em \"Analisar retenção\" mais tarde para a IA tentar de novo.")).wrap());
                    ui.add_space(16.0);
                    if w::button(ui, Kind::Primary, Some(ic::CHECK), "Entendi").clicked() {
                        c = true;
                    }
                });
                c
            });
            if ok == Some(true) || !open {
                self.ai_warn = None;
                self.ai_seen.insert(pid);
                if let Some(v) = self.view.as_mut().filter(|v| v.project.id == pid) {
                    v.project.error_msg = None;
                }
                if let Some(p) = self.projects.iter_mut().find(|p| p.id == pid) {
                    p.error_msg = None;
                }
                let bk = sh.bk.clone();
                drop(sh.bk.spawn(ctx, async move { bk.clips.dismiss_warning(pid).await.map_err(backend::e) }));
            }
        }
        if let Some(pid) = self.confirm_clear {
            let (open, choice) = w::modal(ctx, "clip-clear-ai", "Remover as sugestões da IA?", "", 460.0, |ui| {
                let mut c = None;
                w::pad(ui, |ui| {
                    ui.label(w::dim("Saem da lista e do mapa as sugestões que ainda não foram cortadas, com as edições delas. Clipes feitos à mão e os já cortados ficam. Depois é só arrastar no mapa para marcar os seus."));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if w::button(ui, Kind::Primary, Some(ic::TRASH_2), "Remover").clicked() {
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
                    if let Some(v) = self.view.as_ref().filter(|v| v.project.id == pid) {
                        for c in v.clips.iter().filter(|c| is_ai_pending(c)) {
                            self.drafts.remove(&c.row.id);
                            if self.focus == Some(c.row.id) {
                                self.focus = None;
                            }
                        }
                    }
                    let bk = sh.bk.clone();
                    self.t_clip = Some(sh.bk.spawn(ctx, async move { bk.clips.clear_suggestions(pid).await.map_err(backend::e) }));
                    self.confirm_clear = None;
                }
                Some(false) => self.confirm_clear = None,
                None if !open => self.confirm_clear = None,
                None => {}
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

