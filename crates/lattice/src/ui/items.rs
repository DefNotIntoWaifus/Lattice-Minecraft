//! Drawing item stacks: icon layers, counts, durability, tooltips.

use crate::assets::GameAssets;
use crate::game::items;
use egui::{Align2, Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, TextureId, Vec2};
use lattice_proto::item::PotionContents;
use lattice_proto::{ItemStack, Translator};
use lattice_render::items::ItemQuery;

/// What the item definitions ask about a stack.
pub struct StackQuery<'a> {
    pub stack: &'a ItemStack,
    pub day_fraction: f32,
    pub using_ticks: Option<f32>,
    /// How it's being shown, which some items' looks depend on.
    pub context: &'static str,
}

/// Mixes a potion's effect colours as the game does.
pub fn potion_color(p: &PotionContents) -> Option<u32> {
    if let Some(c) = p.color {
        return Some(c as u32);
    }
    let data = lattice_data::game();
    let mut effects: Vec<(i32, i32)> = p.effects.iter().map(|e| (e.effect, e.amplifier)).collect();
    if let Some(name) = p.potion.and_then(|id| data.registry("potion").get(id as usize)) {
        if let Some(list) = data.potions.get(name) {
            effects.extend(list.iter().map(|(e, a, _)| (*e, *a)));
        }
    }
    if effects.is_empty() {
        return None;
    }
    let (mut r, mut g, mut b, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (e, amp) in effects {
        let Some(info) = data.mob_effects.get(e as usize) else { continue };
        let w = (amp + 1) as f32;
        r += ((info.color >> 16) & 255) as f32 / 255.0 * w;
        g += ((info.color >> 8) & 255) as f32 / 255.0 * w;
        b += (info.color & 255) as f32 / 255.0 * w;
        n += w;
    }
    if n == 0.0 {
        return None;
    }
    let c = |v: f32| ((v / n * 255.0) as u32).min(255);
    Some(0xFF00_0000 | c(r) << 16 | c(g) << 8 | c(b))
}

impl ItemQuery for StackQuery<'_> {
    fn has_component(&self, name: &str) -> bool {
        items::has_component(self.stack, name)
    }
    fn dyed_color(&self) -> Option<u32> {
        self.stack.dyed_color().map(|c| 0xFF00_0000 | c as u32)
    }
    fn potion_color(&self) -> Option<u32> {
        self.stack.potion_contents().and_then(|p| potion_color(&p))
    }
    fn firework_color(&self) -> Option<u32> {
        let raw = self.stack.raw("firework_explosion")?;
        lattice_proto::item::read_raw(raw, |r| {
            r.var_i32()?;
            let colors = r.array(|r| r.i32())?;
            let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
            for v in &colors {
                a += (*v as u32 >> 16) & 255;
                b += (*v as u32 >> 8) & 255;
                c += *v as u32 & 255;
            }
            let n = colors.len().max(1) as u32;
            Ok(0xFF00_0000 | (a / n) << 16 | (b / n) << 8 | (c / n))
        })
    }
    fn map_color(&self) -> Option<u32> {
        self.stack.raw("map_color").and_then(|raw| lattice_proto::item::read_raw(raw, |r| r.i32())).map(|c| 0xFF00_0000 | c as u32)
    }
    fn trim_material(&self) -> Option<String> {
        let registries = crate::game::active_registries()?;
        crate::bodies::trims::trim_of(self.stack, Some(&registries)).map(|(_, material)| material)
    }
    fn broken(&self) -> bool {
        let max = items::max_damage(self.stack);
        max > 0 && items::damage(self.stack) >= max - 1
    }
    fn using(&self) -> bool {
        self.using_ticks.is_some()
    }
    fn use_ticks(&self) -> f32 {
        self.using_ticks.unwrap_or(0.0)
    }
    fn time_of_day(&self) -> f32 {
        self.day_fraction
    }
    fn compass(&self) -> f32 {
        let Some(view) = crate::game::compass_view() else { return 0.0 };
        // Where it's been told to point: a lodestone, the last place you died, or spawn.
        let target = if items::item_name(self.stack) == "recovery_compass" {
            view.death.clone()
        } else if let Some(raw) = self.stack.raw("lodestone_tracker") {
            lattice_proto::item::read_raw(raw, |r| r.option(|r| Ok((r.identifier()?, r.block_pos()?)))).flatten()
        } else {
            view.spawn.clone()
        };
        match target {
            Some((dimension, pos)) if dimension == view.dimension => compass_needle(view.yaw, view.pos, pos),
            // Nothing to point at here: it spins, as it does in the game.
            _ => (view.time * 0.7).rem_euclid(1.0),
        }
    }
    fn display_context(&self) -> &str {
        self.context
    }
}

