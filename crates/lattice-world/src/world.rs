//! The client's copy of the level: loaded columns, block access, light,
//! and the set of sections that need a new mesh.

use crate::chunk::{Biomes, Blocks, Column, Geometry, LightArray, nibble};
use crate::registries::DimensionType;
use lattice_proto::BlockPos;
use lattice_proto::packets::play::LightData;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub type SectionPos = [i32; 3];

pub struct World {
    pub geometry: Geometry,
    pub dimension: DimensionType,
    columns: HashMap<(i32, i32), Column>,
    /// Sections whose mesh is out of date.
    dirty: HashSet<SectionPos>,
}

#[inline]
fn local_index(x: i32, y: i32, z: i32) -> usize {
    (((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize
}

impl World {
    pub fn new(dimension: DimensionType) -> Self {
        let geometry = Geometry { min_y: dimension.min_y, height: dimension.height };
        World { geometry, dimension, columns: HashMap::new(), dirty: HashSet::new() }
    }

    /// Asks for every section to be built again, after a change in how they're drawn.
    pub fn mark_all_dirty(&mut self) {
        let keys: Vec<(i32, i32)> = self.columns.keys().copied().collect();
        for (x, z) in keys {
            self.mark_column_dirty(x, z);
        }
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn has_column(&self, x: i32, z: i32) -> bool {
        self.columns.contains_key(&(x, z))
    }

    pub fn column(&self, x: i32, z: i32) -> Option<&Column> {
        self.columns.get(&(x, z))
    }

    /// New biomes for a loaded column, with its sections redrawn for the new tints.
    pub fn update_biomes(&mut self, x: i32, z: i32, data: &[u8], biome_bits: u32) {
        let Some(col) = self.columns.get_mut(&(x, z)) else { return };
        if col.update_biomes(data, biome_bits).is_err() {
            return;
        }
        let (min, n) = (self.geometry.min_section(), self.geometry.sections() as i32);
        for sy in min..min + n {
            self.dirty.insert([x, sy, z]);
        }
    }

    pub fn columns(&self) -> impl Iterator<Item = &Column> {
        self.columns.values()
    }

    fn mark_column_dirty(&mut self, x: i32, z: i32) {
        let min = self.geometry.min_section();
        for s in 0..self.geometry.sections() as i32 {
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                if self.columns.contains_key(&(x + dx, z + dz)) {
                    self.dirty.insert([x + dx, min + s, z + dz]);
                }
            }
        }
    }

    pub fn insert_column(&mut self, col: Column) {
        let (x, z) = (col.x, col.z);
        self.columns.insert((x, z), col);
        self.mark_column_dirty(x, z);
    }

    pub fn remove_column(&mut self, x: i32, z: i32) -> bool {
        let removed = self.columns.remove(&(x, z)).is_some();
        if removed {
            self.dirty.retain(|p| !(p[0] == x && p[2] == z));
        }
        removed
    }

    pub fn clear(&mut self) {
        self.columns.clear();
        self.dirty.clear();
    }

    /// Block state at a position; `None` if the column isn't loaded.
    pub fn block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        let col = self.columns.get(&(x >> 4, z >> 4))?;
        let s = (y - self.geometry.min_y) >> 4;
        if s < 0 || s as usize >= col.sections.len() {
            return Some(0);
        }
        Some(col.sections[s as usize].blocks.get(local_index(x, y, z)) as u32)
    }

    pub fn block_at(&self, p: BlockPos) -> Option<u32> {
        self.block(p.x, p.y, p.z)
    }

    pub fn set_block(&mut self, p: BlockPos, state: u32) {
        let min_y = self.geometry.min_y;
        let Some(col) = self.columns.get_mut(&(p.x >> 4, p.z >> 4)) else { return };
        let s = (p.y - min_y) >> 4;
        if s < 0 || s as usize >= col.sections.len() {
            return;
        }
        let section = &mut col.sections[s as usize];
        let i = local_index(p.x, p.y, p.z);
        if section.blocks.get(i) as u32 == state {
            return;
        }
        Arc::make_mut(&mut section.blocks).set(i, state as u16);
        self.mark_block_dirty(p);
    }

    pub fn set_section_blocks(&mut self, section: SectionPos, blocks: &[(u16, i32)]) {
        let base = BlockPos::new(section[0] * 16, section[1] * 16, section[2] * 16);
        for &(packed, state) in blocks {
            let (lx, lz, ly) = ((packed >> 8) & 15, (packed >> 4) & 15, packed & 15);
            self.set_block(BlockPos::new(base.x + lx as i32, base.y + ly as i32, base.z + lz as i32), state as u32);
        }
    }

    fn mark_block_dirty(&mut self, p: BlockPos) {
        let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
        let (lx, ly, lz) = (p.x & 15, p.y & 15, p.z & 15);
        // Neighbouring sections only when the block touches their face/edge
        // (ambient occlusion and culling read one block across).
        let rx = if lx == 0 { -1..=0 } else if lx == 15 { 0..=1 } else { 0..=0 };
        for dx in rx {
            let ry = if ly == 0 { -1..=0 } else if ly == 15 { 0..=1 } else { 0..=0 };
            for dy in ry {
                let rz = if lz == 0 { -1..=0 } else if lz == 15 { 0..=1 } else { 0..=0 };
                for dz in rz {
                    self.dirty.insert([sx + dx, sy + dy, sz + dz]);
                }
            }
        }
    }

    pub fn apply_light(&mut self, x: i32, z: i32, light: &LightData) {
        if let Some(col) = self.columns.get_mut(&(x, z)) {
            col.apply_light(light);
            self.mark_column_dirty(x, z);
        }
    }

    /// (sky, block) light at a position; unloaded counts as full sky light.
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let Some(col) = self.columns.get(&(x >> 4, z >> 4)) else { return (15, 0) };
        let s = ((y - self.geometry.min_y) >> 4) + 1;
        if s < 0 || s as usize >= col.sky.len() {
            return (if self.dimension.has_skylight { 15 } else { 0 }, 0);
        }
        let i = local_index(x, y, z);
        let sky = col.sky[s as usize].as_ref().map_or(0, |a| nibble(a, i));
        let block = col.block_light[s as usize].as_ref().map_or(0, |a| nibble(a, i));
        (sky, block)
    }

    /// Takes up to `max` dirty sections, nearest to `center` first.
    pub fn take_dirty(&mut self, center: SectionPos, max: usize) -> Vec<SectionPos> {
        if self.dirty.is_empty() {
            return Vec::new();
        }
        let mut v: Vec<SectionPos> = self.dirty.iter().copied().collect();
        let d = |p: &SectionPos| {
            let (a, b, c) = (p[0] - center[0], p[1] - center[1], p[2] - center[2]);
            a * a + b * b + c * c
        };
        v.sort_unstable_by_key(d);
        v.truncate(max);
        for p in &v {
            self.dirty.remove(p);
        }
        v
    }

    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// Whether all eight horizontal neighbours of a column are loaded.
    pub fn neighbours_loaded(&self, x: i32, z: i32) -> bool {
        (-1..=1).all(|dx| (-1..=1).all(|dz| self.columns.contains_key(&(x + dx, z + dz))))
    }

    /// Immutable copy of a section and its 26 neighbours for meshing.
    pub fn neighbourhood(&self, p: SectionPos) -> Option<Neighbourhood> {
        let min_s = self.geometry.min_section();
        let count = self.geometry.sections() as i32;
        let idx = p[1] - min_s;
        if idx < 0 || idx >= count {
            return None;
        }
        let centre = self.columns.get(&(p[0], p[2]))?;
        if centre.sections[idx as usize].blocks.is_uniform_air() {
            // Nothing to draw; still produce an (empty) mesh so stale ones are cleared.
            return Some(Neighbourhood::empty(p));
        }
        let mut n = Neighbourhood::empty(p);
        n.has_skylight = self.dimension.has_skylight;
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let i = Neighbourhood::slot(dx, dy, dz);
                    let Some(col) = self.columns.get(&(p[0] + dx, p[2] + dz)) else { continue };
                    let s = idx + dy;
                    if s >= 0 && s < count {
                        let sec = &col.sections[s as usize];
                        n.blocks[i] = Some(sec.blocks.clone());
                        n.biomes[i] = Some(sec.biomes.clone());
                    }
                    let ls = (s + 1) as usize;
                    if ls < col.sky.len() {
                        n.sky[i] = col.sky[ls].clone();
                        n.block_light[i] = col.block_light[ls].clone();
                    }
                    n.present[i] = true;
                }
            }
        }
        Some(n)
    }
}

