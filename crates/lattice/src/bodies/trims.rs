//! Things added to bodies: armour trims, what's stuck in a player, parrots on shoulders, leads.

use super::mobs::WHITE;
use super::{Ctx, St};
use crate::game::entities::Entity;
use glam::{DVec3, Mat4, Vec3};
use lattice_assets::Resources;
use lattice_proto::{DataValue, ItemStack};
use lattice_render::mesher::Vertex;
use lattice_render::parts::Rig;
use std::collections::HashMap;

/// Every trim in every metal, recoloured from the grey original: the pictures' greys are looked
/// up in one row of colours to find the place in the metal's row.
pub fn load(res: &Resources, textures: &mut Vec<(String, image::RgbaImage)>) {
    let Some(base) = res.image("textures/palettes/trim_base.png") else {
        tracing::warn!("no picture of the trim colours");
        return;
    };
    let keys: Vec<[u8; 4]> = base.pixels().map(|p| p.0).collect();
    let mut metals = Vec::new();
    for path in res.list("textures/palettes/trim/", ".png") {
        let name = path.trim_start_matches("textures/palettes/trim/").trim_end_matches(".png").to_string();
        if let Some(img) = res.image(&path) {
            let colors: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
            let map: HashMap<[u8; 4], [u8; 4]> = keys.iter().copied().zip(colors).collect();
            metals.push((name, map));
        }
    }
    for dir in ["humanoid", "humanoid_leggings"] {
        for path in res.list(&format!("textures/trims/entity/{dir}/"), ".png") {
            let pattern = path.rsplit('/').next().unwrap_or("").trim_end_matches(".png").to_string();
            let Some(img) = res.image(&path) else { continue };
            for (metal, map) in &metals {
                let mut out = img.clone();
                for p in out.pixels_mut() {
                    if p[3] != 0 {
                        if let Some(c) = map.get(&p.0) {
                            p.0 = *c;
                        }
                    }
                }
                textures.push((format!("trims/entity/{dir}/{pattern}_{metal}"), out));
            }
        }
    }
}

/// A worn piece's trim: pattern and metal, by the names of their pictures.
pub fn trim_of(stack: &ItemStack, registries: Option<&lattice_world::Registries>) -> Option<(String, String)> {
    let raw = stack.raw("trim")?;
    let reg = registries?;
    let (material, pattern) = lattice_proto::item::read_raw(raw, |r| {
        let m = r.var_i32()?;
        if m == 0 {
            return Ok((None, None));
        }
        let p = r.var_i32()?;
        Ok((Some(m - 1), (p != 0).then_some(p - 1)))
    })?;
    let material = reg.names.get("trim_material")?.get(material? as usize)?.trim_start_matches("minecraft:").to_string();
    let pattern = reg.names.get("trim_pattern")?.get(pattern? as usize)?.trim_start_matches("minecraft:").to_string();
    Some((pattern, material))
}

/// Small repeatable numbers, the same for an entity each frame.
struct Dice(u32);

impl Dice {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % 10000) as f32 / 10000.0
    }
}

/// Arrows and bee stings a player's been hit by, at random spots that stay put.
pub fn stuck_in_body(ctx: &mut Ctx, e: &Entity, rig: &Rig, m: Mat4, s: &St) {
    let arrows = e.int_field("ARROW_COUNT").unwrap_or(0);
    let stings = e.int_field("STINGER_COUNT").unwrap_or(0);
    if arrows <= 0 && stings <= 0 {
        return;
    }
    const PARTS: [&str; 6] = ["head", "body", "left_arm", "right_arm", "left_leg", "right_leg"];
    let mut dice = Dice((e.uuid.as_u128() as u32) | 1);
    let spot = |dice: &mut Dice, on_surface: bool| {
        let part = PARTS[(dice.next() * 6.0) as usize % 6];
        let (a, b) = rig.cube_of(part).unwrap_or(([0.0; 3], [1.0; 3]));
        let mut f = [dice.next(), dice.next(), dice.next()];
        if on_surface {
            let axis = (dice.next() * 3.0) as usize % 3;
            f[axis] = if f[axis] < 0.5 { 0.0 } else { 1.0 };
        }
        let at = m * rig.matrix(part) * Mat4::from_translation(Vec3::new((a[0] + (b[0] - a[0]) * f[0]) / 16.0, (a[1] + (b[1] - a[1]) * f[1]) / 16.0, (a[2] + (b[2] - a[2]) * f[2]) / 16.0));
        (at, [-(f[0] * 2.0 - 1.0), -(f[1] * 2.0 - 1.0), -(f[2] * 2.0 - 1.0)])
    };
    for _ in 0..arrows.min(32) {
        let (at, d) = spot(&mut dice, false);
        let flat = (d[0] * d[0] + d[2] * d[2]).sqrt();
        let (yaw, pitch) = (d[0].atan2(d[2]).to_degrees(), d[1].atan2(flat).to_degrees());
        let arrow = at * Mat4::from_rotation_y((yaw - 90.0).to_radians()) * Mat4::from_rotation_z(pitch.to_radians());
        super::things::model(ctx, "arrow", "entity/projectiles/arrow", arrow, s.light, |_| {});
    }
    for _ in 0..stings.min(32) {
        let (at, d) = spot(&mut dice, true);
        let flat = (d[0] * d[0] + d[2] * d[2]).sqrt();
        let (yaw, pitch) = (d[0].atan2(d[2]), d[1].atan2(flat));
        let sting = at * Mat4::from_rotation_y(yaw + std::f32::consts::PI) * Mat4::from_rotation_x(pitch) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
        super::things::model(ctx, "bee_stinger#main", "entity/bee/bee_stinger", sting, s.light, |_| {});
    }
}

