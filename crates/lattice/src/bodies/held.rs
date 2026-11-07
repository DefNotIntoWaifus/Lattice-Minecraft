//! Items and blocks out in the world: in hands, on heads, on the ground, in frames.

use super::mobs::WHITE;
use super::{Ctx, St, blockent};
use crate::game::items;
use crate::ui::items::{StackQuery, model_name};
use glam::{Mat4, Vec3};
use lattice_proto::ItemStack;
use lattice_render::items::{ItemPiece, ItemShape};
use lattice_render::mesher::Vertex;
use lattice_render::parts::{Look, Rig, entity_shade};
use std::f32::consts::PI;

/// Writes an item's shape as quads. `m` carries block space to camera space.
pub fn draw_shape(ctx: &mut Ctx, shape: &ItemShape, tints: &[[u8; 3]], m: Mat4, light: [u8; 2]) {
    let light4 = [light[0], light[1], 0, 0];
    if ctx.shining && ctx.shine.strength > 0 {
        let shine = ctx.shine;
        let color = [255, 255, 255, shine.strength];
        let hand = ctx.to_hand;
        let list = if hand { &mut ctx.out.hand_glint } else { &mut ctx.out.glint_item };
        for q in &shape.quads {
            for k in 0..4 {
                list.push(Vertex { pos: m.transform_point3(q.pos[k]).to_array(), uv: shine.item_uv(q.uv[k]), color, light: light4 });
            }
        }
    }
    for q in &shape.quads {
        let n = m.transform_vector3(q.normal).normalize_or_zero();
        let shade = entity_shade(n);
        let tint = if q.tint >= 0 { tints.get(q.tint as usize).copied().unwrap_or([255, 255, 255]) } else { [255, 255, 255] };
        let c = |v: u8| (v as f32 * shade) as u8;
        let color = [c(tint[0]), c(tint[1]), c(tint[2]), 255];
        let list = ctx.block_list(q.translucent);
        for k in 0..4 {
            list.push(Vertex { pos: m.transform_point3(q.pos[k]).to_array(), uv: q.uv[k], color, light: light4 });
        }
    }
}

/// Draws a stack the way a display context shows it, with `m` at the item's anchor.
pub fn draw_stack(ctx: &mut Ctx, stack: &ItemStack, context: &'static str, m: Mat4, light: [u8; 2], left: bool) {
    let q = StackQuery { stack, day_fraction: ctx.day_fraction, using_ticks: None, context };
    let name = model_name(stack);
    let Some(visuals) = ctx.items else { return };
    ctx.shining = items::has_glint(stack);
    for piece in visuals.pieces(&name, &q) {
        match piece {
            ItemPiece::Shape { shape, tints } => {
                let at = m * shape.transform(context).matrix(left) * Mat4::from_translation(Vec3::splat(-0.5));
                draw_shape(ctx, shape, &tints, at, light);
            }
            ItemPiece::Special { base, kind, texture } => {
                let at = m * base.transform(context).matrix(left) * Mat4::from_translation(Vec3::splat(-0.5));
                blockent::draw_special(ctx, stack, kind, texture, at, light);
            }
        }
    }
    ctx.shining = false;
}

/// How low and how high a stack's model reaches once placed.
pub fn height_of(ctx: &Ctx, stack: &ItemStack, context: &'static str) -> (f32, f32) {
    let q = StackQuery { stack, day_fraction: 0.0, using_ticks: None, context };
    let name = model_name(stack);
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    let Some(visuals) = ctx.items else { return (0.0, 0.0) };
    for piece in visuals.pieces(&name, &q) {
        let (shape, flat_box) = match &piece {
            ItemPiece::Shape { shape, .. } => (*shape, false),
            ItemPiece::Special { base, .. } => (*base, true),
        };
        let (min, max) = if flat_box { (Vec3::ZERO, Vec3::ONE) } else { (shape.min, shape.max) };
        let t = shape.transform(context).matrix(false) * Mat4::from_translation(Vec3::splat(-0.5));
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    let p = t.transform_point3(Vec3::new(x, y, z));
                    low = low.min(p.y);
                    high = high.max(p.y);
                }
            }
        }
    }
    if low > high { (0.0, 0.0) } else { (low, high) }
}

