//! In-game interface.

use super::theme::*;
use super::widgets;
use crate::app::{App, Screen};
use crate::game::{Phase, Session};
use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, RichText, Stroke, Vec2};
use lattice_proto::Translator;

/// The hotbar's slots, on the same grid as the menus: an item's picture and a little room.
fn hotbar_slot() -> f32 {
    20.0 * super::chrome::unit()
}

fn hotbar_width() -> f32 {
    hotbar_slot() * 9.0
}

fn backing(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(ground().r(), ground().g(), ground().b(), a)
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hud")));
    let Some(lang) = (match &app.assets {
        crate::assets::AssetState::Ready(a) => Some(a.clone()),
        _ => None,
    }) else {
        return;
    };
    let tr: &dyn Translator = &lang.lang;
    let Some(session) = app.session.as_ref() else { return };
    crate::game::set_compass_view(crate::game::CompassView {
        yaw: session.player.body.yaw,
        pos: session.player.body.pos,
        dimension: session.dimension_name.clone(),
        spawn: session.spawn_point.clone(),
        death: session.death_point.clone(),
        time: ctx.input(|i| i.time) as f32,
    });

    if session.phase == Phase::LoadingTerrain || !session.player.received_position {
        super::backdrop(&painter, screen, &app.spec.server_name);
        let x = screen.left() + 48.0;
        let y = screen.bottom() - 112.0;
        painter.text(Pos2::new(x, y), Align2::LEFT_TOP, format!("Joining {}", app.spec.server_name), font_bold(SIZE_TITLE), ink());
        painter.text(Pos2::new(x, y + 40.0), Align2::LEFT_TOP, format!("Loading the world, {} chunks so far", session.chunks_received), font(SIZE_BODY), ink());
        return;
    }

    screen_overlays(app, &ctx, &painter, screen);
    let Some(session) = app.session.as_ref() else { return };
    if app.overlays.hide_hud && session.dead.is_none() && !app.overlays.paused && !app.overlays.chat_open && !session.inventory.is_open() && session.modal.is_none() && session.code_of_conduct.is_none() {
        return;
    }
    // Only while you're aiming: behind a menu it's just a mark on the buttons.
    if session.dead.is_none() && !app.overlays.paused && session.modal.is_none() && !session.inventory.is_open() {
        crosshair(&painter, screen, session, app.config.interface.attack_indicator);
    }
    let creative_like = session.player.game_mode == 1 || session.player.game_mode == 3;
    let spectator = session.player.game_mode == 3;
    // A menu gets the screen to itself: the hotbar's in it anyway, and chat only gets in the way.
    let menu_up = session.inventory.is_open() || session.modal.is_some() || app.overlays.paused;
    if !menu_up {
        let hotbar_top = hotbar(app, &ctx, &painter, screen, spectator);
        let Some(session) = app.session.as_ref() else { return };
        let mut above = hotbar_top;
        if !creative_like && session.vitals.known {
            above = vitals(&painter, screen, hotbar_top, session);
        }
        above = held_name(app, &painter, screen, above);
        let Some(session) = app.session.as_ref() else { return };
        action_bar(&painter, screen, above, session, tr);
        titles(&painter, screen, session, tr);
        boss_bars(&painter, screen, session, tr);
        effect_icons(app, &ctx, &painter, screen, tr);
        let Some(session) = app.session.as_ref() else { return };
        subtitles(app, &painter, screen, session, tr);
        sidebar(&painter, screen, session, tr);
    }
    let Some(session) = app.session.as_ref() else { return };
    achieved(&painter, screen, session, tr);
    totem_flash(app, &ctx, &painter, screen);
    let Some(session) = app.session.as_ref() else { return };
    let notices_top = if app.config.video.show_fps { frame_rate(app, &painter, screen) } else { screen.top() + 16.0 };
    notices(&painter, screen, session, notices_top);
    if app.overlays.debug {
        debug(app, &painter, screen, session);
    } else if app.config.interface.show_coordinates {
        let b = &session.player.body;
        let text = format!("{:.1} {:.1} {:.1}", b.pos.x, b.pos.y, b.pos.z);
        let g = painter.layout_no_wrap(text, font(SIZE_SMALL), ink());
        let r = Rect::from_min_size(Pos2::new(screen.left() + 12.0, screen.top() + 12.0), g.size() + Vec2::new(12.0, 6.0));
        painter.rect_filled(r, CornerRadius::ZERO, backing(170));
        painter.galley(r.min + Vec2::new(6.0, 3.0), g, ink());
    }
    let mut chat_hit = None;
    if (app.config.chat.mode != 2 && !menu_up) || app.overlays.chat_open {
        chat_hit = chat_lines(&painter, screen, session, tr, app.overlays.chat_open, &app.config.chat, ctx.pointer_latest_pos());
    }
    let tab_held = app.input.held.contains(&crate::input::Action::PlayerList) && !app.overlays.chat_open;
    if tab_held {
        let faces = tab_faces(app, &ctx);
        let Some(session) = app.session.as_ref() else { return };
        tab_list(&painter, screen, session, tr, &faces);
    }

    // Interactive overlays.
    if app.overlays.chat_open {
        if let Some(hit) = chat_hit {
            use_chat_span(app, &ctx, screen, hit, tr);
        }
        chat_input(app, ui, screen);
    }
    let Some(session) = app.session.as_ref() else { return };
    let inventory_open = session.inventory.is_open();
    if let Some(msg) = session.dead.clone() {
        death_screen(app, ui, screen, &msg, tr);
    } else if session.code_of_conduct.is_some() {
        code_of_conduct(app, ui, screen);
    } else if app.overlays.paused {
        pause_menu(app, ui, screen);
    } else if session.player.body.pose == lattice_world::physics::Pose::Sleeping {
        in_bed(app, ui, screen);
    } else if matches!(session.modal, Some(crate::game::Modal::Block(_))) {
        super::block_screens::draw(app, ui, screen);
    } else if matches!(session.modal, Some(crate::game::Modal::GameMode { .. })) {
        super::block_screens::draw_game_modes(app, ui, screen);
    } else if matches!(session.modal, Some(crate::game::Modal::GameRules { .. })) {
        super::block_screens::draw_game_rules(app, ui, screen);
    } else if matches!(session.modal, Some(crate::game::Modal::Teleport)) {
        super::block_screens::draw_teleport(app, ui, screen);
    } else if matches!(session.modal, Some(crate::game::Modal::Credits { .. })) {
        super::block_screens::draw_credits(app, ui, screen);
    } else if session.modal.is_some() {
        super::modal::draw(app, ui, screen);
    } else if inventory_open {
        super::inventory::draw(app, ui, screen);
    }
}

fn crosshair(p: &Painter, screen: Rect, session: &Session, attack_indicator: bool) {
    let c = screen.center();
    if session.inventory.is_open() || session.modal.is_some() {
        return;
    }
    for (w, color) in [(4.0, Color32::from_black_alpha(160)), (2.0, Color32::WHITE)] {
        let s = Stroke::new(w, color);
        p.line_segment([c - Vec2::new(8.0, 0.0), c + Vec2::new(8.0, 0.0)], s);
        p.line_segment([c - Vec2::new(0.0, 8.0), c + Vec2::new(0.0, 8.0)], s);
    }
    if let Some((_, progress)) = session.player.dig_progress() {
        let r = Rect::from_center_size(c + Vec2::new(0.0, 20.0), Vec2::new(40.0, 4.0));
        p.rect_filled(r, CornerRadius::ZERO, backing(200));
        let mut f = r;
        f.set_width(r.width() * progress);
        p.rect_filled(f, CornerRadius::ZERO, accent());
    } else if attack_indicator {
        let charge = session.player.attack_charge(0.0);
        if charge < 1.0 {
            let r = Rect::from_center_size(c + Vec2::new(0.0, 20.0), Vec2::new(40.0, 4.0));
            p.rect_filled(r, CornerRadius::ZERO, backing(200));
            let mut f = r;
            f.set_width(r.width() * charge);
            p.rect_filled(f, CornerRadius::ZERO, warn());
        }
    }
}

