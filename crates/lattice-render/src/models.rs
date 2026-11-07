//! Block models in the resource-pack format: blockstate files choose models
//! (variants or multipart), models inherit from parents and list elements with
//! textured faces. Everything is baked once into quads per block state.

use crate::atlas::{Atlas, Sprite};
use glam::{Mat3, Vec3};
use lattice_assets::Resources;
use lattice_data::GameData;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Face directions in the game's order (matches the extracted face flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Dir {
    pub const ALL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];

    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s {
            "down" | "bottom" => Dir::Down,
            "up" | "top" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }

    pub fn normal(self) -> Vec3 {
        match self {
            Dir::Down => Vec3::NEG_Y,
            Dir::Up => Vec3::Y,
            Dir::North => Vec3::NEG_Z,
            Dir::South => Vec3::Z,
            Dir::West => Vec3::NEG_X,
            Dir::East => Vec3::X,
        }
    }

    pub fn offset(self) -> [i32; 3] {
        let n = self.normal();
        [n.x as i32, n.y as i32, n.z as i32]
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    fn from_normal(n: Vec3) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.normal().dot(n) > 0.999)
    }

    /// Our own face shading (brightness by orientation).
    pub fn shade(self) -> f32 {
        match self {
            Dir::Up => 1.0,
            Dir::Down => 0.55,
            Dir::North | Dir::South => 0.82,
            Dir::West | Dir::East => 0.68,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Solid = 0,
    Cutout = 1,
    Translucent = 2,
}

#[derive(Debug, Clone)]
pub struct Quad {
    /// Block-local positions (0..1 for a full cube).
    pub pos: [Vec3; 4],
    pub uv: [[f32; 2]; 4],
    /// Axis-aligned facing, if the quad is flat on an axis.
    pub face: Option<Dir>,
    pub cull: Option<Dir>,
    pub tint: i32,
    /// Brightness from orientation (1.0 when unshaded).
    pub shade: f32,
    /// Lies on the block boundary and covers the whole face (eligible for smooth light/AO).
    pub full: bool,
    pub layer: Layer,
}

#[derive(Debug, Default)]
pub struct BakedModel {
    pub quads: Vec<Quad>,
    pub ao: bool,
}

/// One part of a block's appearance: a weighted list of alternatives.
pub type Part = Vec<(u32, Arc<BakedModel>)>;

pub struct BlockModels {
    /// Indexed by block state ID.
    pub states: Vec<Vec<Part>>,
    pub missing_models: usize,
    /// The picture bits of each block break into, by block index.
    particle: Vec<Sprite>,
    missing: Sprite,
}

// ---------------------------------------------------------------- loading

pub(crate) struct Loader<'a> {
    res: &'a Resources,
    raw: HashMap<String, Option<Value>>,
    resolved: HashMap<String, Option<Arc<ResolvedModel>>>,
    baked: HashMap<(String, i32, i32, bool), Arc<BakedModel>>,
}

#[derive(Debug)]
pub(crate) struct ResolvedModel {
    pub(crate) elements: Vec<Value>,
    pub(crate) textures: Map<String, Value>,
    /// Sprites that go in the translucent layer whatever their pixels look like.
    pub(crate) forced: std::collections::HashSet<String>,
    ao: bool,
}

fn model_path(name: &str) -> String {
    let n = name.trim_start_matches("minecraft:");
    format!("models/{n}.json")
}

impl<'a> Loader<'a> {
    pub(crate) fn new(res: &'a Resources) -> Self {
        Loader { res, raw: HashMap::new(), resolved: HashMap::new(), baked: HashMap::new() }
    }

    pub(crate) fn raw(&mut self, name: &str) -> Option<Value> {
        let key = name.trim_start_matches("minecraft:").to_string();
        if !self.raw.contains_key(&key) {
            let v = self.res.json(&model_path(&key));
            self.raw.insert(key.clone(), v);
        }
        self.raw[&key].clone()
    }

