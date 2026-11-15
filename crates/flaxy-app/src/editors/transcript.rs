//! Edita o texto de cada palavra sem mexer nos tempos. Frases separadas por pausa.

use eframe::egui::{self, RichText, TextEdit};
use flaxy_core::captions::Word;

use crate::app::Shared;
use crate::backend::{self, Res, Task};
use crate::format::timecode;
use crate::theme::*;
use crate::widgets::{self as w, Kind};

pub struct TranscriptEditor {
    pub id: i64,
    name: String,
    words: Option<Vec<Word>>,
    load: Option<Task<Res<Vec<Word>>>>,
    save: Option<Task<Res<()>>>,
    dirty: bool,
}

impl TranscriptEditor {
    pub fn new(id: i64, name: &str) -> Self {
        Self { id, name: name.to_string(), words: None, load: None, save: None, dirty: false }
    }

    /// Devolve false quando a janela fecha.
    pub fn show(&mut self, ctx: &egui::Context, sh: &mut Shared) -> bool {
        if self.words.is_none() && self.load.is_none() {
            let (bk, id) = (sh.bk.clone(), self.id);
            self.load = Some(sh.bk.spawn(ctx, async move { bk.transcripts.get(id).await.map(|t| t.words).map_err(backend::e) }));
        }
        if let Some(r) = self.load.as_ref().and_then(|t| t.take()) {
            self.load = None;
            match r {
                Ok(v) => self.words = Some(v),
                Err(e) => {
                    sh.toasts.err(e);
                    return false;
                }
            }
        }
        if let Some(r) = self.save.as_ref().and_then(|t| t.take()) {
            self.save = None;
            match r {
                Ok(()) => {
                    sh.toasts.ok("Legenda salva");
                    return false;
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        let count = self.words.as_ref().map(|v| v.len()).unwrap_or(0);
        let mut keep = true;
        let (open, _) = w::modal(ctx, "transcript", &format!("Legenda · {}", self.name), &format!("{count} palavras"), 880.0, |ui| {
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.6).auto_shrink([false, true]).show(ui, |ui| {
                let Some(words) = self.words.as_mut() else {
                    w::empty(ui, crate::icons::CAPTIONS, "Carregando");
                    return;
                };
                if words.is_empty() {
                    w::empty(ui, crate::icons::CAPTIONS, "Nenhuma fala encontrada");
                    return;
                }
                // Linhas por pausa > 0,6 s ou fim de frase.
                let mut lines: Vec<Vec<usize>> = Vec::new();
                for i in 0..words.len() {
                    let brk = i == 0 || words[i].start - words[i - 1].end > 0.6 || words[i - 1].text.ends_with(['.', '!', '?']);
                    if brk {
                        lines.push(Vec::new());
                    }
                    lines.last_mut().unwrap().push(i);
                }
                for idxs in lines {
                    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 8)).show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            w::fixed(ui, 96.0, w::mono_text(timecode(words[idxs[0]].start, true), T_META, TEXT_3));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                                for i in idxs {
                                    let wd = (words[i].text.chars().count().max(2) as f32 + 1.5) * 8.0;
                                    if ui.add(TextEdit::singleline(&mut words[i].text).desired_width(wd).font(body(T_SEC))).changed() {
                                        self.dirty = true;
                                    }
                                }
                            });
                        });
                    });
                }
            });
            w::line(ui);
            w::pad(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Apague o texto de uma palavra para tirá-la da legenda").font(body(T_SEC)).color(TEXT_3));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(self.dirty && self.save.is_none(), |ui| {
                            if w::button(ui, Kind::Primary, None, "Salvar legenda").clicked() {
                                let words: Vec<Word> = self.words.clone().unwrap_or_default().into_iter().filter(|w| !w.text.trim().is_empty()).collect();
                                let (bk, id) = (sh.bk.clone(), self.id);
                                self.save = Some(sh.bk.spawn(ui.ctx(), async move { bk.transcripts.set_words(id, &words).await.map(|_| ()).map_err(backend::e) }));
                            }
                        });
                        if w::button(ui, Kind::Ghost, None, "Cancelar").clicked() {
                            keep = false;
                        }
                    });
                });
            });
        });
        open && keep
    }
}
