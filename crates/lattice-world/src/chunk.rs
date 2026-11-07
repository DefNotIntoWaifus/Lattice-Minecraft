//! Chunk columns, sections and the network chunk format.

use lattice_proto::codec::{DResult, DecodeError, Reader};
use lattice_proto::packets::play::{ChunkData, LightData};
use std::sync::Arc;

pub const SECTION_VOLUME: usize = 4096;

/// Block states of one 16³ section, indexed `y << 8 | z << 4 | x`.
#[derive(Clone, Debug)]
pub enum Blocks {
    Uniform(u16),
    Mixed(Box<[u16; SECTION_VOLUME]>),
}

impl Blocks {
    #[inline]
    pub fn get(&self, i: usize) -> u16 {
        match self {
            Blocks::Uniform(v) => *v,
            Blocks::Mixed(a) => a[i],
        }
    }

    pub fn set(&mut self, i: usize, v: u16) {
        match self {
            Blocks::Uniform(u) if *u == v => {}
            Blocks::Uniform(u) => {
                let mut a = Box::new([*u; SECTION_VOLUME]);
                a[i] = v;
                *self = Blocks::Mixed(a);
            }
            Blocks::Mixed(a) => a[i] = v,
        }
    }

    pub fn is_uniform_air(&self) -> bool {
        matches!(self, Blocks::Uniform(0))
    }
}

/// Biomes of one section: 4×4×4 cells, indexed `y << 4 | z << 2 | x`.
#[derive(Clone, Debug)]
pub enum Biomes {
    Uniform(u16),
    Mixed(Box<[u16; 64]>),
}

impl Biomes {
    pub fn get(&self, i: usize) -> u16 {
        match self {
            Biomes::Uniform(v) => *v,
            Biomes::Mixed(a) => a[i],
        }
    }
}

/// Nibble array of light levels for one section (2048 bytes).
pub type LightArray = Arc<[u8; 2048]>;

#[inline]
pub fn nibble(arr: &[u8; 2048], i: usize) -> u8 {
    (arr[i >> 1] >> ((i & 1) * 4)) & 0xF
}

#[derive(Clone, Debug)]
pub struct Section {
    pub blocks: Arc<Blocks>,
    pub biomes: Arc<Biomes>,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<Section>,
    /// One more section below and above the world: index 0 is the section under min_y.
    pub sky: Vec<Option<LightArray>>,
    pub block_light: Vec<Option<LightArray>>,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Blocks,
    Biomes,
}

/// Decodes a paletted container into a full array of global IDs.
fn read_paletted(r: &mut Reader, entries: usize, kind: Kind, direct_bits: u32, out: &mut [u16]) -> DResult<Option<u16>> {
    let bits = r.u8()? as u32;
    if bits == 0 {
        return Ok(Some(r.var_i32()? as u16));
    }
    let (bits, palette) = match kind {
        Kind::Blocks if bits <= 8 => (bits.max(4), Some(r.array(|r| r.var_i32())?)),
        Kind::Biomes if bits <= 3 => (bits, Some(r.array(|r| r.var_i32())?)),
        _ => (direct_bits, None),
    };
    let per_long = (64 / bits) as usize;
    let longs = entries.div_ceil(per_long);
    let mask = (1u64 << bits) - 1;
    let mut i = 0;
    for _ in 0..longs {
        let mut v = r.u64()?;
        for _ in 0..per_long {
            if i == entries {
                break;
            }
            let idx = (v & mask) as usize;
            v >>= bits;
            out[i] = match &palette {
                Some(p) => *p.get(idx).ok_or(DecodeError::Invalid { field: "palette index", value: idx as i64 })? as u16,
                None => idx as u16,
            };
            i += 1;
        }
    }
    Ok(None)
}

/// Dimension geometry needed to decode chunks.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub min_y: i32,
    pub height: i32,
}

impl Geometry {
    pub fn sections(&self) -> usize {
        (self.height / 16) as usize
    }
    pub fn min_section(&self) -> i32 {
        self.min_y >> 4
    }
}

impl Column {
    pub fn empty(x: i32, z: i32, geo: Geometry) -> Self {
        let n = geo.sections();
        let section = Section { blocks: Arc::new(Blocks::Uniform(0)), biomes: Arc::new(Biomes::Uniform(0)) };
        Column { x, z, sections: vec![section; n], sky: vec![None; n + 2], block_light: vec![None; n + 2] }
    }

