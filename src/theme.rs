use std::path::PathBuf;

use eframe::egui::{
    self, epaint::Shadow, style::HandleShape, Color32, FontData, FontDefinitions, FontFamily, FontId,
    Rounding, Stroke, TextStyle, Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0C, 0x0E);
pub const PANEL: Color32 = Color32::from_rgb(0x14, 0x17, 0x1B);
pub const RAIL: Color32 = Color32::from_rgb(0x10, 0x13, 0x17);
pub const RAISED: Color32 = Color32::from_rgb(0x1C, 0x20, 0x26);
pub const ACTIVE: Color32 = Color32::from_rgb(0x1F, 0x24, 0x2B);
pub const HOVER: Color32 = Color32::from_rgb(0x26, 0x2B, 0x33);
pub const LINE: Color32 = Color32::from_rgb(0x22, 0x27, 0x2E);
pub const BORDER: Color32 = Color32::from_rgb(0x2A, 0x2F, 0x37);
pub const TEXT: Color32 = Color32::from_rgb(0xE9, 0xEB, 0xEE);
pub const TEXT2: Color32 = Color32::from_rgb(0xA3, 0xAB, 0xB6);
pub const TEXT3: Color32 = Color32::from_rgb(0x85, 0x8E, 0x9B);
pub const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x3D);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x7A, 0x60, 0x1E);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x17, 0x13, 0x0A);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x7A, 0x59);

// Заливка клипа на дорожке и его рамка: по рамке видно, где кончается один клип и начинается другой.
pub const CLIP_VIDEO: Color32 = Color32::from_rgb(0x24, 0x4E, 0x78);
pub const CLIP_VIDEO_RIM: Color32 = Color32::from_rgb(0x5E, 0x97, 0xCF);
pub const CLIP_IMAGE: Color32 = Color32::from_rgb(0x4A, 0x3A, 0x63);
pub const CLIP_IMAGE_RIM: Color32 = Color32::from_rgb(0x9A, 0x80, 0xC8);
pub const CLIP_AUDIO: Color32 = Color32::from_rgb(0x17, 0x49, 0x3F);
pub const CLIP_AUDIO_RIM: Color32 = Color32::from_rgb(0x3F, 0xA5, 0x8C);
pub const CLIP_EFFECT: Color32 = Color32::from_rgb(0x6B, 0x3A, 0x1C);
pub const CLIP_EFFECT_RIM: Color32 = Color32::from_rgb(0xD9, 0x8A, 0x45);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(0x6A, 0x2B, 0x4F);
pub const CLIP_TEXT_RIM: Color32 = Color32::from_rgb(0xD8, 0x6F, 0xA9);
pub const WAVE: Color32 = Color32::from_rgb(0x58, 0xC7, 0xAC);
pub const CLIP_LABEL_BG: Color32 = Color32::from_rgba_premultiplied(6, 7, 9, 158);

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub fn font_mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Подключает системные шрифты с кириллицей; если их нет, остаются встроенные.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR")
        .map(|w| PathBuf::from(w).join("Fonts"))
        .unwrap_or_else(|_| PathBuf::from("C:\\Windows\\Fonts"));

    let mut load = |name: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert(name.to_owned(), FontData::from_owned(bytes));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("ui", "segoeui.ttf");
    let semi = load("ui-semibold", "seguisb.ttf");
    let mono = load("ui-mono", "consola.ttf");

    if regular {
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "ui".to_owned());
    }
    if mono {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "ui-mono".to_owned());
    }
    let mut semi_list = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if semi {
        semi_list.insert(0, "ui-semibold".to_owned());
    }
    fonts.families.insert(semibold(), semi_list);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(8.0);
    v.extreme_bg_color = RAISED;
    v.faint_bg_color = RAISED;
    v.slider_trailing_fill = true;
    v.handle_shape = HandleShape::Circle;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.window_shadow = Shadow { offset: egui::vec2(0.0, 16.0), blur: 36.0, spread: 0.0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = Shadow { offset: egui::vec2(0.0, 8.0), blur: 20.0, spread: 0.0, color: Color32::from_black_alpha(120) };

    let r = Rounding::same(6.0);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, TEXT3);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.rounding = r;

    v.widgets.active.bg_fill = HOVER;
    v.widgets.active.weak_bg_fill = HOVER;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.rounding = r;
    // Под нажатием штатные элементы чуть проседают, а не раздуваются.
    v.widgets.active.expansion = -1.0;

    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.text_styles.insert(TextStyle::Body, font(13.0));
        s.text_styles.insert(TextStyle::Button, font(13.0));
        s.text_styles.insert(TextStyle::Small, font(11.5));
        s.text_styles.insert(TextStyle::Heading, font_semibold(16.0));
        s.text_styles.insert(TextStyle::Monospace, font_mono(12.0));
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.slider_width = 120.0;
    });
}
