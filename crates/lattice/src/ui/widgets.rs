//! Custom-painted controls in Lattice's style. Every one of them can be reached
//! with Tab, pressed with Enter or Space, shows where the focus is, and tells
//! screen readers what it is.

use super::theme::*;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Response, Role, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, pos2, vec2};
use lattice_proto::Span;
use std::sync::atomic::{AtomicBool, Ordering};

/// A button was pressed since this was last asked, so a game window can click.
static PRESSED: AtomicBool = AtomicBool::new(false);

pub fn take_button_press() -> bool {
    PRESSED.swap(false, Ordering::Relaxed)
}

fn radius() -> CornerRadius {
    if game_look() { CornerRadius::ZERO } else { CornerRadius::same(CONTROL_RADIUS) }
}

/// A button's words on stone: sunk with the button, and with a hard shadow.
fn chrome_words(ui: &Ui, rect: Rect, galley: std::sync::Arc<egui::Galley>, color: Color32, down: bool, on_light: bool) {
    let lift = super::chrome::key_lift();
    let sink = if down { vec2(0.0, lift) } else { vec2(0.0, -lift) };
    let at = (rect.center() - galley.size() / 2.0 + sink).floor();
    // Words cut into a pale key catch the light underneath; on a dark one they're just lit.
    if !on_light && super::theme::mode() != Mode::Contrast {
        ui.painter().galley_with_override_text_color(at + vec2(1.0, 1.0), galley.clone(), Color32::from_white_alpha(90));
    }
    ui.painter().galley(at, galley, color);
}

/// The keyboard focus mark, the same on every control.
pub fn focus_ring(ui: &Ui, resp: &Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect.expand(1.0), radius(), Stroke::new(2.0, focus()), StrokeKind::Outside);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Danger,
}

fn button(ui: &mut Ui, label: &str, kind: Kind, enabled: bool, large: bool) -> Response {
    let font = match (kind, large) {
        (_, true) => font_bold(SIZE_LEAD),
        (Kind::Primary, false) => font_bold(SIZE_BODY),
        _ => font(SIZE_BODY),
    };
    let text = match (enabled, kind) {
        (false, _) => ink_muted(),
        (true, Kind::Primary) => on_action(),
        (true, Kind::Secondary) => ink(),
        (true, Kind::Danger) => danger(),
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text);
    let pad = if large { vec2(28.0, 13.0) } else { ui.spacing().button_padding };
    let size = vec2(galley.size().x + pad.x * 2.0, (galley.size().y + pad.y * 2.0).max(TARGET));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    crate::dev::note_widget(label, rect);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if resp.clicked() {
        PRESSED.store(true, Ordering::Relaxed);
    }
    if ui.is_rect_visible(rect) && game_look() {
        let down = resp.is_pointer_button_down_on();
        let face = match (enabled, kind) {
            (false, _) => super::chrome::Face::Off,
            (true, Kind::Primary) => super::chrome::Face::Main,
            (true, Kind::Secondary) => super::chrome::Face::Plain,
            (true, Kind::Danger) => super::chrome::Face::Danger,
        };
        let text = if kind == Kind::Danger && enabled { on_action() } else { text };
        super::chrome::button(ui.painter(), rect, face, resp.hovered(), down);
        focus_ring(ui, &resp);
        chrome_words(ui, rect, galley, text, down, kind == Kind::Primary && enabled);
    } else if ui.is_rect_visible(rect) {
        let down = resp.is_pointer_button_down_on();
        let fill = match (enabled, kind) {
            (false, _) => surface(),
            (true, Kind::Primary) if resp.hovered() || down => action_hover(),
            (true, Kind::Primary) => action(),
            (true, _) if resp.hovered() || down => hover(),
            (true, _) => raised(),
        };
        ui.painter().rect_filled(rect, radius(), fill);
        if kind != Kind::Primary || !enabled {
            ui.painter().rect_stroke(rect, radius(), Stroke::new(1.0, edge()), StrokeKind::Inside);
        }
        focus_ring(ui, &resp);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, text);
    }
    resp
}

/// The one main action on a screen.
pub fn primary(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, Kind::Primary, true, false)
}

pub fn primary_enabled(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, Kind::Primary, enabled, false)
}

/// The big one, for the action a whole page exists for.
pub fn primary_large(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, Kind::Primary, enabled, true)
}

pub fn secondary(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, Kind::Secondary, true, false)
}

pub fn secondary_enabled(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, Kind::Secondary, enabled, false)
}

pub fn secondary_large(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, Kind::Secondary, true, true)
}

/// A destructive action, drawn in the error colour so it isn't pressed by habit.
pub fn danger_button(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, Kind::Danger, true, false)
}

