//! Prospecção: pesquise um assunto e veja o ranking dos vídeos mais vistos
//! dos últimos 7 dias no YouTube, por tipo, com as notícias da semana ao lado.
//! Qualquer vídeo vai direto para Clipes.

use eframe::egui::{self, text::LayoutJob, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, TextFormat, Ui};

use crate::app::{Page, Shared};
use crate::backend::{self, Res, Task};
use crate::icons as ic;
use crate::prospect::{self, Cat, Period, Results, Source, Video, REGIONS};
use crate::theme::*;
use crate::widgets::{self as w, Kind};

#[derive(Default)]
pub struct Prospeccao {
    query: String,
    cat: Cat,
    results: Option<Results>,
    t_search: Option<Task<Res<Results>>>,
    searching: String,
    key: String,
    key_saved: String,
    t_key: Option<Task<String>>,
    t_save: Option<Task<Res<String>>>,
    key_open: bool,
    thumbs: crate::youtube::ThumbCache,
    /// Código do país (vazio = Brasil) e janela de publicação. Ficam salvos.
    region: String,
    period: Period,
    /// Mostrar vídeos de qualquer idioma (desligado = só no idioma do país).
    any_lang: bool,
    t_filters: Option<Task<Filters>>,
    filters_loaded: bool,
    t_filters_save: Option<Task<()>>,
}

#[derive(Clone)]
struct Filters {
    region: String,
    period: Period,
    any_lang: bool,
}

async fn load_filters(db: sqlx::SqlitePool) -> Filters {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'prospect_filters'").fetch_optional(&db).await.ok().flatten();
    let v = row.and_then(|r| serde_json::from_str::<serde_json::Value>(&r.0).ok()).unwrap_or_default();
    Filters {
        region: v["region"].as_str().unwrap_or_default().to_string(),
        period: Period::from_id(v["period"].as_str().unwrap_or_default()),
        any_lang: v["any_lang"].as_bool().unwrap_or(false),
    }
}

