//! Lattice's visual language (see docs/DESIGN.md §11): a technical drawing of a
//! block grid. Surfaces are cyanotype blue or drafting paper, lines carry the
//! structure, and amber (burnt orange on paper) is kept for what you can press.
//! The type is Atkinson Hyperlegible Next, chosen because it stays readable.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Every colour the interface uses, for one theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub ground: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub edge: Color32,
    pub ink: Color32,
    pub ink_muted: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub action: Color32,
    pub on_action: Color32,
    pub action_hover: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub focus: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const CYANOTYPE: Palette = Palette {
    ground: rgb(0x0E2740),
    surface: rgb(0x14324F),
    raised: rgb(0x1D4366),
    hover: rgb(0x26527B),
    edge: rgb(0x7BA3C7),
    ink: rgb(0xF4F7F5),
    ink_muted: rgb(0xBBCDDC),
    accent: rgb(0x8AD4F5),
    accent_dim: rgb(0x1F5480),
    action: rgb(0xFFC247),
    on_action: rgb(0x1E1400),
    action_hover: rgb(0xFFD27A),
    warn: rgb(0xFFC247),
    danger: rgb(0xFFA595),
    focus: rgb(0xFFFFFF),
};

pub const DRAFTING: Palette = Palette {
    ground: rgb(0xE4EBF1),
    surface: rgb(0xF6F9FB),
    raised: rgb(0xD5E0E9),
    hover: rgb(0xC5D4E0),
    edge: rgb(0x5F7C96),
    ink: rgb(0x0C2740),
    ink_muted: rgb(0x3B546B),
    accent: rgb(0x0F5C94),
    accent_dim: rgb(0xC2DBEE),
    action: rgb(0xB34700),
    on_action: rgb(0xFFFFFF),
    action_hover: rgb(0x9E3F00),
    warn: rgb(0x7F4400),
    danger: rgb(0xA8231B),
    focus: rgb(0x0C2740),
};

pub const HIGH_CONTRAST: Palette = Palette {
    ground: rgb(0x000000),
    surface: rgb(0x000000),
    raised: rgb(0x1A1A1A),
    hover: rgb(0x333333),
    edge: rgb(0xFFFFFF),
    ink: rgb(0xFFFFFF),
    ink_muted: rgb(0xE6E6E6),
    accent: rgb(0x66E0FF),
    accent_dim: rgb(0x003A4D),
    action: rgb(0xFFD400),
    on_action: rgb(0x000000),
    action_hover: rgb(0xFFE55C),
    warn: rgb(0xFFD400),
    danger: rgb(0xFF9E94),
    focus: rgb(0xFFFFFF),
};

/// Game windows: weathered blue stone, lighter than the launcher, so panels
/// stand out over a dark world and slots read as cut into them.
pub const STONE: Palette = Palette {
    ground: rgb(0x15283B),
    surface: rgb(0x38566F),
    raised: rgb(0x3F5F7C),
    hover: rgb(0x476A89),
    edge: rgb(0x9DBBD6),
    ink: rgb(0xFFFDF2),
    ink_muted: rgb(0xE0E8EE),
    accent: rgb(0x9BE0FF),
    accent_dim: rgb(0x24506F),
    action: rgb(0xF2B632),
    on_action: rgb(0x241800),
    action_hover: rgb(0xFFCE5C),
    warn: rgb(0xFFD36B),
    danger: rgb(0xFFCFC8),
    focus: rgb(0xFFFFFF),
};

/// Inside a game window's panels: dark words on pale, flecked limestone. The slots cut
/// into it are dark, so what's in them stands out.
pub const LIMESTONE: Palette = Palette {
    ground: rgb(0xDAD2BE),
    surface: rgb(0xC2B9A4),
    raised: rgb(0xD3CBB7),
    hover: rgb(0xE2DBC9),
    edge: rgb(0x5C5344),
    ink: rgb(0x211B11),
    ink_muted: rgb(0x453D2F),
    accent: rgb(0x07433D),
    accent_dim: rgb(0xE6C47A),
    action: rgb(0x8F4414),
    on_action: rgb(0xFFF4E0),
    action_hover: rgb(0xA65218),
    warn: rgb(0x5F3A00),
    danger: rgb(0x7A150C),
    focus: rgb(0x000000),
};

