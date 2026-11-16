//! IA: quem transcreve (Whisper local ou APIs), LLM da análise de retenção e
//! modelos do Whisper local. As chaves ficam só neste computador.

use eframe::egui::{self, RichText, Sense, Ui};
use flaxy_core::ai::{self, AiSettings};

use crate::app::Shared;
use crate::backend::{self, Res, Task};
use crate::icons as ic;
use crate::theme::*;
use crate::widgets::{self as w, Kind, Tone};

struct Provider {
    id: &'static str,
    name: &'static str,
    note: &'static str,
    key: Option<&'static str>,
}

const PROVIDERS: [Provider; 5] = [
    Provider { id: "local", name: "Whisper local", note: "Roda neste computador. Grátis, sem internet depois do download do modelo", key: None },
    Provider { id: "groq", name: "Groq", note: "Whisper large-v3 turbo hospedado. Muito rápido", key: Some("groq") },
    Provider { id: "openai", name: "OpenAI", note: "whisper-1 com tempo por palavra", key: Some("openai") },
    Provider { id: "deepgram", name: "Deepgram", note: "Nova-3, bom em português", key: Some("deepgram") },
    Provider { id: "assemblyai", name: "AssemblyAI", note: "Universal, tempo por palavra", key: Some("assemblyai") },
];

const LLMS: [(&str, &str, &str, Option<&str>); 4] = [
    ("anthropic", "Anthropic", "claude-opus-5-5", Some("anthropic")),
    ("openai", "OpenAI", "nome do modelo", Some("openai")),
    ("groq", "Groq", "llama-3.3-70b-versatile", Some("groq")),
    ("ollama", "Ollama (local)", "qwen2.5:14b", None),
];

const LANGS: [(&str, &str); 4] = [("pt", "Português"), ("en", "Inglês"), ("es", "Espanhol"), ("", "Detectar")];

fn key_mut<'a>(s: &'a mut AiSettings, k: &str) -> &'a mut String {
    match k {
        "groq" => &mut s.keys.groq,
        "openai" => &mut s.keys.openai,
        "deepgram" => &mut s.keys.deepgram,
        "assemblyai" => &mut s.keys.assemblyai,
        _ => &mut s.keys.anthropic,
    }
}

#[derive(Default)]
pub struct Ia {
    s: Option<AiSettings>,
    dirty: bool,
    t_load: Option<Task<Res<AiSettings>>>,
    t_save: Option<Task<Res<AiSettings>>>,
}