/// How low and how thick a stack's model is once placed, for standing it on the ground.
pub fn placed_box(ctx: &Ctx, stack: &ItemStack, context: &'static str) -> (f32, f32) {
    let q = StackQuery { stack, day_fraction: 0.0, using_ticks: None, context };
    let name = model_name(stack);
    let (mut lo_y, mut lo_z, mut hi_z) = (f32::MAX, f32::MAX, f32::MIN);
    let Some(visuals) = ctx.items else { return (0.0, 0.0) };
    for piece in visuals.pieces(&name, &q) {
        let (shape, flat_box) = match &piece {
            ItemPiece::Shape { shape, .. } => (*shape, false),
            ItemPiece::Special { base, .. } => (*base, true),
        };
        let (min, max) = if flat_box { (Vec3::ZERO, Vec3::ONE) } else { (shape.min, shape.max) };
        let t = shape.transform(context).matrix(false) * Mat4::from_translation(Vec3::splat(-0.5));
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    let p = t.transform_point3(Vec3::new(x, y, z));
                    lo_y = lo_y.min(p.y);
                    lo_z = lo_z.min(p.z);
                    hi_z = hi_z.max(p.z);
                }
            }
        }
    }
    if lo_y == f32::MAX {
        return (0.0, 0.0);
    }
    (lo_y, hi_z - lo_z)
}

/// A dropped stack: bobbing, spinning, a few copies for a bigger stack.
pub fn draw_ground_item(ctx: &mut Ctx, stack: &ItemStack, pos: Vec3, id: i32, age: f32, light: [u8; 2]) {
    let bob_offset = (id as u32).wrapping_mul(2654435761) as f32 / u32::MAX as f32 * PI * 2.0;
    let (lo_y, thick) = placed_box(ctx, stack, "ground");
    let up = -lo_y + 0.0625;
    let bob = (age / 10.0 + bob_offset).sin() * 0.1 + 0.1;
    let spin = age / 20.0 + bob_offset;
    let m = Mat4::from_translation(pos + Vec3::new(0.0, bob + up, 0.0)) * Mat4::from_rotation_y(spin);
    let copies = match stack.count {
        c if c <= 1 => 1,
        c if c <= 16 => 2,
        c if c <= 32 => 3,
        c if c <= 48 => 4,
        _ => 5,
    };
    let mut seed = (stack.item as u32).wrapping_mul(31).wrapping_add(stack.count as u32).wrapping_add(187);
    let mut rand = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    if thick > 0.0625 {
        draw_stack(ctx, stack, "ground", m, light, false);
        for _ in 1..copies {
            let t = Vec3::new(rand(), rand(), rand()) * 0.15;
            draw_stack(ctx, stack, "ground", m * Mat4::from_translation(t), light, false);
        }
    } else {
        let step = thick * 1.5;
        let mut at = m * Mat4::from_translation(Vec3::new(0.0, 0.0, -(step * (copies - 1) as f32 / 2.0)));
        draw_stack(ctx, stack, "ground", at, light, false);
        at *= Mat4::from_translation(Vec3::new(0.0, 0.0, step));
        for _ in 1..copies {
            let t = Vec3::new(rand() * 0.075, rand() * 0.075, 0.0);
            draw_stack(ctx, stack, "ground", at * Mat4::from_translation(t), light, false);
            at *= Mat4::from_translation(Vec3::new(0.0, 0.0, step));
        }
    }
}

