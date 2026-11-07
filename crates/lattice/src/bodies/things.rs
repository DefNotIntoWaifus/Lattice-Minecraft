//! Entities that aren't alive: dropped items, arrows, boats, minecarts, paintings, and so on.
//! Returns false for anything that's a mob.

use super::mobs::WHITE;
use super::{Ctx, Pass, held, wrap_degrees};
use crate::game::entities::Entity;
use glam::{Mat4, Quat, Vec3};
use lattice_proto::{DataValue, ItemStack};
use lattice_render::atlas::Sprite;
use lattice_render::mesher::Vertex;
use lattice_render::parts::{self, Look, Rig};
use std::f32::consts::{FRAC_PI_2, PI};

fn look(sprite: Sprite, light: [u8; 2]) -> Look {
    Look { sprite, color: WHITE, light, overlay: [0, 0], scroll: [0.0, 0.0], lit: true }
}

pub fn model(ctx: &mut Ctx, name: &str, texture: &str, m: Mat4, light: [u8; 2], setup: impl FnOnce(&mut Rig)) {
    let Some(skel) = ctx.bodies.skeleton(name) else { return };
    let mut rig = Rig::new(skel);
    setup(&mut rig);
    let sprite = ctx.sprite(texture).unwrap_or(ctx.bodies.sheet.missing);
    parts::draw(&rig, m, &look(sprite, light), ctx.list(Pass::Cutout));
}

/// A flat picture always facing the camera.
pub fn billboard(ctx: &mut Ctx, m: Mat4, sprite: Sprite, uv: [f32; 4], color: [u8; 4], light: [u8; 2], pass: Pass) {
    let (du, dv) = (sprite.u1 - sprite.u0, sprite.v1 - sprite.v0);
    let tc = |u: f32, v: f32| [sprite.u0 + u * du, sprite.v0 + v * dv];
    let corners = [(-0.5, 0.75, uv[0], uv[1]), (-0.5, -0.25, uv[0], uv[3]), (0.5, -0.25, uv[2], uv[3]), (0.5, 0.75, uv[2], uv[1])];
    let light4 = [light[0], light[1], 0, 0];
    let list = ctx.list(pass);
    for (x, y, u, v) in corners {
        list.push(Vertex { pos: m.transform_point3(Vec3::new(x, y, 0.0)).to_array(), uv: tc(u, v), color, light: light4 });
    }
}

fn item_field<'a>(e: &'a Entity, name: &str) -> Option<&'a ItemStack> {
    match e.field(name) {
        Some(DataValue::Item(Some(stack))) => Some(stack),
        _ => None,
    }
}

fn thrown_item(ctx: &mut Ctx, stack: Option<&ItemStack>, fallback: &str, pos: Vec3, scale: f32, light: [u8; 2]) {
    let own;
    let stack = match stack {
        Some(s) => s,
        None => {
            let Some(id) = crate::game::items::item_id(fallback) else { return };
            own = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
            &own
        }
    };
    let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_quat(ctx.camera_rot);
    held::draw_stack(ctx, stack, "ground", m, light, false);
}

fn wood_name(name: &str) -> Option<(&str, bool)> {
    if let Some(w) = name.strip_suffix("_chest_boat") {
        return Some((w, true));
    }
    if let Some(w) = name.strip_suffix("_boat") {
        return Some((w, false));
    }
    if let Some(w) = name.strip_suffix("_chest_raft") {
        return Some((w, true));
    }
    name.strip_suffix("_raft").map(|w| (w, false))
}

fn paddle(r: &mut Rig, part: &str, t: f32, right: bool) {
    let p = r.p(part);
    let a = (((-t).sin() + 1.0) / 2.0).clamp(0.0, 1.0);
    let b = (((-t + 1.0).sin() + 1.0) / 2.0).clamp(0.0, 1.0);
    p.rot.x = -1.0471976 + (-0.2617994 - -1.0471976) * a;
    p.rot.y = -0.7853982 + (0.7853982 - -0.7853982) * b;
    if right {
        p.rot.y = PI - p.rot.y;
    }
}

