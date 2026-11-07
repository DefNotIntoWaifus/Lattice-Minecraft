//! Which model, textures and layers each living thing gets.

use super::anim::{self, ArmPose, IllagerPose};
use super::{Ctx, Pass, St, draw_layer, held, living_matrix, model_space, rot_lerp, wrap_degrees};
use crate::game::entities::Entity;
use crate::game::items;
use glam::{Mat4, Vec3};
use lattice_proto::{DataValue, ItemStack};
use lattice_render::parts::{Look, Rig};
use std::f32::consts::{FRAC_PI_2, PI};
use uuid::Uuid;

pub const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Equipment slots as the entity packets number them.
pub const MAIN_HAND: usize = 0;
pub const OFF_HAND: usize = 1;
pub const FEET: usize = 2;
pub const LEGS: usize = 3;
pub const CHEST: usize = 4;
pub const HEAD: usize = 5;
pub const BODY: usize = 6;
pub const SADDLE: usize = 7;

/// Colours dyes give things, by dye number.
pub const DYE_COLORS: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97, 0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

pub fn rgb(c: u32) -> [u8; 4] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
}

pub fn state_of(e: &Entity, id: i32, partial: f32, light: [u8; 2]) -> St {
    let alive = !e.is_dead();
    let baby = e.is_baby();
    let head = rot_lerp(partial, e.prev_head_yaw, e.head_yaw);
    let body = rot_lerp(partial, e.prev_body_yaw, e.body_yaw);
    let mob_flags = e.int_field("MOB_FLAGS").unwrap_or(0);
    let pitch = e.prev_pitch + (e.pitch - e.prev_pitch) * partial;
    let riding = e.vehicle.is_some();
    let (wp, ws) = if alive && !riding {
        let scale = if baby { 3.0 } else { 1.0 };
        ((e.walk_position - e.walk_speed * (1.0 - partial)) * scale, (e.prev_walk_speed + (e.walk_speed - e.prev_walk_speed) * partial).min(1.0))
    } else {
        (0.0, 0.0)
    };
    St {
        id,
        age: e.age as f32 + partial,
        wp,
        ws,
        yaw: wrap_degrees(head - body),
        pitch,
        body,
        baby,
        age_scale: if baby { 0.5 } else { 1.0 },
        swing: e.swing.progress(partial),
        swing_off_hand: e.swing.off_hand(),
        death: if e.death_ticks > 0 { e.death_ticks as f32 + partial } else { 0.0 },
        hurt: e.hurt_ticks < 10 || e.death_ticks > 0,
        in_water: e.in_water,
        riding,
        crouching: e.pose() == 5 || (e.flag(1) && e.pose() == 0 && e.type_name() == "player"),
        swimming: if e.flag(4) { 1.0 } else { 0.0 },
        gliding: e.flag(7),
        light,
        invisible: e.flag(5),
        left_handed: mob_flags & 2 != 0 || e.data.get(&15).is_some_and(|v| matches!(v, DataValue::Enum(0))) && matches!(e.type_name(), "player" | "mannequin"),
        aggressive: mob_flags & 4 != 0,
        sleeping: e.pose() == 2,
        moving: (e.pos - e.prev_pos).length_squared() > 1e-5 || e.velocity.length_squared() > 1e-4,
        slim: false,
    }
}

/// What an item in hand does to the arm.
fn arm_pose_for(stack: Option<&ItemStack>, using: bool, use_ticks: u32) -> ArmPose {
    let Some(stack) = stack else { return ArmPose::Empty };
    let name = items::item_name(stack);
    if using {
        return match name {
            "bow" => ArmPose::Bow,
            "crossbow" => ArmPose::CrossbowCharge(use_ticks),
            "shield" => ArmPose::Block,
            "spyglass" => ArmPose::Spyglass,
            "trident" => ArmPose::Throw,
            "goat_horn" => ArmPose::Horn,
            "brush" => ArmPose::Brush,
            _ => ArmPose::Item,
        };
    }
    if name == "crossbow" && stack.raw("charged_projectiles").is_some_and(|r| r.first() != Some(&0)) {
        return ArmPose::Crossbow;
    }
    ArmPose::Item
}

/// Arm poses of a person-shaped thing from what it holds and is doing.
fn arm_poses(e: &Entity, s: &St) -> (ArmPose, ArmPose) {
    let flags = e.int_field("LIVING_ENTITY_FLAGS").unwrap_or(0);
    let using = flags & 1 != 0;
    let off_hand_used = flags & 2 != 0;
    let main = e.equipment[MAIN_HAND].as_ref();
    let off = e.equipment[OFF_HAND].as_ref();
    let main_pose = arm_pose_for(main, using && !off_hand_used, e.use_ticks);
    let off_pose = arm_pose_for(off, using && off_hand_used, e.use_ticks);
    if s.left_handed { (off_pose, main_pose) } else { (main_pose, off_pose) }
}

fn holding(e: &Entity, name: &str) -> bool {
    [MAIN_HAND, OFF_HAND].iter().any(|s| e.equipment[*s].as_ref().is_some_and(|i| items::item_name(i) == name))
}

/// Draws a worn piece on top of a body, posed the same.
fn armor_layer(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, model: &str, stack: &ItemStack, kind: &str) {
    let Some(asset) = items::equipment_asset(stack) else { return };
    let Some(eq) = ctx.bodies.equipment(&asset) else { return };
    let Some(layers) = eq.layers.get(kind).cloned() else { return };
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    rig.copy_from(body);
    for l in &layers {
        let Some(sprite) = ctx.sprite(&format!("entity/equipment/{kind}/{}", l.texture)) else { continue };
        let color = if l.dyeable { stack.dyed_color().map(|c| rgb(c as u32)).or(l.undyed.map(rgb)).unwrap_or(WHITE) } else { WHITE };
        draw_layer(ctx, &rig, m, sprite, s, color, Pass::Cutout, true);
    }
    if let Some((pattern, material)) = super::trims::trim_of(stack, ctx.registries) {
        let dir = if kind.ends_with("leggings") { "humanoid_leggings" } else { "humanoid" };
        // A trim of the armour's own metal comes out darker, so it still shows.
        let darker = format!("trims/entity/{dir}/{pattern}_{material}_darker");
        let plain = format!("trims/entity/{dir}/{pattern}_{material}");
        let own_metal = asset == material || (asset == "gold" && material == "gold");
        let sprite = if own_metal { ctx.sprite(&darker).or_else(|| ctx.sprite(&plain)) } else { ctx.sprite(&plain) };
        if let Some(sprite) = sprite {
            draw_layer(ctx, &rig, m, sprite, s, WHITE, Pass::Cutout, true);
        }
    }
    if items::has_glint(stack) && ctx.shine.strength > 0 {
        let shine = ctx.shine;
        let look = Look {
            sprite: lattice_render::atlas::Sprite { u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0, alpha: 0 },
            color: [255, 255, 255, shine.strength],
            light: s.light,
            overlay: [0, 0],
            scroll: [0.0, 0.0],
            lit: false,
        };
        let start = ctx.out.glint_armor.len();
        lattice_render::parts::draw(&rig, m, &look, &mut ctx.out.glint_armor);
        for v in &mut ctx.out.glint_armor[start..] {
            v.uv = shine.armor_uv(v.uv);
        }
    }
}

/// The four armour pieces of anything person-shaped.
fn humanoid_armor(ctx: &mut Ctx, e: &Entity, body: &Rig, m: Mat4, s: &St, prefix: &str) {
    let kind = if s.baby { "humanoid_baby" } else { "humanoid" };
    for (slot, part, k) in [(FEET, "boots", kind), (LEGS, "leggings", if s.baby { "humanoid_baby" } else { "humanoid_leggings" }), (CHEST, "chestplate", kind), (HEAD, "helmet", kind)] {
        if let Some(stack) = &e.equipment[slot] {
            let wear = items::equip_slot(stack);
            let fits = match slot {
                FEET => wear.as_deref() == Some("feet"),
                LEGS => wear.as_deref() == Some("legs"),
                CHEST => wear.as_deref() == Some("chest"),
                _ => wear.as_deref() == Some("head"),
            };
            if fits {
                armor_layer(ctx, body, m, s, &format!("{prefix}#{part}"), stack, k);
            }
        }
    }
    // Elytra: two wings behind the shoulders.
    if let Some(stack) = &e.equipment[CHEST] {
        if items::glides(stack) {
            elytra(ctx, m, s, e);
        }
    }
    // Things on the head that aren't armour: pumpkins, skulls, banners.
    if let Some(stack) = &e.equipment[HEAD] {
        if items::equip_slot(stack).as_deref() != Some("head") || items::equipment_asset(stack).is_none() {
            held::draw_on_head(ctx, body, m, s, stack);
        }
    }
}

fn elytra(ctx: &mut Ctx, m: Mat4, s: &St, e: &Entity) {
    let Some(skel) = ctx.bodies.skeleton(if s.baby { "elytra_baby" } else { "elytra" }) else { return };
    // A player with a cape showing has wings in its colours.
    let cape_shown = e.type_name() == "player" && e.int_field("DATA_PLAYER_MODE_CUSTOMISATION").is_some_and(|parts| parts & 1 != 0);
    let cape = if cape_shown { ctx.skins.cape_of(e.uuid, &ctx.bodies.sheet) } else { None };
    let Some(sprite) = cape.or_else(|| ctx.sprite("entity/equipment/wings/elytra")) else { return };
    let mut rig = Rig::new(skel);
    // Spread while gliding, folded otherwise.
    let (mut x, mut z, mut y) = (0.2617994f32, -0.2617994f32, 0.0f32);
    if s.gliding {
        let v = e.velocity;
        let falling = if v.y < 0.0 { (1.0 - ((-v.y as f32).max(0.0)).powf(1.5).min(1.0)).max(0.0) } else { 1.0 };
        x = falling * 0.34906584 + (1.0 - falling) * FRAC_PI_2;
        z = falling * -FRAC_PI_2 + (1.0 - falling) * -FRAC_PI_2;
    } else if s.crouching {
        x = 0.6981317;
        z = -0.7853982;
        y = 0.08726646;
    }
    let lift = if s.crouching { 3.0 } else { 0.0 };
    let l = rig.p("left_wing");
    l.pos.y = lift;
    l.rot = Vec3::new(x, y, z);
    let r = rig.p("right_wing");
    r.pos.y = lift;
    r.rot = Vec3::new(x, -y, -z);
    draw_layer(ctx, &rig, m, sprite, s, WHITE, Pass::Cutout, true);
}

fn held_items(ctx: &mut Ctx, e: &Entity, body: &Rig, m: Mat4, s: &St) {
    let (main, off) = (e.equipment[MAIN_HAND].as_ref(), e.equipment[OFF_HAND].as_ref());
    let (right, left) = if s.left_handed { (off, main) } else { (main, off) };
    if let Some(stack) = right {
        held::draw_in_hand(ctx, body, m, s, stack, false);
    }
    if let Some(stack) = left {
        held::draw_in_hand(ctx, body, m, s, stack, true);
    }
}