/// Parrots riding on a player's shoulders.
pub fn shoulder_parrots(ctx: &mut Ctx, e: &Entity, m: Mat4, s: &St) {
    for (field, side) in [("SHOULDER_PARROT_LEFT", 0.4f32), ("SHOULDER_PARROT_RIGHT", -0.4)] {
        let Some(DataValue::OptUnsignedInt(Some(variant))) = e.field(field) else { continue };
        let Some(skel) = ctx.bodies.skeleton("parrot") else { continue };
        let color = ["red_blue", "blue", "green", "yellow_blue", "grey"][(*variant).clamp(0, 4) as usize];
        let mut rig = Rig::new(skel);
        super::anim::parrot_on_shoulder(&mut rig);
        let at = m * Mat4::from_translation(Vec3::new(side, if s.crouching { -1.3 } else { -1.5 }, 0.0));
        let Some(sprite) = ctx.sprite(&format!("entity/parrot/parrot_{color}")) else { continue };
        super::draw_layer(ctx, &rig, at, sprite, s, WHITE, super::Pass::Cutout, true);
    }
}

/// Where a lead ends on what's holding it.
fn holding_point(ctx: &Ctx, session: &crate::game::Session, id: i32, knot_or_mob: Option<&Entity>) -> Option<DVec3> {
    let partial = ctx.partial as f64;
    if id == session.player.entity_id {
        let p = &session.player;
        let at = p.prev_pos.lerp(p.body.pos, partial);
        return Some(player_hand(at, p.body_yaw(ctx.partial), p.body.aabb().max.y - p.body.aabb().min.y, p.body.crouching));
    }
    let e = knot_or_mob?;
    let at = e.prev_pos.lerp(e.pos, partial);
    let (_, h) = e.dimensions();
    Some(match e.type_name() {
        "leash_knot" => at + DVec3::new(0.0, 0.2, 0.0),
        "player" => player_hand(at, e.prev_body_yaw + (e.body_yaw - e.prev_body_yaw) * ctx.partial, h, e.flag(1)),
        _ => at + DVec3::new(0.0, e.info().map_or(h, |i| i.eye as f64) * 0.7, 0.0),
    })
}

fn player_hand(at: DVec3, body_yaw: f32, height: f64, crouching: bool) -> DVec3 {
    let (d, l, m) = (-0.22f64, height - 1.0, if crouching { -0.2 } else { 0.07 });
    let g = (-body_yaw).to_radians() as f64;
    let (s, c) = (g.sin(), g.cos());
    at + DVec3::new(d * c + m * s, l, m * c - d * s)
}