/// What the player chose in settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Dark,
    Light,
    Contrast,
}

static MODE: AtomicU8 = AtomicU8::new(0);
/// Set when the player asked for less motion; animations then snap.
static REDUCE_MOTION: AtomicU8 = AtomicU8::new(0);
/// This window is a game window, whose menus are made of stone (see chrome.rs).
static GAME_LOOK: AtomicU8 = AtomicU8::new(0);
static PIXEL_FONT: AtomicBool = AtomicBool::new(false);
static ON_PANEL: AtomicBool = AtomicBool::new(false);

pub fn game_look() -> bool {
    GAME_LOOK.load(Ordering::Relaxed) != 0
}

pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        1 => Mode::Light,
        2 => Mode::Contrast,
        _ => Mode::Dark,
    }
}

pub fn palette() -> &'static Palette {
    match mode() {
        Mode::Dark if game_look() && ON_PANEL.load(Ordering::Relaxed) => &LIMESTONE,
        Mode::Dark if game_look() => &STONE,
        Mode::Dark => &CYANOTYPE,
        Mode::Light => &DRAFTING,
        Mode::Contrast => &HIGH_CONTRAST,
    }
}

pub fn reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed) != 0
}

pub fn ground() -> Color32 { palette().ground }
pub fn surface() -> Color32 { palette().surface }
pub fn raised() -> Color32 { palette().raised }
pub fn hover() -> Color32 { palette().hover }
/// Edge of controls: fields, switches, slider rails.
pub fn edge() -> Color32 { palette().edge }
pub fn ink() -> Color32 { palette().ink }
pub fn ink_muted() -> Color32 { palette().ink_muted }
/// Lines and selection, the colour of the drawing itself.
pub fn accent() -> Color32 { palette().accent }
pub fn accent_dim() -> Color32 { palette().accent_dim }
/// Fill of the one main action on a screen.
pub fn action() -> Color32 { palette().action }
pub fn on_action() -> Color32 { palette().on_action }
pub fn action_hover() -> Color32 { palette().action_hover }
pub fn warn() -> Color32 { palette().warn }
pub fn danger() -> Color32 { palette().danger }
pub fn focus() -> Color32 { palette().focus }

/// Radius means "you can press this"; panels and rows are square.
pub const CONTROL_RADIUS: u8 = 6;

pub const SIZE_SMALL: f32 = 14.0;
pub const SIZE_BODY: f32 = 16.0;
pub const SIZE_LEAD: f32 = 20.0;
pub const SIZE_TITLE: f32 = 28.0;
pub const SIZE_DISPLAY: f32 = 44.0;

/// Smallest thing the pointer has to hit.
pub const TARGET: f32 = 36.0;

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// The pixel face is only sharp where its pixels land on the screen's, which is at
/// whole multiples of 27 points. Anything between blurs until letters run together.
fn sharp(size: f32) -> f32 {
    if !PIXEL_FONT.load(Ordering::Relaxed) {
        return size;
    }
    match size {
        s if s <= 24.0 => 27.0,
        s if s <= 40.0 => 54.0,
        _ => 81.0,
    }
}

pub fn font(size: f32) -> FontId {
    FontId::new(sharp(size), FontFamily::Proportional)
}

pub fn font_bold(size: f32) -> FontId {
    FontId::new(sharp(size), bold())
}

/// Loads the fonts. Call once, then [`apply`].
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    add_faces(&mut fonts);
    ctx.set_fonts(fonts);
    apply(ctx, Mode::Dark, false);
}