/// An item held in a body's hand.
pub fn draw_in_hand(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, stack: &ItemStack, left: bool) {
    let arm = if left { "left_arm" } else { "right_arm" };
    if !body.has(arm) {
        return;
    }
    let mut at = m * body.matrix(arm);
    if s.slim {
        at *= Mat4::from_translation(Vec3::new(if left { -0.5 } else { 0.5 } / 16.0, 0.0, 0.0));
    }
    let (x, y, z) = if s.baby { (0.0, 1.0, -4.5) } else { (1.0, 2.0, -10.0) };
    let side = if left { -1.0 } else { 1.0 };
    at = at
        * Mat4::from_rotation_x(-PI / 2.0)
        * Mat4::from_rotation_y(PI)
        * Mat4::from_translation(Vec3::new(side * x / 16.0, y / 16.0, z / 16.0));
    draw_stack(ctx, stack, if left { "thirdperson_lefthand" } else { "thirdperson_righthand" }, at, s.light, left);
}

/// What villagers carry in their folded arms.
pub fn draw_villager_item(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, stack: &ItemStack) {
    let at = m
        * body.matrix("arms")
        * Mat4::from_rotation_x(0.75)
        * Mat4::from_scale(Vec3::splat(1.07))
        * Mat4::from_translation(Vec3::new(0.0, 0.13, -0.34))
        * Mat4::from_rotation_x(PI);
    draw_stack(ctx, stack, "ground", at, s.light, false);
}

pub fn draw_witch_potion(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, stack: &ItemStack) {
    let at = m
        * body.matrix("nose")
        * Mat4::from_translation(Vec3::new(0.0625, 0.25, 0.0))
        * Mat4::from_rotation_z(PI)
        * Mat4::from_rotation_x(140f32.to_radians())
        * Mat4::from_rotation_z(10f32.to_radians())
        * Mat4::from_rotation_x(PI);
    draw_villager_item_at(ctx, at, s, stack);
}

fn draw_villager_item_at(ctx: &mut Ctx, at: Mat4, s: &St, stack: &ItemStack) {
    let at = at * Mat4::from_rotation_x(0.75) * Mat4::from_scale(Vec3::splat(1.07)) * Mat4::from_translation(Vec3::new(0.0, 0.13, -0.34)) * Mat4::from_rotation_x(PI);
    draw_stack(ctx, stack, "ground", at, s.light, false);
}

/// What foxes and dolphins carry in their mouths.
pub fn draw_in_mouth(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, stack: &ItemStack) {
    if body.has("nose") && !body.has("tail_fin") {
        // A fox: from its head, along where it's looking.
        let head = body.get("head");
        let at = m * Mat4::from_translation(head.pos / 16.0) * if s.baby { Mat4::from_scale(Vec3::splat(0.75)) } else { Mat4::IDENTITY };
        let at = at
            * Mat4::from_rotation_y(s.yaw.to_radians())
            * Mat4::from_rotation_x(s.pitch.to_radians())
            * Mat4::from_translation(if s.baby { Vec3::new(0.06, 0.26, -0.5) } else { Vec3::new(0.06, 0.27, -0.5) })
            * Mat4::from_rotation_x(PI / 2.0);
        draw_stack(ctx, stack, "ground", at, s.light, false);
    } else {
        let v = s.pitch.abs() / 60.0;
        let t = if s.pitch < 0.0 { Vec3::new(0.0, 1.0 - v * 0.5, -1.0 + v * 0.5) } else { Vec3::new(0.0, 1.0 + v * 0.8, -1.0 + v * 0.2) };
        draw_stack(ctx, stack, "ground", m * Mat4::from_translation(t), s.light, false);
    }
}

/// A block model, in block space, using its own default look.
pub fn draw_block(ctx: &mut Ctx, state: u32, m: Mat4, light: [u8; 2], seed: i64, white: Option<f32>) {
    let models = ctx.blocks;
    let light4 = [light[0], light[1], 0, white.map_or(0, |w| (w * 255.0) as u8)];
    for model in models.pick(state, seed) {
        for q in &model.quads {
            let n = m.transform_vector3((q.pos[1] - q.pos[0]).cross(q.pos[3] - q.pos[0])).normalize_or_zero();
            let shade = entity_shade(n);
            let tint = if q.tint >= 0 { [0x7C, 0xBD, 0x6B] } else { [255, 255, 255] };
            let c = |v: u8| (v as f32 * shade) as u8;
            let color = [c(tint[0]), c(tint[1]), c(tint[2]), 255];
            let list = ctx.block_list(q.layer == lattice_render::models::Layer::Translucent);
            for k in 0..4 {
                list.push(Vertex { pos: m.transform_point3(q.pos[k]).to_array(), uv: q.uv[k], color, light: light4 });
            }
        }
    }
}

