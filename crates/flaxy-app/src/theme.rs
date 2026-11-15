//! Paleta, fontes e estilo global. Direção aprovada no `agente.md`:
//! industrial, exato, noturno. Cinza + azul escuro, sem brilho nem gradiente.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle, Visuals,
};
use std::sync::Arc;

// Rampa grafite (tokens do unslop para apps): superfícies neutras, a cor fica
// no conteúdo (timeline, prévia 9:16) e num único acento cobalto.
pub const SIDE: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x10);
pub const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x15);
pub const BG_DEEP: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0d);
pub const CARD: Color32 = Color32::from_rgb(0x19, 0x19, 0x1b);
pub const RAISED: Color32 = Color32::from_rgb(0x23, 0x23, 0x25);
pub const RAISED_2: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2f);
/// Hover de linha: meio passo entre o fundo e o elevado.
pub const HOVER: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x20);
pub const SELECT: Color32 = Color32::from_rgb(0x16, 0x21, 0x3d);
pub const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x63, 0xd6);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x86, 0xa3, 0xf0);
pub const TEXT: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
pub const TEXT_2: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xa6);
pub const TEXT_3: Color32 = Color32::from_rgb(0x85, 0x85, 0x8b);
/// Botão principal no escuro: off-white da rampa com texto escuro.
pub const PRIMARY: Color32 = TEXT;
pub const PRIMARY_HOVER: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const ON_PRIMARY: Color32 = Color32::from_rgb(0x14, 0x14, 0x15);
/// Linhas finas: texto primário a 8% e 12%.
pub const LINE: Color32 = Color32::from_rgba_premultiplied(19, 19, 19, 20);
pub const LINE_2: Color32 = Color32::from_rgba_premultiplied(28, 28, 28, 31);
/// Brilho de 1px no topo de superfícies ativas.
pub const HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);
pub const OK: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x8a);
pub const ERR: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const SHADE: Color32 = Color32::from_rgba_premultiplied(4, 4, 5, 168);

pub const R_SM: u8 = 6;
pub const R: u8 = 8;
pub const R_LG: u8 = 12;
pub const R_WIN: u8 = 16;

/// Escala de tipo: meta, secundário, corpo, logo, número de destaque, título.
pub const T_META: f32 = 12.0;
pub const T_SEC: f32 = 13.0;
pub const T_BODY: f32 = 14.0;
pub const T_LOGO: f32 = 16.0;
pub const T_KEY: f32 = 20.0;
pub const T_FIG: f32 = 28.0;

/// Tamanho único dos ícones.
pub const ICON: f32 = 16.0;

/// Família com peso médio (títulos e botões).
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn strong(size: f32) -> FontId {
    FontId::new(size, medium())
}

/// Fontes do sistema usadas na prévia de legenda (mesmos nomes do ASS).
/// Nome -> (arquivo normal, arquivo negrito) em C:\Windows\Fonts.
pub const CAPTION_FONTS: &[(&str, &str, &str)] = &[
    ("Arial", "arial.ttf", "arialbd.ttf"),
    ("Arial Black", "ariblk.ttf", "ariblk.ttf"),
    ("Impact", "impact.ttf", "impact.ttf"),
    ("Verdana", "verdana.ttf", "verdanab.ttf"),
    ("Segoe UI", "segoeui.ttf", "segoeuib.ttf"),
    ("Trebuchet MS", "trebuc.ttf", "trebucbd.ttf"),
    ("Georgia", "georgia.ttf", "georgiab.ttf"),
    ("Montserrat", "Montserrat-Regular.ttf", "Montserrat-Bold.ttf"),
    ("Poppins", "Poppins-Regular.ttf", "Poppins-Bold.ttf"),
];

/// Família registrada para uma fonte de legenda, se o arquivo existe.
pub fn caption_family(name: &str, bold: bool) -> FontFamily {
    FontFamily::Name(format!("cap:{name}:{}", if bold { "b" } else { "r" }).into())
}