fn add_faces(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        "atkinson".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "atkinson-bold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext-Bold.ttf"))),
    );
    // egui's bundled fonts stay as fallbacks for symbols our face doesn't cover.
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = vec!["atkinson".to_string()];
    regular.extend(fallbacks.iter().cloned());
    let mut boldf = vec!["atkinson-bold".to_string()];
    boldf.extend(fallbacks);
    // The debug overlay asks for monospace; use our face there too.
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "atkinson".to_string());
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(bold(), boldf);
}

/// Turns this window into a game window: stone colours, and the pixel face for
/// everything unless the player asked for the plainer one.
pub fn use_game_look(ctx: &egui::Context, pixel_font: bool) {
    GAME_LOOK.store(1, Ordering::Relaxed);
    PIXEL_FONT.store(pixel_font, Ordering::Relaxed);
    let mut fonts = FontDefinitions::default();
    add_faces(&mut fonts);
    if pixel_font {
        let data = FontData::from_static(include_bytes!("../../assets/fonts/pixel/Jersey15-Regular.ttf"));
        fonts.font_data.insert("pixel".into(), Arc::new(data));
        for family in [FontFamily::Proportional, bold(), FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "pixel".to_string());
        }
    }
    ctx.set_fonts(fonts);
}

/// Works out the theme a window should use. The world behind a game window is
/// always dark, so paper panels would glare there: `in_game` keeps those dark.
pub fn resolve(choice: &str, system_is_light: bool, in_game: bool) -> Mode {
    match choice {
        "high_contrast" => Mode::Contrast,
        "dark" => Mode::Dark,
        "light" if !in_game => Mode::Light,
        "system" if system_is_light && !in_game => Mode::Light,
        _ => Mode::Dark,
    }
}

/// Switches the whole window's look. Cheap enough to call when anything changes.
pub fn apply(ctx: &egui::Context, mode: Mode, reduce: bool) {
    MODE.store(
        match mode {
            Mode::Dark => 0,
            Mode::Light => 1,
            Mode::Contrast => 2,
        },
        Ordering::Relaxed,
    );
    REDUCE_MOTION.store(reduce as u8, Ordering::Relaxed);
    let p = *palette();
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, font(SIZE_SMALL)),
            (TextStyle::Body, font(SIZE_BODY)),
            (TextStyle::Button, font(SIZE_BODY)),
            (TextStyle::Heading, font_bold(SIZE_TITLE)),
            (TextStyle::Monospace, FontId::new(sharp(SIZE_SMALL), FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(16.0, 8.0);
        style.spacing.interact_size = egui::vec2(TARGET, 32.0);
        style.spacing.slider_width = 220.0;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.text_edit_width = 320.0;
        style.spacing.combo_width = 220.0;
        style.interaction.selectable_labels = false;
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 8.0;
        // A fade towards one flat colour shows up against flecked stone.
        if game_look() {
            style.spacing.scroll.fade.strength = 0.0;
        }
        style.animation_time = if reduce { 0.0 } else { 0.08 };

        style.visuals = visuals_for(&p, mode);
    });
}

/// Whether words drawn right now are on a panel.
pub fn panel_mode_now() -> bool {
    ON_PANEL.load(Ordering::Relaxed)
}

pub fn pixel_font() -> bool {
    PIXEL_FONT.load(Ordering::Relaxed)
}

/// What egui's own widgets look like on this palette.
fn visuals_for(p: &Palette, mode: Mode) -> Visuals {
    let mut v = if mode == Mode::Light { Visuals::light() } else { Visuals::dark() };
    v.override_text_color = Some(p.ink);
    v.panel_fill = p.ground;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.ground;
    // Text fields are wells with an edge, visible on any surface.
    v.text_edit_bg_color = Some(if mode == Mode::Light { p.surface } else { p.ground });
    v.faint_bg_color = p.surface;
    v.window_stroke = Stroke::new(1.0, p.edge);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_dim;
    v.selection.stroke = Stroke::new(1.0, p.ink);
    v.text_cursor.stroke = Stroke::new(2.0, p.ink);
    v.error_fg_color = p.danger;
    v.warn_fg_color = p.warn;
    v.slider_trailing_fill = true;
    let r = if game_look() { CornerRadius::ZERO } else { CornerRadius::same(CONTROL_RADIUS) };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.raised);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.edge);
    w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.ink_muted);
    w.hovered.fg_stroke = Stroke::new(1.0, p.ink);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.accent_dim;
    w.active.weak_bg_fill = p.accent_dim;
    w.active.bg_stroke = Stroke::new(2.0, p.focus);
    w.active.fg_stroke = Stroke::new(1.0, p.ink);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.active;
    v
}

