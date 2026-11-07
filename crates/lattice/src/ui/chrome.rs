//! What a game window's menus are made of: flecked stone with lit and shaded edges,
//! dark slots cut into it, and keys that sink when pressed. Everything sits on one
//! grid, `unit()`, the size of one of an item picture's own pixels, so icons stay
//! crisp and hard edges stay hard at any window size. The launcher doesn't use
//! this: it's an ordinary desktop app.

use super::theme::*;
use egui::{Align2, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shape, TextureOptions, Vec2, pos2, vec2};
use std::sync::atomic::{AtomicU32, Ordering};

/// What's written on the dark box of a tooltip, and on slots.
pub const TIP_INK: Color32 = hex(0xFFFDF2);
pub const TIP_MUTED: Color32 = hex(0xD9D2C2);

static UNIT: AtomicU32 = AtomicU32::new(0x4040_0000);
static UNIT_PIXELS: AtomicU32 = AtomicU32::new(3);

/// Works out the grid for this frame, from the size of the window in real pixels.
/// The tallest menu is 224 units and the widest, with the recipe book, about 300.
pub fn set_unit(screen: Rect, pixels_per_point: f32) {
    let (w, h) = (screen.width() * pixels_per_point, screen.height() * pixels_per_point);
    let pixels = (h / 236.0).min(w / 300.0).floor().clamp(1.0, 16.0);
    UNIT_PIXELS.store(pixels as u32, Ordering::Relaxed);
    UNIT.store((pixels / pixels_per_point).to_bits(), Ordering::Relaxed);
}

/// One step of the grid, in points. A whole number of real pixels.
pub fn unit() -> f32 {
    f32::from_bits(UNIT.load(Ordering::Relaxed))
}

/// The pixel typeface is only sharp at whole multiples of its own grid.
pub fn words_size() -> f32 {
    if UNIT_PIXELS.load(Ordering::Relaxed) >= 6 { 54.0 } else { 27.0 }
}

/// Words on a panel, at a size in step with the grid.
pub fn words() -> FontId {
    // `font` moves any size onto the typeface's grid: these land on 27 and 54.
    if game_look() && pixel_font() { font(if words_size() > 27.0 { 40.0 } else { 20.0 }) } else { font(SIZE_BODY) }
}

/// Moves a point onto the grid of real pixels, so edges drawn from it stay sharp.
pub fn snap(at: Pos2, pixels_per_point: f32) -> Pos2 {
    pos2((at.x * pixels_per_point).round() / pixels_per_point, (at.y * pixels_per_point).round() / pixels_per_point)
}

/// The colours of one surface: its face and its two edges.
#[derive(Clone, Copy)]
struct Tone {
    face: Color32,
    lit: Color32,
    shade: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn contrast() -> bool {
    mode() == Mode::Contrast
}

fn outline() -> Color32 {
    if contrast() { hex(0xFFFFFF) } else { hex(0x17130C) }
}

fn slab() -> Tone {
    if contrast() {
        Tone { face: hex(0x000000), lit: hex(0xFFFFFF), shade: hex(0xFFFFFF) }
    } else {
        Tone { face: LIMESTONE.surface, lit: hex(0xEFE9D9), shade: hex(0x7D7461) }
    }
}

fn square(p: &mut Vec<Shape>, r: Rect, c: Color32) {
    p.push(Shape::rect_filled(r, CornerRadius::ZERO, c));
}

/// Top and left edges in one colour, bottom and right in another, `w` thick.
fn edges(out: &mut Vec<Shape>, r: Rect, w: f32, top_left: Color32, bottom_right: Color32) {
    square(out, Rect::from_min_max(r.min, pos2(r.right(), r.top() + w)), top_left);
    square(out, Rect::from_min_max(pos2(r.left(), r.top() + w), pos2(r.left() + w, r.bottom())), top_left);
    square(out, Rect::from_min_max(pos2(r.left(), r.bottom() - w), r.max), bottom_right);
    square(out, Rect::from_min_max(pos2(r.right() - w, r.top()), pos2(r.right(), r.bottom() - w)), bottom_right);
}

/// A shape with its corners cut off in one step.
fn notched(out: &mut Vec<Shape>, r: Rect, step: f32, c: Color32) {
    square(out, Rect::from_min_max(pos2(r.left() + step, r.top()), pos2(r.right() - step, r.bottom())), c);
    square(out, Rect::from_min_max(pos2(r.left(), r.top() + step), pos2(r.right(), r.bottom() - step)), c);
}

/// A small repeating picture of flecks, multiplied over a stone face.
fn grain(ctx: &egui::Context) -> egui::TextureId {
    let key = egui::Id::unique("stone grain");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return handle.id();
    }
    const SIZE: usize = 64;
    let mut pixels = vec![Color32::WHITE; SIZE * SIZE];
    // Always the same stone: a small generator with a fixed start.
    let mut seed = 0x2F6E_2B1Du32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for pixel in &mut pixels {
        let shade = match next() % 100 {
            0..=69 => 255,
            70..=86 => 247,
            87..=95 => 238,
            _ => 228,
        };
        *pixel = Color32::from_gray(shade);
    }
    let image = egui::ColorImage::new([SIZE, SIZE], pixels);
    let options = TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Nearest, wrap_mode: egui::TextureWrapMode::Repeat, mipmap_mode: None };
    let handle = ctx.load_texture("stone grain", image, options);
    let id = handle.id();
    ctx.data_mut(|d| d.insert_temp(key, handle));
    id
}

