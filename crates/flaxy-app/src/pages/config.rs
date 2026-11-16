//! Configurações: estado das ferramentas, pastas e preferências do app.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};

use crate::app::Shared;
use crate::backend::{self, AppSettings, Res, Task};
use crate::icons as ic;
use crate::theme::*;
use crate::updater::{self, Phase, Updater};
use crate::widgets::{self as w, Kind, Tone};

#[derive(Default)]
pub struct Config {
    form: Option<AppSettings>,
    dirty: bool,
    t_save: Option<Task<Res<()>>>,
    upd: Updater,
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}

fn row(ui: &mut Ui, k: &str, v: &str, note: &str, action: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(egui::vec2(220.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(220.0);
                ui.label(w::dim(k));
            });
            ui.allocate_ui(egui::vec2(260.0, 20.0), |ui| {
                w::truncated(ui, w::mono_text(v, T_SEC, TEXT)).on_hover_text(v);
            });
            ui.label(w::muted(note));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), action);
        });
    });
}

fn tool(p: Option<&Path>) -> (String, String) {
    match p {
        Some(p) => ("Encontrado".into(), p.display().to_string()),
        None => ("Ausente".into(), "Reinstale o FLAXY".into()),
    }
}

impl Config {
    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.form.is_none() {
            let mut f = sh.bk.settings.clone();
            if f.output_dir.is_empty() {
                f.output_dir = sh.bk.cfg.output_dir().display().to_string();
            }
            if f.max_concurrent == 0 {
                f.max_concurrent = sh.bk.cfg.max_concurrent;
            }
            self.form = Some(f);
        }
        if let Some(r) = self.t_save.as_ref().and_then(|t| t.take()) {
            self.t_save = None;
            match r {
                Ok(()) => {
                    self.dirty = false;
                    sh.toasts.ok("Salvo. Vale a partir da próxima abertura do FLAXY.");
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        let mut save = false;
        let dirty = self.dirty;
        w::page_head(ui, "Configurações", "Ferramentas, pastas e desempenho", |ui| {
            ui.add_enabled_ui(dirty, |ui| {
                if w::button(ui, Kind::Primary, Some(ic::SAVE), "Salvar").clicked() {
                    save = true;
                }
            });
        });

        let enc = sh.bk.encoder.lock().unwrap().clone().unwrap_or_else(|| "testando".into());
        let max_w = ui.available_width().min(1000.0);
        w::column(ui, max_w, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            self.updates(ui, sh);

            w::card(ui, "Ferramentas", "", |_| {}, |ui| {
                row(ui, "Versão", &format!("v{}", env!("CARGO_PKG_VERSION")), "App nativo (Rust + GPU)", |_| {});
                w::line(ui);
                let (s, n) = tool(flaxy_core::media::ffmpeg());
                row(ui, "FFmpeg", &s, &n, |_| {});
                w::line(ui);
                let note = if enc == "libx264" { "CPU. Nenhuma GPU compatível respondeu ao teste" } else { "GPU detectada no teste de encode" };
                row(ui, "Encoder de vídeo", &enc, note, |_| {});
                w::line(ui);
                let (s, n) = tool(flaxy_core::clips::ytdlp());
                row(ui, "yt-dlp", &s, &n, |_| {});
                w::line(ui);
                let (s, n) = match flaxy_core::ai::whisper_cli() {
                    Some(p) => ("Encontrado".to_string(), p.display().to_string()),
                    None => ("Ausente".to_string(), "Use uma API na aba IA ou coloque whisper-cli.exe em bin\\".to_string()),
                };
                row(ui, "whisper-cli", &s, &n, |_| {});
            });

            let paths: [(&str, PathBuf, &str); 3] = [
                ("Dados (banco, fundos, modelos)", sh.bk.cfg.storage_dir.clone(), ""),
                ("Trabalho (downloads, logs)", sh.bk.cfg.work_dir.clone(), ""),
                ("Vídeos prontos (em uso)", sh.bk.cfg.output_dir(), ""),
            ];
            w::card(ui, "Pastas", "", |_| {}, |ui| {
                for (i, (k, p, n)) in paths.iter().enumerate() {
                    if i > 0 {
                        w::line(ui);
                    }
                    row(ui, k, &p.display().to_string(), n, |ui| {
                        if w::icon_button(ui, ic::FOLDER_OPEN, "Abrir pasta").clicked() {
                            backend::open_path(p);
                        }
                    });
                }
            });

            let f = self.form.as_mut().unwrap();
            let mut changed = false;
            w::card(ui, "Preferências", "valem na próxima abertura", |_| {}, |ui| {
                w::pad(ui, |ui| {
                    w::label(ui, "Pasta dos vídeos prontos");
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(&mut f.output_dir).desired_width(520.0).font(mono(T_SEC))).changed();
                        if w::button(ui, Kind::Default, Some(ic::FOLDER), "Escolher").clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_title("Pasta dos vídeos prontos").pick_folder() {
                                f.output_dir = d.display().to_string();
                                changed = true;
                            }
                        }
                    });
                    ui.add_space(12.0);
                    w::label_value(ui, "Renders simultâneos", &f.max_concurrent.to_string());
                    changed |= ui.add(egui::Slider::new(&mut f.max_concurrent, 1..=4).show_value(false)).changed();
                    ui.label(RichText::new("1 é o ideal na maioria das máquinas: dois encoders disputando a mesma GPU terminam mais devagar que em fila").font(body(T_META)).color(TEXT_3));
                });
            });
            self.dirty |= changed;
        });

        if save {
            let f = self.form.clone().unwrap();
            let bk = sh.bk.clone();
            self.t_save = Some(sh.bk.spawn(ui.ctx(), async move {
                let p = PathBuf::from(f.output_dir.trim());
                if !f.output_dir.trim().is_empty() {
                    tokio::fs::create_dir_all(&p).await.map_err(|e| format!("Pasta inválida: {e}"))?;
                }
                backend::save_settings(&bk.db, &f).await.map_err(backend::e)
            }));
        }
    }
    /// Cartão de atualização: verificar, baixar com progresso e reiniciar.
    fn updates(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let phase = self.upd.phase();
        let busy = self.upd.busy();
        let cur = env!("CARGO_PKG_VERSION");
        let mut check = false;
        let mut install = None;
        let mut restart = false;
        w::card(ui, "Atualizações", &format!("instalada v{cur}"), |ui| {
            ui.add_enabled_ui(!busy && !matches!(phase, Phase::Ready { .. }), |ui| {
                if w::button(ui, Kind::Default, Some(ic::REFRESH_CW), "Verificar atualizações").clicked() {
                    check = true;
                }
            });
        }, |ui| {
            w::pad(ui, |ui| {
                match &phase {
                    Phase::Idle => {
                        ui.label(w::muted("Busca a versão mais nova publicada no GitHub e instala aqui mesmo, sem rodar o instalador."));
                    }
                    Phase::Checking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(w::dim("Procurando versão nova..."));
                        });
                    }
                    Phase::UpToDate { latest } => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ic::CHECK).size(ICON).color(OK));
                            ui.label(w::dim(format!("Você já está na versão mais nova (v{latest}).")));
                        });
                    }
                    Phase::Available(r) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ic::DOWNLOAD).size(ICON).color(ACCENT));
                            ui.label(RichText::new(format!("Versão v{} disponível", r.version)).font(strong(T_BODY)).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if w::button(ui, Kind::Primary, Some(ic::DOWNLOAD), "Atualizar agora").clicked() {
                                    install = Some(r.clone());
                                }
                            });
                        });
                        if !r.notes.is_empty() {
                            ui.add_space(8.0);
                            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                                ui.label(RichText::new(&r.notes).font(body(T_SEC)).color(TEXT_2));
                            });
                        }
                    }
                    Phase::Downloading { version, done, total } => {
                        let f = *done as f32 / (*total).max(1) as f32;
                        ui.horizontal(|ui| {
                            ui.label(w::dim(format!("Baixando v{version}")));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(w::mono_text(format!("{:.0}%  {} de {}", f * 100.0, mb(*done), mb(*total)), T_META, TEXT_2));
                            });
                        });
                        ui.add_space(8.0);
                        w::progress(ui, f, Tone::Accent, None);
                    }
                    Phase::Installing { version } => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(w::dim(format!("Conferindo e instalando v{version}...")));
                        });
                        ui.add_space(8.0);
                        w::progress(ui, 1.0, Tone::Accent, None);
                    }
                    Phase::Ready { version } => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ic::CHECK).size(ICON).color(OK));
                            ui.label(RichText::new(format!("v{version} instalada com sucesso")).font(strong(T_BODY)).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if w::button(ui, Kind::Primary, Some(ic::ROTATE_CCW), "Reiniciar e abrir atualizado").clicked() {
                                    restart = true;
                                }
                            });
                        });
                        ui.add_space(8.0);
                        w::progress(ui, 1.0, Tone::Done, None);
                        ui.add_space(6.0);
                        ui.label(w::muted("Arquivo conferido (tamanho e SHA-256). A nova versão abre ao reiniciar."));
                    }
                    Phase::Failed(e) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ic::CIRCLE_ALERT).size(ICON).color(ERR));
                            ui.label(RichText::new(e).font(body(T_SEC)).color(ERR));
                        });
                        ui.add_space(8.0);
                        w::progress(ui, 1.0, Tone::Err, None);
                    }
                }
            });
        });
        if check {
            let fut = self.upd.check(ui.ctx());
            sh.bk.spawn(ui.ctx(), fut);
        }
        if let Some(r) = install {
            let fut = self.upd.install(ui.ctx(), r);
            sh.bk.spawn(ui.ctx(), fut);
        }
        if restart {
            updater::restart(ui.ctx());
        }
    }
}
