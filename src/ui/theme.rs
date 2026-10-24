//! Visual identity: fonts, icon glyphs and the dark/light palettes derived
//! from the Trunq logo (deep navy + cyan→blue gradient).

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use parking_lot::RwLock;
use std::sync::Arc;

use crate::config::ThemeChoice;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg0: Color32,
    pub bg1: Color32,
    pub bg2: Color32,
    pub bg3: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    /// Resting fill of buttons and segmented controls.
    pub control: Color32,
    pub control_hover: Color32,
    /// Fill of text inputs.
    pub input: Color32,
    /// The one primary-action colour (from the logo gradient).
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_pressed: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub sel: Color32,
    pub row_alt: Color32,
    // syntax
    pub kw: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub ident: Color32,
    pub qident: Color32,
    pub op: Color32,
    pub param: Color32,
    pub null: Color32,
    pub bool_c: Color32,
    pub json: Color32,
}

const fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub const DARK: Palette = Palette {
    dark: true,
    bg0: hex(0x0A1017),
    bg1: hex(0x0F161F),
    bg2: hex(0x141C26),
    bg3: hex(0x1B2531),
    border: hex(0x1F2A37),
    border_strong: hex(0x2C3949),
    control: hex(0x151E29),
    control_hover: hex(0x1C2734),
    input: hex(0x0C131B),
    primary: hex(0x1668F2),
    primary_hover: hex(0x2C7BFF),
    primary_pressed: hex(0x0F55CF),
    text: hex(0xE4EBF3),
    dim: hex(0x95A5B8),
    faint: hex(0x5E6F84),
    accent: hex(0x2BBDF2),
    accent_soft: Color32::from_rgba_premultiplied(6, 28, 36, 38),
    success: hex(0x3DD68C),
    warn: hex(0xF5B84B),
    danger: hex(0xF2555A),
    sel: Color32::from_rgba_premultiplied(11, 36, 61, 64),
    row_alt: Color32::from_rgba_premultiplied(3, 3, 4, 4),
    kw: hex(0x5BB8FF),
    string: hex(0x9ECE6A),
    number: hex(0xFF9E64),
    comment: hex(0x5F7590),
    ident: hex(0xE4EBF3),
    qident: hex(0xC3A6FF),
    op: hex(0x89DDFF),
    param: hex(0xF7768E),
    null: hex(0x55667A),
    bool_c: hex(0xE0AF68),
    json: hex(0x7DCFFF),
};

pub const LIGHT: Palette = Palette {
    dark: false,
    bg0: hex(0xFFFFFF),
    bg1: hex(0xF6F8FB),
    bg2: hex(0xEEF2F7),
    bg3: hex(0xE4EAF1),
    border: hex(0xDCE3EB),
    border_strong: hex(0xC5CFDB),
    control: hex(0xFFFFFF),
    control_hover: hex(0xF0F4F9),
    input: hex(0xFFFFFF),
    primary: hex(0x0B63E5),
    primary_hover: hex(0x1F74F5),
    primary_pressed: hex(0x0852C4),
    text: hex(0x17202B),
    dim: hex(0x536274),
    faint: hex(0x8A97A8),
    accent: hex(0x0A84D6),
    accent_soft: Color32::from_rgba_premultiplied(1, 16, 25, 30),
    success: hex(0x1A9A5C),
    warn: hex(0xB7791F),
    danger: hex(0xD03A3F),
    sel: Color32::from_rgba_premultiplied(6, 21, 44, 46),
    row_alt: Color32::from_rgba_premultiplied(0, 1, 2, 5),
    kw: hex(0x0B5FC2),
    string: hex(0x2E7D32),
    number: hex(0xC2410C),
    comment: hex(0x8090A4),
    ident: hex(0x17202B),
    qident: hex(0x7C3AED),
    op: hex(0x0E7490),
    param: hex(0xBE185D),
    null: hex(0xA0AEC0),
    bool_c: hex(0xA16207),
    json: hex(0x0369A1),
};

static CURRENT: RwLock<Palette> = RwLock::new(DARK);

/// The active palette.
pub fn p() -> Palette {
    *CURRENT.read()
}

