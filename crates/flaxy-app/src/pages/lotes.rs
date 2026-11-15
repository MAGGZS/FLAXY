//! Lotes: escolha os vídeos (direto do disco, sem cópia), ajuste o que
//! precisar e renderize todos no mesmo modelo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui};
use flaxy_core::captions::{CaptionStyle, Style};
use flaxy_core::models::{ItemOverrides, Job, JobCreate, JobItemInput, Template};
use flaxy_core::templates::{ALLOWED_AUDIO, ALLOWED_VIDEO};
use serde::{Deserialize, Serialize};

use crate::app::{Page, Shared};
use crate::backend::{self, Res, Task};
use crate::editors::caption_preview;
use crate::editors::clip_editor::{self, ClipEditor};
use crate::editors::transcript::TranscriptEditor;
use crate::format::{date_time, file_name, ratio_label, status_label, timecode};
use crate::icons as ic;
use crate::media;
use crate::theme::*;
use crate::widgets::{self as w, Kind, Tone};

#[derive(Clone, Serialize, Deserialize)]
struct Tr {
    id: i64,
    status: String,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Item {
    path: String,
    name: String,
    duration: f64,
    overrides: ItemOverrides,
    tr: Option<Tr>,
}

#[derive(Clone, Serialize, Deserialize)]
struct AudioTrack {
    path: String,
    name: String,
    duration: f64,
    volume: f64,
    original: f64,
    offset: f64,
    fade: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Captions {
    on: bool,
    style_id: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Draft {
    items: Vec<Item>,
    template_id: Option<i64>,
    audio: Option<AudioTrack>,
    captions: Captions,
}

#[derive(Default)]
pub struct Lotes {
    loaded: bool,
    d: Draft,
    dirty: bool,
    templates: Vec<Template>,
    styles: Vec<Style>,
    jobs: Vec<Job>,
    open_jobs: HashSet<i64>,
    rev: u64,
    t_meta: Option<Task<Res<(Vec<Template>, Vec<Style>)>>>,
    t_jobs: Option<Task<Res<Vec<Job>>>>,
    t_probe: Vec<Task<Vec<(String, f64)>>>,
    t_audio: Option<Task<Res<AudioTrack>>>,
    t_tr: Option<Task<Vec<(i64, String, Option<String>)>>>,
    t_tr_create: Option<Task<Vec<(String, Result<i64, String>)>>>,
    t_submit: Option<Task<Res<usize>>>,
    t_action: Option<Task<Res<()>>>,
    last_tr_poll: f64,
    editing: Option<(usize, ClipEditor)>,
    tr_edit: Option<TranscriptEditor>,
}

fn has_ext(p: &Path, allowed: &[&str]) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| allowed.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

impl Lotes {
    fn draft_path(sh: &Shared) -> PathBuf {
        sh.bk.cfg.storage_dir.join("rascunho_lote.json")
    }

    fn init(&mut self, ui: &Ui, sh: &mut Shared) {
        self.loaded = true;
        if let Ok(raw) = std::fs::read(Self::draft_path(sh)) {
            if let Ok(d) = serde_json::from_slice::<Draft>(&raw) {
                self.d = d;
                // Vídeos apagados ou movidos saem do rascunho.
                self.d.items.retain(|i| Path::new(&i.path).is_file());
                if self.d.audio.as_ref().is_some_and(|a| !Path::new(&a.path).is_file()) {
                    self.d.audio = None;
                }
            }
        }
        self.reload_meta(ui, sh);
    }

    fn reload_meta(&mut self, ui: &Ui, sh: &Shared) {
        let bk = sh.bk.clone();
        self.t_meta = Some(sh.bk.spawn(ui.ctx(), async move {
            let t = flaxy_core::templates::list(&bk.db).await.map_err(backend::e)?;
            let s = flaxy_core::captions::list(&bk.db).await.map_err(backend::e)?;
            Ok((t, s))
        }));
    }

    fn save_draft(&mut self, sh: &Shared) {
        if self.dirty {
            self.dirty = false;
            if let Ok(s) = serde_json::to_vec(&self.d) {
                let _ = std::fs::write(Self::draft_path(sh), s);
            }
        }
    }

    fn add_paths(&mut self, ui: &Ui, sh: &mut Shared, paths: Vec<PathBuf>) {
        let have: HashSet<String> = self.d.items.iter().map(|i| i.path.clone()).collect();
        let max = sh.bk.cfg.max_videos_per_job;
        let mut new = Vec::new();
        for p in paths.into_iter().filter(|p| p.is_file() && has_ext(p, ALLOWED_VIDEO)) {
            let s = p.to_string_lossy().into_owned();
            if have.contains(&s) || self.d.items.len() + new.len() >= max {
                continue;
            }
            new.push(s);
        }
        if self.d.items.len() + new.len() >= max {
            sh.toasts.err(format!("Máximo de {max} vídeos por lote"));
        }
        if new.is_empty() {
            return;
        }
        for p in &new {
            self.d.items.push(Item { path: p.clone(), name: file_name(p), duration: 0.0, overrides: ItemOverrides::default(), tr: None });
        }
        self.dirty = true;
        self.t_probe.push(sh.bk.spawn(ui.ctx(), async move {
            let mut out = Vec::new();
            for p in new {
                let pr = flaxy_core::media::probe(Path::new(&p)).await;
                out.push((p, pr.duration));
            }
            out
        }));
    }

    fn poll(&mut self, ui: &Ui, sh: &mut Shared) {
        if let Some(r) = self.t_meta.as_ref().and_then(|t| t.take()) {
            self.t_meta = None;
            match r {
                Ok((t, s)) => {
                    if !self.d.template_id.is_some_and(|id| t.iter().any(|x| x.id == id)) {
                        self.d.template_id = t.first().map(|x| x.id);
                    }
                    if !self.d.captions.style_id.is_some_and(|id| s.iter().any(|x| x.row.id == id)) {
                        self.d.captions.style_id = s.first().map(|x| x.row.id);
                    }
                    self.templates = t;
                    self.styles = s;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        self.t_probe.retain(|t| match t.take() {
            Some(res) => {
                for (p, d) in res {
                    if let Some(it) = self.d.items.iter_mut().find(|i| i.path == p) {
                        it.duration = d;
                    }
                }
                self.dirty = true;
                false
            }
            None => true,
        });
        if let Some(r) = self.t_jobs.as_ref().and_then(|t| t.take()) {
            self.t_jobs = None;
            match r {
                Ok(j) => {
                    for job in &j {
                        if matches!(job.job.status.as_str(), "processing" | "queued") {
                            self.open_jobs.insert(job.job.id);
                        }
                    }
                    self.jobs = j;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if self.t_jobs.is_none() && self.rev != sh.bk.rev() {
            self.rev = sh.bk.rev();
            let bk = sh.bk.clone();
            self.t_jobs = Some(sh.bk.spawn(ui.ctx(), async move { bk.jobs.list(20).await.map_err(backend::e) }));
        }
        if let Some(r) = self.t_audio.as_ref().and_then(|t| t.take()) {
            self.t_audio = None;
            match r {
                Ok(a) => {
                    self.d.audio = Some(a);
                    self.dirty = true;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_submit.as_ref().and_then(|t| t.take()) {
            self.t_submit = None;
            match r {
                Ok(n) => {
                    sh.toasts.ok(format!("Lote com {n} vídeos na fila"));
                    self.d.items.clear();
                    self.dirty = true;
                    sh.bk.bump();
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_action.as_ref().and_then(|t| t.take()) {
            self.t_action = None;
            if let Err(e) = r {
                sh.toasts.err(e);
            }
            sh.bk.bump();
        }
        if let Some(res) = self.t_tr_create.as_ref().and_then(|t| t.take()) {
            self.t_tr_create = None;
            for (path, r) in res {
                match r {
                    Ok(id) => {
                        if let Some(it) = self.d.items.iter_mut().find(|i| i.path == path) {
                            it.tr = Some(Tr { id, status: "queued".into(), error: None });
                        }
                    }
                    Err(e) => {
                        sh.toasts.err(e);
                        break;
                    }
                }
            }
            self.dirty = true;
        }
        // Acompanha transcrições em andamento a cada 2 s.
        let pending: Vec<i64> = self.d.items.iter().filter_map(|i| i.tr.as_ref()).filter(|t| t.status == "queued" || t.status == "processing").map(|t| t.id).collect();
        if let Some(res) = self.t_tr.as_ref().and_then(|t| t.take()) {
            self.t_tr = None;
            for (id, status, error) in res {
                for it in self.d.items.iter_mut() {
                    if let Some(tr) = it.tr.as_mut().filter(|t| t.id == id) {
                        tr.status = status.clone();
                        tr.error = error.clone();
                    }
                }
            }
            self.dirty = true;
        }
        let now = ui.input(|i| i.time);
        if !pending.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            if self.t_tr.is_none() && now - self.last_tr_poll > 2.0 {
                self.last_tr_poll = now;
                let bk = sh.bk.clone();
                self.t_tr = Some(sh.bk.spawn(ui.ctx(), async move {
                    let mut out = Vec::new();
                    for id in pending {
                        if let Ok(t) = bk.transcripts.get(id).await {
                            out.push((id, t.row.status, t.row.error_msg));
                        }
                    }
                    out
                }));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if !self.loaded {
            self.init(ui, sh);
        }
        self.poll(ui, sh);

        // Arrastar e soltar vídeos em qualquer lugar da janela.
        let (dropped, hovering) = ui.input(|i| (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(), !i.raw.hovered_files.is_empty()));
        if !dropped.is_empty() {
            self.add_paths(ui, sh, dropped);
        }

        w::page_head(ui, "Lotes", "Escolha os vídeos, ajuste o que precisar e renderize todos no mesmo modelo", |_| {});

        let gap = 12.0;
        let side_w = 340.0;
        let main_w = ui.available_width() - side_w - gap;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            w::column(ui, main_w, |ui| {
                self.videos_card(ui, sh, hovering);
            });
            w::column(ui, side_w, |ui| {
                ui.spacing_mut().item_spacing.y = gap;
                self.template_card(ui, sh);
                self.audio_card(ui, sh);
                self.caption_card(ui, sh);
            });
        });
        ui.add_space(gap);
        self.jobs_card(ui, sh);
        self.save_draft(sh);
    }

    fn videos_card(&mut self, ui: &mut Ui, sh: &mut Shared, hovering: bool) {
        let n = self.d.items.len();
        let total: f64 = self.d.items.iter().map(|i| (i.overrides.trim_end.unwrap_or(i.duration) - i.overrides.trim_start.unwrap_or(0.0)).max(0.0)).sum();
        let adjusted = self.d.items.iter().filter(|i| clip_editor::is_adjusted(&i.overrides)).count();
        let meta = format!("{n} vídeos · {}{}", timecode(total, false), if adjusted > 0 { format!(" · {adjusted} ajustados") } else { String::new() });
        let mut clear = false;
        w::card(
            ui,
            "Vídeos do lote",
            &meta,
            |ui| {
                if n > 0 && w::button(ui, Kind::Ghost, None, "Limpar").clicked() {
                    clear = true;
                }
            },
            |ui| {
                w::pad(ui, |ui| {
                    // Área de soltar.
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 84.0), Sense::click());
                    let p = ui.painter();
                    let over = hovering || resp.hovered();
                    p.rect(rect, CornerRadius::same(R), if over { SELECT } else { BG_DEEP }, Stroke::new(1.0, if over { ACCENT } else { LINE_2 }), StrokeKind::Inside);
                    p.text(rect.left_center() + egui::vec2(24.0, 0.0), Align2::LEFT_CENTER, ic::UPLOAD, body(ICON), TEXT_3);
                    p.text(rect.left_center() + egui::vec2(60.0, -9.0), Align2::LEFT_CENTER, if hovering { "Solte para adicionar" } else { "Arraste vídeos aqui ou clique para escolher" }, body(T_BODY), TEXT);
                    p.text(rect.left_center() + egui::vec2(60.0, 11.0), Align2::LEFT_CENTER, "mp4, mov, webm, mkv, avi · usados direto da pasta, sem cópia", body(T_META), TEXT_3);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        if let Some(files) = rfd::FileDialog::new().set_title("Escolher vídeos").add_filter("Vídeos", ALLOWED_VIDEO).pick_files() {
                            self.add_paths(ui, sh, files);
                        }
                    }
                    if self.d.items.is_empty() {
                        return;
                    }
                    ui.add_space(16.0);
                    self.grid(ui, sh);
                });
            },
        );
        if clear {
            self.d.items.clear();
            self.dirty = true;
        }
    }

    fn grid(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let gap = 12.0;
        let avail = ui.available_width();
        let cols = ((avail + gap) / (210.0 + gap)).floor().max(1.0) as usize;
        let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = cw * 9.0 / 16.0;
        let mut remove = None;
        let mut edit = None;
        let mut tr_edit = None;
        for (row_i, row) in self.d.items.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (ci, it) in row.iter().enumerate() {
                    let idx = row_i * cols + ci;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, th + 3.0 + 40.0), Sense::hover());
                    let adj = clip_editor::is_adjusted(&it.overrides);
                    let p = ui.painter();
                    p.rect(rect, CornerRadius::same(R), RAISED, Stroke::new(1.0, if adj { ACCENT } else { LINE }), StrokeKind::Inside);
                    let thumb = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(cw - 2.0, th));
                    p.rect_filled(thumb, CornerRadius { nw: R, ne: R, sw: 0, se: 0 }, Color32::BLACK);
                    let a = it.overrides.trim_start.unwrap_or(0.0);
                    let b = it.overrides.trim_end.unwrap_or(it.duration);
                    if let Some(t) = sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(&it.path), a + 0.1, (360, 204)) {
                        media::paint_image(ui, &t, thumb, true);
                    }
                    let tc = timecode((b - a).max(0.0), false);
                    let g = ui.painter().layout_no_wrap(tc, mono(T_META), TEXT);
                    let tr = egui::Rect::from_min_size(thumb.right_bottom() - g.size() - egui::vec2(14.0, 10.0), g.size() + egui::vec2(10.0, 4.0));
                    ui.painter().rect_filled(tr, CornerRadius::same(4), Color32::from_black_alpha(200));
                    ui.painter().galley(tr.min + egui::vec2(5.0, 2.0), g, TEXT);
                    // Trecho mantido.
                    let bar = egui::Rect::from_min_size(egui::pos2(thumb.left(), thumb.bottom()), egui::vec2(thumb.width(), 3.0));
                    ui.painter().rect_filled(bar, CornerRadius::ZERO, BG_DEEP);
                    if it.duration > 0.0 {
                        let x0 = bar.left() + bar.width() * (a / it.duration) as f32;
                        let x1 = bar.left() + bar.width() * (b / it.duration).min(1.0) as f32;
                        ui.painter().rect_filled(egui::Rect::from_x_y_ranges(x0..=x1, bar.y_range()), CornerRadius::ZERO, ACCENT);
                    }
                    let foot = egui::Rect::from_min_max(egui::pos2(rect.left() + 10.0, bar.bottom()), rect.max - egui::vec2(4.0, 0.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let right = 28.0 * 2.0 + if it.tr.is_some() { 30.0 } else { 0.0 } + 8.0;
                        ui.allocate_ui(egui::vec2((ui.available_width() - right).max(20.0), 28.0), |ui| {
                            w::truncated(ui, RichText::new(&it.name).font(body(T_SEC)).color(TEXT)).on_hover_text(&it.path);
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_danger(ui, ic::TRASH_2, "Remover do lote").clicked() {
                                remove = Some(idx);
                            }
                            if w::icon_button(ui, ic::CROP, "Recortar e cortar no tempo").clicked() {
                                edit = Some(idx);
                            }
                            if let Some(tr) = &it.tr {
                                match tr.status.as_str() {
                                    "done" => {
                                        if w::icon_button(ui, ic::CAPTIONS, "Editar legenda").clicked() {
                                            tr_edit = Some((tr.id, it.name.clone()));
                                        }
                                    }
                                    s => {
                                        // Estado da transcrição como ponto, com o detalhe na dica.
                                        let (c, tip) = match s {
                                            "error" => (ERR, tr.error.clone().unwrap_or_else(|| "Transcrição falhou".into())),
                                            "queued" => (TEXT_3, "Transcrição na fila".to_string()),
                                            _ => (ACCENT, "Transcrevendo".to_string()),
                                        };
                                        let (r, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
                                        ui.painter().circle_filled(r.center(), 4.0, c);
                                        resp.on_hover_text(tip);
                                    }
                                }
                            }
                        });
                    });
                }
            });
            ui.add_space(gap - 8.0);
        }
        if let Some(i) = remove {
            self.d.items.remove(i);
            self.dirty = true;
        }
        if let Some(i) = edit {
            let it = &self.d.items[i];
            let tpl = self.templates.iter().find(|t| Some(t.id) == self.d.template_id);
            self.editing = Some((i, ClipEditor::new(Path::new(&it.path), &it.name, it.duration, &it.overrides, tpl)));
        }
        if let Some((id, name)) = tr_edit {
            self.tr_edit = Some(TranscriptEditor::new(id, &name));
        }
    }

    fn template_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        w::card(ui, "Modelo", "", |_| {}, |ui| {
            w::pad(ui, |ui| {
                if self.templates.is_empty() {
                    ui.label(w::dim("Nenhum modelo de fundo ainda"));
                    if w::button(ui, Kind::Default, Some(ic::PLUS), "Criar modelo").clicked() {
                        sh.goto = Some(Page::Modelos);
                    }
                } else {
                    w::label(ui, "Fundo");
                    let cur = self.templates.iter().find(|t| Some(t.id) == self.d.template_id).map(|t| t.name.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("tpl").width(ui.available_width()).selected_text(cur).show_ui(ui, |ui| {
                        for t in &self.templates {
                            if ui.selectable_label(Some(t.id) == self.d.template_id, &t.name).clicked() {
                                self.d.template_id = Some(t.id);
                                self.dirty = true;
                            }
                        }
                    });
                    if let Some(t) = self.templates.iter().find(|t| Some(t.id) == self.d.template_id) {
                        ui.add_space(8.0);
                        // Prévia do resultado: fundo + primeiro vídeo na área + legenda.
                        let sample = self.d.items.first().map(|i| (i.path.as_str(), &i.overrides));
                        let caption = if self.d.captions.on {
                            self.styles.iter().find(|s| Some(s.row.id) == self.d.captions.style_id).map(|s| &s.style)
                        } else {
                            None
                        };
                        template_preview(ui, sh, t, 360.0, sample, caption);
                        ui.vertical_centered(|ui| {
                            ui.label(w::mono_text(
                                format!("{}×{} · {} · área {}×{}", t.output_w, t.output_h, ratio_label(t.output_w, t.output_h), t.overlay_w, t.overlay_h),
                                12.0,
                                TEXT_3,
                            ));
                        });
                    }
                }
                ui.add_space(8.0);
                let n = self.d.items.len();
                let can = self.d.template_id.is_some() && n > 0 && self.t_submit.is_none() && self.t_probe.is_empty();
                ui.add_enabled_ui(can, |ui| {
                    ui.vertical_centered_justified(|ui| {
                        let label = match n {
                            0 => "Renderizar".to_string(),
                            1 => "Renderizar 1 vídeo".to_string(),
                            n => format!("Renderizar {n} vídeos"),
                        };
                        if w::button_sized(ui, Kind::Primary, Some(ic::PLAY), &label, 38.0).clicked() {
                            self.submit(ui, sh);
                        }
                    });
                });
            });
        });
    }

    fn submit(&mut self, ui: &Ui, sh: &mut Shared) {
        let Some(tid) = self.d.template_id else { return };
        let cap = &self.d.captions;
        if cap.on && self.d.items.iter().any(|i| i.tr.as_ref().map(|t| t.status.as_str()) != Some("done")) {
            sh.toasts.err("Transcreva todos os vídeos antes de renderizar com legenda");
            return;
        }
        let items: Vec<JobItemInput> = self
            .d
            .items
            .iter()
            .map(|i| {
                let mut o = i.overrides.clone();
                if let Some(a) = &self.d.audio {
                    o.audio_path = Some(a.path.clone());
                    o.audio_volume = Some(a.volume);
                    o.original_volume = Some(a.original);
                    o.audio_offset = (a.offset > 0.0).then_some(a.offset);
                    o.audio_fade = Some(a.fade);
                }
                if let (true, Some(sid), Some(tr)) = (cap.on, cap.style_id, &i.tr) {
                    o.caption_style_id = Some(sid);
                    o.transcript_id = Some(tr.id);
                }
                JobItemInput { path: i.path.clone(), overrides: o }
            })
            .collect();
        let bk = sh.bk.clone();
        self.t_submit = Some(sh.bk.spawn(ui.ctx(), async move {
            let n = items.len();
            bk.jobs.create(JobCreate { template_id: tid, items }).await.map_err(backend::e)?;
            Ok(n)
        }));
    }

    fn audio_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let meta = self.d.audio.as_ref().map(|a| timecode(a.duration, false)).unwrap_or_default();
        let mut remove = false;
        w::card(ui, "Faixa de áudio", &meta, |_| {}, |ui| {
            w::pad(ui, |ui| match self.d.audio.as_mut() {
                None => {
                    ui.label(w::dim("Música ou narração mixada em todos os vídeos. Repete se for mais curta."));
                    ui.add_enabled_ui(self.t_audio.is_none(), |ui| {
                        if w::button(ui, Kind::Default, Some(ic::MUSIC), if self.t_audio.is_some() { "Lendo áudio" } else { "Escolher áudio" }).clicked() {
                            if let Some(f) = rfd::FileDialog::new().set_title("Escolher faixa de áudio").add_filter("Áudio", ALLOWED_AUDIO).pick_file() {
                                self.t_audio = Some(sh.bk.spawn(ui.ctx(), async move {
                                    let pr = flaxy_core::media::probe(&f).await;
                                    if !pr.has_audio {
                                        return Err("O arquivo não tem faixa de áudio".to_string());
                                    }
                                    let path = f.to_string_lossy().into_owned();
                                    Ok(AudioTrack { name: file_name(&path), path, duration: pr.duration, volume: 0.6, original: 1.0, offset: 0.0, fade: true })
                                }));
                            }
                        }
                    });
                }
                Some(a) => {
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ic::MUSIC).size(ICON).color(TEXT_3));
                        ui.allocate_ui(egui::vec2(ui.available_width() - 70.0, 24.0), |ui| {
                            w::truncated(ui, RichText::new(&a.name).color(TEXT)).on_hover_text(&a.path);
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_danger(ui, ic::TRASH_2, "Remover faixa").clicked() {
                                remove = true;
                            }
                            if w::icon_button(ui, ic::PLAY, "Ouvir no player do Windows").clicked() {
                                backend::open_path(Path::new(&a.path));
                            }
                        });
                    });
                    w::label_value(ui, "Volume da faixa", &format!("{}%", (a.volume * 100.0).round()));
                    changed |= ui.add(egui::Slider::new(&mut a.volume, 0.0..=2.0).step_by(0.05).show_value(false)).changed();
                    w::label_value(ui, "Áudio original", &if a.original == 0.0 { "mudo".to_string() } else { format!("{}%", (a.original * 100.0).round()) });
                    changed |= ui.add(egui::Slider::new(&mut a.original, 0.0..=2.0).step_by(0.05).show_value(false)).changed();
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            w::label(ui, "Começar a faixa em (s)");
                            changed |= ui.add(egui::DragValue::new(&mut a.offset).range(0.0..=a.duration.max(0.0)).speed(0.1).max_decimals(1)).changed();
                        });
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            w::label(ui, "Final");
                            changed |= w::seg(ui, &mut a.fade, &[(true, "Fade"), (false, "Corte seco")]);
                        });
                    });
                    self.dirty |= changed;
                }
            });
        });
        if remove {
            self.d.audio = None;
            self.dirty = true;
        }
    }

    fn caption_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let toggle = std::cell::Cell::new(None);
        let cur_on = self.d.captions.on;
        w::card(
            ui,
            "Legenda automática",
            "",
            |ui| {
                let mut on = cur_on;
                if w::seg(ui, &mut on, &[(false, "Não"), (true, "Sim")]) {
                    toggle.set(Some(on));
                }
            },
            |ui| {
                if !self.d.captions.on {
                    return;
                }
                w::pad(ui, |ui| {
                    w::label(ui, "Estilo");
                    let cur = self.styles.iter().find(|s| Some(s.row.id) == self.d.captions.style_id);
                    egui::ComboBox::from_id_salt("cap-style").width(ui.available_width()).selected_text(cur.map(|s| s.row.name.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                        for s in &self.styles {
                            if ui.selectable_label(Some(s.row.id) == self.d.captions.style_id, &s.row.name).clicked() {
                                self.d.captions.style_id = Some(s.row.id);
                                self.dirty = true;
                            }
                        }
                    });
                    if let Some(s) = self.styles.iter().find(|s| Some(s.row.id) == self.d.captions.style_id) {
                        ui.add_space(8.0);
                        caption_preview::preview(ui, &s.style, 240.0, &sh.caption_fonts);
                    }
                    let total = self.d.items.len();
                    let done = self.d.items.iter().filter(|i| i.tr.as_ref().is_some_and(|t| t.status == "done")).count();
                    let running = self.d.items.iter().filter(|i| i.tr.as_ref().is_some_and(|t| t.status == "queued" || t.status == "processing")).count();
                    let missing = total - done - running;
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(w::dim("Transcritos"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(w::mono_text(format!("{done}/{total}{}", if running > 0 { format!(" · {running} em andamento") } else { String::new() }), T_META, TEXT_3));
                        });
                    });
                    w::progress(ui, if total > 0 { done as f32 / total as f32 } else { 0.0 }, Tone::Accent, None);
                    ui.add_enabled_ui(missing > 0 && self.t_tr_create.is_none(), |ui| {
                        let label = if missing > 0 { format!("Transcrever {missing} {}", if missing == 1 { "vídeo" } else { "vídeos" }) } else { "Tudo transcrito".into() };
                        if w::button(ui, Kind::Default, Some(ic::CAPTIONS), &label).clicked() {
                            let paths: Vec<String> = self.d.items.iter().filter(|i| i.tr.as_ref().is_none_or(|t| t.status == "error")).map(|i| i.path.clone()).collect();
                            let bk = sh.bk.clone();
                            self.t_tr_create = Some(sh.bk.spawn(ui.ctx(), async move {
                                let mut out = Vec::new();
                                for p in paths {
                                    let r = bk.transcripts.create(&p).await.map(|t| t.row.id).map_err(backend::e);
                                    let stop = r.is_err();
                                    out.push((p, r));
                                    if stop {
                                        break;
                                    }
                                }
                                out
                            }));
                        }
                    });
                    if ui.link(RichText::new("Provedor e idioma na aba IA").font(body(T_META))).clicked() {
                        sh.goto = Some(Page::Ia);
                    }
                });
            },
        );
        if let Some(on) = toggle.get() {
            self.d.captions.on = on;
            self.dirty = true;
        }
    }

    fn jobs_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let live = sh.bk.live.lock().unwrap().items.clone();
        let meta = format!("{} lotes", self.jobs.len());
        let mut act: Option<(i64, Option<i64>)> = None;
        w::card(ui, "Fila e histórico", &meta, |ui| {
            if w::button(ui, Kind::Ghost, Some(ic::FOLDER_OPEN), "Pasta dos vídeos").clicked() {
                backend::open_path(&sh.bk.cfg.output_dir());
            }
        }, |ui| {
            if self.jobs.is_empty() {
                w::empty(ui, ic::FILM, "Nenhum lote enviado");
                return;
            }
            for (ji, job) in self.jobs.iter().enumerate() {
                if ji > 0 {
                    w::line(ui);
                }
                let (frac, done) = super::inicio::job_progress(job, &live);
                let active = matches!(job.job.status.as_str(), "processing" | "queued");
                let open = self.open_jobs.contains(&job.job.id);
                let resp = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_width(ui.available_width());
                        w::fixed(ui, 72.0, w::mono_text(format!("#{:04}", job.job.id), T_SEC, TEXT_3));
                        w::fixed(ui, 180.0, RichText::new(format!("{} vídeos · {}", job.items.len(), status_label(&job.job.status))).color(TEXT));
                        let tone = match job.job.status.as_str() {
                            "done" => Tone::Done,
                            "error" => Tone::Err,
                            _ => Tone::Accent,
                        };
                        let rest = ui.available_width();
                        w::progress(ui, frac, tone, Some((rest - 300.0).max(80.0)));
                        w::fixed(ui, 60.0, w::mono_text(format!("{done}/{}", job.items.len()), T_SEC, TEXT_2));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if active && w::icon_button(ui, ic::BAN, "Cancelar o que ainda está na fila").clicked() {
                                act = Some((job.job.id, None));
                            }
                            ui.label(w::mono_text(date_time(&job.job.created_at), T_META, TEXT_3));
                        });
                    });
                });
                let click = ui.interact(resp.response.rect, egui::Id::unique(("job-row", job.job.id)), Sense::click());
                if click.clicked() {
                    if open {
                        self.open_jobs.remove(&job.job.id);
                    } else {
                        self.open_jobs.insert(job.job.id);
                    }
                }
                if !open {
                    continue;
                }
                for it in &job.items {
                    let (st, prog) = live.get(&it.id).cloned().unwrap_or((it.status.clone(), it.progress));
                    egui::Frame::NONE.fill(BG_DEEP).inner_margin(egui::Margin { left: 112, right: 20, top: 8, bottom: 8 }).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_width(ui.available_width());
                            let name_w = (ui.available_width() - 480.0).max(120.0);
                            ui.allocate_ui(egui::vec2(name_w, 24.0), |ui| {
                                let mut t = RichText::new(file_name(&it.input_path)).font(body(T_SEC)).color(TEXT_2);
                                if it.error_msg.is_some() && st == "error" {
                                    t = t.color(ERR);
                                }
                                let r = w::truncated(ui, t);
                                if let Some(e) = &it.error_msg {
                                    r.on_hover_text(e);
                                }
                            });
                            let tone = match st.as_str() {
                                "done" => Tone::Done,
                                "error" => Tone::Err,
                                _ => Tone::Accent,
                            };
                            w::progress(ui, if st == "done" { 1.0 } else { prog as f32 / 100.0 }, tone, Some(200.0));
                            w::fixed(ui, 48.0, w::mono_text(if st == "processing" { format!("{prog}%") } else { String::new() }, T_META, TEXT_3));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if matches!(st.as_str(), "error" | "canceled") && w::icon_button(ui, ic::REFRESH_CW, "Tentar de novo").clicked() {
                                    act = Some((job.job.id, Some(it.id)));
                                }
                                if st == "done" {
                                    if let Some(out) = &it.output_path {
                                        if w::icon_button(ui, ic::FOLDER, "Mostrar na pasta").clicked() {
                                            backend::reveal(Path::new(out));
                                        }
                                        if w::icon_button(ui, ic::PLAY, "Abrir vídeo").clicked() {
                                            backend::open_path(Path::new(out));
                                        }
                                    }
                                } else {
                                    ui.label(w::muted(status_label(&st)));
                                }
                                if st == "error" {
                                    if let Some(log) = &it.log_path {
                                        if w::icon_button(ui, ic::CIRCLE_ALERT, "Abrir log do FFmpeg").clicked() {
                                            backend::open_path(Path::new(log));
                                        }
                                    }
                                }
                            });
                        });
                    });
                }
            }
        });
        if let Some((job, item)) = act {
            let bk = sh.bk.clone();
            self.t_action = Some(sh.bk.spawn(ui.ctx(), async move {
                match item {
                    None => bk.jobs.cancel(job).await.map(|_| ()).map_err(backend::e),
                    Some(i) => bk.jobs.retry(job, i).await.map_err(backend::e),
                }
            }));
        }
    }

    /// Editores em janela própria (fora da rolagem da página).
    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        if let Some((idx, ed)) = self.editing.as_mut() {
            match ed.show(ctx, sh) {
                clip_editor::Outcome::Open => {}
                clip_editor::Outcome::Close => self.editing = None,
                clip_editor::Outcome::Save(o) => {
                    if let Some(it) = self.d.items.get_mut(*idx) {
                        it.overrides = o;
                    }
                    self.dirty = true;
                    self.editing = None;
                }
            }
        }
        if let Some(ed) = self.tr_edit.as_mut() {
            if !ed.show(ctx, sh) {
                self.tr_edit = None;
            }
        }
        self.save_draft(sh);
    }
}

