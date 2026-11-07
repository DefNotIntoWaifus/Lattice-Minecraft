//! Turns what happens in the world into particles and the odd sound: blocks breaking, things
//! burning, torches smoking, hearts over a fed animal.

use super::particles::{Opts, Particles};
use super::{Session, SoundCue};
use crate::assets::GameAssets;
use glam::{DVec3, IVec3};
use lattice_proto::{DataValue, Reader};
use rand::Rng;

fn argb_to_rgb(v: i32) -> [f32; 3] {
    let c = v as u32;
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// Reads the options a particle came with.
pub fn options(name: &str, raw: &[u8], assets: &GameAssets) -> Opts {
    let mut o = Opts::default();
    let mut r = Reader::new(raw);
    match name {
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" => {
            if let Ok(state) = r.var_i32() {
                o.block = Some(block_bits(assets, state as u32));
            }
        }
        "dust" => {
            if let (Ok(c), Ok(s)) = (r.i32(), r.f32()) {
                o.color = Some(argb_to_rgb(c));
                o.scale = Some(s);
            }
        }
        "dust_color_transition" => {
            if let (Ok(c), Ok(_), Ok(s)) = (r.i32(), r.i32(), r.f32()) {
                o.color = Some(argb_to_rgb(c));
                o.scale = Some(s);
            }
        }
        "entity_effect" | "tinted_leaves" | "flash" | "effect" | "instant_effect" => {
            if let Ok(c) = r.i32() {
                o.color = Some(argb_to_rgb(c));
                o.alpha = ((c as u32) >> 24) as f32 / 255.0;
                o.alpha_given = o.alpha < 1.0 && o.alpha > 0.0;
            }
        }
        "item" => {
            if let Ok(stack) = lattice_proto::ItemStack::read_template(&mut r) {
                let name = crate::game::items::item_name(&stack);
                o.item = assets.atlas_item.get(name).copied();
            }
        }
        _ => {}
    }
    o
}

/// The sprite and colour of a block's flying bits.
fn block_bits(assets: &GameAssets, state: u32) -> (lattice_render::atlas::Sprite, [f32; 3]) {
    let models = &assets.mesh.models;
    let sprite = models.particle(state);
    let tinted = models.pick(state, 0).any(|m| m.quads.iter().any(|q| q.tint >= 0));
    let name = lattice_data::game().block(state).name.as_str();
    // Grass block bits are the dirt under it; leaves and plants are green.
    let tint = if tinted && name != "grass_block" { [0.49, 0.74, 0.42] } else { [1.0, 1.0, 1.0] };
    (sprite, tint)
}