/// A body whose whole look is one texture.
fn plain(ctx: &mut Ctx, rig: &Rig, m: Mat4, s: &St, texture: &str) {
    if s.invisible {
        return;
    }
    let sprite = ctx.sprite(texture).unwrap_or(ctx.bodies.sheet.missing);
    draw_layer(ctx, rig, m, sprite, s, WHITE, Pass::Cutout, true);
}

fn glow(ctx: &mut Ctx, rig: &Rig, m: Mat4, s: &St, texture: &str) {
    if s.invisible {
        return;
    }
    if let Some(sprite) = ctx.sprite(texture) {
        draw_layer(ctx, rig, m, sprite, s, WHITE, Pass::Glow, false);
    }
}

fn overlay(ctx: &mut Ctx, rig: &Rig, m: Mat4, s: &St, texture: &str, color: [u8; 4]) {
    if s.invisible {
        return;
    }
    if let Some(sprite) = ctx.sprite(texture) {
        draw_layer(ctx, rig, m, sprite, s, color, Pass::Cutout, true);
    }
}

/// A layer drawn with a second model that takes the body's pose.
fn second_model(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, model: &str, texture: &str, color: [u8; 4], pass: Pass) {
    if s.invisible {
        return;
    }
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let Some(sprite) = ctx.sprite(texture) else { return };
    let mut rig = Rig::new(skel);
    rig.copy_from(body);
    draw_layer(ctx, &rig, m, sprite, s, color, pass, pass == Pass::Cutout);
}

/// The energy over a charged creeper or a weakened wither: a second, slightly
/// bigger model whose repeating picture slides across it.
fn energy_layer(ctx: &mut Ctx, body: &Rig, m: Mat4, s: &St, model: &str, pass: Pass, sideways: f32) {
    if s.invisible {
        return;
    }
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    rig.copy_from(body);
    let look = Look {
        sprite: lattice_render::atlas::Sprite { u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0, alpha: 0 },
        color: [128, 128, 128, 255],
        light: [255, 255],
        overlay: [0, 0],
        scroll: [sideways % 1.0, (s.age * 0.01) % 1.0],
        lit: false,
    };
    lattice_render::parts::draw(&rig, m, &look, ctx.list(pass));
}

/// Saddles, carpets, harnesses and horse armour: worn things that come from equipment files.
fn worn(ctx: &mut Ctx, e: &Entity, body: &Rig, m: Mat4, s: &St, slot: usize, model: &str, kind: &str) {
    let Some(stack) = &e.equipment[slot] else { return };
    armor_layer(ctx, body, m, s, model, stack, kind);
}

fn register_hat(ctx: &Ctx, path: &str) -> Option<String> {
    ctx.bodies.hats.get(path).cloned()
}

fn villager_layers(ctx: &mut Ctx, rig: &Rig, m: Mat4, s: &St, folder: &str, data: Option<(i32, i32, i32)>, full_model: &str, no_hat_model: &str) {
    let Some((kind, profession, level)) = data else { return };
    let game = lattice_data::game();
    let kind_name = game.registry("villager_type").get(kind as usize).map(String::as_str).unwrap_or("plains");
    let prof = game.registry("villager_profession").get(profession as usize).map(String::as_str).unwrap_or("none");
    let hat_of_kind = register_hat(ctx, &format!("entity/{folder}/type/{kind_name}")).unwrap_or_else(|| "none".into());
    let hat_of_prof = register_hat(ctx, &format!("entity/{folder}/profession/{prof}")).unwrap_or_else(|| "none".into());
    let use_full = prof == "none" || hat_of_prof == "none" || (hat_of_prof == "partial" && hat_of_kind != "full");
    let model = if use_full { full_model } else { no_hat_model };
    let texture = if s.baby { format!("entity/{folder}/baby/{kind_name}") } else { format!("entity/{folder}/type/{kind_name}") };
    second_model(ctx, rig, m, s, model, &texture, WHITE, Pass::Cutout);
    if prof != "none" && !s.baby {
        overlay(ctx, rig, m, s, &format!("entity/{folder}/profession/{prof}"), WHITE);
        if prof != "nitwit" {
            let badge = ["stone", "iron", "gold", "emerald", "diamond"][(level - 1).clamp(0, 4) as usize];
            overlay(ctx, rig, m, s, &format!("entity/{folder}/profession_level/{badge}"), WHITE);
        }
    }
}

fn villager_data(e: &Entity) -> Option<(i32, i32, i32)> {
    match e.field("VILLAGER_DATA") {
        Some(DataValue::VillagerData { kind, profession, level }) => Some((*kind, *profession, *level)),
        _ => None,
    }
}

fn with_baby(path: &str, baby: bool) -> String {
    if baby { format!("{path}_baby") } else { path.to_string() }
}

/// Draws a person-shaped body with its armour and what it holds.
#[allow(clippy::too_many_arguments)]
fn person(ctx: &mut Ctx, e: &Entity, s: &St, pos: Vec3, adult: &str, baby: &str, texture: &str, setup: impl FnOnce(&mut Rig, &St)) -> Option<Mat4> {
    let model = if s.baby { baby } else { adult };
    let skel = ctx.bodies.skeleton(model)?;
    let mut rig = Rig::new(skel);
    setup(&mut rig, s);
    let m = model_space(living_matrix(pos, s, 1.0, 90.0), None);
    plain(ctx, &rig, m, s, texture);
    humanoid_armor(ctx, e, &rig, m, s, model);
    held_items(ctx, e, &rig, m, s);
    Some(m)
}

pub fn draw_mob(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, id: i32, e: &Entity, pos: Vec3, light: [u8; 2]) {
    let _ = world;
    let s = state_of(e, id, ctx.partial, light);
    let name = e.type_name();
    let (right, left) = arm_poses(e, &s);
    match name {
        "zombie" | "husk" | "drowned" => {
            let tex = with_baby(&format!("entity/zombie/{name}"), s.baby);
            let holding_item = e.equipment[MAIN_HAND].is_some();
            let adult = name.to_string();
            let baby = format!("{name}_baby");
            let trident = name == "drowned" && s.aggressive && holding(e, "trident");
            let swim = name == "drowned" && e.flag(4);
            let mut st = s;
            if swim {
                st.swimming = 1.0;
            }
            let (r, l) = if trident { (ArmPose::Throw, l_pose(left)) } else { (right, left) };
            let m = person(ctx, e, &st, pos, &adult, &baby, &tex, |rig, st| {
                anim::humanoid(rig, st, r, l, Some(holding_item));
            });
            if name == "drowned" {
                if let (Some(m), Some(skel)) = (m, ctx.bodies.skeleton(&format!("{}#outer", if s.baby { &baby } else { &adult }))) {
                    let mut rig = Rig::new(skel);
                    anim::humanoid(&mut rig, &st, r, l, Some(holding_item));
                    overlay(ctx, &rig, m, &st, &with_baby("entity/zombie/drowned_outer_layer", s.baby), WHITE);
                }
            }
        }
        "giant" => {
            person(ctx, e, &s, pos, "giant", "giant", "entity/zombie/zombie", |rig, st| anim::humanoid(rig, st, right, left, Some(false)));
        }
        "skeleton" | "wither_skeleton" | "stray" | "bogged" | "parched" => {
            let bow = holding(e, "bow");
            let (r, l) = if s.aggressive && bow { if s.left_handed { (ArmPose::Item, ArmPose::Bow) } else { (ArmPose::Bow, ArmPose::Item) } } else { (right, left) };
            let melee = s.aggressive && !bow;
            let tex = format!("entity/skeleton/{name}");
            let sheared = e.bool_field("SHEARED");
            let m = person(ctx, e, &s, pos, name, name, &tex, |rig, st| {
                anim::humanoid(rig, st, r, l, None);
                if melee {
                    anim::skeleton_melee(rig, st);
                }
                rig.p("mushrooms").visible = !sheared;
            });
            if matches!(name, "stray" | "bogged") {
                if let (Some(m), Some(skel)) = (m, ctx.bodies.skeleton(&format!("{name}#outer"))) {
                    let mut rig = Rig::new(skel);
                    anim::humanoid(&mut rig, &s, r, l, None);
                    if melee {
                        anim::skeleton_melee(&mut rig, &s);
                    }
                    overlay(ctx, &rig, m, &s, &format!("entity/skeleton/{name}_overlay"), WHITE);
                }
            }
        }
        "piglin" | "piglin_brute" | "zombified_piglin" => {
            let tex = with_baby(&format!("entity/piglin/{name}"), s.baby);
            let dancing = e.bool_field("IS_DANCING");
            let charging = e.bool_field("IS_CHARGING_CROSSBOW");
            let crossbow = holding(e, "crossbow");
            let aggressive = s.aggressive;
            let zombified = name == "zombified_piglin";
            let swinging = s.swing;
            let holding_item = e.equipment[MAIN_HAND].is_some();
            person(ctx, e, &s, pos, name, &format!("{name}_baby"), &tex, |rig, st| {
                anim::humanoid(rig, st, right, left, zombified.then_some(holding_item));
                anim::piglin_ears(rig, st, 30.0);
                if zombified {
                    return;
                }
                if dancing {
                    let t = st.age / 60.0;
                    let d = PI / 180.0;
                    rig.p("right_ear").rot.z = 0.5235988 + d * (t * 30.0).sin() * 10.0;
                    rig.p("left_ear").rot.z = -0.5235988 - d * (t * 30.0).cos() * 10.0;
                    rig.p("head").pos.x += (t * 10.0).sin();
                    rig.p("head").pos.y += (t * 40.0).sin() + 0.4;
                    let z = d * (70.0 + (t * 40.0).cos() * 10.0);
                    rig.p("right_arm").rot.z = z;
                    rig.p("left_arm").rot.z = -z;
                    rig.p("right_arm").pos.y += (t * 40.0).sin() * 0.5 - 0.5;
                    rig.p("left_arm").pos.y += (t * 40.0).sin() * 0.5 + 0.5;
                    rig.p("body").pos.y += (t * 40.0).sin() * 0.35;
                } else if charging {
                    anim::crossbow_charge(rig, 25.0, 12.0, !st.left_handed);
                } else if aggressive && crossbow {
                    anim::crossbow_hold(rig, !st.left_handed);
                } else if aggressive && holding_item && swinging == 0.0 {
                    rig.p(if st.left_handed { "left_arm" } else { "right_arm" }).rot.x = -1.8;
                }
            });
        }
        "enderman" => {
            let Some(skel) = ctx.bodies.skeleton("enderman") else { return };
            let mut rig = Rig::new(skel);
            let carrying = matches!(e.field("CARRY_STATE"), Some(DataValue::OptBlockState(Some(_))));
            let creepy = e.bool_field("CREEPY");
            anim::humanoid(&mut rig, &s, ArmPose::Empty, ArmPose::Empty, None);
            for arm in ["right_arm", "left_arm", "right_leg", "left_leg"] {
                let p = rig.p(arm);
                p.rot.x = (p.rot.x * 0.5).clamp(-0.4, 0.4);
            }
            if carrying {
                rig.p("right_arm").rot.x = -0.5;
                rig.p("left_arm").rot.x = -0.5;
                rig.p("right_arm").rot.z = 0.05;
                rig.p("left_arm").rot.z = -0.05;
            }
            if creepy {
                rig.p("head").pos.y -= 5.0;
                rig.p("hat").pos.y += 5.0;
            }
            let m = model_space(living_matrix(pos, &s, 1.0, 90.0), None);
            plain(ctx, &rig, m, &s, "entity/enderman/enderman");
            glow(ctx, &rig, m, &s, "entity/enderman/enderman_eyes");
            if let Some(DataValue::OptBlockState(Some(block))) = e.field("CARRY_STATE") {
                held::draw_carried_block(ctx, m, *block as u32, s.light);
            }
        }
        "villager" | "wandering_trader" | "witch" | "zombie_villager" => draw_villagers(ctx, e, &s, pos, name, right, left),
        "pillager" | "vindicator" | "evoker" | "illusioner" => {
            let Some(skel) = ctx.bodies.skeleton(name) else { return };
            let mut rig = Rig::new(skel);
            let casting = e.int_field("SPELL_CASTING").unwrap_or(0) > 0;
            let celebrating = e.bool_field("IS_CELEBRATING");
            let holding_item = e.equipment[MAIN_HAND].is_some();
            let pose = if name == "pillager" && e.bool_field("IS_CHARGING_CROSSBOW") {
                IllagerPose::CrossbowCharge(e.use_ticks as f32)
            } else if name == "pillager" && s.aggressive && holding(e, "crossbow") {
                IllagerPose::CrossbowHold
            } else if casting {
                IllagerPose::Spell
            } else if name == "illusioner" && s.aggressive {
                IllagerPose::Bow
            } else if s.aggressive {
                if holding_item { IllagerPose::Attack } else { IllagerPose::AttackBare }
            } else if celebrating {
                IllagerPose::Celebrate
            } else {
                IllagerPose::Crossed
            };
            anim::illager(&mut rig, &s, pose);
            let m = model_space(living_matrix(pos, &s, 1.0, 90.0), None);
            plain(ctx, &rig, m, &s, &format!("entity/illager/{name}"));
            if pose != IllagerPose::Crossed {
                held_items(ctx, e, &rig, m, &s);
            }
        }
        "vex" => {
            let Some(skel) = ctx.bodies.skeleton("vex") else { return };
            let mut rig = Rig::new(skel);
            let charging = e.int_field("FLAGS").unwrap_or(0) & 1 != 0;
            anim::vex(&mut rig, &s, charging);
            let m = model_space(living_matrix(pos, &s, 1.0, 90.0), None);
            plain(ctx, &rig, m, &s, if charging { "entity/illager/vex_charging" } else { "entity/illager/vex" });
            held_items(ctx, e, &rig, m, &s);
        }
        "allay" => {
            let Some(skel) = ctx.bodies.skeleton("allay") else { return };
            let mut rig = Rig::new(skel);
            let hold = if e.equipment[MAIN_HAND].is_some() { 1.0 } else { 0.0 };
            anim::allay(&mut rig, &s, hold);
            let m = model_space(living_matrix(pos, &s, 1.0, 90.0), None);
            plain(ctx, &rig, m, &s, "entity/allay/allay");
            held_items(ctx, e, &rig, m, &s);
        }
        "player" | "mannequin" => draw_player(ctx, session, e, &s, pos, right, left),
        "armor_stand" => draw_armor_stand(ctx, e, s, pos),
        _ => draw_creature(ctx, e, &s, pos, name),
    }
}

