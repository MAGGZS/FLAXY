//! Início: números da produção, gráfico dos últimos 7 dias, disco e lotes recentes.

use eframe::egui::{self, Align2, CornerRadius, RichText, Sense, Stroke, Ui};
use flaxy_core::models::Job;

use crate::app::{Page, Shared};
use crate::backend::{self, Res, Stats, Task};
use crate::format::{bytes, date_time, month_title, status_label, thousands};
use crate::icons as ic;
use crate::theme::*;
use crate::widgets::{self as w, Kind, Tone};

#[derive(Default)]
pub struct Inicio {
    stats: Option<Stats>,
    jobs: Vec<Job>,
    task: Option<Task<Res<(Stats, Vec<Job>)>>>,
    rev: u64,
}

impl Inicio {
    fn load(&mut self, ui: &Ui, sh: &Shared) {
        let bk = sh.bk.clone();
        self.rev = bk.rev();
        self.task = Some(sh.bk.spawn(ui.ctx(), async move {
            let s = backend::stats(bk.clone()).await?;
            let j = bk.jobs.list(6).await.map_err(backend::e)?;
            Ok((s, j))
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.task.is_none() && (self.stats.is_none() || self.rev != sh.bk.rev()) {
            self.load(ui, sh);
        }
        if let Some(r) = self.task.as_ref().and_then(|t| t.take()) {
            self.task = None;
            match r {
                Ok((s, j)) => {
                    self.stats = Some(s);
                    self.jobs = j;
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        w::page_head(ui, &month_title(), "Produção da estação", |ui| {
            if w::button(ui, Kind::Primary, Some(ic::PLUS), "Novo lote").clicked() {
                sh.goto = Some(Page::Lotes);
            }
        });

        let s = self.stats.clone().unwrap_or_default();
        let n = |k: &str| *s.by_status.get(k).unwrap_or(&0);
        let tiles = [
            ("Na fila", n("queued"), "aguardando render"),
            ("Renderizando", n("processing"), "agora"),
            ("Prontos", n("done"), "desde o início"),
            ("Falhas", n("error"), "com log disponível"),
        ];
        let gap = 12.0;
        let tw = (ui.available_width() - gap * 3.0) / 4.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, value, note) in tiles {
                w::column(ui, tw, |ui| {
                    w::card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        w::pad(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.label(w::dim(label));
                            ui.label(RichText::new(thousands(value)).font(mono(T_FIG)).color(TEXT));
                            ui.label(w::muted(note));
                        });
                    });
                });
            }
        });
        ui.add_space(gap);

        let total_w = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let chart_w = (total_w - gap) * 2.0 / 3.0;
            w::column(ui, chart_w, |ui| {
                let days = last_seven(&s.daily);
                let total: i64 = days.iter().map(|d| d.2).sum();
                w::card(ui, "Vídeos prontos por dia", &format!("{total} nos últimos 7 dias"), |_| {}, |ui| {
                    w::pad(ui, |ui| chart(ui, &days));
                });
            });
            let side_w = total_w - chart_w - gap;
            w::column(ui, side_w, |ui| {
                let tot = s.disk_output + s.disk_uploads + s.disk_templates;
                w::card(ui, "Disco", &bytes(tot), |_| {}, |ui| {
                    w::pad(ui, |ui| {
                        for (label, v) in [("Vídeos prontos", s.disk_output), ("Downloads de clipes", s.disk_uploads), ("Fundos de modelo", s.disk_templates)] {
                            ui.horizontal(|ui| {
                                ui.label(w::dim(label));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(w::mono_text(bytes(v), T_META, TEXT_3));
                                });
                            });
                            w::progress(ui, if tot > 0 { v as f32 / tot as f32 } else { 0.0 }, Tone::Neutral, None);
                            ui.add_space(12.0);
                        }
                        ui.label(w::muted(format!("{} {} de fundo", s.templates, if s.templates == 1 { "modelo" } else { "modelos" })));
                        ui.add_space(8.0);
                        if w::button(ui, Kind::Default, Some(ic::FOLDER_OPEN), "Abrir pasta dos vídeos").clicked() {
                            backend::open_path(&sh.bk.cfg.output_dir());
                        }
                    });
                });
            });
        });
        ui.add_space(gap);

        w::card(
            ui,
            "Lotes recentes",
            "",
            |ui| {
                if w::button(ui, Kind::Ghost, None, "Ver todos").clicked() {
                    sh.goto = Some(Page::Lotes);
                }
            },
            |ui| {
                if self.jobs.is_empty() {
                    w::empty(ui, ic::LAYERS, "Nenhum lote ainda");
                    return;
                }
                let live = sh.bk.live.lock().unwrap().items.clone();
                for (i, j) in self.jobs.iter().enumerate() {
                    if i > 0 {
                        w::line(ui);
                    }
                    job_row(ui, j, &live);
                }
            },
        );
    }
}