/// The hotbar: real items, with the selected one marked, and the offhand beside it.
fn hotbar(app: &mut App, ctx: &egui::Context, p: &Painter, screen: Rect, spectator: bool) -> f32 {
    if spectator {
        return screen.bottom() - 16.0;
    }
    let tex = app.item_texture(ctx);
    let Some(assets) = app.ready_assets() else { return screen.bottom() - 16.0 };
    let Some(session) = app.session.as_ref() else { return screen.bottom() - 16.0 };
    let selected = session.player.selected_slot;
    let day = session.day_fraction() as f32;
    let slot = hotbar_slot();
    let gap = 0.0;
    let u = super::chrome::unit();
    let total = hotbar_width();
    let ppp = ctx.pixels_per_point();
    let at = super::chrome::snap(Pos2::new(screen.center().x - total / 2.0, screen.bottom() - slot - 3.0 * u), ppp);
    let (x0, y0) = (at.x, at.y);
    let icon = |r: Rect| Rect::from_center_size(r.center(), Vec2::splat(16.0 * u));
    for i in 0..9 {
        let r = Rect::from_min_size(Pos2::new(x0 + i as f32 * (slot + gap), y0), Vec2::splat(slot));
        super::chrome::hud_slot(p, r);
        if let Some(stack) = session.inventory.hotbar(i as usize) {
            super::items::draw_stack(p, icon(r), tex, &assets, stack, day);
        }
    }
    // The chosen slot, drawn last so its frame sits over its neighbours.
    let r = Rect::from_min_size(Pos2::new(x0 + selected as f32 * (slot + gap), y0), Vec2::splat(slot));
    super::chrome::selected_frame(p, r);
    if let Some(stack) = session.inventory.offhand() {
        let r = Rect::from_min_size(Pos2::new(x0 - slot - 3.0 * u, y0), Vec2::splat(slot));
        super::chrome::hud_slot(p, r);
        super::items::draw_stack(p, icon(r), tex, &assets, stack, day);
    }
    y0
}

/// The held item's name, shown for a moment when it changes.
fn held_name(app: &mut App, p: &Painter, screen: Rect, above: f32) -> f32 {
    let Some(assets) = app.ready_assets() else { return above };
    let Some(session) = app.session.as_ref() else { return above };
    let stack = session.inventory.hotbar(session.player.selected_slot as usize);
    let key = stack.map(|s| (s.item, s.added.len()));
    if key != app.held_name.0 {
        app.held_name = (key, std::time::Instant::now());
    }
    let age = app.held_name.1.elapsed().as_secs_f32();
    let Some(stack) = stack else { return above };
    let time = app.config.interface.held_name_time;
    if age > time {
        return above;
    }
    let alpha = ((time - age) / 0.5).clamp(0.0, 1.0);
    let name = crate::game::items::display_name(stack, &assets.lang);
    let g = p.layout_no_wrap(name, font(SIZE_BODY), ink().gamma_multiply(alpha));
    let size = g.size();
    let pos = Pos2::new(screen.center().x - size.x / 2.0, above - size.y - 6.0);
    p.rect_filled(Rect::from_min_size(pos - Vec2::new(8.0, 2.0), size + Vec2::new(16.0, 4.0)), CornerRadius::ZERO, backing((130.0 * alpha) as u8));
    p.galley(pos, g, ink().gamma_multiply(alpha));
    pos.y - 4.0
}

// The HUD sits over the world, not a theme, and these colours carry meaning
// everywhere: they stay the same whatever the menus look like.
const HEALTH: Color32 = Color32::from_rgb(0xEF, 0x4F, 0x4F);
const ABSORPTION: Color32 = Color32::from_rgb(0xF2, 0xC1, 0x4E);
const FOOD: Color32 = Color32::from_rgb(0xE8, 0x9A, 0x3C);
const ARMOUR: Color32 = Color32::from_rgb(0xC4, 0xD2, 0xE0);
const AIR: Color32 = Color32::from_rgb(0x6F, 0xC7, 0xF2);
const EXPERIENCE: Color32 = Color32::from_rgb(0x8F, 0xE0, 0x5A);

/// Text that stays readable over any part of the world.
fn hud_text(p: &Painter, pos: Pos2, align: Align2, text: impl ToString, f: egui::FontId, color: Color32) -> Rect {
    let text = text.to_string();
    p.text(pos + Vec2::new(1.0, 1.0), align, &text, f.clone(), Color32::from_black_alpha(220));
    p.text(pos, align, &text, f, color)
}

/// A labelled bar of ten cells, so its level can be read at a glance. The number
/// beside the name means colour is never the only thing telling them apart.
fn cell_bar(p: &Painter, rect: Rect, fraction: f32, color: Color32, label: &str) {
    const CELLS: usize = 10;
    let gap = 2.0;
    let w = (rect.width() - gap * (CELLS - 1) as f32) / CELLS as f32;
    let filled = fraction.clamp(0.0, 1.0) * CELLS as f32;
    for i in 0..CELLS {
        let r = Rect::from_min_size(Pos2::new(rect.left() + i as f32 * (w + gap), rect.top()), Vec2::new(w, rect.height()));
        p.rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(170));
        let amount = (filled - i as f32).clamp(0.0, 1.0);
        if amount > 0.0 {
            let mut f = r;
            f.set_width(r.width() * amount);
            p.rect_filled(f, CornerRadius::ZERO, color);
        }
    }
    hud_text(p, rect.left_top() - Vec2::new(0.0, 2.0), Align2::LEFT_BOTTOM, label, font(SIZE_SMALL), Color32::WHITE);
}

/// Who and what the server marks as nearby: a dot where each one is, across the
/// directions in front of you. Those behind you sit at the ends.
fn locator_bar(p: &Painter, rect: Rect, session: &Session) {
    if session.waypoints.is_empty() {
        return;
    }
    p.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(130));
    let b = &session.player.body;
    let (px, py, pz) = (b.pos.x, b.pos.y + b.eye_height() as f64, b.pos.z);
    let look = (b.yaw as f64).to_radians();
    for (id, w) in &session.waypoints {
        use lattice_proto::packets::play::WaypointPlace as P;
        // The way it lies, as an angle from straight ahead, and whether it's above or below.
        let (angle, height): (f64, Option<f64>) = match w.place {
            P::Unknown => continue,
            P::Block(x, y, z) => {
                let (dx, dz) = (x as f64 + 0.5 - px, z as f64 + 0.5 - pz);
                ((-dx).atan2(dz) - look, Some((y as f64 + 0.5 - py) / dx.hypot(dz).max(1.0)))
            }
            P::Chunk(x, z) => {
                let (dx, dz) = (x as f64 * 16.0 + 8.0 - px, z as f64 * 16.0 + 8.0 - pz);
                ((-dx).atan2(dz) - look, None)
            }
            P::Azimuth(a) => (a as f64 - look, None),
        };
        let wrapped = (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        let across = (wrapped.to_degrees() / 60.0).clamp(-1.0, 1.0) as f32;
        let x = rect.center().x + across * (rect.width() / 2.0 - 6.0);
        let color = w.color.map_or_else(|| waypoint_color(id), |c| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
        let centre = Pos2::new(x, rect.center().y);
        p.circle_filled(centre, 5.0, Color32::BLACK);
        p.circle_filled(centre, 4.0, color);
        // A small mark says it's well above or below you.
        if let Some(h) = height.filter(|h| h.abs() > 0.5) {
            let tip = if h > 0.0 { centre - Vec2::new(0.0, 9.0) } else { centre + Vec2::new(0.0, 9.0) };
            let base = if h > 0.0 { tip.y + 3.0 } else { tip.y - 3.0 };
            p.add(egui::Shape::convex_polygon(vec![tip, Pos2::new(tip.x + 3.0, base), Pos2::new(tip.x - 3.0, base)], Color32::WHITE, Stroke::NONE));
        }
    }
}

/// A steady colour for a point the server didn't colour.
fn waypoint_color(id: &lattice_proto::packets::play::WaypointId) -> Color32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    let v = h.finish();
    let hue = (v % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.65, 0.95, 1.0).into()
}