/// Prévia 9:16 do resultado (momento da direção): o fundo do modelo, o vídeo
/// recortado e encaixado na área, e a legenda escolhida por cima. Sem vídeo,
/// a área aparece marcada.
pub fn template_preview(ui: &mut Ui, sh: &mut Shared, t: &Template, max_h: f32, sample: Option<(&str, &ItemOverrides)>, caption: Option<&CaptionStyle>) {
    let aspect = t.output_w as f32 / t.output_h.max(1) as f32;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), max_h), Sense::hover());
    let r = media::fit_rect(outer, aspect);
    ui.painter().rect_filled(r, CornerRadius::same(R), Color32::BLACK);
    if let Some(tex) = sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(&t.file_path), 0.1, (540, 960)) {
        media::paint_image(ui, &tex, r, true);
    }
    let ov = egui::Rect::from_min_size(
        egui::pos2(r.left() + r.width() * t.overlay_x as f32 / t.output_w as f32, r.top() + r.height() * t.overlay_y as f32 / t.output_h as f32),
        egui::vec2(r.width() * t.overlay_w as f32 / t.output_w as f32, r.height() * t.overlay_h as f32 / t.output_h as f32),
    );
    let video = sample.and_then(|(path, o)| {
        let at = o.trim_start.unwrap_or(0.0) + 0.5;
        sh.thumbs.get(&sh.bk, ui.ctx(), Path::new(path), at, (640, 640)).map(|tex| (tex, o))
    });
    let p = ui.painter_at(r);
    match video {
        Some((tex, o)) => {
            let [tw, th] = tex.size();
            let (cw, ch) = (tw as f32 * o.crop_w as f32, th as f32 * o.crop_h as f32);
            let cover = o.fit_mode.as_deref().unwrap_or(&t.fit_mode) != "contain";
            let s = if cover { (ov.width() / cw).max(ov.height() / ch) } else { (ov.width() / cw).min(ov.height() / ch) };
            let dst = egui::Rect::from_center_size(ov.center(), egui::vec2(cw * s, ch * s));
            let uv = egui::Rect::from_min_size(egui::pos2(o.crop_x as f32, o.crop_y as f32), egui::vec2(o.crop_w as f32, o.crop_h as f32));
            if !cover {
                ui.painter_at(ov).rect_filled(ov, CornerRadius::ZERO, Color32::BLACK);
            }
            ui.painter_at(ov.intersect(r)).image(tex.id(), dst, uv, Color32::WHITE);
        }
        None => {
            p.rect(ov, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(59, 99, 214, 46), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
    }
    if let Some(st) = caption {
        let now = ui.input(|i| i.time);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        caption_preview::paint(&p, r, st, &sh.caption_fonts, now);
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<i64, i64>) {}
