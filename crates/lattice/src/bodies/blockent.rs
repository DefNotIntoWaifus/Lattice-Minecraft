//! Things in the world that are blocks but drawn like mobs: chests, banners, skulls, and so on.

use super::mobs::{DYE_COLORS, DYE_NAMES, WHITE, rgb};
use super::{Bodies, Ctx, Pass};
use crate::game::block_entities::BlockEnt;
use crate::game::items;
use glam::{DVec3, Mat4, Vec3};
use lattice_proto::{ItemStack, Nbt};
use lattice_render::atlas::Sprite;
use lattice_render::mesher::Vertex;
use lattice_render::parts::{self, Look, Rig};
use lattice_render::renderer::Batches;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

fn draw_model(ctx: &mut Ctx, rig: &Rig, m: Mat4, sprite: Sprite, light: [u8; 2], color: [u8; 4], pass: Pass) {
    let look = Look { sprite, color, light, overlay: [0, 0], scroll: [0.0, 0.0], lit: pass != Pass::Glow && !ctx.unlit };
    parts::draw(rig, m, &look, ctx.list(pass));
}

fn sprite_or_missing(ctx: &Ctx, name: &str) -> Sprite {
    ctx.sprite(name).unwrap_or(ctx.bodies.sheet.missing)
}

fn y_turn(deg: f32, x: f32, y: f32, z: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x, y, z)) * Mat4::from_rotation_y(deg.to_radians()) * Mat4::from_translation(Vec3::new(-x, -y, -z))
}

fn facing_yaw(facing: &str) -> f32 {
    match facing {
        "south" => 0.0,
        "west" => 90.0,
        "north" => 180.0,
        "east" => 270.0,
        _ => 0.0,
    }
}

/// Shapes and textures of worn or placed heads, by block or item name.
pub fn skull_of(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.strip_suffix("_wall_head").map(|s| format!("{s}_head")).or_else(|| name.strip_suffix("_wall_skull").map(|s| format!("{s}_skull"))).unwrap_or_else(|| name.to_string());
    Some(match n.as_str() {
        "skeleton_skull" => ("skeleton_skull", "entity/skeleton/skeleton"),
        "wither_skeleton_skull" => ("wither_skeleton_skull", "entity/skeleton/wither_skeleton"),
        "zombie_head" => ("zombie_head", "entity/zombie/zombie"),
        "player_head" => ("player_head", "entity/player/wide/steve"),
        "creeper_head" => ("creeper_head", "entity/creeper/creeper"),
        "dragon_head" => ("dragon_skull", "entity/enderdragon/dragon"),
        "piglin_head" => ("piglin_head", "entity/piglin/piglin"),
        _ => return None,
    })
}

pub fn draw_skull_at(ctx: &mut Ctx, model: &str, texture: &str, m: Mat4, light: [u8; 2], anim: f32) {
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    if model == "dragon_skull" {
        rig.p("jaw").rot.x = ((anim * PI * 0.2).sin() + 1.0) * 0.2;
    } else if model == "piglin_head" {
        rig.p("left_ear").rot.z = -((anim * PI * 0.2 * 1.2).cos() + 2.5) * 0.2;
        rig.p("right_ear").rot.z = ((anim * PI * 0.2).cos() + 2.5) * 0.2;
    }
    let sprite = sprite_or_missing(ctx, texture);
    draw_model(ctx, &rig, m, sprite, light, WHITE, Pass::Cutout);
}

fn chest_texture(block: &str, kind: &str) -> String {
    let base = match block {
        "trapped_chest" => "trapped".to_string(),
        "ender_chest" => "ender".to_string(),
        b if b.contains("copper_chest") => {
            let stage = if b.contains("oxidized") {
                "copper_oxidized"
            } else if b.contains("weathered") {
                "copper_weathered"
            } else if b.contains("exposed") {
                "copper_exposed"
            } else {
                "copper"
            };
            stage.to_string()
        }
        _ => "normal".to_string(),
    };
    match kind {
        "left" => format!("entity/chest/{base}_left"),
        "right" => format!("entity/chest/{base}_right"),
        _ => format!("entity/chest/{base}"),
    }
}

fn draw_chest(ctx: &mut Ctx, m: Mat4, light: [u8; 2], block: &str, kind: &str, facing: &str, openness: f32) {
    let model = match kind {
        "left" => "double_chest_left",
        "right" => "double_chest_right",
        _ => "chest",
    };
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    let v = 1.0 - openness;
    let v = 1.0 - v * v * v;
    rig.p("lid").rot.x = -(v * FRAC_PI_2);
    rig.p("lock").rot.x = -(v * FRAC_PI_2);
    let at = m * y_turn(-facing_yaw(facing), 0.5, 0.0, 0.5);
    let sprite = sprite_or_missing(ctx, &chest_texture(block, kind));
    draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
}

fn shulker_matrix(facing: &str) -> Mat4 {
    let turn = match facing {
        "down" => Mat4::from_rotation_x(PI),
        "north" => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(PI),
        "south" => Mat4::from_rotation_x(FRAC_PI_2),
        "west" => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(FRAC_PI_2),
        "east" => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(-FRAC_PI_2),
        _ => Mat4::IDENTITY,
    };
    Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_scale(Vec3::splat(0.9995)) * turn * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0)) * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0))
}