fn default_state(block: &str) -> Option<u32> {
    let b = lattice_data::game().blocks.iter().find(|b| b.name == block)?;
    Some(b.default)
}

pub fn draw_carried_block(ctx: &mut Ctx, m: Mat4, state: u32, light: [u8; 2]) {
    let at = m
        * Mat4::from_translation(Vec3::new(0.0, 0.6875, -0.75))
        * Mat4::from_rotation_x(20f32.to_radians())
        * Mat4::from_rotation_y(45f32.to_radians())
        * Mat4::from_translation(Vec3::new(0.25, 0.1875, 0.25))
        * Mat4::from_scale(Vec3::new(-0.5, -0.5, 0.5))
        * Mat4::from_rotation_y(PI / 2.0);
    draw_block(ctx, state, at, light, 0, None);
}

pub fn draw_mushrooms(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, kind: &str) {
    let Some(state) = default_state(if kind == "brown" { "brown_mushroom" } else { "red_mushroom" }) else { return };
    let center = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0)) * Mat4::from_translation(Vec3::splat(-0.5));
    let back = Mat4::from_translation(Vec3::new(0.2, -0.35, 0.5));
    draw_block(ctx, state, m * back * Mat4::from_rotation_y((-48f32).to_radians()) * center, s.light, 0, None);
    draw_block(
        ctx,
        state,
        m * back * Mat4::from_rotation_y(42f32.to_radians()) * Mat4::from_translation(Vec3::new(0.1, 0.0, -0.6)) * Mat4::from_rotation_y((-48f32).to_radians()) * center,
        s.light,
        0,
        None,
    );
    draw_block(ctx, state, m * body.matrix("head") * Mat4::from_translation(Vec3::new(0.0, -0.7, -0.2)) * Mat4::from_rotation_y((-78f32).to_radians()) * center, s.light, 0, None);
}

/// A block worn on a part: the snow golem's pumpkin.
pub fn draw_block_on_part(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, block: &str, part: &str) {
    let Some(state) = default_state(block) else { return };
    let at = m
        * body.matrix(part)
        * Mat4::from_translation(Vec3::new(0.0, -0.34375, 0.0))
        * Mat4::from_rotation_y(PI)
        * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
        * Mat4::from_translation(Vec3::splat(-0.5));
    draw_block(ctx, state, at, s.light, 0, None);
}

/// Something worn on the head that isn't armour: a pumpkin, a banner, a skull.
pub fn draw_on_head(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, stack: &ItemStack) {
    if !body.has("head") {
        return;
    }
    let name = items::item_name(stack);
    let head = m * body.matrix("head");
    if let Some((model, texture)) = blockent::skull_of(name) {
        let at = head * Mat4::from_scale(Vec3::splat(1.1875));
        blockent::draw_skull_at(ctx, model, texture, at, s.light, 0.0);
        return;
    }
    let at = head * Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0)) * Mat4::from_rotation_y(PI) * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
    draw_stack(ctx, stack, "head", at, s.light, false);
}

// ---------------------------------------------------------------- first person

/// What the camera sees of the player's own arms and what they hold.
pub struct HandState<'a> {
    pub main: Option<&'a ItemStack>,
    pub off: Option<&'a ItemStack>,
    /// 0 fully raised, 1 fully lowered.
    pub main_equip: f32,
    pub off_equip: f32,
    /// Swing progress of each hand, 0..1.
    pub main_swing: f32,
    pub off_swing: f32,
    /// Which hand is being used, and for how long (ticks with fraction) out of how many.
    pub using: Option<(bool, f32, f32)>,
    pub left_handed: bool,
    pub light: [u8; 2],
    pub skin: lattice_render::atlas::Sprite,
    pub slim: bool,
    pub sleeves: bool,
    pub invisible: bool,
    pub scoping: bool,
    /// How far the player's looking up or down, which tilts a map held in both hands.
    pub pitch: f32,
}