/// A lead between something and what's holding it: a hanging ribbon of alternating shades.
pub fn draw_leash(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, e: &Entity, _id: i32, at: DVec3, camera: DVec3) {
    let Some(holder) = e.leashed_to else { return };
    let Some(end) = holding_point(ctx, session, holder, session.entities.map.get(&holder)) else { return };
    let (w, _) = e.dimensions();
    let yaw = (e.prev_body_yaw + (e.body_yaw - e.prev_body_yaw) * ctx.partial).to_radians() as f64;
    let eye = e.info().map_or(0.5, |i| i.eye as f64);
    let (ox, oz) = ((w * 0.4) * -yaw.sin(), (w * 0.4) * yaw.cos());
    let start = at + DVec3::new(ox, eye, oz);
    let d = (end - start).as_vec3();
    let k = (d.x * d.x + d.z * d.z).sqrt().max(1e-4).recip() * 0.05 / 2.0;
    let (fx, fz) = (d.z * k, d.x * k);
    let light_of = |p: DVec3| super::light_at(world, p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let (l0, l1) = (light_of(start), light_of(end));
    let origin = (start - camera).as_vec3();
    let white = ctx.bodies.sheet.white;
    let uv = [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5];
    let pair = |i: i32, y_offset: f32, flip: bool| {
        let t = i as f32 / 24.0;
        let light = [(l0[0] as f32 + (l1[0] as f32 - l0[0] as f32) * t) as u8, (l0[1] as f32 + (l1[1] as f32 - l0[1] as f32) * t) as u8, 0, 0];
        let shade = if i % 2 == flip as i32 { 0.7 } else { 1.0 };
        let color = [(127.5 * shade) as u8, (102.0 * shade) as u8, (76.5 * shade) as u8, 255];
        let y = if d.y > 0.0 { d.y * t * t } else { d.y - d.y * (1.0 - t) * (1.0 - t) };
        let a = origin + Vec3::new(d.x * t - fx, y + y_offset, d.z * t + fz);
        let b = origin + Vec3::new(d.x * t + fx, y + 0.05 - y_offset, d.z * t - fz);
        (a, b, color, light)
    };
    let list = ctx.list(super::Pass::Cutout);
    for (offset, flip) in [(0.05f32, false), (0.0, true)] {
        for i in 0..24 {
            let (a0, b0, c0, l_a) = pair(i, offset, flip);
            let (a1, b1, _, l_b) = pair(i + 1, offset, flip);
            for (p, l) in [(a0, l_a), (b0, l_a), (b1, l_b), (a1, l_b)] {
                list.push(Vertex { pos: p.to_array(), uv, color: c0, light: l });
            }
        }
    }
}

/// The line from a fishing rod to its float: slack near the float, as a line hangs.
pub fn draw_fishing_line(ctx: &mut Ctx, session: &crate::game::Session, e: &Entity, at: DVec3, camera: DVec3) {
    let owner = e.object_data;
    let Some(end) = holding_point(ctx, session, owner, session.entities.map.get(&owner)) else { return };
    let start = at + DVec3::new(0.0, 0.25, 0.0);
    let d = (end - start).as_vec3();
    let origin = (start - camera).as_vec3();
    let white = ctx.bodies.sheet.white;
    let uv = [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5];
    let point = |i: i32| {
        let t = i as f32 / 16.0;
        origin + Vec3::new(d.x * t, d.y * (t * t + t) * 0.5, d.z * t)
    };
    // Two thin ribbons crossed, so the line has width from any side.
    let side = Vec3::new(d.z, 0.0, -d.x).normalize_or_zero() * 0.012;
    let up = Vec3::new(0.0, 0.012, 0.0);
    let list = ctx.list(super::Pass::Cutout);
    for offset in [side, up] {
        for i in 0..16 {
            let (a, b) = (point(i), point(i + 1));
            for p in [a - offset, a + offset, b + offset, b - offset] {
                list.push(Vertex { pos: p.to_array(), uv, color: [20, 20, 20, 255], light: [255, 255, 0, 0] });
            }
        }
    }
}

/// How a cape hangs: how far it's lagging behind the player decides how far it swings out.
pub struct CapeState {
    /// Where the cape's catching up from, relative to the player.
    pub lag: DVec3,
    pub body_yaw: f32,
    /// Bounce from walking, and how far's been walked.
    pub bob: f32,
    pub walked: f32,
    pub gliding: bool,
}

/// A player's cape, if they've got one and aren't wearing wings.
pub fn draw_cape(ctx: &mut Ctx, uuid: uuid::Uuid, body: &Rig, m: Mat4, s: &St, state: &CapeState, wearing_chest_armor: bool) {
    let Some(sprite) = ctx.skins.cape_of(uuid, &ctx.bodies.sheet) else { return };
    let Some(skel) = ctx.bodies.skeleton("player#cape") else { return };
    let yaw = state.body_yaw.to_radians() as f64;
    let (sin, cos) = (yaw.sin(), -yaw.cos());
    let mut flap = ((state.lag.y * 10.0) as f32).clamp(-6.0, 32.0);
    let lean = ((state.lag.x * sin + state.lag.z * cos) as f32 * 100.0 * if state.gliding { 0.0 } else { 1.0 }).clamp(0.0, 150.0);
    let lean2 = ((state.lag.x * cos - state.lag.z * sin) as f32 * 100.0).clamp(-20.0, 20.0);
    flap += (state.walked * 6.0).sin() * 32.0 * state.bob;
    let mut rig = Rig::new(skel);
    rig.copy_from(body);
    let turn = glam::Quat::from_rotation_y(-std::f32::consts::PI)
        * glam::Quat::from_rotation_x((6.0 + lean / 2.0 + flap).to_radians())
        * glam::Quat::from_rotation_z((lean2 / 2.0).to_radians())
        * glam::Quat::from_rotation_y((180.0 - lean2 / 2.0).to_radians());
    rig.p("cape").extra = Some(turn);
    let at = if wearing_chest_armor { m * Mat4::from_translation(Vec3::new(0.0, -0.053125, 0.06875)) } else { m };
    super::draw_layer(ctx, &rig, at, sprite, s, WHITE, super::Pass::Cutout, true);
}
