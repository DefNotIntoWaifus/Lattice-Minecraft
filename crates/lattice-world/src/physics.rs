//! Local player movement.
//!
//! Servers trust client movement within limits and teleport the player back when
//! a move disagrees with their own checks, so this follows the game's rules step
//! by step: `tick` runs in the same order as the game runs a player's tick, and
//! the float and double arithmetic is kept apart where the game keeps it apart.
//! `tests/golden.rs` replays recordings of the official client against it.

use glam::DVec3;
use lattice_data::{GameData, flags};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Block state access for movement. A column that isn't loaded has no blocks.
pub trait BlockAccess {
    /// Block state, or `None` when the column isn't loaded.
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u32>;
}

impl BlockAccess for crate::World {
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.block(x, y, z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }

    pub fn moved(&self, d: DVec3) -> Self {
        Self { min: self.min + d, max: self.max + d }
    }

    pub fn expand_towards(&self, d: DVec3) -> Self {
        let mut a = *self;
        for i in 0..3 {
            if d[i] < 0.0 {
                a.min[i] += d[i];
            } else {
                a.max[i] += d[i];
            }
        }
        a
    }

    pub fn deflate(&self, d: f64) -> Self {
        let v = DVec3::splat(d);
        Self { min: self.min + v, max: self.max - v }
    }

    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.x < o.max.x
            && self.max.x > o.min.x
            && self.min.y < o.max.y
            && self.max.y > o.min.y
            && self.min.z < o.max.z
            && self.max.z > o.min.z
    }
}

const EPS: f64 = 1.0e-7;

/// The game's sine, from its 65536-entry table of floats.
pub fn mth_sin(x: f64) -> f32 {
    let i = ((x * 10430.378350470453) as i64 & 65535) as f64;
    (i * std::f64::consts::TAU / 65536.0).sin() as f32
}

/// The game's cosine, from the same table.
pub fn mth_cos(x: f64) -> f32 {
    let i = (((x * 10430.378350470453) as i64 + 16384) & 65535) as f64;
    (i * std::f64::consts::TAU / 65536.0).sin() as f32
}

const DEG: f32 = 0.017453292;

fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < 1.0e-5f32 as f64
}

fn normalize(v: DVec3) -> DVec3 {
    let d = v.length();
    if d < 1.0e-5f32 as f64 { DVec3::ZERO } else { v / d }
}

fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// Allowed movement of `e` along `axis` before hitting any of `boxes`.
fn collide_axis(e: &Aabb, boxes: &[Aabb], axis: usize, mut d: f64) -> f64 {
    let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
    for b in boxes {
        if d.abs() < EPS {
            return 0.0;
        }
        if b.max[a1] - EPS <= e.min[a1] || b.min[a1] + EPS >= e.max[a1] {
            continue;
        }
        if b.max[a2] - EPS <= e.min[a2] || b.min[a2] + EPS >= e.max[a2] {
            continue;
        }
        if d > 0.0 && b.min[axis] - e.max[axis] >= -EPS {
            d = d.min(b.min[axis] - e.max[axis]);
        } else if d < 0.0 && b.max[axis] - e.min[axis] <= EPS {
            d = d.max(b.max[axis] - e.min[axis]);
        }
    }
    if d.abs() < EPS { 0.0 } else { d }
}

/// The game's axis order: Y first, then the larger horizontal component last.
fn collide_with(boxes: &[Aabb], e: Aabb, m: DVec3) -> DVec3 {
    if boxes.is_empty() {
        return m;
    }
    let mut out = DVec3::ZERO;
    let z_first = m.x.abs() < m.z.abs();
    let order: [usize; 3] = if z_first { [1, 2, 0] } else { [1, 0, 2] };
    for axis in order {
        if m[axis] != 0.0 {
            out[axis] = collide_axis(&e.moved(out), boxes, axis, m[axis]);
        }
    }
    out
}

/// Block groups movement depends on.
#[derive(Debug, Default, Clone)]
pub struct MoveTags {
    /// Block indices, not states: tags apply to all states of a block.
    pub climbable: HashSet<u16>,
    pub blocks_fluid_flow: HashSet<u16>,
    pub suppresses_bounce: HashSet<u16>,
    pub can_glide_through: HashSet<u16>,
    pub fences: HashSet<u16>,
    pub walls_and_gates: HashSet<u16>,
    pub trapdoors: HashSet<u16>,
}

impl MoveTags {
    /// The game's own tag contents, for when a server hasn't sent its copy.
    pub fn defaults(data: &GameData) -> Self {
        let set = |names: &[&str]| -> HashSet<u16> {
            names
                .iter()
                .flat_map(|t| data.block_tags.get(*t).into_iter().flatten())
                .filter_map(|n| data.blocks.iter().position(|b| &b.name == n))
                .map(|i| i as u16)
                .collect()
        };
        MoveTags {
            climbable: set(&["climbable"]),
            blocks_fluid_flow: set(&["blocks_fluid_flow"]),
            suppresses_bounce: set(&["suppresses_bounce"]),
            can_glide_through: set(&["can_glide_through"]),
            fences: set(&["fences"]),
            walls_and_gates: set(&["walls", "fence_gates"]),
            trapdoors: set(&["trapdoors"]),
        }
    }

    /// Replaces one group with a server's copy of the tag, if movement uses it.
    pub fn set_from_server(&mut self, tag: &str, blocks: impl Iterator<Item = u16>) {
        let target = match tag {
            "climbable" => &mut self.climbable,
            "blocks_fluid_flow" => &mut self.blocks_fluid_flow,
            "suppresses_bounce" => &mut self.suppresses_bounce,
            "can_glide_through" => &mut self.can_glide_through,
            "fences" => &mut self.fences,
            "trapdoors" => &mut self.trapdoors,
            _ => return,
        };
        *target = blocks.collect();
    }
}

/// Indices of blocks with their own movement behaviour.
struct Special {
    water: u16,
    bubble_column: u16,
    cobweb: u16,
    berry: u16,
    powder_snow: u16,
    honey: u16,
    scaffolding: u16,
    slime: u16,
    ladder: u16,
    ice: u16,
    frosted_ice: u16,
}

fn special(data: &GameData) -> &'static Special {
    static S: OnceLock<Special> = OnceLock::new();
    S.get_or_init(|| {
        let find = |n: &str| data.blocks.iter().position(|b| b.name == n).unwrap_or(usize::MAX) as u16;
        Special {
            water: find("water"),
            bubble_column: find("bubble_column"),
            cobweb: find("cobweb"),
            berry: find("sweet_berry_bush"),
            powder_snow: find("powder_snow"),
            honey: find("honey_block"),
            scaffolding: find("scaffolding"),
            slime: find("slime_block"),
            ladder: find("ladder"),
            ice: find("ice"),
            frosted_ice: find("frosted_ice"),
        }
    })
}

/// Keys held this tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub shift: bool,
    pub sprint: bool,
}

/// Attribute values the server sends for the player. Movement speed is kept
/// without the sprint modifier, which the player's own state decides.
#[derive(Debug, Clone, Copy)]
pub struct Attributes {
    pub movement_speed: f64,
    pub jump_strength: f64,
    pub gravity: f64,
    pub step_height: f64,
    pub sneaking_speed: f64,
    pub water_movement_efficiency: f64,
    pub movement_efficiency: f64,
    pub friction_modifier: f64,
    pub air_drag_modifier: f64,
    pub scale: f64,
    pub block_range: f64,
    pub entity_range: f64,
}

impl Attributes {
    pub fn defaults(data: &GameData) -> Self {
        let p = &data.player;
        Attributes {
            movement_speed: p.movement_speed,
            jump_strength: p.jump_strength,
            gravity: p.gravity,
            step_height: p.step_height,
            sneaking_speed: p.sneaking_speed,
            water_movement_efficiency: 0.0,
            movement_efficiency: 0.0,
            friction_modifier: 1.0,
            air_drag_modifier: 1.0,
            scale: 1.0,
            block_range: p.block_interaction_range,
            entity_range: p.entity_interaction_range,
        }
    }
}

/// Status effects that change movement directly (Speed and Slowness arrive as attributes).
#[derive(Debug, Clone, Copy, Default)]
pub struct Effects {
    pub jump_boost: Option<i32>,
    pub levitation: Option<i32>,
    pub slow_falling: bool,
    pub dolphins_grace: bool,
    pub blindness: bool,
    pub weaving: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    Swimming,
    FallFlying,
    SpinAttack,
    Sleeping,
}

impl Pose {
    /// Width, height and eye height.
    fn dims(self) -> (f32, f32, f32) {
        match self {
            Pose::Standing => (0.6, 1.8, 1.62),
            Pose::Crouching => (0.6, 1.5, 1.27),
            Pose::Swimming | Pose::FallFlying | Pose::SpinAttack => (0.6, 0.6, 0.4),
            Pose::Sleeping => (0.2, 0.2, 0.2),
        }
    }
}

/// What movement needs to know from outside the player's body.
pub struct MoveCtx<'a> {
    pub data: &'a GameData,
    pub tags: &'a MoveTags,
    /// Lava moves entities faster here (the Nether).
    pub fast_lava: bool,
    pub auto_jump: bool,
    /// Ticks allowed between two taps of forward to start sprinting.
    pub sprint_window: i32,
    /// Food above the level sprinting needs, or able to fly.
    pub enough_food: bool,
    /// Wearing leather boots, which walk on powder snow.
    pub leather_boots: bool,
    /// Wearing something that glides.
    pub can_glide: bool,
    /// Speed multiplier while using an item, if one is in use.
    pub item_use_speed: Option<f32>,
    /// The item in use allows sprinting.
    pub item_use_can_sprint: bool,
    /// Lowest block height of this dimension.
    pub min_y: i32,
    /// Where the world border is, if it's anywhere near: west, east, north, south.
    pub border: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Copy, Default)]