/// A face of flecked stone over `r`.
fn stone(ctx: &egui::Context, r: Rect, face: Color32) -> Shape {
    let u = unit();
    let mut mesh = Mesh::with_texture(grain(ctx));
    let span = 64.0 * u;
    // Tied to the rectangle's corner, so the flecks don't crawl as it moves.
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(r.width() / span, r.height() / span));
    mesh.add_rect_with_uv(r, uv, face);
    Shape::mesh(mesh)
}

/// Everything a panel draws, for placing under content that's already laid out.
pub fn panel_shapes(ctx: &egui::Context, r: Rect) -> Vec<Shape> {
    let u = unit();
    let mut out = Vec::with_capacity(12);
    let t = slab();
    notched(&mut out, r.translate(vec2(u * 2.0, u * 2.0)), u, Color32::from_black_alpha(110));
    notched(&mut out, r, u, outline());
    let inner = r.shrink(u);
    if contrast() {
        square(&mut out, inner, t.face);
    } else {
        out.push(stone(ctx, inner, t.face));
    }
    edges(&mut out, inner, u, t.lit, t.shade);
    out
}

pub fn panel(p: &Painter, r: Rect) {
    p.extend(panel_shapes(p.ctx(), r));
}

/// Runs `add` inside a panel, sized to what it draws.
pub fn window<R>(ui: &mut egui::Ui, margin: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let under = ui.painter().add(Shape::Noop);
    on_panel(ui, |ui| {
        // Scroll areas fade out towards the page's colour, which here is the stone.
        ui.visuals_mut().panel_fill = surface();
        let shown = egui::Frame::new().inner_margin(egui::Margin::same(margin)).show(ui, add);
        ui.painter().set(under, Shape::Vec(panel_shapes(ui.ctx(), shown.response.rect)));
        shown.inner
    })
}

fn slot_colors() -> (Color32, Color32, Color32, Color32) {
    if contrast() { (hex(0x000000), hex(0x333333), hex(0xFFFFFF), hex(0xFFFFFF)) } else { (hex(0x3F392F), hex(0x6D6554), hex(0x1E1A13), hex(0xD8D0BC)) }
}

/// A place cut into a panel: slots, lists, the window on your character.
pub fn well(p: &Painter, r: Rect, hot: bool) {
    let u = unit();
    let (face, lit, dark, light) = slot_colors();
    let mut out = Vec::with_capacity(5);
    square(&mut out, r, if hot { lit } else { face });
    edges(&mut out, r, u, dark, light);
    p.extend(out);
}

/// A larger place for what a menu makes.
pub fn output_well(p: &Painter, r: Rect, hot: bool) {
    let u = unit();
    let (face, lit, dark, light) = slot_colors();
    let mut out = Vec::with_capacity(9);
    edges(&mut out, r, u, dark, light);
    edges(&mut out, r.shrink(u), u, light, dark);
    edges(&mut out, r.shrink(u * 2.0), u, dark, light);
    square(&mut out, r.shrink(u * 3.0), if hot { lit } else { face });
    p.extend(out);
}

