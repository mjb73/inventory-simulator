//! Visual language: four glass-like palettes, one shared type scale.

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    Style, TextStyle, Visuals,
};
use std::sync::Arc;

const CHAKRA_SEMI: &[u8] = include_bytes!("../assets/fonts/ChakraPetch-SemiBold.ttf");
const CHAKRA_MED: &[u8] = include_bytes!("../assets/fonts/ChakraPetch-Medium.ttf");
const JBM_REG: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const JBM_MED: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");

pub const HEADING: &str = "heading";
pub const BODY: &str = "body";
pub const MONO: &str = "numeric";

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_soft: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub input_bg: Color32,
    pub on_accent: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const THEMES: [Theme; 4] = [
    // Midnight — deep blue with a cyan signal colour.
    Theme {
        name: "Midnight",
        dark: true,
        bg: c(0x0A, 0x0E, 0x16),
        panel: c(0x0E, 0x14, 0x1E),
        card: c(0x13, 0x1A, 0x26),
        card_soft: c(0x18, 0x21, 0x2F),
        stroke: c(0x24, 0x31, 0x45),
        text: c(0xE7, 0xEE, 0xF8),
        dim: c(0x8C, 0xA0, 0xBB),
        faint: c(0x5E, 0x70, 0x8C),
        accent: c(0x4C, 0xC2, 0xFF),
        accent2: c(0x9A, 0x86, 0xFF),
        good: c(0x45, 0xDF, 0x9F),
        warn: c(0xF5, 0xB5, 0x36),
        bad: c(0xFF, 0x7A, 0x8A),
        input_bg: c(0x0B, 0x11, 0x1A),
        on_accent: c(0x05, 0x14, 0x1F),
    },
    // Graphite — neutral charcoal with amber.
    Theme {
        name: "Graphite",
        dark: true,
        bg: c(0x0D, 0x0D, 0x0F),
        panel: c(0x12, 0x12, 0x14),
        card: c(0x18, 0x18, 0x1B),
        card_soft: c(0x1E, 0x1E, 0x22),
        stroke: c(0x2C, 0x2C, 0x31),
        text: c(0xEE, 0xEC, 0xE8),
        dim: c(0xA0, 0x9C, 0x96),
        faint: c(0x6C, 0x68, 0x63),
        accent: c(0xF0, 0xB4, 0x29),
        accent2: c(0xE2, 0x7D, 0x4C),
        good: c(0x8F, 0xD1, 0x7A),
        warn: c(0xF0, 0xB4, 0x29),
        bad: c(0xE8, 0x6B, 0x6B),
        input_bg: c(0x0A, 0x0A, 0x0B),
        on_accent: c(0x1A, 0x12, 0x00),
    },
    // Deep Sea — teal and violet.
    Theme {
        name: "Deep Sea",
        dark: true,
        bg: c(0x06, 0x12, 0x15),
        panel: c(0x09, 0x19, 0x1D),
        card: c(0x0D, 0x21, 0x25),
        card_soft: c(0x11, 0x29, 0x2D),
        stroke: c(0x1B, 0x3A, 0x40),
        text: c(0xE2, 0xF4, 0xF1),
        dim: c(0x84, 0xA9, 0xA3),
        faint: c(0x55, 0x77, 0x72),
        accent: c(0x2D, 0xD4, 0xBF),
        accent2: c(0xA7, 0x8B, 0xFA),
        good: c(0x5B, 0xE0, 0xA0),
        warn: c(0xF2, 0xC0, 0x4E),
        bad: c(0xF8, 0x7C, 0x7C),
        input_bg: c(0x05, 0x0F, 0x11),
        on_accent: c(0x02, 0x15, 0x12),
    },
    // Daylight — light paper with blue.
    Theme {
        name: "Daylight",
        dark: false,
        bg: c(0xF2, 0xF4, 0xF8),
        panel: c(0xF7, 0xF9, 0xFC),
        card: c(0xFF, 0xFF, 0xFF),
        card_soft: c(0xF1, 0xF4, 0xF9),
        stroke: c(0xD7, 0xDF, 0xEA),
        text: c(0x16, 0x20, 0x2E),
        dim: c(0x5D, 0x6B, 0x7F),
        faint: c(0x93, 0xA0, 0xB2),
        accent: c(0x1F, 0x63, 0xEB),
        accent2: c(0x7C, 0x3A, 0xED),
        good: c(0x15, 0x9E, 0x62),
        warn: c(0xC2, 0x81, 0x0A),
        bad: c(0xD6, 0x45, 0x45),
        input_bg: c(0xEC, 0xF0, 0xF6),
        on_accent: c(0xFF, 0xFF, 0xFF),
    },
];

pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "chakra_semi".to_owned(),
        Arc::new(FontData::from_static(CHAKRA_SEMI)),
    );
    fonts.font_data.insert(
        "chakra_med".to_owned(),
        Arc::new(FontData::from_static(CHAKRA_MED)),
    );
    fonts.font_data.insert(
        "jbm_reg".to_owned(),
        Arc::new(FontData::from_static(JBM_REG)),
    );
    fonts.font_data.insert(
        "jbm_med".to_owned(),
        Arc::new(FontData::from_static(JBM_MED)),
    );

    let heading = FontFamily::Name(HEADING.into());
    let body = FontFamily::Name(BODY.into());
    let numeric = FontFamily::Name(MONO.into());

    fonts
        .families
        .insert(heading, vec!["chakra_semi".to_owned(), "chakra_med".to_owned()]);
    // Body keeps the default proportional face at the back for symbols/emoji.
    let mut body_list = vec!["chakra_med".to_owned()];
    if let Some(fallback) = fonts.families.get(&FontFamily::Proportional) {
        body_list.extend(fallback.iter().cloned());
    }
    fonts.families.insert(body, body_list);

    let mut mono_list = vec!["jbm_reg".to_owned(), "jbm_med".to_owned()];
    if let Some(fallback) = fonts.families.get(&FontFamily::Monospace) {
        mono_list.extend(fallback.iter().cloned());
    }
    fonts.families.insert(numeric, mono_list.clone());
    fonts.families.insert(FontFamily::Monospace, mono_list);
    let mut prop = vec!["chakra_med".to_owned()];
    if let Some(fallback) = fonts.families.get(&FontFamily::Proportional) {
        prop.extend(fallback.iter().cloned());
    }
    fonts.families.insert(FontFamily::Proportional, prop);

    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &Context, theme: &Theme) {
    let mut style = Style::default();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Name(HEADING.into())),
        ),
        (
            TextStyle::Body,
            FontId::new(13.5, FontFamily::Name(BODY.into())),
        ),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Name(HEADING.into())),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Name(BODY.into())),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Name(MONO.into())),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.floating = true;
    style.animation_time = 0.08;

    let mut v = if theme.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = theme.bg;
    v.window_fill = theme.panel;
    v.window_stroke = Stroke::new(1.0, theme.stroke);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(if theme.dark { 90 } else { 30 }),
    };
    v.popup_shadow = v.window_shadow;
    v.menu_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = theme.input_bg;
    v.faint_bg_color = theme.card_soft;
    v.override_text_color = Some(theme.text);
    v.selection.bg_fill = theme.accent.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, theme.accent);
    v.hyperlink_color = theme.accent;
    v.slider_trailing_fill = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = theme.card;
    w.noninteractive.weak_bg_fill = theme.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, theme.stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, theme.text);
    w.noninteractive.corner_radius = CornerRadius::same(10);

    w.inactive.bg_fill = theme.card_soft;
    w.inactive.weak_bg_fill = theme.card_soft;
    w.inactive.bg_stroke = Stroke::new(1.0, theme.stroke);
    w.inactive.fg_stroke = Stroke::new(1.0, theme.text);
    w.inactive.corner_radius = CornerRadius::same(9);

    w.hovered.bg_fill = theme.accent.gamma_multiply(0.16);
    w.hovered.weak_bg_fill = theme.accent.gamma_multiply(0.16);
    w.hovered.bg_stroke = Stroke::new(1.0, theme.accent.gamma_multiply(0.55));
    w.hovered.fg_stroke = Stroke::new(1.0, theme.text);
    w.hovered.corner_radius = CornerRadius::same(9);

    w.active.bg_fill = theme.accent.gamma_multiply(0.26);
    w.active.weak_bg_fill = theme.accent.gamma_multiply(0.26);
    w.active.bg_stroke = Stroke::new(1.0, theme.accent);
    w.active.fg_stroke = Stroke::new(1.0, theme.text);
    w.active.corner_radius = CornerRadius::same(9);

    w.open.bg_fill = theme.card_soft;
    w.open.weak_bg_fill = theme.card_soft;
    w.open.bg_stroke = Stroke::new(1.0, theme.stroke);
    w.open.fg_stroke = Stroke::new(1.0, theme.text);
    w.open.corner_radius = CornerRadius::same(9);

    style.visuals = v;
    ctx.set_theme(if theme.dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

pub fn heading(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(HEADING.into()))
}

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BODY.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MONO.into()))
}