/// A section plus its neighbours, owned (via `Arc`) so it can cross threads.
#[derive(Clone)]
pub struct Neighbourhood {
    pub origin: SectionPos,
    pub has_skylight: bool,
    pub present: [bool; 27],
    pub blocks: [Option<Arc<Blocks>>; 27],
    pub biomes: [Option<Arc<Biomes>>; 27],
    pub sky: [Option<LightArray>; 27],
    pub block_light: [Option<LightArray>; 27],
}

impl Neighbourhood {
    fn empty(origin: SectionPos) -> Self {
        Neighbourhood {
            origin,
            has_skylight: true,
            present: [false; 27],
            blocks: Default::default(),
            biomes: Default::default(),
            sky: Default::default(),
            block_light: Default::default(),
        }
    }

    #[inline]
    pub fn slot(dx: i32, dy: i32, dz: i32) -> usize {
        ((dy + 1) * 9 + (dz + 1) * 3 + (dx + 1)) as usize
    }

    #[inline]
    fn locate(x: i32, y: i32, z: i32) -> (usize, usize) {
        let (sx, sy, sz) = (x.div_euclid(16), y.div_euclid(16), z.div_euclid(16));
        (Self::slot(sx, sy, sz), local_index(x, y, z))
    }

    pub fn is_empty(&self) -> bool {
        self.blocks[13].is_none()
    }

    /// Block at section-relative coordinates in -16..32.
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> u32 {
        let (s, i) = Self::locate(x, y, z);
        self.blocks[s].as_ref().map_or(0, |b| b.get(i) as u32)
    }

    #[inline]
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let (s, i) = Self::locate(x, y, z);
        let sky = match &self.sky[s] {
            Some(a) => nibble(a, i),
            None if self.has_skylight && (!self.present[s] || self.blocks[s].is_none()) => 15,
            None => 0,
        };
        let block = self.block_light[s].as_ref().map_or(0, |a| nibble(a, i));
        (sky, block)
    }

    /// Biome ID at section-relative block coordinates.
    pub fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        let (s, _) = Self::locate(x, y, z);
        let (lx, ly, lz) = ((x & 15) >> 2, (y & 15) >> 2, (z & 15) >> 2);
        self.biomes[s].as_ref().or(self.biomes[13].as_ref()).map_or(0, |b| b.get((ly << 4 | lz << 2 | lx) as usize))
    }
}
