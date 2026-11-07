//! Turns a 16³ section (plus neighbours) into vertex/index buffers.

use crate::atlas::Sprite;
use crate::models::{BlockModels, Dir, Layer, Quad};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use image::RgbaImage;
use lattice_data::{GameData, flags};
use lattice_world::registries::{Biome, GrassModifier};
use lattice_world::Neighbourhood;
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// Section-local position.
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Tint × face shade × ambient occlusion.
    pub color: [u8; 4],
    /// Sky and block light, 0..=255 (level × 17).
    pub light: [u8; 4],
}

#[derive(Default)]
pub struct LayerMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Default)]
pub struct SectionMesh {
    pub origin: [i32; 3],
    pub layers: [LayerMesh; 3],
}

impl SectionMesh {
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.indices.is_empty())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TintKind {
    None,
    Grass,
    Foliage,
    DryFoliage,
    Water,
    Constant(u32),
    Redstone,
    Stem,
    /// Tint index 0 untinted, 1 grass (petals, wildflowers).
    BlankThenGrass,
}

/// Quality settings that change how meshes are built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOptions {
    pub smooth_lighting: bool,
    /// 0..1 strength of corner shading.
    pub ambient_occlusion: f32,
    /// In steps of four blocks.
    pub biome_blend: i32,
    pub detailed_leaves: bool,
}

impl Default for MeshOptions {
    fn default() -> Self {
        MeshOptions { smooth_lighting: true, ambient_occlusion: 1.0, biome_blend: 1, detailed_leaves: true }
    }
}

/// Everything the mesher needs besides the section itself. Shared across threads;
/// cloning is cheap (the heavy parts are behind `Arc`).
#[derive(Clone)]
pub struct MeshContext {
    pub data: &'static GameData,
    pub models: Arc<BlockModels>,
    pub biomes: Arc<Vec<Biome>>,
    grass_map: Arc<Option<RgbaImage>>,
    foliage_map: Arc<Option<RgbaImage>>,
    dry_foliage_map: Arc<Option<RgbaImage>>,
    tints: Arc<Vec<TintKind>>,
    water_still: Sprite,
    water_flow: Sprite,
    water_overlay: Sprite,
    lava_still: Sprite,
    lava_flow: Sprite,
    /// Blocks fluid doesn't run through, by block index.
    blocks_flow: Arc<std::collections::HashSet<u16>>,
    /// Glass and leaves, which water shows a different side against.
    see_through_side: Arc<Vec<bool>>,
    pub options: MeshOptions,
    leaves: Arc<Vec<bool>>,
}

fn colormap(map: &Option<RgbaImage>, temperature: f32, downfall: f32, fallback: u32) -> u32 {
    let Some(img) = map else { return fallback };
    let t = temperature.clamp(0.0, 1.0);
    let d = downfall.clamp(0.0, 1.0) * t;
    let x = ((1.0 - t) * 255.0) as u32;
    let y = ((1.0 - d) * 255.0) as u32;
    if x >= img.width() || y >= img.height() {
        return fallback;
    }
    let p = img.get_pixel(x, y);
    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
}