fn vitals(p: &Painter, screen: Rect, hotbar_top: f32, session: &Session) -> f32 {
    let v = session.vitals;
    let total = hotbar_width();
    let width = (total - 16.0) / 2.0;
    let x0 = screen.center().x - total / 2.0;
    let right = x0 + total - width;
    // Experience (or a mount's leap) just above the hotbar.
    let xp = Rect::from_min_size(Pos2::new(x0, hotbar_top - 9.0), Vec2::new(total, 5.0));
    p.rect_filled(xp, CornerRadius::ZERO, Color32::from_black_alpha(170));
    let mut xf = xp;
    let leap = session.player.ride.as_ref().filter(|r| r.can_charge_jump());
    if let Some(ride) = leap {
        xf.set_width(xp.width() * ride.jump_scale.clamp(0.0, 1.0));
        p.rect_filled(xf, CornerRadius::ZERO, if ride.cooldown > 0 { ARMOUR } else { FOOD });
    } else {
        xf.set_width(xp.width() * v.xp_bar.clamp(0.0, 1.0));
        p.rect_filled(xf, CornerRadius::ZERO, EXPERIENCE);
    }
    locator_bar(p, Rect::from_min_size(Pos2::new(x0, hotbar_top - 22.0), Vec2::new(total, 11.0)), session);
    let row_h = 10.0;
    let row_step = 32.0;
    let lifted = if session.waypoints.is_empty() { 0.0 } else { 15.0 };
    let mut y = xp.top() - 6.0 - row_h - lifted;

    let max = session.player.max_health.max(1.0) as f32;
    let name = if session.hardcore { "Health (hardcore)" } else { "Health" };
    let extra = if session.absorption > 0.0 { format!(" +{:.0}", session.absorption.ceil()) } else { String::new() };
    let health = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(width, row_h));
    cell_bar(p, health, v.health / max, if session.absorption > 0.0 { ABSORPTION } else { HEALTH }, &format!("{name} {:.0} of {max:.0}{extra}", v.health.max(0.0).ceil()));
    let food = Rect::from_min_size(Pos2::new(right, y), Vec2::new(width, row_h));
    // An animal being ridden shows its health where food usually goes.
    let ridden = session.player.vehicle.and_then(|id| session.entities.map.get(&id)).and_then(|e| {
        let health = e.float_field("DATA_HEALTH_ID")?;
        let max = crate::game::riding::attr(e, "max_health", 20.0) as f32;
        Some((health, max))
    });
    if let Some((health, max)) = ridden {
        cell_bar(p, food, health / max.max(1.0), HEALTH, &format!("Mount {:.0} of {:.0}", health.max(0.0).ceil(), max));
    } else {
        cell_bar(p, food, v.food as f32 / 20.0, FOOD, &format!("Food {} of 20", v.food));
    }
    if v.xp_level > 0 {
        // On the label line, at the far end of the food column, clear of both bars.
        hud_text(p, Pos2::new(x0 + total, y - 2.0), Align2::RIGHT_BOTTOM, format!("Level {}", v.xp_level), font_bold(SIZE_SMALL), EXPERIENCE);
    }
    // Armour and breath only appear when they mean something.
    let armour = session.player.armor;
    let short_of_air = session.air_supply < 300;
    if armour > 0.0 || short_of_air {
        y -= row_step;
    }
    if armour > 0.0 {
        let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(width, row_h));
        cell_bar(p, r, armour as f32 / 20.0, ARMOUR, &format!("Armour {armour:.0} of 20"));
    }
    if short_of_air {
        let r = Rect::from_min_size(Pos2::new(right, y), Vec2::new(width, row_h));
        let left = session.air_supply.max(0) as f32 / 300.0;
        cell_bar(p, r, left, AIR, &format!("Air {:.0} s", session.air_supply.max(0) as f32 / 20.0));
    }
    y - 22.0
}

fn action_bar(p: &Painter, screen: Rect, above: f32, session: &Session, tr: &dyn Translator) {
    let Some((text, at)) = &session.titles.action_bar else { return };
    let age = at.elapsed().as_secs_f32();
    if age > 3.0 {
        return;
    }
    let alpha = ((3.0 - age) / 0.5).clamp(0.0, 1.0);
    let job = widgets::spans_job(&text.spans(tr), SIZE_BODY, ink(), screen.width() * 0.8);
    let g = p.layout_job(job);
    let pos = Pos2::new(screen.center().x - g.size().x / 2.0, above - g.size().y - 8.0);
    p.rect_filled(Rect::from_min_size(pos - Vec2::new(8.0, 3.0), g.size() + Vec2::new(16.0, 6.0)), CornerRadius::ZERO, backing((150.0 * alpha) as u8));
    p.galley(pos, g, ink().gamma_multiply(alpha));
}

fn titles(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator) {
    let t = &session.titles;
    let Some(title) = &t.title else { return };
    let total = t.fade_in + t.stay + t.fade_out;
    if t.age > total {
        return;
    }
    let alpha = if t.age < t.fade_in {
        t.age as f32 / t.fade_in.max(1) as f32
    } else if t.age > t.fade_in + t.stay {
        1.0 - (t.age - t.fade_in - t.stay) as f32 / t.fade_out.max(1) as f32
    } else {
        1.0
    };
    let job = widgets::spans_job(&title.spans(tr), 44.0, ink(), screen.width() * 0.9);
    let g = p.layout_job(job);
    let c = screen.center() - Vec2::new(0.0, screen.height() * 0.12);
    p.galley(c - g.size() / 2.0, g, ink().gamma_multiply(alpha));
    if let Some(sub) = &t.subtitle {
        let job = widgets::spans_job(&sub.spans(tr), SIZE_LEAD, ink(), screen.width() * 0.9);
        let g = p.layout_job(job);
        p.galley(c + Vec2::new(-g.size().x / 2.0, 34.0), g, ink().gamma_multiply(alpha));
    }
}

/// Live frame rate in the top right. Returns where the next thing below it can start.
fn frame_rate(app: &App, p: &Painter, screen: Rect) -> f32 {
    let f = &app.frame;
    let note = if !app.focused && app.config.video.limit_background_fps {
        "background limit"
    } else if app.focused && app.config.video.vsync {
        "VSync"
    } else {
        ""
    };
    let fps = p.layout_no_wrap(format!("{:.0} fps", f.fps), font_bold(SIZE_BODY), ink());
    let times = p.layout_no_wrap(format!("{:.1} ms, worst {:.1}", f.frame_ms, f.worst_ms), font(SIZE_SMALL), ink_muted());
    let note = p.layout_no_wrap(note.to_string(), font(SIZE_SMALL), ink_muted());
    let gap = 10.0;
    let note_w = if note.size().x > 0.0 { note.size().x + gap } else { 0.0 };
    let w = fps.size().x + gap + times.size().x + note_w + 20.0;
    let r = Rect::from_min_size(Pos2::new(screen.right() - w - 12.0, screen.top() + 12.0), Vec2::new(w, 26.0));
    p.rect_filled(r, CornerRadius::ZERO, backing(170));
    let mut x = r.left() + 10.0;
    let centre = r.center().y;
    let fw = fps.size();
    p.galley(Pos2::new(x, centre - fw.y / 2.0), fps, ink());
    x += fw.x + gap;
    let tw = times.size();
    p.galley(Pos2::new(x, centre - tw.y / 2.0), times, ink_muted());
    x += tw.x + gap;
    let nh = note.size().y;
    p.galley(Pos2::new(x, centre - nh / 2.0), note, ink_muted());
    r.bottom() + 8.0
}

fn notices(p: &Painter, screen: Rect, session: &Session, top: f32) {
    let mut y = top;
    for (text, at) in &session.notices {
        if at.elapsed().as_secs() > 10 {
            continue;
        }
        let g = p.layout(text.clone(), font(SIZE_SMALL), ink(), 360.0);
        let r = Rect::from_min_size(Pos2::new(screen.right() - g.size().x - 40.0, y), g.size() + Vec2::new(24.0, 14.0));
        p.rect_filled(r, CornerRadius::ZERO, backing(210));
        p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, edge()), egui::StrokeKind::Inside);
        p.galley(r.min + Vec2::new(14.0, 7.0), g, ink());
        y = r.bottom() + 6.0;
    }
}