fn draw_shulker_box(ctx: &mut Ctx, m: Mat4, light: [u8; 2], color: Option<&str>, facing: &str, open: f32) {
    let Some(skel) = ctx.bodies.skeleton("shulker_box") else { return };
    let mut rig = Rig::new(skel);
    let lid = rig.p("lid");
    lid.pos = Vec3::new(0.0, 24.0 - open * 0.5 * 16.0, 0.0);
    lid.rot.y = 270f32.to_radians() * open;
    let tex = match color {
        Some(c) => format!("entity/shulker/shulker_{c}"),
        None => "entity/shulker/shulker".to_string(),
    };
    let sprite = sprite_or_missing(ctx, &tex);
    draw_model(ctx, &rig, m * shulker_matrix(facing), sprite, light, WHITE, Pass::Cutout);
}

fn dye_index(name: &str) -> Option<usize> {
    DYE_NAMES.iter().position(|d| *d == name)
}

/// The layers on a banner, from its data: pattern name and colour.
fn banner_layers(nbt: Option<&Nbt>) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let Some(list) = nbt.and_then(|n| n.get("patterns")).and_then(Nbt::as_list) else { return out };
    for entry in list {
        let pattern = entry.get("pattern").and_then(Nbt::as_str).unwrap_or("");
        let color = entry.get("color").and_then(Nbt::as_str).and_then(dye_index).unwrap_or(0);
        if !pattern.is_empty() {
            out.push((pattern.trim_start_matches("minecraft:").to_string(), DYE_COLORS[color]));
        }
    }
    out
}

fn draw_flag_layers(ctx: &mut Ctx, rig: &Rig, m: Mat4, light: [u8; 2], base: u32, layers: &[(String, u32)]) {
    let plain = sprite_or_missing(ctx, "entity/banner/banner_base");
    draw_model(ctx, rig, m, plain, light, WHITE, Pass::Cutout);
    if let Some(sprite) = ctx.sprite("entity/banner/base") {
        draw_model(ctx, rig, m, sprite, light, rgb(base), Pass::Translucent);
    }
    for (pattern, color) in layers {
        if let Some(sprite) = ctx.sprite(&format!("entity/banner/{pattern}")) {
            draw_model(ctx, rig, m, sprite, light, rgb(*color), Pass::Translucent);
        }
    }
}

fn banner_matrix(m: Mat4, yaw: f32) -> Mat4 {
    m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5)) * Mat4::from_rotation_y((-yaw).to_radians()) * Mat4::from_scale(Vec3::new(0.6666667, -0.6666667, -0.6666667))
}

fn draw_banner(ctx: &mut Ctx, m: Mat4, light: [u8; 2], wall: bool, yaw: f32, base: u32, layers: &[(String, u32)], phase: f32) {
    let (pole, flag) = if wall { ("wall_banner", "wall_banner#flag") } else { ("standing_banner", "standing_banner#flag") };
    let at = banner_matrix(m, yaw);
    if let Some(skel) = ctx.bodies.skeleton(pole) {
        let rig = Rig::new(skel);
        let sprite = sprite_or_missing(ctx, "entity/banner/banner_base");
        draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
    }
    if let Some(skel) = ctx.bodies.skeleton(flag) {
        let mut rig = Rig::new(skel);
        rig.p("flag").rot.x = (-0.0125 + 0.01 * (TAU * phase).cos()) * PI;
        draw_flag_layers(ctx, &rig, at, light, base, layers);
    }
}

fn draw_pot(ctx: &mut Ctx, m: Mat4, light: [u8; 2], facing: &str, sherds: [&str; 4]) {
    let at = m * Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_rotation_y((180.0 - facing_yaw(facing)).to_radians()) * Mat4::from_translation(Vec3::splat(-0.5));
    if let Some(skel) = ctx.bodies.skeleton("decorated_pot_base") {
        let rig = Rig::new(skel);
        let sprite = sprite_or_missing(ctx, "entity/decorated_pot/decorated_pot_base");
        draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
    }
    if let Some(skel) = ctx.bodies.skeleton("decorated_pot_sides") {
        let rig = Rig::new(skel);
        for (part, sherd) in ["back", "left", "right", "front"].into_iter().zip(sherds) {
            let name = sherd.trim_start_matches("minecraft:");
            let tex = match name.strip_suffix("_sherd") {
                Some(stem) => format!("entity/decorated_pot/{}_pattern", stem),
                None => "entity/decorated_pot/decorated_pot_side".to_string(),
            };
            let sprite = sprite_or_missing(ctx, &tex);
            let look = Look { sprite, color: WHITE, light, overlay: [0, 0], scroll: [0.0, 0.0], lit: !ctx.unlit };
            parts::draw_part(&rig, at, &look, ctx.list(Pass::Cutout), part);
        }
    }
}

fn book_pose(rig: &mut Rig, open: f32, flip1: f32, flip2: f32) {
    rig.p("left_lid").rot.y = PI + open;
    rig.p("right_lid").rot.y = -open;
    rig.p("left_pages").rot.y = open;
    rig.p("right_pages").rot.y = -open;
    rig.p("flip_page1").rot.y = open - open * 2.0 * flip1;
    rig.p("flip_page2").rot.y = open - open * 2.0 * flip2;
    let s = open.sin();
    rig.p("left_pages").pos.x = s;
    rig.p("right_pages").pos.x = s;
    rig.p("flip_page1").pos.x = s;
    rig.p("flip_page2").pos.x = s;
}