impl MeshContext {
    pub fn new(
        data: &'static GameData,
        models: BlockModels,
        res: &lattice_assets::Resources,
        atlas: &crate::atlas::Atlas,
    ) -> Self {
        // Which blocks are tinted how: taken from the game's client (BlockColors).
        let mut tints = vec![TintKind::None; data.blocks.len()];
        let set = |tints: &mut Vec<TintKind>, names: &[&str], k: TintKind| {
            for n in names {
                if let Some(i) = data.blocks.iter().position(|b| b.name == *n) {
                    tints[i] = k;
                }
            }
        };
        set(&mut tints, &["large_fern", "tall_grass", "fern", "short_grass", "potted_fern", "bush", "grass_block", "sugar_cane"], TintKind::Grass);
        set(&mut tints, &["pink_petals", "wildflowers"], TintKind::BlankThenGrass);
        set(&mut tints, &["spruce_leaves"], TintKind::Constant(0x619961));
        set(&mut tints, &["birch_leaves"], TintKind::Constant(0x80A755));
        set(&mut tints, &["oak_leaves", "jungle_leaves", "acacia_leaves", "dark_oak_leaves", "vine", "mangrove_leaves"], TintKind::Foliage);
        set(&mut tints, &["leaf_litter"], TintKind::DryFoliage);
        set(&mut tints, &["water_cauldron", "water", "bubble_column"], TintKind::Water);
        set(&mut tints, &["redstone_wire"], TintKind::Redstone);
        set(&mut tints, &["attached_melon_stem", "attached_pumpkin_stem"], TintKind::Constant(0xE0C71C));
        set(&mut tints, &["melon_stem", "pumpkin_stem"], TintKind::Stem);
        set(&mut tints, &["lily_pad"], TintKind::Constant(0x208030));
        MeshContext {
            data,
            models: Arc::new(models),
            biomes: Arc::new(Vec::new()),
            grass_map: Arc::new(res.image("textures/colormap/grass.png")),
            foliage_map: Arc::new(res.image("textures/colormap/foliage.png")),
            dry_foliage_map: Arc::new(res.image("textures/colormap/dry_foliage.png")),
            tints: Arc::new(tints),
            water_still: atlas.sprite("block/water_still"),
            water_flow: atlas.sprite("block/water_flow"),
            water_overlay: atlas.sprite("block/water_overlay"),
            lava_still: atlas.sprite("block/lava_still"),
            lava_flow: atlas.sprite("block/lava_flow"),
            blocks_flow: Arc::new(lattice_world::physics::MoveTags::defaults(data).blocks_fluid_flow),
            see_through_side: Arc::new(data.blocks.iter().map(|b| b.name.ends_with("glass") || b.name.ends_with("glass_pane") || b.name.ends_with("_leaves") || b.name == "ice" || b.name == "slime_block" || b.name == "honey_block").collect()),
            options: MeshOptions::default(),
            leaves: Arc::new(data.blocks.iter().map(|b| b.name.ends_with("_leaves")).collect()),
        }
    }

    /// The same context with other quality settings.
    pub fn with_options(&self, options: MeshOptions) -> MeshContext {
        MeshContext { options, ..self.clone() }
    }

    /// The same context with a server's biome list (for tints).
    pub fn with_biomes(&self, biomes: Vec<Biome>) -> MeshContext {
        MeshContext { biomes: Arc::new(biomes), ..self.clone() }
    }

    fn biome(&self, id: u16) -> Option<&Biome> {
        self.biomes.get(id as usize)
    }

    fn grass_color(&self, b: &Biome) -> u32 {
        let base = b.grass_color.unwrap_or_else(|| colormap(&self.grass_map, b.temperature, b.downfall, 0x48B518));
        match b.grass_modifier {
            GrassModifier::None => base,
            GrassModifier::DarkForest => ((base & 0xFEFEFE) + 0x28340A) >> 1,
            GrassModifier::Swamp => 0x6A7039,
        }
    }