/// One game tick: what the session gathered, and what simply happens nearby.
pub fn tick(session: &mut Session, particles: &mut Particles, assets: &GameAssets) {
    let game = lattice_data::game();
    let names = game.registry("particle_type");
    for req in session.particle_requests.drain(..) {
        let Some(name) = names.get(req.particle.kind as usize) else { continue };
        let opts = options(name, &req.particle.raw, assets);
        particles.spawn(name, req.pos, req.vel, &opts);
    }
    let broke: Vec<_> = session.player.broke.drain(..).chain(session.broke_elsewhere.drain(..)).collect();
    for (pos, state) in broke {
        let (sprite, tint) = block_bits(assets, state);
        let boxes: Vec<([f64; 3], [f64; 3])> = game.outline(state).iter().map(|b| (b.min, b.max)).collect();
        particles.break_block(IVec3::new(pos.x, pos.y, pos.z), sprite, tint, &boxes);
    }
    // A spray as the player hits the water.
    if let Some(v) = session.player.splashed.take() {
        let b = &session.player.body;
        let top = b.pos.y.floor() + 1.0;
        let mut rng = rand::rng();
        for _ in 0..13 {
            let (dx, dz) = (rng.random_range(-0.6..0.6), rng.random_range(-0.6..0.6));
            particles.spawn("bubble", DVec3::new(b.pos.x + dx, top, b.pos.z + dz), DVec3::new(v.x, v.y - rng.random::<f64>() * 0.2, v.z), &Opts::default());
            let (dx, dz) = (rng.random_range(-0.6..0.6), rng.random_range(-0.6..0.6));
            particles.spawn("splash", DVec3::new(b.pos.x + dx, top, b.pos.z + dz), v, &Opts::default());
        }
    }
    let Some(world) = session.world.as_ref() else {
        return;
    };
    // A bit knocked loose from the block being mined, each tick.
    let mut rng = rand::rng();
    if let Some((pos, _)) = session.player.dig_progress() {
        if let Some(state) = world.block_at(pos) {
            if let Some(hit) = session.player.target {
                let (sprite, tint) = block_bits(assets, state);
                let (dx, dy, dz) = match hit.face {
                    0 => (0.0, -0.1, 0.0),
                    1 => (0.0, 0.1, 0.0),
                    2 => (0.0, 0.0, -0.1),
                    3 => (0.0, 0.0, 0.1),
                    4 => (-0.1, 0.0, 0.0),
                    _ => (0.1, 0.0, 0.0),
                };
                let at = |v: i32, d: f64, r: f64| if d != 0.0 { v as f64 + if d > 0.0 { 1.1 } else { -0.1 } } else { v as f64 + 0.1 + r * 0.8 };
                let p = DVec3::new(at(pos.x, dx, rng.random()), at(pos.y, dy, rng.random()), at(pos.z, dz, rng.random()));
                particles.block_bit(p, DVec3::ZERO, sprite, tint);
            }
        }
    }
    // Sprinting kicks up the ground.
    let body = &session.player.body;
    if body.on_ground && session.player.is_sprinting() && !body.in_water {
        let p = body.pos;
        if let Some(under) = world.block(p.x.floor() as i32, (p.y - 0.2).floor() as i32, p.z.floor() as i32) {
            if !game.state(under).has(lattice_data::flags::AIR) {
                let (sprite, tint) = block_bits(assets, under);
                let at = p + DVec3::new((rng.random::<f64>() - 0.5) * 0.6, 0.1, (rng.random::<f64>() - 0.5) * 0.6);
                particles.block_bit(at, DVec3::new(body.vel.x * -4.0, 1.5, body.vel.z * -4.0), sprite, tint);
            }
        }
    }
    // Hit sparks and entity events.
    let me = session.player.entity_id;
    let my_pos = session.player.body.pos;
    let events: Vec<(i32, i32)> = session.entities.events.drain(..).collect();
    for (id, kind) in events {
        // The player isn't among the entities they see.
        let (pos, (w, h)) = match session.entities.map.get(&id) {
            Some(e) => (e.pos, e.dimensions()),
            None if id == me => (my_pos, (0.6, 1.8)),
            None => continue,
        };
        let around = |r: &mut rand::rngs::ThreadRng| pos + DVec3::new((r.random::<f64>() - 0.5) * w, r.random::<f64>() * h, (r.random::<f64>() - 0.5) * w);
        // What only makes a sound.
        match kind {
            // Tools and armour wearing out.
            47..=52 | 65 => session.cues.push(SoundCue::around("entity.item.break", crate::audio::PLAYERS, pos, 0.8, 0.9)),
            29 => session.cues.push(SoundCue::around("item.shield.block", crate::audio::PLAYERS, pos, 1.0, 1.0)),
            30 => session.cues.push(SoundCue::around("item.shield.break", crate::audio::PLAYERS, pos, 0.8, 1.0)),
            _ => {}
        }
        // A totem saving someone: a burst of green and gold, and its picture for whoever it saved.
        if kind == 35 {
            session.cues.push(SoundCue::around("item.totem.use", crate::audio::PLAYERS, pos, 1.0, 1.0));
            for _ in 0..64 {
                let v = DVec3::new((rng.random::<f64>() - 0.5) * 0.8, rng.random::<f64>() * 0.6 + 0.2, (rng.random::<f64>() - 0.5) * 0.8);
                particles.spawn("totem_of_undying", around(&mut rng), v, &Opts::default());
            }
            if id == me {
                session.totem_shown = Some(std::time::Instant::now());
            }
            continue;
        }
        let (name, count, up): (&str, u32, f64) = match kind {
            // Teleporting.
            46 => ("portal", 64, 0.0),
            // Melee hits.
            -4 => ("crit", 16, 0.2),
            -5 => ("enchanted_hit", 16, 0.2),
            // Dying.
            60 | 20 => ("poof", 20, 0.0),
            // Love and anger.
            18 | 7 | 12 => ("heart", 7, 0.0),
            6 | 13 => ("smoke", 7, 0.0),
            14 => ("happy_villager", 7, 0.0),
            _ => continue,
        };
        for _ in 0..count {
            let v = DVec3::new((rng.random::<f64>() - 0.5) * 0.04, up + (rng.random::<f64>() - 0.5) * 0.04, (rng.random::<f64>() - 0.5) * 0.04);
            particles.spawn(name, around(&mut rng), v, &Opts::default());
        }
    }
    // Swirls around things under an effect.
    for e in session.entities.map.values() {
        let Some(DataValue::Particles(list)) = e.field("EFFECT_PARTICLES") else { continue };
        if list.is_empty() {
            continue;
        }
        let ambient = e.bool_field("EFFECT_AMBIENCE");
        if rng.random_range(0..if ambient { 15 } else { 4 }) != 0 {
            continue;
        }
        let pick = &list[rng.random_range(0..list.len())];
        let Some(name) = names.get(pick.kind as usize) else { continue };
        let (w, h) = e.dimensions();
        let at = e.pos + DVec3::new((rng.random::<f64>() - 0.5) * w, rng.random::<f64>() * h, (rng.random::<f64>() - 0.5) * w);
        let opts = options(name, &pick.raw, assets);
        particles.spawn(name, at, DVec3::new(1.0, 1.0, 1.0), &opts);
    }
    ambient_blocks(session, particles);
}