fn props<'a>(list: &'a [(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    list.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// The colour dyes give to writing.
const DYE_TEXT: [u32; 16] = [0xFFFFFF, 0xFF681F, 0xFF00FF, 0x9AC0CD, 0xFFFF00, 0xBFFF00, 0xFF69B4, 0x808080, 0xD3D3D3, 0x00FFFF, 0xA020F0, 0x0000FF, 0x8B4513, 0x00FF00, 0xFF0000, 0x000000];

fn draw_sign_side(ctx: &mut Ctx, side: &Nbt, m: Mat4, light: [u8; 2], line_height: f32) {
    let Some(messages) = side.get("messages").and_then(Nbt::as_list) else { return };
    let dye = side.get("color").and_then(Nbt::as_str).and_then(dye_index).unwrap_or(15);
    let text = DYE_TEXT[dye];
    let glowing = side.get("has_glowing_text").is_some_and(|g| g.as_bool().unwrap_or(false));
    let scale = |c: u32| (((c >> 16 & 255) as f32 * 0.4) as u32) << 16 | (((c >> 8 & 255) as f32 * 0.4) as u32) << 8 | ((c & 255) as f32 * 0.4) as u32;
    let dark = if dye == 15 && glowing { 0xF0EBCC } else { scale(text) };
    let (color, outline, light) = if glowing { (text, dye == 15, [255, 255]) } else { (dark, false, light) };
    let to_rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    let top = 4.0 * line_height / 2.0;
    let lang = ctx.lang;
    for (i, message) in messages.iter().take(4).enumerate() {
        let spans = lattice_proto::Text::from_nbt(message).spans(lang);
        if spans.iter().all(|s| s.text.is_empty()) {
            continue;
        }
        let width = ctx.bodies.font.width(&spans);
        let (x, y) = (-width / 2.0, i as f32 * line_height - top);
        if outline {
            let plain: Vec<_> = spans.iter().map(|s| lattice_proto::text::Span { color: Some(dark), ..s.clone() }).collect();
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
                let behind = m * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.05));
                super::text::draw_line(ctx, &plain, x + dx, y + dy, behind, super::text::TextStyle { color: to_rgb(dark), alpha: 255, light, pass: Pass::Cutout });
            }
        }
        super::text::draw_line(ctx, &spans, x, y, m, super::text::TextStyle { color: to_rgb(color), alpha: 255, light, pass: Pass::Cutout });
    }
}

fn draw_sign(ctx: &mut Ctx, be: &BlockEnt, name: &str, list: &[(&str, &str)], m: Mat4, light: [u8; 2]) {
    let Some(data) = be.data.as_ref() else { return };
    let hanging = name.contains("hanging_sign");
    let wall = name.contains("wall");
    let yaw = if wall { facing_yaw(props(list, "facing").unwrap_or("north")) } else { props(list, "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0) * 22.5 };
    for (tag, back) in [("front_text", false), ("back_text", true)] {
        let Some(side) = data.get(tag) else { continue };
        let mut at = if hanging {
            m * Mat4::from_translation(Vec3::new(0.5, 0.9375, 0.5)) * Mat4::from_rotation_y((-yaw).to_radians()) * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0))
        } else {
            let mut a = m * Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_rotation_y((-yaw).to_radians());
            if wall {
                a *= Mat4::from_translation(Vec3::new(0.0, -0.3125, -0.4375));
            }
            a
        };
        if back {
            at *= Mat4::from_rotation_y(PI);
        }
        let (offset, scale, line) = if hanging { (Vec3::new(0.0, -0.32, 0.073), 0.0140625, 9.0) } else { (Vec3::new(0.0, 0.33333334, 0.046666667), 0.010416667, 10.0) };
        at = at * Mat4::from_translation(offset) * Mat4::from_scale(Vec3::new(scale, -scale, scale));
        draw_sign_side(ctx, side, at, light, line);
    }
}

/// Draws the block entities near the camera.
pub fn draw_all(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, camera: DVec3, reach: f64, time: f64) {
    let data = lattice_data::game();
    let reach2 = reach * reach;
    let (cx, cz) = ((camera.x.floor() as i32) >> 4, (camera.z.floor() as i32) >> 4);
    let range = (reach as i32 >> 4) + 1;
    for dz in -range..=range {
        for dx in -range..=range {
            let Some(column) = session.block_entities.columns.get(&(cx + dx, cz + dz)) else { continue };
            for (pos, be) in column {
                let centre = DVec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
                if centre.distance_squared(camera) > reach2 {
                    continue;
                }
                let Some(state) = world.block(pos.x, pos.y, pos.z) else { continue };
                let block = data.block(state);
                let m = Mat4::from_translation((DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera).as_vec3());
                let sky_block = super::light_at(world, pos.x, pos.y, pos.z);
                draw_one(ctx, session, be, block.name.as_str(), state, m, sky_block, centre, time, camera);
            }
        }
    }
    // Beams show from far further away than anything else here.
    for column in session.block_entities.columns.values() {
        for pos in column.keys() {
            if world.block(pos.x, pos.y, pos.z).is_some_and(|s| data.block(s).name == "beacon") {
                let m = Mat4::from_translation((DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera).as_vec3());
                draw_beam(ctx, session, world, pos.x, pos.y, pos.z, m);
            }
        }
    }
}