    fn tint(&self, state: u32, index: i32, n: &Neighbourhood, x: i32, y: i32, z: i32) -> u32 {
        if index < 0 {
            return 0xFFFFFF;
        }
        let block = self.data.block_index(state);
        let kind = self.tints[block];
        // Average the biome cells around the block for smooth borders.
        let reach = self.options.biome_blend.clamp(0, 7);
        let blend = |f: &dyn Fn(&Biome) -> u32| -> u32 {
            let (mut r, mut g, mut bl, mut c) = (0u32, 0u32, 0u32, 0u32);
            for dz in (-reach..=reach).map(|v| v * 4) {
                for dx in (-reach..=reach).map(|v| v * 4) {
                    if let Some(b) = self.biome(n.biome(x + dx, y, z + dz)) {
                        let col = f(b);
                        r += col >> 16 & 255;
                        g += col >> 8 & 255;
                        bl += col & 255;
                        c += 1;
                    }
                }
            }
            if c == 0 { 0xFFFFFF } else { (r / c) << 16 | (g / c) << 8 | (bl / c) }
        };
        match kind {
            TintKind::None => 0xFFFFFF,
            TintKind::Grass => blend(&|b| self.grass_color(b)),
            TintKind::BlankThenGrass if index == 0 => 0xFFFFFF,
            TintKind::BlankThenGrass => blend(&|b| self.grass_color(b)),
            TintKind::Foliage => blend(&|b| b.foliage_color.unwrap_or_else(|| colormap(&self.foliage_map, b.temperature, b.downfall, 0x48B518))),
            TintKind::DryFoliage => blend(&|b| colormap(&self.dry_foliage_map, b.temperature, b.downfall, 0x7B5334)),
            TintKind::Water => blend(&|b| b.water_color),
            TintKind::Constant(c) => c,
            TintKind::Redstone => {
                let power = self
                    .data
                    .properties(state)
                    .iter()
                    .find(|(k, _)| *k == "power")
                    .and_then(|(_, v)| v.parse::<f32>().ok())
                    .unwrap_or(0.0)
                    / 15.0;
                let r = power * 0.6 + if power > 0.0 { 0.4 } else { 0.3 };
                let g = (power * power * 0.7 - 0.5).clamp(0.0, 1.0);
                let b = (power * power * 0.6 - 0.7).clamp(0.0, 1.0);
                ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32
            }
            TintKind::Stem => {
                let age = self
                    .data
                    .properties(state)
                    .iter()
                    .find(|(k, _)| *k == "age")
                    .and_then(|(_, v)| v.parse::<u32>().ok())
                    .unwrap_or(0);
                (age * 32) << 16 | (255 - age * 8) << 8 | age * 4
            }
        }
    }

    fn opaque(&self, state: u32) -> bool {
        self.data.state(state).has(flags::SOLID_RENDER)
    }

    /// Whether the face of `state` towards `dir` is hidden by `neighbour`.
    fn culled(&self, state: u32, neighbour: u32, dir: Dir) -> bool {
        let ns = self.data.state(neighbour);
        if ns.has(flags::CAN_OCCLUDE) && ns.face_full(dir.opposite() as usize) {
            return true;
        }
        let me = self.data.state(state);
        let same_block = self.data.block_index(state) == self.data.block_index(neighbour);
        if me.has(flags::SKIP_SAME) && same_block {
            return true;
        }
        // With detailed leaves off, leaf faces between leaves aren't drawn.
        !self.options.detailed_leaves && self.leaves[self.data.block_index(state)] && self.leaves[self.data.block_index(neighbour)]
    }
}

/// Tangent axes used to find the cells around a face corner.
fn tangents(d: Dir) -> (Vec3, Vec3) {
    match d {
        Dir::Down | Dir::Up => (Vec3::X, Vec3::Z),
        Dir::North | Dir::South => (Vec3::X, Vec3::Y),
        Dir::West | Dir::East => (Vec3::Z, Vec3::Y),
    }
}

const AO_CURVE: [f32; 4] = [0.42, 0.62, 0.8, 1.0];

fn pack_color(rgb: u32, k: f32) -> [u8; 4] {
    let c = |s: u32| ((s & 255) as f32 * k).round().clamp(0.0, 255.0) as u8;
    [c(rgb >> 16), c(rgb >> 8), c(rgb), 255]
}

fn light_bytes(sky: f32, block: f32) -> [u8; 4] {
    [(sky * 17.0).round() as u8, (block * 17.0).round() as u8, 0, 0]
}