pub fn job_progress(j: &Job, live: &std::collections::HashMap<i64, (String, i64)>) -> (f32, usize) {
    let mut sum = 0i64;
    let mut done = 0;
    for it in &j.items {
        let (st, p) = live.get(&it.id).cloned().unwrap_or((it.status.clone(), it.progress));
        if st == "done" {
            done += 1;
            sum += 100;
        } else if st == "processing" {
            sum += p;
        }
    }
    let n = j.items.len().max(1) as f32;
    (sum as f32 / 100.0 / n, done)
}

fn job_row(ui: &mut Ui, j: &Job, live: &std::collections::HashMap<i64, (String, i64)>) {
    let (frac, done) = job_progress(j, live);
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            w::fixed(ui, 72.0, w::mono_text(format!("#{:04}", j.job.id), T_SEC, TEXT_3));
            w::fixed(ui, 120.0, format!("{} vídeos", j.items.len()));
            let tone = match j.job.status.as_str() {
                "done" => Tone::Done,
                "error" => Tone::Err,
                _ => Tone::Accent,
            };
            let rest = ui.available_width();
            w::progress(ui, frac, tone, Some((rest - 360.0).max(80.0)));
            w::fixed(ui, 60.0, w::mono_text(format!("{done}/{}", j.items.len()), T_SEC, TEXT_2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(w::mono_text(date_time(&j.job.created_at), T_META, TEXT_3));
                ui.label(w::dim(format!("{} ·", status_label(&j.job.status))));
            });
        });
    });
}

fn last_seven(rows: &[(String, i64)]) -> Vec<(String, String, i64)> {
    const WD: [&str; 7] = ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"];
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    (0..7)
        .rev()
        .map(|i| {
            let d = today - chrono::Duration::days(i);
            let key = d.format("%Y-%m-%d").to_string();
            let count = rows.iter().find(|r| r.0 == key).map(|r| r.1).unwrap_or(0);
            let label = if i == 0 { "hoje".to_string() } else { WD[d.weekday().num_days_from_monday() as usize].to_string() };
            (key, label, count)
        })
        .collect()
}

fn chart(ui: &mut Ui, days: &[(String, String, i64)]) {
    let max = days.iter().map(|d| d.2).max().unwrap_or(0).max(4);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 270.0), Sense::hover());
    let p = ui.painter();
    let axis = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 40.0, rect.bottom() - 28.0));
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left() + 52.0, rect.top() + 8.0), egui::pos2(rect.right(), rect.bottom() - 28.0));
    for (i, v) in [max, max / 2, 0].iter().enumerate() {
        let y = plot.top() + plot.height() * i as f32 / 2.0;
        p.text(egui::pos2(axis.right(), y), Align2::RIGHT_CENTER, v.to_string(), mono(T_META), TEXT_3);
        p.hline(plot.x_range(), y, Stroke::new(1.0, if i == 2 { LINE_2 } else { LINE }));
    }
    let n = days.len() as f32;
    let slot = plot.width() / n;
    for (i, (_, label, count)) in days.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let bw = (slot - 16.0).min(56.0);
        let h = plot.height() * (*count as f32 / max as f32);
        let today = i + 1 == days.len();
        if *count > 0 {
            let bar = egui::Rect::from_min_max(egui::pos2(cx - bw / 2.0, plot.bottom() - h.max(4.0)), egui::pos2(cx + bw / 2.0, plot.bottom()));
            p.rect_filled(bar, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, if today { ACCENT } else { RAISED_2 });
            p.text(egui::pos2(cx, bar.top() - 6.0), Align2::CENTER_BOTTOM, count.to_string(), mono(T_META), if today { TEXT } else { TEXT_3 });
        }
        p.text(egui::pos2(cx, rect.bottom() - 6.0), Align2::CENTER_BOTTOM, label, mono(T_META), if today { TEXT_2 } else { TEXT_3 });
    }
}