async fn save_filters(db: sqlx::SqlitePool, f: Filters) {
    let v = serde_json::json!({ "region": f.region, "period": f.period.id(), "any_lang": f.any_lang }).to_string();
    let r = sqlx::query("INSERT INTO settings (key, value) VALUES ('prospect_filters', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(v)
        .execute(&db)
        .await;
    if let Err(e) = r {
        tracing::warn!("prospecção: não salvou os filtros: {e}");
    }
}

const ROW_H: f32 = 112.0;
const THUMB: egui::Vec2 = egui::vec2(160.0, 88.0);

/// Tira emojis e símbolos que as fontes do app não desenham (viram quadrados).
fn clean(s: &str) -> String {
    let kept: String = s
        .chars()
        .filter(|c| {
            let u = *c as u32;
            !(u >= 0x1F000 || (0x2600..=0x27BF).contains(&u) || (0x2B00..=0x2BFF).contains(&u) || u == 0xFE0F || u == 0x200D || (0xE000..=0xF8FF).contains(&u))
        })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn load_key(db: sqlx::SqlitePool) -> String {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = 'prospect'").fetch_optional(&db).await.ok().flatten();
    row.and_then(|r| serde_json::from_str::<serde_json::Value>(&r.0).ok())
        .and_then(|v| v["youtube_key"].as_str().map(str::to_string))
        .unwrap_or_default()
}

async fn save_key(db: sqlx::SqlitePool, key: String) -> Res<String> {
    let v = serde_json::json!({ "youtube_key": key }).to_string();
    sqlx::query("INSERT INTO settings (key, value) VALUES ('prospect', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(v)
        .execute(&db)
        .await
        .map_err(backend::e)?;
    Ok(key)
}

impl Prospeccao {
    fn start(&mut self, ui: &Ui, sh: &Shared) {
        let q = self.query.trim().to_string();
        if q.is_empty() || self.t_search.is_some() {
            return;
        }
        self.searching = q.clone();
        let key = Some(self.key_saved.clone()).filter(|k| !k.is_empty());
        self.t_search = Some(sh.bk.spawn(ui.ctx(), prospect::search(q, key, prospect::region(&self.region), self.period, !self.any_lang, Some(sh.bk.db.clone()))));
    }

    pub fn ui(&mut self, ui: &mut Ui, sh: &mut Shared) {
        // FLAXY_PROSPECT pesquisa direto ao abrir (atalho de desenvolvimento).
        if self.results.is_none() && self.t_search.is_none() && self.query.is_empty() {
            if let Ok(q) = std::env::var("FLAXY_PROSPECT") {
                self.query = q;
                self.start(ui, sh);
            }
        }
        // Chave salva: lida uma vez do banco.
        if self.t_key.is_none() && self.key_saved.is_empty() && self.key.is_empty() {
            self.t_key = Some(sh.bk.spawn(ui.ctx(), load_key(sh.bk.db.clone())));
        }
        if let Some(k) = self.t_key.as_ref().and_then(|t| t.take()) {
            self.key = k.clone();
            self.key_saved = k;
        }
        // Região e período salvos: lidos uma vez do banco.
        if !self.filters_loaded && self.t_filters.is_none() {
            self.t_filters = Some(sh.bk.spawn(ui.ctx(), load_filters(sh.bk.db.clone())));
        }
        if let Some(f) = self.t_filters.as_ref().and_then(|t| t.take()) {
            self.t_filters = None;
            self.filters_loaded = true;
            self.region = f.region;
            self.period = f.period;
            self.any_lang = f.any_lang;
        }
        if let Some(r) = self.t_save.as_ref().and_then(|t| t.take()) {
            self.t_save = None;
            match r {
                Ok(k) => {
                    sh.toasts.ok(if k.is_empty() { "Chave removida. A busca volta a usar o yt-dlp" } else { "Chave salva. As próximas buscas usam a API do YouTube" });
                    self.key_saved = k;
                    self.key_open = false;
                }
                Err(e) => sh.toasts.err(e),
            }
        }
        if let Some(r) = self.t_search.as_ref().and_then(|t| t.take()) {
            self.t_search = None;
            match r {
                Ok(res) => {
                    for wn in &res.warnings {
                        tracing::warn!("prospecção: {wn}");
                    }
                    // Abre na primeira categoria com resultado.
                    if res.list(self.cat).is_empty() {
                        if let Some(c) = Cat::ALL.iter().find(|c| !res.list(**c).is_empty()) {
                            self.cat = *c;
                        }
                    }
                    self.results = Some(res);
                }
                Err(e) => sh.toasts.err(e),
            }
        }

        let has_key = !self.key_saved.is_empty();
        let mut toggle_key = false;
        w::page_head(ui, "Prospecção", "Os vídeos mais vistos no YouTube sobre um assunto, por país e período", |ui| {
            if w::button(ui, Kind::Ghost, Some(ic::KEY_ROUND), if has_key { "API do YouTube ligada" } else { "Chave da API" }).clicked() {
                toggle_key = true;
            }
        });
        if toggle_key {
            self.key_open = !self.key_open;
        }
        if self.key_open {
            self.key_card(ui, sh);
            ui.add_space(12.0);
        }

        self.search_card(ui, sh);
        ui.add_space(16.0);

        if self.t_search.is_some() {
            w::card_frame().show(ui, |ui| {
                w::pad(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        ui.add(egui::Spinner::new().size(16.0).color(TEXT_3));
                        ui.label(RichText::new(format!("Pesquisando “{}” no YouTube e no Google Notícias", self.searching)).color(TEXT));
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.label(w::muted("Leva de 10 a 30 segundos"));
                    });
                });
            });
            return;
        }

        let Some(res) = self.results.take() else {
            w::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                w::empty(ui, ic::TELESCOPE, "Pesquise um assunto para ver o ranking dos mais vistos");
            });
            return;
        };
        self.results_view(ui, sh, &res);
        self.results = Some(res);
    }

    fn search_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let busy = self.t_search.is_some();
        let mut go = false;
        let mut filters_changed = false;
        w::card_frame().show(ui, |ui| {
            w::pad(ui, |ui| {
                ui.horizontal(|ui| {
                    let bw = 132.0;
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
                                ui.label(RichText::new(ic::SEARCH).color(TEXT_3));
                                let r = ui.add(
                                    egui::TextEdit::singleline(&mut self.query)
                                        .hint_text("Assunto, canal ou jogo")
                                        .frame(egui::Frame::NONE)
                                        .align(Align2::LEFT_CENTER)
                                        .min_size(egui::vec2(0.0, 44.0))
                                        .desired_width(f32::INFINITY)
                                        .font(body(T_BODY)),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    go = true;
                                }
                            });
                        });
                    ui.add_enabled_ui(!busy && !self.query.trim().is_empty(), |ui| {
                        if w::button_sized(ui, Kind::Primary, Some(ic::SEARCH), "Pesquisar", 44.0).clicked() {
                            go = true;
                        }
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(w::muted("Região"));
                    let cur = prospect::region(&self.region);
                    egui::ComboBox::from_id_salt("pros-region").width(170.0).selected_text(cur.name).show_ui(ui, |ui| {
                        for r in &REGIONS {
                            if ui.selectable_label(std::ptr::eq(r, cur), r.name).clicked() && !std::ptr::eq(r, cur) {
                                self.region = r.code.to_string();
                                filters_changed = true;
                            }
                        }
                    });
                    let only = !self.any_lang;
                    let tip = "Mostra só vídeos de canais que informam esse país no YouTube. Canal sem país conta pelo idioma do título. Desligue para ver os de fora também";
                    if w::check(ui, only, 18.0).on_hover_text(tip).clicked() {
                        self.any_lang = only;
                        filters_changed = true;
                    }
                    if ui.add(egui::Label::new(w::dim(format!("Só canais {}", cur.of))).sense(Sense::click())).on_hover_text(tip).clicked() {
                        self.any_lang = only;
                        filters_changed = true;
                    }
                    ui.add_space(16.0);
                    ui.label(w::muted("Período"));
                    let opts: Vec<(Period, &str)> = Period::ALL.iter().map(|p| (*p, p.label())).collect();
                    filters_changed |= w::seg(ui, &mut self.period, &opts);
                });
            });
        });
        if filters_changed {
            let f = Filters { region: self.region.clone(), period: self.period, any_lang: self.any_lang };
            self.t_filters_save = Some(sh.bk.spawn(ui.ctx(), save_filters(sh.bk.db.clone(), f)));
            // Com uma busca na tela, refaz com os filtros novos.
            if self.results.is_some() {
                go = true;
            }
        }
        if go {
            self.start(ui, sh);
        }
    }

    fn key_card(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let mut save: Option<String> = None;
        w::card(ui, "YouTube Data API", "opcional", |_| {}, |ui| {
            w::pad(ui, |ui| {
                ui.label(w::dim("Com a chave, a busca traz datas e durações exatas e mais shorts. Sem ela, o FLAXY pesquisa com o yt-dlp. Cada busca gasta cerca de 500 das 10.000 unidades diárias gratuitas."));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.key).password(true).hint_text("AIza…").desired_width(420.0).font(mono(T_SEC)));
                    ui.add_enabled_ui(self.t_save.is_none() && self.key.trim() != self.key_saved, |ui| {
                        if w::button(ui, Kind::Primary, Some(ic::SAVE), "Salvar").clicked() {
                            save = Some(self.key.trim().to_string());
                        }
                    });
                    if !self.key_saved.is_empty() && w::button(ui, Kind::Danger, Some(ic::TRASH_2), "Remover").clicked() {
                        self.key.clear();
                        save = Some(String::new());
                    }
                    if w::button(ui, Kind::Ghost, Some(ic::EXTERNAL_LINK), "Criar chave no Google Cloud").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab("https://console.cloud.google.com/apis/library/youtube.googleapis.com"));
                    }
                });
            });
        });
        if let Some(k) = save {
            self.t_save = Some(sh.bk.spawn(ui.ctx(), save_key(sh.bk.db.clone(), k)));
        }
    }

    fn results_view(&mut self, ui: &mut Ui, sh: &mut Shared, res: &Results) {
        // Tipos com a contagem de cada um + origem dos dados à direita.
        let labels: Vec<String> = Cat::ALL.iter().map(|c| format!("{}  {}", c.label(), res.list(*c).len())).collect();
        let opts: Vec<(Cat, &str)> = Cat::ALL.iter().zip(labels.iter()).map(|(c, l)| (*c, l.as_str())).collect();
        ui.horizontal(|ui| {
            w::seg(ui, &mut self.cat, &opts);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let src = match res.source {
                    Source::Api => "YouTube Data API",
                    Source::YtDlp => "YouTube",
                };
                ui.label(w::mono_text(format!("{src} · {} · {} · {}", res.region.name, res.period.label().to_lowercase(), res.at.format("%H:%M")), T_META, TEXT_3));
            });
        });
        if !res.related.is_empty() {
            ui.add_space(8.0);
            ui.label(w::muted(format!("Também buscado, temas próximos sugeridos pela IA: {}", res.related.join(", "))));
        }
        ui.add_space(16.0);

        let avail = ui.available_width();
        let side = 360.0;
        let wide = avail >= 1080.0;
        if wide {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                w::column(ui, avail - side - 16.0, |ui| self.ranking(ui, sh, res));
                w::column(ui, side, |ui| news(ui, res));
            });
        } else {
            self.ranking(ui, sh, res);
            ui.add_space(16.0);
            news(ui, res);
        }
    }

    fn ranking(&mut self, ui: &mut Ui, sh: &mut Shared, res: &Results) {
        let list = res.list(self.cat);
        let meta = if list.is_empty() { String::new() } else { format!("{} vídeos", list.len()) };
        let title = format!("{} sobre “{}”", self.cat.label(), res.query);
        w::card(ui, &title, &meta, |_| {}, |ui| {
            if list.is_empty() {
                let msg = match self.cat {
                    Cat::Shorts if res.source == Source::YtDlp => "Nenhum short apareceu na busca. Com a chave da API a lista de shorts fica completa".to_string(),
                    _ if res.local_only => format!("Nada de canais {} {}. Desmarque \"Só canais {}\" para ver os de fora", res.region.of, res.period.phrase(), res.region.of),
                    _ => format!("Nada desta categoria {}", res.period.phrase()),
                };
                w::empty(ui, ic::SEARCH, &msg);
                return;
            }
            let top = list.first().map(|v| v.views).unwrap_or(1).max(1);
            for (i, v) in list.iter().enumerate() {
                if i > 0 {
                    w::line(ui);
                }
                self.row(ui, sh, i, v, top);
            }
        });
    }

    fn row(&mut self, ui: &mut Ui, sh: &mut Shared, i: usize, v: &Video, top: u64) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let p = ui.painter().clone();
        if resp.hovered() {
            p.rect_filled(rect.shrink(1.0), CornerRadius::ZERO, HOVER);
        }
        let cy = rect.center().y;

        // Posição: as três primeiras em texto forte.
        let rank_c = if i < 3 { TEXT } else { TEXT_3 };
        p.text(egui::pos2(rect.left() + 20.0, cy), Align2::LEFT_CENTER, format!("{:02}", i + 1), mono(T_KEY), rank_c);

        // Capa com a duração no canto.
        let tr = Rect::from_min_size(egui::pos2(rect.left() + 64.0, cy - THUMB.y / 2.0), THUMB);
        p.rect_filled(tr, CornerRadius::same(R_SM), RAISED);
        match self.thumbs.get(&sh.bk, ui.ctx(), &v.id) {
            Some(t) => {
                let [tw, th] = t.size();
                let sc = (tr.width() / tw as f32).max(tr.height() / th as f32);
                let img = egui::Image::new((t.id(), egui::vec2(tw as f32 * sc, th as f32 * sc))).corner_radius(CornerRadius::same(R_SM));
                img.paint_at(ui, Rect::from_center_size(tr.center(), tr.size()));
            }
            None => {
                p.text(tr.center(), Align2::CENTER_CENTER, ic::IMAGE, body(ICON), TEXT_3);
            }
        }
        if let Some(d) = v.duration.filter(|d| *d > 0.0) {
            let g = p.layout_no_wrap(prospect::duration(d), mono(T_META), TEXT);
            let br = Rect::from_min_size(tr.right_bottom() - g.size() - egui::vec2(12.0, 8.0), g.size() + egui::vec2(8.0, 4.0));
            p.rect_filled(br, CornerRadius::same(4), Color32::from_black_alpha(200));
            p.galley(br.min + egui::vec2(4.0, 2.0), g, TEXT);
        }
        let tresp = ui.interact(tr, egui::Id::unique(("pros-thumb", &v.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Abrir no YouTube");
        if tresp.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(&v.url));
        }

        // Ações e visualizações à direita.
        let actions_w = 32.0 * 3.0 + 4.0 * 2.0;
        let ar = Rect::from_min_size(egui::pos2(rect.right() - 12.0 - actions_w, cy - 16.0), egui::vec2(actions_w, 32.0));
        // Filho solto: não mexe no cursor da lista.
        {
            let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(ar).layout(egui::Layout::left_to_right(egui::Align::Center)));
            ui.spacing_mut().item_spacing.x = 4.0;
            if w::icon_button(ui, ic::SCISSORS, "Criar clipes deste vídeo").clicked() {
                sh.clip_url = Some(v.url.clone());
                sh.goto = Some(Page::Clipes);
            }
            if w::icon_button(ui, ic::COPY, "Copiar link").clicked() {
                ui.ctx().copy_text(v.url.clone());
                sh.toasts.ok("Link copiado");
            }
            if w::icon_button(ui, ic::EXTERNAL_LINK, "Abrir no YouTube").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&v.url));
            }
        }
        let vw = 128.0;
        let vx = ar.left() - 24.0;
        p.text(egui::pos2(vx, cy - 20.0), Align2::RIGHT_CENTER, prospect::compact(v.views), mono(T_KEY), TEXT);
        p.text(egui::pos2(vx, cy + 2.0), Align2::RIGHT_CENTER, "visualizações", body(T_META), TEXT_3);
        // Barra: participação em relação ao primeiro; só o líder em cobalto.
        let bar = Rect::from_min_size(egui::pos2(vx - vw, cy + 16.0), egui::vec2(vw, 4.0));
        p.rect_filled(bar, CornerRadius::same(2), RAISED);
        let frac = (v.views as f32 / top as f32).clamp(0.0, 1.0);
        if frac > 0.0 {
            let mut fill = bar;
            fill.set_left(bar.right() - (vw * frac).max(4.0));
            p.rect_filled(fill, CornerRadius::same(2), if i == 0 { ACCENT } else { TEXT_3 });
        }

        // Título (até duas linhas), canal e data.
        let tx = tr.right() + 16.0;
        let tw = (vx - vw - 24.0 - tx).max(80.0);
        let mut job = LayoutJob::single_section(clean(&v.title), TextFormat { font_id: strong(T_BODY), color: TEXT, ..Default::default() });
        job.wrap.max_width = tw;
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        let title = ui.fonts_mut(|f| f.layout_job(job));
        let channel = p.layout(clean(&v.channel), body(T_SEC), TEXT_2, f32::INFINITY);
        let meta = v.published.map(prospect::ago).unwrap_or_default();
        let block_h = title.size().y + 4.0 + channel.size().y + if meta.is_empty() { 0.0 } else { 4.0 + 16.0 };
        let mut y = cy - block_h / 2.0;
        let title_h = title.size().y;
        let title_rect = Rect::from_min_size(egui::pos2(tx, y), title.size());
        p.galley(egui::pos2(tx, y), title, TEXT);
        y += title_h + 4.0;
        p.with_clip_rect(Rect::from_min_size(egui::pos2(tx, y), egui::vec2(tw, 20.0))).galley(egui::pos2(tx, y), channel, TEXT_2);
        y += 20.0;
        if !meta.is_empty() {
            p.text(egui::pos2(tx, y + 4.0), Align2::LEFT_TOP, meta, mono(T_META), TEXT_3);
        }
        let ttl = ui.interact(title_rect, egui::Id::unique(("pros-title", &v.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if ttl.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(&v.url));
        }
        if resp.hovered() || tresp.hovered() || ttl.hovered() {
            p.rect_stroke(tr, CornerRadius::same(R_SM), Stroke::new(1.0, LINE_2), StrokeKind::Inside);
        }
    }
}