fn rot_y(d: f32) -> Mat4 {
    Mat4::from_rotation_y(d.to_radians())
}
fn rot_x(d: f32) -> Mat4 {
    Mat4::from_rotation_x(d.to_radians())
}
fn rot_z(d: f32) -> Mat4 {
    Mat4::from_rotation_z(d.to_radians())
}

fn arm_transform(side: f32, equip: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(side * 0.56, -0.52 + equip * -0.6, -0.72))
}

fn attack_transform(side: f32, swing: f32) -> Mat4 {
    let a = (swing * swing * PI).sin();
    let b = (swing.sqrt() * PI).sin();
    rot_y(side * (45.0 + a * -20.0)) * rot_z(side * b * -20.0) * rot_x(b * -80.0) * rot_y(side * -45.0)
}

fn swing_transform(side: f32, swing: f32) -> Mat4 {
    let x = -0.4 * (swing.sqrt() * PI).sin();
    let y = 0.2 * (swing.sqrt() * std::f32::consts::TAU).sin();
    let z = -0.2 * (swing * PI).sin();
    Mat4::from_translation(Vec3::new(side * x, y, z)) * attack_transform(side, swing)
}

/// The bare arm.
fn draw_arm(ctx: &mut Ctx, h: &HandState, equip: f32, swing: f32, right: bool) {
    draw_arm_from(ctx, h, Mat4::IDENTITY, equip, swing, right);
}

fn draw_arm_from(ctx: &mut Ctx, h: &HandState, base: Mat4, equip: f32, swing: f32, right: bool) {
    let side = if right { 1.0 } else { -1.0 };
    let g = swing.sqrt();
    let x = -0.3 * (g * PI).sin();
    let y = 0.4 * (g * std::f32::consts::TAU).sin();
    let z = -0.4 * (swing * PI).sin();
    let m = base * Mat4::from_translation(Vec3::new(side * (x + 0.64000005), y - 0.6 + equip * -0.6, z - 0.71999997))
        * rot_y(side * 45.0)
        * rot_y(side * (g * PI).sin() * 70.0)
        * rot_z(side * (swing * swing * PI).sin() * -20.0)
        * Mat4::from_translation(Vec3::new(side * -1.0, 3.6, 3.5))
        * rot_z(side * 120.0)
        * rot_x(200.0)
        * rot_y(side * -135.0)
        * Mat4::from_translation(Vec3::new(side * 5.6, 0.0, 0.0));
    draw_hand_part(ctx, h, m, right);
}

fn draw_hand_part(ctx: &mut Ctx, h: &HandState, m: Mat4, right: bool) {
    let Some(skel) = ctx.bodies.skeleton(if h.slim { "player_slim" } else { "player" }) else { return };
    let mut rig = Rig::new(skel);
    let (part, sleeve) = if right { ("right_arm", "right_sleeve") } else { ("left_arm", "left_sleeve") };
    rig.p(part).rot.z = if right { 0.1 } else { -0.1 };
    rig.p(sleeve).visible = h.sleeves;
    let look = Look { sprite: h.skin, color: WHITE, light: h.light, overlay: [0, 0], scroll: [0.0, 0.0], lit: true };
    let out = ctx.list(super::Pass::Cutout);
    lattice_render::parts::draw_part(&rig, m, &look, out, part);
}

/// One hand holding up a map, the arm below it.
fn draw_map_one_handed(ctx: &mut Ctx, h: &HandState, stack: &ItemStack, right: bool, equip: f32, swing: f32) {
    let side = if right { 1.0 } else { -1.0 };
    let base = Mat4::from_translation(Vec3::new(side * 0.125, -0.125, 0.0));
    if !h.invisible {
        draw_arm_from(ctx, h, base * rot_z(side * 10.0), equip, swing, right);
    }
    let g = swing.sqrt();
    let s = (g * PI).sin();
    let (x, y, z) = (-0.5 * s, 0.4 * (g * std::f32::consts::TAU).sin(), -0.3 * (swing * PI).sin());
    let m = base
        * Mat4::from_translation(Vec3::new(side * 0.51, -0.08 + equip * -1.2, -0.75))
        * Mat4::from_translation(Vec3::new(side * x, y - 0.3 * s, z))
        * rot_x(s * -45.0)
        * rot_y(side * s * -30.0);
    draw_map_held(ctx, m, stack, h.light);
}