/// How far round the dial a compass points, from the game's own sum.
pub fn compass_needle(yaw: f32, from: glam::DVec3, to: lattice_proto::BlockPos) -> f32 {
    let angle = ((to.z as f64 + 0.5 - from.z).atan2(to.x as f64 + 0.5 - from.x) / std::f64::consts::TAU) as f32;
    let facing = (yaw / 360.0).rem_euclid(1.0);
    (0.5 - (facing - 0.25 - angle)).rem_euclid(1.0)
}

/// Item name, as far as it's the stack's own model.
pub fn model_name(stack: &ItemStack) -> String {
    stack.item_model().unwrap_or_else(|| {
        lattice_data::game().items.get(stack.item as usize).and_then(|i| i.components.get("item_model")).and_then(|v| v.as_str()).map_or_else(|| items::item_name(stack).to_string(), |s| s.trim_start_matches("minecraft:").to_string())
    })
}

/// Draws just the picture of an item.
pub fn draw_icon(p: &Painter, rect: Rect, texture: TextureId, assets: &GameAssets, query: &StackQuery) {
    let layers = assets.items.resolve(&model_name(query.stack), query);
    if layers.is_empty() {
        return;
    }
    let mut mesh = Mesh::with_texture(texture);
    for l in layers {
        let uv = Rect::from_min_max(Pos2::new(l.uv[0], l.uv[1]), Pos2::new(l.uv[2], l.uv[3]));
        mesh.add_rect_with_uv(rect, uv, Color32::from_rgb(l.tint[0], l.tint[1], l.tint[2]));
    }
    // A banner's or shield's patterns, each in its dye.
    let is_shield = items::item_name(query.stack) == "shield";
    let base = if is_shield { items::shield_base_color(query.stack) } else { None };
    if query.stack.raw("banner_patterns").is_some() || base.is_some() {
        if let Some(registries) = crate::game::active_registries() {
            let names = registries.names.get("banner_pattern").map_or(&[][..], |n| n.as_slice());
            let (_, patterns) = items::banner_look(query.stack, names);
            let sheet = if is_shield { &assets.items.shield_layers } else { &assets.items.banner_layers };
            let mut layer = |pattern: &str, color: u32| {
                if let Some(l) = sheet.get(pattern) {
                    let uv = Rect::from_min_max(Pos2::new(l[0], l[1]), Pos2::new(l[2], l[3]));
                    mesh.add_rect_with_uv(rect, uv, Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8));
                }
            };
            if let Some(color) = base {
                layer("base", color);
            }
            for (pattern, color) in patterns {
                layer(&pattern, color);
            }
        }
    }
    p.add(Shape::mesh(mesh));
}