pub fn mesh_section(ctx: &MeshContext, n: &Neighbourhood) -> SectionMesh {
    let mut out = SectionMesh { origin: n.origin, ..Default::default() };
    if n.is_empty() {
        return out;
    }
    let data = ctx.data;
    let base = [n.origin[0] * 16, n.origin[1] * 16, n.origin[2] * 16];
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                let state = n.block(x, y, z);
                if state == 0 {
                    continue;
                }
                let info = data.state(state);
                if info.fluid == 1 || info.fluid == 2 {
                    fluid(ctx, n, &mut out, x, y, z, state);
                }
                if info.has(flags::AIR) || info.has(flags::INVISIBLE) {
                    continue;
                }
                let block = data.block(state);
                let (wx, wy, wz) = (base[0] + x, base[1] + y, base[2] + z);
                let seed = lattice_data::position_seed(wx, wy, wz);
                let off = lattice_data::block_offset(block, wx, wy, wz);
                let offset = Vec3::new(off[0] as f32, off[1] as f32, off[2] as f32);
                let origin = Vec3::new(x as f32, y as f32, z as f32);
                for model in ctx.models.pick(state, seed) {
                    for q in &model.quads {
                        if let Some(c) = q.cull {
                            let o = c.offset();
                            if ctx.culled(state, n.block(x + o[0], y + o[1], z + o[2]), c) {
                                continue;
                            }
                        }
                        emit_quad(ctx, n, &mut out, q, model.ao, state, [x, y, z], origin + offset);
                    }
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_quad(ctx: &MeshContext, n: &Neighbourhood, out: &mut SectionMesh, q: &Quad, ao: bool, state: u32, p: [i32; 3], origin: Vec3) {
    let [x, y, z] = p;
    let tint = ctx.tint(state, q.tint, n, x, y, z);
    let mut colors = [[0u8; 4]; 4];
    let mut lights = [[0u8; 4]; 4];
    let mut ao_levels = [3usize; 4];

    match q.face {
        Some(d) if q.full && ao && ctx.options.smooth_lighting => {
            let o = d.offset();
            let (nx, ny, nz) = (x + o[0], y + o[1], z + o[2]);
            let (t1, t2) = tangents(d);
            for (i, corner) in q.pos.iter().enumerate() {
                // Direction from face centre toward this corner along each tangent.
                let rel = *corner - Vec3::splat(0.5);
                let s1 = if rel.dot(t1) > 0.0 { 1 } else { -1 };
                let s2 = if rel.dot(t2) > 0.0 { 1 } else { -1 };
                let a = [t1.x as i32 * s1, t1.y as i32 * s1, t1.z as i32 * s1];
                let b = [t2.x as i32 * s2, t2.y as i32 * s2, t2.z as i32 * s2];
                let cells = [
                    (nx, ny, nz),
                    (nx + a[0], ny + a[1], nz + a[2]),
                    (nx + b[0], ny + b[1], nz + b[2]),
                    (nx + a[0] + b[0], ny + a[1] + b[1], nz + a[2] + b[2]),
                ];
                let side1 = ctx.opaque(n.block(cells[1].0, cells[1].1, cells[1].2));
                let side2 = ctx.opaque(n.block(cells[2].0, cells[2].1, cells[2].2));
                let corner_o = ctx.opaque(n.block(cells[3].0, cells[3].1, cells[3].2));
                let level = if side1 && side2 { 0 } else { 3 - (side1 as usize + side2 as usize + corner_o as usize) };
                ao_levels[i] = level;
                let (mut sky, mut blk, mut cnt) = (0.0f32, 0.0f32, 0.0f32);
                for (ci, c) in cells.iter().enumerate() {
                    // The diagonal cell only contributes if a side cell lets light through.
                    if ci == 3 && side1 && side2 {
                        continue;
                    }
                    if ci > 0 && ctx.opaque(n.block(c.0, c.1, c.2)) {
                        continue;
                    }
                    let (s, b) = n.light(c.0, c.1, c.2);
                    sky += s as f32;
                    blk += b as f32;
                    cnt += 1.0;
                }
                lights[i] = light_bytes(sky / cnt, blk / cnt);
                let shade = 1.0 + (AO_CURVE[level] - 1.0) * ctx.options.ambient_occlusion;
                colors[i] = pack_color(tint, q.shade * shade);
            }
        }
        _ => {
            // Flat light: from the neighbour for boundary faces, else from the block itself.
            let (lx, ly, lz) = match q.face {
                Some(d) if q.full || q.cull.is_some() => {
                    let o = d.offset();
                    (x + o[0], y + o[1], z + o[2])
                }
                _ => (x, y, z),
            };
            let (mut s, mut b) = n.light(lx, ly, lz);
            if (lx, ly, lz) == (x, y, z) {
                // Light inside a partial block can read 0; take the brightest neighbour above.
                let (s2, b2) = n.light(x, y + 1, z);
                s = s.max(s2);
                b = b.max(b2);
            }
            let l = light_bytes(s as f32, b as f32);
            for i in 0..4 {
                lights[i] = l;
                colors[i] = pack_color(tint, q.shade);
            }
        }
    }

    let layer = &mut out.layers[q.layer as usize];
    let start = layer.vertices.len() as u32;
    for i in 0..4 {
        let p = q.pos[i] + origin;
        layer.vertices.push(Vertex { pos: p.to_array(), uv: inset_uv(q.uv, i), color: colors[i], light: lights[i] });
    }
    // Flip the diagonal so AO gradients don't show a seam.
    if ao_levels[0] + ao_levels[2] >= ao_levels[1] + ao_levels[3] {
        layer.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    } else {
        layer.indices.extend_from_slice(&[start + 1, start + 2, start + 3, start + 1, start + 3, start]);
    }
}

/// A corner's texture position, a hair inside the quad. A pixel centred exactly on
/// a tile's edge would otherwise sample the next texture over on the atlas, which
/// shows as dashes along block seams when the camera lines up with the grid.
fn inset_uv(uv: [[f32; 2]; 4], i: usize) -> [f32; 2] {
    const INSET: f32 = 0.004;
    let centre = [(uv[0][0] + uv[2][0]) * 0.5, (uv[0][1] + uv[2][1]) * 0.5];
    [uv[i][0] + (centre[0] - uv[i][0]) * INSET, uv[i][1] + (centre[1] - uv[i][1]) * INSET]
}

/// Which way a fluid block's surface runs, as the game works it out from its neighbours.
fn fluid_flow(ctx: &MeshContext, n: &Neighbourhood, x: i32, y: i32, z: i32, state: u32) -> Vec3 {
    let data = ctx.data;
    let info = data.state(state);
    let kind = info.fluid;
    let height = |s: u32| data.state(s).fluid_amount as f32 / 9.0;
    let own = height(state);
    let affects = |s: u32| {
        let f = data.state(s).fluid;
        f == 0 || f == kind
    };
    const SIDES: [(Dir, i32, i32); 4] = [(Dir::North, 0, -1), (Dir::East, 1, 0), (Dir::South, 0, 1), (Dir::West, -1, 0)];
    let (mut fx, mut fz) = (0.0f32, 0.0f32);
    for (_, dx, dz) in SIDES {
        let nb = n.block(x + dx, y, z + dz);
        if !affects(nb) {
            continue;
        }
        let h = if data.state(nb).fluid == 0 { 0.0 } else { height(nb) };
        let mut d = 0.0;
        if h == 0.0 {
            if !ctx.blocks_flow.contains(&(data.block_index(nb) as u16)) {
                let below = n.block(x + dx, y - 1, z + dz);
                if affects(below) {
                    let hb = if data.state(below).fluid == 0 { 0.0 } else { height(below) };
                    if hb > 0.0 {
                        d = own - (hb - 0.8888889);
                    }
                }
            }
        } else if h > 0.0 {
            d = own - h;
        }
        if d != 0.0 {
            fx += dx as f32 * d;
            fz += dz as f32 * d;
        }
    }
    let mut flow = Vec3::new(fx, 0.0, fz);
    // Falling fluid against a wall runs straight down.
    let falling = info.fluid_amount == 8 && !info.has(flags::FLUID_SOURCE);
    if falling {
        let sturdy = |s: u32, d: Dir| {
            let st = data.state(s);
            if st.fluid == kind {
                return false;
            }
            st.flags >> flags::FACE_STURDY_SHIFT & (1 << d as u32) != 0
        };
        for (d, dx, dz) in SIDES {
            if sturdy(n.block(x + dx, y, z + dz), d) || sturdy(n.block(x + dx, y + 1, z + dz), d) {
                flow = flow.normalize_or_zero() + Vec3::new(0.0, -6.0, 0.0);
                break;
            }
        }
    }
    flow.normalize_or_zero()
}

fn fluid(ctx: &MeshContext, n: &Neighbourhood, out: &mut SectionMesh, x: i32, y: i32, z: i32, state: u32) {
    let data = ctx.data;
    let info = data.state(state);
    let kind = info.fluid;
    let same = |s: u32| data.state(s).fluid == kind;
    // The face of a block towards a direction blocks the fluid when it's whole.
    let occludes = |s: u32, face: Dir, height: f32, up: bool| {
        let st = data.state(s);
        st.has(flags::CAN_OCCLUDE) && st.face_full(face as usize) && (!up || height >= 1.0)
    };
    let hidden_by_self = |d: Dir| occludes(state, d, 1.0, false);

    let above = n.block(x, y + 1, z);
    let below = n.block(x, y - 1, z);
    let draw_up = !same(above);
    let draw_down = !same(below) && !hidden_by_self(Dir::Down) && !occludes(below, Dir::Up, 0.8888889, false);
    let side_blocks = [(Dir::North, n.block(x, y, z - 1)), (Dir::South, n.block(x, y, z + 1)), (Dir::West, n.block(x - 1, y, z)), (Dir::East, n.block(x + 1, y, z))];
    let draw_side = |d: Dir, nb: u32| !same(nb) && !hidden_by_self(d);
    if !draw_up && !draw_down && side_blocks.iter().all(|(d, nb)| !draw_side(*d, *nb)) {
        return;
    }

    let height_of = |px: i32, py: i32, pz: i32| -> f32 {
        let s = n.block(px, py, pz);
        if same(s) {
            if same(n.block(px, py + 1, pz)) { 1.0 } else { data.state(s).fluid_amount as f32 / 9.0 }
        } else if data.state(s).has(flags::SOLID) {
            -1.0
        } else {
            0.0
        }
    };
    let own = height_of(x, y, z);
    let (mut ne, mut nw, mut se, mut sw) = (1.0f32, 1.0f32, 1.0f32, 1.0f32);
    if own < 1.0 {
        let (hn, hs, he, hw) = (height_of(x, y, z - 1), height_of(x, y, z + 1), height_of(x + 1, y, z), height_of(x - 1, y, z));
        let corner = |a: f32, b: f32, dx: i32, dz: i32| -> f32 {
            if b >= 1.0 || a >= 1.0 {
                return 1.0;
            }
            let (mut sum, mut weight) = (0.0f32, 0.0f32);
            let mut add = |v: f32| {
                if v >= 0.8 {
                    sum += v * 10.0;
                    weight += 10.0;
                } else if v >= 0.0 {
                    sum += v;
                    weight += 1.0;
                }
            };
            if b > 0.0 || a > 0.0 {
                let d = height_of(x + dx, y, z + dz);
                if d >= 1.0 {
                    return 1.0;
                }
                add(d);
            }
            add(own);
            add(b);
            add(a);
            if weight > 0.0 { sum / weight } else { 0.0 }
        };
        ne = corner(hn, he, 1, -1);
        nw = corner(hn, hw, -1, -1);
        se = corner(hs, he, 1, 1);
        sw = corner(hs, hw, -1, 1);
    }

    let (still, flowing, overlay, layer, tint) = if kind == 1 {
        (ctx.water_still, ctx.water_flow, Some(ctx.water_overlay), Layer::Translucent, ctx.tint(data.block_by_name("water").map_or(0, |b| b.first), 0, n, x, y, z))
    } else {
        (ctx.lava_still, ctx.lava_flow, None, Layer::Solid, 0xFFFFFF)
    };
    let (s0, b0) = n.light(x, y, z);
    let (s1, b1) = n.light(x, y + 1, z);
    let light = light_bytes(s0.max(s1) as f32, if kind == 2 { 15.0 } else { b0.max(b1) as f32 });
    let (fx, fy, fz) = (x as f32, y as f32, z as f32);
    let down_gap = if draw_down { 0.001 } else { 0.0 };
    let lm = &mut out.layers[layer as usize];
    let mut face = |pos: [Vec3; 4], uv: [[f32; 2]; 4], k: f32, both: bool, light: [u8; 4]| {
        let color = pack_color(tint, k);
        let start = lm.vertices.len() as u32;
        for i in 0..4 {
            lm.vertices.push(Vertex { pos: pos[i].to_array(), uv: uv[i], color, light });
        }
        lm.indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
        if both {
            lm.indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
        }
    };

    if draw_up && !occludes(above, Dir::Down, nw.min(sw).min(se).min(ne), true) {
        let (nw, sw, se, ne) = (nw - 0.001, sw - 0.001, se - 0.001, ne - 0.001);
        let flow = fluid_flow(ctx, n, x, y, z, state);
        let uv = if flow.x == 0.0 && flow.z == 0.0 {
            [[still.u0, still.v0], [still.u0, still.v1], [still.u1, still.v1], [still.u1, still.v0]]
        } else {
            let angle = flow.z.atan2(flow.x) - std::f32::consts::FRAC_PI_2;
            let (a, b) = (angle.sin() * 0.25, angle.cos() * 0.25);
            [
                flowing.lerp(0.5 + (-b - a), 0.5 + (-b + a)),
                flowing.lerp(0.5 + (-b + a), 0.5 + (b + a)),
                flowing.lerp(0.5 + (b + a), 0.5 + (b - a)),
                flowing.lerp(0.5 + (b - a), 0.5 + (-b - a)),
            ]
        };
        let pos = [Vec3::new(fx, fy + nw, fz), Vec3::new(fx, fy + sw, fz + 1.0), Vec3::new(fx + 1.0, fy + se, fz + 1.0), Vec3::new(fx + 1.0, fy + ne, fz)];
        face(pos, uv, Dir::Up.shade(), kind == 1, light);
    }
    if draw_down {
        let (s2, b2) = n.light(x, y - 1, z);
        let low = light_bytes(s0.max(s2) as f32, if kind == 2 { 15.0 } else { b0.max(b2) as f32 });
        let y0 = fy + down_gap;
        let pos = [Vec3::new(fx, y0, fz + 1.0), Vec3::new(fx, y0, fz), Vec3::new(fx + 1.0, y0, fz), Vec3::new(fx + 1.0, y0, fz + 1.0)];
        let uv = [[still.u0, still.v1], [still.u0, still.v0], [still.u1, still.v0], [still.u1, still.v1]];
        face(pos, uv, Dir::Down.shade(), false, low);
    }
    for (d, nb) in side_blocks {
        if !draw_side(d, nb) {
            continue;
        }
        let e = 0.001;
        let (h0, h1, x0, z0, x1, z1) = match d {
            Dir::North => (nw, ne, fx, fz + e, fx + 1.0, fz + e),
            Dir::South => (se, sw, fx + 1.0, fz + 1.0 - e, fx, fz + 1.0 - e),
            Dir::West => (sw, nw, fx + e, fz + 1.0, fx + e, fz),
            _ => (ne, se, fx + 1.0 - e, fz, fx + 1.0 - e, fz + 1.0),
        };
        if occludes(nb, d.opposite(), h0.max(h1), false) {
            continue;
        }
        // Beside glass or leaves, water shows a clearer side and only from outside.
        let beside_clear = overlay.is_some() && ctx.see_through_side[data.block_index(nb)];
        let sprite = if beside_clear { overlay.unwrap_or(flowing) } else { flowing };
        let uv = [sprite.lerp(0.0, (1.0 - h0) * 0.5), sprite.lerp(0.5, (1.0 - h1) * 0.5), sprite.lerp(0.5, 0.5), sprite.lerp(0.0, 0.5)];
        let pos = [Vec3::new(x0, fy + h0, z0), Vec3::new(x1, fy + h1, z1), Vec3::new(x1, fy + down_gap, z1), Vec3::new(x0, fy + down_gap, z0)];
        face(pos, uv, d.shade(), !beside_clear, light);
    }
}