pub fn draw_thing(ctx: &mut Ctx, _session: &crate::game::Session, _world: &lattice_world::World, id: i32, e: &Entity, pos: Vec3, light: [u8; 2]) -> bool {
    let name = e.type_name();
    let age = e.age as f32 + ctx.partial;
    let yaw = super::rot_lerp(ctx.partial, e.prev_yaw, e.yaw);
    let pitch = e.prev_pitch + (e.pitch - e.prev_pitch) * ctx.partial;
    if let Some((wood, chest)) = wood_name(name) {
        let key = if chest { format!("chest_boat/{wood}") } else { format!("boat/{wood}") };
        let tex = if chest { format!("entity/chest_boat/{wood}") } else { format!("entity/boat/{wood}") };
        let hurt = e.int_field("ID_HURT").unwrap_or(0) as f32 - ctx.partial;
        let dmg = e.float_field("ID_DAMAGE").unwrap_or(0.0);
        let dir = e.int_field("ID_HURTDIR").unwrap_or(1) as f32;
        let mut m = Mat4::from_translation(pos + Vec3::new(0.0, 0.375, 0.0)) * Mat4::from_rotation_y((180.0 - yaw).to_radians());
        if hurt > 0.0 {
            m *= Mat4::from_rotation_x((hurt.sin() * hurt * (dmg - ctx.partial).max(0.0) / 10.0 * dir).to_radians());
        }
        m = m * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0)) * Mat4::from_rotation_y(FRAC_PI_2);
        let (left, right) = (e.bool_field("ID_PADDLE_LEFT"), e.bool_field("ID_PADDLE_RIGHT"));
        let row = age * PI / 8.0;
        model(ctx, &key, &tex, m, light, |r| {
            paddle(r, "left_paddle", if left { row } else { 0.0 }, false);
            paddle(r, "right_paddle", if right { row } else { 0.0 }, true);
        });
        return true;
    }
    match name {
        "item" => {
            if let Some(stack) = item_field(e, "ITEM") {
                held::draw_ground_item(ctx, stack, pos, id, age, light);
            }
        }
        "arrow" | "spectral_arrow" => {
            let tex = if name == "spectral_arrow" {
                "entity/projectiles/arrow_spectral"
            } else if e.int_field("ID_EFFECT_COLOR").is_some_and(|c| c != -1) {
                "entity/projectiles/arrow_tipped"
            } else {
                "entity/projectiles/arrow"
            };
            let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((yaw - 90.0).to_radians()) * Mat4::from_rotation_z(pitch.to_radians());
            model(ctx, "arrow", tex, m, light, |_| {});
        }
        "trident" => {
            let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((yaw - 90.0).to_radians()) * Mat4::from_rotation_z((pitch + 90.0).to_radians());
            model(ctx, "trident", "entity/trident/trident", m, light, |_| {});
        }
        "minecart" | "chest_minecart" | "furnace_minecart" | "hopper_minecart" | "spawner_minecart" | "command_block_minecart" | "tnt_minecart" => {
            let hurt = e.int_field("ID_HURT").unwrap_or(0) as f32 - ctx.partial;
            let dmg = e.float_field("ID_DAMAGE").unwrap_or(0.0);
            let dir = e.int_field("ID_HURTDIR").unwrap_or(1) as f32;
            let mut m = Mat4::from_translation(pos + Vec3::new(0.0, 0.375, 0.0)) * Mat4::from_rotation_y((180.0 - yaw).to_radians()) * Mat4::from_rotation_z((-pitch).to_radians());
            if hurt > 0.0 {
                m *= Mat4::from_rotation_x((hurt.sin() * hurt * (dmg - ctx.partial).max(0.0) / 10.0 * dir).to_radians());
            }
            // What it carries: the custom block if set, otherwise what its type implies.
            let custom = match e.field("ID_CUSTOM_DISPLAY_BLOCK") {
                Some(DataValue::OptBlockState(Some(s))) => Some(*s as u32),
                _ => None,
            };
            let carried = custom.or_else(|| {
                let block = match name {
                    "chest_minecart" => "chest",
                    "furnace_minecart" => "furnace",
                    "hopper_minecart" => "hopper",
                    "spawner_minecart" => "spawner",
                    "command_block_minecart" => "command_block",
                    "tnt_minecart" => "tnt",
                    _ => return None,
                };
                lattice_data::game().blocks.iter().find(|b| b.name == block).map(|b| b.default)
            });
            if let Some(state) = carried {
                let offset = e.int_field("ID_DISPLAY_OFFSET").unwrap_or(6) as f32;
                let at = m * Mat4::from_scale(Vec3::splat(0.75)) * Mat4::from_translation(Vec3::new(-0.5, (offset - 8.0) / 16.0, 0.5)) * Mat4::from_rotation_y(FRAC_PI_2);
                if lattice_data::game().block(state).name == "chest" {
                    let be_m = at;
                    super::blockent::draw_special(ctx, &ItemStack { item: 0, count: 1, added: Vec::new(), removed: Vec::new() }, "chest", Some("normal"), be_m, light);
                } else {
                    held::draw_block(ctx, state, at, light, 0, None);
                }
            }
            let skin = if name == "minecart" { "minecart" } else { name };
            model(ctx, skin, "entity/minecart/minecart", m * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0)), light, |_| {});
        }
        "painting" => draw_painting(ctx, e, pos, light),
        "item_frame" | "glow_item_frame" => draw_frame(ctx, e, pos, light, name == "glow_item_frame"),
        "falling_block" => {
            let at = Mat4::from_translation(pos + Vec3::new(-0.5, 0.0, -0.5));
            held::draw_block(ctx, e.object_data as u32, at, light, 0, None);
        }
        "tnt" => {
            let fuse = e.int_field("FUSE").unwrap_or(80) as f32 - ctx.partial + 1.0;
            let state = match e.field("BLOCK_STATE") {
                Some(DataValue::BlockState(s)) => *s as u32,
                _ => lattice_data::game().blocks.iter().find(|b| b.name == "tnt").map_or(0, |b| b.default),
            };
            let mut m = Mat4::from_translation(pos + Vec3::new(0.0, 0.5, 0.0));
            if fuse < 10.0 {
                let t = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                let t = t * t;
                let t = t * t;
                m *= Mat4::from_scale(Vec3::splat(1.0 + t * 0.3));
            }
            m = m * Mat4::from_rotation_y(-FRAC_PI_2) * Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.5)) * Mat4::from_rotation_y(FRAC_PI_2);
            let white = if (fuse as i32 / 5) % 2 == 0 { Some(0.6) } else { None };
            held::draw_block(ctx, state, m, light, 0, white);
        }
        "egg" | "snowball" | "ender_pearl" | "experience_bottle" | "splash_potion" | "lingering_potion" => {
            thrown_item(ctx, item_field(e, "ITEM_STACK"), name, pos, 1.0, light);
        }
        "fireball" => thrown_item(ctx, item_field(e, "ITEM_STACK"), "fire_charge", pos, 3.0, [light[0], 255]),
        "small_fireball" => thrown_item(ctx, item_field(e, "ITEM_STACK"), "fire_charge", pos, 0.75, [light[0], 255]),
        "eye_of_ender" => thrown_item(ctx, item_field(e, "ITEM_STACK"), "ender_eye", pos, 1.0, [light[0], 255]),
        "firework_rocket" => {
            let Some(stack) = item_field(e, "ID_FIREWORKS_ITEM").cloned().or_else(|| crate::game::items::item_id("firework_rocket").map(|id| ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() })) else { return true };
            let mut m = Mat4::from_translation(pos) * Mat4::from_quat(ctx.camera_rot);
            if e.bool_field("SHOT_AT_ANGLE") {
                m = m * Mat4::from_rotation_z(PI) * Mat4::from_rotation_y(PI) * Mat4::from_rotation_x(FRAC_PI_2);
            }
            held::draw_stack(ctx, &stack, "ground", m, light, false);
        }
        "experience_orb" => {
            let Some(sprite) = ctx.sprite("entity/experience/experience_orb") else { return true };
            let value = e.int_field("VALUE").unwrap_or(1);
            let icon = match value {
                v if v >= 2477 => 10,
                v if v >= 1237 => 9,
                v if v >= 617 => 8,
                v if v >= 307 => 7,
                v if v >= 149 => 6,
                v if v >= 73 => 5,
                v if v >= 37 => 4,
                v if v >= 17 => 3,
                v if v >= 7 => 2,
                v if v >= 3 => 1,
                _ => 0,
            };
            let uv = [(icon % 4) as f32 * 16.0 / 64.0, (icon / 4) as f32 * 16.0 / 64.0, ((icon % 4) as f32 * 16.0 + 16.0) / 64.0, ((icon / 4) as f32 * 16.0 + 16.0) / 64.0];
            let t = age / 2.0;
            let r = (((t).sin() + 1.0) * 0.5 * 255.0) as u8;
            let b = (((t + 4.1887903).sin() + 1.0) * 0.1 * 255.0) as u8;
            let m = Mat4::from_translation(pos + Vec3::new(0.0, 0.1, 0.0)) * Mat4::from_quat(ctx.camera_rot) * Mat4::from_scale(Vec3::splat(0.3));
            billboard(ctx, m, sprite, uv, [r, 255, b, 128], [255, 255], Pass::Translucent);
        }
        "dragon_fireball" => {
            let Some(sprite) = ctx.sprite("entity/enderdragon/dragon_fireball") else { return true };
            let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(2.0)) * Mat4::from_quat(ctx.camera_rot);
            billboard(ctx, m, sprite, [0.0, 0.0, 1.0, 1.0], WHITE, [255, 255], Pass::Cutout);
        }
        "fishing_bobber" => {
            let Some(sprite) = ctx.sprite("entity/fishing/fishing_hook") else { return true };
            let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(0.5)) * Mat4::from_quat(ctx.camera_rot);
            billboard(ctx, m, sprite, [0.0, 0.0, 1.0, 1.0], WHITE, light, Pass::Cutout);
        }
        "wither_skull" => {
            let tex = if e.bool_field("DANGEROUS") { "entity/wither/wither_invulnerable" } else { "entity/wither/wither" };
            let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            let (y, p) = (wrap_degrees(yaw), pitch);
            model(ctx, "wither_skull", tex, m, light, |r| {
                let h = r.p("head");
                h.rot.y = y.to_radians();
                h.rot.x = p.to_radians();
            });
        }
        "shulker_bullet" => {
            let m = Mat4::from_translation(pos + Vec3::new(0.0, 0.15, 0.0))
                * Mat4::from_rotation_y(((age * 0.1).sin() * 180.0).to_radians())
                * Mat4::from_rotation_x(((age * 0.1).cos() * 180.0).to_radians())
                * Mat4::from_rotation_z(((age * 0.15).sin() * 360.0).to_radians())
                * Mat4::from_scale(Vec3::new(-0.5, -0.5, 0.5));
            let (y, p) = (yaw, pitch);
            let pose = |r: &mut Rig| {
                let main = r.p("main");
                main.rot.y = y.to_radians();
                main.rot.x = p.to_radians();
            };
            model(ctx, "shulker_bullet", "entity/shulker/spark", m, [255, 255], pose);
            if let Some(skel) = ctx.bodies.skeleton("shulker_bullet") {
                let mut rig = Rig::new(skel);
                pose(&mut rig);
                let sprite = ctx.sprite("entity/shulker/spark").unwrap_or(ctx.bodies.sheet.missing);
                let mut l = look(sprite, [255, 255]);
                l.color = [255, 255, 255, 38];
                parts::draw(&rig, m * Mat4::from_scale(Vec3::splat(1.5)), &l, ctx.list(Pass::Translucent));
            }
        }
        "llama_spit" => {
            let m = Mat4::from_translation(pos + Vec3::new(0.0, 0.15, 0.0)) * Mat4::from_rotation_y((yaw - 90.0).to_radians()) * Mat4::from_rotation_z(pitch.to_radians());
            model(ctx, "llama_spit", "entity/llama/llama_spit", m, light, |_| {});
        }
        "wind_charge" | "breeze_wind_charge" => {
            let m = Mat4::from_translation(pos);
            model(ctx, "wind_charge", "entity/projectiles/wind_charge", m, light, |r| {
                r.p("wind_charge").rot.y = (-age * 16.0).to_radians();
                r.p("wind").rot.y = (age * 16.0).to_radians();
            });
        }
        "evoker_fangs" => {
            // Fangs rise, bite and sink over twenty ticks once they appear.
            let bite = ((age - 2.0).max(0.0) / 20.0).min(1.0);
            let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((90.0 - yaw).to_radians()) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0)) * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0));
            model(ctx, "evoker_fangs", "entity/illager/evoker_fangs", m, light, |r| {
                let v = (bite * 2.0).min(1.0);
                let v = 1.0 - v * v * v;
                r.p("upper_jaw").rot.z = PI - v * 0.35 * PI;
                r.p("lower_jaw").rot.z = PI + v * 0.35 * PI;
                r.p("base").pos.y -= (bite + (bite * 2.7).sin()) * 7.2;
                let k = if bite > 0.9 { (1.0 - bite) / 0.1 } else { 1.0 };
                let root = r.p("root");
                root.pos.y = 24.0 - 20.0 * k;
                root.scale = Vec3::splat(k);
            });
        }
        "end_crystal" => {
            let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(2.0)) * Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0));
            let spin = age * 3.0;
            let tilt = Quat::from_axis_angle(Vec3::new(std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2), 1.0471976);
            let show_base = e.bool_field("SHOW_BOTTOM");
            let bob = {
                let t = age * 0.2;
                let v = t.sin() / 2.0 + 0.5;
                ((v * v + v) * 0.4 - 1.4) * 16.0
            };
            model(ctx, "end_crystal", "entity/end_crystal/end_crystal", m, [255, 255], |r| {
                r.p("base").visible = show_base;
                let outer = r.p("outer_glass");
                outer.pos.y += bob / 2.0;
                outer.extra = Some(Quat::from_rotation_y(spin.to_radians()) * tilt);
                r.p("inner_glass").extra = Some(tilt * Quat::from_rotation_y(spin.to_radians()));
                r.p("cube").extra = Some(tilt * Quat::from_rotation_y(spin.to_radians()));
            });
        }
        "leash_knot" => {
            let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            model(ctx, "leash_knot", "entity/lead_knot/lead_knot", m, light, |_| {});
        }
        "block_display" => {
            let state = match e.field("BLOCK_STATE") {
                Some(DataValue::BlockState(s)) => *s as u32,
                _ => return true,
            };
            let m = display_matrix(ctx, e, pos);
            held::draw_block(ctx, state, m, display_light(e, light), 0, None);
        }
        "item_display" => {
            let Some(stack) = item_field(e, "ITEM_STACK") else { return true };
            let context = match e.int_field("ITEM_DISPLAY").unwrap_or(0) {
                1 => "thirdperson_lefthand",
                2 => "thirdperson_righthand",
                3 => "firstperson_lefthand",
                4 => "firstperson_righthand",
                5 => "head",
                6 => "gui",
                7 => "ground",
                8 => "fixed",
                _ => "none",
            };
            let m = display_matrix(ctx, e, pos) * Mat4::from_rotation_y(PI);
            held::draw_stack(ctx, stack, context, m, display_light(e, light), false);
        }
        "ominous_item_spawner" => {
            if let Some(stack) = item_field(e, "ITEM") {
                let s = (age / 50.0).min(1.0);
                let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((age * 40.0).to_radians() / 20.0) * Mat4::from_scale(Vec3::splat(s));
                held::draw_stack(ctx, stack, "ground", m, [255, 255], false);
            }
        }
        // Nothing to see, or particles only.
        "text_display" => draw_text_display(ctx, e, pos, light),
        "lightning_bolt" => draw_lightning(ctx, id, pos),
        "marker" | "interaction" | "area_effect_cloud" | "cushion" => {}
        _ => return false,
    }
    true
}

