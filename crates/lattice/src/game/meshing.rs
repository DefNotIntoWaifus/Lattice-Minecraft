//! Background meshing: sections are snapshotted on the main thread and meshed
//! on a rayon pool; results come back over a channel. A generation number per
//! section drops results that a newer job has superseded.

use crossbeam_channel::{Receiver, Sender, unbounded};
use lattice_render::mesher::{MeshContext, SectionMesh, mesh_section};
use lattice_world::{SectionPos, World};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Mesher {
    ctx: Arc<MeshContext>,
    tx: Sender<(u64, SectionMesh)>,
    rx: Receiver<(u64, SectionMesh)>,
    latest: HashMap<SectionPos, u64>,
    next_gen: u64,
    in_flight: usize,
}

const MAX_IN_FLIGHT: usize = 192;

impl Mesher {
    pub fn new(ctx: Arc<MeshContext>) -> Self {
        let (tx, rx) = unbounded();
        Mesher { ctx, tx, rx, latest: HashMap::new(), next_gen: 1, in_flight: 0 }
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub fn clear(&mut self) {
        self.latest.clear();
    }

    pub fn forget_column(&mut self, x: i32, z: i32) {
        self.latest.retain(|p, _| !(p[0] == x && p[2] == z));
    }

    /// Schedules dirty sections near `centre`.
    pub fn schedule(&mut self, world: &mut World, centre: SectionPos) {
        let room = MAX_IN_FLIGHT.saturating_sub(self.in_flight);
        if room == 0 {
            return;
        }
        let mut deferred = Vec::new();
        for pos in world.take_dirty(centre, room) {
            if !world.neighbours_loaded(pos[0], pos[2]) {
                deferred.push(pos);
                continue;
            }
            let Some(n) = world.neighbourhood(pos) else { continue };
            let generation = self.next_gen;
            self.next_gen += 1;
            self.latest.insert(pos, generation);
            self.in_flight += 1;
            let ctx = self.ctx.clone();
            let tx = self.tx.clone();
            rayon::spawn(move || {
                let mesh = mesh_section(&ctx, &n);
                let _ = tx.send((generation, mesh));
            });
        }
        // Sections at the edge wait until their neighbours arrive (columns are
        // re-marked dirty when a neighbour loads).
        drop(deferred);
    }

    /// Finished meshes that are still current.
    pub fn collect(&mut self) -> Vec<SectionMesh> {
        let mut out = Vec::new();
        while let Ok((generation, mesh)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if self.latest.get(&mesh.origin) == Some(&generation) {
                self.latest.remove(&mesh.origin);
                out.push(mesh);
            }
        }
        out
    }

    pub fn set_context(&mut self, ctx: Arc<MeshContext>) {
        // A new server's biomes don't change the quality settings.
        let options = self.ctx.options;
        self.ctx = Arc::new(ctx.with_options(options));
    }

    pub fn options(&self) -> lattice_render::mesher::MeshOptions {
        self.ctx.options
    }

    pub fn set_options(&mut self, options: lattice_render::mesher::MeshOptions) {
        self.ctx = Arc::new(self.ctx.with_options(options));
    }
}