/// A block state as saved: its name and properties.
fn state_from_nbt(n: &Nbt) -> Option<u32> {
    let game = lattice_data::game();
    let name = n.get("Name")?.as_str()?.trim_start_matches("minecraft:");
    let block = game.blocks.iter().find(|b| b.name == name)?;
    let props = n.get("Properties");
    let mut wanted: Vec<(String, String)> = Vec::new();
    if let Some(Nbt::Compound(map)) = props {
        for (k, v) in map {
            if let Some(v) = v.as_str() {
                wanted.push((k.clone(), v.to_string()));
            }
        }
    }
    let pairs: Vec<(&str, &str)> = wanted.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    game.state_with(block, &pairs).or(Some(block.default))
}

/// The beam above a beacon: a section of colour for each run of the same stained glass, as the
/// game works it out climbing up from the block.
fn beam_sections(world: &lattice_world::World, x: i32, y: i32, z: i32) -> Vec<(u32, i32)> {
    let data = lattice_data::game();
    // At least one layer of the right blocks underneath.
    let tag_fits = |bx: i32, bz: i32| world.block(bx, y - 1, bz).is_some_and(|s| matches!(data.block(s).name.as_str(), "iron_block" | "gold_block" | "diamond_block" | "emerald_block" | "netherite_block"));
    for dx in -1..=1 {
        for dz in -1..=1 {
            if !tag_fits(x + dx, z + dz) {
                return Vec::new();
            }
        }
    }
    let top = world.geometry.min_y + world.geometry.height;
    let mut sections: Vec<(u32, i32)> = vec![(0xFFFFFF, 1)];
    let mut first = true;
    for ny in y + 1..top {
        let Some(s) = world.block(x, ny, z) else { break };
        let block = data.block(s);
        let glass = block.name.strip_suffix("_stained_glass_pane").or(block.name.strip_suffix("_stained_glass"));
        if let Some(colour) = glass {
            let c = super::mobs::dye_color(colour);
            let last = sections.len() - 1;
            if first {
                sections.push((c, 1));
                first = false;
            } else if sections[last].0 == c {
                sections[last].1 += 1;
            } else {
                let mix = |s: u32| (((sections[last].0 >> s) & 255) + ((c >> s) & 255)) / 2;
                sections.push((mix(16) << 16 | mix(8) << 8 | mix(0), 1));
            }
        } else {
            let info = data.state(s);
            if info.dampening >= 15 && block.name != "bedrock" {
                return Vec::new();
            }
            let last = sections.len() - 1;
            sections[last].1 += 1;
        }
    }
    sections
}

/// Draws one beam: an inner core and a wider soft glow, turning and flowing upward.
fn draw_beam(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, x: i32, y: i32, z: i32, m: Mat4) {
    let Some(sprite) = ctx.sprite("entity/beacon/beacon_beam") else { return };
    let sections = beam_sections(world, x, y, z);
    if sections.is_empty() {
        return;
    }
    let time = (session.game_time.rem_euclid(40) as f32 + ctx.partial) % 40.0;
    let mut from = 0;
    let count = sections.len();
    for (i, (color, height)) in sections.into_iter().enumerate() {
        // The last goes on up into the sky.
        let height = if i + 1 == count { 2048 } else { height };
        beam_part(ctx, m, sprite, color, time, from as f32, height as f32);
        from += height;
    }
}

fn beam_part(ctx: &mut Ctx, m: Mat4, sprite: Sprite, color: u32, time: f32, from: f32, height: f32) {
    let t = -time;
    let scroll = (t * 0.2 - (t * 0.1).floor()).rem_euclid(1.0);
    let rgb = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
    let base = m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5));
    let inner = base * Mat4::from_rotation_y((time * 2.25 - 45.0).to_radians());
    let r = 0.2f32;
    let core = [[0.0, r], [r, 0.0], [-r, 0.0], [0.0, -r]];
    side_quads(ctx, inner, core, from, height, height * (0.5 / r), scroll, sprite, [rgb[0], rgb[1], rgb[2], 255], super::Pass::Cutout);
    let g = 0.25f32;
    let glow = [[-g, -g], [g, -g], [-g, g], [g, g]];
    side_quads(ctx, base, glow, from, height, height, scroll, sprite, [rgb[0], rgb[1], rgb[2], 32], super::Pass::Translucent);
}