/// What blocks get up to on their own near the player: torches smoke, lava pops.
fn ambient_blocks(session: &mut Session, particles: &mut Particles) {
    let Some(world) = session.world.as_ref() else { return };
    let game = lattice_data::game();
    let p = session.player.body.pos;
    let (cx, cy, cz) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    let mut rng = rand::rng();
    let none = Opts::default();
    let mut cues: Vec<SoundCue> = Vec::new();
    for attempt in 0..1334 {
        let range = if attempt % 2 == 0 { 16 } else { 32 };
        let x = cx + rng.random_range(-range..range);
        let y = cy + rng.random_range(-range..range);
        let z = cz + rng.random_range(-range..range);
        let Some(state) = world.block(x, y, z) else { continue };
        let info = game.state(state);
        if info.has(lattice_data::flags::AIR) {
            continue;
        }
        let name = game.block(state).name.as_str();
        let (fx, fy, fz) = (x as f64, y as f64, z as f64);
        let at = DVec3::new(fx + 0.5, fy + 0.7, fz + 0.5);
        let has = |key: &str, value: &str| game.properties(state).iter().any(|(k, v)| *k == key && *v == value);
        match name {
            "torch" | "soul_torch" | "copper_torch" => {
                particles.spawn("smoke", at, DVec3::ZERO, &none);
                let flame = if name == "soul_torch" { "soul_fire_flame" } else if name == "copper_torch" { "copper_fire_flame" } else { "flame" };
                particles.spawn(flame, at, DVec3::ZERO, &none);
            }
            "wall_torch" | "soul_wall_torch" | "copper_wall_torch" => {
                let (dx, dz) = if has("facing", "north") {
                    (0.0, 1.0)
                } else if has("facing", "south") {
                    (0.0, -1.0)
                } else if has("facing", "west") {
                    (1.0, 0.0)
                } else {
                    (-1.0, 0.0)
                };
                let wall = DVec3::new(fx + 0.5 + dx * 0.27, fy + 0.92, fz + 0.5 + dz * 0.27);
                particles.spawn("smoke", wall, DVec3::ZERO, &none);
                let flame = if name == "soul_wall_torch" { "soul_fire_flame" } else if name == "copper_wall_torch" { "copper_fire_flame" } else { "flame" };
                particles.spawn(flame, wall, DVec3::ZERO, &none);
            }
            "redstone_torch" if has("lit", "true") => {
                let opts = Opts { color: Some([1.0, 0.0, 0.0]), scale: Some(1.0), ..Default::default() };
                particles.spawn("dust", DVec3::new(fx + 0.5 + (rng.random::<f64>() - 0.5) * 0.2, fy + 0.7 + (rng.random::<f64>() - 0.5) * 0.2, fz + 0.5 + (rng.random::<f64>() - 0.5) * 0.2), DVec3::ZERO, &opts);
            }
            "fire" | "soul_fire" => {
                if rng.random_range(0..24) == 0 {
                    cues.push(SoundCue::at("block.fire.ambient", crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 1.0 + rng.random::<f32>(), rng.random::<f32>() * 0.7 + 0.3));
                }
                particles.spawn("large_smoke", DVec3::new(fx + rng.random::<f64>(), fy + rng.random::<f64>() * 0.5 + 0.5, fz + rng.random::<f64>()), DVec3::ZERO, &none);
            }
            "lava" => {
                let open = world.block(x, y + 1, z).is_some_and(|s| game.state(s).has(lattice_data::flags::AIR));
                if open && info.has(lattice_data::flags::FLUID_SOURCE) {
                    if rng.random_range(0..100) == 0 {
                        particles.spawn("lava", DVec3::new(fx + rng.random::<f64>(), fy + 1.0, fz + rng.random::<f64>()), DVec3::ZERO, &none);
                        cues.push(SoundCue::at("block.lava.pop", crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 0.2 + rng.random::<f32>() * 0.2, 0.9 + rng.random::<f32>() * 0.15));
                    }
                    if rng.random_range(0..200) == 0 {
                        cues.push(SoundCue::at("block.lava.ambient", crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 0.2 + rng.random::<f32>() * 0.2, 0.9 + rng.random::<f32>() * 0.15));
                    }
                }
            }
            "furnace" | "blast_furnace" | "smoker" if has("lit", "true") => {
                let event = match name {
                    "furnace" => "block.furnace.fire_crackle",
                    "blast_furnace" => "block.blastfurnace.fire_crackle",
                    _ => "block.smoker.smoke",
                };
                if rng.random_range(0..10) == 0 {
                    cues.push(SoundCue::at(event, crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 1.0, 1.0));
                }
                let (dx, dz) = if has("facing", "north") {
                    (0.0, -1.0)
                } else if has("facing", "south") {
                    (0.0, 1.0)
                } else if has("facing", "west") {
                    (-1.0, 0.0)
                } else {
                    (1.0, 0.0)
                };
                let jitter = rng.random::<f64>() * 0.6 - 0.3;
                let front = DVec3::new(fx + 0.5 + dx * 0.52 + if dz != 0.0 { jitter } else { 0.0 }, fy + rng.random::<f64>() * 6.0 / 16.0, fz + 0.5 + dz * 0.52 + if dx != 0.0 { jitter } else { 0.0 });
                particles.spawn("smoke", front, DVec3::ZERO, &none);
                if name != "smoker" {
                    particles.spawn("flame", front, DVec3::ZERO, &none);
                }
            }
            "campfire" | "soul_campfire" if has("lit", "true") => {
                if rng.random_range(0..10) == 0 {
                    cues.push(SoundCue::at("block.campfire.crackle", crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 0.5 + rng.random::<f32>(), rng.random::<f32>() * 0.7 + 0.6));
                }
                if rng.random_range(0..5) == 0 {
                    particles.spawn("lava", DVec3::new(fx + 0.5, fy + 0.5, fz + 0.5), DVec3::ZERO, &none);
                }
                let smoke = if has("signal_fire", "true") { "campfire_signal_smoke" } else { "campfire_cosy_smoke" };
                particles.spawn(smoke, DVec3::new(fx + 0.5 + (rng.random::<f64>() - 0.5) / 3.0, fy + rng.random::<f64>() + 0.5, fz + 0.5 + (rng.random::<f64>() - 0.5) / 3.0), DVec3::new(0.0, 0.07, 0.0), &none);
            }
            "nether_portal" => {
                if rng.random_range(0..100) == 0 {
                    cues.push(SoundCue::at("block.portal.ambient", crate::audio::BLOCKS, lattice_proto::BlockPos::new(x, y, z), 0.5, rng.random::<f32>() * 0.4 + 0.8));
                }
                for _ in 0..4 {
                    let start = DVec3::new(fx + rng.random::<f64>(), fy + rng.random::<f64>(), fz + rng.random::<f64>());
                    let v = DVec3::new((rng.random::<f64>() - 0.5) * 0.5, (rng.random::<f64>() - 0.5) * 0.5, (rng.random::<f64>() - 0.5) * 0.5);
                    particles.spawn("portal", start, v * 4.0, &none);
                }
            }
            "end_rod" => {
                if rng.random_range(0..5) == 0 {
                    let v = DVec3::new((rng.random::<f64>() - rng.random::<f64>()) * 0.005, (rng.random::<f64>() - rng.random::<f64>()) * 0.005, (rng.random::<f64>() - rng.random::<f64>()) * 0.005);
                    particles.spawn("end_rod", DVec3::new(fx + 0.5 + v.x * 40.0, fy + 0.5 + v.y * 40.0, fz + 0.5 + v.z * 40.0), v, &none);
                }
            }
            n if n.ends_with("candle") && has("lit", "true") => {
                if rng.random_range(0..4) == 0 {
                    particles.spawn("small_flame", DVec3::new(fx + 0.5, fy + 0.6, fz + 0.5), DVec3::ZERO, &none);
                    if rng.random_range(0..3) == 0 {
                        particles.spawn("smoke", DVec3::new(fx + 0.5, fy + 0.65, fz + 0.5), DVec3::ZERO, &none);
                    }
                }
            }
            "mycelium" if rng.random_range(0..10) == 0 => {
                particles.spawn("mycelium", DVec3::new(fx + rng.random::<f64>(), fy + 1.1, fz + rng.random::<f64>()), DVec3::ZERO, &none);
            }
            n if n.ends_with("_leaves") && n != "cherry_leaves" && n != "pale_oak_leaves" => {
                // Water dripping through leaves in the rain.
                if session.rain_level > 0.2 && rng.random_range(0..15) == 0 {
                    let below = world.block(x, y - 1, z).unwrap_or(0);
                    if !game.state(below).has(lattice_data::flags::SOLID) {
                        particles.spawn("dripping_water", DVec3::new(fx + rng.random::<f64>(), fy - 0.05, fz + rng.random::<f64>()), DVec3::ZERO, &none);
                    }
                }
            }
            "cherry_leaves" | "pale_oak_leaves" => {
                if rng.random_range(0..10) == 0 && world.block(x, y - 1, z).is_some_and(|s| game.state(s).has(lattice_data::flags::AIR)) {
                    particles.spawn(name, DVec3::new(fx + rng.random::<f64>(), fy - 0.05, fz + rng.random::<f64>()), DVec3::ZERO, &none);
                }
            }
            _ => {}
        }
    }
    session.cues.append(&mut cues);
}