struct FluidTracker {
    height: f64,
    eyes: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Current {
    height: f64,
    sum: DVec3Default,
    count: u32,
}

#[derive(Debug, Clone, Copy)]
struct DVec3Default(DVec3);

impl Default for DVec3Default {
    fn default() -> Self {
        DVec3Default(DVec3::ZERO)
    }
}

/// One step of movement during a tick, kept for working out which blocks it touched.
#[derive(Debug, Clone, Copy)]
struct Step {
    from: DVec3,
    to: DVec3,
    original: Option<DVec3>,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub pos: DVec3,
    pub vel: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub minor_horizontal_collision: bool,
    pub vertical_collision: bool,
    vertical_below: bool,
    pub sprinting: bool,
    swimming_flag: bool,
    pub flying: bool,
    pub may_fly: bool,
    pub fly_speed: f32,
    pub fall_flying: bool,
    /// Walks through blocks (spectators).
    pub no_physics: bool,
    pub spectator: bool,
    /// Riding something; movement comes from the vehicle.
    pub passenger: bool,
    /// Can't act: asleep or dead.
    pub immobile: bool,
    pub pose: Pose,
    pub crouching: bool,
    pub in_water: bool,
    pub in_lava: bool,
    pub eye_in_water: bool,
    pub eye_in_lava: bool,
    water_height: f64,
    lava_height: f64,
    /// `isUnderWater`: the eyes were in water when this tick began.
    pub under_water: bool,
    in_powder_snow: bool,
    was_in_powder_snow: bool,
    stuck: DVec3,
    pub fall_distance: f64,
    pub attrs: Attributes,
    pub effects: Effects,
    pub keys: Keys,
    move_vector: (f32, f32),
    xxa: f32,
    zza: f32,
    jumping: bool,
    no_jump_delay: u8,
    sprint_trigger: i32,
    jump_trigger: i32,
    auto_jump_time: i32,
    support: Option<[i32; 3]>,
    on_ground_no_blocks: bool,
    in_block_state: Option<u32>,
    old_pos: DVec3,
    steps: Vec<Step>,
    /// Flight was switched by double-tapping jump; the server must be told.
    pub abilities_changed: bool,
    /// Gliding was started; the server must be told.
    pub start_gliding: bool,
    /// The player jumped this tick.
    pub jumped: bool,
    /// Width and height, for a body that isn't a player's.
    fixed_dims: Option<(f32, f32)>,
    /// An animal with a player steering it, which moves faster through the air.
    ridden: bool,
    /// Up and down input, for animals that swim or fly.
    yya: f32,
    /// A strider, which walks on lava as if it were ground.
    stands_on_lava: bool,
}

impl Body {
    pub fn new(data: &GameData) -> Self {
        Body {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
            minor_horizontal_collision: false,
            vertical_collision: false,
            vertical_below: false,
            sprinting: false,
            swimming_flag: false,
            flying: false,
            may_fly: false,
            fly_speed: 0.05,
            fall_flying: false,
            no_physics: false,
            spectator: false,
            passenger: false,
            immobile: false,
            pose: Pose::Standing,
            crouching: false,
            in_water: false,
            in_lava: false,
            eye_in_water: false,
            eye_in_lava: false,
            water_height: 0.0,
            lava_height: 0.0,
            under_water: false,
            in_powder_snow: false,
            was_in_powder_snow: false,
            stuck: DVec3::ZERO,
            fall_distance: 0.0,
            attrs: Attributes::defaults(data),
            effects: Effects::default(),
            keys: Keys::default(),
            move_vector: (0.0, 0.0),
            xxa: 0.0,
            zza: 0.0,
            jumping: false,
            no_jump_delay: 0,
            sprint_trigger: 0,
            jump_trigger: 0,
            auto_jump_time: 0,
            support: None,
            on_ground_no_blocks: false,
            in_block_state: None,
            old_pos: DVec3::ZERO,
            steps: Vec::new(),
            abilities_changed: false,
            start_gliding: false,
            jumped: false,
            fixed_dims: None,
            ridden: false,
            yya: 0.0,
            stands_on_lava: false,
        }
    }

    // ---------------------------------------------------------------- shape

    fn dims(&self, pose: Pose) -> (f32, f32, f32) {
        if let Some((w, h)) = self.fixed_dims {
            return (w, h, h * 0.85);
        }
        let (w, h, e) = pose.dims();
        let s = self.attrs.scale as f32;
        (w * s, h * s, e * s)
    }

    fn box_at(&self, pos: DVec3, pose: Pose) -> Aabb {
        let (w, h, _) = self.dims(pose);
        let f = (w / 2.0) as f64;
        Aabb::new(DVec3::new(pos.x - f, pos.y, pos.z - f), DVec3::new(pos.x + f, pos.y + h as f64, pos.z + f))
    }

    pub fn aabb(&self) -> Aabb {
        self.box_at(self.pos, self.pose)
    }

    pub fn eye_height(&self) -> f32 {
        self.dims(self.pose).2
    }

    pub fn block_pos(&self) -> [i32; 3] {
        [floor(self.pos.x), floor(self.pos.y), floor(self.pos.z)]
    }

    pub fn is_swimming(&self) -> bool {
        !self.flying && !self.spectator && self.swimming_flag
    }

    fn state_at(w: &impl BlockAccess, p: [i32; 3]) -> u32 {
        w.state(p[0], p[1], p[2]).unwrap_or(0)
    }

    // ------------------------------------------------------------ collision

    /// Collision boxes of a block as this player sees them. A few blocks depend on who's asking.
    fn block_boxes(&self, ctx: &MoveCtx, state: u32, y: i32) -> Vec<lattice_data::Aabb> {
        let data = ctx.data;
        let sp = special(data);
        let idx = data.block_index(state) as u16;
        if idx == sp.powder_snow {
            if self.fall_distance > 2.5 {
                return vec![lattice_data::Aabb { min: [0.0, 0.0, 0.0], max: [1.0, 0.9f32 as f64, 1.0] }];
            }
            if ctx.leather_boots && self.pos.y > y as f64 + 1.0 - 1.0e-5f32 as f64 && !self.keys.shift {
                return vec![lattice_data::Aabb { min: [0.0; 3], max: [1.0; 3] }];
            }
            return Vec::new();
        }
        if idx == sp.scaffolding {
            if self.pos.y > y as f64 + 1.0 - 1.0e-5f32 as f64 && !self.keys.shift {
                // The stable shape: the top plate and the four legs.
                let block = data.block(state);
                let stable = data.state_with(block, &[("bottom", "false"), ("distance", "0"), ("waterlogged", "false")]);
                return stable.map_or_else(Vec::new, |s| data.outline(s).to_vec());
            }
            let props = data.properties(state);
            let distance = props.iter().find(|(k, _)| *k == "distance").is_some_and(|(_, v)| *v != "0");
            let bottom = props.iter().any(|(k, v)| *k == "bottom" && *v == "true");
            // Just above the block below: the thin unstable bottom plate.
            if distance && bottom && self.pos.y > y as f64 - 1.0e-5f32 as f64 {
                return vec![lattice_data::Aabb { min: [0.0, 0.0, 0.0], max: [1.0, 2.0 / 16.0, 1.0] }];
            }
            return Vec::new();
        }
        if self.stands_on_lava && data.block(state).name == "lava" && state == data.block(state).first && self.pos.y > y as f64 + 0.5 - 1.0e-5f32 as f64 {
            return vec![lattice_data::Aabb { min: [0.0, 0.0, 0.0], max: [1.0, 0.5, 1.0] }];
        }
        data.collision(state).to_vec()
    }

    /// Collision boxes touching `region`. Columns that aren't loaded are empty, as in the game.
    fn boxes(&self, w: &impl BlockAccess, ctx: &MoveCtx, region: &Aabb) -> Vec<Aabb> {
        let mut out = Vec::new();
        // Everything beyond the border is solid, as long as the body started inside it.
        if let Some([west, east, north, south]) = ctx.border {
            let me = self.aabb();
            let inside = me.max.x >= west && me.min.x <= east && me.max.z >= north && me.min.z <= south;
            if inside {
                const FAR: f64 = 1.0e7;
                let (y0, y1) = (region.min.y - 1.0, region.max.y + 1.0);
                for wall in [
                    Aabb::new(DVec3::new(west - FAR, y0, north - FAR), DVec3::new(west, y1, south + FAR)),
                    Aabb::new(DVec3::new(east, y0, north - FAR), DVec3::new(east + FAR, y1, south + FAR)),
                    Aabb::new(DVec3::new(west - FAR, y0, north - FAR), DVec3::new(east + FAR, y1, north)),
                    Aabb::new(DVec3::new(west - FAR, y0, south), DVec3::new(east + FAR, y1, south + FAR)),
                ] {
                    if wall.intersects(region) {
                        out.push(wall);
                    }
                }
            }
        }
        let (x0, x1) = (floor(region.min.x - EPS) - 1, floor(region.max.x + EPS) + 1);
        let (y0, y1) = (floor(region.min.y - EPS) - 1, floor(region.max.y + EPS) + 1);
        let (z0, z1) = (floor(region.min.z - EPS) - 1, floor(region.max.z + EPS) + 1);
        for x in x0..=x1 {
            for z in z0..=z1 {
                for y in y0..=y1 {
                    let Some(s) = w.state(x, y, z) else { continue };
                    if s == 0 {
                        continue;
                    }
                    let base = DVec3::new(x as f64, y as f64, z as f64);
                    for b in self.block_boxes(ctx, s, y) {
                        let bb = Aabb::new(base + DVec3::from(b.min), base + DVec3::from(b.max));
                        if bb.intersects(region) {
                            out.push(bb);
                        }
                    }
                }
            }
        }
        out
    }

    fn no_collision(&self, w: &impl BlockAccess, ctx: &MoveCtx, region: &Aabb) -> bool {
        self.boxes(w, ctx, region).is_empty()
    }

    fn fits(&self, w: &impl BlockAccess, ctx: &MoveCtx, pose: Pose) -> bool {
        self.no_collision(w, ctx, &self.box_at(self.pos, pose).deflate(1.0e-7))
    }

