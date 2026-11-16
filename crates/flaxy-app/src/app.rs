//! Janela principal: barra lateral, páginas e confirmação de fechamento.

use std::sync::Arc;

use eframe::egui::{self, Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke};

use crate::backend::Backend;
use crate::icons as ic;
use crate::media::Thumbs;
use crate::pages;
use crate::theme::*;
use crate::widgets::{self as w, Toasts};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Inicio,
    Lotes,
    Prospeccao,
    Clipes,
    Biblioteca,
    Modelos,
    Ia,
    Config,
}

/// Estado compartilhado que toda página recebe.
pub struct Shared {
    pub bk: Arc<Backend>,
    pub toasts: Toasts,
    pub thumbs: Thumbs,
    pub goto: Option<Page>,
    /// Link enviado por outra página (Prospecção) para o formulário de Clipes.
    pub clip_url: Option<String>,
    pub caption_fonts: Vec<String>,
}

pub struct App {
    sh: Shared,
    page: Page,
    inicio: pages::inicio::Inicio,
    lotes: pages::lotes::Lotes,
    prospeccao: pages::prospeccao::Prospeccao,
    clipes: pages::clipes::Clipes,
    biblioteca: pages::biblioteca::Biblioteca,
    modelos: pages::modelos::Modelos,
    ia: pages::ia::Ia,
    config: pages::config::Config,
    confirm_close: bool,
    allow_close: bool,
}

impl App {
    pub fn new(bk: Arc<Backend>, caption_fonts: Vec<String>) -> Self {
        let mut toasts = Toasts::default();
        if std::env::args().any(|a| a == crate::updater::RELAUNCH_ARG) {
            toasts.ok(format!("FLAXY atualizado para v{}", env!("CARGO_PKG_VERSION")));
        }
        Self {
            sh: Shared { bk, toasts, thumbs: Thumbs::default(), goto: None, clip_url: None, caption_fonts },
            // FLAXY_PAGE abre direto numa página (atalho de desenvolvimento).
            page: match std::env::var("FLAXY_PAGE").unwrap_or_default().as_str() {
                "lotes" => Page::Lotes,
                "clipes" => Page::Clipes,
                "prospeccao" => Page::Prospeccao,
                "biblioteca" => Page::Biblioteca,
                "modelos" => Page::Modelos,
                "ia" => Page::Ia,
                "config" => Page::Config,
                _ => Page::Inicio,
            },
            inicio: Default::default(),
            lotes: pages::lotes::Lotes::default(),
            prospeccao: Default::default(),
            clipes: Default::default(),
            biblioteca: Default::default(),
            modelos: Default::default(),
            ia: Default::default(),
            config: Default::default(),
            confirm_close: false,
            allow_close: false,
        }
    }