pub fn install(ctx: &egui::Context) -> Vec<String> {
    let mut fonts = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "geist", include_bytes!("../assets/Geist-Regular.ttf"));
    add(&mut fonts, "geist-medium", include_bytes!("../assets/Geist-Medium.ttf"));
    add(&mut fonts, "geist-mono", include_bytes!("../assets/GeistMono-Regular.ttf"));
    add(&mut fonts, "lucide", include_bytes!("../assets/lucide.ttf"));

    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "geist".into());
    prop.push("lucide".into());
    let monof = fonts.families.entry(FontFamily::Monospace).or_default();
    monof.insert(0, "geist-mono".into());
    monof.push("lucide".into());
    fonts.families.insert(
        medium(),
        vec!["geist-medium".into(), "geist".into(), "lucide".into()],
    );

    // Fontes de legenda do sistema (carregadas uma vez, só as que existem).
    let mut available = Vec::new();
    let dir = std::env::var("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|_| "C:\\Windows".into()).join("Fonts");
    for (name, reg, bold) in CAPTION_FONTS {
        let (Ok(r), b) = (std::fs::read(dir.join(reg)), std::fs::read(dir.join(bold))) else { continue };
        let rk = format!("cap-{name}-r");
        fonts.font_data.insert(rk.clone(), Arc::new(FontData::from_owned(r)));
        let bk = match b {
            Ok(b) if bold != reg => {
                let k = format!("cap-{name}-b");
                fonts.font_data.insert(k.clone(), Arc::new(FontData::from_owned(b)));
                k
            }
            _ => rk.clone(),
        };
        fonts.families.insert(caption_family(name, false), vec![rk, "geist".into()]);
        fonts.families.insert(caption_family(name, true), vec![bk, "geist".into()]);
        available.push(name.to_string());
    }
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, body(12.0)),
            (TextStyle::Body, body(14.0)),
            (TextStyle::Button, body(14.0)),
            (TextStyle::Heading, strong(T_FIG)),
            (TextStyle::Monospace, mono(13.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 8.0);
        s.spacing.interact_size = egui::vec2(32.0, 32.0);
        s.spacing.slider_width = 200.0;
        s.spacing.combo_width = 200.0;
        s.spacing.text_edit_width = 220.0;
        s.spacing.window_margin = Margin::same(16);
        s.spacing.menu_margin = Margin::same(4);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        s.interaction.tooltip_delay = 0.35;

        let mut v = Visuals::dark();
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = BG_DEEP;
        v.faint_bg_color = RAISED;
        v.code_bg_color = RAISED;
        v.window_stroke = Stroke::new(1.0, LINE_2);
        v.window_corner_radius = CornerRadius::same(R_WIN);
        v.menu_corner_radius = CornerRadius::same(R);
        v.window_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) };
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) };
        v.selection.bg_fill = SELECT;
        v.selection.stroke = Stroke::new(1.0, ACCENT_INK);
        v.hyperlink_color = ACCENT_INK;
        v.warn_fg_color = Color32::from_rgb(0xd9, 0xa4, 0x41);
        v.error_fg_color = ERR;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
        w.noninteractive.corner_radius = CornerRadius::same(R);

        w.inactive.bg_fill = RAISED;
        w.inactive.weak_bg_fill = RAISED;
        w.inactive.bg_stroke = Stroke::new(1.0, LINE_2);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = CornerRadius::same(R);

        w.hovered.bg_fill = RAISED_2;
        w.hovered.weak_bg_fill = RAISED_2;
        w.hovered.bg_stroke = Stroke::new(1.0, LINE_2);
        w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.corner_radius = CornerRadius::same(R);
        w.hovered.expansion = 0.0;

        w.active.bg_fill = SELECT;
        w.active.weak_bg_fill = SELECT;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.0, TEXT);
        w.active.corner_radius = CornerRadius::same(R);
        w.active.expansion = 0.0;

        w.open.bg_fill = RAISED_2;
        w.open.weak_bg_fill = RAISED_2;
        w.open.bg_stroke = Stroke::new(1.0, LINE_2);
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = CornerRadius::same(R);
        s.visuals = v;
    });
    available
}