/// Four walls between corners a,b,c,d (given as a, b, c, d in the game's order), with the picture
/// repeating along their height: the sheet doesn't wrap, so each repeat is its own quad.
#[allow(clippy::too_many_arguments)]
fn side_quads(ctx: &mut Ctx, m: Mat4, c: [[f32; 2]; 4], from: f32, height: f32, repeats: f32, scroll: f32, s: Sprite, color: [u8; 4], pass: super::Pass) {
    let per_block = repeats / height.max(0.001);
    let walls = [(c[0], c[1]), (c[3], c[2]), (c[1], c[3]), (c[2], c[0])];
    for (a, b) in walls {
        let mut y = from;
        let end = from + height;
        // How far up the picture the bottom edge sits.
        let mut v = -1.0 + scroll + repeats;
        while y < end {
            let frac = v - (v - 1e-5).floor();
            let step = (frac / per_block).min(end - y).max(1e-3);
            let (v_top, v_low) = (frac - step * per_block, frac);
            let (v0, v1) = (s.v0 + (s.v1 - s.v0) * v_top, s.v0 + (s.v1 - s.v0) * v_low);
            let list = ctx.list(pass);
            let full = [255, 255, 0, 0];
            let mut put = |x: f32, py: f32, z: f32, u: f32, tv: f32| list.push(Vertex { pos: m.transform_point3(Vec3::new(x, py, z)).to_array(), uv: [u, tv], color, light: full });
            put(a[0], y + step, a[1], s.u1, v0);
            put(a[0], y, a[1], s.u1, v1);
            put(b[0], y, b[1], s.u0, v1);
            put(b[0], y + step, b[1], s.u0, v0);
            y += step;
            v -= step * per_block;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_one(ctx: &mut Ctx, session: &crate::game::Session, be: &BlockEnt, name: &str, state: u32, m: Mat4, light: [u8; 2], centre: DVec3, time: f64, camera: DVec3) {
    let data = lattice_data::game();
    let list = data.properties(state);
    let partial = ctx.partial;
    let open = be.prev_open + (be.open - be.prev_open) * partial;
    if name.ends_with("copper_golem_statue") {
        let level = ["exposed", "weathered", "oxidized"].into_iter().find(|l| name.contains(l));
        let sprite = sprite_or_missing(ctx, &level.map_or("entity/copper_golem/copper_golem".to_string(), |l| format!("entity/copper_golem/copper_golem_{l}")));
        // Each pose is its own shape.
        let shape = match props(&list, "copper_golem_pose").unwrap_or("standing") {
            "running" => "copper_golem_running",
            "sitting" => "copper_golem_sitting",
            "star" => "copper_golem_star",
            _ => "copper_golem",
        };
        if let Some(skel) = ctx.bodies.skeleton(shape) {
            let mut rig = Rig::new(skel);
            rig.p("root").pos.y = 0.0;
            let facing = props(&list, "facing").unwrap_or("north");
            let at = m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5)) * Mat4::from_rotation_y((180.0 - facing_yaw(facing)).to_radians()) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
        }
        return;
    }
    if name == "chest" || name == "trapped_chest" || name == "ender_chest" || name.contains("copper_chest") {
        let kind = props(&list, "type").unwrap_or("single");
        let facing = props(&list, "facing").unwrap_or("north");
        draw_chest(ctx, m, light, name, kind, facing, open);
    } else if name.ends_with("shulker_box") {
        let color = name.strip_suffix("_shulker_box");
        draw_shulker_box(ctx, m, light, color, props(&list, "facing").unwrap_or("up"), open);
    } else if name.ends_with("_banner") {
        let wall = name.ends_with("_wall_banner");
        let colour = name.strip_suffix("_wall_banner").or(name.strip_suffix("_banner")).unwrap_or("white");
        let base = DYE_COLORS[dye_index(colour).unwrap_or(0)];
        let yaw = if wall { facing_yaw(props(&list, "facing").unwrap_or("north")) } else { props(&list, "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0) * 22.5 };
        let pos_hash = (centre.x.floor() * 7.0 + centre.y.floor() * 9.0 + centre.z.floor() * 13.0) as i64;
        let phase = ((pos_hash + session.game_time()).rem_euclid(100) as f32 + partial) / 100.0;
        let layers = banner_layers(be.data.as_ref());
        draw_banner(ctx, m, light, wall, yaw, base, &layers, phase);
    } else if let Some((model, texture)) = skull_of(name) {
        let at = if name.contains("_wall_") {
            let facing = props(&list, "facing").unwrap_or("north");
            let (dx, dz) = match facing {
                "north" => (0.0, -1.0),
                "south" => (0.0, 1.0),
                "west" => (-1.0, 0.0),
                _ => (1.0, 0.0),
            };
            let opposite = facing_yaw(match facing {
                "north" => "south",
                "south" => "north",
                "west" => "east",
                _ => "west",
            });
            m * Mat4::from_translation(Vec3::new(0.5 - dx * 0.25, 0.25, 0.5 - dz * 0.25)) * Mat4::from_rotation_y((-opposite).to_radians()) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        } else {
            let rot = props(&list, "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0) * 22.5;
            m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5)) * Mat4::from_rotation_y((-rot).to_radians()) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        };
        draw_skull_at(ctx, model, texture, at, light, 0.0);
    } else if name == "moving_piston" {
        // What a piston's pushing, sliding across for the two ticks it takes.
        let Some(d) = be.data.as_ref() else { return };
        let Some(moved) = d.get("blockState").and_then(state_from_nbt) else { return };
        let facing = d.get("facing").and_then(Nbt::as_i64).unwrap_or(0) as i32;
        let extending = d.get("extending").and_then(Nbt::as_i64).unwrap_or(0) != 0;
        let progress = ((be.age as f32 + partial) * 0.5).min(1.0);
        let along = if extending { progress - 1.0 } else { 1.0 - progress };
        let dir = crate::game::entities::direction_step(facing).as_vec3();
        super::held::draw_block(ctx, moved, m * Mat4::from_translation(dir * along), light, 0, None);
    } else if name.ends_with("_shelf") {
        let yaw = facing_yaw(props(&list, "facing").unwrap_or("north"));
        let bottom = be.data.as_ref().and_then(|d| d.get("align_items_to_bottom")).and_then(Nbt::as_i64).unwrap_or(0) != 0;
        if let Some(on_it) = be.data.as_ref().and_then(|d| d.get("Items")).and_then(Nbt::as_list) {
            for entry in on_it {
                let slot = entry.get("Slot").and_then(Nbt::as_i64).unwrap_or(0) as f32;
                let Some(item) = entry.get("id").and_then(Nbt::as_str).and_then(items::item_id) else { continue };
                let stack = ItemStack::new(item, entry.get("count").and_then(Nbt::as_i64).unwrap_or(1) as i32);
                let at = m
                    * Mat4::from_translation(Vec3::splat(0.5))
                    * Mat4::from_rotation_y((-yaw).to_radians())
                    * Mat4::from_translation(Vec3::new((slot - 1.0) * 0.3125, if bottom { -0.25 } else { 0.0 }, -0.25))
                    * Mat4::from_scale(Vec3::splat(0.25));
                // Sits on the shelf or hangs centred, by the shape of the thing.
                let (low, high) = super::held::height_of(ctx, &stack, "on_shelf");
                let lift = if bottom { -low } else { -low - (high - low) / 2.0 };
                super::held::draw_stack(ctx, &stack, "on_shelf", at * Mat4::from_translation(Vec3::new(0.0, lift, 0.0)), light, false);
            }
        }
    } else if name == "campfire" || name == "soul_campfire" {
        // What's cooking, one on each side.
        let facing = match props(&list, "facing").unwrap_or("north") {
            "south" => 0,
            "west" => 1,
            "north" => 2,
            _ => 3,
        };
        if let Some(cooking) = be.data.as_ref().and_then(|d| d.get("Items")).and_then(Nbt::as_list) {
            for entry in cooking {
                let slot = entry.get("Slot").and_then(Nbt::as_i64).unwrap_or(0) as i32;
                let Some(item) = entry.get("id").and_then(Nbt::as_str).and_then(items::item_id) else { continue };
                let stack = ItemStack::new(item, 1);
                let dir: i32 = (slot + facing) % 4;
                let at = m
                    * Mat4::from_translation(Vec3::new(0.5, 0.44921875, 0.5))
                    * Mat4::from_rotation_y((-(dir as f32 * 90.0)).to_radians())
                    * Mat4::from_rotation_x(FRAC_PI_2)
                    * Mat4::from_translation(Vec3::new(-0.3125, -0.3125, 0.0))
                    * Mat4::from_scale(Vec3::splat(0.375));
                super::held::draw_stack(ctx, &stack, "fixed", at, light, false);
            }
        }
    } else if name == "spawner" {
        let entity = be.data.as_ref().and_then(|d| d.get("SpawnData")).and_then(|d| d.get("entity")).and_then(|d| d.get("id")).and_then(Nbt::as_str).map(|n| n.trim_start_matches("minecraft:").to_string());
        if let Some(info) = entity.and_then(|n| lattice_data::game().entities.iter().find(|e| e.name == n)) {
            let scale = 0.53125 / info.width.max(info.height).max(1.0);
            let spin = (time as f32 * 180.0) % 360.0;
            let at = m * Mat4::from_translation(Vec3::new(0.5, 0.4, 0.5)) * Mat4::from_rotation_y(spin.to_radians()) * Mat4::from_translation(Vec3::new(0.0, -0.2, 0.0)) * Mat4::from_rotation_x((-30.0f32).to_radians()) * Mat4::from_scale(Vec3::splat(scale));
            super::draw_at(ctx, session, info.id as i32, at, light);
        }
    } else if name == "bell" {
        let Some(skel) = ctx.bodies.skeleton("bell") else { return };
        let mut rig = Rig::new(skel);
        if let Some((dir, ticks)) = be.ringing {
            let t = ticks as f32 + partial;
            let v = (t / PI).sin() / (4.0 + t / 3.0);
            let body = rig.p("bell_body");
            match dir {
                2 => body.rot.x = -v,
                3 => body.rot.x = v,
                4 => body.rot.z = -v,
                5 => body.rot.z = v,
                _ => {}
            }
        }
        let sprite = sprite_or_missing(ctx, "entity/bell/bell_body");
        draw_model(ctx, &rig, m, sprite, light, WHITE, Pass::Cutout);
    } else if name == "decorated_pot" {
        let sherds: Vec<String> = be
            .data
            .as_ref()
            .and_then(|n| n.get("sherds"))
            .and_then(Nbt::as_list)
            .map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let at = |i: usize| sherds.get(i).map(String::as_str).unwrap_or("minecraft:brick");
        draw_pot(ctx, m, light, props(&list, "facing").unwrap_or("north"), [at(0), at(1), at(2), at(3)]);
    } else if name == "enchanting_table" {
        let Some(skel) = ctx.bodies.skeleton("book") else { return };
        let mut rig = Rig::new(skel);
        let t = (session.game_time() as f32 + partial) + (centre.x + centre.z) as f32;
        let to_cam = camera - centre;
        let near = to_cam.length_squared() < 9.0;
        let openness = if near { 1.0 } else { 0.0 };
        let yaw = if near { (to_cam.z as f32).atan2(to_cam.x as f32) } else { 0.0 };
        book_pose(&mut rig, openness * 1.2, ((t * 0.02).fract() * 1.6 - 0.3).clamp(0.0, 1.0), (((t * 0.02) + 0.5).fract() * 1.6 - 0.3).clamp(0.0, 1.0));
        let at = m
            * Mat4::from_translation(Vec3::new(0.5, 0.75, 0.5))
            * Mat4::from_translation(Vec3::new(0.0, 0.1 + (t * 0.1).sin() * 0.01, 0.0))
            * Mat4::from_rotation_y(-yaw)
            * Mat4::from_rotation_z(80f32.to_radians());
        let sprite = sprite_or_missing(ctx, "entity/enchantment/enchanting_table_book");
        draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
    } else if name == "lectern" {
        if props(&list, "has_book") != Some("true") {
            return;
        }
        let Some(skel) = ctx.bodies.skeleton("book") else { return };
        let mut rig = Rig::new(skel);
        book_pose(&mut rig, 1.2, 0.1, 0.9);
        let yaw = facing_yaw(props(&list, "facing").unwrap_or("north")) + 90.0;
        let at = m
            * Mat4::from_translation(Vec3::new(0.5, 1.0625, 0.5))
            * Mat4::from_rotation_y((-yaw).to_radians())
            * Mat4::from_rotation_z(67.5f32.to_radians())
            * Mat4::from_translation(Vec3::new(0.0, -0.125, 0.0));
        let sprite = sprite_or_missing(ctx, "entity/enchantment/enchanting_table_book");
        draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
    } else if name == "conduit" {
        let Some(skel) = ctx.bodies.skeleton("conduit#shell") else { return };
        let rig = Rig::new(skel);
        let t = (time as f32 * 20.0) % 360.0;
        let at = m * Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_rotation_y(t.to_radians());
        let sprite = sprite_or_missing(ctx, "entity/conduit/base");
        draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
    } else if name.ends_with("_sign") {
        draw_sign(ctx, be, name, &list, m, light);
    } else if matches!(name, "end_portal" | "end_gateway") {
        // The picture is the same for all of them: stars fixed to the screen. A
        // portal shows its top and underside; a gateway is a whole cube of it.
        let t = (time as f32 / 1200.0) % 1.0;
        let vertex = |x: f32, y: f32, z: f32| lattice_render::mesher::Vertex { pos: m.transform_point3(Vec3::new(x, y, z)).to_array(), uv: [t, 0.0], color: [255, 255, 255, 255], light: [255, 255, 0, 0] };
        let (low, high) = if name == "end_portal" { (0.375, 0.75) } else { (0.0, 1.0) };
        let mut faces = vec![
            [vertex(0.0, high, 0.0), vertex(0.0, high, 1.0), vertex(1.0, high, 1.0), vertex(1.0, high, 0.0)],
            [vertex(0.0, low, 0.0), vertex(1.0, low, 0.0), vertex(1.0, low, 1.0), vertex(0.0, low, 1.0)],
        ];
        if name == "end_gateway" {
            faces.extend([
                [vertex(0.0, 0.0, 0.0), vertex(0.0, 1.0, 0.0), vertex(1.0, 1.0, 0.0), vertex(1.0, 0.0, 0.0)],
                [vertex(0.0, 0.0, 1.0), vertex(1.0, 0.0, 1.0), vertex(1.0, 1.0, 1.0), vertex(0.0, 1.0, 1.0)],
                [vertex(0.0, 0.0, 0.0), vertex(0.0, 0.0, 1.0), vertex(0.0, 1.0, 1.0), vertex(0.0, 1.0, 0.0)],
                [vertex(1.0, 0.0, 0.0), vertex(1.0, 1.0, 0.0), vertex(1.0, 1.0, 1.0), vertex(1.0, 0.0, 1.0)],
            ]);
        }
        let list = ctx.list(Pass::EndPortal);
        for face in faces {
            list.extend(face);
        }
    }
}

/// A shield's dyed base, if it has one, and its patterns.
fn shield_look(ctx: &Ctx, stack: &ItemStack) -> (Option<u32>, Vec<(String, u32)>) {
    let names = ctx.registries.and_then(|r| r.names.get("banner_pattern")).map_or(&[][..], |n| n.as_slice());
    let (_, layers) = crate::game::items::banner_look(stack, names);
    (crate::game::items::shield_base_color(stack), layers)
}

/// Items drawn by code in the game. `m` is already in the base model's block space.
pub fn draw_special(ctx: &mut Ctx, stack: &ItemStack, kind: &str, texture: Option<&str>, m: Mat4, light: [u8; 2]) {
    match kind {
        "chest" => {
            let name = texture.unwrap_or("normal").trim_start_matches("minecraft:");
            let Some(skel) = ctx.bodies.skeleton("chest") else { return };
            let rig = Rig::new(skel);
            let sprite = sprite_or_missing(ctx, &format!("entity/chest/{name}"));
            draw_model(ctx, &rig, m, sprite, light, WHITE, Pass::Cutout);
        }
        "shulker_box" => {
            let name = texture.unwrap_or("shulker").trim_start_matches("minecraft:");
            let color = name.strip_prefix("shulker_");
            draw_shulker_box(ctx, m, light, color, "up", 0.0);
        }
        "banner" => {
            let colour = texture.unwrap_or("white").trim_start_matches("minecraft:");
            let base = DYE_COLORS[dye_index(colour).unwrap_or(0)];
            let names = ctx.registries.and_then(|r| r.names.get("banner_pattern")).map_or(&[][..], |n| n.as_slice());
            let (_, layers) = crate::game::items::banner_look(stack, names);
            draw_banner(ctx, m, light, false, 0.0, base, &layers, 0.0);
        }
        // One pattern alone on the flag, for the icon maker to lay over banners.
        "banner_pattern" => {
            let Some(skel) = ctx.bodies.skeleton("standing_banner#flag") else { return };
            let Some(sprite) = ctx.sprite(&format!("entity/banner/{}", texture.unwrap_or(""))) else { return };
            let mut rig = Rig::new(skel);
            rig.p("flag").rot.x = (-0.0125 + 0.01) * PI;
            draw_model(ctx, &rig, banner_matrix(m, 0.0), sprite, light, WHITE, Pass::Cutout);
        }
        // One pattern alone on the shield, for the icon maker.
        "shield_pattern" => {
            let at = m * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0));
            let Some(skel) = ctx.bodies.skeleton("shield") else { return };
            let Some(sprite) = ctx.sprite(&format!("entity/shield/{}", texture.unwrap_or(""))) else { return };
            draw_model(ctx, &Rig::new(skel), at, sprite, light, WHITE, Pass::Cutout);
        }
        "shield" => {
            let at = m * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0));
            let Some(skel) = ctx.bodies.skeleton("shield") else { return };
            let rig = Rig::new(skel);
            let (base, layers) = shield_look(ctx, stack);
            if base.is_none() && layers.is_empty() {
                let sprite = sprite_or_missing(ctx, "entity/shield/shield_base_nopattern");
                draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
                return;
            }
            let sprite = sprite_or_missing(ctx, "entity/shield/shield_base");
            draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
            if let (Some(color), Some(sprite)) = (base, ctx.sprite("entity/shield/base")) {
                draw_model(ctx, &rig, at, sprite, light, rgb(color), Pass::Translucent);
            }
            for (pattern, color) in layers {
                if let Some(sprite) = ctx.sprite(&format!("entity/shield/{pattern}")) {
                    draw_model(ctx, &rig, at, sprite, light, rgb(color), Pass::Translucent);
                }
            }
        }
        "trident" => {
            let at = m * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0));
            let Some(skel) = ctx.bodies.skeleton("trident") else { return };
            let rig = Rig::new(skel);
            let sprite = sprite_or_missing(ctx, "entity/trident/trident");
            draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
        }
        "head" | "player_head" => {
            let name = items::item_name(stack);
            if let Some((model, tex)) = skull_of(name) {
                let at = m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5)) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
                draw_skull_at(ctx, model, tex, at, light, 0.0);
            }
        }
        "decorated_pot" => draw_pot(ctx, m, light, "south", ["brick"; 4]),
        "conduit" => {
            let Some(skel) = ctx.bodies.skeleton("conduit#shell") else { return };
            let rig = Rig::new(skel);
            let sprite = sprite_or_missing(ctx, "entity/conduit/base");
            draw_model(ctx, &rig, m * Mat4::from_translation(Vec3::splat(0.5)), sprite, light, WHITE, Pass::Cutout);
        }
        "copper_golem_statue" => {
            let Some(skel) = ctx.bodies.skeleton("copper_golem") else { return };
            let rig = Rig::new(skel);
            let tex = texture.map(|t| t.trim_start_matches("minecraft:").trim_start_matches("textures/").trim_end_matches(".png").to_string()).unwrap_or_else(|| "entity/copper_golem/copper_golem".into());
            let sprite = sprite_or_missing(ctx, &tex);
            let at = m * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5)) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            draw_model(ctx, &rig, at, sprite, light, WHITE, Pass::Cutout);
        }
        _ => {}
    }
}