/// A full-width button for stacked menus.
pub fn menu_button(ui: &mut Ui, label: &str, primary: bool, width: f32) -> Response {
    let kind = if primary { Kind::Primary } else { Kind::Secondary };
    let (fill, fill_hover, text) = if primary { (action(), action_hover(), on_action()) } else { (raised(), hover(), ink()) };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
    crate::dev::note_widget(label, rect);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if resp.clicked() {
        PRESSED.store(true, Ordering::Relaxed);
    }
    let f = if kind == Kind::Primary { font_bold(SIZE_BODY) } else { font(SIZE_BODY) };
    if game_look() {
        let down = resp.is_pointer_button_down_on();
        super::chrome::button(ui.painter(), rect, if primary { super::chrome::Face::Main } else { super::chrome::Face::Plain }, resp.hovered(), down);
        focus_ring(ui, &resp);
        let galley = ui.painter().layout_no_wrap(label.to_string(), font_bold(SIZE_LEAD), text);
        chrome_words(ui, rect, galley, text, down, primary);
        return resp;
    }
    ui.painter().rect_filled(rect, radius(), if resp.hovered() { fill_hover } else { fill });
    if !primary {
        ui.painter().rect_stroke(rect, radius(), Stroke::new(1.0, edge()), StrokeKind::Inside);
    }
    focus_ring(ui, &resp);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, f, text);
    resp
}

/// On/off control. `label` is what a screen reader announces; draw visible text
/// beside it yourself, or use [`switch_line`].
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(46.0, 26.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
        PRESSED.store(true, Ordering::Relaxed);
    }
    paint_switch(ui, rect, *on, resp.hovered());
    focus_ring(ui, &resp);
    let value = *on;
    resp.widget_info(|| WidgetInfo::selected(Role::Switch, true, value, label));
    resp
}

fn paint_switch(ui: &Ui, rect: Rect, on: bool, hovered: bool) {
    if game_look() {
        // A block sliding in a slot cut into the stone.
        super::chrome::well(ui.painter(), rect, false);
        let inner = rect.shrink(3.0);
        if on {
            ui.painter().rect_filled(inner, CornerRadius::ZERO, super::chrome::TIP_MUTED);
        }
        let size = vec2(inner.height(), inner.height());
        let block = Rect::from_min_size(if on { pos2(inner.right() - size.x, inner.top()) } else { inner.min }, size);
        super::chrome::button(ui.painter(), block, if on { super::chrome::Face::Main } else { super::chrome::Face::Plain }, hovered, false);
        return;
    }
    let track = if on { accent() } else { ground() };
    let r = CornerRadius::same(13);
    ui.painter().rect_filled(rect, r, track);
    let border = if hovered { ink() } else { edge() };
    ui.painter().rect_stroke(rect, r, Stroke::new(if on { 0.0 } else { 1.5 }, border), StrokeKind::Inside);
    // The knob's side says the state, so colour isn't the only signal.
    let x = if on { rect.right() - 13.0 } else { rect.left() + 13.0 };
    let knob = if on { ground() } else { ink_muted() };
    ui.painter().circle_filled(pos2(x, rect.center().y), 8.0, knob);
}

/// A switch with its text; the whole line can be pressed.
pub fn switch_line(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout(label.to_string(), font(SIZE_BODY), ink(), (ui.available_width() - 58.0).max(120.0));
    let h = galley.size().y.max(30.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(58.0 + galley.size().x, h), Sense::click());
    crate::dev::note_widget(label, rect);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
        PRESSED.store(true, Ordering::Relaxed);
    }
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 13.0), vec2(46.0, 26.0));
    paint_switch(ui, track, *on, resp.hovered());
    ui.painter().galley(pos2(rect.left() + 58.0, rect.center().y - galley.size().y / 2.0), galley, ink());
    focus_ring(ui, &resp);
    let value = *on;
    resp.widget_info(|| WidgetInfo::selected(Role::Switch, true, value, label));
    resp
}