/// The list of what could be typed next, just above the box.
fn suggestion_list(app: &mut App, ui: &mut egui::Ui, screen: Rect, width: f32, move_cursor: &mut Option<usize>) {
    const ROWS: usize = 10;
    let row = 22.0;
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("suggestions")));
    let s = &app.overlays.suggest;
    let first = s.selected.saturating_sub(ROWS - 1).min(s.list.len().saturating_sub(ROWS));
    let visible = &s.list[first..(first + ROWS).min(s.list.len())];
    let galleys: Vec<_> = visible.iter().map(|(t, _)| p.layout_no_wrap(t.clone(), font(SIZE_BODY), ink())).collect();
    let widest = galleys.iter().map(|g| g.size().x).fold(60.0f32, f32::max);
    // Lines up with where the word being completed starts.
    let prefix: String = app.overlays.chat_input.chars().take(s.start).collect();
    let indent = p.layout_no_wrap(prefix, font(SIZE_BODY), ink()).size().x;
    let x = (screen.left() + 20.0 + indent).min(screen.left() + 12.0 + width - widest - 16.0).max(screen.left() + 12.0);
    let height = galleys.len() as f32 * row + 6.0;
    let panel = Rect::from_min_size(Pos2::new(x, screen.bottom() - 58.0 - height), Vec2::new(widest + 16.0, height));
    p.rect_filled(panel, CornerRadius::ZERO, backing(240));
    let selected = s.selected;
    let list_len = s.list.len();
    let mut chosen = None;
    let mut tip = None;
    for (i, g) in galleys.into_iter().enumerate() {
        let index = first + i;
        let r = Rect::from_min_size(Pos2::new(panel.left(), panel.top() + 3.0 + i as f32 * row), Vec2::new(panel.width(), row));
        let resp = ui.interact(r, egui::Id::unique(("suggestion", index)), egui::Sense::click());
        crate::dev::note_widget(&format!("suggestion:{}", s.list[index].0), r);
        if index == selected {
            p.rect_filled(r, CornerRadius::ZERO, accent_dim());
            tip = s.list[index].1.clone();
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::ZERO, raised());
        }
        if resp.clicked() {
            chosen = Some(index);
        }
        let h = g.size().y;
        p.galley(Pos2::new(r.left() + 8.0, r.center().y - h / 2.0), g, ink());
    }
    if list_len > ROWS {
        p.text(Pos2::new(panel.right() + 6.0, panel.bottom()), Align2::LEFT_BOTTOM, format!("{} of {list_len}", selected + 1), font(SIZE_SMALL), ink_muted());
    }
    if let Some(tip) = tip {
        let g = p.layout_no_wrap(tip, font(SIZE_SMALL), ink_muted());
        let at = Pos2::new(panel.left(), panel.top() - g.size().y - 8.0);
        p.rect_filled(Rect::from_min_size(at, g.size() + Vec2::new(12.0, 6.0)), CornerRadius::ZERO, backing(230));
        p.galley(at + Vec2::new(6.0, 3.0), g, ink_muted());
    }
    if let Some(index) = chosen {
        app.overlays.suggest.selected = index;
        *move_cursor = accept_suggestion(app);
    }
}

/// Hover text for a span under the pointer, and what clicking it does.
fn use_chat_span(app: &mut App, ctx: &egui::Context, screen: Rect, hit: lattice_proto::Span, tr: &dyn Translator) {
    use lattice_proto::text::Click;
    if let (Some(text), Some(at)) = (&hit.hover, ctx.pointer_latest_pos()) {
        let tip = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("chat_tip")));
        let lines: Vec<(String, Color32)> = text.to_plain(tr).lines().map(|l| (l.to_string(), ink())).collect();
        super::items::tooltip(&tip, at, screen, &lines);
    }
    if !ctx.input(|i| i.pointer.primary_clicked()) {
        return;
    }
    let Some(click) = hit.click else { return };
    match click {
        Click::OpenUrl(url) => {
            if !app.config.chat.links || !(url.starts_with("http://") || url.starts_with("https://")) {
                return;
            }
            if app.config.chat.link_prompt {
                app.overlays.chat_open = false;
                if let Some(s) = &mut app.session {
                    s.modal = Some(crate::game::Modal::Link { url });
                    s.screen_opened = true;
                }
            } else {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }
        Click::RunCommand(command) => {
            if let Some(s) = &mut app.session {
                let line = if command.starts_with('/') { command } else { format!("/{command}") };
                if let Err(e) = s.send_chat(&line) {
                    app.overlays.chat_error = Some(e);
                    return;
                }
            }
            app.overlays.chat_open = false;
            app.capture_mouse();
        }
        Click::SuggestCommand(text) => {
            app.overlays.chat_input = text;
            app.overlays.focus_chat = true;
        }
        Click::Copy(text) => ctx.copy_text(text),
        Click::ChangePage(_) => {}
    }
}

fn chat_lines(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator, open: bool, c: &crate::config::Chat, pointer: Option<Pos2>) -> Option<lattice_proto::Span> {
    let width = (c.width as f32).min(screen.width() - 24.0);
    let mut y = screen.bottom() - if open { 64.0 } else { 120.0 };
    let max_lines = if open { c.lines_open } else { c.lines_closed } as usize;
    let text_alpha = c.opacity as f32 / 100.0;
    let back_alpha = c.background as f32 / 100.0;
    let mut hit = None;
    for line in session.chat.lines.iter().rev().filter(|l| l.at.elapsed().as_secs_f32() >= c.delay).take(max_lines) {
        let age = line.at.elapsed().as_secs_f32() - c.delay;
        let alpha = if open { 1.0 } else { (c.fade_after - age).clamp(0.0, 1.0) };
        if alpha <= 0.0 {
            break;
        }
        let spans = line.text.spans(tr);
        let spans: Vec<_> = if c.colors { spans } else { spans.into_iter().map(|mut s| { s.color = None; s }).collect() };
        let job = widgets::spans_job(&spans, c.text_size, ink(), width - 16.0);
        let g = p.layout_job(job);
        let r = Rect::from_min_size(Pos2::new(screen.left() + 12.0, y - g.size().y - 6.0 - c.line_spacing), Vec2::new(width, g.size().y + 6.0 + c.line_spacing));
        p.rect_filled(r, CornerRadius::ZERO, backing((255.0 * back_alpha * alpha) as u8));
        // With chat open, what's under the pointer can be used.
        if let (true, Some(q)) = (open, pointer) {
            let origin = r.min + Vec2::new(8.0, 3.0);
            let inside = Rect::from_min_size(origin, g.size()).contains(q);
            if inside {
                let at = g.cursor_from_pos(q - origin).index.0;
                let mut passed = 0;
                hit = spans.iter().find(|s| {
                    passed += s.text.chars().count();
                    at < passed
                }).filter(|s| s.click.is_some() || s.hover.is_some()).cloned();
            }
        }
        p.galley(r.min + Vec2::new(8.0, 3.0), g, ink().gamma_multiply(alpha * text_alpha));
        y = r.top();
    }
    hit
}

/// How bright a light level looks, by the game's curve, with a dimension's own ambient light.
pub fn light_brightness(level: u8, ambient: f32) -> f32 {
    let f = level as f32 / 15.0;
    let curve = f / (4.0 - 3.0 * f);
    ambient + (1.0 - ambient) * curve
}

/// Whole-screen pictures: dark edges, a pumpkin on the head, a spyglass, frost, a portal.
fn screen_overlays(app: &mut App, ctx: &egui::Context, p: &Painter, screen: Rect) {
    let Some(assets) = app.ready_assets() else { return };
    let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if app.config.video.vignette {
        let alpha = (app.vignette.clamp(0.0, 1.0) * 255.0) as u8;
        if alpha > 0 {
            if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "misc/vignette") {
                p.image(t, screen, full_uv, Color32::from_white_alpha(alpha));
            }
        }
    }
    let Some(session) = app.session.as_ref() else { return };
    let scoping = app.perspective == 0
        && session.player.using.is_some_and(|hand| {
            let stack = if hand == 0 { session.player.shown_main.as_ref() } else { session.player.shown_off.as_ref() };
            stack.is_some_and(|s| crate::game::items::item_name(s) == "spyglass")
        });
    let pumpkin = app.perspective == 0 && session.inventory.slots.get(crate::game::inventory::ARMOR_HEAD).and_then(|s| s.as_ref()).is_some_and(|s| crate::game::items::camera_overlay(s).is_some());
    let frozen = (session.frozen_ticks.min(140) as f32 / 140.0).clamp(0.0, 1.0);
    let partial = app.tick_partial();
    let portal = session.portal_effect.0 + (session.portal_effect.1 - session.portal_effect.0) * partial;
    let ticks = session.ticks;
    // The scope opens up quickly as it's raised.
    if scoping {
        app.scope_scale += (1.125 - app.scope_scale) * 0.5 * app.frame_ticks();
        let side = screen.width().min(screen.height()) * app.scope_scale;
        let r = Rect::from_center_size(screen.center(), Vec2::splat(side));
        if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "misc/spyglass_scope") {
            p.image(t, r, full_uv, Color32::WHITE);
        }
        for bar in [
            Rect::from_min_max(screen.min, Pos2::new(screen.right(), r.top())),
            Rect::from_min_max(Pos2::new(screen.left(), r.bottom()), screen.max),
            Rect::from_min_max(Pos2::new(screen.left(), r.top()), Pos2::new(r.left(), r.bottom())),
            Rect::from_min_max(Pos2::new(r.right(), r.top()), Pos2::new(screen.right(), r.bottom())),
        ] {
            p.rect_filled(bar, CornerRadius::ZERO, Color32::BLACK);
        }
    } else {
        app.scope_scale = 0.5;
        if pumpkin {
            if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "misc/pumpkinblur") {
                p.image(t, screen, full_uv, Color32::WHITE);
            }
        }
    }
    if frozen > 0.0 {
        if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "misc/powder_snow_outline") {
            p.image(t, screen, full_uv, Color32::from_white_alpha((frozen * 255.0) as u8));
        }
    }
    // With the warping turned down, a pale tint says the same thing without moving.
    let scale = 1.0 - app.config.video.distortion as f32 / 100.0;
    let queasy = session.player.effect_duration("nausea").map_or(0.0, |d| if d < 0 { 1.0 } else { (d as f32 / 60.0).min(1.0) });
    if scale > 0.0 && (queasy > 0.0 || portal > 0.0) {
        if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "misc/nausea") {
            let strength = queasy.max(portal) * scale;
            p.image(t, screen, full_uv, Color32::from_white_alpha((strength * 255.0) as u8));
        }
    }
    if portal > 0.0 {
        let mut a = portal;
        if a < 1.0 {
            a = a * a;
            a = a * a;
            a = a * 0.8 + 0.2;
        }
        if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, "block/nether_portal") {
            // A strip of 32 frames, one after another.
            let frame = (ticks % 32) as f32;
            let uv = Rect::from_min_max(Pos2::new(0.0, frame / 32.0), Pos2::new(1.0, (frame + 1.0) / 32.0));
            p.image(t, screen, uv, Color32::from_white_alpha((a * 255.0) as u8));
        }
    }
}