pub const BOLD: &str = "bold";
pub const MONO_BOLD: &str = "mono-bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "Inter", include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "Inter-SemiBold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"));
    add(&mut fonts, "JetBrainsMono", include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"));
    add(&mut fonts, "JetBrainsMono-Bold", include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf"));
    fonts.font_data.insert(
        "lucide".to_owned(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/lucide.ttf")).tweak(egui::FontTweak {
            scale: 1.0,
            y_offset_factor: 0.07,
            ..Default::default()
        })),
    );
    // Lucide goes first: it only has Private-Use-Area glyphs, while Inter and
    // JetBrains Mono also define (different) glyphs in that range.
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "Inter".into());
    prop.insert(0, "lucide".into());
    let monof = fonts.families.entry(FontFamily::Monospace).or_default();
    monof.insert(0, "JetBrainsMono".into());
    monof.insert(0, "lucide".into());
    let fallback_prop: Vec<String> = fonts.families[&FontFamily::Proportional].iter().skip(2).cloned().collect();
    let fallback_mono: Vec<String> = fonts.families[&FontFamily::Monospace].iter().skip(2).cloned().collect();
    let mut b = vec!["lucide".to_string(), "Inter-SemiBold".to_string()];
    b.extend(fallback_prop);
    fonts.families.insert(FontFamily::Name(BOLD.into()), b);
    let mut mb = vec!["lucide".to_string(), "JetBrainsMono-Bold".to_string()];
    mb.extend(fallback_mono);
    fonts.families.insert(FontFamily::Name(MONO_BOLD.into()), mb);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    let pal = match choice {
        ThemeChoice::Dark => DARK,
        ThemeChoice::Light => LIGHT,
    };
    *CURRENT.write() = pal;
    let mut v = if pal.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = pal.bg1;
    v.window_fill = pal.bg1;
    v.extreme_bg_color = pal.input;
    v.text_edit_bg_color = Some(pal.input);
    v.faint_bg_color = pal.bg2;
    v.code_bg_color = pal.bg2;
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.hyperlink_color = pal.accent;
    v.selection.bg_fill = pal.sel;
    v.selection.stroke = Stroke::new(1.0, pal.text);
    v.warn_fg_color = pal.warn;
    v.error_fg_color = pal.danger;
    v.override_text_color = Some(pal.text);
    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = pal.bg1;
    v.widgets.noninteractive.weak_bg_fill = pal.bg1;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = pal.control;
    v.widgets.inactive.weak_bg_fill = pal.control;
    // quiet resting state: no hard outline, slightly raised fill
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = pal.control_hover;
    v.widgets.hovered.weak_bg_fill = pal.control_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = pal.accent_soft;
    v.widgets.active.weak_bg_fill = pal.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.hovered;
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(if pal.dark { 120 } else { 40 }) };
    v.window_shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(if pal.dark { 140 } else { 50 }) };
    v.striped = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    ctx.set_theme(if pal.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.menu_spacing = 4.0;
        s.spacing.combo_height = 320.0;
        s.spacing.icon_spacing = 6.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.slider_width = 220.0;
        s.spacing.slider_rail_height = 5.0;
        s.spacing.window_margin = egui::Margin::same(14);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(13.5));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        s.text_styles.insert(TextStyle::Heading, bold(19.0));
        s.interaction.tooltip_delay = 0.35;
    });
}