/// A few mutually exclusive choices, all visible. Returns true when it changed.
pub fn segmented(ui: &mut Ui, group: &str, options: &[&str], selected: &mut usize) -> bool {
    let mut changed = false;
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
    let f = font(SIZE_BODY);
    for (i, label) in options.iter().enumerate() {
        let on = *selected == i;
        let galley = ui.painter().layout_no_wrap(label.to_string(), if on { font_bold(SIZE_BODY) } else { f.clone() }, ink());
        let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, TARGET), Sense::click());
        crate::dev::note_widget(&format!("{group}:{label}"), rect);
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
            PRESSED.store(true, Ordering::Relaxed);
        }
        if game_look() {
            // The chosen one stays pushed in, like a held-down key.
            super::chrome::button(ui.painter(), rect, super::chrome::Face::Plain, resp.hovered() && !on, on);
            if on {
                ui.painter().rect_filled(rect.shrink(4.0), CornerRadius::ZERO, accent_dim());
            }
            if resp.has_focus() {
                ui.painter().rect_stroke(rect.expand(1.0), CornerRadius::ZERO, Stroke::new(2.0, focus()), StrokeKind::Outside);
            }
            chrome_words(ui, rect, galley, ink(), on, false);
            let label = format!("{group}: {label}");
            resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, on, &label));
            continue;
        }
        let c = CONTROL_RADIUS;
        let r = CornerRadius {
            nw: if i == 0 { c } else { 0 },
            sw: if i == 0 { c } else { 0 },
            ne: if i + 1 == options.len() { c } else { 0 },
            se: if i + 1 == options.len() { c } else { 0 },
        };
        let fill = if on {
            accent_dim()
        } else if resp.hovered() {
            hover()
        } else {
            raised()
        };
        ui.painter().rect_filled(rect, r, fill);
        ui.painter().rect_stroke(rect, r, Stroke::new(if on { 2.0 } else { 1.0 }, if on { accent() } else { edge() }), StrokeKind::Inside);
        if resp.has_focus() {
            ui.painter().rect_stroke(rect.expand(1.0), r, Stroke::new(2.0, focus()), StrokeKind::Outside);
        }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink());
        let label = format!("{group}: {label}");
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, on, &label));
    }
    ui.spacing_mut().item_spacing.x = spacing;
    changed
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Sessions,
    Account,
    Settings,
}

/// The small drawn icons. Lines, like everything else in the drawing.
pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.8, color);
    let c = rect.center();
    let u = rect.width() / 20.0;
    match icon {
        Icon::Play => {
            p.add(Shape::convex_polygon(vec![c + vec2(-5.0, -7.0) * u, c + vec2(7.0, 0.0) * u, c + vec2(-5.0, 7.0) * u], color, Stroke::NONE));
        }
        Icon::Sessions => {
            // Two windows, one behind the other.
            let front = Rect::from_center_size(c + vec2(-2.0, 2.0) * u, vec2(13.0, 11.0) * u);
            p.rect_stroke(front, CornerRadius::same(2), s, StrokeKind::Middle);
            let (tl, tr, br) = (c + vec2(2.0, -7.0) * u, c + vec2(9.0, -7.0) * u, c + vec2(9.0, 2.0) * u);
            p.line_segment([tl, tr], s);
            p.line_segment([tr, br], s);
        }
        Icon::Account => {
            p.circle_stroke(c + vec2(0.0, -4.0) * u, 4.0 * u, s);
            let mut pts = Vec::new();
            for i in 0..=12 {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
                pts.push(c + vec2(a.cos() * 7.5, 8.0 + a.sin() * 6.0) * u);
            }
            p.add(Shape::line(pts, s));
        }
        Icon::Settings => {
            // Three sliders.
            for (y, x) in [(-5.5, 3.0), (0.0, -3.0), (5.5, 1.5)] {
                p.line_segment([c + vec2(-8.0, y) * u, c + vec2(8.0, y) * u], s);
                p.circle_filled(c + vec2(x, y) * u, 2.6 * u, color);
            }
        }
    }
}

/// A sidebar entry. `badge` is a short status such as "2 running".
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, badge: Option<&str>, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 42.0), Sense::click());
    crate::dev::note_widget(label, rect);
    let fill = if selected {
        Some(accent_dim())
    } else if resp.hovered() {
        Some(hover())
    } else {
        None
    };
    if let Some(f) = fill {
        ui.painter().rect_filled(rect, radius(), f);
    }
    let color = if selected || resp.hovered() { ink() } else { ink_muted() };
    paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(20.0)), icon, color);
    let f = if selected { font_bold(SIZE_BODY) } else { font(SIZE_BODY) };
    ui.painter().text(pos2(rect.left() + 42.0, rect.center().y), Align2::LEFT_CENTER, label, f, color);
    if let Some(b) = badge {
        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, b, font_bold(SIZE_SMALL), accent());
    }
    focus_ring(ui, &resp);
    let spoken = badge.map_or(label.to_string(), |b| format!("{label}, {b}"));
    resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, &spoken));
    resp
}