fn l_pose(p: ArmPose) -> ArmPose {
    p
}

fn draw_villagers(ctx: &mut Ctx, e: &Entity, s: &St, pos: Vec3, name: &str, right: ArmPose, left: ArmPose) {
    match name {
        "witch" => {
            let Some(skel) = ctx.bodies.skeleton("witch") else { return };
            let mut rig = Rig::new(skel);
            let using = e.bool_field("USING_ITEM");
            anim::witch(&mut rig, s, using);
            let m = model_space(living_matrix(pos, s, 1.0, 90.0), None);
            plain(ctx, &rig, m, s, "entity/witch/witch");
            if let Some(stack) = &e.equipment[MAIN_HAND] {
                if items::item_name(stack) == "potion" && using {
                    held::draw_witch_potion(ctx, &rig, m, s, stack);
                } else {
                    held::draw_villager_item(ctx, &rig, m, s, stack);
                }
            }
        }
        "wandering_trader" => {
            let Some(skel) = ctx.bodies.skeleton("wandering_trader") else { return };
            let mut rig = Rig::new(skel);
            let unhappy = e.int_field("UNHAPPY_COUNTER").unwrap_or(0) > 0;
            anim::villager_like(&mut rig, s, unhappy);
            let m = model_space(living_matrix(pos, s, 1.0, 90.0), None);
            plain(ctx, &rig, m, s, "entity/wandering_trader/wandering_trader");
            if let Some(stack) = &e.equipment[MAIN_HAND] {
                held::draw_villager_item(ctx, &rig, m, s, stack);
            }
        }
        "villager" => {
            let model = if s.baby { "villager_baby" } else { "villager" };
            let Some(skel) = ctx.bodies.skeleton(model) else { return };
            let mut rig = Rig::new(skel);
            let unhappy = e.int_field("UNHAPPY_COUNTER").unwrap_or(0) > 0;
            anim::villager_like(&mut rig, s, unhappy);
            let m = model_space(living_matrix(pos, s, 1.0, 90.0), None);
            plain(ctx, &rig, m, s, &with_baby("entity/villager/villager", s.baby));
            let no_hat = format!("{model}_no_hat");
            villager_layers(ctx, &rig, m, s, "villager", villager_data(e), model, &no_hat);
            if let Some(stack) = &e.equipment[MAIN_HAND] {
                held::draw_villager_item(ctx, &rig, m, s, stack);
            }
        }
        _ => {
            let model = if s.baby { "zombie_villager_baby" } else { "zombie_villager" };
            let Some(skel) = ctx.bodies.skeleton(model) else { return };
            let mut rig = Rig::new(skel);
            let holding_item = e.equipment[MAIN_HAND].is_some();
            anim::humanoid(&mut rig, s, right, left, Some(holding_item));
            let m = model_space(living_matrix(pos, s, 1.0, 90.0), None);
            plain(ctx, &rig, m, s, &with_baby("entity/zombie_villager/zombie_villager", s.baby));
            let no_hat = format!("{model}_no_hat");
            villager_layers(ctx, &rig, m, s, "zombie_villager", villager_data(e), model, &no_hat);
            humanoid_armor(ctx, e, &rig, m, s, model);
            held_items(ctx, e, &rig, m, s);
        }
    }
}

/// The game's default look for a player id: its hash picks one of the bundled skins.
pub fn default_skin(uuid: Uuid) -> (String, bool) {
    let (hi, lo) = uuid.as_u64_pair();
    let hilo = hi ^ lo;
    let hash = ((hilo >> 32) as u32 ^ hilo as u32) as i32;
    let names = ["alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri"];
    let i = hash.rem_euclid(18) as usize;
    if i < 9 { (format!("entity/player/slim/{}", names[i]), true) } else { (format!("entity/player/wide/{}", names[i - 9]), false) }
}

fn player_customisation(e: &Entity) -> i32 {
    e.int_field("PLAYER_MODE_CUSTOMISATION").unwrap_or(0x7F)
}

fn draw_player(ctx: &mut Ctx, session: &crate::game::Session, e: &Entity, s: &St, pos: Vec3, right: ArmPose, left: ArmPose) {
    let flags = player_customisation(e);
    let spectator = session.tab.get(&e.uuid).is_some_and(|t| t.game_mode == 3);
    draw_player_body(ctx, e.uuid, s, pos, right, left, flags, spectator, Some(e), e.velocity.as_vec3(), Mat4::IDENTITY);
}

/// How a player's whole body tips while swimming or gliding.
fn player_rotations(s: &St, velocity: Vec3) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    if s.gliding {
        // Eases in over the first second of flight.
        let scale = 1.0f32;
        m *= Mat4::from_rotation_x((scale * (-90.0 - s.pitch)).to_radians());
        let horizontal = Vec3::new(velocity.x, 0.0, velocity.z);
        let (y, p) = ((s.body + s.yaw).to_radians(), s.pitch.to_radians());
        let look = Vec3::new(-y.sin() * p.cos(), 0.0, y.cos() * p.cos());
        if horizontal.length_squared() > 0.0 && look.length_squared() > 0.0 {
            let dot = horizontal.dot(look);
            let cross = horizontal.x * look.z - horizontal.z * look.x;
            m *= Mat4::from_rotation_y(cross.signum() * (dot / (horizontal.length() * look.length())).clamp(-1.0, 1.0).acos());
        }
    } else if s.swimming > 0.0 {
        let target = if s.in_water { -90.0 - s.pitch } else { -90.0 };
        m *= Mat4::from_rotation_x((s.swimming * target).to_radians());
        m *= Mat4::from_translation(Vec3::new(0.0, -1.0, 0.3));
    }
    m
}

#[allow(clippy::too_many_arguments)]
pub fn draw_player_body(ctx: &mut Ctx, uuid: Uuid, s: &St, pos: Vec3, right: ArmPose, left: ArmPose, flags: i32, spectator: bool, e: Option<&Entity>, velocity: Vec3, _pre: Mat4) {
    let (sprite, slim) = ctx.skins.look_of(uuid, &ctx.bodies.sheet);
    let model = if slim { "player_slim" } else { "player" };
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    let mut st = St { slim, ..*s };
    anim::humanoid(&mut rig, &st, right, left, None);
    rig.p("hat").visible = flags & 0x40 != 0;
    rig.p("jacket").visible = flags & 0x02 != 0;
    rig.p("left_sleeve").visible = flags & 0x04 != 0;
    rig.p("right_sleeve").visible = flags & 0x08 != 0;
    rig.p("left_pants").visible = flags & 0x10 != 0;
    rig.p("right_pants").visible = flags & 0x20 != 0;
    if spectator {
        for part in ["body", "left_arm", "right_arm", "left_leg", "right_leg"] {
            rig.p(part).visible = false;
        }
    }
    let m = living_matrix(pos, &st, 1.0, 90.0) * player_rotations(&st, velocity);
    let m = model_space(m, Some(Mat4::from_scale(Vec3::splat(0.9375))));
    if st.invisible {
        st.invisible = false;
        if let Some(e) = e {
            humanoid_armor(ctx, e, &rig, m, &st, model);
            held_items(ctx, e, &rig, m, &st);
        }
        return;
    }
    draw_layer(ctx, &rig, m, sprite, &st, WHITE, Pass::Cutout, true);
    if let Some(e) = e {
        humanoid_armor(ctx, e, &rig, m, &st, model);
        held_items(ctx, e, &rig, m, &st);
        super::trims::stuck_in_body(ctx, e, &rig, m, &st);
        super::trims::shoulder_parrots(ctx, e, m, &st);
        let chest = e.equipment[CHEST].as_ref();
        if flags & 0x01 != 0 && !chest.is_some_and(items::glides) {
            let p = ctx.partial as f64;
            let at = e.prev_pos.lerp(e.pos, p);
            let state = super::trims::CapeState {
                lag: e.prev_cloak.lerp(e.cloak, p) - at,
                body_yaw: st.body,
                bob: e.walk_speed * 0.1,
                walked: e.walk_position * 0.6,
                gliding: st.gliding,
            };
            super::trims::draw_cape(ctx, uuid, &rig, m, &st, &state, chest.is_some_and(|c| items::equip_slot(c).as_deref() == Some("chest")));
        }
    }
}