fn news(ui: &mut Ui, res: &Results) {
    let meta = if res.news.is_empty() { String::new() } else { "Google Notícias".to_string() };
    w::card(ui, "Notícias", &meta, |_| {}, |ui| {
        if res.news.is_empty() {
            w::empty(ui, ic::NEWSPAPER, &format!("Nenhuma notícia {}", res.period.phrase()));
            return;
        }
        let width = ui.available_width();
        for (i, a) in res.news.iter().enumerate() {
            if i > 0 {
                w::line(ui);
            }
            let mut job = LayoutJob::single_section(clean(&a.title), TextFormat { font_id: body(T_SEC), color: TEXT, ..Default::default() });
            job.wrap.max_width = width - 40.0;
            job.wrap.max_rows = 3;
            job.wrap.overflow_character = Some('…');
            let g = ui.fonts_mut(|f| f.layout_job(job));
            let h = 12.0 + 16.0 + 4.0 + g.size().y + 12.0;
            let (rect, r) = ui.allocate_exact_size(egui::vec2(width, h.ceil()), Sense::click());
            let p = ui.painter();
            if r.hovered() {
                p.rect_filled(rect.shrink(1.0), CornerRadius::ZERO, HOVER);
            }
            let when = a.published.map(prospect::ago).unwrap_or_default();
            let src = if when.is_empty() { clean(&a.source) } else { format!("{} · {when}", clean(&a.source)) };
            p.text(rect.left_top() + egui::vec2(20.0, 12.0), Align2::LEFT_TOP, src, mono(T_META), TEXT_3);
            p.galley(rect.left_top() + egui::vec2(20.0, 32.0), g, TEXT);
            if r.hovered() {
                p.text(rect.right_top() + egui::vec2(-20.0, 12.0), Align2::RIGHT_TOP, ic::ARROW_UP_RIGHT, body(ICON), TEXT_2);
            }
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&a.link));
            }
        }
    });
}