/// What's affecting the player, under the frame rate: good things, then bad.
fn effect_icons(app: &mut App, ctx: &egui::Context, p: &Painter, screen: Rect, tr: &dyn Translator) {
    let Some(assets) = app.ready_assets() else { return };
    let Some(session) = app.session.as_ref() else { return };
    let game = lattice_data::game();
    let mut list: Vec<(String, bool, i32, i32, u8)> = session
        .player
        .effects
        .iter()
        .filter(|(_, v)| v.2 & 4 != 0)
        .filter_map(|(id, v)| {
            let e = game.mob_effects.get(*id as usize)?;
            Some((e.name.clone(), e.category != "harmful", v.0, v.1, v.2))
        })
        .collect();
    if list.is_empty() {
        return;
    }
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let pointer = ctx.pointer_latest_pos();
    const TILE: f32 = 36.0;
    let top = if app.overlays.debug { screen.top() + 16.0 } else { screen.top() + 44.0 };
    let mut tip = None;
    for (row, good) in [(0, true), (1, false)] {
        let mut x = screen.right() - TILE - 12.0;
        for (name, is_good, amp, ticks, _) in list.iter().filter(|e| e.1 == good) {
            let _ = is_good;
            let r = Rect::from_min_size(Pos2::new(x, top + row as f32 * (TILE + 30.0)), Vec2::splat(TILE));
            super::chrome::hud_slot(p, r);
            // About to end: it blinks, as it does in the game.
            let ending = *ticks >= 0 && *ticks <= 200;
            let bright = if ending { let t = *ticks as f32; 0.5 + 0.5 * (t * std::f32::consts::PI / 5.0).cos().abs() } else { 1.0 };
            if let Some(tex) = app.screen_pictures.get_or_make(ctx, &assets, &format!("mob_effect/{name}")) {
                p.image(tex, r.shrink(5.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(bright));
            }
            let time = if *ticks < 0 { "∞".to_string() } else { let s = *ticks / 20; if s >= 60 { format!("{}:{:02}", s / 60, s % 60) } else { format!("{s}") } };
            p.text(Pos2::new(r.center().x, r.bottom() + 1.0), Align2::CENTER_TOP, time, font(11.0), ink_muted());
            if pointer.is_some_and(|q| r.contains(q)) {
                let key = format!("effect.minecraft.{name}");
                let shown = tr.translate(&key).map_or(name.clone(), str::to_string);
                let level = if *amp > 0 { format!(" {}", amp + 1) } else { String::new() };
                tip = Some(format!("{shown}{level}"));
            }
            x -= TILE + 4.0;
        }
    }
    if let (Some(text), Some(q)) = (tip, pointer) {
        let layer = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("effect_tip")));
        super::items::tooltip(&layer, q, screen, &[(text, ink())]);
    }
}

/// A note in the corner when something's been achieved.
fn achieved(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator) {
    let mut y = screen.top() + 52.0;
    for (title, at) in &session.achieved {
        let age = at.elapsed().as_secs_f32();
        if age > 6.0 {
            continue;
        }
        let heading = p.layout_no_wrap("Advancement made".to_string(), font(SIZE_SMALL), ink_muted());
        let name = p.layout_job(widgets::spans_job(&title.spans(tr), SIZE_BODY, ink(), 300.0));
        let w = heading.size().x.max(name.size().x) + 28.0;
        let lines = heading.size().y + name.size().y;
        let r = Rect::from_min_size(Pos2::new(screen.right() - w - 12.0, y), Vec2::new(w, lines + 18.0));
        super::chrome::tip_box(p, r);
        let below = r.min + Vec2::new(14.0, 9.0 + heading.size().y);
        p.galley(r.min + Vec2::new(14.0, 9.0), heading, ink_muted());
        p.galley(below, name, ink());
        y = r.bottom() + 6.0;
    }
}

/// Something just heard, written down.
pub struct Subtitle {
    pub key: String,
    pub pos: Option<glam::Vec3>,
    pub at: std::time::Instant,
}

/// What can be heard, lower right, with which side it's on.
fn subtitles(app: &App, p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator) {
    let list = &app.overlays.subtitles;
    if list.is_empty() {
        return;
    }
    let (yaw, _) = app.look;
    let (right_x, right_z) = (-yaw.to_radians().cos(), -yaw.to_radians().sin());
    let eye = session.player.body.pos.as_vec3() + glam::Vec3::new(0.0, session.player.eye_height, 0.0);
    let rows: Vec<(String, f32, i8)> = list
        .iter()
        .rev()
        .take(10)
        .map(|s| {
            let text = tr.translate(&s.key).map_or_else(|| s.key.clone(), str::to_string);
            let fade = (1.0 - (s.at.elapsed().as_secs_f32() - 1.5) / 1.5).clamp(0.3, 1.0);
            let side = s.pos.map_or(0, |pos| {
                let d = pos - eye;
                let along = (d.x * right_x + d.z * right_z) / (d.x * d.x + d.z * d.z).sqrt().max(0.001);
                if along > 0.5 {
                    1
                } else if along < -0.5 {
                    -1
                } else {
                    0
                }
            });
            (text, fade, side)
        })
        .collect();
    let galleys: Vec<_> = rows.iter().map(|(t, ..)| p.layout_no_wrap(t.clone(), font(SIZE_SMALL), ink())).collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 44.0;
    let row_h = 20.0;
    let mut y = screen.bottom() - 60.0;
    let x = screen.right() - width - 8.0;
    for (g, (_, fade, side)) in galleys.into_iter().zip(&rows).rev() {
        let r = Rect::from_min_size(Pos2::new(x, y - row_h), Vec2::new(width, row_h));
        p.rect_filled(r, CornerRadius::ZERO, backing((180.0 * fade) as u8));
        let color = ink().gamma_multiply(*fade);
        let w = g.size().x;
        let h = g.size().y;
        p.galley_with_override_text_color(Pos2::new(r.center().x - w / 2.0, r.center().y - h / 2.0), g, color);
        // A small chevron, as the interface's typeface has no arrows.
        let mark = |c: Pos2, dir: f32| {
            let stroke = Stroke::new(1.5, color);
            p.line_segment([c + Vec2::new(-3.0 * dir, -4.0), c + Vec2::new(3.0 * dir, 0.0)], stroke);
            p.line_segment([c + Vec2::new(-3.0 * dir, 4.0), c + Vec2::new(3.0 * dir, 0.0)], stroke);
        };
        if *side < 0 {
            mark(Pos2::new(r.left() + 10.0, r.center().y), -1.0);
        } else if *side > 0 {
            mark(Pos2::new(r.right() - 10.0, r.center().y), 1.0);
        }
        y -= row_h;
    }
}

/// What tab completion has to offer while typing.
#[derive(Default)]
pub struct Suggest {
    /// The text last asked about, and which question that was.
    asked: String,
    transaction: i32,
    /// Offered replacements with an explanation, for the part of the text at `start`.
    pub list: Vec<(String, Option<String>)>,
    start: usize,
    length: usize,
    selected: usize,
    /// Esc closed the list, so it stays away until the text changes.
    dismissed: bool,
    /// That key press has been dealt with, so the chat box mustn't close on it too.
    esc_taken: bool,
}

impl Suggest {
    /// Esc closes an open list before anything else. True if there was one.
    pub fn take_escape(&mut self) -> bool {
        if self.dismissed || self.list.is_empty() {
            return false;
        }
        self.dismissed = true;
        self.esc_taken = true;
        true
    }
}