/// A band of light slowly crossing an enchanted icon. Egui adds a colour with no alpha rather
/// than blending it, and the icon's own picture keeps the light on its shape.
fn draw_icon_shine(p: &Painter, rect: Rect, texture: TextureId, assets: &GameAssets, query: &StackQuery) {
    let layers = assets.items.resolve(&model_name(query.stack), query);
    // With reduced motion the shine stays put.
    let time = if super::theme::reduce_motion() { 0.0 } else { p.ctx().input(|i| i.time) as f32 };
    if !super::theme::reduce_motion() {
        p.ctx().request_repaint();
    }
    const CELLS: usize = 6;
    let mut mesh = Mesh::with_texture(texture);
    for l in layers {
        let start = mesh.vertices.len() as u32;
        for y in 0..=CELLS {
            for x in 0..=CELLS {
                let (fx, fy) = (x as f32 / CELLS as f32, y as f32 / CELLS as f32);
                let band = (0.5 + 0.5 * ((fx + fy * 0.35) * 3.2 - time * 0.9).sin()).powi(3);
                let k = 0.12 + band * 0.55;
                let color = Color32::from_rgba_premultiplied((150.0 * k) as u8, (70.0 * k) as u8, (220.0 * k) as u8, 0);
                let pos = Pos2::new(rect.left() + rect.width() * fx, rect.top() + rect.height() * fy);
                let uv = Pos2::new(l.uv[0] + (l.uv[2] - l.uv[0]) * fx, l.uv[1] + (l.uv[3] - l.uv[1]) * fy);
                mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
            }
        }
        let row = CELLS as u32 + 1;
        for y in 0..CELLS as u32 {
            for x in 0..CELLS as u32 {
                let i = start + y * row + x;
                mesh.indices.extend_from_slice(&[i, i + 1, i + row, i + 1, i + row + 1, i + row]);
            }
        }
    }
    p.add(Shape::mesh(mesh));
}

/// Draws a stack filling `rect`: icon, durability bar, count.
pub fn draw_stack(p: &Painter, rect: Rect, texture: TextureId, assets: &GameAssets, stack: &ItemStack, day: f32) {
    draw_stack_with(p, rect, texture, assets, stack, day, None);
}

/// The same, with the count in a chosen colour.
pub fn draw_stack_with(p: &Painter, rect: Rect, texture: TextureId, assets: &GameAssets, stack: &ItemStack, day: f32, count_color: Option<Color32>) {
    let q = StackQuery { stack, day_fraction: day, using_ticks: None, context: "gui" };
    draw_icon(p, rect, texture, assets, &q);
    if items::has_glint(stack) {
        draw_icon_shine(p, rect, texture, assets, &q);
    }
    if let Some(f) = items::durability(stack) {
        let h = (rect.height() * 0.06).max(2.0);
        let bar = Rect::from_min_size(Pos2::new(rect.left() + rect.width() * 0.1, rect.bottom() - h * 2.5), Vec2::new(rect.width() * 0.8, h));
        p.rect_filled(bar, CornerRadius::ZERO, Color32::BLACK);
        let mut fill = bar;
        fill.set_width(bar.width() * f);
        let hue = Color32::from_rgb(((1.0 - f) * 255.0) as u8, (f * 255.0) as u8, 0);
        p.rect_filled(fill, CornerRadius::ZERO, hue);
    }
    if stack.count != 1 {
        // Always light on dark: this sits on an item picture, whatever's around it.
        let pos = rect.right_bottom() + Vec2::new(0.0, 2.0);
        let text = stack.count.to_string();
        let f = super::chrome::words();
        p.text(pos + Vec2::new(2.0, 2.0), Align2::RIGHT_BOTTOM, &text, f.clone(), Color32::from_black_alpha(235));
        p.text(pos, Align2::RIGHT_BOTTOM, text, f, count_color.unwrap_or(super::chrome::TIP_INK));
    }
}

fn roman(n: i32) -> String {
    match n {
        1 => "I".into(),
        2 => "II".into(),
        3 => "III".into(),
        4 => "IV".into(),
        5 => "V".into(),
        6 => "VI".into(),
        7 => "VII".into(),
        8 => "VIII".into(),
        9 => "IX".into(),
        10 => "X".into(),
        n => n.to_string(),
    }
}

static ADVANCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether tooltips also show ids and exact durability: F3 and H.
pub fn advanced_tooltips() -> bool {
    ADVANCED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_advanced_tooltips(on: bool) {
    ADVANCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Lines of a stack's tooltip: text and whether it's the title.
pub fn tooltip_lines(stack: &ItemStack, tr: &dyn Translator, enchantments: &[String]) -> Vec<(String, Color32)> {
    let mut lines = Vec::new();
    let title_color = match items::rarity(stack) {
        "uncommon" => Color32::from_rgb(0xFF, 0xFF, 0x55),
        "rare" => Color32::from_rgb(0x55, 0xFF, 0xFF),
        "epic" => Color32::from_rgb(0xFF, 0x55, 0xFF),
        _ => super::chrome::TIP_INK,
    };
    lines.push((items::display_name(stack, tr), title_color));
    let muted = super::chrome::TIP_MUTED;
    for kind in ["enchantments", "stored_enchantments"] {
        for (id, level) in stack.enchantments(kind) {
            let name = enchantments.get(id as usize).cloned().unwrap_or_else(|| format!("Enchantment {id}"));
            let key = format!("enchantment.minecraft.{}", name.trim_start_matches("minecraft:"));
            let shown = tr.translate(&key).map(str::to_string).unwrap_or(name);
            lines.push((if level == 1 && matches!(shown.as_str(), "Mending" | "Infinity" | "Silk Touch" | "Flame" | "Channeling" | "Multishot" | "Aqua Affinity" | "Curse of Binding" | "Curse of Vanishing") { shown } else { format!("{shown} {}", roman(level)) }, muted));
        }
    }
    if let Some(p) = stack.potion_contents() {
        let data = lattice_data::game();
        let mut effects: Vec<(i32, i32, i32)> = p.effects.iter().map(|e| (e.effect, e.amplifier, e.duration)).collect();
        if let Some(name) = p.potion.and_then(|id| data.registry("potion").get(id as usize)) {
            effects.extend(data.potions.get(name).into_iter().flatten().copied());
        }
        for (e, amp, duration) in effects {
            let Some(info) = data.mob_effects.get(e as usize) else { continue };
            let name = tr.translate(&info.key).unwrap_or(&info.name).to_string();
            let level = if amp > 0 { format!(" {}", roman(amp + 1)) } else { String::new() };
            let time = if duration > 20 { format!(" ({}:{:02})", duration / 20 / 60, duration / 20 % 60) } else { String::new() };
            let color = if info.category == "harmful" { Color32::from_rgb(0xFF, 0x55, 0x55) } else { Color32::from_rgb(0x55, 0x55, 0xFF) };
            lines.push((format!("{name}{level}{time}"), color));
        }
    }
    let contents = stack.container_contents();
    if !contents.is_empty() {
        let mut shown = 0;
        let mut rest = 0;
        for s in contents.iter().flatten() {
            if shown < 5 {
                lines.push((format!("{} x{}", items::display_name(s, tr), s.count), muted));
                shown += 1;
            } else {
                rest += 1;
            }
        }
        if rest > 0 {
            lines.push((format!("and {rest} more"), muted));
        }
    }
    for s in stack.bundle_contents().iter().take(8) {
        lines.push((format!("{} x{}", items::display_name(s, tr), s.count), muted));
    }
    for l in stack.lore() {
        lines.push((l.to_plain(tr), Color32::from_rgb(0xAA, 0x55, 0xAA)));
    }
    if items::has_component(stack, "unbreakable") {
        lines.push(("Unbreakable".into(), Color32::from_rgb(0x55, 0x55, 0xFF)));
    }
    let max = items::max_damage(stack);
    if max > 0 {
        lines.push((format!("Durability: {} / {max}", max - items::damage(stack)), muted));
    }
    if advanced_tooltips() {
        lines.push((format!("minecraft:{}", items::item_name(stack)), Color32::from_gray(150)));
        let components = stack.added.len();
        if components > 0 {
            lines.push((format!("{components} changed {}", if components == 1 { "component" } else { "components" }), Color32::from_gray(150)));
        }
    }
    lines
}

/// Draws a tooltip near the pointer.
pub fn tooltip(p: &Painter, at: Pos2, screen: Rect, lines: &[(String, Color32)]) {
    if lines.is_empty() {
        return;
    }
    let galleys: Vec<_> = lines.iter().map(|(t, c)| p.layout(t.clone(), super::chrome::words(), *c, 420.0)).collect();
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 20.0;
    let h = galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>() + 16.0;
    let mut pos = at + Vec2::new(14.0, 6.0);
    if pos.x + w > screen.right() - 4.0 {
        pos.x = at.x - w - 8.0;
    }
    if pos.y + h > screen.bottom() - 4.0 {
        pos.y = screen.bottom() - h - 4.0;
    }
    let r = Rect::from_min_size(pos, Vec2::new(w, h));
    super::chrome::tip_box(p, r);
    let mut y = r.top() + 8.0;
    for (g, (_, c)) in galleys.into_iter().zip(lines) {
        let gh = g.size().y;
        p.galley(Pos2::new(r.left() + 10.0, y), g, *c);
        y += gh + 2.0;
    }
}

/// A bundle's tooltip: what's inside as a little grid, how full it is, and which item comes
/// out next. False if this isn't a bundle holding anything.
#[allow(clippy::too_many_arguments)]
pub fn bundle_tooltip(p: &Painter, at: Pos2, screen: Rect, stack: &ItemStack, selected: i32, tr: &dyn Translator, texture: TextureId, assets: &GameAssets) -> bool {
    if !items::has_component(stack, "bundle_contents") {
        return false;
    }
    let inside = stack.bundle_contents();
    let u = super::chrome::unit();
    let cell = 18.0 * u;
    let cols = inside.len().clamp(1, 4);
    let rows = inside.len().div_ceil(cols).max(1);
    let name = items::display_name(stack, tr);
    let chosen = usize::try_from(selected).ok().and_then(|i| inside.get(i));
    let header = chosen.map_or(name, |s| items::display_name(s, tr));
    let title = p.layout_no_wrap(header, super::chrome::words(), super::chrome::TIP_INK);
    // Each bundle holds one stack's worth, and a stack that can't stack counts as all of it.
    let weight: f32 = inside.iter().map(|s| s.count as f32 / items::max_stack(s).max(1) as f32).sum();
    let line = if inside.is_empty() { "Empty".to_string() } else { format!("{} of 64", (weight * 64.0).round() as i32) };
    let note = p.layout_no_wrap(line, super::chrome::words(), super::chrome::TIP_MUTED);
    let w = (cell * cols as f32).max(title.size().x).max(note.size().x) + 8.0 * u;
    let body = if inside.is_empty() { 0.0 } else { cell * rows as f32 + 2.0 * u };
    let h = title.size().y + body + 6.0 * u + note.size().y + 3.0 * u;
    let mut pos = at + Vec2::new(14.0, 6.0);
    if pos.x + w > screen.right() - 4.0 {
        pos.x = at.x - w - 8.0;
    }
    if pos.y + h > screen.bottom() - 4.0 {
        pos.y = screen.bottom() - h - 4.0;
    }
    let r = Rect::from_min_size(pos, Vec2::new(w, h));
    super::chrome::tip_box(p, r);
    let mut y = r.top() + 3.0 * u;
    let title_h = title.size().y;
    p.galley(Pos2::new(r.left() + 4.0 * u, y), title, super::chrome::TIP_INK);
    y += title_h + u;
    for (i, s) in inside.iter().enumerate() {
        let cr = Rect::from_min_size(Pos2::new(r.left() + 4.0 * u + (i % cols) as f32 * cell, y + (i / cols) as f32 * cell), Vec2::splat(cell));
        super::chrome::well(p, cr, i as i32 == selected);
        draw_stack(p, Rect::from_center_size(cr.center(), Vec2::splat(16.0 * u)), texture, assets, s, 0.0);
    }
    y += body + u;
    // A filling bar.
    let bar = Rect::from_min_size(Pos2::new(r.left() + 4.0 * u, y), Vec2::new(w - 8.0 * u, u * 2.0));
    p.rect_filled(bar, CornerRadius::ZERO, Color32::from_black_alpha(200));
    let mut fill = bar;
    fill.set_width(bar.width() * weight.clamp(0.0, 1.0));
    p.rect_filled(fill, CornerRadius::ZERO, Color32::from_rgb(0x9D, 0x6B, 0xD9));
    p.galley(Pos2::new(r.left() + 4.0 * u, bar.bottom() + u), note, super::chrome::TIP_MUTED);
    true
}
