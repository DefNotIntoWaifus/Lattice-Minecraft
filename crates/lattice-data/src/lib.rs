//! Facts about blocks, entities and registries for the supported game version,
//! extracted from the official server by `tools/gen-data.sh`. Nothing in here
//! is hand-written.

use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

static RAW: &[u8] = include_bytes!("../data/game.json.gz");

/// Axis-aligned box in block-local coordinates (0..1, may exceed for fences etc).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(from = "[f64; 6]")]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl From<[f64; 6]> for Aabb {
    fn from(v: [f64; 6]) -> Self {
        Aabb { min: [v[0], v[1], v[2]], max: [v[3], v[4], v[5]] }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Offset {
    /// Whether the block is also offset vertically.
    pub y: bool,
    pub max_h: f32,
    pub max_v: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockInfo {
    pub name: String,
    pub first: u32,
    pub count: u32,
    pub default: u32,
    pub friction: f32,
    pub speed: f32,
    pub jump: f32,
    pub hardness: f32,
    pub resistance: f32,
    /// Requires the correct tool for drops.
    pub tool: bool,
    /// How much of a fall's speed the block gives back (slime, beds).
    #[serde(default)]
    pub bounce: f32,
    #[serde(default)]
    pub has_entity: bool,
    /// The item that places this block.
    pub item: Option<String>,
    /// Index into [`GameData::sound_types`].
    #[serde(default)]
    pub sound: u16,
    pub props: Vec<(String, Vec<String>)>,
    pub offset: Option<Offset>,
}

/// The sounds a block makes.
#[derive(Debug, Clone, Deserialize)]
pub struct SoundType {
    #[serde(rename = "break")]
    pub break_sound: String,
    pub step: String,
    pub place: String,
    pub hit: String,
    pub fall: String,
    pub volume: f32,
    pub pitch: f32,
}

/// One synced data field of an entity class: wire index, serializer id, the game's field name.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityField(pub u8, pub i32, pub String);

#[derive(Debug, Clone, Deserialize)]
pub struct EntityClass {
    pub parent: Option<String>,
    pub fields: Vec<EntityField>,
}

/// A status effect.
#[derive(Debug, Clone, Deserialize)]
pub struct MobEffect {
    pub name: String,
    pub color: u32,
    pub category: String,
    /// Translation key.
    pub key: String,
}

/// An item and its default data components (see tools/gen-data.sh for what's left out).
#[derive(Debug, Clone, Deserialize)]
pub struct ItemInfo {
    pub name: String,
    pub components: serde_json::Map<String, serde_json::Value>,
}

/// A page of the creative inventory.
#[derive(Debug, Clone, Deserialize)]
pub struct CreativeTab {
    pub id: String,
    /// Translation key.
    pub title: String,
    pub icon: String,
    /// category, hotbar, inventory or search.
    pub kind: String,
    pub row: String,
    pub column: i32,
    /// Item name, optionally followed by its components as JSON.
    pub items: Vec<Vec<String>>,
}

/// Per-state flags (bit layout written by tools/extractor/Extract.java).
pub mod flags {
    pub const AIR: u32 = 1;
    pub const CAN_OCCLUDE: u32 = 1 << 1;
    pub const SOLID_RENDER: u32 = 1 << 2;
    pub const LIQUID: u32 = 1 << 3;
    pub const SKYLIGHT_DOWN: u32 = 1 << 4;
    pub const INVISIBLE: u32 = 1 << 5;
    pub const SKIP_SAME: u32 = 1 << 6;
    pub const FULL_COLLISION: u32 = 1 << 7;
    pub const REPLACEABLE: u32 = 1 << 8;
    pub const FLUID_SOURCE: u32 = 1 << 9;
    pub const SOLID: u32 = 1 << 10;
    pub const SHAPE_FOR_LIGHT: u32 = 1 << 11;
    /// Full occluding face toward a direction, indexed by direction 0..6
    /// (down, up, north, south, west, east).
    pub const FACE_FULL_SHIFT: u32 = 12;
    pub const SUFFOCATING: u32 = 1 << 18;
    /// Sturdy face toward a direction, indexed like FACE_FULL_SHIFT.
    pub const FACE_STURDY_SHIFT: u32 = 20;
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(from = "[u32; 7]")]
pub struct StateInfo {
    pub collision: u16,
    pub outline: u16,
    pub flags: u32,
    pub emission: u8,
    pub dampening: u8,
    /// 0 none, 1 water, 2 lava, 3 other
    pub fluid: u8,
    pub fluid_amount: u8,
}

impl From<[u32; 7]> for StateInfo {
    fn from(v: [u32; 7]) -> Self {
        StateInfo {
            collision: v[0] as u16,
            outline: v[1] as u16,
            flags: v[2],
            emission: v[3] as u8,
            dampening: v[4] as u8,
            fluid: v[5] as u8,
            fluid_amount: v[6] as u8,
        }
    }
}

impl StateInfo {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
    pub fn face_full(&self, dir: usize) -> bool {
        self.flags & (1 << (flags::FACE_FULL_SHIFT + dir as u32)) != 0
    }
    pub fn face_sturdy(&self, dir: usize) -> bool {
        self.flags & (1 << (flags::FACE_STURDY_SHIFT + dir as u32)) != 0
    }
    pub fn is_air(&self) -> bool {
        self.has(flags::AIR)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MapMarker {
    /// Its picture, under `map/decorations/`.
    pub asset: String,
    /// Whether it still shows when the map hangs in a frame.
    pub on_frame: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityInfo {
    pub name: String,
    pub id: u32,
    pub width: f32,
    pub height: f32,
    pub eye: f32,
    /// The game's class for this type; see [`GameData::entity_classes`].
    pub class: Option<String>,
    /// Where each rider sits, from the entity's feet.
    #[serde(default)]
    pub seats: Vec<[f64; 3]>,
    /// The point of it that sits on what it rides.
    #[serde(default)]
    pub rides_at: [f64; 3],
    /// Default values of what decides how it moves.
    #[serde(default)]
    pub attributes: std::collections::HashMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerDefaults {
    pub movement_speed: f64,
    pub gravity: f64,
    pub jump_strength: f64,
    pub step_height: f64,
    pub sneaking_speed: f64,
    pub block_interaction_range: f64,
    pub entity_interaction_range: f64,
    pub width: f32,
    pub height: f32,
    pub eye: f32,
}

#[derive(Debug, Deserialize)]
pub struct Vectors {
    /// [x, y, z, seed as decimal string]
    pub seed: Vec<(i32, i32, i32, String)>,
    /// [block, x, y, z, ox, oy, oz]
    pub offset: Vec<(String, i32, i32, i32, f64, f64, f64)>,
}

#[derive(Debug, Deserialize)]
pub struct GameData {
    /// How each map marker looks and where it shows, by its number.
    #[serde(default)]
    pub map_decorations: Vec<MapMarker>,
    /// What each map colour number looks like, as 0xRRGGBB.
    #[serde(default)]
    pub map_colors: Vec<u32>,
    pub version: String,
    pub protocol: i32,
    pub blocks: Vec<BlockInfo>,
    pub states: Vec<StateInfo>,
    pub shapes: Vec<Vec<Aabb>>,
    pub entities: Vec<EntityInfo>,
    pub entity_classes: HashMap<String, EntityClass>,
    /// Serializer names ("BYTE", "ROTATIONS", ...) to their wire ids.
    pub entity_serializers: HashMap<String, i32>,
    pub sound_types: Vec<SoundType>,
    pub items: Vec<ItemInfo>,
    pub mob_effects: Vec<MobEffect>,
    /// Potion name to its effects: effect id, amplifier, duration.
    pub potions: HashMap<String, Vec<(i32, i32, i32)>>,
    /// Default contents of the block tags movement depends on.
    pub block_tags: HashMap<String, Vec<String>>,
    pub player: PlayerDefaults,
    pub registries: HashMap<String, Vec<String>>,
    pub vectors: Vectors,
    #[serde(skip)]
    state_block: Vec<u16>,
    #[serde(skip)]
    by_name: HashMap<String, u16>,
}

pub fn game() -> &'static GameData {
    static DATA: OnceLock<GameData> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut json = String::new();
        flate2::read::GzDecoder::new(RAW).read_to_string(&mut json).expect("embedded game data");
        let mut d: GameData = serde_json::from_str(&json).expect("embedded game data");
        d.state_block = vec![0; d.states.len()];
        for (i, b) in d.blocks.iter().enumerate() {
            for s in b.first..b.first + b.count {
                d.state_block[s as usize] = i as u16;
            }
            d.by_name.insert(b.name.clone(), i as u16);
        }
        d
    })
}

pub type StateId = u32;

impl GameData {
    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    /// Bits used by the "direct" palette for block states: ceil(log2(count)).
    pub fn block_state_bits(&self) -> u32 {
        ceil_log2(self.states.len() as u32)
    }

    pub fn state(&self, id: StateId) -> &StateInfo {
        self.states.get(id as usize).unwrap_or(&self.states[0])
    }

    pub fn block_index(&self, id: StateId) -> usize {
        *self.state_block.get(id as usize).unwrap_or(&0) as usize
    }

    pub fn block(&self, id: StateId) -> &BlockInfo {
        &self.blocks[self.block_index(id)]
    }

    pub fn block_by_name(&self, name: &str) -> Option<&BlockInfo> {
        self.by_name.get(name.trim_start_matches("minecraft:")).map(|&i| &self.blocks[i as usize])
    }

    /// Property values of a state, in definition order.
    pub fn properties(&self, id: StateId) -> Vec<(&str, &str)> {
        let b = self.block(id);
        let mut idx = id - b.first;
        let mut out = Vec::with_capacity(b.props.len());
        for (name, values) in b.props.iter().rev() {
            let n = values.len() as u32;
            out.push((name.as_str(), values[(idx % n) as usize].as_str()));
            idx /= n;
        }
        out.reverse();
        out
    }

    /// State ID of `block` with the given properties; unspecified ones keep their default.
    pub fn state_with(&self, block: &BlockInfo, props: &[(&str, &str)]) -> Option<StateId> {
        let defaults = self.properties(block.default);
        let mut idx = 0u32;
        for (name, values) in &block.props {
            let wanted = props
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| *v)
                .or_else(|| defaults.iter().find(|(k, _)| k == name).map(|(_, v)| *v))?;
            let vi = values.iter().position(|v| v == wanted)? as u32;
            idx = idx * values.len() as u32 + vi;
        }
        Some(block.first + idx)
    }

    pub fn collision(&self, id: StateId) -> &[Aabb] {
        &self.shapes[self.state(id).collision as usize]
    }

    pub fn outline(&self, id: StateId) -> &[Aabb] {
        &self.shapes[self.state(id).outline as usize]
    }

    pub fn registry(&self, name: &str) -> &[String] {
        self.registries.get(name.trim_start_matches("minecraft:")).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn entity(&self, type_id: i32) -> Option<&EntityInfo> {
        // The list is in id order in practice, so this is usually one step.
        if let Some(e) = self.entities.get(type_id as usize) {
            if e.id as i32 == type_id {
                return Some(e);
            }
        }
        self.entities.iter().find(|e| e.id as i32 == type_id)
    }
}

pub fn ceil_log2(n: u32) -> u32 {
    if n <= 1 { 0 } else { 32 - (n - 1).leading_zeros() }
}

/// `Mth.getSeed` — position hash used for block model variants and offsets.
pub fn position_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut l = (x.wrapping_mul(3129871) as i64) ^ (z as i64).wrapping_mul(116129781) ^ (y as i64);
    l = l.wrapping_mul(l).wrapping_mul(42317861).wrapping_add(l.wrapping_mul(11));
    l >> 16
}

/// Random XZ(Y) offset of plants etc. at a position.
pub fn block_offset(block: &BlockInfo, x: i32, y: i32, z: i32) -> [f64; 3] {
    let Some(o) = block.offset else { return [0.0; 3] };
    let seed = position_seed(x, 0, z);
    let max_h = o.max_h as f64;
    let ox = ((((seed & 15) as f32 / 15.0) as f64 - 0.5) * 0.5).clamp(-max_h, max_h);
    let oz = ((((seed >> 8 & 15) as f32 / 15.0) as f64 - 0.5) * 0.5).clamp(-max_h, max_h);
    let oy = if o.y { (((seed >> 4 & 15) as f32 / 15.0) as f64 - 1.0) * o.max_v as f64 } else { 0.0 };
    let _ = y;
    [ox, oy, oz]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads() {
        let g = game();
        assert_eq!(g.protocol, 777);
        assert_eq!(g.block(0).name, "air");
        assert_eq!(g.block_by_name("stone").unwrap().first, 1);
        assert_eq!(g.block_state_bits(), 16);
    }

    #[test]
    fn properties_round_trip() {
        let g = game();
        for b in &g.blocks {
            for id in b.first..b.first + b.count {
                let props = g.properties(id);
                let back = g.state_with(b, &props).unwrap();
                assert_eq!(back, id, "{}", b.name);
            }
        }
    }

    #[test]
    fn stairs_default() {
        let g = game();
        let stairs = g.block_by_name("oak_stairs").unwrap();
        let props = g.properties(stairs.default);
        assert_eq!(props, vec![("facing", "north"), ("half", "bottom"), ("shape", "straight"), ("waterlogged", "false")]);
    }

    /// Our Mth.getSeed port must match values produced by the real game.
    #[test]
    fn seed_matches_game() {
        for (x, y, z, s) in &game().vectors.seed {
            assert_eq!(position_seed(*x, *y, *z).to_string(), *s, "{x} {y} {z}");
        }
    }

    #[test]
    fn offsets_match_game() {
        let g = game();
        for (name, x, y, z, ox, oy, oz) in &g.vectors.offset {
            let b = g.block_by_name(name).unwrap();
            let o = block_offset(b, *x, *y, *z);
            for (a, e) in o.iter().zip([ox, oy, oz]) {
                assert!((a - e).abs() < 1e-6, "{name} at {x},{y},{z}: got {o:?}, want {ox},{oy},{oz}");
            }
        }
    }
}
