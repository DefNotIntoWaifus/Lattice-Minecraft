//! Gives the world's things their bodies: the shapes and animations read from the player's
//! own game files, posed each frame from what the server says about each entity.

pub mod anim;
pub mod blockent;
pub mod held;
pub mod maps;
pub mod overlay;
pub mod mobs;
pub mod sky;
pub mod text;
pub mod trims;
pub mod things;

use crate::game::entities::Entity;
use glam::{DVec3, Mat4, Vec3};
use lattice_assets::Resources;
use lattice_assets::geometry::{Animation, Animations, Models};
use lattice_render::atlas::{Sheet, Sprite};
use lattice_render::mesher::Vertex;
use lattice_render::parts::{self, Look, Rig, Skeleton};
use lattice_render::renderer::Batches;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// How many players' own skins can be held at once.
pub const SKIN_SLOTS: u32 = 48;

/// A worn thing's textures for each kind of body it fits, from `equipment/*.json`.
#[derive(Debug, Default)]
pub struct Equipment {
    /// Layer kind ("humanoid", "horse_body", ...) to its textures.
    pub layers: HashMap<String, Vec<EquipmentLayer>>,
}

#[derive(Debug, Clone)]
pub struct EquipmentLayer {
    /// Under `entity/equipment/<kind>/`.
    pub texture: String,
    pub dyeable: bool,
    pub undyed: Option<u32>,
}

pub struct Bodies {
    pub sheet: Sheet,
    pub font: text::Font,
    pub clouds: sky::CloudMap,
    /// Rain and snow pictures, handed to the renderer once.
    pub weather: Option<(image::RgbaImage, image::RgbaImage)>,
    skeletons: HashMap<String, Skeleton>,
    animations: Animations,
    equipment: HashMap<String, Equipment>,
    /// Hat rules of villager types and professions: whether a hat replaces the head.
    pub hats: HashMap<String, String>,
}

impl Bodies {
    /// Reads shapes, animations and textures out of the jar. Failing to read shapes isn't fatal:
    /// the game still runs, with things drawn as plain boxes.
    pub fn load(jar: &Path, res: &Resources) -> Bodies {
        let models: Models = match lattice_assets::geometry::read_models(jar) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("couldn't read mob shapes from the game files: {e}");
                HashMap::new()
            }
        };
        let animations = lattice_assets::geometry::read_animations(jar).unwrap_or_else(|e| {
            tracing::error!("couldn't read animations from the game files: {e}");
            HashMap::new()
        });
        let skeletons: HashMap<String, Skeleton> = models.iter().map(|(k, m)| (k.clone(), Skeleton::new(m))).collect();

        let mut textures = Vec::new();
        for prefix in ["textures/entity/", "textures/painting/", "textures/trims/entity/", "textures/particle/", "textures/map/"] {
            for path in res.list(prefix, ".png") {
                let name = path.trim_start_matches("textures/").trim_end_matches(".png").to_string();
                if let Some(img) = res.image(&path) {
                    textures.push((name, img));
                }
            }
        }
        // Shadows and enchantment shine live outside `entity/`.
        for extra in ["misc/shadow", "misc/underwater", "misc/forcefield", "misc/enchanted_glint_item", "misc/enchanted_glint_armor"] {
            if let Some(img) = res.image(&format!("textures/{extra}.png")) {
                textures.push((extra.to_string(), img));
            }
        }
        for path in res.list("textures/environment/celestial/", ".png") {
            let name = path.trim_start_matches("textures/").trim_end_matches(".png").to_string();
            if let Some(img) = res.image(&path) {
                textures.push((name, img));
            }
        }
        trims::load(res, &mut textures);
        let clouds = res.image("textures/environment/clouds.png").map_or(sky::CloudMap { cells: Vec::new(), size: 0 }, |i| sky::CloudMap::from_image(&i));
        let weather = res.image("textures/environment/rain.png").zip(res.image("textures/environment/snow.png"));
        let font = text::Font::load(res, &mut textures);
        let sheet = Sheet::build(textures, SKIN_SLOTS, maps::SLOTS);
        tracing::info!("mob shapes: {} models, {} animations, {} textures on a {}x{} sheet", skeletons.len(), animations.len(), sheet.sprites.len(), sheet.width, sheet.height);

        let mut equipment = HashMap::new();
        for path in res.list("equipment/", ".json") {
            let Some(v) = res.json(&path) else { continue };
            let name = path.trim_start_matches("equipment/").trim_end_matches(".json").to_string();
            let mut e = Equipment::default();
            if let Some(layers) = v.get("layers").and_then(Value::as_object) {
                for (kind, list) in layers {
                    let parsed = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|l| {
                            Some(EquipmentLayer {
                                texture: l.get("texture")?.as_str()?.trim_start_matches("minecraft:").to_string(),
                                dyeable: l.get("dyeable").is_some(),
                                undyed: l.get("dyeable").and_then(|d| d.get("color_when_undyed")).and_then(Value::as_i64).map(|c| c as u32),
                            })
                        })
                        .collect();
                    e.layers.insert(kind.clone(), parsed);
                }
            }
            equipment.insert(name, e);
        }

        let mut hats = HashMap::new();
        for path in res.list("textures/entity/villager/", ".png.mcmeta").into_iter().chain(res.list("textures/entity/zombie_villager/", ".png.mcmeta")) {
            let Some(v) = res.json(&path) else { continue };
            if let Some(h) = v.get("villager").and_then(|x| x.get("hat")).and_then(Value::as_str) {
                let key = path.trim_start_matches("textures/").trim_end_matches(".png.mcmeta").to_string();
                hats.insert(key, h.to_string());
            }
        }
        Bodies { sheet, font, clouds, weather, skeletons, animations, equipment, hats }
    }

    pub fn skeleton(&self, name: &str) -> Option<&Skeleton> {
        if let Some(s) = self.skeletons.get(name) {
            return Some(s);
        }
        let found = self.skeletons.get(&format!("{name}#main"));
        if found.is_none() {
            warn_once(&format!("no shape named {name}"));
        }
        found
    }

    pub fn animation(&self, name: &str) -> Option<&Animation> {
        self.animations.get(name)
    }

    pub fn equipment(&self, name: &str) -> Option<&Equipment> {
        self.equipment.get(name.trim_start_matches("minecraft:"))
    }

}