/// A slot shown straight on the world, for the hotbar: see-through, so the view shows.
pub fn hud_slot(p: &Painter, r: Rect) {
    let u = unit();
    let mut out = Vec::with_capacity(5);
    square(&mut out, r, Color32::from_black_alpha(if contrast() { 220 } else { 120 }));
    edges(&mut out, r, u, Color32::from_black_alpha(200), Color32::from_white_alpha(60));
    p.extend(out);
}

/// The heavy frame round the chosen hotbar slot.
pub fn selected_frame(p: &Painter, r: Rect) {
    let u = unit();
    let mut out = Vec::with_capacity(8);
    let outer = r.expand(u);
    edges(&mut out, outer, u * 2.0, hex(0x000000), hex(0x000000));
    edges(&mut out, outer.shrink((u * 0.34).round().max(1.0)), u, hex(0xFFFFFF), hex(0xFFFFFF));
    p.extend(out);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Face {
    Plain,
    Main,
    Danger,
    Off,
    /// Same stone as the panels, for a tab that joins onto one.
    Slab,
}

/// A key's body. Held down, its edges swap, so it looks pushed in.
pub fn button(p: &Painter, r: Rect, face: Face, hot: bool, down: bool) {
    let u = unit().min(4.0);
    let e = (u * 0.67).round().max(1.0);
    let t = match face {
        _ if contrast() => Tone { face: if hot { hover() } else { raised() }, lit: hex(0xFFFFFF), shade: hex(0xFFFFFF) },
        Face::Main => Tone { face: if hot { LIMESTONE.action_hover } else { LIMESTONE.action }, lit: hex(0xD08A50), shade: hex(0x54260A) },
        Face::Danger => Tone { face: if hot { hex(0x94211A) } else { LIMESTONE.danger }, lit: hex(0xC2685C), shade: hex(0x3F0905) },
        Face::Slab => slab(),
        Face::Off => Tone { face: hex(0xAEA591), lit: hex(0xBDB4A0), shade: hex(0x948B78) },
        Face::Plain => Tone { face: if hot { LIMESTONE.hover } else { LIMESTONE.raised }, lit: hex(0xF7F2E6), shade: hex(0x8B8370) },
    };
    let mut out = Vec::with_capacity(10);
    notched(&mut out, r, e, outline());
    let inner = r.shrink(e);
    square(&mut out, inner, t.face);
    if down {
        edges(&mut out, inner, e, t.shade, t.lit);
    } else {
        edges(&mut out, inner, e, t.lit, t.shade);
        // A thicker bottom edge gives it some height.
        square(&mut out, Rect::from_min_max(pos2(inner.left(), inner.bottom() - e * 2.0), inner.max), t.shade);
    }
    p.extend(out);
}

/// How far words on a key sit above its middle, given the thicker bottom edge.
pub fn key_lift() -> f32 {
    (unit().min(4.0) * 0.67).round().max(1.0) / 2.0
}

/// Words on the world or on stone. On the world they get a hard shadow that keeps them
/// readable; on stone, a light edge below, as if cut into it.
pub fn text(p: &Painter, at: Pos2, align: Align2, words: impl ToString, font: FontId, color: Color32) {
    let words = words.to_string();
    if panel_mode_now() {
        if !contrast() {
            p.text(at + vec2(1.0, 1.0), align, &words, font.clone(), Color32::from_white_alpha(70));
        }
    } else {
        p.text(at + vec2(2.0, 2.0), align, &words, font.clone(), Color32::from_black_alpha(210));
    }
    p.text(at, align, words, font, color);
}

/// A small picture from rows of `#`, each mark `size` across, centred in `area`.
/// `keep` chooses which marks are drawn, by column and row.
pub fn picture(p: &Painter, area: Rect, rows: &[&str], size: f32, color: Color32, keep: impl Fn(usize, usize) -> bool) {
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0) as f32 * size;
    let h = rows.len() as f32 * size;
    let at = area.center() - vec2(w, h) / 2.0;
    let at = pos2((at.x / size).round() * size, (at.y / size).round() * size);
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' && keep(x, y) {
                p.rect_filled(Rect::from_min_size(at + vec2(x as f32, y as f32) * size, Vec2::splat(size)), CornerRadius::ZERO, color);
            }
        }
    }
}