impl Ia {
    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        if self.s.is_none() && self.t_load.is_none() {
            let bk = sh.bk.clone();
            self.t_load = Some(sh.bk.spawn(ui.ctx(), async move { ai::load(&bk.db).await.map_err(backend::e) }));
        }
        if let Some(r) = self.t_load.as_ref().and_then(|t| t.take()) {
            self.t_load = None;
            match r {
                Ok(s) => self.s = Some(s),
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_save.as_ref().and_then(|t| t.take()) {
            self.t_save = None;
            match r {
                Ok(s) => {
                    self.s = Some(s);
                    self.dirty = false;
                    sh.toasts.ok("Configuração salva");
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        let mut save = false;
        let dirty = self.dirty;
        w::page_head(ui, "IA", "Quem transcreve os vídeos e quem escolhe os melhores trechos", |ui| {
            ui.add_enabled_ui(dirty, |ui| {
                if w::button(ui, Kind::Primary, Some(ic::SAVE), "Salvar").clicked() {
                    save = true;
                }
            });
        });
        let Some(s) = self.s.as_mut() else { return };
        if save {
            let bk = sh.bk.clone();
            let v = s.clone();
            self.t_save = Some(sh.bk.spawn(ui.ctx(), async move { ai::save(&bk.db, v).await.map_err(backend::e) }));
        }

        let gap = 12.0;
        let side_w = 380.0;
        let main_w = ui.available_width() - side_w - gap;
        let mut changed = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            w::column(ui, main_w, |ui| {
                ui.spacing_mut().item_spacing.y = gap;
                w::card(ui, "Transcrição", "", |_| {}, |ui| {
                    let cli = ai::whisper_cli().is_some();
                    for (i, p) in PROVIDERS.iter().enumerate() {
                        if i > 0 {
                            w::line(ui);
                        }
                        let on = s.transcriber == p.id;
                        // Clique na linha inteira escolhe o provedor. Registrado ANTES do
                        // conteúdo (com o retângulo do frame anterior) para ficar por baixo
                        // do campo da chave; registrado depois, ele cobria o campo e o
                        // TextEdit nunca recebia foco, então não dava para digitar nem colar.
                        let row_id = egui::Id::unique(("prov", p.id));
                        let row_rect = ui.ctx().data(|d| d.get_temp::<egui::Rect>(row_id));
                        let row_clicked = row_rect.is_some_and(|r| ui.interact(r, row_id, Sense::click()).clicked());
                        let resp = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_width(ui.available_width());
                                if w::radio(ui, on).clicked() && !on {
                                    s.transcriber = p.id.into();
                                    changed = true;
                                }
                                w::fixed(ui, 130.0, RichText::new(p.name).font(strong(T_BODY)).color(TEXT));
                                let note_w = (ui.available_width() - 290.0).max(120.0);
                                ui.allocate_ui(egui::vec2(note_w, 20.0), |ui| {
                                    ui.add(egui::Label::new(w::dim(p.note)).wrap());
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match p.key {
                                    Some(k) => {
                                        let v = key_mut(s, k);
                                        if ui.add(egui::TextEdit::singleline(v).password(true).hint_text("Chave da API").desired_width(230.0).font(mono(T_SEC))).changed() {
                                            changed = true;
                                        }
                                        let has = !key_mut(s, k).is_empty();
                                        let (r, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
                                        ui.painter().circle_filled(r.center(), 4.0, if has { OK } else { RAISED_2 });
                                        resp.on_hover_text(if has { "Chave salva" } else { "Sem chave" });
                                    }
                                    None => {
                                        ui.label(w::dim(if cli { "whisper-cli pronto" } else { "whisper-cli ausente" }));
                                    }
                                });
                            });
                        });
                        ui.ctx().data_mut(|d| d.insert_temp(row_id, resp.response.rect));
                        if row_clicked && !on {
                            s.transcriber = p.id.into();
                            changed = true;
                        }
                    }
                    w::line(ui);
                    w::pad(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                w::label(ui, "Idioma dos vídeos");
                                let cur = LANGS.iter().find(|l| l.0 == s.language).map(|l| l.1).unwrap_or("Detectar");
                                egui::ComboBox::from_id_salt("lang").width(200.0).selected_text(cur).show_ui(ui, |ui| {
                                    for (k, l) in LANGS {
                                        if ui.selectable_label(s.language == k, l).clicked() {
                                            s.language = k.into();
                                            changed = true;
                                        }
                                    }
                                });
                            });
                            ui.add_space(16.0);
                            let cur = PROVIDERS.iter().find(|p| p.id == s.transcriber).map(|p| p.name).unwrap_or("");
                            ui.label(w::muted(format!("Em uso: {cur}. As chaves ficam salvas só neste computador")));
                        });
                    });
                });