    pub(crate) fn resolve(&mut self, name: &str) -> Option<Arc<ResolvedModel>> {
        let key = name.trim_start_matches("minecraft:").to_string();
        if let Some(r) = self.resolved.get(&key) {
            return r.clone();
        }
        let mut chain = Vec::new();
        let mut cur = Some(key.clone());
        while let Some(n) = cur.take() {
            if chain.len() > 32 || n.starts_with("builtin/") {
                break;
            }
            let Some(v) = self.raw(&n) else { break };
            cur = v.get("parent").and_then(Value::as_str).map(|p| p.trim_start_matches("minecraft:").to_string());
            chain.push(v);
        }
        let result = if chain.is_empty() {
            None
        } else {
            let mut textures = Map::new();
            let mut forced = std::collections::HashSet::new();
            let mut elements = None;
            let mut ao = None;
            // Child first: the first definition found wins.
            for v in &chain {
                if let Some(t) = v.get("textures").and_then(Value::as_object) {
                    for (k, tv) in t {
                        // Either a name, or a name with notes on how it's drawn.
                        let name = match tv.as_object() {
                            Some(o) => {
                                let sprite = o.get("sprite").cloned().unwrap_or(Value::Null);
                                if o.get("force_translucent").and_then(Value::as_bool).unwrap_or(false) && !textures.contains_key(k) {
                                    if let Some(n) = sprite.as_str() {
                                        forced.insert(n.trim_start_matches("minecraft:").to_string());
                                    }
                                }
                                sprite
                            }
                            None => tv.clone(),
                        };
                        textures.entry(k.clone()).or_insert(name);
                    }
                }
                if elements.is_none() {
                    elements = v.get("elements").and_then(Value::as_array).cloned();
                }
                if ao.is_none() {
                    ao = v.get("ambientocclusion").and_then(Value::as_bool);
                }
            }
            Some(Arc::new(ResolvedModel { elements: elements.unwrap_or_default(), textures, forced, ao: ao.unwrap_or(true) }))
        };
        self.resolved.insert(key, result.clone());
        result
    }
}

pub(crate) fn resolve_texture(textures: &Map<String, Value>, reference: &str) -> Option<String> {
    let mut r = reference.to_string();
    // A face may name a slot with or without the mark.
    if !r.starts_with('#') && textures.contains_key(&r) {
        r.insert(0, '#');
    }
    for _ in 0..16 {
        match r.strip_prefix('#') {
            Some(key) => r = textures.get(key)?.as_str()?.to_string(),
            None => return Some(r.trim_start_matches("minecraft:").to_string()),
        }
    }
    None
}

/// Says so, once each, when a model asks for a picture that isn't there.
fn warn_missing(what: &str) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.get_or_insert_with(Default::default).insert(what.to_string()) {
        tracing::warn!("a model asks for a picture that isn't there: {what}");
    }
}

/// Every texture name referenced by any block model reachable from blockstates.
pub fn collect_textures(res: &Resources) -> Vec<String> {
    let mut loader = Loader::new(res);
    let mut names = std::collections::BTreeSet::new();
    for path in res.list("blockstates/", ".json") {
        let Some(bs) = res.json(&path) else { continue };
        let mut models = Vec::new();
        let mut push = |v: &Value| {
            for m in v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]) {
                if let Some(name) = m.get("model").and_then(Value::as_str) {
                    models.push(name.to_string());
                }
            }
        };
        if let Some(vars) = bs.get("variants").and_then(Value::as_object) {
            vars.values().for_each(&mut push);
        }
        if let Some(parts) = bs.get("multipart").and_then(Value::as_array) {
            parts.iter().filter_map(|p| p.get("apply")).for_each(&mut push);
        }
        for m in models {
            let Some(rm) = loader.resolve(&m) else { continue };
            if let Some(t) = rm.textures.get("particle").and_then(Value::as_str).and_then(|t| resolve_texture(&rm.textures, t)) {
                names.insert(t);
            }
            for e in &rm.elements {
                if let Some(faces) = e.get("faces").and_then(Value::as_object) {
                    for f in faces.values() {
                        if let Some(t) = f.get("texture").and_then(Value::as_str).and_then(|t| resolve_texture(&rm.textures, t)) {
                            names.insert(t);
                        }
                    }
                }
            }
        }
    }
    // Fluids are drawn by the mesher, not by models.
    for t in ["block/water_still", "block/water_flow", "block/water_overlay", "block/lava_still", "block/lava_flow", "block/fire_0", "block/fire_1", "block/destroy_stage_0", "block/destroy_stage_1", "block/destroy_stage_2", "block/destroy_stage_3", "block/destroy_stage_4", "block/destroy_stage_5", "block/destroy_stage_6", "block/destroy_stage_7", "block/destroy_stage_8", "block/destroy_stage_9"] {
        names.insert(t.to_string());
    }
    names.into_iter().collect()
}