/// Runs `add` as part of a panel: dark words on stone instead of light words on the world.
/// Plain and high-contrast looks don't change.
pub fn on_panel<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let was = ON_PANEL.swap(true, Ordering::Relaxed);
    let before = ui.visuals().clone();
    let mode = current_mode();
    *ui.visuals_mut() = visuals_for(palette(), mode);
    let out = add(ui);
    *ui.visuals_mut() = before;
    ON_PANEL.store(was, Ordering::Relaxed);
    out
}

/// Whether words drawn right now would be on a panel.
pub fn panel_mode(on: bool) -> bool {
    ON_PANEL.swap(on, Ordering::Relaxed)
}

fn current_mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        1 => Mode::Light,
        2 => Mode::Contrast,
        _ => Mode::Dark,
    }
}

/// RGB of a text-component colour, as an egui colour.
pub fn component_color(rgb: Option<u32>, default: Color32) -> Color32 {
    match rgb {
        Some(c) => Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8),
        None => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Text needs 4.5:1 (AA), controls and focus rings 3:1, in every theme.
    #[test]
    fn every_theme_meets_wcag_aa() {
        for (name, p) in [("cyanotype", CYANOTYPE), ("drafting", DRAFTING), ("high contrast", HIGH_CONTRAST), ("stone", STONE), ("limestone", LIMESTONE)] {
            let mut failures = Vec::new();
            let mut check = |what: &str, fg: Color32, bg: Color32, need: f32| {
                let got = contrast(fg, bg);
                if got < need {
                    failures.push(format!("{what}: {got:.2} < {need}"));
                }
            };
            for (bg_name, bg) in [("ground", p.ground), ("surface", p.surface), ("raised", p.raised), ("hover", p.hover)] {
                check(&format!("ink on {bg_name}"), p.ink, bg, 4.5);
                check(&format!("muted on {bg_name}"), p.ink_muted, bg, 4.5);
            }
            for (bg_name, bg) in [("ground", p.ground), ("surface", p.surface), ("raised", p.raised)] {
                check(&format!("accent on {bg_name}"), p.accent, bg, 4.5);
                check(&format!("warn on {bg_name}"), p.warn, bg, 4.5);
                check(&format!("danger on {bg_name}"), p.danger, bg, 4.5);
            }
            check("ink on selection", p.ink, p.accent_dim, 4.5);
            check("text on action", p.on_action, p.action, 4.5);
            check("text on hovered action", p.on_action, p.action_hover, 4.5);
            for (bg_name, bg) in [("ground", p.ground), ("surface", p.surface)] {
                check(&format!("action fill on {bg_name}"), p.action, bg, 3.0);
                check(&format!("control edge on {bg_name}"), p.edge, bg, 3.0);
                check(&format!("focus ring on {bg_name}"), p.focus, bg, 3.0);
            }
            assert!(failures.is_empty(), "{name}: {failures:?}");
        }
    }

    #[test]
    fn game_windows_never_use_the_paper_theme() {
        assert_eq!(resolve("light", true, true), Mode::Dark);
        assert_eq!(resolve("system", true, true), Mode::Dark);
        assert_eq!(resolve("light", false, false), Mode::Light);
        assert_eq!(resolve("system", true, false), Mode::Light);
        assert_eq!(resolve("system", false, false), Mode::Dark);
        assert_eq!(resolve("high_contrast", true, true), Mode::Contrast);
    }
}