/// Which list a drawn layer goes in.
/// Where the shine on enchanted things has got to this frame.
#[derive(Clone, Copy)]
pub struct Shine {
    /// Moves a texture coordinate of an item's or a worn piece's to the shine's.
    item: [f32; 6],
    armor: [f32; 6],
    /// How bright, 0 to 255.
    pub strength: u8,
}

impl Shine {
    /// `millis` since anything; the speed and strength settings are fractions.
    pub fn at(millis: f64, speed: f32, strength: f32, atlas_scale: f32) -> Shine {
        let l = (millis * speed as f64 * 8.0) as i64;
        let (x, y) = (-((l % 110000) as f32 / 110000.0), (l % 30000) as f32 / 30000.0);
        let (s, c) = (0.17453292f32.sin(), 0.17453292f32.cos());
        let matrix = |k: f32| [c * k, -s * k, s * k, c * k, x, y];
        Shine { item: matrix(8.0 * atlas_scale), armor: matrix(0.16), strength: (strength * 255.0) as u8 }
    }

    pub fn none() -> Shine {
        Shine { item: [0.0; 6], armor: [0.0; 6], strength: 0 }
    }

    pub fn item_uv(&self, uv: [f32; 2]) -> [f32; 2] {
        Self::apply(&self.item, uv)
    }

    pub fn armor_uv(&self, uv: [f32; 2]) -> [f32; 2] {
        Self::apply(&self.armor, uv)
    }