// ---------------------------------------------------------------- baking

fn vec3(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    Some(Vec3::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32))
}

/// Default UV rectangle (in 0..16) for a face, from the element's extent.
fn default_uv(dir: Dir, from: Vec3, to: Vec3) -> [f32; 4] {
    match dir {
        Dir::Down => [from.x, 16.0 - to.z, to.x, 16.0 - from.z],
        Dir::Up => [from.x, from.z, to.x, to.z],
        Dir::North => [16.0 - to.x, 16.0 - to.y, 16.0 - from.x, 16.0 - from.y],
        Dir::South => [from.x, 16.0 - to.y, to.x, 16.0 - from.y],
        Dir::West => [from.z, 16.0 - to.y, to.z, 16.0 - from.y],
        Dir::East => [16.0 - to.z, 16.0 - to.y, 16.0 - from.z, 16.0 - from.y],
    }
}

/// Corners of a face, ordered top-left, bottom-left, bottom-right, top-right
/// as seen from outside, plus each corner's (s, t) position on the face.
fn face_corners(dir: Dir, f: Vec3, t: Vec3) -> [Vec3; 4] {
    match dir {
        Dir::Down => [Vec3::new(f.x, f.y, t.z), Vec3::new(f.x, f.y, f.z), Vec3::new(t.x, f.y, f.z), Vec3::new(t.x, f.y, t.z)],
        Dir::Up => [Vec3::new(f.x, t.y, f.z), Vec3::new(f.x, t.y, t.z), Vec3::new(t.x, t.y, t.z), Vec3::new(t.x, t.y, f.z)],
        Dir::North => [Vec3::new(t.x, t.y, f.z), Vec3::new(t.x, f.y, f.z), Vec3::new(f.x, f.y, f.z), Vec3::new(f.x, t.y, f.z)],
        Dir::South => [Vec3::new(f.x, t.y, t.z), Vec3::new(f.x, f.y, t.z), Vec3::new(t.x, f.y, t.z), Vec3::new(t.x, t.y, t.z)],
        Dir::West => [Vec3::new(f.x, t.y, f.z), Vec3::new(f.x, f.y, f.z), Vec3::new(f.x, f.y, t.z), Vec3::new(f.x, t.y, t.z)],
        Dir::East => [Vec3::new(t.x, t.y, t.z), Vec3::new(t.x, f.y, t.z), Vec3::new(t.x, f.y, f.z), Vec3::new(t.x, t.y, f.z)],
    }
}

/// Face-local (s, t) of corners in `face_corners` order.
const CORNER_ST: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

fn rotate_st(st: [f32; 2], rotation: i32) -> [f32; 2] {
    let [s, t] = st;
    match rotation.rem_euclid(360) {
        90 => [t, 1.0 - s],
        180 => [1.0 - s, 1.0 - t],
        270 => [1.0 - t, s],
        _ => [s, t],
    }
}

fn element_rotation(rot: &Value) -> Option<(Vec3, Mat3, Vec3)> {
    let origin = rot.get("origin").and_then(vec3).unwrap_or(Vec3::splat(8.0)) / 16.0;
    if let (Some(axis), Some(angle)) = (rot.get("axis").and_then(Value::as_str), rot.get("angle").and_then(Value::as_f64)) {
        let a = (angle as f32).to_radians();
        let (m, scale_axes) = match axis {
            "x" => (Mat3::from_rotation_x(a), Vec3::new(0.0, 1.0, 1.0)),
            "y" => (Mat3::from_rotation_y(a), Vec3::new(1.0, 0.0, 1.0)),
            _ => (Mat3::from_rotation_z(a), Vec3::new(1.0, 1.0, 0.0)),
        };
        let rescale = rot.get("rescale").and_then(Value::as_bool).unwrap_or(false);
        let scale = if rescale { Vec3::ONE + scale_axes * (1.0 / a.cos() - 1.0) } else { Vec3::ONE };
        return Some((origin, m, scale));
    }
    // Euler form: {"x": .., "y": .., "z": ..}
    let deg = |k: &str| rot.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let m = Mat3::from_rotation_x(deg("x").to_radians())
        * Mat3::from_rotation_y(deg("y").to_radians())
        * Mat3::from_rotation_z(deg("z").to_radians());
    Some((origin, m, Vec3::ONE))
}

