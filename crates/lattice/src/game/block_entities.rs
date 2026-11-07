//! Blocks with more to them than a state: what's known about each, and the small animations
//! that run on the client, such as chest lids.

use lattice_proto::{BlockPos, Nbt};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct BlockEnt {
    pub data: Option<Nbt>,
    /// How far a lid is open, 0..1.
    pub open: f32,
    pub prev_open: f32,
    /// Players with this open, as the server reports.
    pub viewers: i32,
    /// Direction and ticks of a ringing bell.
    pub ringing: Option<(i32, u32)>,
    /// Ticks since it was first heard of.
    pub age: u32,
}

#[derive(Default)]
pub struct BlockEntities {
    pub columns: HashMap<(i32, i32), HashMap<BlockPos, BlockEnt>>,
}

impl BlockEntities {
    pub fn clear(&mut self) {
        self.columns.clear();
    }

    pub fn forget_column(&mut self, x: i32, z: i32) {
        self.columns.remove(&(x, z));
    }

    pub fn get(&self, pos: BlockPos) -> Option<&BlockEnt> {
        self.columns.get(&(pos.x >> 4, pos.z >> 4))?.get(&pos)
    }

    /// Makes sure an entry exists, keeping what's already known.
    pub fn touch(&mut self, pos: BlockPos) -> &mut BlockEnt {
        self.columns.entry((pos.x >> 4, pos.z >> 4)).or_default().entry(pos).or_default()
    }

    pub fn set_data(&mut self, pos: BlockPos, data: Nbt) {
        self.touch(pos).data = Some(data);
    }

    pub fn remove(&mut self, pos: BlockPos) {
        if let Some(c) = self.columns.get_mut(&(pos.x >> 4, pos.z >> 4)) {
            c.remove(&pos);
        }
    }

    /// Keeps entries in step with a block changing: blocks that carry data get one, others lose it.
    pub fn block_changed(&mut self, pos: BlockPos, state: u32) {
        if lattice_data::game().block(state).has_entity {
            self.touch(pos);
        } else {
            self.remove(pos);
        }
    }

    /// Server events: lids and bells.
    pub fn event(&mut self, pos: BlockPos, a: u8, b: u8) {
        let e = self.touch(pos);
        if a == 1 {
            e.viewers = b as i32;
            e.ringing = Some((b as i32, 0));
        }
    }

    pub fn tick(&mut self) {
        for column in self.columns.values_mut() {
            for e in column.values_mut() {
                e.age = e.age.saturating_add(1);
                e.prev_open = e.open;
                if e.viewers > 0 {
                    e.open = (e.open + 0.1).min(1.0);
                } else {
                    e.open = (e.open - 0.1).max(0.0);
                }
                if let Some((_, t)) = &mut e.ringing {
                    *t += 1;
                    if *t > 50 {
                        e.ringing = None;
                    }
                }
            }
        }
    }
}
