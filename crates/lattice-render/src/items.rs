//! Item appearance: the game's item definitions (`items/*.json`) choose models, and
//! each model becomes an icon. Flat items are stacks of tintable sprites; block-shaped
//! items are drawn once at load into an icon atlas with a small software rasterizer, so
//! no GPU work is needed to show an inventory. Everything comes from the player's own
//! game files at run time.

use crate::atlas::Atlas;
use crate::models::{BakedModel, Loader, Quad, resolve_texture};
use glam::{Mat3, Mat4, Vec3};
use image::RgbaImage;
use lattice_assets::Resources;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Pixels per side of a block-shaped icon.
const ICON: u32 = 48;

/// A colour for one layer of a model.
#[derive(Debug, Clone)]
pub enum Tint {
    Constant(u32),
    Dye(u32),
    Grass { temperature: f32, downfall: f32 },
    Firework(u32),
    Potion(u32),
    MapColor(u32),
    Team(u32),
    CustomModelData { index: usize, default: u32 },
}

fn argb(v: Option<&Value>) -> u32 {
    v.and_then(|d| match d {
        Value::Number(n) => n.as_i64().map(|i| i as u32),
        Value::Array(a) if a.len() >= 3 => {
            let c = |i: usize| (a[i].as_f64().unwrap_or(1.0) * 255.0) as u32;
            Some(0xFF00_0000 | c(0) << 16 | c(1) << 8 | c(2))
        }
        _ => None,
    })
    .unwrap_or(0xFFFF_FFFF)
}