/// Lucide icon glyphs (Private Use Area code points of the bundled font).
#[allow(dead_code)]
pub mod icon {
    pub const ACTIVITY: &str = "\u{e038}";
    pub const ALIGN_LEFT: &str = "\u{e041}";
    pub const ARROW_DOWN: &str = "\u{e046}";
    pub const ARROW_UP: &str = "\u{e04e}";
    pub const ARROW_RIGHT: &str = "\u{e04d}";
    pub const SORT_DESC: &str = "\u{e04b}";
    pub const SORT_ASC: &str = "\u{e050}";
    pub const BELL: &str = "\u{e05d}";
    pub const BOOK: &str = "\u{e063}";
    pub const BOX: &str = "\u{e065}";
    pub const BRACES: &str = "\u{e36d}";
    pub const CHART_BAR: &str = "\u{e2a1}";
    pub const CHART_LINE: &str = "\u{e2a4}";
    pub const CHECK: &str = "\u{e070}";
    pub const CHEVRON_DOWN: &str = "\u{e071}";
    pub const CHEVRON_RIGHT: &str = "\u{e073}";
    pub const CHEVRON_LEFT: &str = "\u{e072}";
    pub const CHEVRON_UP: &str = "\u{e074}";
    pub const CIRCLE_CHECK: &str = "\u{e225}";
    pub const CIRCLE_X: &str = "\u{e088}";
    pub const CIRCLE_HELP: &str = "\u{e082}";
    pub const CIRCLE: &str = "\u{e07a}";
    pub const CLOCK: &str = "\u{e08b}";
    pub const CLOUD: &str = "\u{e08c}";
    pub const CODE: &str = "\u{e097}";
    pub const COLUMNS: &str = "\u{e09d}";
    pub const COMMAND: &str = "\u{e09e}";
    pub const COPY: &str = "\u{e0a2}";
    pub const CPU: &str = "\u{e0ad}";
    pub const DATABASE: &str = "\u{e0b1}";
    pub const DOWNLOAD: &str = "\u{e0b6}";
    pub const ELLIPSIS: &str = "\u{e0ba}";
    pub const EXTERNAL: &str = "\u{e0bd}";
    pub const EYE: &str = "\u{e0be}";
    pub const EYE_OFF: &str = "\u{e0bf}";
    pub const FILE_CODE: &str = "\u{e0c7}";
    pub const FILE_PLUS: &str = "\u{e0cd}";
    pub const FILE_TEXT: &str = "\u{e0d0}";
    pub const FILE_JSON: &str = "\u{e36e}";
    pub const FILE_SHEET: &str = "\u{e326}";
    pub const FILTER: &str = "\u{e0d5}";
    pub const FOLDER: &str = "\u{e0dc}";
    pub const FOLDER_OPEN: &str = "\u{e246}";
    pub const GAUGE: &str = "\u{e1be}";
    pub const GLOBE: &str = "\u{e0eb}";
    pub const GRID: &str = "\u{e0ec}";
    pub const HARD_DRIVE: &str = "\u{e0f0}";
    pub const HASH: &str = "\u{e0f2}";
    pub const HISTORY: &str = "\u{e1f4}";
    pub const HOURGLASS: &str = "\u{e295}";
    pub const IMPORT: &str = "\u{e22e}";
    pub const INFO: &str = "\u{e0fe}";
    pub const KEY: &str = "\u{e102}";
    pub const KEYBOARD: &str = "\u{e283}";
    pub const LAYERS: &str = "\u{e52d}";
    pub const LINK: &str = "\u{e107}";
    pub const LIST: &str = "\u{e10b}";
    pub const LIST_TREE: &str = "\u{e40c}";
    pub const LOADER: &str = "\u{e10d}";
    pub const LOCK: &str = "\u{e10e}";
    pub const LOCK_OPEN: &str = "\u{e10f}";
    pub const LOG_IN: &str = "\u{e110}";
    pub const LOG_OUT: &str = "\u{e111}";
    pub const MINUS: &str = "\u{e11f}";
    pub const MOON: &str = "\u{e121}";
    pub const NETWORK: &str = "\u{e128}";
    pub const PANEL_LEFT: &str = "\u{e12d}";
    pub const PENCIL: &str = "\u{e1f8}";
    pub const PIN: &str = "\u{e258}";
    pub const PLAY: &str = "\u{e13f}";
    pub const PLAY_CIRCLE: &str = "\u{e085}";
    pub const PLUG: &str = "\u{e382}";
    pub const PLUS: &str = "\u{e140}";
    pub const POWER: &str = "\u{e143}";
    pub const PUZZLE: &str = "\u{e29b}";
    pub const REFRESH: &str = "\u{e148}";
    pub const ROTATE_CCW: &str = "\u{e14b}";
    pub const ROWS: &str = "\u{e58e}";
    pub const SAVE: &str = "\u{e150}";
    pub const SCROLL: &str = "\u{e463}";
    pub const SEARCH: &str = "\u{e154}";
    pub const SERVER: &str = "\u{e156}";
    pub const SETTINGS: &str = "\u{e157}";
    pub const SHIELD: &str = "\u{e15b}";
    pub const SIGMA: &str = "\u{e200}";
    pub const SPARKLES: &str = "\u{e416}";
    pub const SPLIT: &str = "\u{e444}";
    pub const SQUARE: &str = "\u{e16a}";
    pub const SQUARE_PEN: &str = "\u{e175}";
    pub const STOP: &str = "\u{e087}";
    pub const STAR: &str = "\u{e179}";
    pub const SUN: &str = "\u{e17b}";
    pub const FUNCTION: &str = "\u{e22c}";
    pub const TABLE: &str = "\u{e180}";
    pub const TABLE_PROPS: &str = "\u{e4df}";
    pub const TERMINAL: &str = "\u{e184}";
    pub const TEXT: &str = "\u{e3ee}";
    pub const TIMER: &str = "\u{e1df}";
    pub const TRASH: &str = "\u{e18d}";
    pub const WARN: &str = "\u{e192}";
    pub const TYPE: &str = "\u{e197}";
    pub const UNDO: &str = "\u{e2a0}";
    pub const UNPLUG: &str = "\u{e461}";
    pub const UPLOAD: &str = "\u{e19d}";
    pub const USER: &str = "\u{e19e}";
    pub const USERS: &str = "\u{e1a3}";
    pub const WAND: &str = "\u{e35a}";
    pub const WIFI_OFF: &str = "\u{e1ae}";
    pub const WORKFLOW: &str = "\u{e429}";
    pub const X: &str = "\u{e1b1}";
    pub const ZAP: &str = "\u{e1b3}";
    pub const EXPAND: &str = "\u{e442}";
    pub const CHEVRONS_DOWN: &str = "\u{e075}";
    pub const COLLAPSE: &str = "\u{e440}";
    pub const SLIDERS: &str = "\u{e299}";
    pub const BINARY: &str = "\u{e1f1}";
    pub const CALENDAR: &str = "\u{e067}";
    pub const TOGGLE: &str = "\u{e18a}";
    pub const CLIPBOARD: &str = "\u{e089}";
    pub const BRACKETS: &str = "\u{e447}";
    pub const REPLACE: &str = "\u{e3de}";
    pub const MEMORY: &str = "\u{e449}";
    pub const ERASER: &str = "\u{e28e}";
}

/// Preset connection colors (production = red, staging = orange, ...).
pub const CONN_COLORS: [(u32, &str); 8] = [
    (0x1CB8F2, "Blue"),
    (0x3DD68C, "Green"),
    (0xF5B84B, "Orange"),
    (0xF2555A, "Red"),
    (0xB48CFF, "Purple"),
    (0xFF7EB6, "Pink"),
    (0x2DD4BF, "Teal"),
    (0x94A3B8, "Gray"),
];

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn rgb_hex(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}