    pub fn decode(chunk: &ChunkData, geo: Geometry, block_bits: u32, biome_bits: u32) -> DResult<Column> {
        let mut col = Column::empty(chunk.x, chunk.z, geo);
        let mut r = Reader::new(&chunk.data);
        let mut scratch = vec![0u16; SECTION_VOLUME];
        for s in 0..geo.sections() {
            let _non_air = r.i16()?;
            let _fluids = r.i16()?;
            let blocks = match read_paletted(&mut r, SECTION_VOLUME, Kind::Blocks, block_bits, &mut scratch)? {
                Some(v) => Blocks::Uniform(v),
                None => Blocks::Mixed(scratch.clone().into_boxed_slice().try_into().unwrap()),
            };
            let mut b = [0u16; 64];
            let biomes = match read_paletted(&mut r, 64, Kind::Biomes, biome_bits, &mut b)? {
                Some(v) => Biomes::Uniform(v),
                None => Biomes::Mixed(Box::new(b)),
            };
            col.sections[s] = Section { blocks: Arc::new(blocks), biomes: Arc::new(biomes) };
        }
        col.apply_light(&chunk.light);
        Ok(col)
    }

    /// Replaces the biomes of every section from a `chunks_biomes` entry.
    pub fn update_biomes(&mut self, data: &[u8], biome_bits: u32) -> DResult<()> {
        let mut r = Reader::new(data);
        for section in &mut self.sections {
            let mut b = [0u16; 64];
            let biomes = match read_paletted(&mut r, 64, Kind::Biomes, biome_bits, &mut b)? {
                Some(v) => Biomes::Uniform(v),
                None => Biomes::Mixed(Box::new(b)),
            };
            section.biomes = Arc::new(biomes);
        }
        Ok(())
    }

    pub fn apply_light(&mut self, light: &LightData) {
        fn apply(target: &mut [Option<LightArray>], mask: &[u64], empty: &[u64], arrays: &[Vec<u8>]) {
            let bit = |m: &[u64], i: usize| m.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1);
            let mut next = arrays.iter();
            for (i, slot) in target.iter_mut().enumerate() {
                if bit(mask, i) {
                    if let Some(a) = next.next() {
                        if let Ok(arr) = <[u8; 2048]>::try_from(a.as_slice()) {
                            *slot = Some(Arc::new(arr));
                        }
                    }
                } else if bit(empty, i) {
                    *slot = None;
                }
            }
        }
        apply(&mut self.sky, &light.sky_mask, &light.empty_sky_mask, &light.sky);
        apply(&mut self.block_light, &light.block_mask, &light.empty_block_mask, &light.block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_proto::Writer;

    /// Packs values the way the protocol does: entries never straddle longs.
    fn pack(values: &[u16], bits: u32) -> Vec<u64> {
        let per = (64 / bits) as usize;
        values
            .chunks(per)
            .map(|c| c.iter().enumerate().fold(0u64, |acc, (i, &v)| acc | (v as u64) << (i as u32 * bits)))
            .collect()
    }

    #[test]
    fn decode_indirect_and_direct() {
        let geo = Geometry { min_y: 0, height: 32 };
        let mut w = Writer::new();
        // Section 0: indirect, 2 bits requested → stored with 4 bits, palette [0, 1, 9].
        let vals: Vec<u16> = (0..4096).map(|i| (i % 3) as u16).collect();
        w.i16(100).i16(0).u8(2).var_i32(3).var_i32(0).var_i32(1).var_i32(9);
        for l in pack(&vals, 4) {
            w.u64(l);
        }
        // Biomes: single value 5
        w.u8(0).var_i32(5);
        // Section 1: direct with 16 bits.
        let direct: Vec<u16> = (0..4096).map(|i| (i * 7 % 35000) as u16).collect();
        w.i16(4096).i16(0).u8(16);
        for l in pack(&direct, 16) {
            w.u64(l);
        }
        // Biomes: indirect 1 bit [2, 3]
        let bvals: Vec<u16> = (0..64).map(|i| (i % 2) as u16).collect();
        w.u8(1).var_i32(2).var_i32(2).var_i32(3);
        for l in pack(&bvals, 1) {
            w.u64(l);
        }
        let chunk = ChunkData { x: 0, z: 0, data: w.buf, block_entities: vec![], light: Default::default() };
        let col = Column::decode(&chunk, geo, 16, 7).unwrap();
        let s0 = &col.sections[0].blocks;
        assert_eq!(s0.get(0), 0);
        assert_eq!(s0.get(1), 1);
        assert_eq!(s0.get(2), 9);
        assert_eq!(col.sections[0].biomes.get(10), 5);
        assert_eq!(col.sections[1].blocks.get(4095), (4095 * 7 % 35000) as u16);
        assert_eq!(col.sections[1].biomes.get(1), 3);
    }
}