fn parse_tint(v: &Value) -> Option<Tint> {
    let d = v.get("default");
    Some(match v.get("type")?.as_str()?.trim_start_matches("minecraft:") {
        "constant" => Tint::Constant(argb(v.get("value"))),
        "dye" => Tint::Dye(argb(d)),
        "grass" => Tint::Grass {
            temperature: v.get("temperature").and_then(Value::as_f64).unwrap_or(0.5) as f32,
            downfall: v.get("downfall").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        },
        "firework" => Tint::Firework(argb(d)),
        "potion" => Tint::Potion(argb(d)),
        "map_color" => Tint::MapColor(argb(d)),
        "team" => Tint::Team(argb(d)),
        "custom_model_data" => Tint::CustomModelData { index: v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize, default: argb(d) },
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub enum ItemDef {
    Empty,
    Model { model: String, tints: Vec<Tint> },
    Composite(Vec<ItemDef>),
    Condition { property: String, component: Option<String>, on_true: Box<ItemDef>, on_false: Box<ItemDef> },
    Select { property: String, block_state_property: Option<String>, pattern: Option<String>, cases: Vec<(Vec<String>, ItemDef)>, fallback: Box<ItemDef> },
    Range { property: String, scale: f32, entries: Vec<(f32, ItemDef)>, fallback: Box<ItemDef> },
    /// Drawn by code in the game; `kind` says what ("chest", "banner", ...).
    Special { base: String, kind: String, texture: Option<String> },
}

fn parse_def(v: &Value) -> ItemDef {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("").trim_start_matches("minecraft:");
    let boxed = |k: &str| Box::new(v.get(k).map_or(ItemDef::Empty, parse_def));
    let prop = || v.get("property").and_then(Value::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string();
    match kind {
        "model" => ItemDef::Model {
            model: v.get("model").and_then(Value::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string(),
            tints: v.get("tints").and_then(Value::as_array).map(|t| t.iter().filter_map(parse_tint).collect()).unwrap_or_default(),
        },
        "composite" => ItemDef::Composite(v.get("models").and_then(Value::as_array).map(|m| m.iter().map(parse_def).collect()).unwrap_or_default()),
        "condition" => ItemDef::Condition {
            property: prop(),
            component: v.get("component").or_else(|| v.get("predicate")).and_then(Value::as_str).map(|c| c.trim_start_matches("minecraft:").to_string()),
            on_true: boxed("on_true"),
            on_false: boxed("on_false"),
        },
        "select" => ItemDef::Select {
            property: prop(),
            block_state_property: v.get("block_state_property").and_then(Value::as_str).map(str::to_string),
            pattern: v.get("pattern").and_then(Value::as_str).map(str::to_string),
            cases: v
                .get("cases")
                .and_then(Value::as_array)
                .map(|cases| {
                    cases
                        .iter()
                        .map(|c| {
                            let when = match c.get("when") {
                                Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(|s| s.trim_start_matches("minecraft:").to_string())).collect(),
                                Some(Value::String(s)) => vec![s.trim_start_matches("minecraft:").to_string()],
                                _ => Vec::new(),
                            };
                            (when, c.get("model").map_or(ItemDef::Empty, parse_def))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            fallback: boxed("fallback"),
        },
        "range_dispatch" => ItemDef::Range {
            property: prop(),
            scale: v.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            entries: {
                let mut e: Vec<(f32, ItemDef)> = v
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().map(|x| (x.get("threshold").and_then(Value::as_f64).unwrap_or(0.0) as f32, x.get("model").map_or(ItemDef::Empty, parse_def))).collect())
                    .unwrap_or_default();
                e.sort_by(|a, b| a.0.total_cmp(&b.0));
                e
            },
            fallback: boxed("fallback"),
        },
        "special" => {
            let m = v.get("model");
            ItemDef::Special {
                base: v.get("base").and_then(Value::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string(),
                kind: m.and_then(|m| m.get("type")).and_then(Value::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string(),
                texture: m.and_then(|m| m.get("texture").or_else(|| m.get("name")).or_else(|| m.get("kind")).or_else(|| m.get("color"))).and_then(Value::as_str).map(|s| s.trim_start_matches("minecraft:").to_string()),
            }
        }
        // Bundles show their selected item in hand; the plain model is what shows here.
        "bundle/selected_item" => ItemDef::Empty,
        _ => ItemDef::Empty,
    }
}

/// How one model looks as an icon. Rectangles are normalised icon atlas coordinates.
#[derive(Debug, Clone)]
pub enum IconModel {
    /// Sprites on top of each other, each with its own tint slot.
    Flat(Vec<[f32; 4]>),
    /// A picture of the whole model, colours included.
    Baked([f32; 4]),
}

/// Answers to whatever an item definition asks about a stack. Everything has a sensible
/// default so plain icons need nothing filled in.
pub trait ItemQuery {
    fn has_component(&self, _name: &str) -> bool {
        false
    }
    fn dyed_color(&self) -> Option<u32> {
        None
    }
    fn potion_color(&self) -> Option<u32> {
        None
    }
    fn firework_color(&self) -> Option<u32> {
        None
    }
    fn map_color(&self) -> Option<u32> {
        None
    }
    fn trim_material(&self) -> Option<String> {
        None
    }
    fn charge_type(&self) -> Option<String> {
        None
    }
    fn block_state(&self, _property: &str) -> Option<String> {
        None
    }
    fn display_context(&self) -> &str {
        "gui"
    }
    fn dimension(&self) -> Option<String> {
        None
    }
    fn broken(&self) -> bool {
        false
    }
    fn using(&self) -> bool {
        false
    }
    fn use_ticks(&self) -> f32 {
        0.0
    }
    /// For clocks: 0..1 through the day.
    fn time_of_day(&self) -> f32 {
        0.0
    }
    /// For compasses: 0..1 around the circle.
    fn compass(&self) -> f32 {
        0.0
    }
    fn custom_float(&self, _index: usize) -> Option<f32> {
        None
    }
    fn custom_color(&self, _index: usize) -> Option<u32> {
        None
    }
}

/// A layer ready to draw: where it is in the atlas and how to colour it.
#[derive(Debug, Clone, Copy)]
pub struct IconLayer {
    pub uv: [f32; 4],
    pub tint: [u8; 3],
}

/// Where a held, dropped or worn item sits for one way of being shown.
#[derive(Debug, Clone, Copy)]
pub struct DisplayTransform {
    pub rotation: Vec3,
    /// In blocks.
    pub translation: Vec3,
    pub scale: Vec3,
}

impl Default for DisplayTransform {
    fn default() -> Self {
        DisplayTransform { rotation: Vec3::ZERO, translation: Vec3::ZERO, scale: Vec3::ONE }
    }
}

impl DisplayTransform {
    /// As a matrix, centred so the model's middle is at the origin first.
    pub fn matrix(&self, left_hand: bool) -> Mat4 {
        let (mut ry, mut rz) = (self.rotation.y, self.rotation.z);
        let mut t = self.translation;
        if left_hand {
            ry = -ry;
            rz = -rz;
            t.x = -t.x;
        }
        Mat4::from_translation(t)
            * Mat4::from_rotation_x(self.rotation.x.to_radians())
            * Mat4::from_rotation_y(ry.to_radians())
            * Mat4::from_rotation_z(rz.to_radians())
            * Mat4::from_scale(self.scale)
    }
}

/// One quad of an item's 3D shape, in block space, with block atlas coordinates.
#[derive(Debug, Clone, Copy)]
pub struct ShapeQuad {
    pub pos: [Vec3; 4],
    pub uv: [[f32; 2]; 4],
    pub normal: Vec3,
    pub tint: i32,
    pub translucent: bool,
}

/// The 3D form of an item model.
#[derive(Debug, Clone, Default)]
pub struct ItemShape {
    pub quads: Vec<ShapeQuad>,
    pub display: HashMap<String, DisplayTransform>,
    pub min: Vec3,
    pub max: Vec3,
    /// Generated from a flat texture rather than shaped by blocks.
    pub flat: bool,
}

impl ItemShape {
    /// The display transform for a context. Left hands borrow the right hand's when they've none.
    pub fn transform(&self, context: &str) -> DisplayTransform {
        if let Some(t) = self.display.get(context) {
            return *t;
        }
        let right = match context {
            "thirdperson_lefthand" => "thirdperson_righthand",
            "firstperson_lefthand" => "firstperson_righthand",
            _ => return DisplayTransform::default(),
        };
        self.display.get(right).copied().unwrap_or_default()
    }
}

/// What to draw for a stack: shapes with their tint colours, and things only code can draw.
pub enum ItemPiece<'a> {
    Shape { shape: &'a ItemShape, tints: Vec<[u8; 3]> },
    Special { base: &'a ItemShape, kind: &'a str, texture: Option<&'a str> },
}

/// Draws an item that only code knows the shape of: returns quads in block space and the
/// picture they're textured from, grouped by what they sample.
pub type SpecialDrawer<'a> = &'a dyn Fn(&str, Option<&str>, &str) -> Vec<(Vec<crate::mesher::Vertex>, &'a RgbaImage)>;

pub struct ItemVisuals {
    pub atlas: RgbaImage,
    defs: HashMap<String, ItemDef>,
    models: HashMap<String, IconModel>,
    /// Pictures of items drawn by code, by kind, texture and item.
    special_icons: HashMap<String, [f32; 4]>,
    /// Where each banner pattern's picture is on the sheet.
    pub banner_layers: HashMap<String, [f32; 4]>,
    pub shield_layers: HashMap<String, [f32; 4]>,
    shapes: HashMap<String, ItemShape>,
    grass_colormap: Option<RgbaImage>,
    fallback: Option<IconModel>,
}

/// Models drawn in the world that no item names, such as the wooden part of an item frame.
const EXTRA_SHAPES: [&str; 4] = ["block/item_frame", "block/item_frame_map", "block/glow_item_frame", "block/glow_item_frame_map"];

/// Every way a model can name a display transform.
const DISPLAY_CONTEXTS: [&str; 10] = [
    "thirdperson_righthand",
    "thirdperson_lefthand",
    "firstperson_righthand",
    "firstperson_lefthand",
    "head",
    "gui",
    "ground",
    "fixed",
    "on_shelf",
    "none",
];

fn read_displays(loader: &mut Loader, name: &str) -> HashMap<String, DisplayTransform> {
    let mut out: HashMap<String, DisplayTransform> = HashMap::new();
    let mut cur = Some(name.trim_start_matches("minecraft:").to_string());
    let mut guard = 0;
    while let Some(n) = cur.take() {
        guard += 1;
        if guard > 32 || n.starts_with("builtin/") {
            break;
        }
        let Some(v) = loader.raw(&n) else { break };
        if let Some(display) = v.get("display") {
            for ctx in DISPLAY_CONTEXTS {
                if out.contains_key(ctx) {
                    continue;
                }
                let Some(g) = display.get(ctx) else { continue };
                let read = |k: &str, default: Vec3| {
                    g.get(k).and_then(Value::as_array).filter(|a| a.len() == 3).map_or(default, |a| Vec3::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32))
                };
                // The game clamps these on loading.
                let t = read("translation", Vec3::ZERO).clamp(Vec3::splat(-80.0), Vec3::splat(80.0)) / 16.0;
                let sc = read("scale", Vec3::ONE).clamp(Vec3::splat(-4.0), Vec3::splat(4.0));
                out.insert(ctx.to_string(), DisplayTransform { rotation: read("rotation", Vec3::ZERO), translation: t, scale: sc });
            }
        }
        cur = v.get("parent").and_then(Value::as_str).map(|p| p.trim_start_matches("minecraft:").to_string());
    }
    // Left hands use the right hand's numbers mirrored, when they have none of their own.
    out
}

/// Thickens a flat texture into a 3D item: a front, a back and an edge around each pixel.
fn extrude(sprite: crate::atlas::Sprite, layer: i32, atlas: &Atlas, out: &mut Vec<ShapeQuad>) {
    let img = &atlas.mips[0];
    let size = atlas.size as f32;
    let (x0, y0) = ((sprite.u0 * size).round() as u32, (sprite.v0 * size).round() as u32);
    let w = ((sprite.u1 - sprite.u0) * size).round() as i32;
    let h = ((sprite.v1 - sprite.v0) * size).round() as i32;
    let solid = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && img.get_pixel(x0 + x as u32, y0 + y as u32)[3] > 8;
    let (z0, z1) = (7.5 / 16.0, 8.5 / 16.0);
    let translucent = sprite.alpha == 2;
    let (u0, v0, u1, v1) = (sprite.u0, sprite.v0, sprite.u1, sprite.v1);
    out.push(ShapeQuad {
        pos: [Vec3::new(0.0, 1.0, z1), Vec3::new(0.0, 0.0, z1), Vec3::new(1.0, 0.0, z1), Vec3::new(1.0, 1.0, z1)],
        uv: [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
        normal: Vec3::Z,
        tint: layer,
        translucent,
    });
    out.push(ShapeQuad {
        pos: [Vec3::new(1.0, 1.0, z0), Vec3::new(1.0, 0.0, z0), Vec3::new(0.0, 0.0, z0), Vec3::new(0.0, 1.0, z0)],
        uv: [[u1, v0], [u1, v1], [u0, v1], [u0, v0]],
        normal: Vec3::NEG_Z,
        tint: layer,
        translucent,
    });
    let (pw, ph) = (1.0 / w as f32, 1.0 / h as f32);
    let (du, dv) = ((u1 - u0) * pw, (v1 - v0) * ph);
    // A sliver in the pixel's middle, so the edge takes only that pixel's colour.
    let (eu, ev) = (du * 0.0625, dv * 0.0625);
    for y in 0..h {
        for x in 0..w {
            if !solid(x, y) {
                continue;
            }
            let (px, py) = (x as f32 * pw, 1.0 - y as f32 * ph);
            let (qx, qy) = (px + pw, py - ph);
            let (a, b) = (u0 + x as f32 * du, v0 + y as f32 * dv);
            let (c, d) = (a + du, b + dv);
            let mut edge = |pos: [Vec3; 4], uv: [[f32; 2]; 4], normal: Vec3| {
                out.push(ShapeQuad { pos, uv, normal, tint: layer, translucent });
            };
            if !solid(x, y - 1) {
                edge(
                    [Vec3::new(px, py, z0), Vec3::new(px, py, z1), Vec3::new(qx, py, z1), Vec3::new(qx, py, z0)],
                    [[a, b + ev], [a, b + dv - ev], [c, b + dv - ev], [c, b + ev]],
                    Vec3::Y,
                );
            }
            if !solid(x, y + 1) {
                edge(
                    [Vec3::new(px, qy, z1), Vec3::new(px, qy, z0), Vec3::new(qx, qy, z0), Vec3::new(qx, qy, z1)],
                    [[a, d - dv + ev], [a, d - ev], [c, d - ev], [c, d - dv + ev]],
                    Vec3::NEG_Y,
                );
            }
            if !solid(x - 1, y) {
                edge(
                    [Vec3::new(px, py, z0), Vec3::new(px, qy, z0), Vec3::new(px, qy, z1), Vec3::new(px, py, z1)],
                    [[a + eu, b], [a + eu, d], [a + du - eu, d], [a + du - eu, b]],
                    Vec3::NEG_X,
                );
            }
            if !solid(x + 1, y) {
                edge(
                    [Vec3::new(qx, py, z1), Vec3::new(qx, qy, z1), Vec3::new(qx, qy, z0), Vec3::new(qx, py, z0)],
                    [[c - du + eu, b], [c - du + eu, d], [c - eu, d], [c - eu, b]],
                    Vec3::X,
                );
            }
        }
    }
}

fn finish_shape(mut quads: Vec<ShapeQuad>, display: HashMap<String, DisplayTransform>, flat: bool) -> ItemShape {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for q in &quads {
        for p in q.pos {
            min = min.min(p);
            max = max.max(p);
        }
    }
    if quads.is_empty() {
        (min, max) = (Vec3::splat(0.0), Vec3::splat(1.0));
    }
    quads.shrink_to_fit();
    ItemShape { quads, display, min, max, flat }
}

fn to_rgb(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

fn walk_models(d: &ItemDef, out: &mut BTreeSet<String>) {
    match d {
        ItemDef::Empty => {}
        ItemDef::Model { model, .. } => {
            out.insert(model.clone());
        }
        ItemDef::Composite(list) => list.iter().for_each(|m| walk_models(m, out)),
        ItemDef::Condition { on_true, on_false, .. } => {
            walk_models(on_true, out);
            walk_models(on_false, out);
        }
        ItemDef::Select { cases, fallback, .. } => {
            cases.iter().for_each(|(_, m)| walk_models(m, out));
            walk_models(fallback, out);
        }
        ItemDef::Range { entries, fallback, .. } => {
            entries.iter().for_each(|(_, m)| walk_models(m, out));
            walk_models(fallback, out);
        }
        ItemDef::Special { base, .. } => {
            out.insert(base.clone());
        }
    }
}

fn special_key(item: &str, kind: &str, texture: Option<&str>) -> String {
    format!("{item}|{kind}|{}", texture.unwrap_or(""))
}

/// Everything in a definition that only code can draw.
fn collect_specials(d: &ItemDef, item: &str, out: &mut Vec<(String, String, String, Option<String>)>) {
    match d {
        ItemDef::Empty | ItemDef::Model { .. } => {}
        ItemDef::Composite(list) => list.iter().for_each(|m| collect_specials(m, item, out)),
        ItemDef::Condition { on_true, on_false, .. } => {
            collect_specials(on_true, item, out);
            collect_specials(on_false, item, out);
        }
        ItemDef::Select { cases, fallback, .. } => {
            cases.iter().for_each(|(_, m)| collect_specials(m, item, out));
            collect_specials(fallback, item, out);
        }
        ItemDef::Range { entries, fallback, .. } => {
            entries.iter().for_each(|(_, m)| collect_specials(m, item, out));
            collect_specials(fallback, item, out);
        }
        ItemDef::Special { base, kind, texture } => out.push((item.to_string(), base.clone(), kind.clone(), texture.clone())),
    }
}

/// All item definitions in the game files.
pub fn load_definitions(res: &Resources) -> HashMap<String, ItemDef> {
    let mut out = HashMap::new();
    for path in res.list("items/", ".json") {
        let Some(v) = res.json(&path) else { continue };
        let name = path.trim_start_matches("items/").trim_end_matches(".json").to_string();
        if let Some(m) = v.get("model") {
            out.insert(name, parse_def(m));
        }
    }
    out
}

/// Textures that block-shaped item models need in the world atlas.
pub fn collect_item_textures(res: &Resources, defs: &HashMap<String, ItemDef>) -> Vec<String> {
    let mut models = BTreeSet::new();
    defs.values().for_each(|d| walk_models(d, &mut models));
    models.extend(EXTRA_SHAPES.iter().map(|s| s.to_string()));
    let mut loader = Loader::new(res);
    let mut names = BTreeSet::new();
    for m in &models {
        let Some(rm) = loader.resolve(m) else { continue };
        for e in &rm.elements {
            if let Some(faces) = e.get("faces").and_then(Value::as_object) {
                for f in faces.values() {
                    if let Some(t) = f.get("texture").and_then(Value::as_str).and_then(|t| resolve_texture(&rm.textures, t)) {
                        names.insert(t);
                    }
                }
            }
        }
        // Flat items go in too: dropped and held ones are drawn in the world.
        for i in 0..8 {
            if let Some(t) = rm.textures.get(&format!("layer{i}")).and_then(Value::as_str) {
                names.insert(t.trim_start_matches("minecraft:").to_string());
            }
        }
    }
    names.into_iter().collect()
}

/// A texture for the block atlas: from its file, or worked out if it's an armour trim.
pub fn load_sprite_or_trim(res: &Resources, name: &str) -> Option<RgbaImage> {
    if name.starts_with("trims/") { load_sprite(res, name) } else { res.image(&format!("textures/{name}.png")) }
}

/// Reads an item texture, working out paletted armour trims, which aren't stored as files.
fn load_sprite(res: &Resources, name: &str) -> Option<RgbaImage> {
    let n = name.trim_start_matches("minecraft:");
    if let Some(img) = res.image(&format!("textures/{n}.png")) {
        let w = img.width();
        return Some(if img.height() > w { image::imageops::crop_imm(&img, 0, 0, w, w).to_image() } else { img });
    }
    // "trims/items/chestplate_trim_gold_darker": the base's grays swapped for the metal's colours.
    let (shape, metal) = n.split_once("_trim_")?;
    let mut img = res.image(&format!("textures/{shape}_trim.png"))?;
    let key = res.image("textures/palettes/trim_base.png")?;
    let palette = res.image(&format!("textures/palettes/trim/{metal}.png"))?;
    let map: HashMap<[u8; 4], [u8; 4]> = key.pixels().map(|p| p.0).zip(palette.pixels().map(|p| p.0)).collect();
    for p in img.pixels_mut() {
        if p[3] != 0 {
            if let Some(c) = map.get(&p.0) {
                p.0 = *c;
            }
        }
    }
    Some(img)
}

/// Display transform of a model for the inventory: rotation in degrees, translation, scale.
struct Display {
    rotation: Vec3,
    translation: Vec3,
    scale: Vec3,
    /// Lit from the front rather than from the side.
    front_light: bool,
}

fn read_display(loader: &mut Loader, name: &str) -> Display {
    let mut d = Display { rotation: Vec3::ZERO, translation: Vec3::ZERO, scale: Vec3::ONE, front_light: false };
    let mut found = false;
    let mut light = None;
    let mut cur = Some(name.trim_start_matches("minecraft:").to_string());
    let mut guard = 0;
    while let Some(n) = cur.take() {
        guard += 1;
        if guard > 32 || n.starts_with("builtin/") {
            break;
        }
        let Some(v) = loader.raw(&n) else { break };
        if light.is_none() {
            light = v.get("gui_light").and_then(Value::as_str).map(str::to_string);
        }
        if !found {
            if let Some(g) = v.get("display").and_then(|x| x.get("gui")) {
                let read = |k: &str, default: Vec3| {
                    g.get(k).and_then(Value::as_array).filter(|a| a.len() == 3).map_or(default, |a| Vec3::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32))
                };
                d.rotation = read("rotation", Vec3::ZERO);
                d.translation = read("translation", Vec3::ZERO);
                d.scale = read("scale", Vec3::ONE);
                found = true;
            }
        }
        cur = v.get("parent").and_then(Value::as_str).map(|p| p.trim_start_matches("minecraft:").to_string());
    }
    d.front_light = light.as_deref() == Some("front");
    d
}

/// Draws a block-shaped model into a square image.
fn rasterize(model: &BakedModel, display: &Display, atlas: &Atlas, tints: &HashMap<i32, [u8; 3]>, size: u32) -> RgbaImage {
    let mut img = RgbaImage::new(size, size);
    let mut depth = vec![f32::NEG_INFINITY; (size * size) as usize];
    let r = display.rotation;
    let rot = Mat3::from_rotation_x(r.x.to_radians()) * Mat3::from_rotation_y(r.y.to_radians()) * Mat3::from_rotation_z(r.z.to_radians());
    // The game's two fixed lights for block-shaped items in menus.
    let (l0, l1) = if display.front_light {
        (Vec3::new(0.2, 1.0, -0.7).normalize(), Vec3::new(-0.2, 1.0, 0.7).normalize())
    } else {
        let m = Mat4::from_rotation_y(1.0821041) * Mat4::from_rotation_x(3.2375858) * Mat4::from_rotation_y(-0.3926991) * Mat4::from_rotation_x(2.3561945);
        (m.transform_vector3(Vec3::new(0.2, 1.0, -0.7).normalize()), m.transform_vector3(Vec3::new(-0.2, 1.0, 0.7).normalize()))
    };
    let tex = &atlas.mips[0];
    let (tw, th) = (tex.width() as f32, tex.height() as f32);
    let s = size as f32;
    let project = |v: Vec3| -> Vec3 {
        let p = rot * ((v - Vec3::splat(0.5)) * display.scale) + display.translation / 16.0;
        Vec3::new((p.x + 0.5) * s, (0.5 - p.y) * s, p.z)
    };
    for q in &model.quads {
        let quad: &Quad = q;
        let pts: [Vec3; 4] = quad.pos.map(project);
        // Lit by the transformed normal.
        let normal = (rot * (quad.pos[1] - quad.pos[0]).cross(quad.pos[3] - quad.pos[0])).normalize_or_zero();
        let light = ((l0.dot(normal).max(0.0) + l1.dot(normal).max(0.0)) * 0.6 + 0.4).min(1.0);
        let tint = tints.get(&quad.tint).copied().unwrap_or([255, 255, 255]);
        for tri in [[0usize, 1, 2], [0, 2, 3]] {
            let (a, b, c) = (pts[tri[0]], pts[tri[1]], pts[tri[2]]);
            let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
            if area.abs() < 1e-6 {
                continue;
            }
            let (x0, x1) = (a.x.min(b.x).min(c.x).floor().max(0.0) as u32, (a.x.max(b.x).max(c.x).ceil() as u32).min(size));
            let (y0, y1) = (a.y.min(b.y).min(c.y).floor().max(0.0) as u32, (a.y.max(b.y).max(c.y).ceil() as u32).min(size));
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b.x - px) * (c.y - py) - (c.x - px) * (b.y - py)) / area;
                    let w1 = ((c.x - px) * (a.y - py) - (a.x - px) * (c.y - py)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let z = a.z * w0 + b.z * w1 + c.z * w2;
                    let i = (y * size + x) as usize;
                    if z <= depth[i] {
                        continue;
                    }
                    let (ua, ub, uc) = (quad.uv[tri[0]], quad.uv[tri[1]], quad.uv[tri[2]]);
                    let u = ua[0] * w0 + ub[0] * w1 + uc[0] * w2;
                    let v = ua[1] * w0 + ub[1] * w1 + uc[1] * w2;
                    let t = tex.get_pixel((u * tw).clamp(0.0, tw - 1.0) as u32, (v * th).clamp(0.0, th - 1.0) as u32);
                    if t[3] < 16 {
                        continue;
                    }
                    depth[i] = z;
                    let m = |c: u8, k: u8| (c as f32 * (k as f32 / 255.0) * light).min(255.0) as u8;
                    img.put_pixel(x, y, image::Rgba([m(t[0], tint[0]), m(t[1], tint[1]), m(t[2], tint[2]), t[3]]));
                }
            }
        }
    }
    img
}

/// Draws arbitrary quads, given in block space, into a square image, lit like a block icon.
fn rasterize_quads(groups: &[(Vec<crate::mesher::Vertex>, &RgbaImage)], display: &Display, size: u32) -> RgbaImage {
    let mut img = RgbaImage::new(size, size);
    let mut depth = vec![f32::NEG_INFINITY; (size * size) as usize];
    let r = display.rotation;
    let rot = Mat3::from_rotation_x(r.x.to_radians()) * Mat3::from_rotation_y(r.y.to_radians()) * Mat3::from_rotation_z(r.z.to_radians());
    let (l0, l1) = if display.front_light {
        (Vec3::new(0.2, 1.0, -0.7).normalize(), Vec3::new(-0.2, 1.0, 0.7).normalize())
    } else {
        let m = Mat4::from_rotation_y(1.0821041) * Mat4::from_rotation_x(3.2375858) * Mat4::from_rotation_y(-0.3926991) * Mat4::from_rotation_x(2.3561945);
        (m.transform_vector3(Vec3::new(0.2, 1.0, -0.7).normalize()), m.transform_vector3(Vec3::new(-0.2, 1.0, 0.7).normalize()))
    };
    let s = size as f32;
    let project = |v: Vec3| -> Vec3 {
        let p = rot * ((v - Vec3::splat(0.5)) * display.scale) + display.translation / 16.0;
        Vec3::new((p.x + 0.5) * s, (0.5 - p.y) * s, p.z)
    };
    for (verts, tex) in groups {
        let (tw, th) = (tex.width() as f32, tex.height() as f32);
        for q in verts.chunks_exact(4) {
            let raw: [Vec3; 4] = [Vec3::from(q[0].pos), Vec3::from(q[1].pos), Vec3::from(q[2].pos), Vec3::from(q[3].pos)];
            let pts = raw.map(project);
            // Either winding: quads here come from both blocks and mob-style parts.
            let mut normal = (rot * (raw[1] - raw[0]).cross(raw[3] - raw[0])).normalize_or_zero();
            if normal.z < 0.0 {
                normal = -normal;
            }
            let light = ((l0.dot(normal).max(0.0) + l1.dot(normal).max(0.0)) * 0.6 + 0.4).min(1.0);
            for tri in [[0usize, 1, 2], [0, 2, 3]] {
                let (a, b, c) = (pts[tri[0]], pts[tri[1]], pts[tri[2]]);
                let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
                if area.abs() < 1e-6 {
                    continue;
                }
                let (x0, x1) = (a.x.min(b.x).min(c.x).floor().max(0.0) as u32, (a.x.max(b.x).max(c.x).ceil().max(0.0) as u32).min(size));
                let (y0, y1) = (a.y.min(b.y).min(c.y).floor().max(0.0) as u32, (a.y.max(b.y).max(c.y).ceil().max(0.0) as u32).min(size));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let w0 = ((b.x - px) * (c.y - py) - (c.x - px) * (b.y - py)) / area;
                        let w1 = ((c.x - px) * (a.y - py) - (a.x - px) * (c.y - py)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                            continue;
                        }
                        let z = a.z * w0 + b.z * w1 + c.z * w2;
                        let i = (y * size + x) as usize;
                        // Layers on the same surface draw over each other.
                        if z < depth[i] - 1e-3 {
                            continue;
                        }
                        let (ua, ub, uc) = (q[tri[0]].uv, q[tri[1]].uv, q[tri[2]].uv);
                        let u = ua[0] * w0 + ub[0] * w1 + uc[0] * w2;
                        let v = ua[1] * w0 + ub[1] * w1 + uc[1] * w2;
                        let t = tex.get_pixel((u * tw).clamp(0.0, tw - 1.0) as u32, (v * th).clamp(0.0, th - 1.0) as u32);
                        if t[3] < 16 {
                            continue;
                        }
                        depth[i] = z;
                        let tint = q[tri[0]].color;
                        let m = |c: u8, k: u8| (c as f32 * (k as f32 / 255.0) * light).min(255.0) as u8;
                        let top = [m(t[0], tint[0]), m(t[1], tint[1]), m(t[2], tint[2])];
                        let under = *img.get_pixel(x, y);
                        let a = t[3] as f32 / 255.0;
                        let mix = |i: usize| if under[3] == 0 { top[i] } else { (top[i] as f32 * a + under[i] as f32 * (1.0 - a)) as u8 };
                        let alpha = if under[3] == 0 { t[3] } else { under[3].max(t[3]) };
                        img.put_pixel(x, y, image::Rgba([mix(0), mix(1), mix(2), alpha]));
                    }
                }
            }
        }
    }
    img
}

/// Places images on a sheet.
struct Sheet {
    image: RgbaImage,
    x: u32,
    y: u32,
    row: u32,
}

impl Sheet {
    fn place(&mut self, img: &RgbaImage) -> [f32; 4] {
        let (w, h) = (img.width(), img.height());
        if self.x + w + 2 > self.image.width() {
            self.x = 0;
            self.y += self.row + 2;
            self.row = 0;
        }
        if self.y + h + 2 > self.image.height() {
            let (ow, oh) = (self.image.width(), self.image.height());
            let mut bigger = RgbaImage::new(ow, oh * 2);
            image::imageops::replace(&mut bigger, &self.image, 0, 0);
            self.image = bigger;
        }
        image::imageops::replace(&mut self.image, img, self.x as i64 + 1, self.y as i64 + 1);
        let at = [self.x + 1, self.y + 1];
        self.x += w + 2;
        self.row = self.row.max(h);
        [at[0] as f32, at[1] as f32, (at[0] + w) as f32, (at[1] + h) as f32]
    }
}

impl ItemVisuals {
    pub fn load(res: &Resources, defs: HashMap<String, ItemDef>, atlas: &Atlas, special: SpecialDrawer, banner_patterns: &[String]) -> ItemVisuals {
        let grass_colormap = res.image("textures/colormap/grass.png");
        let mut names = BTreeSet::new();
        defs.values().for_each(|d| walk_models(d, &mut names));
        names.extend(EXTRA_SHAPES.iter().map(|s| s.to_string()));
        // Constant tints on block-shaped models are part of the picture.
        let mut model_tints: HashMap<String, Vec<Tint>> = HashMap::new();
        fn collect_tints(d: &ItemDef, out: &mut HashMap<String, Vec<Tint>>) {
            match d {
                ItemDef::Model { model, tints } => {
                    out.entry(model.clone()).or_insert_with(|| tints.clone());
                }
                ItemDef::Composite(l) => l.iter().for_each(|m| collect_tints(m, out)),
                ItemDef::Condition { on_true, on_false, .. } => {
                    collect_tints(on_true, out);
                    collect_tints(on_false, out);
                }
                ItemDef::Select { cases, fallback, .. } => {
                    cases.iter().for_each(|(_, m)| collect_tints(m, out));
                    collect_tints(fallback, out);
                }
                ItemDef::Range { entries, fallback, .. } => {
                    entries.iter().for_each(|(_, m)| collect_tints(m, out));
                    collect_tints(fallback, out);
                }
                _ => {}
            }
        }
        defs.values().for_each(|d| collect_tints(d, &mut model_tints));

        let mut loader = Loader::new(res);
        let mut sheet = Sheet { image: RgbaImage::new(2048, 1024), x: 0, y: 0, row: 0 };
        let mut sprites: HashMap<String, [f32; 4]> = HashMap::new();
        let mut models: HashMap<String, IconModel> = HashMap::new();
        let mut shapes: HashMap<String, ItemShape> = HashMap::new();
        for name in &names {
            let Some(rm) = loader.resolve(name) else { continue };
            let displays = read_displays(&mut loader, name);
            if !rm.elements.is_empty() {
                let baked = crate::models::bake_plain(&rm, atlas);
                let quads = baked
                    .quads
                    .iter()
                    .map(|q| ShapeQuad {
                        pos: q.pos,
                        uv: q.uv,
                        normal: (q.pos[1] - q.pos[0]).cross(q.pos[3] - q.pos[0]).normalize_or_zero(),
                        tint: q.tint,
                        translucent: q.layer == crate::models::Layer::Translucent,
                    })
                    .collect();
                shapes.insert(name.clone(), finish_shape(quads, displays, false));
                let display = read_display(&mut loader, name);
                let mut tints = HashMap::new();
                for (i, t) in model_tints.get(name).into_iter().flatten().enumerate() {
                    let c = match t {
                        Tint::Constant(c) | Tint::Dye(c) | Tint::Potion(c) | Tint::Firework(c) | Tint::MapColor(c) | Tint::Team(c) | Tint::CustomModelData { default: c, .. } => to_rgb(*c),
                        Tint::Grass { temperature, downfall } => grass_color(grass_colormap.as_ref(), *temperature, *downfall),
                    };
                    tints.insert(i as i32, c);
                }
                let img = rasterize(&baked, &display, atlas, &tints, ICON);
                models.insert(name.clone(), IconModel::Baked(sheet.place(&img)));
                continue;
            }
            let mut layers = Vec::new();
            let mut quads = Vec::new();
            for i in 0..8 {
                let Some(t) = rm.textures.get(&format!("layer{i}")).and_then(Value::as_str) else { continue };
                let t = t.trim_start_matches("minecraft:").to_string();
                if let Some(sprite) = atlas.sprites.get(&t) {
                    extrude(*sprite, i, atlas, &mut quads);
                }
                if !sprites.contains_key(&t) {
                    let Some(img) = load_sprite(res, &t) else { continue };
                    let rect = sheet.place(&img);
                    sprites.insert(t.clone(), rect);
                }
                layers.push(sprites[&t]);
            }
            shapes.insert(name.clone(), finish_shape(quads, displays, true));
            // Items drawn by code in the game have only a particle texture to go on for now.
            if layers.is_empty() {
                if let Some(t) = rm.textures.get("particle").and_then(Value::as_str) {
                    let t = t.trim_start_matches("minecraft:").to_string();
                    if !sprites.contains_key(&t) {
                        if let Some(img) = load_sprite(res, &t) {
                            let rect = sheet.place(&img);
                            sprites.insert(t.clone(), rect);
                        }
                    }
                    if let Some(r) = sprites.get(&t) {
                        layers.push(*r);
                    }
                }
            }
            if !layers.is_empty() {
                models.insert(name.clone(), IconModel::Flat(layers));
            }
        }
        // Items only code can draw get a picture of what that code draws.
        let mut specials: Vec<(String, String, String, Option<String>)> = Vec::new();
        for (item, def) in &defs {
            collect_specials(def, item, &mut specials);
        }
        specials.sort();
        specials.dedup();
        let mut special_rects: HashMap<String, [f32; 4]> = HashMap::new();
        for (item, base, kind, texture) in &specials {
            let key = special_key(item, kind, texture.as_deref());
            let groups = special(kind, texture.as_deref(), item);
            if groups.iter().all(|(v, _)| v.is_empty()) {
                continue;
            }
            let display = read_display(&mut loader, base);
            let img = rasterize_quads(&groups, &display, ICON);
            special_rects.insert(key, sheet.place(&img));
        }
        // A flag's pattern on its own, in the same pose, to lay over any banner's icon.
        let mut banner_layers: HashMap<String, [f32; 4]> = HashMap::new();
        if let Some((_, base, _, _)) = specials.iter().find(|s| s.2 == "banner") {
            let display = read_display(&mut loader, base);
            for pattern in banner_patterns {
                let groups = special("banner_pattern", Some(pattern), "white_banner");
                if groups.iter().any(|(v, _)| !v.is_empty()) {
                    let img = rasterize_quads(&groups, &display, ICON);
                    banner_layers.insert(pattern.clone(), sheet.place(&img));
                }
            }
        }
        // The same for shields, whose dyed base is a layer too.
        let mut shield_layers: HashMap<String, [f32; 4]> = HashMap::new();
        if let Some((_, base, _, _)) = specials.iter().find(|s| s.2 == "shield") {
            let display = read_display(&mut loader, base);
            for pattern in banner_patterns.iter().map(String::as_str).chain(["base"]) {
                let groups = special("shield_pattern", Some(pattern), "shield");
                if groups.iter().any(|(v, _)| !v.is_empty()) {
                    let img = rasterize_quads(&groups, &display, ICON);
                    shield_layers.insert(pattern.to_string(), sheet.place(&img));
                }
            }
        }
        // Normalise now that the sheet's final size is known.
        let (w, h) = (sheet.image.width() as f32, sheet.image.height() as f32);
        let norm = |r: [f32; 4]| [r[0] / w, r[1] / h, r[2] / w, r[3] / h];
        for m in models.values_mut() {
            match m {
                IconModel::Flat(l) => l.iter_mut().for_each(|r| *r = norm(*r)),
                IconModel::Baked(r) => *r = norm(*r),
            }
        }
        for r in special_rects.values_mut().chain(banner_layers.values_mut()).chain(shield_layers.values_mut()) {
            *r = norm(*r);
        }
        tracing::info!("item icons: {} definitions, {} models, sheet {}x{}", defs.len(), models.len(), sheet.image.width(), sheet.image.height());
        let fallback = models.get("item/barrier").cloned();
        ItemVisuals { atlas: sheet.image, defs, models, special_icons: special_rects, banner_layers, shield_layers, shapes, grass_colormap, fallback }
    }

    /// A model's 3D shape by its name.
    pub fn shape(&self, model: &str) -> Option<&ItemShape> {
        self.shapes.get(model)
    }

    /// What to draw in the world for an item, picked the same way as its icon.
    pub fn pieces(&self, item_model: &str, q: &dyn ItemQuery) -> Vec<ItemPiece<'_>> {
        let mut out = Vec::new();
        if let Some(d) = self.defs.get(item_model.trim_start_matches("minecraft:")) {
            self.pieces_of(d, q, &mut out);
        }
        out
    }

    fn pieces_of<'s>(&'s self, d: &'s ItemDef, q: &dyn ItemQuery, out: &mut Vec<ItemPiece<'s>>) {
        match d {
            ItemDef::Empty => {}
            ItemDef::Model { model, tints } => {
                if let Some(shape) = self.shapes.get(model) {
                    out.push(ItemPiece::Shape { shape, tints: tints.iter().map(|t| self.tint(t, q)).collect() });
                }
            }
            ItemDef::Composite(list) => list.iter().for_each(|m| self.pieces_of(m, q, out)),
            ItemDef::Special { base, kind, texture } => {
                if let Some(shape) = self.shapes.get(base) {
                    out.push(ItemPiece::Special { base: shape, kind, texture: texture.as_deref() });
                }
            }
            other => {
                if let Some(next) = self.choose(other, q) {
                    self.pieces_of(next, q, out);
                }
            }
        }
    }

    /// Follows one step of a definition that depends on the stack.
    fn choose<'s>(&'s self, d: &'s ItemDef, q: &dyn ItemQuery) -> Option<&'s ItemDef> {
        match d {
            ItemDef::Condition { property, component, on_true, on_false } => {
                let yes = match property.as_str() {
                    "has_component" => component.as_deref().is_some_and(|c| q.has_component(c)),
                    "broken" => q.broken(),
                    "using_item" => q.using(),
                    _ => false,
                };
                Some(if yes { on_true } else { on_false })
            }
            ItemDef::Select { property, block_state_property, pattern: _, cases, fallback } => {
                let value = match property.as_str() {
                    "trim_material" => q.trim_material(),
                    "charge_type" => q.charge_type(),
                    "display_context" => Some(q.display_context().to_string()),
                    "block_state" => block_state_property.as_deref().and_then(|p| q.block_state(p)),
                    "context_dimension" => q.dimension(),
                    _ => None,
                };
                Some(value.and_then(|v| cases.iter().find(|(when, _)| when.iter().any(|w| *w == v)).map(|(_, m)| m)).unwrap_or(fallback))
            }
            ItemDef::Range { property, scale, entries, fallback } => {
                let v = match property.as_str() {
                    "time" => q.time_of_day(),
                    "compass" => q.compass(),
                    "use_duration" | "use_cycle" | "crossbow/pull" => q.use_ticks(),
                    _ => 0.0,
                } * scale;
                Some(entries.iter().rev().find(|(t, _)| v >= *t).map_or(&**fallback, |(_, m)| m))
            }
            _ => None,
        }
    }

    /// Layers to draw for an item, whose model name usually matches the item's.
    pub fn resolve(&self, item_model: &str, q: &dyn ItemQuery) -> Vec<IconLayer> {
        let name = item_model.trim_start_matches("minecraft:");
        let mut out = Vec::new();
        match self.defs.get(name) {
            Some(d) => self.resolve_def(d, name, q, &mut out),
            None => {
                tracing::debug!("no item definition for {name}");
            }
        }
        if out.is_empty() {
            if let Some(IconModel::Flat(l)) = &self.fallback {
                out.extend(l.iter().map(|uv| IconLayer { uv: *uv, tint: [255, 255, 255] }));
            }
        }
        out
    }

    fn resolve_def(&self, d: &ItemDef, item: &str, q: &dyn ItemQuery, out: &mut Vec<IconLayer>) {
        match d {
            ItemDef::Empty => {}
            ItemDef::Model { model, tints } => match self.models.get(model) {
                Some(IconModel::Baked(uv)) => out.push(IconLayer { uv: *uv, tint: [255, 255, 255] }),
                Some(IconModel::Flat(layers)) => {
                    for (i, uv) in layers.iter().enumerate() {
                        let tint = tints.get(i).map_or([255, 255, 255], |t| self.tint(t, q));
                        out.push(IconLayer { uv: *uv, tint });
                    }
                }
                None => {}
            },
            ItemDef::Composite(list) => list.iter().for_each(|m| self.resolve_def(m, item, q, out)),
            ItemDef::Special { base, kind, texture } => match self.special_icons.get(&special_key(item, kind, texture.as_deref())) {
                Some(uv) => out.push(IconLayer { uv: *uv, tint: [255, 255, 255] }),
                None => match self.models.get(base) {
                    Some(IconModel::Baked(uv)) => out.push(IconLayer { uv: *uv, tint: [255, 255, 255] }),
                    Some(IconModel::Flat(layers)) => out.extend(layers.iter().map(|uv| IconLayer { uv: *uv, tint: [255, 255, 255] })),
                    None => {}
                },
            },
            other => {
                if let Some(next) = self.choose(other, q) {
                    self.resolve_def(next, item, q, out);
                }
            }
        }
    }

    fn tint(&self, t: &Tint, q: &dyn ItemQuery) -> [u8; 3] {
        let c = match t {
            Tint::Constant(c) => *c,
            Tint::Dye(d) => q.dyed_color().unwrap_or(*d),
            Tint::Firework(d) => q.firework_color().unwrap_or(*d),
            Tint::Potion(d) => q.potion_color().unwrap_or(*d),
            Tint::MapColor(d) => q.map_color().unwrap_or(*d),
            Tint::Team(d) => *d,
            Tint::CustomModelData { index, default } => q.custom_color(*index).unwrap_or(*default),
            Tint::Grass { temperature, downfall } => return grass_color(self.grass_colormap.as_ref(), *temperature, *downfall),
        };
        to_rgb(c)
    }
}

/// The grass colour for a climate, from the game's colour map.
fn grass_color(map: Option<&RgbaImage>, temperature: f32, downfall: f32) -> [u8; 3] {
    let Some(m) = map else { return [124, 189, 107] };
    let t = temperature.clamp(0.0, 1.0);
    let d = downfall.clamp(0.0, 1.0) * t;
    let x = ((1.0 - t) * 255.0) as u32;
    let y = ((1.0 - d) * 255.0) as u32;
    let p = m.get_pixel(x.min(m.width() - 1), y.min(m.height() - 1));
    [p[0], p[1], p[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_parse() {
        let v = serde_json::json!({"type":"minecraft:condition","property":"minecraft:using_item","on_true":{"type":"minecraft:model","model":"minecraft:item/bow_pulling_0"},"on_false":{"type":"minecraft:model","model":"minecraft:item/bow"}});
        match parse_def(&v) {
            ItemDef::Condition { property, on_false, .. } => {
                assert_eq!(property, "using_item");
                assert!(matches!(*on_false, ItemDef::Model { .. }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn colours_from_argb() {
        assert_eq!(to_rgb(0xFF12_3456), [0x12, 0x34, 0x56]);
        assert_eq!(to_rgb(argb(Some(&serde_json::json!(-6265536)))), [0xA0, 0x65, 0x40]);
    }
}