/// Words hanging in the air, placed like any other display.
fn draw_text_display(ctx: &mut Ctx, e: &Entity, pos: Vec3, light: [u8; 2]) {
    let Some(DataValue::Text(raw)) = e.field("TEXT") else { return };
    let text = lattice_proto::Text::from_nbt(raw);
    let spans = text.spans(ctx.lang);
    let line_width = e.int_field("LINE_WIDTH").unwrap_or(200) as f32;
    let background = e.int_field("BACKGROUND_COLOR").unwrap_or(0x40000000) as u32;
    let flags = e.int_field("STYLE_FLAGS").unwrap_or(0);
    let opacity = e.int_field("TEXT_OPACITY").unwrap_or(-1);
    let alpha = if opacity == -1 || opacity == 255 { 255 } else { opacity.clamp(0, 255) as u8 };
    // Wrap on spaces, as wide as the display asks for.
    let font = &ctx.bodies.font;
    let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
    let mut lines: Vec<String> = Vec::new();
    for raw_line in plain.split('\n') {
        let mut current = String::new();
        for word in raw_line.split(' ') {
            let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
            let probe = [lattice_proto::text::Span { text: candidate.clone(), ..Default::default() }];
            if !current.is_empty() && font.width(&probe) > line_width {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        lines.push(current);
    }
    let first_color = spans.iter().find_map(|s| s.color);
    let widths: Vec<f32> = lines.iter().map(|l| font.width(&[lattice_proto::text::Span { text: l.clone(), ..Default::default() }])).collect();
    let widest = widths.iter().copied().fold(0.0, f32::max);
    let height = lines.len() as f32 * text_line();
    let m = display_matrix(ctx, e, pos) * Mat4::from_rotation_y(PI) * Mat4::from_scale(Vec3::new(0.025, -0.025, 0.025)) * Mat4::from_translation(Vec3::new(0.0, -height, 0.0));
    let lit = if e.int_field("BRIGHTNESS_OVERRIDE").is_some_and(|v| v >= 0) { display_light(e, light) } else { light };
    if (background >> 24) & 255 != 0 {
        let c = [(background >> 16) as u8, (background >> 8) as u8, background as u8, (background >> 24) as u8];
        super::text::draw_rect(ctx, -widest / 2.0 - 1.0, -1.0, widest / 2.0 + 1.0, height, 0.0, m, c, lit, Pass::Translucent);
    }
    let color = first_color.map_or([255, 255, 255], |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    for (i, line) in lines.iter().enumerate() {
        let x = match flags & 24 {
            8 => -widest / 2.0,
            16 => widest / 2.0 - widths[i],
            _ => -widths[i] / 2.0,
        };
        let span = [lattice_proto::text::Span { text: line.clone(), ..Default::default() }];
        let m = m * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.02));
        super::text::draw_line(ctx, &span, x, i as f32 * text_line(), m, super::text::TextStyle { color, alpha, light: lit, pass: if alpha == 255 { Pass::Cutout } else { Pass::Translucent } });
    }
}

fn text_line() -> f32 {
    super::text::LINE_HEIGHT + 1.0
}

fn display_light(e: &Entity, light: [u8; 2]) -> [u8; 2] {
    match e.int_field("BRIGHTNESS_OVERRIDE") {
        Some(v) if v >= 0 => [(((v >> 20) & 15) * 17) as u8, (((v >> 4) & 15) * 17) as u8],
        _ => light,
    }
}

/// The transform of a display entity from its synced parts.
fn display_matrix(ctx: &Ctx, e: &Entity, pos: Vec3) -> Mat4 {
    let vec = |n: &str, d: [f32; 3]| match e.field(n) {
        Some(DataValue::Vector3(v)) => Vec3::from(*v),
        _ => Vec3::from(d),
    };
    let quat = |n: &str| match e.field(n) {
        Some(DataValue::Quaternion(q)) => Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize(),
        _ => Quat::IDENTITY,
    };
    let t = vec("TRANSLATION", [0.0, 0.0, 0.0]);
    let s = vec("SCALE", [1.0, 1.0, 1.0]);
    let base = match e.int_field("BILLBOARD_RENDER_CONSTRAINTS").unwrap_or(0) {
        3 => Mat4::from_quat(ctx.camera_rot),
        1 => Mat4::from_rotation_y(ctx.camera_yaw_rad),
        2 => Mat4::from_rotation_x(ctx.camera_pitch_rad),
        _ => Mat4::from_rotation_y(-e.yaw.to_radians()) * Mat4::from_rotation_x(e.pitch.to_radians()),
    };
    Mat4::from_translation(pos) * base * Mat4::from_translation(t) * Mat4::from_quat(quat("LEFT_ROTATION")) * Mat4::from_scale(s) * Mat4::from_quat(quat("RIGHT_ROTATION"))
}

fn direction_of(e: &Entity) -> i32 {
    match e.field("DIRECTION") {
        Some(DataValue::Direction(d)) => *d,
        _ => e.object_data,
    }
}

/// Horizontal directions numbered south, west, north, east.
fn direction_2d(d: i32) -> i32 {
    match d {
        3 => 0,
        4 => 1,
        2 => 2,
        5 => 3,
        _ => 0,
    }
}

fn draw_painting(ctx: &mut Ctx, e: &Entity, pos: Vec3, light: [u8; 2]) {
    let id = e.int_field("PAINTING_VARIANT").unwrap_or(0);
    let Some(name) = ctx.variant("painting_variant", id).map(str::to_string) else { return };
    let Some(front) = ctx.sprite(&format!("painting/{name}")) else { return };
    let back = ctx.sprite("painting/back").unwrap_or(front);
    let sheet = &ctx.bodies.sheet;
    let w = ((front.u1 - front.u0) * sheet.width as f32 / 16.0).round().max(1.0);
    let h = ((front.v1 - front.v0) * sheet.height as f32 / 16.0).round().max(1.0);
    let dir = direction_of(e);
    // Even sizes sit half a block over from the block they're hung in.
    let along = match dir {
        2 => Vec3::NEG_X,
        3 => Vec3::X,
        4 => Vec3::Z,
        _ => Vec3::NEG_Z,
    };
    let pos = pos + along * if (w as i32) % 2 == 0 { 0.5 } else { 0.0 } + Vec3::Y * if (h as i32) % 2 == 0 { 0.5 } else { 0.0 };
    let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((180.0 - direction_2d(dir) as f32 * 90.0).to_radians());
    let (hw, hh, d) = (w / 2.0, h / 2.0, 0.03125);
    let light4 = [light[0], light[1], 0, 0];
    let quad = |ctx: &mut Ctx, p: [Vec3; 4], s: Sprite, n: Vec3| {
        let normal = m.transform_vector3(n).normalize_or_zero();
        let shade = (parts::entity_shade(normal) * 255.0) as u8;
        let color = [shade, shade, shade, 255];
        let uv = [[s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1], [s.u0, s.v0]];
        let list = ctx.list(Pass::Cutout);
        for k in 0..4 {
            list.push(Vertex { pos: m.transform_point3(p[k]).to_array(), uv: uv[k], color, light: light4 });
        }
    };
    quad(ctx, [Vec3::new(hw, hh, -d), Vec3::new(hw, -hh, -d), Vec3::new(-hw, -hh, -d), Vec3::new(-hw, hh, -d)], front, Vec3::NEG_Z);
    quad(ctx, [Vec3::new(-hw, hh, d), Vec3::new(-hw, -hh, d), Vec3::new(hw, -hh, d), Vec3::new(hw, hh, d)], back, Vec3::Z);
    quad(ctx, [Vec3::new(hw, hh, d), Vec3::new(hw, hh, -d), Vec3::new(-hw, hh, -d), Vec3::new(-hw, hh, d)], back, Vec3::Y);
    quad(ctx, [Vec3::new(-hw, -hh, d), Vec3::new(-hw, -hh, -d), Vec3::new(hw, -hh, -d), Vec3::new(hw, -hh, d)], back, Vec3::NEG_Y);
    quad(ctx, [Vec3::new(hw, -hh, d), Vec3::new(hw, -hh, -d), Vec3::new(hw, hh, -d), Vec3::new(hw, hh, d)], back, Vec3::X);
    quad(ctx, [Vec3::new(-hw, hh, d), Vec3::new(-hw, hh, -d), Vec3::new(-hw, -hh, -d), Vec3::new(-hw, -hh, d)], back, Vec3::NEG_X);
}

/// A bolt: jagged segments from the sky down, with branches. The shape's only random, so any
/// seed will do; this one's kept steady for as long as the bolt lasts.
fn draw_lightning(ctx: &mut Ctx, id: i32, pos: Vec3) {
    let mut state = (id as u32).wrapping_mul(2654435761) | 1;
    let mut next = move |n: i32| -> f32 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % n as u32) as i32 as f32
    };
    let (mut xs, mut zs) = ([0.0f32; 8], [0.0f32; 8]);
    let (mut lx, mut lz) = (0.0, 0.0);
    for i in (0..8).rev() {
        xs[i] = lx;
        zs[i] = lz;
        lx += next(11) - 5.0;
        lz += next(11) - 5.0;
    }
    let m = Mat4::from_translation(pos);
    let seed_state = (id as u32).wrapping_mul(40503) | 1;
    for layer in 0..4 {
        let mut s2 = seed_state;
        let mut next2 = move |n: i32| -> f32 {
            s2 ^= s2 << 13;
            s2 ^= s2 >> 17;
            s2 ^= s2 << 5;
            (s2 % n as u32) as i32 as f32
        };
        for part in 0..3 {
            let (mut top, mut bottom) = (7i32, 0i32);
            if part > 0 {
                top = 7 - part;
                bottom = top - 2;
            }
            let (mut x, mut z) = (xs[top as usize] - lx, zs[top as usize] - lz);
            let mut s = top;
            while s >= bottom {
                let (px, pz) = (x, z);
                if part == 0 {
                    x += next2(11) - 5.0;
                    z += next2(11) - 5.0;
                } else {
                    x += next2(31) - 15.0;
                    z += next2(31) - 15.0;
                }
                let mut low = 0.1 + layer as f32 * 0.2;
                if part == 0 {
                    low *= s as f32 * 0.1 + 1.0;
                }
                let mut high = 0.1 + layer as f32 * 0.2;
                if part == 0 {
                    high *= (s as f32 - 1.0) * 0.1 + 1.0;
                }
                for (a, b, c, d) in [(false, false, true, false), (true, false, true, true), (true, true, false, true), (false, true, false, false)] {
                    bolt_quad(ctx, m, x, z, s, px, pz, low, high, a, b, c, d);
                }
                s -= 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn bolt_quad(ctx: &mut Ctx, m: Mat4, x: f32, z: f32, s: i32, px: f32, pz: f32, low: f32, high: f32, a: bool, b: bool, c: bool, d: bool) {
    let white = ctx.bodies.sheet.white;
    let uv = [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5];
    let color = [115, 115, 128, 77];
    let side = |v: bool, k: f32| if v { k } else { -k };
    let (y0, y1) = ((s * 16) as f32, ((s + 1) * 16) as f32);
    let points = [
        (x + side(a, high), y0, z + side(b, high)),
        (px + side(a, low), y1, pz + side(b, low)),
        (px + side(c, low), y1, pz + side(d, low)),
        (x + side(c, high), y0, z + side(d, high)),
    ];
    let list = ctx.list(super::Pass::Glow);
    for (px, py, pz) in points {
        list.push(Vertex { pos: m.transform_point3(Vec3::new(px, py, pz)).to_array(), uv, color, light: [255, 255, 0, 0] });
    }
}

fn draw_frame(ctx: &mut Ctx, e: &Entity, pos: Vec3, light: [u8; 2], glow: bool) {
    let dir = direction_of(e);
    let step = crate::game::entities::direction_step(dir).as_vec3();
    let (rx, ry) = match dir {
        0 => (90.0, 180.0),
        1 => (-90.0, 180.0),
        2 => (0.0, 0.0),
        3 => (0.0, 180.0),
        4 => (0.0, 90.0),
        _ => (0.0, 270.0),
    };
    let m = Mat4::from_translation(pos + step * 0.46875) * Mat4::from_rotation_x(f32::to_radians(rx)) * Mat4::from_rotation_y(f32::to_radians(ry));
    let frame = if glow { "block/glow_item_frame" } else { "block/item_frame" };
    let stack = item_field(e, "ITEM");
    let is_map = stack.is_some_and(|s| s.raw("map_id").is_some() || crate::game::items::has_component(s, "map_id"));
    let shape_name = if is_map { format!("{frame}_map") } else { frame.to_string() };
    let invisible = e.flag(5);
    if !invisible {
        if let Some(shape) = ctx.items.and_then(|v| v.shape(&shape_name)) {
            held::draw_shape(ctx, shape, &[], m * Mat4::from_translation(Vec3::splat(-0.5)), light);
        }
    }
    let Some(stack) = stack else { return };
    let rotation = e.int_field("ROTATION").unwrap_or(0) as f32;
    let front = m * Mat4::from_translation(Vec3::new(0.0, 0.0, if invisible { 0.5 } else { 0.4375 }));
    if let (true, Some(id)) = (is_map, stack.var_int("map_id")) {
        // Maps turn in quarter steps, and sit a hair in front of the backing.
        let turn = (rotation as i32 % 4) * 2;
        let at = front * Mat4::from_rotation_z((turn as f32 * 45.0).to_radians()) * Mat4::from_rotation_z(PI) * Mat4::from_scale(Vec3::splat(1.0 / 128.0)) * Mat4::from_translation(Vec3::new(-64.0, -64.0, -1.0));
        let map_light = if glow { [223, 255] } else { light };
        super::maps::draw(ctx, at, id, true, map_light);
        return;
    }
    let item_light = if glow { [255, 255] } else { light };
    let at = front * Mat4::from_rotation_z((rotation * 45.0).to_radians()) * Mat4::from_scale(Vec3::splat(0.5));
    held::draw_stack(ctx, stack, "fixed", at, item_light, false);
}