/// Variant rotation: X first, then Y, by negative angles (matches the game).
fn variant_matrix(x: i32, y: i32) -> Mat3 {
    Mat3::from_rotation_y((-y as f32).to_radians()) * Mat3::from_rotation_x((-x as f32).to_radians())
}

fn snap(v: Vec3) -> Vec3 {
    let r = |f: f32| if (f - f.round()).abs() < 1e-5 { f.round() } else { f };
    Vec3::new(r(v.x), r(v.y), r(v.z))
}

/// A model as it is, with no blockstate rotation.
pub(crate) fn bake_plain(model: &ResolvedModel, atlas: &Atlas) -> BakedModel {
    bake(model, 0, 0, false, atlas)
}

fn bake(model: &ResolvedModel, x: i32, y: i32, uvlock: bool, atlas: &Atlas) -> BakedModel {
    let vm = variant_matrix(x, y);
    let centre = Vec3::splat(0.5);
    let mut quads = Vec::new();
    for e in &model.elements {
        let (Some(from), Some(to)) = (e.get("from").and_then(vec3), e.get("to").and_then(vec3)) else { continue };
        let erot = e.get("rotation").and_then(element_rotation);
        let shade_override = e.get("shade_direction_override").and_then(Value::as_str).and_then(Dir::parse);
        let shade_enabled = e.get("shade").and_then(Value::as_bool).unwrap_or(true);
        let Some(faces) = e.get("faces").and_then(Value::as_object) else { continue };
        for (fname, f) in faces {
            let Some(dir) = Dir::parse(fname) else { continue };
            let texture_name = f.get("texture").and_then(Value::as_str).and_then(|t| resolve_texture(&model.textures, t));
            let forced_translucent = texture_name.as_ref().is_some_and(|t| model.forced.contains(t));
            let sprite: Sprite = match &texture_name {
                Some(t) if atlas.sprites.contains_key(t.as_str()) => atlas.sprite(t),
                other => {
                    warn_missing(&format!("{:?} of {:?}", other, f.get("texture")));
                    atlas.missing
                }
            };
            let uv_rect = f
                .get("uv")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 4)
                .map(|a| [0, 1, 2, 3].map(|i| a[i].as_f64().unwrap_or(0.0) as f32))
                .unwrap_or_else(|| default_uv(dir, from, to));
            let tex_rot = f.get("rotation").and_then(Value::as_i64).unwrap_or(0) as i32;
            let corners = face_corners(dir, from / 16.0, to / 16.0);

            let mut pos = corners;
            if let Some((origin, m, scale)) = erot {
                for p in &mut pos {
                    *p = m * ((*p - origin) * scale) + origin;
                }
            }
            for p in &mut pos {
                *p = snap(vm * (*p - centre) + centre);
            }

            let mut uv = [[0.0f32; 2]; 4];
            let new_face = Dir::from_normal(vm * dir.normal());
            if uvlock && erot.is_none() && let Some(nf) = new_face {
                // Texture stays aligned with the world: derive UVs from final positions.
                let (lo, hi) = (pos.iter().fold(Vec3::splat(9.0), |a, p| a.min(*p)), pos.iter().fold(Vec3::splat(-9.0), |a, p| a.max(*p)));
                let r = default_uv(nf, lo * 16.0, hi * 16.0);
                let fc = face_corners(nf, lo, hi);
                for (i, p) in pos.iter().enumerate() {
                    let ci = fc.iter().position(|c| c.distance_squared(*p) < 1e-6).unwrap_or(i);
                    let [s, t] = CORNER_ST[ci];
                    uv[i] = sprite.lerp((r[0] + (r[2] - r[0]) * s) / 16.0, (r[1] + (r[3] - r[1]) * t) / 16.0);
                }
            } else {
                for i in 0..4 {
                    let [s, t] = rotate_st(CORNER_ST[i], tex_rot);
                    uv[i] = sprite.lerp((uv_rect[0] + (uv_rect[2] - uv_rect[0]) * s) / 16.0, (uv_rect[1] + (uv_rect[3] - uv_rect[1]) * t) / 16.0);
                }
            }

            let cull = f.get("cullface").and_then(Value::as_str).and_then(Dir::parse).and_then(|c| Dir::from_normal(vm * c.normal()));
            let face = if erot.is_some() { Dir::from_normal(vm * erot.unwrap().1 * dir.normal()) } else { new_face };
            let full = face.is_some_and(|d| {
                let axis = match d {
                    Dir::Down | Dir::Up => 1,
                    Dir::North | Dir::South => 2,
                    _ => 0,
                };
                let plane = if matches!(d, Dir::Up | Dir::South | Dir::East) { 1.0 } else { 0.0 };
                pos.iter().all(|p| (p[axis] - plane).abs() < 1e-5)
                    && pos.iter().map(|p| p[(axis + 1) % 3]).fold(f32::MAX, f32::min) < 1e-5
                    && pos.iter().map(|p| p[(axis + 1) % 3]).fold(f32::MIN, f32::max) > 1.0 - 1e-5
                    && pos.iter().map(|p| p[(axis + 2) % 3]).fold(f32::MAX, f32::min) < 1e-5
                    && pos.iter().map(|p| p[(axis + 2) % 3]).fold(f32::MIN, f32::max) > 1.0 - 1e-5
            });
            let shade = if !shade_enabled {
                1.0
            } else if let Some(o) = shade_override {
                Dir::from_normal(vm * o.normal()).unwrap_or(o).shade()
            } else {
                face.map_or(0.85, Dir::shade)
            };
            let layer = match sprite.alpha {
                _ if forced_translucent => Layer::Translucent,
                0 => Layer::Solid,
                1 => Layer::Cutout,
                _ => Layer::Translucent,
            };
            quads.push(Quad {
                pos,
                uv,
                face,
                cull,
                tint: f.get("tintindex").and_then(Value::as_i64).map_or(-1, |t| t as i32),
                shade,
                full,
                layer,
            });
        }
    }
    BakedModel { quads, ao: model.ao }
}