/// Asks the server about what's typed, and takes in whatever it's answered.
fn update_suggestions(app: &mut App, tr: &dyn Translator) {
    let text = app.overlays.chat_input.clone();
    let Some(session) = app.session.as_mut() else { return };
    let s = &mut app.overlays.suggest;
    if text != s.asked {
        s.asked = text.clone();
        // Without suggestions as you type, the list waits for Tab.
        s.dismissed = !app.config.chat.command_suggestions;
        s.selected = 0;
        s.list.clear();
        s.transaction += 1;
        if text.starts_with('/') {
            session.conn.send(lattice_proto::packets::play::command_suggestion(s.transaction, &text));
        } else {
            // Plain chat: the names of players, for the word being typed.
            let word_start = text.rfind(' ').map_or(0, |i| i + 1);
            let word = text[word_start..].to_lowercase();
            if !word.is_empty() {
                let mut names: Vec<String> = session.tab.values().map(|e| e.name.clone()).chain(session.chat_completions.iter().cloned()).filter(|n| n.to_lowercase().starts_with(&word)).collect();
                names.sort_by_key(|n| n.to_lowercase());
                s.list = names.into_iter().map(|n| (n, None)).collect();
                s.start = word_start;
                s.length = text.len() - word_start;
            }
        }
    }
    if let Some((id, start, length, entries)) = session.suggestions.take() {
        if id == s.transaction {
            s.list = entries.into_iter().map(|(t, tip)| (t, tip.map(|x| x.to_plain(tr)))).collect();
            s.start = start.max(0) as usize;
            s.length = length.max(0) as usize;
            s.selected = 0;
        }
    }
}

/// Puts the chosen suggestion into the text.
fn accept_suggestion(app: &mut App) -> Option<usize> {
    let s = &mut app.overlays.suggest;
    let (choice, _) = s.list.get(s.selected)?.clone();
    let text = &app.overlays.chat_input;
    // The server counts in UTF-16 units; for what can be typed into a command these are chars.
    let chars: Vec<char> = text.chars().collect();
    let start = s.start.min(chars.len());
    let end = (s.start + s.length).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.push_str(&choice);
    let cursor = out.chars().count();
    out.extend(&chars[end..]);
    app.overlays.chat_input = out;
    Some(cursor)
}

fn chat_input(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let mut send = None;
    let mut close = false;
    let width = (app.config.chat.width as f32).min(screen.width() - 24.0);
    if let crate::assets::AssetState::Ready(a) = &app.assets {
        let a = a.clone();
        update_suggestions(app, &a.lang);
    }
    if std::mem::take(&mut app.overlays.suggest.esc_taken) {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    }
    let showing = !app.overlays.suggest.dismissed && !app.overlays.suggest.list.is_empty();
    // Keys the list takes, before the text box can see them.
    let (tab, shift_tab, list_up, list_down, list_esc) = ui.input_mut(|i| {
        let tab = i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
        let shift_tab = i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab);
        if showing {
            (tab, shift_tab, i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        } else {
            (tab, shift_tab, false, false, false)
        }
    });
    {
        let s = &mut app.overlays.suggest;
        let n = s.list.len();
        if n > 0 {
            if list_up || shift_tab {
                s.selected = (s.selected + n - 1) % n;
            }
            if list_down {
                s.selected = (s.selected + 1) % n;
            }
        }
        if list_esc {
            s.dismissed = true;
        }
    }
    let mut move_cursor = None;
    if tab && app.overlays.suggest.dismissed {
        // The list was waiting to be asked for.
        app.overlays.suggest.dismissed = false;
    } else if tab {
        move_cursor = accept_suggestion(app);
    }
    if showing && !app.overlays.suggest.dismissed {
        suggestion_list(app, ui, screen, width, &mut move_cursor);
    }
    egui::Area::new(egui::Id::unique("chat_input"))
        .fixed_pos(Pos2::new(screen.left() + 12.0, screen.bottom() - 52.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new().fill(backing(220)).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(width);
                if let Some(e) = &app.overlays.chat_error {
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xF0, 0x9A, 0x8A)).font(font(SIZE_SMALL)));
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.overlays.chat_input)
                        .desired_width(width - 16.0)
                        .font(font(SIZE_BODY))
                        .frame(egui::Frame::NONE)
                        .hint_text(RichText::new("Message, or / for a command").color(ink_muted()))
                        .lock_focus(true)
                        .char_limit(256),
                );
                if let Some(at) = move_cursor {
                    r.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
                        state.store(ui.ctx(), r.id);
                    }
                }
                if app.overlays.focus_chat {
                    r.request_focus();
                    app.overlays.focus_chat = false;
                    // Put the cursor after any prefilled "/".
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        let end = egui::text::CCursor::new(app.overlays.chat_input.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), r.id);
                    }
                }
                let (enter, esc, up, down) = ui.input(|i| {
                    (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown))
                });
                if enter {
                    send = Some(app.overlays.chat_input.clone());
                }
                if esc {
                    close = true;
                }
                if let Some(s) = &app.session {
                    let hist = &s.chat.sent;
                    if up && !hist.is_empty() {
                        let pos = app.overlays.chat_history_pos.map_or(hist.len() - 1, |p| p.saturating_sub(1));
                        app.overlays.chat_history_pos = Some(pos);
                        app.overlays.chat_input = hist[pos].clone();
                    }
                    if down {
                        if let Some(p) = app.overlays.chat_history_pos {
                            if p + 1 < hist.len() {
                                app.overlays.chat_history_pos = Some(p + 1);
                                app.overlays.chat_input = hist[p + 1].clone();
                            } else {
                                app.overlays.chat_history_pos = None;
                                app.overlays.chat_input.clear();
                            }
                        }
                    }
                }
            });
        });
    if let Some(line) = send {
        if let Some(s) = &mut app.session {
            match s.send_chat(&line) {
                Ok(()) => {
                    if !line.trim().is_empty() {
                        s.chat.remember_sent(line.trim());
                    }
                    close = true;
                }
                Err(e) => app.overlays.chat_error = Some(e),
            }
        }
    }
    if close {
        app.overlays.chat_open = false;
        app.overlays.chat_input.clear();
        app.capture_mouse();
    }
}

/// Bars for bosses and whatever else the server wants one for, at the top in the middle.
fn boss_bars(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator) {
    // Pink, blue, red, green, yellow, purple, white.
    const COLORS: [Color32; 7] = [
        Color32::from_rgb(0xE8, 0x6A, 0xC8),
        Color32::from_rgb(0x4F, 0xA8, 0xE8),
        Color32::from_rgb(0xD8, 0x4A, 0x3C),
        Color32::from_rgb(0x5C, 0xC8, 0x5C),
        Color32::from_rgb(0xE6, 0xCE, 0x48),
        Color32::from_rgb(0xA0, 0x5C, 0xD8),
        Color32::from_rgb(0xE8, 0xE8, 0xE8),
    ];
    let width = 364.0f32.min(screen.width() - 40.0);
    let mut y = screen.top() + 12.0;
    for (_, bar) in &session.boss_bars {
        if y > screen.height() / 3.0 {
            break;
        }
        let spans = bar.name.spans(tr);
        let g = p.layout_job(widgets::spans_job(&spans, SIZE_SMALL, ink(), width + 200.0));
        let size = g.size();
        let at = Pos2::new(screen.center().x - size.x / 2.0, y);
        p.rect_filled(Rect::from_min_size(at - Vec2::new(6.0, 1.0), size + Vec2::new(12.0, 2.0)), CornerRadius::ZERO, backing(150));
        p.galley(at, g, ink());
        y += size.y + 3.0;
        let track = Rect::from_min_size(Pos2::new(screen.center().x - width / 2.0, y), Vec2::new(width, 8.0));
        p.rect_filled(track, CornerRadius::ZERO, backing(210));
        let color = COLORS[bar.color.clamp(0, 6) as usize];
        let mut fill = track;
        fill.set_width(track.width() * bar.shown.clamp(0.0, 1.0));
        p.rect_filled(fill, CornerRadius::ZERO, color);
        // Progress, then six, ten, twelve or twenty notches.
        let notches = [0, 6, 10, 12, 20].get(bar.overlay.clamp(0, 4) as usize).copied().unwrap_or(0);
        for i in 1..notches {
            let x = track.left() + track.width() * i as f32 / notches as f32;
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.0, backing(220)));
        }
        y += 8.0 + 8.0;
    }
}