    fn contains_any_liquid(&self, w: &impl BlockAccess, ctx: &MoveCtx, region: &Aabb) -> bool {
        for x in floor(region.min.x)..(region.max.x.ceil() as i32) {
            for y in floor(region.min.y)..(region.max.y.ceil() as i32) {
                for z in floor(region.min.z)..(region.max.z.ceil() as i32) {
                    if w.state(x, y, z).is_some_and(|s| ctx.data.state(s).fluid != 0) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn is_free(&self, w: &impl BlockAccess, ctx: &MoveCtx, d: DVec3) -> bool {
        let region = self.aabb().moved(d);
        self.no_collision(w, ctx, &region) && !self.contains_any_liquid(w, ctx, &region)
    }

    fn collide(&self, m: DVec3, w: &impl BlockAccess, ctx: &MoveCtx) -> DVec3 {
        let bb = self.aabb();
        let res = if m.length_squared() == 0.0 { m } else { collide_with(&self.boxes(w, ctx, &bb.expand_towards(m)), bb, m) };
        let (xc, yc, zc) = (m.x != res.x, m.y != res.y, m.z != res.z);
        let landed = yc && m.y < 0.0;
        let step = self.max_up_step();
        if step > 0.0 && (landed || self.on_ground) && (xc || zc) {
            let base = if landed { bb.moved(DVec3::new(0.0, res.y, 0.0)) } else { bb };
            let mut region = base.expand_towards(DVec3::new(m.x, step as f64, m.z));
            if !landed {
                region = region.expand_towards(DVec3::new(0.0, -9.999999747378752e-6, 0.0));
            }
            let boxes = self.boxes(w, ctx, &region);
            let skip = res.y as f32;
            let mut heights: Vec<f32> = Vec::new();
            for b in &boxes {
                for y in [b.min.y, b.max.y] {
                    let h = (y - base.min.y) as f32;
                    if h >= 0.0 && h != skip && h <= step && !heights.contains(&h) {
                        heights.push(h);
                    }
                }
            }
            heights.sort_by(|a, b| a.total_cmp(b));
            for h in heights {
                let attempt = collide_with(&boxes, base, DVec3::new(m.x, h as f64, m.z));
                if attempt.x * attempt.x + attempt.z * attempt.z > res.x * res.x + res.z * res.z {
                    return attempt - DVec3::new(0.0, bb.min.y - base.min.y, 0.0);
                }
            }
        }
        res
    }

    fn max_up_step(&self) -> f32 {
        self.attrs.step_height as f32
    }

    fn can_fall_at_least(&self, w: &impl BlockAccess, ctx: &MoveCtx, dx: f64, dz: f64, h: f64) -> bool {
        let b = self.aabb();
        let region = Aabb::new(
            DVec3::new(b.min.x + 1.0e-7 + dx, b.min.y - h - 1.0e-7, b.min.z + 1.0e-7 + dz),
            DVec3::new(b.max.x - 1.0e-7 + dx, b.min.y, b.max.z - 1.0e-7 + dz),
        );
        self.no_collision(w, ctx, &region)
    }

    /// Sneaking doesn't walk off edges.
    fn back_off_from_edge(&self, m: DVec3, w: &impl BlockAccess, ctx: &MoveCtx) -> DVec3 {
        let step = self.max_up_step() as f64;
        let above_ground = self.on_ground || (self.fall_distance < step && !self.can_fall_at_least(w, ctx, 0.0, 0.0, step - self.fall_distance));
        if self.flying || m.y > 0.0 || !self.keys.shift || !above_ground {
            return m;
        }
        let (mut x, mut z) = (m.x, m.z);
        let (sx, sz) = (x.signum() * 0.05, z.signum() * 0.05);
        while x != 0.0 && self.can_fall_at_least(w, ctx, x, 0.0, step) {
            x = if x.abs() <= 0.05 { 0.0 } else { x - sx };
        }
        while z != 0.0 && self.can_fall_at_least(w, ctx, 0.0, z, step) {
            z = if z.abs() <= 0.05 { 0.0 } else { z - sz };
        }
        while x != 0.0 && z != 0.0 && self.can_fall_at_least(w, ctx, x, z, step) {
            x = if x.abs() <= 0.05 { 0.0 } else { x - sx };
            z = if z.abs() <= 0.05 { 0.0 } else { z - sz };
        }
        DVec3::new(x, m.y, z)
    }

    fn find_support(&self, w: &impl BlockAccess, ctx: &MoveCtx, region: &Aabb) -> Option<[i32; 3]> {
        let mut best: Option<[i32; 3]> = None;
        let mut best_d = f64::MAX;
        let (x0, x1) = (floor(region.min.x - EPS) - 1, floor(region.max.x + EPS) + 1);
        let (y0, y1) = (floor(region.min.y - EPS) - 1, floor(region.max.y + EPS) + 1);
        let (z0, z1) = (floor(region.min.z - EPS) - 1, floor(region.max.z + EPS) + 1);
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let Some(s) = w.state(x, y, z) else { continue };
                    if s == 0 {
                        continue;
                    }
                    let base = DVec3::new(x as f64, y as f64, z as f64);
                    let touches = self.block_boxes(ctx, s, y).iter().any(|b| Aabb::new(base + DVec3::from(b.min), base + DVec3::from(b.max)).intersects(region));
                    if !touches {
                        continue;
                    }
                    let c = base + DVec3::splat(0.5) - self.pos;
                    let d = c.length_squared();
                    // The game keeps the greater position on a tie: by y, then z, then x.
                    let greater = best.is_none_or(|b| (y, z, x) > (b[1], b[2], b[0]));
                    if d < best_d || (d == best_d && greater) {
                        best = Some([x, y, z]);
                        best_d = d;
                    }
                }
            }
        }
        best
    }

    fn set_on_ground(&mut self, on: bool, moved: DVec3, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.on_ground = on;
        if on {
            let b = self.aabb();
            let strip = Aabb::new(DVec3::new(b.min.x, b.min.y - 1.0e-6, b.min.z), DVec3::new(b.max.x, b.min.y, b.max.z));
            let mut found = self.find_support(w, ctx, &strip);
            if found.is_some() || self.on_ground_no_blocks {
                self.support = found;
            } else {
                found = self.find_support(w, ctx, &strip.moved(DVec3::new(-moved.x, 0.0, -moved.z)));
                self.support = found;
            }
            self.on_ground_no_blocks = found.is_none();
        } else {
            self.on_ground_no_blocks = false;
            self.support = None;
        }
    }

    fn on_pos(&self, f: f32, w: &impl BlockAccess, ctx: &MoveCtx) -> [i32; 3] {
        if let Some(p) = self.support {
            if f <= 1.0e-5 {
                return p;
            }
            let idx = ctx.data.block_index(Self::state_at(w, p)) as u16;
            if (f as f64 <= 0.5 && ctx.tags.fences.contains(&idx)) || ctx.tags.walls_and_gates.contains(&idx) {
                return p;
            }
            return [p[0], floor(self.pos.y - f as f64), p[2]];
        }
        [floor(self.pos.x), floor(self.pos.y - f as f64), floor(self.pos.z)]
    }

    fn block_below_movement(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> [i32; 3] {
        self.on_pos(0.500001, w, ctx)
    }

    fn block_speed_factor(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> f32 {
        let data = ctx.data;
        let sp = special(data);
        let s = Self::state_at(w, self.block_pos());
        let b = data.block(s);
        let idx = data.block_index(s) as u16;
        let factor = if idx == sp.water || idx == sp.bubble_column {
            b.speed
        } else if b.speed as f64 != 1.0 {
            b.speed
        } else {
            data.block(Self::state_at(w, self.block_below_movement(w, ctx))).speed
        };
        let t = self.attrs.movement_efficiency as f32;
        factor + t * (1.0 - factor)
    }

    fn block_jump_factor(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> f32 {
        let here = ctx.data.block(Self::state_at(w, self.block_pos())).jump;
        let below = ctx.data.block(Self::state_at(w, self.block_below_movement(w, ctx))).jump;
        if here as f64 == 1.0 { below } else { here }
    }

    fn in_block_state(&mut self, w: &impl BlockAccess) -> u32 {
        *self.in_block_state.get_or_insert_with(|| Self::state_at(w, [floor(self.pos.x), floor(self.pos.y), floor(self.pos.z)]))
    }

    pub fn on_climbable(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) -> bool {
        if self.spectator {
            return false;
        }
        let s = self.in_block_state(w);
        let idx = ctx.data.block_index(s) as u16;
        if self.fall_flying && ctx.tags.can_glide_through.contains(&idx) {
            return false;
        }
        if ctx.tags.climbable.contains(&idx) {
            return true;
        }
        if ctx.tags.trapdoors.contains(&idx) {
            // An open trapdoor above a ladder facing the same way is climbable too.
            let props = ctx.data.properties(s);
            if props.iter().any(|(k, v)| *k == "open" && *v == "true") {
                let p = self.block_pos();
                let below = Self::state_at(w, [p[0], p[1] - 1, p[2]]);
                if ctx.data.block_index(below) as u16 == special(ctx.data).ladder {
                    let facing = |s: u32| ctx.data.properties(s).into_iter().find(|(k, _)| *k == "facing").map(|(_, v)| v.to_string());
                    return facing(s) == facing(below);
                }
            }
        }
        false
    }

    /// Moves by `m` with collisions, as the game's `Entity.move` does.
    fn do_move(&mut self, m: DVec3, w: &impl BlockAccess, ctx: &MoveCtx) {
        if self.no_physics {
            self.pos += m;
            self.steps.push(Step { from: self.pos - m, to: self.pos, original: Some(m) });
            self.horizontal_collision = false;
            self.vertical_collision = false;
            self.vertical_below = false;
            self.minor_horizontal_collision = false;
            return;
        }
        let mut m = m;
        if self.stuck.length_squared() > 1.0e-7 {
            m *= self.stuck;
            self.stuck = DVec3::ZERO;
            self.vel = DVec3::ZERO;
        }
        let m = self.back_off_from_edge(m, w, ctx);
        let c = self.collide(m, w, ctx);
        let len = c.length_squared();
        if len > 1.0e-7 || m.length_squared() - len >= 1.0e-7 {
            let from = self.pos;
            let before = self.block_pos();
            self.pos = from + c;
            if self.block_pos() != before {
                self.in_block_state = None;
            }
            self.steps.push(Step { from, to: self.pos, original: Some(m) });
        }
        let (xc, zc) = (!mth_equal(m.x, c.x), !mth_equal(m.z, c.z));
        self.horizontal_collision = xc || zc;
        self.vertical_collision = m.y != c.y;
        self.vertical_below = self.vertical_collision && m.y < 0.0;
        self.set_on_ground(self.vertical_below, c, w, ctx);
        self.minor_horizontal_collision = self.horizontal_collision && self.is_collision_minor(c);
        let on_pos = self.on_pos(0.2, w, ctx);
        let state = Self::state_at(w, on_pos);
        // checkFallDamage. Being in water is looked at again here, with the new position.
        if !self.in_water {
            self.update_fluids(w, ctx);
        }
        if !self.in_water && c.y < 0.0 {
            self.fall_distance -= (c.y as f32) as f64;
        }
        if self.on_ground {
            self.fall_distance = 0.0;
        }
        if (m.y.abs() > 0.0 && self.vertical_collision) || self.horizontal_collision {
            self.restitute(state, xc, zc, c, ctx);
        }
        let f = self.block_speed_factor(w, ctx) as f64;
        self.vel.x *= f;
        self.vel.z *= f;
    }

    fn is_collision_minor(&self, c: DVec3) -> bool {
        let r = self.yaw * DEG;
        let (s, co) = (mth_sin(r as f64) as f64, mth_cos(r as f64) as f64);
        let x = self.xxa as f64 * co - self.zza as f64 * s;
        let z = self.zza as f64 * co + self.xxa as f64 * s;
        let a = x * x + z * z;
        let b = c.x * c.x + c.z * c.z;
        if a < 9.999999747378752e-6 || b < 9.999999747378752e-6 {
            return false;
        }
        let dot = x * c.x + z * c.z;
        (dot / (a * b).sqrt()).acos() < 0.13962633907794952
    }

    /// Velocity after hitting something; only slime and beds give any of it back.
    fn restitute(&mut self, state: u32, xc: bool, zc: bool, c: DVec3, ctx: &MoveCtx) {
        let suppressing = self.keys.shift;
        let mut bounce = 0.0f64;
        let v = self.vel;
        let mut out = v;
        if xc {
            out.x = -v.x * bounce;
        }
        if zc {
            out.z = -v.z * bounce;
        }
        if self.vertical_collision {
            if self.vertical_below {
                let idx = ctx.data.block_index(state) as u16;
                bounce = if -v.y <= self.effective_gravity() || suppressing || ctx.tags.suppresses_bounce.contains(&idx) {
                    0.0
                } else {
                    bounce.max(ctx.data.block(state).bounce as f64)
                };
            }
            let (g, drag) = if bounce > 0.0 {
                let t = c.y / v.y;
                (t * self.effective_gravity(), 1.0 + t * (0.98f32 as f64 - 1.0))
            } else {
                (0.0, 1.0)
            };
            out.y = (g - v.y) * drag * bounce;
        }
        self.vel = out;
    }

    // --------------------------------------------------------------- fluids

    fn fluid_at(w: &impl BlockAccess, ctx: &MoveCtx, x: i32, y: i32, z: i32) -> u8 {
        w.state(x, y, z).map_or(0, |s| ctx.data.state(s).fluid)
    }

    fn own_height(ctx: &MoveCtx, state: u32) -> f32 {
        ctx.data.state(state).fluid_amount as f32 / 9.0
    }

    fn is_falling_fluid(ctx: &MoveCtx, state: u32) -> bool {
        // `level` 8 and up on a fluid block means falling; everything else holds a source.
        let b = ctx.data.block(state);
        (b.name == "water" || b.name == "lava") && state - b.first >= 8
    }

    fn is_solid_face(w: &impl BlockAccess, ctx: &MoveCtx, x: i32, y: i32, z: i32, dir: usize, kind: u8) -> bool {
        let Some(s) = w.state(x, y, z) else { return false };
        let info = ctx.data.state(s);
        if info.fluid == kind {
            return false;
        }
        if dir == 1 {
            return true;
        }
        let sp = special(ctx.data);
        let idx = ctx.data.block_index(s) as u16;
        if idx == sp.ice || idx == sp.frosted_ice {
            return false;
        }
        info.face_sturdy(dir)
    }

    fn flow(w: &impl BlockAccess, ctx: &MoveCtx, x: i32, y: i32, z: i32, state: u32) -> DVec3 {
        const HORIZONTAL: [(i32, i32, usize); 4] = [(0, -1, 2), (1, 0, 5), (0, 1, 3), (-1, 0, 4)];
        let kind = ctx.data.state(state).fluid;
        let own = Self::own_height(ctx, state);
        let (mut dx, mut dz) = (0.0f64, 0.0f64);
        let affects = |f: u8| f == 0 || f == kind;
        for (sx, sz, _) in HORIZONTAL {
            let (nx, nz) = (x + sx, z + sz);
            let n = w.state(nx, y, nz).unwrap_or(0);
            let ninfo = ctx.data.state(n);
            if !affects(ninfo.fluid) {
                continue;
            }
            let h = if ninfo.fluid == 0 { 0.0 } else { Self::own_height(ctx, n) };
            let mut d = 0.0f32;
            if h == 0.0 {
                if !ctx.tags.blocks_fluid_flow.contains(&(ctx.data.block_index(n) as u16)) {
                    let b = w.state(nx, y - 1, nz).unwrap_or(0);
                    let binfo = ctx.data.state(b);
                    if affects(binfo.fluid) && binfo.fluid != 0 {
                        let hb = Self::own_height(ctx, b);
                        if hb > 0.0 {
                            d = own - (hb - 0.8888889);
                        }
                    }
                }
            } else if h > 0.0 {
                d = own - h;
            }
            if d != 0.0 {
                dx += (sx as f32 * d) as f64;
                dz += (sz as f32 * d) as f64;
            }
        }
        let mut v = DVec3::new(dx, 0.0, dz);
        if Self::is_falling_fluid(ctx, state) {
            for (sx, sz, dir) in HORIZONTAL {
                if Self::is_solid_face(w, ctx, x + sx, y, z + sz, dir, kind) || Self::is_solid_face(w, ctx, x + sx, y + 1, z + sz, dir, kind) {
                    v = normalize(v) + DVec3::new(0.0, -6.0, 0.0);
                    break;
                }
            }
        }
        normalize(v)
    }

    /// Which fluids the player is in, how deep, and the pull of their currents.
    fn update_fluids(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.water_height = 0.0;
        self.lava_height = 0.0;
        self.eye_in_water = false;
        self.eye_in_lava = false;
        let pushed = !self.flying;
        let mut currents = [Current::default(); 2];
        let raw = self.aabb();
        let bb = raw.deflate(0.001);
        let (x0, y0, z0) = (floor(bb.min.x), floor(bb.min.y), floor(bb.min.z));
        let (x1, y1, z1) = (bb.max.x.ceil() as i32 - 1, bb.max.y.ceil() as i32 - 1, bb.max.z.ceil() as i32 - 1);
        let mut loaded = true;
        for cx in ((x0 - 1) >> 4)..=((x1 + 1) >> 4) {
            for cz in ((z0 - 1) >> 4)..=((z1 + 1) >> 4) {
                loaded &= w.state(cx * 16, y0, cz * 16).is_some();
            }
        }
        let mut trackers = [FluidTracker::default(); 4];
        if loaded {
            let [bx, _, bz] = self.block_pos();
            let eye = self.pos.y + self.eye_height() as f64;
            for x in x0..=x1 {
                for y in y0..=y1 {
                    for z in z0..=z1 {
                        let Some(s) = w.state(x, y, z) else { continue };
                        let info = ctx.data.state(s);
                        if info.fluid != 1 && info.fluid != 2 {
                            continue;
                        }
                        let kind = info.fluid as usize - 1;
                        let same_above = Self::fluid_at(w, ctx, x, y + 1, z) == info.fluid;
                        let height: f32 = if same_above { 1.0 } else { Self::own_height(ctx, s) };
                        let base = y as f64;
                        let top = base + height as f64;
                        if top < bb.min.y {
                            continue;
                        }
                        let t = kind * 2 + if info.has(flags::FLUID_SOURCE) { 0 } else { 1 };
                        if x == bx && z == bz && eye >= base {
                            let solid_above = info.has(flags::FLUID_SOURCE) && w.state(x, y + 1, z).is_some_and(|a| ctx.data.state(a).face_sturdy(0));
                            let cam: f32 = if solid_above { 1.0 } else { height };
                            if eye <= base + cam as f64 {
                                trackers[t].eyes = true;
                            }
                        }
                        trackers[t].height = (top - raw.min.y).max(trackers[t].height);
                        if pushed {
                            let c = &mut currents[kind];
                            let mut f = Self::flow(w, ctx, x, y, z, s);
                            c.height = trackers[t].height.max(c.height);
                            if c.height < 0.4 {
                                f *= c.height;
                            }
                            c.sum.0 += f;
                            c.count += 1;
                        }
                    }
                }
            }
        }
        self.water_height = trackers[0].height.max(trackers[1].height);
        self.lava_height = trackers[2].height.max(trackers[3].height);
        self.eye_in_water = trackers[0].eyes || trackers[1].eyes;
        self.eye_in_lava = trackers[2].eyes || trackers[3].eyes;
        self.in_water = self.water_height > 0.0;
        self.in_lava = self.lava_height > 0.0;
        if self.in_water {
            self.fall_distance = 0.0;
        }
        if pushed {
            if self.in_water {
                self.apply_current(currents[0], 0.014);
            }
            if self.in_lava {
                self.apply_current(currents[1], if ctx.fast_lava { 0.007 } else { 0.0023333333333333335 });
            }
        }
    }

    fn apply_current(&mut self, c: Current, scale: f64) {
        if c.count == 0 || c.sum.0.length_squared() < 9.999999747378752e-6 {
            return;
        }
        let mut v = c.sum.0 * (1.0 / c.count as f64) * scale;
        if self.vel.x.abs() < 0.003 && self.vel.z.abs() < 0.003 && v.length() < 0.0045000000000000005 {
            v = normalize(v) * 0.0045000000000000005;
        }
        self.vel += v;
    }

    fn update_swimming(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        if self.flying {
            self.swimming_flag = false;
        } else if self.is_swimming() {
            self.swimming_flag = self.sprinting && self.in_water && !self.passenger;
        } else {
            let [x, y, z] = self.block_pos();
            self.swimming_flag = self.sprinting && self.under_water && !self.passenger && Self::fluid_at(w, ctx, x, y, z) == 1;
        }
    }

    fn fluid_jump_threshold(&self) -> f64 {
        if (self.eye_height() as f64) < 0.4 { 0.0 } else { 0.4 }
    }

    // -------------------------------------------------------------- travel

    fn speed(&self) -> f32 {
        let v = if self.sprinting { self.attrs.movement_speed * 1.3 } else { self.attrs.movement_speed };
        v as f32
    }

    fn flying_speed(&self) -> f32 {
        if self.ridden {
            return self.speed() * 0.1;
        }
        if self.flying && !self.passenger {
            return if self.sprinting { self.fly_speed * 2.0 } else { self.fly_speed };
        }
        if self.sprinting { 0.025999999 } else { 0.02 }
    }

    fn effective_gravity(&self) -> f64 {
        if self.vel.y <= 0.0 && self.effects.slow_falling {
            return self.attrs.gravity.min(0.01);
        }
        self.attrs.gravity
    }

    fn move_relative(&mut self, speed: f32) {
        let input = DVec3::new(self.xxa as f64, self.yya as f64, self.zza as f64);
        let len = input.length_squared();
        if len < 1.0e-7 {
            return;
        }
        let v = if len > 1.0 { normalize(input) } else { input } * speed as f64;
        let r = (self.yaw * DEG) as f64;
        let (s, c) = (mth_sin(r) as f64, mth_cos(r) as f64);
        self.vel += DVec3::new(v.x * c - v.z * s, v.y, v.z * c + v.x * s);
    }

    fn fluid_falling_adjusted(&self, gravity: f64, falling: bool, v: DVec3) -> DVec3 {
        if gravity != 0.0 && !self.sprinting {
            let y = if falling && (v.y - 0.005).abs() >= 0.003 && (v.y - gravity / 16.0).abs() < 0.003 { -0.003 } else { v.y - gravity / 16.0 };
            return DVec3::new(v.x, y, v.z);
        }
        v
    }

    fn jump_out_of_fluid(&mut self, old_y: f64, w: &impl BlockAccess, ctx: &MoveCtx) {
        let v = self.vel;
        if self.horizontal_collision && self.is_free(w, ctx, DVec3::new(v.x, v.y + 0.6000000238418579 - self.pos.y + old_y, v.z)) {
            self.vel.y = 0.30000001192092896;
        }
    }

    fn travel_in_water(&mut self, gravity: f64, falling: bool, old_y: f64, w: &impl BlockAccess, ctx: &MoveCtx) {
        let mut slow: f32 = if self.sprinting { 0.9 } else { 0.8 };
        let mut speed: f32 = 0.02;
        let mut eff = self.attrs.water_movement_efficiency as f32;
        if !self.on_ground {
            eff *= 0.5;
        }
        if eff > 0.0 {
            slow += (0.54600006 - slow) * eff;
            speed += (self.speed() - speed) * eff;
        }
        if self.effects.dolphins_grace {
            slow = 0.96;
        }
        self.move_relative(speed);
        self.do_move(self.vel, w, ctx);
        let mut v = self.vel;
        if self.horizontal_collision && self.on_climbable(w, ctx) {
            v.y = 0.2;
        }
        let v = DVec3::new(v.x * slow as f64, v.y * 0.800000011920929, v.z * slow as f64);
        self.vel = self.fluid_falling_adjusted(gravity, falling, v);
        self.jump_out_of_fluid(old_y, w, ctx);
    }

    fn travel_in_lava(&mut self, gravity: f64, falling: bool, old_y: f64, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.move_relative(0.02);
        self.do_move(self.vel, w, ctx);
        if self.lava_height <= self.fluid_jump_threshold() {
            let v = self.vel * DVec3::new(0.5, 0.800000011920929, 0.5);
            self.vel = self.fluid_falling_adjusted(gravity, falling, v);
        } else {
            self.vel *= 0.5;
        }
        if gravity != 0.0 {
            self.vel.y += -gravity / 4.0;
        }
        self.jump_out_of_fluid(old_y, w, ctx);
    }

    fn travel_in_air(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        let below = self.block_below_movement(w, ctx);
        let modified = |f: f32, m: f32| (1.0 - (1.0 - f) * m).clamp(0.0, 1.0);
        let friction: f32 = if self.on_ground {
            modified(ctx.data.block(Self::state_at(w, below)).friction, self.attrs.friction_modifier as f32)
        } else {
            1.0
        };
        // handleRelativeFrictionAndCalculateMovement
        let speed = if self.on_ground {
            if friction as f64 <= 0.6 { self.speed() } else { self.speed() * (0.21600002 / (friction * friction * friction)) }
        } else {
            self.flying_speed()
        };
        self.move_relative(speed);
        if self.on_climbable(w, ctx) {
            self.fall_distance = 0.0;
            let v = self.vel;
            let mut y = v.y.max(-0.15000000596046448);
            let scaffolding = ctx.data.block_index(self.in_block_state(w)) as u16 == special(ctx.data).scaffolding;
            if y < 0.0 && !scaffolding && self.keys.shift {
                y = 0.0;
            }
            self.vel = DVec3::new(v.x.clamp(-0.15000000596046448, 0.15000000596046448), y, v.z.clamp(-0.15000000596046448, 0.15000000596046448));
        }
        self.do_move(self.vel, w, ctx);
        let mut v = self.vel;
        if (self.horizontal_collision || self.jumping) && (self.on_climbable(w, ctx) || (self.was_in_powder_snow && ctx.leather_boots)) {
            v.y = 0.2;
        }

        let mut y = v.y;
        if let Some(amp) = self.effects.levitation {
            y += (0.05 * (amp + 1) as f64 - v.y) * 0.2;
        } else if w.state(below[0], below[1], below[2]).is_some() {
            y -= self.effective_gravity();
        } else if self.pos.y > ctx.min_y as f64 {
            y = -0.1;
        } else {
            y = 0.0;
        }
        let drag_mod = self.attrs.air_drag_modifier as f32;
        let horizontal = friction * modified(0.91, drag_mod);
        let vertical = modified(0.98, drag_mod);
        self.vel = DVec3::new(v.x * horizontal as f64, y * vertical as f64, v.z * horizontal as f64);
    }

    fn travel_gliding(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        if self.on_climbable(w, ctx) {
            self.travel_in_air(w, ctx);
            self.fall_flying = false;
            return;
        }
        let look = self.look_angle();
        let pitch = self.pitch * DEG;
        let l = (look.x * look.x + look.z * look.z).sqrt();
        let mut v = self.vel;
        let h = (v.x * v.x + v.z * v.z).sqrt();
        let g = self.effective_gravity();
        let c2 = (pitch as f64).cos().powi(2);
        v.y += g * (-1.0 + c2 * 0.75);
        if v.y < 0.0 && l > 0.0 {
            let k = v.y * -0.1 * c2;
            v += DVec3::new(look.x * k / l, k, look.z * k / l);
        }
        if pitch < 0.0 && l > 0.0 {
            let k = h * (-mth_sin(pitch as f64)) as f64 * 0.04;
            v += DVec3::new(-look.x * k / l, k * 3.2, -look.z * k / l);
        }
        if l > 0.0 {
            v.x += (look.x / l * h - v.x) * 0.1;
            v.z += (look.z / l * h - v.z) * 0.1;
        }
        self.vel = v * DVec3::new(0.9900000095367432, 0.9800000190734863, 0.9900000095367432);
        self.do_move(self.vel, w, ctx);
    }

    fn look_angle(&self) -> DVec3 {
        let f = self.pitch * DEG;
        let g = -self.yaw * DEG;
        let (h, i) = (mth_cos(g as f64), mth_sin(g as f64));
        let (j, k) = (mth_cos(f as f64), mth_sin(f as f64));
        DVec3::new((i * j) as f64, (-k) as f64, (h * j) as f64)
    }

    fn travel(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        if self.is_swimming() {
            let look_y = self.look_angle().y;
            let k = if look_y < -0.2 { 0.085 } else { 0.06 };
            let above = Self::fluid_at(w, ctx, floor(self.pos.x), floor(self.pos.y + 1.0 - 0.1), floor(self.pos.z));
            if look_y <= 0.0 || self.jumping || above != 0 {
                self.vel.y += (look_y - self.vel.y) * k;
            }
        }
        let flying_y = self.flying.then_some(self.vel.y);
        if (self.in_water || self.in_lava) && !(self.in_lava && self.stands_on_lava) {
            let falling = self.vel.y <= 0.0;
            let old_y = self.pos.y;
            let g = self.effective_gravity();
            if self.in_water {
                self.travel_in_water(g, falling, old_y, w, ctx);
            } else {
                self.travel_in_lava(g, falling, old_y, w, ctx);
            }
        } else if self.fall_flying {
            self.travel_gliding(w, ctx);
        } else {
            self.travel_in_air(w, ctx);
        }
        if let Some(y) = flying_y {
            self.vel.y = y * 0.6;
        }
    }

    // -------------------------------------------------- blocks the body is in

    fn apply_block_effects(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        let mut steps = std::mem::take(&mut self.steps);
        match steps.last() {
            None => steps.push(Step { from: self.old_pos, to: self.pos, original: None }),
            Some(l) if l.to.distance_squared(self.pos) > 9.999999439624929e-11 => {
                steps.push(Step { from: l.to, to: self.pos, original: None });
            }
            _ => {}
        }
        if self.no_physics {
            return;
        }
        let sp = special(ctx.data);
        if self.on_ground {
            let on = self.on_pos(0.2, w, ctx);
            if ctx.data.block_index(Self::state_at(w, on)) as u16 == sp.slime {
                let y = self.vel.y.abs();
                if y < 0.1 && !self.keys.shift {
                    let k = 0.4 + y * 0.2;
                    self.vel.x *= k;
                    self.vel.z *= k;
                }
            }
        }
        let mut visited: HashSet<[i32; 3]> = HashSet::new();
        for step in steps {
            let delta = step.to - step.from;
            match step.original {
                Some(orig) if delta.length_squared() > 0.0 => {
                    let mut at = step.from;
                    let order: [usize; 3] = if orig.x.abs() < orig.z.abs() { [1, 2, 0] } else { [1, 0, 2] };
                    for axis in order {
                        if delta[axis] != 0.0 {
                            let mut next = at;
                            next[axis] += delta[axis];
                            self.inside_blocks(at, next, &mut visited, w, ctx);
                            at = next;
                        }
                    }
                }
                _ => self.inside_blocks(step.from, step.to, &mut visited, w, ctx),
            }
        }
    }

    /// Effects of the blocks touched moving from `from` to `to`.
    fn inside_blocks(&mut self, from: DVec3, to: DVec3, visited: &mut HashSet<[i32; 3]>, w: &impl BlockAccess, ctx: &MoveCtx) {
        let sp = special(ctx.data);
        let bb = self.box_at(to, self.pose).deflate(9.999999747378752e-6);
        let far = from.distance_squared(to) > 0.9999900000002526f64 * 0.9999900000002526;
        let d = to - from;
        let swept = Aabb::new(bb.min.min(bb.min - d), bb.max.max(bb.max - d));
        for x in floor(swept.min.x)..=floor(swept.max.x) {
            for y in floor(swept.min.y)..=floor(swept.max.y) {
                for z in floor(swept.min.z)..=floor(swept.max.z) {
                    let Some(s) = w.state(x, y, z) else { continue };
                    if s == 0 || ctx.data.state(s).is_air() {
                        continue;
                    }
                    let idx = ctx.data.block_index(s) as u16;
                    let interesting = idx == sp.cobweb || idx == sp.berry || idx == sp.powder_snow || idx == sp.honey || idx == sp.bubble_column;
                    if !interesting || !visited.insert([x, y, z]) {
                        continue;
                    }
                    let cell = Aabb::new(DVec3::new(x as f64, y as f64, z as f64), DVec3::new(x as f64 + 1.0, y as f64 + 1.0, z as f64 + 1.0));
                    if idx == sp.cobweb {
                        self.fall_distance = 0.0;
                        self.stuck = if self.effects.weaving { DVec3::new(0.5, 0.25, 0.5) } else { DVec3::new(0.25, 0.05000000074505806, 0.25) };
                    } else if idx == sp.berry {
                        self.fall_distance = 0.0;
                        self.stuck = DVec3::new(0.800000011920929, 0.75, 0.800000011920929);
                    } else if idx == sp.powder_snow {
                        // The shape counts here, so walking on top isn't being inside.
                        let shape = self.block_boxes(ctx, s, y);
                        let inside = shape.is_empty()
                            || shape.iter().any(|b| Aabb::new(cell.min + DVec3::from(b.min), cell.min + DVec3::from(b.max)).intersects(&bb));
                        if inside {
                            if ctx.data.block_index(self.in_block_state(w)) as u16 == sp.powder_snow {
                                self.fall_distance = 0.0;
                                self.stuck = DVec3::new(0.8999999761581421, 1.5, 0.8999999761581421);
                            }
                            self.in_powder_snow = true;
                        }
                    } else if idx == sp.honey {
                        self.honey_slide(x, y, z);
                    } else if idx == sp.bubble_column && (far || cell.intersects(&bb)) {
                        let drag = s != ctx.data.block(s).default && Self::bubble_drags(ctx, s);
                        let drag = drag || Self::bubble_drags(ctx, s);
                        let above = w.state(x, y + 1, z).unwrap_or(0);
                        let open = ctx.data.collision(above).is_empty() && ctx.data.state(above).fluid == 0;
                        let v = self.vel;
                        if open {
                            self.vel.y = if drag { (-0.9f64).max(v.y - 0.03) } else { 1.8f64.min(v.y + 0.1) };
                        } else {
                            self.vel.y = if drag { (-0.3f64).max(v.y - 0.03) } else { 0.7f64.min(v.y + 0.06) };
                            self.fall_distance = 0.0;
                        }
                    }
                }
            }
        }
    }

    fn bubble_drags(ctx: &MoveCtx, s: u32) -> bool {
        ctx.data.properties(s).iter().any(|(k, v)| *k == "drag" && *v == "true")
    }

    fn honey_slide(&mut self, x: i32, y: i32, z: i32) {
        let old_dy = |v: f64| v / 0.9800000190734863 + 0.08;
        let new_dy = |v: f64| (v - 0.08) * 0.9800000190734863;
        if self.on_ground || self.pos.y > y as f64 + 0.9375 - 1.0e-7 || old_dy(self.vel.y) >= -0.08 {
            return;
        }
        let (dx, dz) = ((x as f64 + 0.5 - self.pos.x).abs(), (z as f64 + 0.5 - self.pos.z).abs());
        let reach = 0.4375 + (self.dims(self.pose).0 / 2.0) as f64;
        if dx + 1.0e-7 <= reach && dz + 1.0e-7 <= reach {
            return;
        }
        let v = self.vel;
        let old = old_dy(v.y);
        self.vel = if old < -0.13 {
            let k = -0.05 / old;
            DVec3::new(v.x * k, new_dy(-0.05), v.z * k)
        } else {
            DVec3::new(v.x, new_dy(-0.05), v.z)
        };
        self.fall_distance = 0.0;
    }

    // ---------------------------------------------------------- player rules

    fn suffocates_at(&self, x: i32, z: i32, w: &impl BlockAccess, ctx: &MoveCtx) -> bool {
        let b = self.aabb();
        let region = Aabb::new(DVec3::new(x as f64, b.min.y, z as f64), DVec3::new(x as f64 + 1.0, b.max.y, z as f64 + 1.0)).deflate(1.0e-7);
        for y in floor(region.min.y) - 1..=floor(region.max.y) + 1 {
            let Some(s) = w.state(x, y, z) else { continue };
            if !ctx.data.state(s).has(flags::SUFFOCATING) {
                continue;
            }
            let base = DVec3::new(x as f64, y as f64, z as f64);
            if self.block_boxes(ctx, s, y).iter().any(|c| Aabb::new(base + DVec3::from(c.min), base + DVec3::from(c.max)).intersects(&region)) {
                return true;
            }
        }
        false
    }

    /// Pushes the player out of a block they're stuck inside.
    fn move_towards_closest_space(&mut self, px: f64, pz: f64, w: &impl BlockAccess, ctx: &MoveCtx) {
        let (bx, bz) = (floor(px), floor(pz));
        if !self.suffocates_at(bx, bz, w, ctx) {
            return;
        }
        let (fx, fz) = (px - bx as f64, pz - bz as f64);
        // West, east, north, south; the closest free side wins.
        let sides: [(i32, i32, f64); 4] = [(-1, 0, fx), (1, 0, 1.0 - fx), (0, -1, fz), (0, 1, 1.0 - fz)];
        let mut best: Option<(i32, i32)> = None;
        let mut best_d = f64::MAX;
        for (dx, dz, d) in sides {
            if d < best_d && !self.suffocates_at(bx + dx, bz + dz, w, ctx) {
                best_d = d;
                best = Some((dx, dz));
            }
        }
        match best {
            Some((dx, 0)) => self.vel.x = 0.1 * dx as f64,
            Some((_, dz)) => self.vel.z = 0.1 * dz as f64,
            None => {}
        }
    }

    fn sprinting_possible(&self, ctx: &MoveCtx, ignore_shallow: bool) -> bool {
        !self.effects.blindness && (self.passenger || ctx.enough_food) && (ignore_shallow || !self.in_shallow_water())
    }

    fn in_shallow_water(&self) -> bool {
        self.in_water && !self.under_water
    }

    fn forward_impulse(&self) -> bool {
        self.move_vector.1 > 1.0e-5
    }

    fn moving_slowly(&self) -> bool {
        self.crouching || (self.pose == Pose::Swimming && !self.in_water)
    }

    fn slow_using_item(&self, ctx: &MoveCtx) -> bool {
        ctx.item_use_speed.is_some() && !ctx.item_use_can_sprint
    }

    fn can_start_sprinting(&self, ctx: &MoveCtx) -> bool {
        !self.sprinting
            && self.forward_impulse()
            && self.sprinting_possible(ctx, self.flying)
            && !self.slow_using_item(ctx)
            && (!self.fall_flying || self.under_water)
            && (!self.moving_slowly() || self.under_water)
    }

    fn modified_input(&self, ctx: &MoveCtx) -> (f32, f32) {
        let (x, y) = self.move_vector;
        if x * x + y * y == 0.0 {
            return (x, y);
        }
        let (mut x, mut y) = (x * 0.98, y * 0.98);
        if let Some(m) = ctx.item_use_speed {
            if !self.passenger {
                x *= m;
                y *= m;
            }
        }
        if self.moving_slowly() {
            let s = self.attrs.sneaking_speed as f32;
            x *= s;
            y *= s;
        }
        // Diagonal keys move as fast as the square's corner allows, up to full speed.
        let len = (x * x + y * y).sqrt();
        if len <= 0.0 {
            return (x, y);
        }
        let (ux, uy) = (x * (1.0 / len), y * (1.0 / len));
        let (a, b) = (ux.abs(), uy.abs());
        let r = if b > a { a / b } else { b / a };
        let k = (1.0 + r * r).sqrt();
        let l = (len * k).min(1.0);
        (ux * l, uy * l)
    }

    fn jump_from_ground(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        let boost = self.effects.jump_boost.map_or(0.0, |a| 0.1 * (a as f32 + 1.0));
        let power = (self.attrs.jump_strength as f32 * 1.0) * self.block_jump_factor(w, ctx) + boost;
        if power <= 1.0e-5 {
            return;
        }
        self.vel.y = (power as f64).max(self.vel.y);
        if self.sprinting {
            let r = (self.yaw * DEG) as f64;
            self.vel += DVec3::new(-mth_sin(r) as f64 * 0.2, 0.0, mth_cos(r) as f64 * 0.2);
        }
        self.jumped = true;
    }

    fn update_auto_jump(&mut self, dx: f32, dz: f32, w: &impl BlockAccess, ctx: &MoveCtx) {
        let can = ctx.auto_jump
            && self.auto_jump_time <= 0
            && self.on_ground
            && !self.keys.shift
            && !self.passenger
            && (self.move_vector.0 != 0.0 || self.move_vector.1 != 0.0)
            && self.block_jump_factor(w, ctx) as f64 >= 1.0;
        if !can {
            return;
        }
        let from = self.pos;
        let to = from + DVec3::new(dx as f64, 0.0, dz as f64);
        let mut dir = DVec3::new(dx as f64, 0.0, dz as f64);
        let speed = self.speed();
        let mut len2 = dir.length_squared() as f32;
        if len2 <= 0.001 {
            let (ix, iz) = (speed * self.move_vector.0, speed * self.move_vector.1);
            let r = (self.yaw * DEG) as f64;
            let (s, c) = (mth_sin(r), mth_cos(r));
            dir = DVec3::new((ix * c - iz * s) as f64, dir.y, (iz * c + ix * s) as f64);
            len2 = dir.length_squared() as f32;
            if len2 <= 0.001 {
                return;
            }
        }
        let inv = 1.0 / len2.sqrt();
        let unit = dir * inv as f64;
        let r = (self.yaw * DEG) as f64;
        let forward = DVec3::new(-mth_sin(r) as f64, 0.0, mth_cos(r) as f64);
        if ((forward.x * unit.x + forward.z * unit.z) as f32) < -0.15 {
            return;
        }
        let top = floor(self.aabb().max.y);
        let (bx, bz) = (floor(self.pos.x), floor(self.pos.z));
        let clear = |y: i32| w.state(bx, y, bz).is_none_or(|s| ctx.data.collision(s).is_empty());
        if !clear(top) || !clear(top + 1) {
            return;
        }
        let mut reach: f32 = 1.2;
        if let Some(a) = self.effects.jump_boost {
            reach += (a + 1) as f32 * 0.75;
        }
        let dist = (speed * 7.0).max(1.0 / inv);
        let end = to + unit * dist as f64;
        let (width, height, _) = self.dims(self.pose);
        let area = Aabb::new(from.min(end), from.max(end + DVec3::new(0.0, height as f64, 0.0)))
            .deflate(-(width as f64))
            .with_y(from.y.min(end.y), from.y.max(end.y) + height as f64);
        let lift = DVec3::new(0.0, 0.5099999904632568, 0.0);
        let (a, b) = (from + lift, end + lift);
        let side = unit.cross(DVec3::Y) * (width * 0.5) as f64;
        let rays = [(a - side, b - side), (a + side, b + side)];
        let mut found: Option<f32> = None;
        'search: for bx2 in self.boxes(w, ctx, &area) {
            if !rays.iter().any(|(p, q)| segment_hits(*p, *q, &bx2)) {
                continue;
            }
            let mut height_there = bx2.max.y as f32;
            let centre = (bx2.min + bx2.max) * 0.5;
            let (cx, cy, cz) = (floor(centre.x), floor(centre.y), floor(centre.z));
            for i in 1..(reach.ceil() as i32) {
                let s = w.state(cx, cy + i, cz).unwrap_or(0);
                let shape = ctx.data.collision(s);
                if !shape.is_empty() {
                    let max = shape.iter().map(|c| c.max[1]).fold(0.0, f64::max);
                    height_there = max as f32 + (cy + i) as f32;
                    if (height_there as f64 - self.pos.y) > reach as f64 {
                        return;
                    }
                }
                if i > 1 && !clear(top + i) {
                    return;
                }
            }
            found = Some(height_there);
            break 'search;
        }
        let Some(h) = found else { return };
        let rise = (h as f64 - self.pos.y) as f32;
        if rise > 0.5 && rise <= reach {
            self.auto_jump_time = 1;
        }
    }

    fn update_pose(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        if !self.fits(w, ctx, Pose::Swimming) {
            return;
        }
        let desired = if self.pose == Pose::Sleeping {
            Pose::Sleeping
        } else if self.is_swimming() {
            Pose::Swimming
        } else if self.fall_flying {
            Pose::FallFlying
        } else if self.keys.shift && !self.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        self.pose = if self.spectator || self.passenger || self.fits(w, ctx, desired) {
            desired
        } else if self.fits(w, ctx, Pose::Crouching) {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
    }

    /// Everything the game does in `LocalPlayer.aiStep` and below it.
    fn ai_step(&mut self, keys: Keys, w: &impl BlockAccess, ctx: &MoveCtx) {
        if self.sprint_trigger > 0 {
            self.sprint_trigger -= 1;
        }
        let prev_jump = self.keys.jump;
        let prev_shift = self.keys.shift;
        let prev_forward = self.forward_impulse();
        let sleeping = self.pose == Pose::Sleeping;
        self.crouching = !self.flying
            && !self.is_swimming()
            && !self.passenger
            && self.fits(w, ctx, Pose::Crouching)
            && (self.keys.shift || (!sleeping && !self.fits(w, ctx, Pose::Standing)));

        self.keys = keys;
        let x = (keys.left as i32 - keys.right as i32) as f32;
        let y = (keys.forward as i32 - keys.back as i32) as f32;
        let len = (x * x + y * y).sqrt();
        self.move_vector = if len < 1.0e-4 { (0.0, 0.0) } else { (x / len, y / len) };

        let mut auto_jumped = false;
        if self.auto_jump_time > 0 {
            self.auto_jump_time -= 1;
            auto_jumped = true;
            self.keys.jump = true;
        }
        if !self.no_physics && !self.passenger {
            let w2 = (self.dims(self.pose).0 as f64) * 0.35;
            let (px, pz) = (self.pos.x, self.pos.z);
            self.move_towards_closest_space(px - w2, pz + w2, w, ctx);
            self.move_towards_closest_space(px - w2, pz - w2, w, ctx);
            self.move_towards_closest_space(px + w2, pz - w2, w, ctx);
            self.move_towards_closest_space(px + w2, pz + w2, w, ctx);
        }

        if prev_shift || (self.slow_using_item(ctx) && !self.passenger) || self.keys.back {
            self.sprint_trigger = 0;
        }
        if self.can_start_sprinting(ctx) {
            if !prev_forward {
                if self.sprint_trigger > 0 {
                    self.sprinting = true;
                } else {
                    self.sprint_trigger = ctx.sprint_window;
                }
            }
            if self.keys.sprint {
                self.sprinting = true;
            }
        }
        if self.sprinting {
            let stop = if self.is_swimming() {
                !self.sprinting_possible(ctx, true) || !self.in_water || (!self.forward_impulse() && !self.on_ground && !self.keys.shift)
            } else {
                !self.sprinting_possible(ctx, self.flying) || !self.forward_impulse() || (self.horizontal_collision && !self.minor_horizontal_collision)
            };
            if stop {
                self.sprinting = false;
            }
        }

        let mut changed = false;
        if self.may_fly {
            if self.spectator {
                if !self.flying {
                    self.flying = true;
                    changed = true;
                    self.abilities_changed = true;
                }
            } else if !prev_jump && self.keys.jump && !auto_jumped {
                if self.jump_trigger == 0 {
                    self.jump_trigger = 7;
                } else if !self.is_swimming() && !self.passenger {
                    self.flying = !self.flying;
                    if self.flying && self.on_ground {
                        self.jump_from_ground(w, ctx);
                    }
                    changed = true;
                    self.abilities_changed = true;
                    self.jump_trigger = 0;
                }
            }
        }
        if self.keys.jump && !changed && !prev_jump && !self.on_climbable(w, ctx) && !self.fall_flying && !self.on_ground && !self.passenger && !self.effects.levitation.is_some() && ctx.can_glide && !(self.in_water || self.in_lava) {
            self.fall_flying = true;
            self.start_gliding = true;
        }
        if self.in_water && self.keys.shift {
            self.vel.y += -0.03999999910593033;
        }
        if self.flying {
            let d = self.keys.jump as i32 - self.keys.shift as i32;
            if d != 0 {
                self.vel.y += (d as f32 * self.fly_speed * 3.0) as f64;
            }
        }

        // Player.aiStep
        if self.jump_trigger > 0 {
            self.jump_trigger -= 1;
        }
        if self.flying && !self.passenger {
            self.fall_distance = 0.0;
        }

        // LivingEntity.aiStep
        if self.no_jump_delay > 0 {
            self.no_jump_delay -= 1;
        }
        let v = self.vel;
        let (mut vx, mut vy, mut vz) = (v.x, v.y, v.z);
        if v.x * v.x + v.z * v.z < 9.0e-6 {
            vx = 0.0;
            vz = 0.0;
        }
        if v.y.abs() < 0.003 {
            vy = 0.0;
        }
        self.vel = DVec3::new(vx, vy, vz);

        let (xxa, zza) = self.modified_input(ctx);
        self.xxa = xxa;
        self.zza = zza;
        self.jumping = self.keys.jump;
        if self.immobile {
            self.jumping = false;
            self.xxa = 0.0;
            self.zza = 0.0;
        }

        if self.jumping {
            let height = if self.in_lava { self.lava_height } else { self.water_height };
            let in_water = self.in_water && height > 0.0;
            let threshold = self.fluid_jump_threshold();
            if in_water && (!self.on_ground || height > threshold) {
                self.vel.y += 0.03999999910593033;
            } else if self.in_lava && (!self.on_ground || self.lava_height > threshold) {
                self.vel.y += 0.03999999910593033;
            } else if (self.on_ground || (in_water && height <= threshold)) && self.no_jump_delay == 0 {
                self.jump_from_ground(w, ctx);
                self.no_jump_delay = 10;
            }
        } else {
            self.no_jump_delay = 0;
        }

        if self.effects.slow_falling || self.effects.levitation.is_some() {
            self.fall_distance = 0.0;
        }

        let before = self.pos;
        self.travel(w, ctx);
        let moved = self.pos - before;
        self.update_auto_jump(moved.x as f32, moved.z as f32, w, ctx);
        self.apply_block_effects(w, ctx);

        if self.on_ground && self.flying && !self.spectator {
            self.flying = false;
            self.abilities_changed = true;
        }
    }

    /// One 50 ms tick.
    pub fn tick(&mut self, keys: Keys, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.old_pos = self.pos;
        self.jumped = false;
        self.no_physics = self.spectator;
        if self.spectator || self.passenger {
            self.on_ground = false;
        }
        // Player.tick begins by noting where the eyes were.
        self.under_water = self.eye_in_water;
        // Entity.baseTick
        self.in_block_state = None;
        self.was_in_powder_snow = self.in_powder_snow;
        self.in_powder_snow = false;
        self.update_fluids(w, ctx);
        self.update_swimming(w, ctx);
        if self.in_lava {
            self.fall_distance *= 0.5;
        }
        self.ai_step(keys, w, ctx);
        self.update_pose(w, ctx);
    }
}

/// What a rider asks of the animal under them for one tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct RideInput {
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    /// Moves by this much per unit of input wherever it is, and doesn't fall.
    pub flying: Option<f32>,
    /// Swims with its own speed, and this much of it kept each tick.
    pub swims: Option<f64>,
    /// Walks on lava.
    pub lava_walker: bool,
}

impl Body {
    /// A body for something being driven rather than a player.
    pub fn for_vehicle(data: &GameData, width: f32, height: f32, pos: DVec3, yaw: f32) -> Body {
        let mut b = Body::new(data);
        b.fixed_dims = Some((width, height));
        b.pos = pos;
        b.old_pos = pos;
        b.yaw = yaw;
        b.attrs.step_height = 0.0;
        b.flying = false;
        b.may_fly = false;
        b
    }

    /// The block counted as under the feet: exactly, and the older, looser way.
    pub fn step_blocks(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> ([i32; 3], [i32; 3]) {
        (self.on_pos(1.0e-5, w, ctx), self.on_pos(0.2, w, ctx))
    }

    /// The left-right and forward-back a rider's keys come to.
    pub fn rider_axes(&mut self, keys: Keys, ctx: &MoveCtx) -> (f32, f32) {
        self.keys = keys;
        self.crouching = false;
        let x = (keys.left as i32 - keys.right as i32) as f32;
        let y = (keys.forward as i32 - keys.back as i32) as f32;
        let len = (x * x + y * y).sqrt();
        self.move_vector = if len < 1.0e-4 { (0.0, 0.0) } else { (x / len, y / len) };
        self.modified_input(ctx)
    }

    /// Notices what fluid someone sitting here is in, without moving them.
    pub fn sit_tick(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.under_water = self.eye_in_water;
        self.update_fluids(w, ctx);
        self.on_ground = false;
        self.fall_distance = 0.0;
    }

    /// How hard a jump of this strength pushes off from the block underneath.
    pub fn jump_power(&self, strength: f64, scale: f32, w: &impl BlockAccess, ctx: &MoveCtx) -> f64 {
        let boost = self.effects.jump_boost.map_or(0.0, |a| 0.1 * (a as f32 + 1.0));
        ((strength as f32 * scale) * self.block_jump_factor(w, ctx) + boost) as f64
    }

    pub fn speed_factor(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> f64 {
        self.block_speed_factor(w, ctx) as f64
    }

    /// One tick of an animal with a player steering it.
    pub fn ride_tick(&mut self, input: RideInput, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.old_pos = self.pos;
        self.jumped = false;
        self.in_block_state = None;
        self.was_in_powder_snow = self.in_powder_snow;
        self.in_powder_snow = false;
        self.ridden = true;
        self.stands_on_lava = input.lava_walker;
        self.update_fluids(w, ctx);
        if self.in_lava {
            self.fall_distance *= 0.5;
        }
        // Floats on the lava, as a strider does, unless it's sunk in deep.
        if input.lava_walker && self.in_lava {
            let [x, y, z] = self.block_pos();
            let above_stable = self.pos.y > y as f64 + 0.5 - 1.0e-5f32 as f64;
            if above_stable && Self::fluid_at(w, ctx, x, y + 1, z) != Self::fluid_at(w, ctx, x, y, z) {
                self.on_ground = true;
            } else {
                self.vel = self.vel * 0.5 + DVec3::new(0.0, 0.05, 0.0);
            }
        }
        let v = self.vel;
        let tiny = |a: f64| if a.abs() < 0.003 { 0.0 } else { a };
        self.vel = DVec3::new(tiny(v.x), tiny(v.y), tiny(v.z));
        self.xxa = input.xxa;
        self.yya = input.yya;
        self.zza = input.zza;
        self.jumping = false;
        self.sprinting = false;
        self.attrs.movement_speed = input.speed as f64;
        if let Some(speed) = input.flying {
            self.move_relative(speed);
            self.do_move(self.vel, w, ctx);
            let keep = if self.in_water {
                0.8
            } else if self.in_lava {
                0.5
            } else {
                0.91
            };
            self.vel *= keep;
        } else if let (Some(keep), true) = (input.swims, self.in_water) {
            self.move_relative(input.speed);
            self.do_move(self.vel, w, ctx);
            self.vel *= keep;
        } else {
            self.travel(w, ctx);
        }
        self.apply_block_effects(w, ctx);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoatStatus {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}

/// A boat being paddled by this player.
#[derive(Debug, Clone)]
pub struct Boat {
    pub body: Body,
    pub status: BoatStatus,
    old_status: BoatStatus,
    water_level: f64,
    land_friction: f32,
    last_yd: f64,
    /// Turning, degrees per tick; the rider turns along with it.
    pub delta_rotation: f32,
    pub paddle_left: bool,
    pub paddle_right: bool,
}

const BOAT_GRAVITY: f64 = 0.04;

impl Boat {
    pub fn new(data: &GameData, width: f32, height: f32, pos: DVec3, yaw: f32) -> Boat {
        Boat {
            body: Body::for_vehicle(data, width, height, pos, yaw),
            status: BoatStatus::InAir,
            old_status: BoatStatus::InAir,
            water_level: 0.0,
            land_friction: 0.0,
            last_yd: 0.0,
            delta_rotation: 0.0,
            paddle_left: false,
            paddle_right: false,
        }
    }

    fn water_top(w: &impl BlockAccess, ctx: &MoveCtx, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        let s = w.state(x, y, z)?;
        let info = ctx.data.state(s);
        if info.fluid != 1 {
            return None;
        }
        let height = if Body::fluid_at(w, ctx, x, y + 1, z) == 1 { 1.0 } else { Body::own_height(ctx, s) };
        Some((height, info.has(flags::FLUID_SOURCE)))
    }

    fn is_underwater(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> Option<BoatStatus> {
        let b = self.body.aabb();
        let top = b.max.y + 0.001;
        let mut any = false;
        for x in floor(b.min.x)..b.max.x.ceil() as i32 {
            for y in floor(b.max.y)..top.ceil() as i32 {
                for z in floor(b.min.z)..b.max.z.ceil() as i32 {
                    if let Some((h, source)) = Self::water_top(w, ctx, x, y, z) {
                        if top < (y as f32 + h) as f64 {
                            if !source {
                                return Some(BoatStatus::UnderFlowingWater);
                            }
                            any = true;
                        }
                    }
                }
            }
        }
        any.then_some(BoatStatus::UnderWater)
    }

    fn check_in_water(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) -> bool {
        let b = self.body.aabb();
        let mut wet = false;
        self.water_level = -f64::MAX;
        for x in floor(b.min.x)..b.max.x.ceil() as i32 {
            for y in floor(b.min.y)..(b.min.y + 0.001).ceil() as i32 {
                for z in floor(b.min.z)..b.max.z.ceil() as i32 {
                    if let Some((h, _)) = Self::water_top(w, ctx, x, y, z) {
                        let top = y as f32 + h;
                        self.water_level = self.water_level.max(top as f64);
                        wet |= b.min.y < top as f64;
                    }
                }
            }
        }
        wet
    }

    fn ground_friction(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> f32 {
        let bb = self.body.aabb();
        let under = Aabb::new(DVec3::new(bb.min.x, bb.min.y - 0.001, bb.min.z), DVec3::new(bb.max.x, bb.min.y, bb.max.z));
        let (x0, x1) = (floor(under.min.x) - 1, under.max.x.ceil() as i32 + 1);
        let (y0, y1) = (floor(under.min.y) - 1, under.max.y.ceil() as i32 + 1);
        let (z0, z1) = (floor(under.min.z) - 1, under.max.z.ceil() as i32 + 1);
        let (mut sum, mut count) = (0.0f32, 0);
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = (x == x0 || x == x1 - 1) as i32 + (z == z0 || z == z1 - 1) as i32;
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges > 0 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let Some(s) = w.state(x, y, z) else { continue };
                    let block = ctx.data.block(s);
                    if block.name == "lily_pad" {
                        continue;
                    }
                    let base = DVec3::new(x as f64, y as f64, z as f64);
                    let hit = self.body.block_boxes(ctx, s, y).iter().any(|b| Aabb::new(base + DVec3::from(b.min), base + DVec3::from(b.max)).intersects(&under));
                    if hit {
                        sum += block.friction;
                        count += 1;
                    }
                }
            }
        }
        if count == 0 { 0.0 } else { sum / count as f32 }
    }

    fn water_level_above(&self, w: &impl BlockAccess, ctx: &MoveCtx) -> f32 {
        let b = self.body.aabb();
        let (x0, x1) = (floor(b.min.x), b.max.x.ceil() as i32);
        let (z0, z1) = (floor(b.min.z), b.max.z.ceil() as i32);
        let (y0, y1) = (floor(b.max.y), (b.max.y - self.last_yd).ceil() as i32);
        'rows: for y in y0..y1 {
            let mut best = 0.0f32;
            for x in x0..x1 {
                for z in z0..z1 {
                    if let Some((h, _)) = Self::water_top(w, ctx, x, y, z) {
                        best = best.max(h);
                    }
                    if best >= 1.0 {
                        continue 'rows;
                    }
                }
            }
            if best < 1.0 {
                return y as f32 + best;
            }
        }
        (y1 + 1) as f32
    }

    fn get_status(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) -> BoatStatus {
        if let Some(s) = self.is_underwater(w, ctx) {
            self.water_level = self.body.aabb().max.y;
            return s;
        }
        if self.check_in_water(w, ctx) {
            return BoatStatus::InWater;
        }
        let f = self.ground_friction(w, ctx);
        if f > 0.0 {
            self.land_friction = f;
            return BoatStatus::OnLand;
        }
        BoatStatus::InAir
    }

    fn float_boat(&mut self, w: &impl BlockAccess, ctx: &MoveCtx) {
        let mut gravity = -BOAT_GRAVITY;
        let mut buoyancy = 0.0;
        let mut friction;
        if self.old_status == BoatStatus::InAir && self.status != BoatStatus::InAir && self.status != BoatStatus::OnLand {
            self.water_level = self.body.pos.y + self.body.aabb().max.y - self.body.aabb().min.y;
            let height = self.body.aabb().max.y - self.body.aabb().min.y;
            let y = (self.water_level_above(w, ctx) - height as f32) as f64 + 0.101;
            let moved = self.body.aabb().moved(DVec3::new(0.0, y - self.body.pos.y, 0.0));
            if self.body.no_collision(w, ctx, &moved) {
                self.body.pos.y = y;
                self.body.vel.y = 0.0;
                self.last_yd = 0.0;
            }
            self.status = BoatStatus::InWater;
            return;
        }
        match self.status {
            BoatStatus::InWater => {
                let height = self.body.aabb().max.y - self.body.aabb().min.y;
                buoyancy = (self.water_level - self.body.pos.y) / height;
                friction = 0.9;
            }
            BoatStatus::UnderFlowingWater => {
                gravity = -7.0e-4;
                friction = 0.9;
            }
            BoatStatus::UnderWater => {
                buoyancy = 0.009999999776482582;
                friction = 0.45;
            }
            BoatStatus::InAir => friction = 0.9,
            BoatStatus::OnLand => {
                friction = self.land_friction;
                self.land_friction /= 2.0;
            }
        }
        friction = friction.max(0.0);
        let v = self.body.vel;
        self.body.vel = DVec3::new(v.x * friction as f64, v.y + gravity, v.z * friction as f64);
        self.delta_rotation *= friction;
        if buoyancy > 0.0 {
            let v = self.body.vel;
            self.body.vel = DVec3::new(v.x, (v.y + buoyancy * (BOAT_GRAVITY / 0.65)) * 0.75, v.z);
        }
    }

    fn control(&mut self, left: bool, right: bool, up: bool, down: bool) {
        let mut push = 0.0f32;
        if left {
            self.delta_rotation -= 1.0;
        }
        if right {
            self.delta_rotation += 1.0;
        }
        if right != left && !up && !down {
            push += 0.005;
        }
        self.body.yaw += self.delta_rotation;
        if up {
            push += 0.04;
        }
        if down {
            push -= 0.005;
        }
        let yaw = self.body.yaw;
        self.body.vel += DVec3::new((mth_sin((-yaw * DEG) as f64) * push) as f64, 0.0, (mth_cos((yaw * DEG) as f64) * push) as f64);
        self.paddle_left = (right && !left) || up;
        self.paddle_right = (left && !right) || up;
    }

    /// One tick, with the keys held on this tick.
    pub fn tick(&mut self, left: bool, right: bool, up: bool, down: bool, w: &impl BlockAccess, ctx: &MoveCtx) {
        self.old_status = self.status;
        self.status = self.get_status(w, ctx);
        self.body.old_pos = self.body.pos;
        self.body.in_block_state = None;
        self.body.update_fluids(w, ctx);
        self.float_boat(w, ctx);
        self.control(left, right, up, down);
        // Noted before the move can stop it, which is how the game keeps it.
        self.last_yd = self.body.vel.y;
        self.body.do_move(self.body.vel, w, ctx);
        self.body.apply_block_effects(w, ctx);
    }
}

impl Aabb {
    fn with_y(mut self, min: f64, max: f64) -> Self {
        self.min.y = min;
        self.max.y = max;
        self
    }
}

/// Whether the segment from `p` to `q` passes through `b`.
fn segment_hits(p: DVec3, q: DVec3, b: &Aabb) -> bool {
    let d = q - p;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for a in 0..3 {
        if d[a].abs() < 1.0e-12 {
            if p[a] < b.min[a] || p[a] > b.max[a] {
                return false;
            }
            continue;
        }
        let (mut near, mut far) = ((b.min[a] - p[a]) / d[a], (b.max[a] - p[a]) / d[a]);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_trig_matches_the_game() {
        assert_eq!(mth_sin(0.0), 0.0);
        assert_eq!(mth_cos(0.0), 1.0);
        assert!((mth_sin(std::f64::consts::FRAC_PI_2) - 1.0).abs() < 1e-6);
    }
}