    fn apply(m: &[f32; 6], uv: [f32; 2]) -> [f32; 2] {
        [m[0] * uv[0] + m[1] * uv[1] + m[4], m[2] * uv[0] + m[3] * uv[1] + m[5]]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Cutout,
    Translucent,
    Glow,
    Shadow,
    Through,
    /// Drawn with the repeating pictures of energy, not the sheet.
    EnergyCreeper,
    EnergyWither,
    EndPortal,
}

/// Everything a frame's bodies are drawn with.
pub struct Ctx<'a> {
    pub bodies: &'a Bodies,
    /// `None` only while drawing item icons, before the visuals exist.
    pub items: Option<&'a lattice_render::items::ItemVisuals>,
    pub out: &'a mut Batches,
    /// Registries of the server we're on, for turning variant numbers into names.
    pub registries: Option<&'a lattice_world::Registries>,
    /// Skin sprite for a player, when theirs has arrived.
    pub skins: &'a crate::skins::SkinStore,
    pub partial: f32,
    pub day_fraction: f32,
    pub blocks: &'a lattice_render::models::BlockModels,
    /// Faces the camera, for flat things.
    pub camera_rot: glam::Quat,
    pub camera_yaw_rad: f32,
    pub camera_pitch_rad: f32,
    pub camera_yaw_deg: f32,
    /// Two flame pictures from the block atlas.
    pub fire: [Sprite; 2],
    /// Sends everything to the held-item lists, which are drawn last.
    pub to_hand: bool,
    pub lang: &'a dyn lattice_proto::Translator,
    /// Keeps shapes unshaded, for drawing into icons that get lit afterwards.
    pub unlit: bool,
    pub shine: Shine,
    /// What's being drawn right now is enchanted.
    pub shining: bool,
    /// Maps known to this session and where their pictures are on the sheet.
    pub maps: &'a HashMap<i32, crate::game::MapData>,
    pub map_textures: &'a maps::MapTextures,
    /// Options that decide what's drawn.
    pub shadows: bool,
    pub tag_background: f32,
}

/// Logs a problem once rather than every frame.
fn warn_once(what: &str) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap();
    if seen.get_or_insert_default().insert(what.to_string()) {
        tracing::warn!("{what}");
    }
}

impl Ctx<'_> {
    pub fn sprite(&self, name: &str) -> Option<Sprite> {
        let found = self.bodies.sheet.try_sprite(name);
        if found.is_none() {
            warn_once(&format!("no texture named {name}"));
        }
        found
    }

    /// A registry's entry name for a variant number the server sent.
    pub fn variant(&self, registry: &str, id: i32) -> Option<&str> {
        self.registries?.names.get(registry)?.get(id as usize).map(|s| s.trim_start_matches("minecraft:"))
    }

    pub fn list(&mut self, pass: Pass) -> &mut Vec<Vertex> {
        if self.to_hand {
            return &mut self.out.hand_entity;
        }
        match pass {
            Pass::Cutout => &mut self.out.entity,
            Pass::Translucent => &mut self.out.entity_translucent,
            Pass::Glow => &mut self.out.entity_glow,
            Pass::Shadow => &mut self.out.shadow,
            Pass::Through => &mut self.out.through,
            Pass::EnergyCreeper => &mut self.out.energy_creeper,
            Pass::EnergyWither => &mut self.out.energy_wither,
            Pass::EndPortal => &mut self.out.end_portal,
        }
    }
}

impl Ctx<'_> {
    /// Where quads textured from the block atlas go.
    pub fn block_list(&mut self, translucent: bool) -> &mut Vec<Vertex> {
        if self.to_hand {
            &mut self.out.hand_blocks
        } else if translucent {
            &mut self.out.blocks_translucent
        } else {
            &mut self.out.blocks
        }
    }
}

/// The state of a living thing at the moment of drawing, as its animations read it.
#[derive(Debug, Clone, Copy)]
pub struct St {
    pub id: i32,
    pub age: f32,
    /// Walk cycle position and speed.
    pub wp: f32,
    pub ws: f32,
    /// Head turned from the body, and looking down, degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub body: f32,
    pub baby: bool,
    pub age_scale: f32,
    /// Arm swing, 0..1.
    pub swing: f32,
    pub swing_off_hand: bool,
    /// Ticks into dying, with fraction; 0 while alive.
    pub death: f32,
    pub hurt: bool,
    pub in_water: bool,
    pub riding: bool,
    pub crouching: bool,
    pub swimming: f32,
    pub gliding: bool,
    pub light: [u8; 2],
    pub invisible: bool,
    /// Right, unless the data says left.
    pub left_handed: bool,
    pub aggressive: bool,
    /// Height of its box, for turning around its middle.
    pub sleeping: bool,
    pub moving: bool,
    /// Slim arms, on a player.
    pub slim: bool,
}