/// Both hands holding up a map, tilting down out of the way as the player looks up.
fn draw_map_two_handed(ctx: &mut Ctx, h: &HandState, stack: &ItemStack, equip: f32, swing: f32) {
    let g = swing.sqrt();
    let lift = -0.2 * (swing * PI).sin();
    let back = -0.4 * (g * PI).sin();
    // How far the map tips, from how far up the player's looking.
    let t = (1.0 - h.pitch / 45.0 + 0.1).clamp(0.0, 1.0);
    let tilt = -(t * PI).cos() * 0.5 + 0.5;
    let m = Mat4::from_translation(Vec3::new(0.0, -lift / 2.0, back))
        * Mat4::from_translation(Vec3::new(0.0, 0.04 + equip * -1.2 + tilt * -0.5, -0.72))
        * rot_x(tilt * -85.0);
    if !h.invisible {
        for right in [true, false] {
            let side = if right { 1.0 } else { -1.0 };
            let hand = m * rot_y(90.0) * rot_y(92.0) * rot_x(45.0) * rot_z(side * -41.0) * Mat4::from_translation(Vec3::new(side * 0.3, -1.1, 0.45));
            draw_hand_part(ctx, h, hand, right);
        }
    }
    let held = m * rot_x((g * PI).sin() * 20.0) * Mat4::from_scale(Vec3::splat(2.0));
    draw_map_held(ctx, held, stack, h.light);
}

fn draw_map_held(ctx: &mut Ctx, m: Mat4, stack: &ItemStack, light: [u8; 2]) {
    let m = m * rot_y(180.0) * rot_z(180.0) * Mat4::from_scale(Vec3::splat(0.38)) * Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.0)) * Mat4::from_scale(Vec3::splat(1.0 / 128.0));
    let id = stack.var_int("map_id");
    let known = id.is_some_and(|i| ctx.maps.contains_key(&i));
    let back = if known { "map/map_background_checkerboard" } else { "map/map_background" };
    let s = ctx.bodies.sheet.sprite(back);
    super::maps::quad(ctx, m, [-7.0, -7.0], [135.0, 135.0], 0.0, s, light);
    if let Some(id) = id {
        super::maps::draw(ctx, m, id, false, light);
    }
}

/// How an item's use changes what the hand does, by its use animation.
fn use_transform(stack: &ItemStack, side: f32, equip: f32, ticks: f32, duration: f32) -> Option<Mat4> {
    let name = items::item_name(stack);
    let kind = items::use_animation(stack);
    match kind.as_deref() {
        Some("eat" | "drink") => {
            let used = ticks;
            let frac = used / duration.max(1.0);
            let mut m = Mat4::IDENTITY;
            if frac < 0.8 {
                m *= Mat4::from_translation(Vec3::new(0.0, ((used / 4.0 * PI).cos() * 0.1).abs(), 0.0));
            }
            let v = 1.0 - frac.powi(27);
            m *= Mat4::from_translation(Vec3::new(v * 0.6 * side, v * -0.5, 0.0));
            m *= rot_y(side * v * 90.0) * rot_x(v * 10.0) * rot_z(side * v * 30.0);
            Some(m * arm_transform(side, equip))
        }
        Some("block") if name != "shield" => Some(arm_transform(side, equip) * Mat4::from_translation(Vec3::new(side * -0.14142136, 0.08, 0.14142136)) * rot_x(-102.25) * rot_y(side * 13.365) * rot_z(side * 78.05)),
        Some("block") => Some(arm_transform(side, equip)),
        Some("bow") => {
            let t = ticks / 20.0;
            let pull = ((t * t + t * 2.0) / 3.0).min(1.0);
            let mut m = arm_transform(side, equip) * Mat4::from_translation(Vec3::new(side * -0.2785682, 0.18344387, 0.15731531)) * rot_x(-13.935) * rot_y(side * 35.3) * rot_z(side * -9.785);
            if pull > 0.1 {
                let shake = ((ticks - 0.1) * 1.3).sin() * (pull - 0.1);
                m *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
            }
            m *= Mat4::from_translation(Vec3::new(0.0, 0.0, pull * 0.04));
            m *= Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + pull * 0.2));
            Some(m * Mat4::from_rotation_y((side * -45.0f32).to_radians()))
        }
        Some("trident" | "spear") => {
            let pull = (ticks / 10.0).min(1.0);
            let mut m = arm_transform(side, equip) * Mat4::from_translation(Vec3::new(side * -0.5, 0.7, 0.1)) * rot_x(-55.0) * rot_y(side * 35.3) * rot_z(side * -9.785);
            if pull > 0.1 {
                let shake = ((ticks - 0.1) * 1.3).sin() * (pull - 0.1);
                m *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
            }
            m *= Mat4::from_translation(Vec3::new(0.0, 0.0, pull * 0.2));
            m *= Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + pull * 0.2));
            Some(m * Mat4::from_rotation_y((side * -45.0f32).to_radians()))
        }
        _ => None,
    }
}