pub const PLUS: [&str; 9] = ["...###...", "...###...", "...###...", "#########", "#########", "#########", "...###...", "...###...", "...###..."];
pub const CROSS: [&str; 7] = ["##...##", "###.###", ".#####.", "..###..", ".#####.", "###.###", "##...##"];
pub const FLAME: [&str; 13] = ["......#......", "......##.....", ".....###.....", ".....####....", "....#####.#..", "..#.#######..", "..#########..", ".###########.", ".#####.#####.", "#####...#####", "#####...#####", ".####...####.", "..#########.."];
pub const BOOK: [&str; 12] = [".####...####.", "######.######", "##..##.##..##", "######.######", "##..##.##..##", "######.######", "##..##.##..##", "######.######", "######.######", ".#####.#####.", "...###.###...", ".....###....."];

/// Where a mark isn't filled in: visible on the panel in any theme.
fn unfilled() -> Color32 {
    if contrast() { hex(0x8C8C8C) } else { hex(0x8E8571) }
}

/// The arrow between what goes in and what comes out, filling as work is done.
pub fn arrow(p: &Painter, area: Rect, fraction: f32) {
    arrow_colored(p, area, fraction, if contrast() { hex(0xFFFFFF) } else { LIMESTONE.action });
}

pub fn arrow_colored(p: &Painter, area: Rect, fraction: f32, fill: Color32) {
    let u = unit();
    let (w, h) = (22.0, 15.0);
    let at = area.center() - vec2(w, h) * u / 2.0;
    let at = pos2((at.x / u).round() * u, (at.y / u).round() * u);
    let cut = (fraction.clamp(0.0, 1.0) * w).round();
    for y in 0..15 {
        let away = (y as i32 - 7).abs();
        // The head narrows away from the middle; the shaft is five rows high.
        let (start, end) = if away <= 2 { (0.0, 22.0 - away as f32) } else { (14.0, 22.0 - away as f32) };
        let row = |from: f32, to: f32, c: Color32| {
            if to > from {
                p.rect_filled(Rect::from_min_max(at + vec2(from, y as f32) * u, at + vec2(to, y as f32 + 1.0) * u), CornerRadius::ZERO, c);
            }
        };
        row(start, end, unfilled());
        row(start, end.min(cut), fill);
    }
}

/// A flame that burns down as fuel runs out.
pub fn flame(p: &Painter, area: Rect, fraction: f32) {
    let u = unit();
    let rows = FLAME.len();
    let cut = rows - (fraction.clamp(0.0, 1.0) * rows as f32).round() as usize;
    picture(p, area, &FLAME, u, unfilled(), |_, _| true);
    picture(p, area, &FLAME, u, if contrast() { hex(0xFFFFFF) } else { hex(0xD9641E) }, |_, y| y >= cut);
}

/// A sideways or upright gauge cut into the panel.
pub fn gauge(p: &Painter, r: Rect, fraction: f32, color: Color32, upright: bool) {
    well(p, r, false);
    let inner = r.shrink(unit());
    let f = fraction.clamp(0.0, 1.0);
    let filled = if upright { Rect::from_min_max(pos2(inner.left(), inner.bottom() - inner.height() * f), inner.max) } else { Rect::from_min_max(inner.min, pos2(inner.left() + inner.width() * f, inner.bottom())) };
    p.rect_filled(filled, CornerRadius::ZERO, color);
}

/// A track with a key in it, for lists longer than their window. `position` runs 0 to 1.
pub fn scroller(p: &Painter, track: Rect, position: f32, shown: f32) {
    well(p, track, false);
    let inner = track.shrink(unit());
    let h = (inner.height() * shown.clamp(0.0, 1.0)).max(unit() * 8.0).min(inner.height());
    let top = inner.top() + (inner.height() - h) * position.clamp(0.0, 1.0);
    button(p, Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), h)), Face::Plain, false, false);
}