/// Labelled single-line text input. Returns the edit response.
pub fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str, width: f32) -> Response {
    let l = ui.label(egui::RichText::new(label).color(ink()).font(font(SIZE_SMALL)));
    let resp = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(egui::RichText::new(hint).color(ink_muted()))
            .desired_width(width)
            .margin(vec2(10.0, 8.0))
            .font(font(SIZE_BODY)),
    );
    let resp = resp.labelled_by(l.id);
    crate::dev::note_widget(label, resp.rect);
    focus_ring(ui, &resp);
    resp
}

pub fn muted(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.label(egui::RichText::new(text.into()).color(ink_muted()).font(font(SIZE_SMALL)))
}

pub fn heading(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.label(egui::RichText::new(text.into()).font(font_bold(SIZE_TITLE)).color(ink()))
}

pub fn subheading(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.label(egui::RichText::new(text.into()).font(font_bold(SIZE_LEAD)).color(ink()))
}

pub fn error_text(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.label(egui::RichText::new(text.into()).color(danger()))
}

/// Server-provided rich text (chat, MOTD, kick reasons) as a layout job.
pub fn spans_job(spans: &[Span], size: f32, default: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    for s in spans {
        let color = component_color(s.color, default);
        let mut f = TextFormat {
            font_id: if s.bold { font_bold(size) } else { font(size) },
            color,
            italics: s.italic,
            ..Default::default()
        };
        if s.underlined || s.link.is_some() {
            f.underline = Stroke::new(1.0, color);
        }
        if s.strikethrough {
            f.strikethrough = Stroke::new(1.0, color);
        }
        let text = if s.obfuscated { s.text.chars().map(|c| if c.is_whitespace() { c } else { '▒' }).collect() } else { s.text.clone() };
        job.append(&text, 0.0, f);
    }
    job
}

/// Determinate progress: cells of a lattice filling up one by one.
pub fn progress(ui: &mut Ui, fraction: f32, width: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
    let fraction = fraction.clamp(0.0, 1.0);
    resp.widget_info(|| WidgetInfo { value: Some(fraction as f64 * 100.0), ..WidgetInfo::labeled(Role::ProgressIndicator, true, "Progress") });
    let cell = 10.0;
    let gap = 2.0;
    let n = ((width + gap) / (cell + gap)).floor().max(1.0) as usize;
    let step = (width + gap) / n as f32;
    let filled = fraction * n as f32;
    for i in 0..n {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * step, rect.top()), vec2(step - gap, rect.height()));
        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, edge()), StrokeKind::Inside);
        let amount = (filled - i as f32).clamp(0.0, 1.0);
        if amount > 0.0 {
            let mut f = r;
            f.set_width(r.width() * amount);
            ui.painter().rect_filled(f, CornerRadius::ZERO, accent());
        }
    }
}

pub fn format_bytes(n: u64) -> String {
    if n >= 1 << 20 { format!("{:.1} MB", n as f64 / (1 << 20) as f64) } else { format!("{} KB", n / 1024) }
}

/// Slider rails you can see against the page, filled up to the handle.
pub fn slider_look(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.widgets.inactive.bg_fill = edge();
    v.widgets.hovered.bg_fill = edge();
    v.widgets.active.bg_fill = edge();
    v.selection.bg_fill = accent();
}

/// Shows that something's under way. With reduced motion it holds still.
pub fn busy(ui: &mut Ui) {
    if reduce_motion() {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(2.0, accent()));
    } else {
        ui.add(egui::Spinner::new().size(18.0).color(accent()));
    }
}

/// A colour between two others.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// The mark: one cube of the lattice, a wire drawing with its nodes marked.
pub fn mark(p: &Painter, center: Pos2, size: f32) {
    let r = size / 2.0;
    let (dx, dy) = (r * 0.866, r * 0.5);
    let top = center + vec2(0.0, -r);
    let bottom = center + vec2(0.0, r);
    let (ul, ur, ll, lr) = (center + vec2(-dx, -dy), center + vec2(dx, -dy), center + vec2(-dx, dy), center + vec2(dx, dy));
    let s = Stroke::new((size / 16.0).max(1.5), accent());
    for (a, b) in [(top, ur), (ur, lr), (lr, bottom), (bottom, ll), (ll, ul), (ul, top), (center, ul), (center, ur), (center, bottom)] {
        p.line_segment([a, b], s);
    }
    for n in [top, ur, lr, bottom, ll, ul] {
        p.circle_filled(n, (size / 11.0).max(2.0), accent());
    }
    p.circle_filled(center, (size / 7.0).max(2.5), action());
}

fn noise(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

fn smooth_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = noise(xi, yi, seed) + (noise(xi + 1, yi, seed) - noise(xi, yi, seed)) * sx;
    let b = noise(xi, yi + 1, seed) + (noise(xi + 1, yi + 1, seed) - noise(xi, yi + 1, seed)) * sx;
    a + (b - a) * sy
}