/// The scoreboard the server chose to show down the right side.
fn sidebar(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator) {
    let Some((title, rows)) = session.scoreboard.sidebar(tr) else { return };
    let title = p.layout_job(widgets::spans_job(&title, SIZE_SMALL, ink(), 400.0));
    let rows: Vec<_> = rows
        .iter()
        .map(|(name, score)| {
            (p.layout_job(widgets::spans_job(name, SIZE_SMALL, ink(), 400.0)), score.as_ref().map(|s| p.layout_job(widgets::spans_job(s, SIZE_SMALL, ink(), 200.0))))
        })
        .collect();
    let row_h = 18.0;
    let mut width = title.size().x;
    for (n, s) in &rows {
        width = width.max(n.size().x + s.as_ref().map_or(0.0, |s| s.size().x + 16.0));
    }
    let width = width + 16.0;
    let height = rows.len() as f32 * row_h + 26.0;
    let r = Rect::from_min_size(Pos2::new(screen.right() - width - 4.0, screen.center().y - height / 2.0), Vec2::new(width, height));
    p.rect_filled(r, CornerRadius::ZERO, backing(170));
    let title_top = Rect::from_min_size(r.min, Vec2::new(width, 24.0));
    p.rect_filled(title_top, CornerRadius::ZERO, backing(120));
    let tw = title.size();
    p.galley(Pos2::new(r.center().x - tw.x / 2.0, r.top() + 12.0 - tw.y / 2.0), title, ink());
    let mut y = r.top() + 26.0;
    for (n, s) in rows {
        p.galley(Pos2::new(r.left() + 8.0, y), n, ink());
        if let Some(s) = s {
            let sw = s.size().x;
            p.galley(Pos2::new(r.right() - 8.0 - sw, y), s, ink());
        }
        y += row_h;
    }
}

/// The little faces beside names in the player list: their own skin once it's
/// arrived, else the bundled look their id picks.
fn tab_faces(app: &mut App, ctx: &egui::Context) -> std::collections::HashMap<uuid::Uuid, egui::TextureId> {
    let mut out = std::collections::HashMap::new();
    let Some(assets) = app.ready_assets() else { return out };
    let Some(session) = app.session.as_ref() else { return out };
    for id in session.tab.keys().copied() {
        if let Some(img) = app.skins.face_of(id) {
            let t = app.face_textures.entry(id).or_insert_with(|| {
                let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                ctx.load_texture(format!("face {id}"), color, egui::TextureOptions::NEAREST)
            });
            out.insert(id, t.id());
        } else {
            let (name, _) = crate::bodies::mobs::default_skin(id);
            if let Some(t) = app.screen_pictures.get_or_make(ctx, &assets, &format!("face/{name}")) {
                out.insert(id, t);
            }
        }
    }
    out
}

fn tab_list(p: &Painter, screen: Rect, session: &Session, tr: &dyn Translator, faces: &std::collections::HashMap<uuid::Uuid, egui::TextureId>) {
    let mut players: Vec<(&uuid::Uuid, &crate::game::TabEntry)> = session.tab.iter().filter(|(_, e)| e.listed).collect();
    let board = &session.scoreboard;
    players.sort_by(|(_, a), (_, b)| {
        b.priority
            .cmp(&a.priority)
            .then((a.game_mode == 3).cmp(&(b.game_mode == 3)))
            .then(board.team_name_of(&a.name).cmp(board.team_name_of(&b.name)))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let width = 420.0f32.min(screen.width() - 40.0);
    let row = 26.0;
    let header = session.tab_header.as_ref().map(|h| p.layout_job(widgets::spans_job(&h.spans(tr), SIZE_BODY, ink(), width - 20.0)));
    let footer = session.tab_footer.as_ref().map(|h| p.layout_job(widgets::spans_job(&h.spans(tr), SIZE_SMALL, ink_muted(), width - 20.0)));
    let h = 16.0 + header.as_ref().map_or(0.0, |g| g.size().y + 10.0) + players.len().min(40) as f32 * row + footer.as_ref().map_or(0.0, |g| g.size().y + 10.0) + 12.0;
    let r = Rect::from_min_size(Pos2::new(screen.center().x - width / 2.0, screen.top() + 40.0), Vec2::new(width, h));
    p.rect_filled(r, CornerRadius::ZERO, backing(225));
    let mut y = r.top() + 10.0;
    if let Some(g) = header {
        let x = r.center().x - g.size().x / 2.0;
        let gh = g.size().y;
        p.galley(Pos2::new(x, y), g, ink());
        y += gh + 10.0;
    }
    for (id, e) in players.iter().take(40) {
        let face = Rect::from_min_size(Pos2::new(r.left() + 12.0, y), Vec2::splat(20.0));
        match faces.get(id) {
            Some(t) => {
                p.image(*t, face, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(face, CornerRadius::ZERO, raised());
            }
        }
        let name = e.display.as_ref().map(|d| d.spans(tr)).unwrap_or_else(|| board.decorated(&e.name, tr));
        let color = if e.game_mode == 3 { ink_muted() } else { ink() };
        let g = p.layout_job(widgets::spans_job(&name, SIZE_BODY, color, width - 180.0));
        p.galley(Pos2::new(r.left() + 40.0, y), g, color);
        let ping = if e.latency < 0 { "—".to_string() } else { format!("{} ms", e.latency) };
        p.text(Pos2::new(r.right() - 14.0, y + 1.0), Align2::RIGHT_TOP, ping, font(SIZE_SMALL), ink_muted());
        if let Some(score) = board.list_score(&e.name, tr) {
            let g = p.layout_job(widgets::spans_job(&score, SIZE_SMALL, ink(), 80.0));
            let w = g.size().x;
            p.galley(Pos2::new(r.right() - 76.0 - w, y + 1.0), g, ink());
        }
        y += row;
    }
    if let Some(g) = footer {
        p.galley(Pos2::new(r.center().x - g.size().x / 2.0, y + 6.0), g, ink_muted());
    }
}

fn debug(app: &App, p: &Painter, screen: Rect, session: &Session) {
    let b = &session.player.body;
    let data = lattice_data::game();
    let facing = match ((b.yaw.rem_euclid(360.0) + 45.0) / 90.0) as i32 % 4 {
        0 => "south (+Z)",
        1 => "west (−X)",
        2 => "north (−Z)",
        _ => "east (+X)",
    };
    let (bx, by, bz) = (b.pos.x.floor() as i32, b.pos.y.floor() as i32, b.pos.z.floor() as i32);
    let mut lines = vec![
        format!("Minecraft {} (protocol {}), Lattice {}", lattice_proto::GAME_VERSION, lattice_proto::PROTOCOL_VERSION, env!("CARGO_PKG_VERSION")),
        format!("{:.0} fps, {:.1} ms, worst {:.1} ms", app.frame.fps, app.frame.frame_ms, app.frame.worst_ms),
        format!("XYZ {:.3} / {:.3} / {:.3}", b.pos.x, b.pos.y, b.pos.z),
        format!("Block {bx} {by} {bz}   chunk {} {}", bx >> 4, bz >> 4),
        format!("Facing {facing}   yaw {:.1} pitch {:.1}", b.yaw, b.pitch),
        format!("On ground {}   velocity {:.3} {:.3} {:.3}", b.on_ground, b.vel.x, b.vel.y, b.vel.z),
    ];
    if let Some(w) = &session.world {
        let (sky, block) = w.light(bx, by, bz);
        lines.push(format!("Light sky {sky} block {block}   dimension {}", w.dimension.name));
        if let Some(biome) = session.biome_at(bx, by, bz) {
            lines.push(format!("Biome {}   temperature {:.2} downfall {:.2}", biome.name, biome.temperature, biome.downfall));
        }
        lines.push(format!("Columns {}   dirty sections {}", w.column_count(), w.dirty_count()));
        if let Some(hit) = session.player.target {
            if let Some(s) = w.block_at(hit.pos) {
                let props: Vec<String> = data.properties(s).iter().map(|(k, v)| format!("{k}={v}")).collect();
                lines.push(format!("Looking at {} {} {}: {}", hit.pos.x, hit.pos.y, hit.pos.z, data.block(s).name));
                if !props.is_empty() {
                    lines.push(format!("  {}", props.join(" ")));
                }
            }
        }
    }
    let st = app.renderer.stats;
    lines.push(format!("Sections drawn {} of {}   draw calls {}", st.visible, st.sections, st.draws));
    lines.push(format!("Meshing {}   entities {}", app.mesher.as_ref().map_or(0, |m| m.in_flight()), session.entities.map.len()));
    if let Some(me) = session.tab.get(&session.self_uuid) {
        lines.push(format!("Ping {} ms", me.latency));
    }
    lines.push(format!("Day time {:.0}", session.day_time.rem_euclid(24000.0)));
    lines.push(format!("GPU {} ({:?})", app.renderer.adapter_info.name, app.renderer.adapter_info.backend));
    let mut y = screen.top() + 10.0;
    for l in lines {
        let g = p.layout_no_wrap(l, egui::FontId::monospace(SIZE_SMALL), ink());
        let r = Rect::from_min_size(Pos2::new(screen.left() + 10.0, y), g.size() + Vec2::new(10.0, 4.0));
        p.rect_filled(r, CornerRadius::ZERO, backing(170));
        p.galley(r.min + Vec2::new(5.0, 2.0), g, ink());
        y = r.bottom();
    }
}

/// The dialogs over the world: centred, on a dimmed picture of the game.
fn modal_frame(ui: &mut egui::Ui, screen: Rect, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique(format!("{id}_dim"))));
    super::chrome::dim(&painter, screen, Color32::from_black_alpha(150));
    egui::Area::new(egui::Id::unique(id)).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ui.ctx(), |ui| {
        super::chrome::window(ui, 24, |ui| {
            ui.set_width(MODAL_W);
            add(ui);
        });
    });
}