const HELMET: [&str; 8] = ["..######..", ".########.", "##########", "##########", "###....###", "##......##", "##......##", "##......##"];
const CHEST: [&str; 10] = ["###....###", "####..####", "##########", ".########.", "..######..", "..######..", "..######..", "..######..", "..######..", "..######.."];
const LEGS: [&str; 10] = [".########.", ".########.", ".########.", ".###..###.", ".###..###.", ".###..###.", ".##....##.", ".##....##.", ".##....##.", ".##....##."];
const BOOTS: [&str; 7] = [".##....##.", ".##....##.", ".##....##.", ".##....##.", "###...###.", "####..####", "####..####"];
const SHIELD: [&str; 10] = ["##########", "##########", "##########", "##########", "##########", ".########.", ".########.", "..######..", "..######..", "....##...."];
const SADDLE: [&str; 8] = ["..........", "##......##", "###....###", "##########", "##########", ".########.", "..##..##..", "..##..##.."];
const BOTTLE: [&str; 11] = ["...####...", "...####...", "....##....", "....##....", "...####...", "..######..", ".########.", ".########.", ".########.", ".########.", "..######.."];
const DUST: [&str; 8] = ["....##....", "...####...", "...####...", "..######..", "..######..", ".########.", ".########.", "##########"];
const INGOT: [&str; 7] = ["..........", "...######.", "..######..", ".######...", "#########.", "########..", ".######..."];
const BANNER: [&str; 11] = ["##########", "##########", "##########", "##########", "##########", "##########", "##########", "##########", "##########", "###....###", "##......##"];
const GEM: [&str; 9] = ["...####...", "..######..", ".########.", "##########", "##########", ".########.", "..######..", "...####...", "....##...."];

/// What goes in an empty slot, as a faint outline. Other hints show on hover.
pub fn slot_hint(p: &Painter, r: Rect, hint: &str) {
    let rows: &[&str] = match hint {
        "Head" => &HELMET,
        "Chest" | "Armour" => &CHEST,
        "Legs" => &LEGS,
        "Feet" => &BOOTS,
        "Offhand" => &SHIELD,
        "Fuel" => &FLAME,
        "Saddle" => &SADDLE,
        "Bottle" => &BOTTLE,
        "Ingredient" | "Blaze powder" => &DUST,
        "Material" | "Base" => &INGOT,
        "Banner" => &BANNER,
        "Lapis" | "Payment" => &GEM,
        _ => return,
    };
    let size = (unit() * 0.67).round().max(1.0).max(unit().round());
    picture(p, r, rows, size, if contrast() { hex(0xBFBFBF) } else { hex(0x635C4E) }, |_, _| true);
}

/// Dims the world behind a menu.
pub fn dim(p: &Painter, screen: Rect, tint: Color32) {
    p.rect_filled(screen, CornerRadius::ZERO, tint);
}

/// The box round a tooltip: darker than any panel, so it can sit on one.
pub fn tip_box(p: &Painter, r: Rect) {
    let u = unit().min(3.0);
    let mut out = Vec::with_capacity(6);
    notched(&mut out, r, u, hex(0x0E0B07));
    let inner = r.shrink(u);
    square(&mut out, inner, hex(0x241E15));
    edges(&mut out, inner, u, hex(0x8C7F63), hex(0x5A4F3B));
    p.extend(out);
}

/// A line of words centred on the width of `ui`, with the hard shadow.
pub fn title(ui: &mut egui::Ui, words: &str, size: f32, color: Color32) {
    let width = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(width, size * 1.3), egui::Sense::hover());
    text(ui.painter(), r.center(), Align2::CENTER_CENTER, words, font_bold(size), color);
}

/// Words explaining an empty place, wrapped to fit it.
pub fn note_in(p: &Painter, r: Rect, words: &str) {
    let galley = p.layout(words.to_string(), self::words(), TIP_MUTED, r.width() - unit() * 6.0);
    // Laid out left-aligned, so each line isn't centred, but the block is.
    let at = (r.center() - galley.size() / 2.0).floor();
    p.galley(at, galley, TIP_MUTED);
}