/// Lattice's own picture: a piece of ground drawn as an isometric block grid,
/// different for every `seed`. Used wherever there's no frame from the game yet.
pub fn lattice_field(p: &Painter, rect: Rect, seed: u32, cell: f32) {
    let p = p.with_clip_rect(rect.intersect(p.clip_rect()));
    p.rect_filled(rect, CornerRadius::ZERO, ground());
    let (ex, ey) = (cell * 0.866, cell * 0.5);
    let height = |i: i32, j: i32| -> i32 {
        let n = smooth_noise(i as f32 / 7.0, j as f32 / 7.0, seed) * 0.8 + smooth_noise(i as f32 / 2.5, j as f32 / 2.5, seed ^ 0x55) * 0.2;
        ((n - 0.25) * 8.0).floor().clamp(0.0, 5.0) as i32
    };
    let origin = rect.center();
    let span = ((rect.width() / ex + rect.height() / ey) / 2.0).ceil() as i32 + 8;
    let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(accent().r(), accent().g(), accent().b(), 90));
    let (base_top, base_left, base_right) = (raised(), ground(), surface());
    for sum in -2 * span..=2 * span {
        for i in -span..=span {
            let j = sum - i;
            if j.abs() > span {
                continue;
            }
            let x = (i - j) as f32 * ex;
            let y = (i + j) as f32 * ey;
            let h = height(i, j);
            let at = origin + vec2(x, y);
            if at.x < rect.left() - ex * 2.0 || at.x > rect.right() + ex * 2.0 || at.y < rect.top() - cell * 2.0 || at.y > rect.bottom() + cell * 8.0 {
                continue;
            }
            let lift = h as f32 * cell;
            let t = at + vec2(0.0, -lift);
            let top = vec![t + vec2(0.0, -ey), t + vec2(ex, 0.0), t + vec2(0.0, ey), t + vec2(-ex, 0.0)];
            let glow = h as f32 / 5.0;
            let fill = mix(base_top, accent_dim(), glow);
            // Only the faces a lower neighbour leaves showing.
            let left_drop = (h - height(i, j + 1)).max(0) as f32 * cell;
            let right_drop = (h - height(i + 1, j)).max(0) as f32 * cell;
            if left_drop > 0.0 {
                let a = t + vec2(-ex, 0.0);
                let b = t + vec2(0.0, ey);
                p.add(Shape::convex_polygon(vec![a, b, b + vec2(0.0, left_drop), a + vec2(0.0, left_drop)], base_left, line));
            }
            if right_drop > 0.0 {
                let a = t + vec2(0.0, ey);
                let b = t + vec2(ex, 0.0);
                p.add(Shape::convex_polygon(vec![a, b, b + vec2(0.0, right_drop), a + vec2(0.0, right_drop)], base_right, line));
            }
            p.add(Shape::convex_polygon(top, fill, line));
        }
    }
}

/// Fades a rectangle from `top` to `bottom` opacity of a colour, so text stays
/// readable over a picture.
pub fn scrim(p: &Painter, rect: Rect, color: Color32, top: u8, bottom: u8) {
    let c = |a: u8| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), c(top));
    mesh.colored_vertex(rect.right_top(), c(top));
    mesh.colored_vertex(rect.right_bottom(), c(bottom));
    mesh.colored_vertex(rect.left_bottom(), c(bottom));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
}

/// A picture scaled to cover `rect`, cropped rather than stretched.
pub fn image_cover(p: &Painter, rect: Rect, tex: &egui::TextureHandle) {
    let [w, h] = tex.size();
    let (tw, th) = (w as f32, h as f32);
    let (ra, rb) = (rect.width() / rect.height(), tw / th);
    let uv = if rb > ra {
        let cut = (1.0 - ra / rb) / 2.0;
        Rect::from_min_max(pos2(cut, 0.0), pos2(1.0 - cut, 1.0))
    } else {
        let cut = (1.0 - rb / ra) / 2.0;
        Rect::from_min_max(pos2(0.0, cut), pos2(1.0, 1.0 - cut))
    };
    p.image(tex.id(), rect, uv, Color32::WHITE);
}

/// A player's face tile, or a drawn head when there's no skin to show.
pub fn face_tile(ui: &Ui, rect: Rect, face: Option<&egui::TextureHandle>) {
    match face {
        Some(t) => {
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, raised());
            paint_icon(ui.painter(), rect.shrink(rect.width() * 0.14), Icon::Account, ink_muted());
        }
    }
}