    fn rendering(&self) -> bool {
        self.sh.bk.live.lock().map(|l| l.items.values().any(|(s, _)| s == "processing")).unwrap_or(false)
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("side")
            .exact_size(248.0)
            .resizable(false)
            .frame(Frame::NONE.fill(SIDE).inner_margin(Margin { left: 12, right: 12, top: 20, bottom: 16 }).stroke(Stroke::new(1.0, LINE)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // Marca: glifo plano + nome. Versão em mono, discreta.
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 32.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add_space(12.0);
                    ui.label(RichText::new(ic::CLAPPERBOARD).size(ICON).color(ACCENT_INK));
                    ui.label(RichText::new("FLAXY").font(strong(T_LOGO)).color(TEXT));
                    ui.label(w::mono_text(format!("v{}", env!("CARGO_PKG_VERSION")), T_META, TEXT_3));
                });
                let groups: [(&str, &[(Page, &str, &str)]); 2] = [
                    (
                        "Produção",
                        &[
                            (Page::Inicio, ic::GAUGE, "Início"),
                            (Page::Lotes, ic::LAYERS, "Lotes"),
                            (Page::Prospeccao, ic::TELESCOPE, "Prospecção"),
                            (Page::Clipes, ic::SCISSORS, "Clipes"),
                            (Page::Biblioteca, ic::FOLDER_OPEN, "Biblioteca"),
                        ],
                    ),
                    (
                        "Ajustes",
                        &[
                            (Page::Modelos, ic::LAYOUT_TEMPLATE, "Modelos"),
                            (Page::Ia, ic::BOT, "IA"),
                            (Page::Config, ic::SETTINGS, "Configurações"),
                        ],
                    ),
                ];
                for (label, items) in groups {
                    // Rótulo: 24 acima, 8 abaixo; linhas de 32 com 2 entre elas.
                    ui.add_space(24.0);
                    let (lr, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
                    ui.painter().text(lr.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, label, body(T_META), TEXT_3);
                    ui.add_space(8.0);
                    for (i, (page, icon, name)) in items.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(2.0);
                        }
                        let on = self.page == *page;
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
                        let p = ui.painter();
                        if on {
                            p.rect_filled(rect, CornerRadius::same(R), RAISED);
                            p.hline(rect.x_range().shrink(R as f32), rect.top() + 1.0, Stroke::new(1.0, HIGHLIGHT));
                        } else if r.hovered() {
                            p.rect_filled(rect, CornerRadius::same(R), HOVER);
                        }
                        let c = if on || r.hovered() { TEXT } else { TEXT_2 };
                        let ic_c = if on { TEXT } else { TEXT_3 };
                        p.text(rect.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, *icon, body(ICON), ic_c);
                        p.text(rect.left_center() + egui::vec2(38.0, 0.0), egui::Align2::LEFT_CENTER, *name, body(T_BODY), c);
                        if r.has_focus() {
                            p.rect_stroke(rect, CornerRadius::same(R), Stroke::new(1.5, ACCENT_INK), egui::StrokeKind::Inside);
                        }
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.page = *page;
                        }
                    }
                }

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let enc = self.sh.bk.encoder.lock().unwrap().clone();
                    let ff = flaxy_core::media::ffmpeg().is_some();
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(w::mono_text(
                            format!("{} · {}", enc.as_deref().unwrap_or("testando GPU"), if ff { "ffmpeg ok" } else { "sem ffmpeg" }),
                            T_META,
                            TEXT_3,
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_space(12.0);
                        let rendering = self.rendering();
                        w::status_dot(ui, if !ff { ERR } else if rendering { ACCENT } else { OK });
                        ui.label(w::dim(if !ff { "FFmpeg ausente" } else if rendering { "Renderizando" } else { "Pronto" }));
                    });
                });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Fechar com renders em andamento pede confirmação.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.rendering() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }

        // Procura versão nova ao abrir e baixa sozinho (vale ao reabrir).
        self.config.auto_update(&ctx, &mut self.sh);

        self.sidebar(ui);
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt(("page", self.page as u8)).auto_shrink([false, false]).show(ui, |ui| {
                    Frame::NONE.inner_margin(Margin { left: 32, right: 32, top: 32, bottom: 48 }).show(ui, |ui| {
                        // Conteúdo fluido até 1600, como app de desktop.
                        ui.set_width(ui.available_width().min(1600.0));
                        let sh = &mut self.sh;
                        match self.page {
                            Page::Inicio => self.inicio.ui(ui, sh),
                            Page::Lotes => self.lotes.ui(ui, sh),
                            Page::Prospeccao => self.prospeccao.ui(ui, sh),
                            Page::Clipes => self.clipes.ui(ui, sh),
                            Page::Biblioteca => self.biblioteca.ui(ui, sh),
                            Page::Modelos => self.modelos.ui(ui, sh),
                            Page::Ia => self.ia.ui(ui, sh),
                            Page::Config => self.config.ui(ui, sh),
                        }
                    });
                });
            });

        // Janelas que pertencem às páginas (editores) ficam fora da rolagem.
        let sh = &mut self.sh;
        match self.page {
            Page::Lotes => self.lotes.overlays(&ctx, sh),
            Page::Clipes => self.clipes.overlays(&ctx, sh),
            Page::Modelos => self.modelos.overlays(&ctx, sh),
            Page::Biblioteca => self.biblioteca.overlays(&ctx, sh),
            _ => {}
        }

        if let Some(p) = self.sh.goto.take() {
            self.page = p;
        }

        if self.confirm_close {
            let (open, choice) = w::modal(&ctx, "confirm-close", "Fechar o FLAXY?", "", 440.0, |ui| {
                let mut c = None;
                w::pad(ui, |ui| {
                    ui.label(RichText::new("Há vídeos renderizando agora").color(TEXT));
                    ui.label(w::dim("Os itens em andamento voltam para a fila e continuam quando você abrir o FLAXY de novo."));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if w::button(ui, w::Kind::Primary, None, "Fechar mesmo assim").clicked() {
                            c = Some(true);
                        }
                        if w::button(ui, w::Kind::Ghost, None, "Continuar").clicked() {
                            c = Some(false);
                        }
                    });
                });
                c
            });
            match choice.flatten() {
                Some(true) => {
                    self.allow_close = true;
                    self.confirm_close = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(false) => {
                    self.confirm_close = false;
                    crate::updater::cancel_restart();
                }
                None if !open => {
                    self.confirm_close = false;
                    crate::updater::cancel_restart();
                }
                None => {}
            }
        }

        self.sh.toasts.show(&ctx);
    }
}