/// The player's own body, when the camera is outside it.
pub fn draw_local_player(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, camera: glam::DVec3) {
    let p = &session.player;
    let at = p.prev_pos.lerp(p.body.pos, ctx.partial as f64);
    let lit = at + glam::DVec3::new(0.0, 0.9, 0.0);
    let light = super::light_at(world, lit.x.floor() as i32, lit.y.floor() as i32, lit.z.floor() as i32);
    let id = p.entity_id;
    let s = St {
        id,
        age: p.age as f32 + ctx.partial,
        wp: p.limb_position - p.limb_speed * (1.0 - ctx.partial),
        ws: (p.prev_limb_speed + (p.limb_speed - p.prev_limb_speed) * ctx.partial).min(1.0),
        yaw: wrap_degrees(p.body.yaw - p.body_yaw(ctx.partial)),
        pitch: p.body.pitch,
        body: p.body_yaw(ctx.partial),
        baby: false,
        age_scale: 1.0,
        swing: p.swing.progress(ctx.partial),
        swing_off_hand: p.swing.off_hand(),
        death: 0.0,
        hurt: session.hurt_ticks < 10,
        in_water: p.body.in_water,
        riding: p.vehicle.is_some(),
        crouching: p.body.crouching,
        swimming: if p.body.is_swimming() { 1.0 } else { 0.0 },
        gliding: p.body.fall_flying,
        light,
        invisible: false,
        left_handed: false,
        aggressive: false,
        sleeping: false,
        moving: false,
        slim: false,
    };
    let inv = &session.inventory;
    let main = inv.hotbar(p.selected_slot.clamp(0, 8) as usize);
    let off = inv.offhand();
    let right = arm_pose_for(main, p.using == Some(0), p.use_ticks);
    let left = arm_pose_for(off, p.using == Some(1), p.use_ticks);
    let flags = session.self_customisation();
    let (sprite, slim) = ctx.skins.look_of(session.self_uuid(), &ctx.bodies.sheet);
    let s = St { slim, ..s };
    let model = if slim { "player_slim" } else { "player" };
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    anim::humanoid(&mut rig, &s, right, left, None);
    rig.p("hat").visible = flags & 0x40 != 0;
    rig.p("jacket").visible = flags & 0x02 != 0;
    rig.p("left_sleeve").visible = flags & 0x04 != 0;
    rig.p("right_sleeve").visible = flags & 0x08 != 0;
    rig.p("left_pants").visible = flags & 0x10 != 0;
    rig.p("right_pants").visible = flags & 0x20 != 0;
    let pos = (at - camera).as_vec3();
    let m = living_matrix(pos, &s, 1.0, 90.0) * player_rotations(&s, p.body.vel.as_vec3());
    let m = model_space(m, Some(Mat4::from_scale(Vec3::splat(0.9375))));
    draw_layer(ctx, &rig, m, sprite, &s, WHITE, Pass::Cutout, true);
    let chest = inv.slots.get(crate::game::inventory::ARMOR_CHEST).and_then(|x| x.as_ref());
    if flags & 0x01 != 0 && !chest.is_some_and(items::glides) {
        let partial = ctx.partial as f64;
        let state = super::trims::CapeState { lag: p.prev_cloak.lerp(p.cloak, partial) - at, body_yaw: s.body, bob: p.bob, walked: p.walk_dist, gliding: s.gliding };
        super::trims::draw_cape(ctx, session.self_uuid(), &rig, m, &s, &state, chest.is_some_and(|c| items::equip_slot(c).as_deref() == Some("chest")));
    }
    if let Some(stack) = main {
        held::draw_in_hand(ctx, &rig, m, &s, stack, false);
    }
    if let Some(stack) = off {
        held::draw_in_hand(ctx, &rig, m, &s, stack, true);
    }
    // Armour worn by the player, from the inventory rather than entity data.
    for (slot, part, kind, want) in [
        (crate::game::inventory::ARMOR_FEET, "boots", "humanoid", "feet"),
        (crate::game::inventory::ARMOR_LEGS, "leggings", "humanoid_leggings", "legs"),
        (crate::game::inventory::ARMOR_CHEST, "chestplate", "humanoid", "chest"),
        (crate::game::inventory::ARMOR_HEAD, "helmet", "humanoid", "head"),
    ] {
        if let Some(stack) = inv.slots.get(slot).and_then(|x| x.as_ref()) {
            if items::equip_slot(stack).as_deref() == Some(want) {
                armor_layer(ctx, &rig, m, &s, &format!("{model}#{part}"), stack, kind);
            }
        }
    }
}

fn draw_armor_stand(ctx: &mut Ctx, e: &Entity, s: St, pos: Vec3) {
    let flags = e.int_field("CLIENT_FLAGS").unwrap_or(0);
    let small = flags & 1 != 0;
    let model = if small { "armor_stand_small" } else { "armor_stand" };
    let Some(skel) = ctx.bodies.skeleton(model) else { return };
    let mut rig = Rig::new(skel);
    let pose = |n: &str, dflt: [f32; 3]| match e.field(n) {
        Some(DataValue::Rotations(r)) => Vec3::from(*r) * (PI / 180.0),
        _ => Vec3::from(dflt) * (PI / 180.0),
    };
    rig.p("head").rot = pose("HEAD_POSE", [0.0, 0.0, 0.0]);
    rig.p("body").rot = pose("BODY_POSE", [0.0, 0.0, 0.0]);
    rig.p("left_arm").rot = pose("LEFT_ARM_POSE", [-10.0, 0.0, -10.0]);
    rig.p("right_arm").rot = pose("RIGHT_ARM_POSE", [-15.0, 0.0, 10.0]);
    rig.p("left_leg").rot = pose("LEFT_LEG_POSE", [-1.0, 0.0, -1.0]);
    rig.p("right_leg").rot = pose("RIGHT_LEG_POSE", [1.0, 0.0, 1.0]);
    let body_pose = pose("BODY_POSE", [0.0, 0.0, 0.0]);
    rig.p("base_plate").rot.y = -s.yaw * PI / 180.0;
    rig.p("base_plate").visible = flags & 8 == 0;
    rig.p("left_arm").visible = flags & 4 != 0;
    rig.p("right_arm").visible = flags & 4 != 0;
    rig.p("right_body_stick").rot = body_pose;
    rig.p("left_body_stick").rot = body_pose;
    rig.p("shoulder_stick").rot = body_pose;
    let mut st = s;
    st.body = wrap_degrees(e.yaw);
    st.hurt = false;
    st.death = 0.0;
    st.baby = false;
    st.swing = 0.0;
    let m = model_space(living_matrix(pos, &st, 1.0, 90.0), None);
    if flags & 16 == 0 {
        plain(ctx, &rig, m, &st, "entity/armorstand/armorstand");
    }
    let mut armor_st = st;
    armor_st.invisible = false;
    humanoid_armor(ctx, e, &rig, m, &armor_st, model);
    if flags & 4 != 0 {
        held_items(ctx, e, &rig, m, &armor_st);
    }
}