pub fn rot_lerp(t: f32, a: f32, b: f32) -> f32 {
    a + wrap_degrees(b - a) * t
}

pub fn wrap_degrees(mut d: f32) -> f32 {
    d %= 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// Light at a world position, packed as vertices expect.
pub fn light_at(world: &lattice_world::World, x: i32, y: i32, z: i32) -> [u8; 2] {
    let (sky, block) = world.light(x, y, z);
    [sky.saturating_mul(17), block.saturating_mul(17)]
}

/// Draws one layer of a posed model.
pub fn draw_layer(ctx: &mut Ctx, rig: &Rig, m: Mat4, sprite: Sprite, s: &St, color: [u8; 4], pass: Pass, flash: bool) {
    let look = Look {
        sprite,
        color,
        light: if pass == Pass::Glow { [255, 255] } else { s.light },
        overlay: [if flash && s.hurt { 255 } else { 0 }, 0],
        scroll: [0.0, 0.0],
        lit: pass != Pass::Glow && !ctx.unlit,
    };
    parts::draw(rig, m, &look, ctx.list(pass));
}

/// The usual transform from world to a living model: the rules every mob's renderer shares.
pub fn living_matrix(pos: Vec3, s: &St, scale: f32, flip_degrees: f32) -> Mat4 {
    let mut m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(scale));
    if !s.sleeping {
        m *= Mat4::from_rotation_y((180.0 - s.body).to_radians());
    }
    if s.death > 0.0 {
        let f = (((s.death - 1.0) / 20.0 * 1.6).max(0.0)).sqrt().min(1.0);
        m *= Mat4::from_rotation_z((f * flip_degrees).to_radians());
    }
    m
}

/// After any mob-specific turns: flips into model space.
pub fn model_space(m: Mat4, extra: Option<Mat4>) -> Mat4 {
    let mut out = m * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    if let Some(e) = extra {
        out *= e;
    }
    out * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
}

/// Every entity in view gets its body.
pub fn draw_entities(ctx: &mut Ctx, session: &crate::game::Session, camera: DVec3, third_person: bool, reach: f64) {
    let Some(world) = session.world.as_ref() else { return };
    let reach2 = reach * reach;
    for (id, e) in &session.entities.map {
        let p = e.prev_pos.lerp(e.pos, ctx.partial as f64);
        if p.distance_squared(camera) > reach2 && e.type_name() != "ender_dragon" {
            continue;
        }
        let pos = (p - camera).as_vec3();
        draw_entity(ctx, session, world, *id, e, p, pos);
        draw_label(ctx, session, world, *id, e, p, pos);
        if ctx.shadows {
            draw_shadow(ctx, world, e, p, camera);
        }
        trims::draw_leash(ctx, session, world, e, *id, p, camera);
        if e.type_name() == "fishing_bobber" {
            trims::draw_fishing_line(ctx, session, e, p, camera);
        }
    }
    if third_person {
        mobs::draw_local_player(ctx, session, world, camera);
    }
}

/// Draws something of a kind that isn't really there, with `m` as its feet: a spawner's mob.
pub fn draw_at(ctx: &mut Ctx, session: &crate::game::Session, kind: i32, m: Mat4, light: [u8; 2]) {
    let Some(world) = session.world.as_ref() else { return };
    let e = crate::game::entities::Entities::display(kind);
    // Drawn where it would be at the origin, then what it came to is moved into place.
    let (a, b) = (ctx.out.entity.len(), ctx.out.entity_translucent.len());
    mobs::draw_mob(ctx, session, world, 0, &e, Vec3::ZERO, light);
    for v in ctx.out.entity[a..].iter_mut().chain(ctx.out.entity_translucent[b..].iter_mut()) {
        v.pos = m.transform_point3(Vec3::from(v.pos)).to_array();
    }
}

