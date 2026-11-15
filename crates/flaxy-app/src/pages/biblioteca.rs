//! Biblioteca: vídeos prontos na pasta de saída. Abrir, mostrar na pasta,
//! exportar (copiar ou ZIP) e apagar.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui};

use crate::app::Shared;
use crate::backend::{self, OutputFile, Res, Task};
use crate::format::{bytes, date_time};
use crate::icons as ic;
use crate::media;
use crate::theme::*;
use crate::widgets::{self as w, Kind};

#[derive(Default)]
pub struct Biblioteca {
    files: Vec<OutputFile>,
    sel: HashSet<PathBuf>,
    rev: u64,
    loaded: bool,
    t_list: Option<Task<Res<Vec<OutputFile>>>>,
    t_op: Option<Task<Res<String>>>,
    confirm_delete: Option<Vec<PathBuf>>,
}

impl Biblioteca {
    fn load(&mut self, ui: &Ui, sh: &Shared) {
        self.rev = sh.bk.rev();
        let bk = sh.bk.clone();
        self.t_list = Some(sh.bk.spawn(ui.ctx(), backend::list_output(bk)));
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.t_list.is_none() && (!self.loaded || self.rev != sh.bk.rev()) {
            self.loaded = true;
            self.load(ui, sh);
        }
        if let Some(r) = self.t_list.as_ref().and_then(|t| t.take()) {
            self.t_list = None;
            match r {
                Ok(f) => {
                    let alive: HashSet<PathBuf> = f.iter().map(|x| x.path.clone()).collect();
                    self.sel.retain(|p| alive.contains(p));
                    self.files = f;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_op.as_ref().and_then(|t| t.take()) {
            self.t_op = None;
            match r {
                Ok(m) => sh.toasts.ok(m),
                Err(e) => sh.toasts.err(e),
            }
            self.load(ui, sh);
        }

        let total: u64 = self.files.iter().map(|f| f.size).sum();
        let sub = format!("{} vídeos prontos · {}", self.files.len(), bytes(total));
        let mut act: Option<&str> = None;
        w::page_head(ui, "Biblioteca", &sub, |ui| {
            if !self.sel.is_empty() {
                ui.add_enabled_ui(self.t_op.is_none(), |ui| {
                    if w::button(ui, Kind::Primary, Some(ic::ARCHIVE), &format!("ZIP com {}", self.sel.len())).clicked() {
                        act = Some("zip");
                    }
                    if w::button(ui, Kind::Default, Some(ic::DOWNLOAD), &format!("Copiar {} para…", self.sel.len())).clicked() {
                        act = Some("copy");
                    }
                    if w::button(ui, Kind::Danger, Some(ic::TRASH_2), &format!("Apagar {}", self.sel.len())).clicked() {
                        act = Some("delete");
                    }
                    if w::button(ui, Kind::Ghost, Some(ic::X), "Limpar seleção").clicked() {
                        act = Some("clear");
                    }
                });
            } else {
                if w::button(ui, Kind::Default, Some(ic::FOLDER_OPEN), "Abrir pasta").clicked() {
                    backend::open_path(&sh.bk.cfg.output_dir());
                }
                if !self.files.is_empty() && w::button(ui, Kind::Default, None, "Selecionar todos").clicked() {
                    act = Some("all");
                }
            }
        });
        match act {
            Some("all") => self.sel = self.files.iter().map(|f| f.path.clone()).collect(),
            Some("clear") => self.sel.clear(),
            Some("delete") => self.confirm_delete = Some(self.sel.iter().cloned().collect()),
            Some("zip") => {
                let stamp = chrono::Local::now().format("%Y%m%d_%H%M");
                if let Some(dest) = rfd::FileDialog::new().set_title("Salvar ZIP").set_file_name(format!("flaxy_{stamp}.zip")).add_filter("ZIP", &["zip"]).save_file() {
                    let files: Vec<PathBuf> = self.sel.iter().cloned().collect();
                    self.t_op = Some(sh.bk.spawn(ui.ctx(), async move {
                        let n = backend::write_zip(files, dest.clone()).await?;
                        backend::reveal(&dest);
                        Ok(format!("{n} vídeos no ZIP"))
                    }));
                }
            }
            Some("copy") => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Copiar vídeos para").pick_folder() {
                    let files: Vec<PathBuf> = self.sel.iter().cloned().collect();
                    self.t_op = Some(sh.bk.spawn(ui.ctx(), async move {
                        let mut n = 0;
                        for f in files {
                            if let Some(name) = f.file_name() {
                                tokio::fs::copy(&f, dir.join(name)).await.map_err(backend::e)?;
                                n += 1;
                            }
                        }
                        backend::open_path(&dir);
                        Ok(format!("{n} vídeos copiados"))
                    }));
                }
            }
            _ => {}
        }

        if self.files.is_empty() {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::FOLDER_OPEN, "Nada renderizado ainda");
            });
            return;
        }

        let gap = 12.0;
        let avail = ui.available_width();
        let cols = ((avail + gap) / (190.0 + gap)).floor().max(1.0) as usize;
        let cw = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        let th = cw * 16.0 / 9.0;
        let mut toggle = None;
        for row in self.files.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for f in row {
                    let on = self.sel.contains(&f.path);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, th + 52.0), Sense::hover());
                    let p = ui.painter();
                    p.rect(rect, CornerRadius::same(R_LG), if on { RAISED } else { CARD }, Stroke::new(1.0, if on { ACCENT } else { LINE }), StrokeKind::Inside);
                    let thumb = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(cw - 2.0, th));
                    p.rect_filled(thumb, CornerRadius { nw: R_LG, ne: R_LG, sw: 0, se: 0 }, Color32::BLACK);
                    if let Some(t) = sh.thumbs.get(&sh.bk, ui.ctx(), &f.path, 0.5, (270, 480)) {
                        media::paint_image(ui, &t, thumb, false);
                    }
                    // Clique duplo abre no player do Windows.
                    let tr = ui.interact(thumb, egui::Id::unique(("lib", &f.path)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Clique duplo para abrir");
                    if tr.double_clicked() {
                        backend::open_path(&f.path);
                    } else if tr.clicked() {
                        toggle = Some(f.path.clone());
                    }
                    let cb = egui::Rect::from_min_size(thumb.min + egui::vec2(8.0, 8.0), egui::vec2(24.0, 24.0));
                    if ui.put(cb, |ui: &mut Ui| w::check(ui, on, 22.0)).clicked() {
                        toggle = Some(f.path.clone());
                    }
                    let foot = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, thumb.bottom() + 6.0), rect.max - egui::vec2(6.0, 6.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.allocate_ui(egui::vec2(ui.available_width() - 62.0, 40.0), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                w::truncated(ui, RichText::new(&f.name).font(body(T_SEC)).color(TEXT)).on_hover_text(&f.name);
                                ui.label(w::mono_text(format!("{} · {}", bytes(f.size), date_time(&f.modified)), T_META, TEXT_3));
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::icon_button(ui, ic::FOLDER, "Mostrar na pasta").clicked() {
                                backend::reveal(&f.path);
                            }
                            if w::icon_button(ui, ic::PLAY, "Abrir").clicked() {
                                backend::open_path(&f.path);
                            }
                        });
                    });
                }
            });
            ui.add_space(gap - 8.0);
        }
        if let Some(p) = toggle {
            if !self.sel.remove(&p) {
                self.sel.insert(p);
            }
        }
    }

    pub fn overlays(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let Some(list) = self.confirm_delete.clone() else { return };
        let (open, choice) = w::modal(ctx, "lib-delete", &format!("Apagar {} vídeos?", list.len()), "", 440.0, |ui| {
            let mut c = None;
            w::pad(ui, |ui| {
                ui.label(w::dim("Os arquivos são apagados do disco de vez. Eles não vão para a Lixeira."));
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
                self.confirm_delete = None;
                self.sel.clear();
                self.t_op = Some(sh.bk.spawn(ctx, async move {
                    let mut n = 0;
                    for p in list {
                        if tokio::fs::remove_file(&p).await.is_ok() {
                            n += 1;
                        }
                    }
                    Ok(format!("{n} vídeos apagados"))
                }));
            }
            Some(false) => self.confirm_delete = None,
            None if !open => self.confirm_delete = None,
            None => {}
        }
    }
}