// ---------------------------------------------------------------- selection

fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn matches_when(when: &Value, props: &[(&str, &str)]) -> bool {
    let Some(obj) = when.as_object() else { return true };
    obj.iter().all(|(k, v)| match k.as_str() {
        "OR" => v.as_array().is_some_and(|l| l.iter().any(|c| matches_when(c, props))),
        "AND" => v.as_array().is_some_and(|l| l.iter().all(|c| matches_when(c, props))),
        key => {
            let actual = props.iter().find(|(p, _)| *p == key).map(|(_, v)| *v).unwrap_or("");
            let spec = value_str(v);
            let (negate, spec) = match spec.strip_prefix('!') {
                Some(s) => (true, s.to_string()),
                None => (false, spec),
            };
            spec.split('|').any(|alt| alt == actual) != negate
        }
    })
}

fn matches_variant_key(key: &str, props: &[(&str, &str)]) -> bool {
    key.is_empty()
        || key.split(',').all(|cond| {
            let Some((k, v)) = cond.split_once('=') else { return true };
            props.iter().any(|(pk, pv)| *pk == k && *pv == v)
        })
}

impl Loader<'_> {
    fn part(&mut self, apply: &Value, atlas: &Atlas) -> Part {
        let list = apply.as_array().cloned().unwrap_or_else(|| vec![apply.clone()]);
        list.iter()
            .filter_map(|m| {
                let name = m.get("model")?.as_str()?.trim_start_matches("minecraft:").to_string();
                let x = m.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                let y = m.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                let uvlock = m.get("uvlock").and_then(Value::as_bool).unwrap_or(false);
                let weight = m.get("weight").and_then(Value::as_u64).unwrap_or(1) as u32;
                let key = (name.clone(), x, y, uvlock);
                if let Some(b) = self.baked.get(&key) {
                    return Some((weight, b.clone()));
                }
                let resolved = self.resolve(&name)?;
                let b = Arc::new(bake(&resolved, x, y, uvlock, atlas));
                self.baked.insert(key, b.clone());
                Some((weight, b))
            })
            .collect()
    }
}