fn draw_creature(ctx: &mut Ctx, e: &Entity, s: &St, pos: Vec3, name: &str) {
    let base = living_matrix(pos, s, 1.0, 90.0);
    let default_m = model_space(base, None);
    let bodies = ctx.bodies;
    let setup = |model: &str| bodies.skeleton(model).map(Rig::new);
    let flags_of = |n: &str| e.int_field(n).unwrap_or(0);
    let variant_name = |ctx: &Ctx, reg: &str, field: &str, dflt: &str| -> String {
        e.int_field(field).and_then(|v| ctx.variant(reg, v).map(str::to_string)).unwrap_or_else(|| dflt.to_string())
    };
    match name {
        "creeper" => {
            let Some(mut r) = setup("creeper") else { return };
            anim::quadruped(&mut r, s, 1.4);
            let swelling = e.swell_at(ctx.partial);
            let v = 1.0 + (swelling * 100.0).sin() * swelling * 0.01;
            let c = swelling.clamp(0.0, 1.0);
            let c = c * c;
            let c = c * c;
            let (a, b) = ((1.0 + c * 0.4) * v, (1.0 + c * 0.1) / v);
            let m = model_space(base, Some(Mat4::from_scale(Vec3::new(a, b, a))));
            let white = if ((swelling * 10.0) as i32) % 2 == 0 { 0.0 } else { swelling.clamp(0.5, 1.0) * 0.75 };
            if !s.invisible {
                let look = Look {
                    sprite: ctx.sprite("entity/creeper/creeper").unwrap_or(ctx.bodies.sheet.missing),
                    color: WHITE,
                    light: s.light,
                    overlay: [if s.hurt { 255 } else { 0 }, (white * 255.0) as u8],
                    scroll: [0.0, 0.0],
                    lit: true,
                };
                lattice_render::parts::draw(&r, m, &look, ctx.list(Pass::Cutout));
                if e.bool_field("IS_POWERED") {
                    energy_layer(ctx, &r, m, s, "creeper#armor", Pass::EnergyCreeper, s.age * 0.01);
                }
            }
        }
        "spider" | "cave_spider" => {
            let Some(mut r) = setup(name) else { return };
            anim::spider(&mut r, s);
            plain(ctx, &r, default_m, s, &format!("entity/spider/{name}"));
            glow(ctx, &r, default_m, s, "entity/spider/spider_eyes");
        }
        "pig" => {
            let v = variant_name(ctx, "pig_variant", "VARIANT", "temperate");
            let model = if s.baby { "pig_baby".to_string() } else if v == "cold" { "cold_pig".to_string() } else { "pig".to_string() };
            let Some(mut r) = setup(&model) else { return };
            anim::quadruped(&mut r, s, 1.4);
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/pig/pig_{v}"), s.baby));
            worn(ctx, e, &r, default_m, s, SADDLE, "pig#saddle", "pig_saddle");
        }
        "cow" => {
            let v = variant_name(ctx, "cow_variant", "VARIANT", "temperate");
            let model = match (v.as_str(), s.baby) {
                ("cold", false) => "cold_cow",
                ("cold", true) => "cold_cow_baby",
                ("warm", false) => "warm_cow",
                ("warm", true) => "warm_cow_baby",
                (_, true) => "cow_baby",
                _ => "cow",
            };
            let Some(mut r) = setup(model) else { return };
            anim::quadruped(&mut r, s, 1.4);
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/cow/cow_{v}"), s.baby));
        }
        "mooshroom" => {
            let Some(mut r) = setup(if s.baby { "mooshroom_baby" } else { "mooshroom" }) else { return };
            anim::quadruped(&mut r, s, 1.4);
            let kind = if e.int_field("TYPE").unwrap_or(0) == 1 { "brown" } else { "red" };
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/cow/mooshroom_{kind}"), s.baby));
            if !s.baby {
                held::draw_mushrooms(ctx, &r, default_m, s, kind);
            }
        }
        "chicken" => {
            let v = variant_name(ctx, "chicken_variant", "VARIANT", "temperate");
            let model = if s.baby { "chicken_baby" } else if v == "cold" { "cold_chicken" } else { "chicken" };
            let Some(mut r) = setup(model) else { return };
            let flap = if e.on_ground { 0.0 } else { ((s.age * 1.3).sin() + 1.0) * 1.0 };
            anim::chicken(&mut r, s, flap);
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/chicken/chicken_{v}"), s.baby));
        }
        "sheep" => {
            let Some(mut r) = setup(if s.baby { "sheep_baby" } else { "sheep" }) else { return };
            anim::quadruped(&mut r, s, 1.4);
            plain(ctx, &r, default_m, s, &with_baby("entity/sheep/sheep", s.baby));
            let wool = e.int_field("WOOL").unwrap_or(0);
            let color = sheep_color((wool & 15) as usize);
            if s.invisible {
                return;
            }
            if !s.baby {
                second_model(ctx, &r, default_m, s, "sheep#wool_undercoat", "entity/sheep/sheep_wool_undercoat", color, Pass::Cutout);
            }
            if wool & 16 == 0 {
                let model = if s.baby { "sheep_baby#wool" } else { "sheep#wool" };
                second_model(ctx, &r, default_m, s, model, &with_baby("entity/sheep/sheep_wool", s.baby), color, Pass::Cutout);
            }
        }
        "goat" => {
            let Some(mut r) = setup(if s.baby { "goat_baby" } else { "goat" }) else { return };
            anim::quadruped(&mut r, s, 1.4);
            let (left_horn, right_horn) = (e.bool_field("HAS_LEFT_HORN"), e.bool_field("HAS_RIGHT_HORN"));
            r.p("left_horn").visible = left_horn;
            r.p("right_horn").visible = right_horn;
            plain(ctx, &r, default_m, s, &with_baby("entity/goat/goat", s.baby));
        }
        "polar_bear" => {
            let Some(mut r) = setup(if s.baby { "polar_bear_baby" } else { "polar_bear" }) else { return };
            anim::polar_bear(&mut r, s, if e.bool_field("STANDING") { 1.0 } else { 0.0 });
            plain(ctx, &r, default_m, s, &with_baby("entity/bear/polarbear", s.baby));
        }
        "panda" => {
            let Some(mut r) = setup(if s.baby { "panda_baby" } else { "panda" }) else { return };
            let f = flags_of("ID_FLAGS");
            anim::panda(&mut r, s, if f & 8 != 0 { 1.0 } else { 0.0 }, if f & 4 != 0 { 1.0 } else { 0.0 }, if f & 16 != 0 { 1.0 } else { 0.0 });
            let main = flags_of("MAIN_GENE");
            let hidden = flags_of("HIDDEN_GENE");
            let recessive = |g: i32| g == 4 || g == 5;
            let gene = if recessive(main) { if main == hidden { main } else { 0 } } else { main };
            let trait_name = ["", "lazy", "worried", "playful", "brown", "weak", "aggressive"][gene.clamp(0, 6) as usize];
            let tex = match (trait_name.is_empty(), s.baby) {
                (true, false) => "entity/panda/panda".to_string(),
                (true, true) => "entity/panda/panda_baby".to_string(),
                (false, false) => format!("entity/panda/panda_{trait_name}"),
                (false, true) => format!("entity/panda/{trait_name}_panda_baby"),
            };
            plain(ctx, &r, default_m, s, &tex);
        }
        "turtle" => {
            let Some(mut r) = setup(if s.baby { "turtle_baby" } else { "turtle" }) else { return };
            anim::quadruped(&mut r, s, 1.4);
            anim::turtle(&mut r, s, !s.in_water, e.bool_field("LAYING_EGG"), e.bool_field("HAS_EGG"));
            plain(ctx, &r, default_m, s, &with_baby("entity/turtle/turtle", s.baby));
        }
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => {
            let model = if s.baby { format!("{name}_baby") } else { name.to_string() };
            let Some(mut r) = setup(&model) else { return };
            let f = flags_of("ID_FLAGS");
            let eat = if f & 16 != 0 { 1.0 } else { 0.0 };
            let stand = if f & 32 != 0 { 1.0 } else { 0.0 };
            anim::equine(&mut r, s, eat, stand, f & 8 != 0);
            let chest = e.bool_field("ID_CHEST");
            r.p("left_chest").visible = chest;
            r.p("right_chest").visible = chest;
            match name {
                "horse" => {
                    let v = flags_of("ID_TYPE_VARIANT");
                    let colour = ["white", "creamy", "chestnut", "brown", "black", "gray", "darkbrown"][(v & 255).clamp(0, 6) as usize];
                    plain(ctx, &r, default_m, s, &with_baby(&format!("entity/horse/horse_{colour}"), s.baby));
                    let marks = (v >> 8) & 255;
                    if let Some(m) = ["", "white", "whitefield", "whitedots", "blackdots"].get(marks as usize).filter(|m| !m.is_empty()) {
                        overlay(ctx, &r, default_m, s, &with_baby(&format!("entity/horse/horse_markings_{m}"), s.baby), WHITE);
                    }
                    if !s.baby {
                        worn(ctx, e, &r, default_m, s, BODY, "horse_armor", "horse_body");
                    }
                    worn(ctx, e, &r, default_m, s, SADDLE, "horse#saddle", "horse_saddle");
                }
                "donkey" | "mule" => {
                    plain(ctx, &r, default_m, s, &with_baby(&format!("entity/horse/{name}"), s.baby));
                    worn(ctx, e, &r, default_m, s, SADDLE, &format!("{name}#saddle"), &format!("{name}_saddle"));
                }
                _ => {
                    let tex = if name == "skeleton_horse" { "entity/horse/horse_skeleton" } else { "entity/horse/horse_zombie" };
                    plain(ctx, &r, default_m, s, &with_baby(tex, s.baby));
                    worn(ctx, e, &r, default_m, s, SADDLE, &format!("{name}#saddle"), &format!("{name}_saddle"));
                }
            }
        }
        "llama" | "trader_llama" => {
            let model = if s.baby { format!("{name}_baby") } else { name.to_string() };
            let Some(mut r) = setup(&model) else { return };
            anim::llama(&mut r, s, e.bool_field("ID_CHEST"));
            let colour = ["creamy", "white", "brown", "gray"][flags_of("VARIANT").clamp(0, 3) as usize];
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/llama/llama_{colour}"), s.baby));
            let decor = if s.baby { "llama_baby#decor" } else { "llama#decor" };
            if let Some(stack) = &e.equipment[BODY] {
                armor_layer(ctx, &r, default_m, s, decor, stack, "llama_body");
            } else if name == "trader_llama" {
                let tex = if s.baby { "entity/equipment/llama_body/trader_llama_baby" } else { "entity/equipment/llama_body/trader_llama" };
                second_model(ctx, &r, default_m, s, decor, tex, WHITE, Pass::Cutout);
            }
        }
        "camel" | "camel_husk" => {
            let model = if s.baby { "camel_baby" } else { "camel" };
            let Some(mut r) = setup(model) else { return };
            {
                let h = r.p("head");
                h.rot.y = s.yaw.clamp(-30.0, 30.0) * PI / 180.0;
                h.rot.x = s.pitch.clamp(-25.0, 45.0) * PI / 180.0;
            }
            let walk = if s.baby { "CamelBabyAnimation.CAMEL_BABY_WALK" } else { "CamelAnimation.CAMEL_WALK" };
            if let Some(a) = bodies.animation(walk) {
                r.play_walk(a, s.wp, s.ws, 2.0, 2.5);
            }
            if e.pose() == 8 || e.pose() == 7 {
                let sit = if s.baby { "CamelBabyAnimation.CAMEL_BABY_SIT_POSE" } else { "CamelAnimation.CAMEL_SIT_POSE" };
                if let Some(a) = bodies.animation(sit) {
                    r.play(a, 0.0, 1.0);
                }
            }
            let tex = if name == "camel" { with_baby("entity/camel/camel", s.baby) } else { "entity/camel/camel_husk".to_string() };
            plain(ctx, &r, default_m, s, &tex);
            let saddle_model = format!("{name}#saddle");
            worn(ctx, e, &r, default_m, s, SADDLE, &saddle_model, &format!("{name}_saddle"));
        }
        "hoglin" | "zoglin" => {
            let model = if s.baby { format!("{name}_baby") } else { name.to_string() };
            let Some(mut r) = setup(&model) else { return };
            anim::hoglin(&mut r, s, e.attack_ticks(ctx.partial));
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/hoglin/{name}"), s.baby));
        }
        "ravager" => {
            let Some(mut r) = setup("ravager") else { return };
            anim::ravager(&mut r, s, e.attack_ticks(ctx.partial), 0.0, 0.0);
            plain(ctx, &r, default_m, s, "entity/illager/ravager");
        }
        "wolf" => {
            let v = variant_name(ctx, "wolf_variant", "VARIANT", "pale");
            let f = flags_of("FLAGS");
            let tame = f & 4 != 0;
            let angry = matches!(e.field("ANGER_END_TIME"), Some(DataValue::Long(t)) if *t > 0);
            let Some(mut r) = setup(if s.baby { "wolf_baby" } else { "wolf" }) else { return };
            let health = e.float_field("HEALTH").unwrap_or(20.0);
            let tail = if angry { 1.5393804 } else if tame { (0.55 - (40.0 - health) * 0.02) * PI } else { PI / 5.0 };
            anim::wolf(&mut r, s, angry, f & 1 != 0, tail);
            let base_name = if v == "pale" { "entity/wolf/wolf".to_string() } else { format!("entity/wolf/wolf_{v}") };
            let state = if tame { "_tame" } else if angry { "_angry" } else { "" };
            plain(ctx, &r, default_m, s, &with_baby(&format!("{base_name}{state}"), s.baby));
            if tame {
                let color = rgb(DYE_COLORS[flags_of("COLLAR_COLOR").clamp(0, 15) as usize]);
                overlay(ctx, &r, default_m, s, &with_baby("entity/wolf/wolf_collar", s.baby), color);
            }
            if !s.baby {
                worn(ctx, e, &r, default_m, s, BODY, "wolf_armor", "wolf_body");
            }
        }
        "cat" | "ocelot" => {
            let Some(mut r) = setup(&if s.baby { format!("{name}_baby") } else { name.to_string() }) else { return };
            let f = flags_of("FLAGS");
            let sitting = name == "cat" && f & 1 != 0;
            anim::feline(&mut r, s, e.flag(1), e.flag(3), sitting);
            if name == "ocelot" {
                plain(ctx, &r, default_m, s, &with_baby("entity/cat/ocelot", s.baby));
                return;
            }
            let v = variant_name(ctx, "cat_variant", "VARIANT", "tabby");
            let lying = e.bool_field("IS_LYING");
            let m = if lying { model_space(base * Mat4::from_translation(Vec3::new(0.4, 0.15, 0.1)) * Mat4::from_rotation_z(FRAC_PI_2), None) } else { default_m };
            plain(ctx, &r, m, s, &with_baby(&format!("entity/cat/cat_{v}"), s.baby));
            if f & 4 != 0 {
                let color = rgb(DYE_COLORS[flags_of("COLLAR_COLOR").clamp(0, 15) as usize]);
                second_model(ctx, &r, m, s, if s.baby { "cat_baby#collar" } else { "cat#collar" }, &with_baby("entity/cat/cat_collar", s.baby), color, Pass::Cutout);
            }
        }
        "fox" => {
            let Some(mut r) = setup(if s.baby { "fox_baby" } else { "fox" }) else { return };
            let f = flags_of("FLAGS");
            let (sitting, crouching, sleeping) = (f & 1 != 0, f & 4 != 0, f & 32 != 0);
            if s.baby {
                look_only(&mut r, s);
                if let Some(a) = bodies.animation("FoxBabyAnimation.FOX_BABY_WALK") {
                    r.play_walk(a, s.wp, s.ws, 1.0, 2.5);
                }
            } else {
                anim::fox(&mut r, s, crouching, sleeping, sitting);
            }
            let kind = if flags_of("TYPE") == 1 { "fox_snow" } else { "fox" };
            let tex = if sleeping { format!("entity/fox/{kind}_sleep") } else { format!("entity/fox/{kind}") };
            plain(ctx, &r, default_m, s, &with_baby(&tex, s.baby));
            if let Some(stack) = &e.equipment[MAIN_HAND] {
                held::draw_in_mouth(ctx, &r, default_m, s, stack);
            }
        }
        "rabbit" => {
            let Some(mut r) = setup(if s.baby { "rabbit_baby" } else { "rabbit" }) else { return };
            anim::look(&mut r, s);
            let hop = if s.baby { "BabyRabbitAnimation.HOP" } else { "RabbitAnimation.HOP" };
            if !e.on_ground || s.moving {
                if let Some(a) = bodies.animation(hop) {
                    r.play(a, s.age / 20.0, 1.0);
                }
            }
            let t = flags_of("TYPE");
            let kind = match t {
                0 => "brown",
                1 => "white",
                2 => "black",
                3 => "white_splotched",
                4 => "gold",
                5 => "salt",
                99 => "caerbannog",
                _ => "brown",
            };
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/rabbit/rabbit_{kind}"), s.baby));
        }
        "frog" => {
            let Some(mut r) = setup("frog") else { return };
            r.p("croaking_body").visible = false;
            if e.pose() == 8 || s.in_water {
                if let Some(a) = bodies.animation("FrogAnimation.FROG_SWIM") {
                    r.play_walk(a, s.wp, s.ws, 1.0, 2.5);
                }
                if let Some(a) = bodies.animation("FrogAnimation.FROG_IDLE_WATER") {
                    if s.ws < 0.1 {
                        r.play(a, s.age / 20.0, 1.0);
                    }
                }
            } else if let Some(a) = bodies.animation("FrogAnimation.FROG_WALK") {
                r.play_walk(a, s.wp, s.ws, 1.5, 2.5);
            }
            let v = variant_name(ctx, "frog_variant", "VARIANT", "temperate");
            plain(ctx, &r, default_m, s, &format!("entity/frog/frog_{v}"));
        }
        "tadpole" => {
            let Some(mut r) = setup("tadpole") else { return };
            anim::fish_tail(&mut r, s, "tail", 0.25, 0.3);
            plain(ctx, &r, default_m, s, "entity/tadpole/tadpole");
        }
        "axolotl" => {
            let Some(mut r) = setup(if s.baby { "axolotl_baby" } else { "axolotl" }) else { return };
            let moving = if s.ws > 0.05 || s.moving { 1.0 } else { 0.0 };
            let water = if s.in_water { 1.0 } else { 0.0 };
            let ground = 1.0 - water;
            let dead = if e.bool_field("PLAYING_DEAD") { 1.0 } else { 0.0 };
            if s.baby {
                let name = if s.in_water {
                    if moving > 0.0 { "BabyAxolotlAnimation.BABY_AXOLOTL_SWIM" } else { "BabyAxolotlAnimation.IDLE_UNDERWATER" }
                } else if moving > 0.0 {
                    "BabyAxolotlAnimation.AXOLOTL_WALK_FLOOR"
                } else {
                    "BabyAxolotlAnimation.BABY_AXOLOTL_IDLE_FLOOR"
                };
                if let Some(a) = bodies.animation(name) {
                    r.play(a, s.age / 20.0, 1.0);
                }
            } else {
                anim::axolotl(&mut r, s, moving, water, ground, dead);
            }
            let colour = ["lucy", "wild", "gold", "cyan", "blue"][flags_of("VARIANT").clamp(0, 4) as usize];
            plain(ctx, &r, default_m, s, &with_baby(&format!("entity/axolotl/axolotl_{colour}"), s.baby));
        }
        "bat" => {
            let Some(mut r) = setup("bat") else { return };
            let resting = flags_of("ID_FLAGS") & 1 != 0;
            if resting {
                r.p("head").rot.y = s.yaw * PI / 180.0;
                if let Some(a) = bodies.animation("BatAnimation.BAT_RESTING") {
                    r.play(a, s.age / 20.0, 1.0);
                }
            } else if let Some(a) = bodies.animation("BatAnimation.BAT_FLYING") {
                r.play(a, s.age / 20.0, 1.0);
            }
            plain(ctx, &r, default_m, s, "entity/bat/bat");
        }
        "bee" => {
            let Some(mut r) = setup(if s.baby { "bee_baby" } else { "bee" }) else { return };
            let f = flags_of("FLAGS");
            let angry = matches!(e.field("ANGER_END_TIME"), Some(DataValue::Long(t)) if *t > 0);
            anim::bee(&mut r, s, e.on_ground, angry, f & 4 == 0);
            let nectar = f & 8 != 0;
            let tex = match (angry, nectar) {
                (true, true) => "entity/bee/bee_angry_nectar",
                (true, false) => "entity/bee/bee_angry",
                (false, true) => "entity/bee/bee_nectar",
                _ => "entity/bee/bee",
            };
            plain(ctx, &r, default_m, s, &with_baby(tex, s.baby));
        }
        "blaze" => {
            let Some(mut r) = setup("blaze") else { return };
            anim::blaze(&mut r, s);
            let mut st = *s;
            st.light[1] = 255;
            plain(ctx, &r, default_m, &st, "entity/blaze/blaze");
        }
        "breeze" => {
            let Some(mut r) = setup("breeze") else { return };
            if let Some(a) = bodies.animation("BreezeAnimation.IDLE") {
                r.play(a, s.age / 20.0, 1.0);
            }
            plain(ctx, &r, default_m, s, "entity/breeze/breeze");
            let mut st = *s;
            st.light = [255, 255];
            second_model(ctx, &r, default_m, &st, "breeze#wind", "entity/breeze/breeze_wind", WHITE, Pass::Translucent);
            second_model(ctx, &r, default_m, &st, "breeze#eyes", "entity/breeze/breeze_eyes", WHITE, Pass::Glow);
        }
        "copper_golem" => {
            let Some(mut r) = setup("copper_golem") else { return };
            anim::look(&mut r, s);
            if let Some(a) = bodies.animation("CopperGolemAnimation.COPPER_GOLEM_WALK") {
                r.play_walk(a, s.wp, s.ws, 2.0, 2.5);
            }
            let state = ["", "_exposed", "_weathered", "_oxidized"][flags_of("WEATHER_STATE").clamp(0, 3) as usize];
            plain(ctx, &r, default_m, s, &format!("entity/copper_golem/copper_golem{state}"));
            second_model(ctx, &r, default_m, s, "copper_golem#eyes", &format!("entity/copper_golem/copper_golem_eyes{state}"), WHITE, Pass::Glow);
            held_items(ctx, e, &r, default_m, s);
        }
        "creaking" => {
            let Some(mut r) = setup("creaking") else { return };
            anim::look(&mut r, s);
            if e.bool_field("CAN_MOVE") {
                if let Some(a) = bodies.animation("CreakingAnimation.CREAKING_WALK") {
                    r.play_walk(a, s.wp, s.ws, 1.0, 1.0);
                }
            }
            plain(ctx, &r, default_m, s, "entity/creaking/creaking");
            if e.bool_field("IS_ACTIVE") {
                second_model(ctx, &r, default_m, s, "creaking#eyes", "entity/creaking/creaking_eyes", WHITE, Pass::Glow);
            }
        }
        "dolphin" => {
            let Some(mut r) = setup(if s.baby { "dolphin_baby" } else { "dolphin" }) else { return };
            anim::dolphin(&mut r, s, s.moving);
            plain(ctx, &r, default_m, s, &with_baby("entity/dolphin/dolphin", s.baby));
            if let Some(stack) = &e.equipment[MAIN_HAND] {
                held::draw_in_mouth(ctx, &r, default_m, s, stack);
            }
        }
        "squid" | "glow_squid" => {
            let model = if s.baby { format!("{name}_baby") } else { name.to_string() };
            let Some(mut r) = setup(&model) else { return };
            let t = (s.age * 0.15) % std::f32::consts::TAU;
            let angle = if !s.in_water {
                t.sin().abs() * PI * 0.25
            } else if t < PI {
                let f = t / PI;
                (f * f * PI).sin() * PI * 0.25
            } else {
                0.0
            };
            anim::squid(&mut r, angle);
            let up = if s.baby { 0.25 } else { 0.5 };
            let tilt = e.prev_body_pitch + (e.body_pitch - e.prev_body_pitch) * ctx.partial;
            let m = Mat4::from_translation(pos)
                * Mat4::from_translation(Vec3::new(0.0, up, 0.0))
                * Mat4::from_rotation_y((180.0 - s.body).to_radians())
                * Mat4::from_rotation_x(tilt.to_radians())
                * Mat4::from_translation(Vec3::new(0.0, if s.baby { -0.6 } else { -1.2 }, 0.0));
            let m = model_space(m, None);
            let mut st = *s;
            if name == "glow_squid" {
                st.light = [255, 255];
            }
            plain(ctx, &r, m, &st, &with_baby(&format!("entity/squid/{name}"), s.baby));
        }
        "guardian" | "elder_guardian" => {
            let Some(mut r) = setup(name) else { return };
            anim::guardian(&mut r, s, if e.bool_field("ID_MOVING") { 0.0 } else { 1.0 }, s.age * 0.25);
            plain(ctx, &r, default_m, s, if name == "guardian" { "entity/guardian/guardian" } else { "entity/guardian/guardian_elder" });
        }
        "ghast" => {
            let Some(mut r) = setup("ghast") else { return };
            anim::ghast(&mut r, s);
            let mut st = *s;
            st.light[1] = 255;
            plain(ctx, &r, default_m, &st, if e.bool_field("IS_CHARGING") { "entity/ghast/ghast_shooting" } else { "entity/ghast/ghast" });
        }
        "happy_ghast" => {
            let model = if s.baby { "happy_ghast_baby" } else { "happy_ghast" };
            let Some(mut r) = setup(model) else { return };
            anim::ghast(&mut r, s);
            if e.equipment[BODY].is_some() {
                r.p("body").scale = Vec3::splat(0.9375);
            }
            plain(ctx, &r, default_m, s, &with_baby("entity/ghast/happy_ghast", s.baby));
            if !s.baby {
                worn(ctx, e, &r, default_m, s, BODY, "happy_ghast_harness", "happy_ghast_body");
                if e.bool_field("IS_LEASH_HOLDER") {
                    second_model(ctx, &r, default_m, s, "happy_ghast_ropes", "entity/ghast/happy_ghast_ropes", WHITE, Pass::Cutout);
                }
            }
        }
        "slime" | "magma_cube" | "sulfur_cube" => {
            let size = flags_of("ID_SIZE").max(1) as f32;
            let squish = e.squish_at(ctx.partial) / (size * 0.5 + 1.0);
            let h = 1.0 / (squish + 1.0);
            let extra = Mat4::from_scale(Vec3::new(h * size, size / h, h * size));
            let shrink = Mat4::from_scale(Vec3::splat(0.999));
            let m = model_space(base, Some(if name == "magma_cube" { extra } else { shrink * extra }));
            match name {
                "magma_cube" => {
                    let Some(mut r) = setup("magma_cube") else { return };
                    let v = e.squish_at(ctx.partial).max(0.0);
                    for (i, cube) in ["cube0", "cube1", "cube2", "cube3", "cube4", "cube5", "cube6", "cube7"].iter().enumerate() {
                        r.p(cube).pos.y = -(4.0 - i as f32) * v * 1.7;
                    }
                    let mut st = *s;
                    st.light[1] = 255;
                    plain(ctx, &r, m, &st, "entity/slime/magmacube");
                }
                "slime" => {
                    let Some(r) = setup("slime") else { return };
                    plain(ctx, &r, m, s, "entity/slime/slime");
                    second_model(ctx, &r, m, s, "slime#outer", "entity/slime/slime", WHITE, Pass::Translucent);
                }
                _ => {
                    let (model, tex) = if s.baby || size <= 1.0 { ("sulfur_cube_small", "entity/sulfur_cube/sulfur_cube_outer_small") } else { ("sulfur_cube", "entity/sulfur_cube/sulfur_cube_outer") };
                    let Some(r) = setup(model) else { return };
                    let inner = if model == "sulfur_cube" { "entity/sulfur_cube/sulfur_cube_inner" } else { "entity/sulfur_cube/sulfur_cube_inner_small" };
                    second_model(ctx, &r, m, s, &format!("{model}#inner"), inner, WHITE, Pass::Cutout);
                    second_model(ctx, &r, m, s, model, tex, WHITE, Pass::Translucent);
                }
            }
        }
        "silverfish" => {
            let Some(mut r) = setup("silverfish") else { return };
            anim::silverfish(&mut r, s);
            plain(ctx, &r, default_m, s, "entity/silverfish/silverfish");
        }
        "endermite" => {
            let Some(mut r) = setup("endermite") else { return };
            anim::endermite(&mut r, s);
            plain(ctx, &r, default_m, s, "entity/endermite/endermite");
        }
        "shulker" => {
            let Some(mut r) = setup("shulker") else { return };
            let peek = flags_of("PEEK") as f32 / 100.0;
            anim::shulker(&mut r, s, peek, e.head_yaw);
            let face = match e.field("ATTACH_FACE") {
                Some(DataValue::Direction(d)) => *d,
                _ => 0,
            };
            let turn = attach_rotation(face);
            let body = Mat4::from_translation(pos) * Mat4::from_rotation_y((180.0 - (s.body + 180.0)).to_radians()) * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)) * turn * Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0));
            let m = model_space(body, None);
            let colour = flags_of("COLOR");
            let tex = match DYE_NAMES.get(colour as usize) {
                Some(c) => format!("entity/shulker/shulker_{c}"),
                None => "entity/shulker/shulker".to_string(),
            };
            plain(ctx, &r, m, s, &tex);
        }
        "iron_golem" => {
            let Some(mut r) = setup("iron_golem") else { return };
            anim::iron_golem(&mut r, s, e.attack_ticks(ctx.partial));
            let t = ((s.wp + 6.0) % 13.0 - 6.5).abs();
            let wobble = (t - 3.25) / 3.25 * 6.5;
            let m = if s.ws >= 0.01 { model_space(base * Mat4::from_rotation_z(wobble.to_radians()), None) } else { default_m };
            plain(ctx, &r, m, s, "entity/iron_golem/iron_golem");
            let health = e.float_field("HEALTH").unwrap_or(100.0) / 100.0;
            let crack = if health < 0.25 { Some("high") } else if health < 0.5 { Some("medium") } else if health < 0.75 { Some("low") } else { None };
            if let Some(c) = crack {
                overlay(ctx, &r, m, s, &format!("entity/iron_golem/iron_golem_crackiness_{c}"), WHITE);
            }
        }
        "snow_golem" => {
            let Some(mut r) = setup("snow_golem") else { return };
            anim::snow_golem(&mut r, s);
            plain(ctx, &r, default_m, s, "entity/snow_golem/snow_golem");
            if flags_of("PUMPKIN") & 16 != 0 {
                held::draw_block_on_part(ctx, &r, default_m, s, "carved_pumpkin", "head");
            }
        }
        "phantom" => {
            let Some(mut r) = setup("phantom") else { return };
            anim::phantom(&mut r, s.age + s.id as f32 * 3.0);
            let size = flags_of("ID_SIZE") as f32;
            let k = 1.0 + 0.15 * size;
            let m = model_space(base * Mat4::from_rotation_x(s.pitch.to_radians()), Some(Mat4::from_scale(Vec3::splat(k)) * Mat4::from_translation(Vec3::new(0.0, 1.3125, 0.1875))));
            plain(ctx, &r, m, s, "entity/phantom/phantom");
            glow(ctx, &r, m, s, "entity/phantom/phantom_eyes");
        }
        "wither" => {
            let Some(mut r) = setup("wither") else { return };
            anim::wither(&mut r, s, [(s.body + s.yaw, s.pitch), (s.body + s.yaw, s.pitch)]);
            let inv = flags_of("ID_INV") as f32;
            let k = 2.0 - if inv > 0.0 { inv / 220.0 * 0.5 } else { 0.0 };
            let m = model_space(base, Some(Mat4::from_scale(Vec3::splat(k))));
            let blink = inv > 0.0 && (inv > 80.0 || (inv as i32 / 5) % 2 != 1);
            plain(ctx, &r, m, s, if blink { "entity/wither/wither_invulnerable" } else { "entity/wither/wither" });
            if e.float_field("HEALTH").unwrap_or(300.0) <= 150.0 {
                energy_layer(ctx, &r, m, s, "wither#armor", Pass::EnergyWither, (s.age * 0.02).cos() * 3.0);
            }
        }
        "ender_dragon" => {
            let Some(r) = setup("ender_dragon") else { return };
            let mut r = r;
            draw_dragon_pose(&mut r, s);
            let m = Mat4::from_translation(pos) * Mat4::from_rotation_y((180.0 - e.yaw).to_radians()) * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
            plain(ctx, &r, m, s, "entity/enderdragon/dragon");
            glow(ctx, &r, m, s, "entity/enderdragon/dragon_eyes");
        }
        "warden" => {
            let Some(mut r) = setup("warden") else { return };
            anim::warden(&mut r, s, 0.0);
            plain(ctx, &r, default_m, s, "entity/warden/warden");
            let mut st = *s;
            st.light = [255, 255];
            second_model(ctx, &r, default_m, &st, "warden#bioluminescent", "entity/warden/warden_bioluminescent_layer", WHITE, Pass::Translucent);
            let beat = ((s.age * 0.2).sin() + 1.0) * 0.5;
            let a = |v: f32| [255, 255, 255, (v * 255.0) as u8];
            second_model(ctx, &r, default_m, &st, "warden#pulsating_spots", "entity/warden/warden_pulsating_spots_1", a(beat), Pass::Translucent);
            second_model(ctx, &r, default_m, &st, "warden#heart", "entity/warden/warden_heart", a(1.0 - beat), Pass::Translucent);
        }
        "nautilus" | "zombie_nautilus" => {
            let warm = name == "zombie_nautilus" && variant_name(ctx, "zombie_nautilus_variant", "VARIANT", "temperate") == "warm";
            let model = if warm {
                "zombie_nautilus_coral".to_string()
            } else if s.baby {
                format!("{name}_baby")
            } else {
                name.to_string()
            };
            let Some(mut r) = setup(&model) else { return };
            let body = r.p("body");
            body.rot.y = s.yaw.clamp(-10.0, 10.0) * PI / 180.0;
            body.rot.x = s.pitch.clamp(-10.0, 10.0) * PI / 180.0;
            if let Some(a) = bodies.animation("NautilusAnimation.SWIMMING") {
                r.play_walk(a, s.wp + s.age / 5.0, s.ws + 0.2, 2.0, 3.0);
            }
            let tex = if name == "nautilus" {
                with_baby("entity/nautilus/nautilus", s.baby)
            } else if warm {
                "entity/nautilus/zombie_nautilus_coral".to_string()
            } else {
                "entity/nautilus/zombie_nautilus".to_string()
            };
            plain(ctx, &r, default_m, s, &tex);
            worn(ctx, e, &r, default_m, s, BODY, "nautilus_armor", "nautilus_body");
            worn(ctx, e, &r, default_m, s, SADDLE, "nautilus#saddle", "nautilus_saddle");
        }
        "cod" => {
            let Some(mut r) = setup("cod") else { return };
            anim::fish_tail(&mut r, s, "tail_fin", 0.45, 0.6);
            let m = fish_matrix(base, s, Vec3::new(0.1, 0.1, -0.1), 1.0);
            plain(ctx, &r, model_space(m, None), s, "entity/fish/cod");
        }
        "salmon" => {
            let size = ["salmon_small", "salmon", "salmon_large"][flags_of("TYPE").clamp(0, 2) as usize];
            let Some(mut r) = setup(size) else { return };
            let (a, b) = if s.in_water { (1.0, 1.0) } else { (1.3, 1.7) };
            r.p("body_back").rot.y = -a * 0.25 * (b * 0.6 * s.age).sin();
            let mut m = base * Mat4::from_rotation_y((a * 4.3 * (b * 0.6 * s.age).sin()).to_radians());
            if !s.in_water {
                m = m * Mat4::from_translation(Vec3::new(0.2, 0.1, 0.0)) * Mat4::from_rotation_z(FRAC_PI_2);
            }
            plain(ctx, &r, model_space(m, None), s, "entity/fish/salmon");
        }
        "pufferfish" => {
            let state = flags_of("PUFF_STATE");
            let model = ["pufferfish_small", "pufferfish_medium", "pufferfish_big"][state.clamp(0, 2) as usize];
            let Some(mut r) = setup(model) else { return };
            let v = 0.4 * (s.age * 0.2).sin();
            let (right, left) = if state == 0 { ("right_fin", "left_fin") } else { ("right_blue_fin", "left_blue_fin") };
            r.p(right).rot.z = -0.2 + v;
            r.p(left).rot.z = 0.2 - v;
            let m = Mat4::from_translation(Vec3::new(0.0, (s.age * 0.05).cos() * 0.08, 0.0)) * base;
            plain(ctx, &r, model_space(m, None), s, "entity/fish/pufferfish");
        }
        "tropical_fish" => {
            let v = flags_of("ID_TYPE_VARIANT");
            let large = v & 255 != 0;
            let pattern = (v >> 8) & 255;
            let base_colour = (v >> 16) & 255;
            let pattern_colour = (v >> 24) & 255;
            let (model, letter) = if large { ("tropical_fish_large", "b") } else { ("tropical_fish_small", "a") };
            let Some(mut r) = setup(model) else { return };
            anim::fish_tail(&mut r, s, "tail", 0.45, 0.6);
            let m = model_space(fish_matrix(base, s, Vec3::new(0.2, 0.1, 0.0), 1.0), None);
            let dye = |i: i32| rgb(DYE_COLORS[i.clamp(0, 15) as usize]);
            let sprite = ctx.sprite(&format!("entity/fish/tropical_{letter}")).unwrap_or(ctx.bodies.sheet.missing);
            if !s.invisible {
                draw_layer(ctx, &r, m, sprite, s, dye(base_colour), Pass::Cutout, true);
            }
            second_model(ctx, &r, m, s, &format!("{model}#pattern"), &format!("entity/fish/tropical_{letter}_pattern_{}", pattern + 1), dye(pattern_colour), Pass::Cutout);
        }
        "parrot" => {
            let Some(mut r) = setup("parrot") else { return };
            let flying = !e.on_ground;
            let flap = if flying { ((s.age * 1.2).sin() + 1.0) * 0.5 } else { 0.0 };
            // Bit 0 of a tame animal's flags: told to sit.
            if e.int_field("FLAGS_ID").is_some_and(|f| f & 1 != 0) {
                anim::parrot_sitting(&mut r, s);
            } else {
                anim::parrot(&mut r, s, flying, flap);
            }
            let c = ["red_blue", "blue", "green", "yellow_blue", "grey"][flags_of("VARIANT").clamp(0, 4) as usize];
            plain(ctx, &r, default_m, s, &format!("entity/parrot/parrot_{c}"));
        }
        "strider" => {
            let Some(mut r) = setup(if s.baby { "strider_baby" } else { "strider" }) else { return };
            anim::strider(&mut r, s, !e.passengers.is_empty());
            let cold = e.bool_field("SUFFOCATING");
            plain(ctx, &r, default_m, s, &with_baby(if cold { "entity/strider/strider_cold" } else { "entity/strider/strider" }, s.baby));
            worn(ctx, e, &r, default_m, s, SADDLE, "strider#saddle", "strider_saddle");
        }
        "sniffer" => {
            let Some(mut r) = setup(if s.baby { "sniffer_baby" } else { "sniffer" }) else { return };
            anim::look(&mut r, s);
            if let Some(a) = bodies.animation("SnifferAnimation.SNIFFER_WALK") {
                r.play_walk(a, s.wp, s.ws, 9.0, 100.0);
            }
            if s.baby {
                if let Some(a) = bodies.animation("SnifferAnimation.BABY_TRANSFORM") {
                    r.play(a, 0.0, 1.0);
                }
            }
            plain(ctx, &r, default_m, s, if s.baby { "entity/sniffer/snifflet" } else { "entity/sniffer/sniffer" });
        }
        "armadillo" => {
            let Some(mut r) = setup(if s.baby { "armadillo_baby" } else { "armadillo" }) else { return };
            let state = flags_of("ARMADILLO_STATE");
            let hiding = state == 1 || state == 2;
            r.p("body").skip_draw = hiding;
            for part in ["left_hind_leg", "right_hind_leg", "tail"] {
                r.p(part).visible = !hiding;
            }
            r.p("cube").visible = hiding;
            if !hiding {
                let h = r.p("head");
                h.rot.x = s.pitch.clamp(-22.5, 25.0) * PI / 180.0;
                h.rot.y = s.yaw.clamp(-32.5, 32.5) * PI / 180.0;
                let w = if s.baby { "BabyArmadilloAnimation.ARMADILLO_BABY_WALK" } else { "ArmadilloAnimation.ARMADILLO_WALK" };
                if let Some(a) = bodies.animation(w) {
                    r.play_walk(a, s.wp, s.ws, 16.5, 2.5);
                }
            }
            plain(ctx, &r, default_m, s, &with_baby("entity/armadillo/armadillo", s.baby));
        }
        other => {
            // An entity this table doesn't know: draw its shape if the name matches one.
            let Some(r) = setup(other) else { return };
            plain(ctx, &r, default_m, s, &format!("entity/{other}/{other}"));
        }
    }
}

