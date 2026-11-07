//! Particles: motes, smoke, sparks and bits of broken block. Each kind's numbers follow the
//! game's particle classes; the pictures are the player's own, listed in `particles/*.json`.

use glam::{DVec3, Vec3};
use lattice_render::atlas::Sprite;
use rand::Rng;
use rand::rngs::SmallRng;
use rand::SeedableRng;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// A named set of pictures: index, and how they're chosen.
    Set(u16, Frames),
    /// A corner of a block's own texture, from the block atlas.
    Block([f32; 4]),
    /// A corner of an item's picture, from the block atlas.
    Item([f32; 4]),
}

#[derive(Clone, Copy, PartialEq)]
enum Frames {
    /// Runs through the pictures over its life.
    ByAge,
    /// Keeps one it picked at the start.
    Fixed(u16),
}

pub struct Mote {
    pos: DVec3,
    prev: DVec3,
    vel: DVec3,
    age: u32,
    life: u32,
    size: f32,
    gravity: f32,
    friction: f32,
    color: [f32; 3],
    alpha: f32,
    look: Look,
    collide: bool,
    on_ground: bool,
    bright: bool,
    /// Shrinks over its life, as flames and hearts do, by the given kind.
    shrink: Shrink,
    fade: bool,
    /// Drawn with blending rather than cut out.
    translucent: bool,
    /// Pulled towards this point, as enchanting and portal motes are.
    target: Option<DVec3>,
}

#[derive(Clone, Copy, PartialEq)]
enum Shrink {
    None,
    Flame,
    Smoke,
    Heart,
}

#[derive(Default)]
pub struct Particles {
    motes: Vec<Mote>,
    /// Picture lists by set, loaded once.
    sets: Vec<Vec<Sprite>>,
    by_name: HashMap<String, u16>,
    rng: Option<SmallRng>,
}

const MAX_MOTES: usize = 8192;

impl Particles {
    /// Reads which pictures each kind uses.
    pub fn load(&mut self, list: &[(String, Vec<Sprite>)]) {
        self.sets.clear();
        self.by_name.clear();
        for (name, sprites) in list {
            self.by_name.insert(name.clone(), self.sets.len() as u16);
            self.sets.push(sprites.clone());
        }
    }

    pub fn clear(&mut self) {
        self.motes.clear();
    }

    pub fn count(&self) -> usize {
        self.motes.len()
    }

    fn rng(&mut self) -> &mut SmallRng {
        self.rng.get_or_insert_with(|| SmallRng::seed_from_u64(0x5EED))
    }

    fn set_look(&mut self, name: &str, by_age: bool) -> Option<Look> {
        let i = *self.by_name.get(name)?;
        let count = self.sets[i as usize].len().max(1);
        if by_age {
            Some(Look::Set(i, Frames::ByAge))
        } else {
            let f = self.rng().random_range(0..count) as u16;
            Some(Look::Set(i, Frames::Fixed(f)))
        }
    }

    fn base(&mut self, pos: DVec3, vel: DVec3, look: Look) -> Mote {
        let r = self.rng();
        let life = (4.0 / (r.random::<f32>() * 0.9 + 0.1)) as u32;
        let size = 0.1 * (r.random::<f32>() * 0.5 + 0.5) * 2.0;
        Mote {
            pos,
            prev: pos,
            vel,
            age: 0,
            life,
            size,
            gravity: 0.0,
            friction: 0.98,
            color: [1.0, 1.0, 1.0],
            alpha: 1.0,
            look,
            collide: true,
            on_ground: false,
            bright: false,
            shrink: Shrink::None,
            fade: false,
            translucent: false,
            target: None,
        }
    }

    /// The default start speed the game gives particles that don't set their own.
    fn scatter(&mut self, v: DVec3) -> DVec3 {
        let r = self.rng();
        let mut x = v.x + (r.random::<f64>() * 2.0 - 1.0) * 0.4;
        let mut y = v.y + (r.random::<f64>() * 2.0 - 1.0) * 0.4;
        let mut z = v.z + (r.random::<f64>() * 2.0 - 1.0) * 0.4;
        let f = (r.random::<f64>() + r.random::<f64>() + 1.0) * 0.15;
        let len = (x * x + y * y + z * z).sqrt().max(1e-6);
        x = x / len * f * 0.4;
        y = y / len * f * 0.4 + 0.1;
        z = z / len * f * 0.4;
        DVec3::new(x, y, z)
    }