/// Names floating over players, and over things that were given one.
fn draw_label(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, id: i32, e: &Entity, world_pos: DVec3, pos: Vec3) {
    if e.flag(5) {
        return;
    }
    let distance2 = pos.length_squared();
    let is_player = matches!(e.type_name(), "player");
    let custom = match e.field("CUSTOM_NAME") {
        Some(lattice_proto::DataValue::OptText(Some(n))) => Some(lattice_proto::Text::from_nbt(n)),
        _ => None,
    };
    let player_name = if is_player { session.tab.get(&e.uuid).map(|t| t.name.as_str()) } else { None };
    let spans = if let Some(name) = player_name {
        let me = session.tab.get(&session.self_uuid).map_or("", |t| t.name.as_str());
        if !session.scoreboard.name_visible(name, me) {
            return;
        }
        match session.tab.get(&e.uuid).and_then(|t| t.display.as_ref()) {
            Some(d) => d.spans(ctx.lang),
            None => session.scoreboard.decorated(name, ctx.lang),
        }
    } else {
        let looking_at = session.player.target_entity.is_some_and(|(t, _)| t == id);
        let Some(text) = custom.filter(|_| e.bool_field("CUSTOM_NAME_VISIBLE") || looking_at) else { return };
        text.spans(ctx.lang)
    };
    if is_player && player_name.is_none() {
        return;
    }
    let discrete = e.flag(1);
    let limit: f32 = if discrete { 32.0 } else { 64.0 };
    if distance2 > limit * limit {
        return;
    }
    let height = e.dimensions().1 as f32;
    let mut at = pos + Vec3::new(0.0, height + 0.5, 0.0);
    let light = light_at(world, world_pos.x.floor() as i32, (world_pos.y + height as f64).floor() as i32, world_pos.z.floor() as i32);
    let background = ctx.tag_background;
    // The score below the name, close up only, with the name lifted to make room.
    if let Some(name) = player_name {
        if distance2 < 100.0 {
            if let Some(line) = session.scoreboard.below_name(name, ctx.lang) {
                text::draw_name_tag(ctx, &line, at, discrete, light, background);
                at.y += 9.0 * 1.15 * 0.025;
            }
        }
    }
    text::draw_name_tag(ctx, &spans, at, discrete, light, background);
}

/// How wide a shadow each thing throws, from each renderer's own number.
fn shadow_radius(e: &Entity) -> (f32, f32) {
    let name = e.type_name();
    let baby = if e.is_baby() { 0.5 } else { 1.0 };
    let r = match name {
        "item" | "experience_orb" => return (0.15, 0.75),
        "bat" | "tadpole" => 0.25,
        "slime" | "magma_cube" | "sulfur_cube" => 0.25 * e.int_field("ID_SIZE").unwrap_or(1).max(1) as f32,
        "chicken" | "cod" | "endermite" | "frog" | "parrot" | "silverfish" | "vex" | "armadillo" | "rabbit" => 0.3,
        "allay" | "salmon" | "fox" | "turtle" => 0.4,
        "cow" | "mooshroom" | "pig" | "sheep" | "iron_golem" | "camel" | "camel_husk" | "squid" | "glow_squid" | "hoglin" | "zoglin" | "wolf" | "goat" | "nautilus" | "zombie_nautilus" | "llama" | "trader_llama" => 0.7,
        "cave_spider" => 0.56,
        "creaking" => 0.6,
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "phantom" => 0.75,
        "spider" | "oak_boat" | "polar_bear" | "panda" => 0.8,
        "warden" => 0.9,
        "wither" => 1.0,
        "ravager" => 1.1,
        "elder_guardian" => 1.2,
        "ghast" | "happy_ghast" | "sniffer" => 1.5,
        "giant" => 3.0,
        "minecart" | "chest_minecart" | "furnace_minecart" | "hopper_minecart" | "spawner_minecart" | "command_block_minecart" | "tnt_minecart" => 0.7,
        "shulker" | "marker" | "interaction" | "item_frame" | "glow_item_frame" | "painting" | "leash_knot" | "arrow" | "spectral_arrow" | "trident" | "area_effect_cloud" | "lightning_bolt" | "block_display" | "item_display" | "text_display" | "evoker_fangs" | "fishing_bobber" | "egg" | "snowball" | "ender_pearl" | "experience_bottle" | "splash_potion" | "lingering_potion" | "eye_of_ender" | "fireball" | "small_fireball" | "dragon_fireball" | "wither_skull" | "shulker_bullet" | "llama_spit" | "wind_charge" | "breeze_wind_charge" | "firework_rocket" | "ominous_item_spawner" | "cushion" => 0.0,
        n if n.ends_with("_boat") || n.ends_with("_raft") => 0.8,
        _ => 0.5,
    };
    (r * baby, 1.0)
}

