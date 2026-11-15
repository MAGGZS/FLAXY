//! Prévia animada da legenda num quadro 9:16, com as fontes reais do Windows.
//! Aproximação fiel o bastante para escolher o estilo; o render usa libass.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Ui};
use flaxy_core::captions::{CaptionStyle, Kind};

use crate::theme::*;

const SAMPLE: [&str; 7] = ["Isso", "aqui", "muda", "tudo", "na", "sua", "página"];

pub fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0xffffff);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn preview(ui: &mut Ui, st: &CaptionStyle, height: f32, fonts: &[String]) {
    let width = height * 9.0 / 16.0;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let rect = Rect::from_center_size(outer.center(), egui::vec2(width, height));
    ui.painter().rect_filled(rect, CornerRadius::same(R), RAISED);
    let t = ui.input(|i| i.time);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    paint(&ui.painter_at(rect), rect, st, fonts, t);
}

/// Desenha a legenda animada dentro de `rect` (o quadro de saída inteiro).
pub fn paint(p: &egui::Painter, rect: Rect, st: &CaptionStyle, fonts: &[String], t: f64) {
    let height = rect.height();
    let width = rect.width();
    let tick = (t / 0.42) as usize;
    let per = match st.kind {
        Kind::Word => 1,
        Kind::Classic => st.max_words.clamp(1, 7),
        _ => st.max_words.max(1),
    };
    let idx = tick % SAMPLE.len();
    let start = idx / per * per;
    let chunk: Vec<String> = SAMPLE[start..(start + per).min(SAMPLE.len())]
        .iter()
        .map(|w| if st.uppercase { w.to_uppercase() } else { w.to_string() })
        .collect();
    let active = idx - start;

    let scale = height / 1920.0;
    let family = if fonts.iter().any(|f| f == &st.font) { caption_family(&st.font, st.bold) } else { if st.bold { medium() } else { egui::FontFamily::Proportional } };
    let mut size = (st.size as f32 * scale).max(6.0);
    if st.kind == Kind::Word {
        // "pop": cresce um pouco no começo de cada palavra.
        let phase = ((t / 0.42).fract() as f32 * 3.0).min(1.0);
        size *= 0.85 + 0.15 * phase;
    }
    let font = FontId::new(size, family);
    let space = p.layout_no_wrap(" ".into(), font.clone(), Color32::WHITE).size().x;
    let galleys: Vec<_> = chunk.iter().map(|w| p.layout_no_wrap(w.clone(), font.clone(), Color32::WHITE)).collect();

    // Quebra gulosa pela largura útil (88% do quadro).
    let max_w = width * 0.88;
    let mut lines: Vec<Vec<usize>> = vec![vec![]];
    let mut lw = 0.0;
    for (i, g) in galleys.iter().enumerate() {
        let add = g.size().x + if lines.last().unwrap().is_empty() { 0.0 } else { space };
        if lw + add > max_w && !lines.last().unwrap().is_empty() {
            lines.push(vec![]);
            lw = 0.0;
        }
        lw += g.size().x + if lines.last().unwrap().is_empty() { 0.0 } else { space };
        lines.last_mut().unwrap().push(i);
    }
    let line_h = size * 1.15;
    let block_h = line_h * lines.len() as f32;
    let top = match st.position.as_str() {
        "top" => rect.top() + height * st.margin as f32,
        "center" => rect.center().y - block_h / 2.0,
        _ => rect.bottom() - height * st.margin as f32 - block_h,
    };

    let color = hex(&st.color);
    let hl = hex(&st.highlight);
    let outline_c = hex(&st.outline_color);
    let stroke = (st.outline as f32 * scale).max(0.0);
    for (li, idxs) in lines.iter().enumerate() {
        let total: f32 = idxs.iter().map(|&i| galleys[i].size().x).sum::<f32>() + space * (idxs.len().saturating_sub(1)) as f32;
        let mut x = rect.center().x - total / 2.0;
        let y = top + li as f32 * line_h;
        if st.kind == Kind::Classic {
            let back = hex(&st.back_color);
            let a = (st.back_opacity.clamp(0.0, 1.0) * 255.0) as u8;
            let pad = egui::vec2(size * 0.25, size * 0.08);
            let r = Rect::from_min_size(egui::pos2(x, y), egui::vec2(total, line_h)).expand2(pad);
            p.rect_filled(r, CornerRadius::same(2), Color32::from_rgba_unmultiplied(back.r(), back.g(), back.b(), a));
        }
        for &i in idxs {
            let g = &galleys[i];
            let on = i == active;
            let pos = egui::pos2(x, y + (line_h - g.size().y) / 2.0);
            let mut fill = color;
            match st.kind {
                Kind::Highlight if on => fill = hl,
                Kind::Box if on => {
                    let r = Rect::from_min_size(pos, g.size()).expand2(egui::vec2(size * 0.18, 0.0));
                    p.rect_filled(r, CornerRadius::same(3), hl);
                }
                _ => {}
            }
            let boxed = st.kind == Kind::Box && on;
            if stroke > 0.0 && !boxed {
                let steps = 12;
                for k in 0..steps {
                    let a = k as f32 / steps as f32 * std::f32::consts::TAU;
                    p.galley_with_override_text_color(pos + egui::vec2(a.cos(), a.sin()) * stroke, g.clone(), outline_c);
                }
            }
            p.galley_with_override_text_color(pos, g.clone(), fill);
            x += g.size().x + space;
        }
    }
    let _ = Align2::CENTER_CENTER;
}