    fn push(&mut self, m: Mote) {
        if self.motes.len() >= MAX_MOTES {
            self.motes.remove(0);
        }
        self.motes.push(m);
    }

    /// Bits of a block's texture flying out.
    pub fn block_bit(&mut self, pos: DVec3, vel: DVec3, sprite: Sprite, tint: [f32; 3]) {
        let scattered = self.scatter(vel);
        let (uo, vo) = {
            let r = self.rng();
            (r.random::<f32>() * 3.0, r.random::<f32>() * 3.0)
        };
        let du = (sprite.u1 - sprite.u0) / 4.0;
        let dv = (sprite.v1 - sprite.v0) / 4.0;
        let rect = [sprite.u0 + uo * du, sprite.v0 + vo * dv, sprite.u0 + (uo + 1.0) * du, sprite.v0 + (vo + 1.0) * dv];
        let mut m = self.base(pos, scattered, Look::Block(rect));
        m.gravity = 1.0;
        m.color = [0.6 * tint[0], 0.6 * tint[1], 0.6 * tint[2]];
        m.size /= 2.0;
        self.push(m);
    }

    /// The shower from a block breaking.
    pub fn break_block(&mut self, pos: glam::IVec3, sprite: Sprite, tint: [f32; 3], boxes: &[([f64; 3], [f64; 3])]) {
        let origin = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64);
        for (min, max) in boxes {
            let (sx, sy, sz) = (max[0] - min[0], max[1] - min[1], max[2] - min[2]);
            let count = |s: f64| ((s / 0.25).ceil() as i32).clamp(1, 4);
            let (nx, ny, nz) = (count(sx), count(sy), count(sz));
            for ix in 0..nx {
                for iy in 0..ny {
                    for iz in 0..nz {
                        let fx = (ix as f64 + 0.5) / nx as f64;
                        let fy = (iy as f64 + 0.5) / ny as f64;
                        let fz = (iz as f64 + 0.5) / nz as f64;
                        let p = origin + DVec3::new(fx * sx + min[0], fy * sy + min[1], fz * sz + min[2]);
                        self.block_bit(p, DVec3::new(fx - 0.5, fy - 0.5, fz - 0.5), sprite, tint);
                    }
                }
            }
        }
    }

    /// Spawns one particle by the game's name for it, with its start speed.
    pub fn spawn(&mut self, name: &str, pos: DVec3, v: DVec3, opts: &Opts) {
        let (r1, r2, r3) = {
            let r = self.rng();
            (r.random::<f32>(), r.random::<f32>(), r.random::<f32>())
        };
        match name {
            "block" | "block_crumble" | "dust_pillar" | "falling_dust" | "block_marker" => {
                if let Some((sprite, tint)) = opts.block {
                    if name == "block_marker" {
                        return;
                    }
                    self.block_bit(pos, v, sprite, tint);
                }
            }
            "item" | "item_slime" | "item_snowball" | "item_cobweb" => {
                let Some(sprite) = opts.item else { return };
                let (uo, vo) = {
                    let r = self.rng();
                    (r.random::<f32>() * 3.0, r.random::<f32>() * 3.0)
                };
                let (du, dv) = ((sprite.u1 - sprite.u0) / 4.0, (sprite.v1 - sprite.v0) / 4.0);
                let rect = [sprite.u0 + uo * du, sprite.v0 + vo * dv, sprite.u0 + (uo + 1.0) * du, sprite.v0 + (vo + 1.0) * dv];
                let vel = self.scatter(v) * 0.1 + v * 1.0;
                let mut m = self.base(pos, vel, Look::Item(rect));
                m.gravity = 1.0;
                m.color = [1.0, 1.0, 1.0];
                m.size /= 2.0;
                self.push(m);
            }
            "smoke" | "large_smoke" | "white_smoke" | "ash" | "white_ash" | "squid_ink" | "glow_squid_ink" | "campfire_cosy_smoke" | "campfire_signal_smoke" | "spit" | "dust_plume" | "cloud" | "sneeze" | "poof" | "firefly" | "gust" | "small_gust" | "sculk_charge_pop" | "sonic_boom" | "flash" | "explosion" | "wax_on" | "wax_off" | "electric_spark" | "scrape" | "crimson_spore" | "warped_spore" | "mycelium" | "sulfur_bubbles" | "noxious_gas" | "noxious_gas_cloud" | "geyser_poof" | "geyser_plume" | "geyser_base" => {
                let Some(look) = self.set_look(name, true) else { return };
                let mut m = self.base(pos, v, look);
                m.friction = 0.96;
                m.gravity = -0.1;
                m.shrink = Shrink::Smoke;
                let scale = match name {
                    "large_smoke" => 2.5,
                    "campfire_cosy_smoke" | "campfire_signal_smoke" => 3.0,
                    "poof" | "cloud" | "sneeze" => 2.0,
                    "explosion" => 2.0,
                    "sonic_boom" => 3.0,
                    "gust" => 2.0,
                    _ => 1.0,
                };
                m.size = 0.1 * 0.75 * scale;
                m.life = ((8.0 / (r1 * 0.8 + 0.2)) * scale.min(2.0)) as u32;
                let g = r2 * 0.3 + 0.1;
                m.color = match name {
                    "smoke" | "large_smoke" => [g, g, g],
                    "white_smoke" => [1.0, 1.0, 1.0],
                    "ash" | "white_ash" | "cloud" | "poof" | "sneeze" => [0.8 + r2 * 0.2; 3],
                    "squid_ink" => [0.0, 0.0, 0.0],
                    _ => [1.0, 1.0, 1.0],
                };
                match name {
                    "explosion" => {
                        m.size = 2.0 * (1.0 - v.x as f32 * 0.5);
                        m.life = 6 + (r1 * 4.0) as u32;
                        m.gravity = 0.0;
                        m.vel = DVec3::ZERO;
                        m.collide = false;
                        m.bright = true;
                        m.shrink = Shrink::None;
                    }
                    "poof" | "cloud" | "sneeze" => {
                        m.vel = v + DVec3::new((r1 as f64 - 0.5) * 0.05, (r2 as f64 - 0.5) * 0.05, (r3 as f64 - 0.5) * 0.05);
                        m.gravity = 0.0;
                        m.friction = 0.9;
                        m.life = (16.0 / (r1 * 0.8 + 0.2)) as u32 + 2;
                    }
                    "sonic_boom" | "flash" => {
                        m.vel = DVec3::ZERO;
                        m.gravity = 0.0;
                        m.collide = false;
                        m.life = 16;
                        m.bright = true;
                        m.shrink = Shrink::None;
                    }
                    "campfire_cosy_smoke" | "campfire_signal_smoke" => {
                        m.life = if name == "campfire_signal_smoke" { 280 } else { 80 } + (r1 * 50.0) as u32;
                        m.gravity = 3.0e-6;
                        m.vel = DVec3::new(v.x, v.y + r2 as f64 / 500.0, v.z);
                        m.friction = 1.0;
                        m.collide = false;
                        m.translucent = true;
                        m.alpha = if name == "campfire_signal_smoke" { 0.95 } else { 0.9 };
                        m.shrink = Shrink::None;
                        m.fade = true;
                    }
                    "firefly" => {
                        m.gravity = 0.0;
                        m.bright = true;
                        m.life = 40 + (r1 * 80.0) as u32;
                        m.shrink = Shrink::None;
                        m.fade = true;
                        m.translucent = true;
                        m.friction = 0.96;
                    }
                    _ => {}
                }
                self.push(m);
            }
            "flame" | "soul_fire_flame" | "small_flame" | "copper_fire_flame" => {
                let Some(look) = self.set_look(name, false) else { return };
                let mut m = self.base(pos, v * 1.0 + DVec3::new((r1 as f64 - 0.5) * 0.02, (r2 as f64 - 0.5) * 0.02, (r3 as f64 - 0.5) * 0.02), look);
                m.friction = 0.96;
                m.life = (8.0 / (r1 * 0.8 + 0.2)) as u32 + 4;
                m.bright = true;
                m.shrink = Shrink::Flame;
                m.size *= if name == "small_flame" { 0.5 } else { 1.0 };
                self.push(m);
            }
            "lava" => {
                let Some(look) = self.set_look(name, false) else { return };
                let vel = DVec3::new((r1 as f64 - 0.5) * 0.8 * 0.4, r2 as f64 * 0.4 + 0.05, (r3 as f64 - 0.5) * 0.8 * 0.4);
                let mut m = self.base(pos, vel, look);
                m.gravity = 0.75;
                m.friction = 0.999;
                m.size *= r2 * 2.0 + 0.2;
                m.life = (16.0 / (r1 * 0.8 + 0.2)) as u32;
                m.bright = true;
                self.push(m);
            }
            "dust" | "dust_color_transition" => {
                let Some(look) = self.set_look("dust", true) else { return };
                let scale = opts.scale.unwrap_or(1.0);
                let vel = self.scatter(v) * 0.1 + v;
                let mut m = self.base(pos, vel, look);
                m.friction = 0.96;
                m.size *= 0.75 * scale;
                m.life = ((8.0 / (r1 * 0.8 + 0.2)) * scale.max(1.0)) as u32;
                let tint = r2 * 0.4 + 0.6;
                let c = opts.color.unwrap_or([1.0, 0.0, 0.0]);
                m.color = [c[0] * tint, c[1] * tint, c[2] * tint];
                self.push(m);
            }
            "entity_effect" | "effect" | "instant_effect" | "witch" | "dragon_breath" | "glow" | "infested" => {
                let Some(look) = self.set_look(name, true) else { return };
                let mut m = self.base(pos, DVec3::new(v.x * 0.1, (v.y * 0.1) + 0.1, v.z * 0.1), look);
                m.friction = 0.96;
                m.gravity = -0.1;
                m.life = (8.0 / (r1 * 0.8 + 0.2)) as u32;
                m.size *= 0.75;
                if let Some(c) = opts.color {
                    m.color = c;
                }
                if name == "witch" {
                    let g = r2 * 0.5 + 0.35;
                    m.color = [g, 0.0, g];
                }
                if opts.alpha_given {
                    m.alpha = opts.alpha;
                    m.translucent = true;
                }
                self.push(m);
            }
            "heart" | "angry_villager" => {
                let Some(look) = self.set_look(name, false) else { return };
                let mut m = self.base(pos, DVec3::new(0.0, 0.1, 0.0), look);
                m.friction = 0.86;
                m.life = 16;
                m.size *= 1.5;
                m.shrink = Shrink::Heart;
                m.collide = false;
                self.push(m);
            }
            "happy_villager" | "composter" | "note" | "underwater" | "current_down" | "nautilus" | "enchant" | "portal" | "reverse_portal" | "end_rod" | "totem_of_undying" | "soul" | "sculk_soul" | "sculk_charge" | "snowflake" | "dolphin" | "fishing" | "bubble" | "bubble_pop" | "bubble_column_up" | "splash" | "rain" | "crit" | "enchanted_hit" | "damage_indicator" | "sweep_attack" | "firework" | "shriek" | "vibration" | "trail" | "pause_mob_growth" | "reset_mob_growth" | "egg_crack" | "trial_spawner_detection" | "trial_spawner_detection_ominous" | "vault_connection" | "ominous_spawning" | "raid_omen" | "trial_omen" | "elder_guardian" | "spore_blossom_air" | "sulfur_cube_goo" => self.spawn_motion(name, pos, v, opts, [r1, r2, r3]),
            "dripping_water" | "falling_water" | "dripping_lava" | "falling_lava" | "landing_lava" | "dripping_honey" | "falling_honey" | "landing_honey" | "falling_nectar" | "falling_spore_blossom" | "dripping_obsidian_tear" | "falling_obsidian_tear" | "landing_obsidian_tear" | "dripping_dripstone_lava" | "falling_dripstone_lava" | "dripping_dripstone_water" | "falling_dripstone_water" | "cherry_leaves" | "pale_oak_leaves" | "tinted_leaves" | "red_poplar_leaves" | "orange_poplar_leaves" | "yellow_poplar_leaves" => {
                let Some(look) = self.set_look(name, false) else { return };
                let mut m = self.base(pos, DVec3::new(v.x, v.y, v.z) * 0.02, look);
                let drip = name.contains("dripping") || name.contains("falling") || name.contains("landing");
                if drip {
                    m.gravity = if name.starts_with("dripping") { 0.06 } else { 0.06 };
                    m.life = if name.starts_with("landing") { (16.0 / (r1 * 0.8 + 0.2)) as u32 } else { 64 / (1 + (r1 * 3.0) as u32).max(1) };
                    m.friction = 0.98;
                    m.size = 0.1 * 0.75;
                    m.color = if name.contains("water") {
                        [0.2, 0.3, 1.0]
                    } else if name.contains("lava") {
                        [1.0, 0.25, 0.04]
                    } else if name.contains("honey") {
                        [0.62, 0.42, 0.1]
                    } else if name.contains("obsidian_tear") {
                        [0.51, 0.03, 0.83]
                    } else {
                        [1.0, 1.0, 1.0]
                    };
                    m.bright = name.contains("lava") || name.contains("obsidian");
                } else {
                    // Leaves drift down, swaying.
                    m.life = 300;
                    m.gravity = 0.07;
                    m.friction = 1.0;
                    m.size *= if r1 > 0.5 { 1.2 } else { 0.7 };
                    m.vel = DVec3::new((r2 as f64 - 0.5) * 0.05, -0.02, (r3 as f64 - 0.5) * 0.05);
                    m.fade = true;
                    m.translucent = true;
                    if let Some(c) = opts.color {
                        m.color = c;
                    }
                }
                self.push(m);
            }
            // Spawners that make other particles, on the spot.
            "explosion_emitter" => {
                for _ in 0..6 {
                    let (a, b, c) = {
                        let r = self.rng();
                        (r.random::<f64>() - r.random::<f64>(), r.random::<f64>() - r.random::<f64>(), r.random::<f64>() - r.random::<f64>())
                    };
                    self.spawn("explosion", pos + DVec3::new(a, b, c) * 4.0, DVec3::new(0.0, 0.0, 0.0), opts);
                }
            }
            "gust_emitter_large" | "gust_emitter_small" => self.spawn("gust", pos, DVec3::ZERO, opts),
            _ => {}
        }
    }

    fn spawn_motion(&mut self, name: &str, pos: DVec3, v: DVec3, opts: &Opts, [r1, r2, r3]: [f32; 3]) {
        let by_age = matches!(name, "sweep_attack" | "firework" | "bubble_pop" | "end_rod" | "totem_of_undying" | "soul" | "sculk_soul" | "sculk_charge" | "snowflake" | "shriek" | "vibration" | "trial_spawner_detection" | "trial_spawner_detection_ominous" | "vault_connection" | "ominous_spawning" | "raid_omen" | "trial_omen" | "pause_mob_growth" | "reset_mob_growth");
        let Some(look) = self.set_look(name, by_age) else { return };
        let mut m = self.base(pos, v, look);
        match name {
            "crit" | "enchanted_hit" | "damage_indicator" => {
                m.vel = DVec3::new(v.x * 0.4, v.y * 0.4, v.z * 0.4) + DVec3::new((r1 as f64 * 2.0 - 1.0) * 0.1, (r2 as f64 * 2.0 - 1.0) * 0.1 + 0.1, (r3 as f64 * 2.0 - 1.0) * 0.1);
                m.friction = 0.7;
                m.gravity = 0.5;
                m.size *= 0.75;
                m.life = ((6.0 / (r1 * 0.8 + 0.6)) as u32).max(1);
                if name == "enchanted_hit" {
                    m.color = [0.3, 0.8, 1.0];
                } else if name == "crit" {
                    m.color = [1.0, 0.7, 0.3];
                }
            }
            "sweep_attack" => {
                m.life = 4;
                m.vel = DVec3::ZERO;
                m.size = 1.0 - v.x as f32 * 0.5;
                m.collide = false;
                let g = r1 * 0.6 + 0.4;
                m.color = [g, g, g];
                m.bright = true;
            }
            "firework" => {
                m.gravity = 0.1;
                m.friction = 0.91;
                m.life = 8 + (r1 * 12.0) as u32;
                m.bright = true;
                if let Some(c) = opts.color {
                    m.color = c;
                }
            }
            "totem_of_undying" => {
                m.gravity = 1.25;
                m.friction = 0.6;
                m.vel = v;
                m.life = 60 + (r1 * 12.0) as u32;
                m.size *= 0.75;
                m.color = if r2 < 0.5 { [0.6 + r3 * 0.2, 0.7 + r3 * 0.3, 0.1] } else { [0.1 + r3 * 0.2, 0.4 + r3 * 0.3, 0.2] };
            }
            "end_rod" | "soul" | "sculk_soul" | "trial_spawner_detection" | "trial_spawner_detection_ominous" | "ominous_spawning" | "raid_omen" | "trial_omen" => {
                m.vel = v;
                m.gravity = if name == "end_rod" { -0.001 } else { 0.0 };
                m.friction = 0.96;
                m.life = 40 + (r1 * 12.0) as u32;
                m.bright = true;
                m.fade = true;
                m.translucent = true;
                m.size *= 0.75;
            }
            "enchant" | "portal" | "reverse_portal" | "nautilus" | "vault_connection" => {
                // Comes from the offset and is drawn in to where it started.
                let start = pos + v;
                m.pos = start;
                m.prev = start;
                m.target = Some(pos);
                m.vel = DVec3::ZERO;
                m.collide = false;
                m.life = 30 + (r1 * 10.0) as u32;
                m.size = 0.1 * (r2 * 0.5 + 0.2);
                let g = r3 * 0.6 + 0.4;
                m.color = if name == "enchant" { [0.9 * g, 0.9 * g, g] } else { [g * 0.9, g * 0.3, g] };
                m.bright = true;
            }
            "happy_villager" | "composter" | "underwater" | "current_down" | "dolphin" | "spore_blossom_air" => {
                m.vel = v * 0.1 + DVec3::new(0.0, 0.0, 0.0);
                m.life = (16.0 / (r1 * 0.8 + 0.2)) as u32;
                m.friction = 1.0;
                m.size *= r2 * 0.6 + 0.2;
                m.collide = name != "underwater";
                if name == "underwater" {
                    m.color = [0.4, 0.4, 0.7];
                    m.vel = DVec3::ZERO;
                }
            }
            "note" => {
                m.vel = DVec3::new(0.0, 0.2, 0.0);
                m.friction = 0.66;
                m.life = 6;
                m.size *= 1.5;
                m.collide = false;
                // The value picks the pitch's colour round the wheel.
                let t = v.x as f32;
                let wave = |o: f32| ((t + o) * std::f32::consts::TAU).sin().max(0.0) * 0.65 + 0.35;
                m.color = [wave(0.0), wave(1.0 / 3.0), wave(2.0 / 3.0)];
            }
            "bubble" | "bubble_column_up" => {
                m.vel = v * 0.2 + DVec3::new((r1 as f64 * 2.0 - 1.0) * 0.02, (r2 as f64 * 2.0 - 1.0) * 0.02 + 0.03, (r3 as f64 * 2.0 - 1.0) * 0.02);
                m.gravity = -0.125;
                m.friction = 0.85;
                m.life = (8.0 / (r1 * 0.8 + 0.2)) as u32;
                m.size *= 0.5;
            }
            "bubble_pop" => {
                m.vel = v;
                m.gravity = 0.008;
                m.life = 4;
                m.size *= 1.0;
            }
            "splash" | "rain" | "fishing" => {
                m.vel = if name == "fishing" { v } else { DVec3::new(v.x * 0.3 + (r1 as f64 - 0.5) * 0.1, r2 as f64 * 0.2 + 0.1, v.z * 0.3 + (r3 as f64 - 0.5) * 0.1) };
                m.gravity = 0.06;
                m.friction = 0.98;
                m.life = (8.0 / (r1 * 0.8 + 0.2)) as u32;
                m.size *= 0.5;
                m.color = [0.4, 0.5, 1.0];
            }
            "snowflake" => {
                m.vel = v * 0.1;
                m.gravity = 0.225;
                m.friction = 1.0;
                m.life = (18.0 / (r1 * 0.8 + 0.2)) as u32;
            }
            _ => {
                m.vel = v;
                m.life = m.life.max(20);
            }
        }
        self.push(m);
    }

    /// One game tick.
    pub fn tick(&mut self, world: &lattice_world::World) {
        let data = lattice_data::game();
        self.motes.retain_mut(|m| {
            m.prev = m.pos;
            m.age += 1;
            if m.age >= m.life {
                return false;
            }
            if let Some(t) = m.target {
                let f = 1.0 - m.age as f64 / m.life as f64;
                let k = f * f;
                let start = m.pos;
                let _ = start;
                m.pos = t + (m.pos - t) * (0.85 + 0.15 * k);
                return true;
            }
            m.vel.y -= 0.04 * m.gravity as f64;
            let next = m.pos + m.vel;
            if m.collide {
                let (x, y, z) = (next.x.floor() as i32, next.y.floor() as i32, next.z.floor() as i32);
                let inside = world.block(x, y, z).is_some_and(|s| {
                    let base = DVec3::new(x as f64, y as f64, z as f64);
                    let p = next - base;
                    data.collision(s).iter().any(|b| p.x >= b.min[0] && p.x <= b.max[0] && p.y >= b.min[1] && p.y <= b.max[1] && p.z >= b.min[2] && p.z <= b.max[2])
                });
                if inside {
                    // Stops on whichever way it came in; good enough for something this small.
                    if m.vel.y < 0.0 {
                        m.on_ground = true;
                    }
                    m.vel = DVec3::new(m.vel.x * 0.0, 0.0, m.vel.z * 0.0);
                } else {
                    m.pos = next;
                    m.on_ground = false;
                }
            } else {
                m.pos = next;
            }
            m.vel *= m.friction as f64;
            if m.on_ground {
                m.vel.x *= 0.7;
                m.vel.z *= 0.7;
            }
            true
        });
    }

    /// Writes every mote as a quad facing the camera.
    pub fn draw(&self, ctx: &mut crate::bodies::Ctx, world: &lattice_world::World, camera: DVec3, partial: f32) {
        use crate::bodies::Pass;
        use lattice_render::mesher::Vertex;
        let rot = ctx.camera_rot;
        let (right, up) = (rot * Vec3::X, rot * Vec3::Y);
        for m in &self.motes {
            let at = m.prev.lerp(m.pos, partial as f64);
            let rel = (at - camera).as_vec3();
            if rel.length_squared() > 4096.0 * 4.0 {
                continue;
            }
            let t = (m.age as f32 + partial) / m.life.max(1) as f32;
            let size = match m.shrink {
                Shrink::None => m.size,
                Shrink::Flame => m.size * (1.0 - t * t * 0.5),
                Shrink::Smoke => m.size * ((m.age as f32 + partial) / m.life.max(1) as f32 * 32.0).clamp(0.0, 1.0),
                Shrink::Heart => m.size * ((m.age as f32 + partial) / m.life.max(1) as f32 * 32.0).clamp(0.0, 1.0),
            };
            let light = if m.bright {
                [255, 255]
            } else {
                let (s, b) = world.light(at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
                [s.saturating_mul(17), b.saturating_mul(17)]
            };
            let alpha = m.alpha * if m.fade { (1.0 - t).clamp(0.0, 1.0) } else { 1.0 };
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
            let color = [c(m.color[0]), c(m.color[1]), c(m.color[2]), c(alpha)];
            let rect = match m.look {
                Look::Block(r) | Look::Item(r) => r,
                Look::Set(set, frames) => {
                    let list = &self.sets[set as usize];
                    if list.is_empty() {
                        continue;
                    }
                    let f = match frames {
                        Frames::ByAge => ((t * list.len() as f32) as usize).min(list.len() - 1),
                        Frames::Fixed(f) => f as usize % list.len(),
                    };
                    let s = list[f];
                    [s.u0, s.v0, s.u1, s.v1]
                }
            };
            let (r, u) = (right * size, up * size);
            let quad = [(rel - r + u, [rect[0], rect[1]]), (rel - r - u, [rect[0], rect[3]]), (rel + r - u, [rect[2], rect[3]]), (rel + r + u, [rect[2], rect[1]])];
            let list = match m.look {
                Look::Block(_) | Look::Item(_) => ctx.block_list(false),
                Look::Set(..) => ctx.list(if m.translucent { Pass::Translucent } else { Pass::Cutout }),
            };
            let lit = [light[0], light[1], 0, 0];
            for (p, uv) in quad {
                list.push(Vertex { pos: p.to_array(), uv, color, light: lit });
            }
        }
    }
}

/// What a particle's options say.
#[derive(Default, Clone, Copy)]
pub struct Opts {
    pub block: Option<(Sprite, [f32; 3])>,
    pub item: Option<Sprite>,
    pub color: Option<[f32; 3]>,
    pub scale: Option<f32>,
    pub alpha: f32,
    pub alpha_given: bool,
}