/// A soft dark circle on the blocks under something.
fn draw_shadow(ctx: &mut Ctx, world: &lattice_world::World, e: &Entity, p: DVec3, camera: DVec3) {
    if e.flag(5) {
        return;
    }
    let (radius, strength_mul) = shadow_radius(e);
    let radius = radius.min(32.0);
    if radius <= 0.0 {
        return;
    }
    let strength = ((1.0 - p.distance_squared(camera) / 256.0) as f32) * strength_mul;
    if strength <= 0.0 {
        return;
    }
    let data = lattice_data::game();
    let sprite = ctx.bodies.sheet.try_sprite("misc/shadow").unwrap_or(ctx.bodies.sheet.missing);
    let reach = ((strength / 0.5 - 1.0).min(radius)) as f64;
    let (x0, x1) = ((p.x - radius as f64).floor() as i32, (p.x + radius as f64).floor() as i32);
    let (z0, z1) = ((p.z - radius as f64).floor() as i32, (p.z + radius as f64).floor() as i32);
    let (y0, y1) = ((p.y - reach).floor() as i32, p.y.floor() as i32);
    for z in z0..=z1 {
        for x in x0..=x1 {
            for y in y0..=y1 {
                let piece = strength - ((p.y - y as f64) as f32) * 0.5;
                let Some(below) = world.block(x, y - 1, z) else { continue };
                if data.state(below).flags & lattice_data::flags::FULL_COLLISION == 0 {
                    continue;
                }
                let (sky, block) = world.light(x, y, z);
                let level = sky.max(block);
                if level <= 3 {
                    continue;
                }
                let alpha = (piece * 0.5 * (level as f32 / 15.0)).clamp(0.0, 1.0);
                let rel = Vec3::new((x as f64 - p.x) as f32, (y as f64 - p.y) as f32, (z as f64 - p.z) as f32);
                // Only as far as the shadow picture reaches.
                let (ax, bx, az, bz) = (rel.x.max(-radius), (rel.x + 1.0).min(radius), rel.z.max(-radius), (rel.z + 1.0).min(radius));
                if ax >= bx || az >= bz {
                    continue;
                }
                let u = |v: f32| -v / 2.0 / radius + 0.5;
                let origin = (p - camera).as_vec3() + Vec3::new(0.0, rel.y + 0.0015, 0.0);
                let (du, dv) = (sprite.u1 - sprite.u0, sprite.v1 - sprite.v0);
                let tc = |a: f32, b: f32| [sprite.u0 + a * du, sprite.v0 + b * dv];
                let color = [255, 255, 255, (alpha * 255.0) as u8];
                let list = ctx.list(Pass::Shadow);
                for (vx, vz) in [(ax, az), (ax, bz), (bx, bz), (bx, az)] {
                    list.push(Vertex { pos: (origin + Vec3::new(vx, 0.0, vz)).to_array(), uv: tc(u(vx), u(vz)), color, light: [255, 255, 0, 0] });
                }
            }
        }
    }
}

fn draw_entity(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, id: i32, e: &Entity, world_pos: DVec3, pos: Vec3) {
    let light_pos = world_pos + DVec3::new(0.0, e.dimensions().1 * 0.5, 0.0);
    if e.flag(0) && !matches!(e.type_name(), "blaze" | "magma_cube" | "strider" | "wither" | "ender_dragon") {
        let (w, h) = e.dimensions();
        let fire = ctx.fire;
        let yaw = ctx.camera_yaw_deg;
        let l = light_at(world, world_pos.x.floor() as i32, world_pos.y.floor() as i32, world_pos.z.floor() as i32);
        sky::draw_flames(ctx, pos, w as f32, h as f32, l, yaw, fire);
    }
    let light = {
        let l = light_at(world, light_pos.x.floor() as i32, light_pos.y.floor() as i32, light_pos.z.floor() as i32);
        // Things that burn light themselves.
        if e.flag(0) { [l[0], 255] } else { l }
    };
    if !things::draw_thing(ctx, session, world, id, e, pos, light) {
        mobs::draw_mob(ctx, session, world, id, e, pos, light);
    }
}