/// A menu that stands straight on the dimmed world, with no box round it: a title
/// and a column of big buttons, as a game's menus are.
fn game_menu(ui: &mut egui::Ui, screen: Rect, id: &str, tint: Color32, add: impl FnOnce(&mut egui::Ui)) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique(format!("{id}_dim"))));
    super::chrome::dim(&painter, screen, tint);
    egui::Area::new(egui::Id::unique(id)).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ui.ctx(), |ui| {
        ui.set_width(MODAL_W);
        ui.spacing_mut().item_spacing.y = 8.0;
        // Keys of stone, with their words on the stone.
        on_panel(ui, add);
    });
}

/// The totem that just saved you, flying up the screen.
fn totem_flash(app: &mut App, ctx: &egui::Context, p: &Painter, screen: Rect) {
    let tex = app.item_texture(ctx);
    let Some(assets) = app.ready_assets() else { return };
    let Some(session) = app.session.as_ref() else { return };
    let Some(at) = session.totem_shown else { return };
    let t = at.elapsed().as_secs_f32() / 2.0;
    if t >= 1.0 {
        return;
    }
    ctx.request_repaint();
    let Some(id) = crate::game::items::item_id("totem_of_undying") else { return };
    let grown = (t * 2.0).min(1.0);
    let size = screen.height() * 0.5 * (0.4 + grown * 0.6);
    let rise = screen.height() * (0.25 - t * 0.9);
    let spot = Rect::from_center_size(screen.center() + Vec2::new(0.0, rise), Vec2::splat(size));
    let query = super::items::StackQuery { stack: &lattice_proto::ItemStack::new(id, 1), day_fraction: 0.0, using_ticks: None, context: "gui" };
    super::items::draw_icon(p, spot, tex, &assets, &query);
}

/// Lying in bed, with a way out: moving doesn't get you up.
fn in_bed(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let mut leave = false;
    let dim = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("bed_dim")));
    super::chrome::dim(&dim, screen, Color32::from_black_alpha(110));
    egui::Area::new(egui::Id::unique("bed")).order(egui::Order::Foreground).anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0)).show(ui.ctx(), |ui| {
        on_panel(ui, |ui| {
            if widgets::menu_button(ui, "Leave bed", true, 260.0).clicked() {
                leave = true;
            }
        });
    });
    if leave {
        if let Some(s) = &app.session {
            s.conn.send(lattice_proto::packets::play::player_command(s.player.entity_id, lattice_proto::packets::play::command::STOP_SLEEPING));
        }
    }
}

/// A title over the dimmed world, in light words whatever's around it.
fn world_title(ui: &mut egui::Ui, words: &str, size: f32) {
    let was = panel_mode(false);
    super::chrome::title(ui, words, size, super::chrome::TIP_INK);
    panel_mode(was);
}

const MODAL_W: f32 = 380.0;

fn pause_menu(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let name = app.spec.server_name.clone();
    let has_links = app.session.as_ref().is_some_and(|s| !s.server_links.is_empty());
    let can_edit_rules = app.session.as_ref().is_some_and(|s| s.player.permission >= 2);
    let mut action = 0;
    game_menu(ui, screen, "pause", Color32::from_black_alpha(150), |ui| {
        world_title(ui, "Game paused", SIZE_TITLE);
        world_title(ui, &format!("On {name}"), SIZE_BODY);
        ui.add_space(16.0);
        if widgets::menu_button(ui, "Back to game", true, MODAL_W).clicked() {
            action = 1;
        }
        if widgets::menu_button(ui, "Settings", false, MODAL_W).clicked() {
            action = 2;
        }
        ui.horizontal(|ui| {
            let half = (MODAL_W - ui.spacing().item_spacing.x) / 2.0;
            if widgets::menu_button(ui, "Advancements", false, half).clicked() {
                action = 4;
            }
            if widgets::menu_button(ui, "Statistics", false, half).clicked() {
                action = 5;
            }
        });
        if can_edit_rules && widgets::menu_button(ui, "Game rules", false, MODAL_W).clicked() {
            action = 7;
        }
        if has_links && widgets::menu_button(ui, "Server links", false, MODAL_W).clicked() {
            action = 6;
        }
        ui.add_space(14.0);
        if widgets::menu_button(ui, "Disconnect and close", false, MODAL_W).clicked() {
            action = 3;
        }
    });
    let version = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("version")));
    super::chrome::text(&version, screen.left_bottom() + Vec2::new(12.0, -10.0), Align2::LEFT_BOTTOM, format!("Minecraft {}, Lattice {}", lattice_proto::GAME_VERSION, env!("CARGO_PKG_VERSION")), font(SIZE_SMALL), ink_muted());
    match action {
        1 => {
            app.overlays.paused = false;
            app.capture_mouse();
        }
        2 => {
            app.overlays.paused = false;
            app.screen = Screen::Settings;
        }
        3 => app.leave(),
        7 => {
            app.overlays.paused = false;
            if let Some(s) = &mut app.session {
                let difficulty = s.difficulty.map_or(2, |d| d.0);
                s.modal = Some(crate::game::Modal::GameRules { rules: Vec::new(), search: String::new(), difficulty });
                s.conn.send(lattice_proto::packets::play::client_command(2));
            }
        }
        4 => {
            app.overlays.paused = false;
            if let Some(s) = &mut app.session {
                super::modal::open_advancements(s);
            }
        }
        5 => {
            app.overlays.paused = false;
            if let Some(s) = &mut app.session {
                super::modal::open_stats(s);
            }
        }
        6 => {
            app.overlays.paused = false;
            if let Some(s) = &mut app.session {
                super::modal::open_links(s);
            }
        }
        _ => {}
    }
}

fn death_screen(app: &mut App, ui: &mut egui::Ui, screen: Rect, msg: &lattice_proto::Text, tr: &dyn Translator) {
    app.release_mouse();
    let job = widgets::spans_job(&msg.spans(tr), SIZE_LEAD, ink(), MODAL_W);
    let hardcore = app.session.as_ref().is_some_and(|s| s.hardcore);
    let mut action = 0;
    game_menu(ui, screen, "death", Color32::from_rgba_unmultiplied(90, 8, 8, 170), |ui| {
        super::chrome::title(ui, "You died", SIZE_DISPLAY, ink());
        ui.vertical_centered(|ui| ui.label(egui::WidgetText::LayoutJob(job.into())));
        ui.add_space(18.0);
        if !hardcore && widgets::menu_button(ui, "Respawn", true, MODAL_W).clicked() {
            action = 1;
        }
        if hardcore {
            super::chrome::title(ui, "This is a hardcore world, so you can't respawn.", SIZE_BODY, ink());
        }
        if widgets::menu_button(ui, "Disconnect and close", false, MODAL_W).clicked() {
            action = 2;
        }
    });
    match action {
        1 => {
            if let Some(s) = &mut app.session {
                s.respawn();
            }
            app.capture_mouse();
        }
        2 => app.leave(),
        _ => {}
    }
}

fn code_of_conduct(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    app.release_mouse();
    let text = app.session.as_ref().and_then(|s| s.code_of_conduct.clone()).unwrap_or_default();
    let mut action = 0;
    modal_frame(ui, screen, "coc", |ui| {
        ui.label(RichText::new("This server's code of conduct").font(font_bold(SIZE_LEAD)));
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            ui.label(&text);
        });
        ui.add_space(12.0);
        if widgets::menu_button(ui, "Accept", true, MODAL_W).clicked() {
            action = 1;
        }
        if widgets::menu_button(ui, "Decline and disconnect", false, MODAL_W).clicked() {
            action = 2;
        }
    });
    match action {
        1 => {
            if let Some(s) = &mut app.session {
                s.accept_code_of_conduct();
            }
            app.capture_mouse();
        }
        2 => app.leave(),
        _ => {}
    }
}