/// An item's picture for the inventory: draws it unlit in block space, on its own, for the icon maker.
pub fn special_icon<'a>(
    bodies: &Bodies,
    blocks: &lattice_render::models::BlockModels,
    kind: &str,
    texture: Option<&str>,
    item: &str,
    sheet: &'a image::RgbaImage,
    atlas: &'a image::RgbaImage,
) -> Vec<(Vec<lattice_render::mesher::Vertex>, &'a image::RgbaImage)> {
    let mut out = Batches::default();
    let skins = crate::skins::SkinStore::default();
    let no_maps = std::collections::HashMap::new();
    let no_map_textures = super::maps::MapTextures::default();
    let mut ctx = Ctx {
        maps: &no_maps,
        map_textures: &no_map_textures,
        shine: super::Shine::none(),
        shining: false,
        bodies,
        items: None,
        out: &mut out,
        registries: None,
        skins: &skins,
        partial: 0.0,
        day_fraction: 0.0,
        blocks,
        camera_rot: glam::Quat::IDENTITY,
        camera_yaw_rad: 0.0,
        camera_pitch_rad: 0.0,
        camera_yaw_deg: 0.0,
        fire: [bodies.sheet.missing; 2],
        to_hand: false,
        lang: &lattice_proto::text::NoTranslations,
        unlit: true,
        shadows: false,
        tag_background: 0.0,
    };
    let stack = ItemStack { item: crate::game::items::item_id(item).unwrap_or(0), count: 1, added: Vec::new(), removed: Vec::new() };
    draw_special(&mut ctx, &stack, kind, texture, Mat4::IDENTITY, [255, 255]);
    let mut entity = std::mem::take(&mut out.entity);
    entity.append(&mut out.entity_translucent);
    let blocks_list = std::mem::take(&mut out.blocks);
    vec![(entity, sheet), (blocks_list, atlas)]
}