fn look_only(r: &mut Rig, s: &St) {
    anim::look(r, s);
}

/// Fish wiggle, and lie on their side out of water.
fn fish_matrix(base: Mat4, s: &St, out_of_water: Vec3, k: f32) -> Mat4 {
    let mut m = base * Mat4::from_rotation_y((k * 4.3 * (0.6 * s.age).sin()).to_radians());
    if !s.in_water {
        m = m * Mat4::from_translation(out_of_water) * Mat4::from_rotation_z(FRAC_PI_2);
    }
    m
}

/// The colour of a dye, by its name.
pub fn dye_color(name: &str) -> u32 {
    DYE_NAMES.iter().position(|n| *n == name).map_or(0xFFFFFF, |i| DYE_COLORS[i])
}

pub const DYE_NAMES: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];

fn sheep_color(dye: usize) -> [u8; 4] {
    if dye == 0 {
        // Unmodified white wool is a shade off white.
        return [0xE6, 0xE6, 0xE6, 255];
    }
    let c = DYE_COLORS[dye.min(15)];
    let scale = |v: u32| ((v & 255) as f32 * 0.75) as u8;
    [scale(c >> 16), scale(c >> 8), scale(c), 255]
}

/// Turns a shulker so its base sits on the face it's stuck to. Directions: down, up, north, south, west, east.
fn attach_rotation(face: i32) -> Mat4 {
    let opposite = [1, 0, 3, 2, 5, 4][face.clamp(0, 5) as usize];
    match opposite {
        0 => Mat4::from_rotation_x(PI),
        1 => Mat4::IDENTITY,
        2 => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(PI),
        3 => Mat4::from_rotation_x(FRAC_PI_2),
        4 => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(FRAC_PI_2),
        _ => Mat4::from_rotation_x(FRAC_PI_2) * Mat4::from_rotation_z(-FRAC_PI_2),
    }
}