fn draw_hand_item(ctx: &mut Ctx, h: &HandState, stack: &ItemStack, right: bool, main: bool, equip: f32, swing: f32) {
    let side = if right { 1.0 } else { -1.0 };
    let name = items::item_name(stack);
    if name == "filled_map" {
        if main && h.off.is_none() {
            draw_map_two_handed(ctx, h, stack, equip, swing);
        } else {
            draw_map_one_handed(ctx, h, stack, right, equip, swing);
        }
        return;
    }
    let use_here = h.using.filter(|(in_main, _, _)| *in_main == main);
    let mut m;
    if name == "crossbow" {
        m = arm_transform(side, equip);
        let charged = stack.raw("charged_projectiles").is_some_and(|r| r.first() != Some(&0));
        if let (Some((_, ticks, _)), false) = (use_here, charged) {
            m *= Mat4::from_translation(Vec3::new(side * -0.4785682, -0.094387, 0.05731531)) * rot_x(-11.935) * rot_y(side * 65.3) * rot_z(side * -9.785);
            let pull = (ticks / 25.0).min(1.0);
            if pull > 0.1 {
                let shake = ((ticks - 0.1) * 1.3).sin() * (pull - 0.1);
                m *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
            }
            m *= Mat4::from_translation(Vec3::new(0.0, 0.0, pull * 0.04));
            m *= Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + pull * 0.2));
            m *= Mat4::from_rotation_y((side * -45.0f32).to_radians());
        } else {
            m *= swing_transform(side, swing);
            if charged && swing < 0.001 && main {
                m *= Mat4::from_translation(Vec3::new(side * -0.641864, 0.0, 0.0)) * rot_y(side * 10.0);
            }
        }
    } else if let Some((_, ticks, duration)) = use_here {
        m = use_transform(stack, side, equip, ticks, duration).unwrap_or_else(|| arm_transform(side, equip));
    } else {
        m = arm_transform(side, equip) * swing_transform(side, swing);
    }
    let context = if right { "firstperson_righthand" } else { "firstperson_lefthand" };
    draw_stack(ctx, stack, context, m, h.light, !right);
}

pub fn draw_first_person(ctx: &mut Ctx, h: &HandState) {
    if h.scoping {
        return;
    }
    ctx.to_hand = true;
    let main_right = !h.left_handed;
    match h.main {
        Some(stack) => draw_hand_item(ctx, h, stack, main_right, true, h.main_equip, h.main_swing),
        None if !h.invisible => draw_arm(ctx, h, h.main_equip, h.main_swing, main_right),
        None => {}
    }
    if let Some(stack) = h.off {
        draw_hand_item(ctx, h, stack, !main_right, false, h.off_equip, h.off_swing);
    }
    ctx.to_hand = false;
}