                w::card(ui, "Análise de retenção", "LLM que escolhe os melhores trechos na página Clipes", |_| {}, |ui| {
                    w::pad(ui, |ui| {
                        w::label(ui, "Provedor");
                        let opts: Vec<(String, &str)> = LLMS.iter().map(|l| (l.0.to_string(), l.1)).collect();
                        changed |= w::seg(ui, &mut s.llm_provider, &opts);
                        ui.add_space(8.0);
                        match LLMS.iter().find(|l| l.0 == s.llm_provider) {
                            None => {
                                ui.label(w::muted("Escolha um provedor para usar a página Clipes"));
                            }
                            Some((id, name, placeholder, key)) => {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        w::label(ui, match *id {
                                            "anthropic" => "Modelo (vazio usa o padrão)",
                                            "groq" => "Modelo preferido (troca sozinho no limite)",
                                            _ => "Modelo",
                                        });
                                        changed |= ui.add(egui::TextEdit::singleline(&mut s.llm_model).hint_text(*placeholder).desired_width(260.0).font(mono(T_SEC))).changed();
                                    });
                                    ui.add_space(16.0);
                                    ui.vertical(|ui| match key {
                                        Some(k) => {
                                            w::label(ui, &format!("Chave da {name}{}", if *k != "anthropic" { " (a mesma da transcrição)" } else { "" }));
                                            changed |= ui.add(egui::TextEdit::singleline(key_mut(s, k)).password(true).hint_text("Chave da API").desired_width(260.0).font(mono(T_SEC))).changed();
                                        }
                                        None => {
                                            w::label(ui, "Endereço do Ollama");
                                            changed |= ui.add(egui::TextEdit::singleline(&mut s.ollama_url).hint_text("http://127.0.0.1:11434").desired_width(260.0).font(mono(T_SEC))).changed();
                                        }
                                    });
                                });
                                if *id == "groq" {
                                    ui.add_space(8.0);
                                    ui.add(egui::Label::new(w::muted("Cada modelo do Groq tem a sua cota grátis. Quando o preferido bate no limite, o FLAXY passa sozinho para outro modelo da sua chave e volta quando a cota dele renova. Pode deixar vazio")).wrap());
                                }
                            }
                        }
                    });
                });
            });

            w::column(ui, side_w, |ui| {
                w::card(ui, "Modelos do Whisper local", "", |_| {}, |ui| {
                    let dir = sh.bk.models_dir();
                    let dl = sh.bk.downloads.lock().unwrap().clone();
                    let mut busy = false;
                    for (i, (name, mb)) in ai::MODELS.iter().enumerate() {
                        if i > 0 {
                            w::line(ui);
                        }
                        let installed = ai::model_file(&dir, name).is_file();
                        let state = dl.get(*name).cloned();
                        let progress = match &state {
                            Some(Ok(p)) if !installed => Some(*p),
                            _ => None,
                        };
                        busy |= progress.is_some();
                        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_width(ui.available_width());
                                ui.allocate_ui(egui::vec2(ui.available_width() - 100.0, 36.0), |ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        let mut t = name.to_string();
                                        if *name == "large-v3-turbo" {
                                            t.push_str(" · recomendado");
                                        }
                                        ui.label(w::mono_text(t, T_SEC, TEXT));
                                        match (&state, progress) {
                                            (_, Some(p)) => {
                                                w::progress(ui, p as f32 / 100.0, Tone::Accent, None);
                                            }
                                            (Some(Err(e)), _) => {
                                                ui.label(RichText::new(e).font(body(T_META)).color(ERR));
                                            }
                                            _ => {
                                                ui.label(w::mono_text(format!("{} MB", crate::format::thousands(*mb as i64)), T_META, TEXT_3));
                                            }
                                        }
                                    });
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if installed {
                                        let on = s.local_model == *name;
                                        if w::button(ui, if on { Kind::Primary } else { Kind::Default }, on.then_some(ic::CHECK), if on { "Em uso" } else { "Usar" }).clicked() && !on {
                                            s.local_model = name.to_string();
                                            changed = true;
                                        }
                                    } else if let Some(p) = progress {
                                        ui.label(w::mono_text(format!("{p}%"), T_META, TEXT_3));
                                    } else if w::button(ui, Kind::Default, Some(ic::DOWNLOAD), "Baixar").clicked() {
                                        start_download(sh, ui.ctx(), name);
                                    }
                                });
                            });
                        });
                    }
                    if busy {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                    }
                });
            });
        });
        self.dirty |= changed;
    }
}

fn start_download(sh: &Shared, ctx: &egui::Context, name: &str) {
    {
        let mut dl = sh.bk.downloads.lock().unwrap();
        if matches!(dl.get(name), Some(Ok(p)) if *p < 100) {
            return;
        }
        dl.insert(name.to_string(), Ok(0));
    }
    let (bk, n, ctx) = (sh.bk.clone(), name.to_string(), ctx.clone());
    sh.bk.rt.spawn(async move {
        let dir = bk.models_dir();
        let (b2, n2, c2) = (bk.clone(), n.clone(), ctx.clone());
        let res = ai::download_model(&dir, &n, move |p| {
            b2.downloads.lock().unwrap().insert(n2.clone(), Ok(p));
            c2.request_repaint();
        })
        .await;
        let mut dl = bk.downloads.lock().unwrap();
        match res {
            Ok(_) => {
                dl.remove(&n);
            }
            Err(e) => {
                dl.insert(n, Err(e.to_string()));
            }
        }
        ctx.request_repaint();
    });
}