/// The dragon flaps and sways; its long neck and tail follow from a chain of parts.
fn draw_dragon_pose(r: &mut Rig, s: &St) {
    let flap = (s.age * 0.04).fract();
    let v2 = flap * std::f32::consts::TAU;
    r.p("jaw").rot.x = (v2.sin() + 1.0) * 0.2;
    let v3 = (v2 - 1.0).sin() + 1.0;
    let v3 = (v3 * v3 + v3 * 2.0) * 0.05;
    let root = r.p("root");
    root.pos.y = (v3 - 2.0) * 16.0;
    root.pos.z = -48.0;
    root.rot.x = v3 * 2.0 * PI / 180.0;
    let start = r.get("neck0").pos;
    let (mut x, mut y, mut z) = (start.x, start.y, start.z);
    for i in 0..5 {
        let name = ["neck0", "neck1", "neck2", "neck3", "neck4"][i];
        let p = r.p(name);
        let wave = (i as f32 * 0.45 + v2).cos() * 0.15;
        p.rot.x = wave;
        p.rot.y = 0.0;
        p.rot.z = 0.0;
        p.pos = Vec3::new(x, y, z);
        x -= p.rot.y.sin() * p.rot.x.cos() * 10.0;
        y += p.rot.x.sin() * 10.0;
        z -= p.rot.y.cos() * p.rot.x.cos() * 10.0;
    }
    let h = r.p("head");
    h.pos = Vec3::new(x, y, z);
    h.rot = Vec3::ZERO;
    let wing_x = 0.125 - v2.cos() * 0.2;
    let wing_z = -(v2.sin() + 0.125) * 0.8;
    let tip_z = ((v2 + 2.0).sin() + 0.5) * 0.75;
    let w = r.p("left_wing");
    w.rot = Vec3::new(wing_x, -0.25, wing_z);
    r.p("left_wing_tip").rot.z = tip_z;
    let w = r.p("right_wing");
    w.rot = Vec3::new(wing_x, 0.25, -wing_z);
    r.p("right_wing_tip").rot.z = -tip_z;
    for side in ["left", "right"] {
        r.p(&format!("{side}_front_leg")).rot.x = 1.0 + v3 * 0.1;
        r.p(&format!("{side}_front_leg_tip")).rot.x = 0.5 + v3 * 0.1;
        r.p(&format!("{side}_front_foot")).rot.x = 0.75 + v3 * 0.1;
        r.p(&format!("{side}_hind_leg")).rot.x = 1.0 + v3 * 0.1;
        r.p(&format!("{side}_hind_leg_tip")).rot.x = -0.5 - v3 * 0.1;
        r.p(&format!("{side}_hind_foot")).rot.x = 0.75 + v3 * 0.1;
    }
    let start = r.get("tail0").pos;
    let (mut x, mut y, mut z) = (start.x, start.y, start.z);
    let mut sway = 0.0f32;
    for i in 0..12 {
        let name = format!("tail{i}");
        sway += (i as f32 * 0.45 + v2).sin() * 0.05;
        let p = r.p(&name);
        p.rot = Vec3::new(sway, PI, 0.0);
        p.pos = Vec3::new(x, y, z);
        y += p.rot.x.sin() * 10.0;
        z -= p.rot.y.cos() * p.rot.x.cos() * 10.0;
        x -= p.rot.y.sin() * p.rot.x.cos() * 10.0;
    }
}