impl BlockModels {
    pub fn load(res: &Resources, data: &GameData, atlas: &Atlas) -> BlockModels {
        let mut loader = Loader { res, raw: HashMap::new(), resolved: HashMap::new(), baked: HashMap::new() };
        let mut states: Vec<Vec<Part>> = vec![Vec::new(); data.state_count()];
        let mut missing = 0;
        let mut particle = Vec::with_capacity(data.blocks.len());
        for block in &data.blocks {
            let Some(bs) = res.json(&format!("blockstates/{}.json", block.name)) else {
                if block.name != "air" && block.name != "cave_air" && block.name != "void_air" {
                    missing += 1;
                }
                particle.push(atlas.missing);
                continue;
            };
            // The first model named has the picture of the bits it breaks into.
            let first = bs.get("variants").and_then(Value::as_object).and_then(|v| v.values().next()).or_else(|| bs.get("multipart").and_then(Value::as_array).and_then(|m| m.first()).and_then(|p| p.get("apply")));
            let name = first.and_then(|v| v.as_array().and_then(|a| a.first()).or(Some(v))).and_then(|v| v.get("model")).and_then(Value::as_str);
            let sprite = name.and_then(|n| loader.resolve(n)).and_then(|rm| rm.textures.get("particle").and_then(Value::as_str).and_then(|t| resolve_texture(&rm.textures, t))).map(|t| atlas.sprite(&t));
            particle.push(sprite.unwrap_or(atlas.missing));
            for id in block.first..block.first + block.count {
                let props = data.properties(id);
                let mut parts = Vec::new();
                if let Some(vars) = bs.get("variants").and_then(Value::as_object) {
                    if let Some((_, v)) = vars.iter().find(|(k, _)| matches_variant_key(k, &props)) {
                        parts.push(loader.part(v, atlas));
                    }
                }
                if let Some(mp) = bs.get("multipart").and_then(Value::as_array) {
                    for p in mp {
                        if p.get("when").is_none_or(|w| matches_when(w, &props)) {
                            if let Some(apply) = p.get("apply") {
                                parts.push(loader.part(apply, atlas));
                            }
                        }
                    }
                }
                parts.retain(|p| !p.is_empty());
                states[id as usize] = parts;
            }
        }
        BlockModels { states, missing_models: missing, particle, missing: atlas.missing }
    }

    /// The picture a block's bits come from.
    pub fn particle(&self, state: u32) -> Sprite {
        let data = lattice_data::game();
        self.particle.get(data.block_index(state)).copied().unwrap_or(self.missing)
    }

    /// Picks the model of each part for a position (weighted, deterministic).
    pub fn pick(&self, state: u32, seed: i64) -> impl Iterator<Item = &BakedModel> {
        self.states.get(state as usize).into_iter().flatten().map(move |part| {
            if part.len() == 1 {
                return &*part[0].1;
            }
            let total: u32 = part.iter().map(|(w, _)| *w).sum();
            let mut r = (seed.unsigned_abs() % total.max(1) as u64) as u32;
            for (w, m) in part {
                if r < *w {
                    return &**m;
                }
                r -= w;
            }
            &*part[0].1
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_rotation_y90_turns_north_to_east() {
        let m = variant_matrix(0, 90);
        let n = m * Dir::North.normal();
        assert_eq!(Dir::from_normal(n), Some(Dir::East));
        let m = variant_matrix(90, 0);
        assert_eq!(Dir::from_normal(m * Dir::Up.normal()), Some(Dir::North));
        let m = variant_matrix(180, 0);
        assert_eq!(Dir::from_normal(m * Dir::Up.normal()), Some(Dir::Down));
    }

    #[test]
    fn conditions() {
        let props = [("north", "true"), ("east", "false"), ("waterlogged", "false")];
        assert!(matches_variant_key("north=true,east=false", &props));
        assert!(!matches_variant_key("north=false", &props));
        let when: Value = serde_json::from_str(r#"{"OR":[{"north":"false"},{"east":"false|true"}]}"#).unwrap();
        assert!(matches_when(&when, &props));
        let when: Value = serde_json::from_str(r#"{"AND":[{"north":"true"},{"east":"true"}]}"#).unwrap();
        assert!(!matches_when(&when, &props));
    }
}
