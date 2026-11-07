//! Part-based models: mobs, block entities and the like. A skeleton is built once from the
//! shapes read out of the player's game files; each frame, code poses a copy of it and writes
//! quads straight into a vertex list.

use crate::atlas::Sprite;
use crate::mesher::Vertex;
use glam::{Mat4, Quat, Vec3};
use lattice_assets::geometry::{Animation, Key, Model, Part, Target};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct PartPose {
    pub pos: Vec3,
    pub rot: Vec3,
    pub scale: Vec3,
    pub visible: bool,
    /// The part's own cubes are left out; children still draw.
    pub skip_draw: bool,
    /// Extra rotation after the Euler angles.
    pub extra: Option<Quat>,
}

impl PartPose {
    fn matrix(&self) -> Mat4 {
        let mut m = Mat4::from_translation(self.pos / 16.0);
        if self.rot != Vec3::ZERO {
            m *= Mat4::from_quat(Quat::from_rotation_z(self.rot.z) * Quat::from_rotation_y(self.rot.y) * Quat::from_rotation_x(self.rot.x));
        }
        if let Some(q) = self.extra {
            m *= Mat4::from_quat(q);
        }
        if self.scale != Vec3::ONE {
            m *= Mat4::from_scale(self.scale);
        }
        m
    }
}

struct SkelQuad {
    pos: [Vec3; 4],
    /// 0..1 across the model's texture.
    uv: [[f32; 2]; 4],
    normal: Vec3,
}

struct SkelPart {
    parent: Option<u16>,
    base: PartPose,
    quads: Vec<SkelQuad>,
    /// Corners of the first cube, in the part's own pixels.
    first_cube: Option<([f32; 3], [f32; 3])>,
}

pub struct Skeleton {
    parts: Vec<SkelPart>,
    names: HashMap<String, u16>,
    pub tex_size: [f32; 2],
}

fn build_cube(c: &lattice_assets::geometry::Cube, tex_size: [f32; 2], out: &mut Vec<SkelQuad>) {
    let [mut x, y, z] = c.origin;
    let (mut f, g, h) = (x + c.size[0], y + c.size[1], z + c.size[2]);
    x -= c.grow[0];
    let y = y - c.grow[1];
    let z = z - c.grow[2];
    f += c.grow[0];
    let g = g + c.grow[1];
    let h = h + c.grow[2];
    if c.mirror {
        std::mem::swap(&mut x, &mut f);
    }
    let v = |a: f32, b: f32, cc: f32| Vec3::new(a, b, cc) / 16.0;
    let (v0, v1, v2, v3) = (v(x, y, z), v(f, y, z), v(f, g, z), v(x, g, z));
    let (v4, v5, v6, v7) = (v(x, y, h), v(f, y, h), v(f, g, h), v(x, g, h));
    let (sx, sy, sz) = (c.size[0], c.size[1], c.size[2]);
    let (u, tv) = (c.tex[0], c.tex[1]);
    let (j, k, l, m, n, o) = (u, u + sz, u + sz + sx, u + sz + sx + sx, u + sz + sx + sz, u + sz + sx + sz + sx);
    let (p, q, r) = (tv, tv + sz, tv + sz + sy);
    let tw = tex_size[0] * c.tex_scale[0];
    let th = tex_size[1] * c.tex_scale[1];
    let mut face = |on: bool, corners: [Vec3; 4], u1: f32, v1: f32, u2: f32, v2: f32, normal: Vec3| {
        if !on {
            return;
        }
        let mut pos = corners;
        let mut uv = [[u2 / tw, v1 / th], [u1 / tw, v1 / th], [u1 / tw, v2 / th], [u2 / tw, v2 / th]];
        let mut normal = normal;
        if c.mirror {
            // The game flips the winding and the X of the normal for mirrored cubes.
            pos.reverse();
            uv.reverse();
            // Its texture coordinates stay with the corners, so re-pair them.
            normal.x = -normal.x;
        }
        out.push(SkelQuad { pos, uv, normal });
    };
    // Faces: down, up, north, south, west, east, in the shapes' flag order.
    face(c.faces[0], [v5, v4, v0, v1], k, p, l, q, Vec3::NEG_Y);
    face(c.faces[1], [v2, v3, v7, v6], l, q, m, p, Vec3::Y);
    face(c.faces[4], [v0, v4, v7, v3], j, q, k, r, Vec3::NEG_X);
    face(c.faces[2], [v1, v0, v3, v2], k, q, l, r, Vec3::NEG_Z);
    face(c.faces[5], [v5, v1, v2, v6], l, q, n, r, Vec3::X);
    face(c.faces[3], [v4, v5, v6, v7], n, q, o, r, Vec3::Z);
}

impl Skeleton {
    pub fn new(model: &Model) -> Skeleton {
        let mut s = Skeleton { parts: Vec::new(), names: HashMap::new(), tex_size: model.tex_size };
        s.add(&model.root, None, "root");
        s
    }

    fn add(&mut self, part: &Part, parent: Option<u16>, name: &str) {
        let index = self.parts.len() as u16;
        let mut quads = Vec::new();
        for c in &part.cubes {
            build_cube(c, self.tex_size, &mut quads);
        }
        self.parts.push(SkelPart {
            first_cube: part.cubes.first().map(|c| (c.origin, [c.origin[0] + c.size[0], c.origin[1] + c.size[1], c.origin[2] + c.size[2]])),
            parent,
            base: PartPose {
                pos: Vec3::from(part.offset),
                rot: Vec3::from(part.rotation),
                scale: Vec3::from(part.scale),
                visible: true,
                skip_draw: false,
                extra: None,
            },
            quads,
        });
        // The first part of a name wins, as outer parts are the ones code looks for.
        self.names.entry(name.to_string()).or_insert(index);
        for (n, child) in &part.children {
            self.add(child, Some(index), n);
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub fn index(&self, name: &str) -> Option<u16> {
        self.names.get(name).copied()
    }
}

/// A posable copy of a skeleton's parts.
pub struct Rig<'a> {
    skeleton: &'a Skeleton,
    poses: Vec<PartPose>,
    spare: PartPose,
}

impl<'a> Rig<'a> {
    pub fn new(skeleton: &'a Skeleton) -> Rig<'a> {
        Rig {
            skeleton,
            poses: skeleton.parts.iter().map(|p| p.base).collect(),
            spare: PartPose { pos: Vec3::ZERO, rot: Vec3::ZERO, scale: Vec3::ONE, visible: true, skip_draw: false, extra: None },
        }
    }

    pub fn skeleton(&self) -> &'a Skeleton {
        self.skeleton
    }

    pub fn has(&self, name: &str) -> bool {
        self.skeleton.names.contains_key(name)
    }

    /// The pose of a part by name. Parts the model lacks write into a spare, so code that
    /// animates a whole family of models needn't check each one.
    pub fn p(&mut self, name: &str) -> &mut PartPose {
        match self.skeleton.names.get(name) {
            Some(i) => &mut self.poses[*i as usize],
            None => &mut self.spare,
        }
    }

    pub fn get(&self, name: &str) -> PartPose {
        match self.skeleton.names.get(name) {
            Some(i) => self.poses[*i as usize],
            None => self.spare,
        }
    }

    /// Takes another rig's pose, part by part, for layers on top of a body.
    pub fn copy_from(&mut self, other: &Rig) {
        let names: Vec<(&String, u16)> = self.skeleton.names.iter().map(|(k, v)| (k, *v)).collect();
        for (name, i) in names {
            if let Some(o) = other.skeleton.names.get(name) {
                let from = other.poses[*o as usize];
                let to = &mut self.poses[i as usize];
                to.pos = from.pos;
                to.rot = from.rot;
                to.scale = from.scale;
                to.extra = from.extra;
            }
        }
    }

    /// The transform of a part relative to the model.
    /// The corners of a part's first cube, in its own pixels.
    pub fn cube_of(&self, name: &str) -> Option<([f32; 3], [f32; 3])> {
        self.skeleton.parts[self.skeleton.index(name)? as usize].first_cube
    }

    pub fn matrix(&self, name: &str) -> Mat4 {
        let Some(i) = self.skeleton.index(name) else { return Mat4::IDENTITY };
        let mut at = Some(i as usize);
        let mut m = Mat4::IDENTITY;
        while let Some(k) = at {
            m = self.poses[k].matrix() * m;
            at = self.skeleton.parts[k].parent.map(|p| p as usize);
        }
        m
    }

    /// Plays a keyframe animation: `seconds` in, weighted by `scale`.
    pub fn play(&mut self, animation: &Animation, seconds: f32, scale: f32) {
        let t = if animation.looping && animation.length > 0.0 { seconds % animation.length } else { seconds };
        for ch in &animation.channels {
            let Some(v) = sample(&ch.keys, t) else { continue };
            let v = Vec3::from(v) * scale;
            let p = self.p(&ch.part);
            match ch.target {
                // Positions run the other way up, the same as the game's own offset.
                Target::Position => p.pos += Vec3::new(v.x, -v.y, v.z),
                Target::Rotation => p.rot += v,
                Target::Scale => p.scale += v,
            }
        }
    }

    /// Plays a looping walk from how far and how fast the mob is moving.
    pub fn play_walk(&mut self, animation: &Animation, position: f32, speed: f32, time_mul: f32, speed_mul: f32) {
        let ms = (position * 50.0 * time_mul) as i64;
        self.play(animation, ms as f32 / 1000.0, (speed * speed_mul).min(1.0));
    }
}

fn catmull(t: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * b - a - 3.0 * c + d) * t * t * t)
}

fn sample(keys: &[Key], t: f32) -> Option<[f32; 3]> {
    if keys.is_empty() {
        return None;
    }
    let after = keys.partition_point(|k| k.time <= t);
    let i = after.saturating_sub(1);
    let j = (i + 1).min(keys.len() - 1);
    let (a, b) = (&keys[i], &keys[j]);
    let f = if j != i { ((t - a.time) / (b.time - a.time)).clamp(0.0, 1.0) } else { 0.0 };
    if b.smooth {
        let p = &keys[i.saturating_sub(1)];
        let n = &keys[(j + 1).min(keys.len() - 1)];
        let c = |x: usize| catmull(f, p.post[x], a.post[x], b.post[x], n.post[x]);
        Some([c(0), c(1), c(2)])
    } else {
        let c = |x: usize| a.post[x] + (b.pre[x] - a.post[x]) * f;
        Some([c(0), c(1), c(2)])
    }
}

/// The game's two fixed lights for entities.
pub fn entity_shade(n: Vec3) -> f32 {
    const L0: Vec3 = Vec3::new(0.16169, 0.80845, -0.56592);
    const L1: Vec3 = Vec3::new(-0.16169, 0.80845, 0.56592);
    ((n.dot(L0).max(0.0) + n.dot(L1).max(0.0)) * 0.6 + 0.4).min(1.0)
}

/// What a drawn model looks like beyond its shape.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub sprite: Sprite,
    pub color: [u8; 4],
    /// Sky and block light, 0..=255.
    pub light: [u8; 2],
    /// Red hurt flash and white flash, 0..=255.
    pub overlay: [u8; 2],
    /// Moves the texture over time, for energy swirls.
    pub scroll: [f32; 2],
    /// Shaded by the fixed lights; glowing parts aren't.
    pub lit: bool,
}

/// Writes a posed model as quads. `m` carries model space to camera-relative space.
pub fn draw(rig: &Rig, m: Mat4, look: &Look, out: &mut Vec<Vertex>) {
    draw_filtered(rig, m, look, out, None);
}

/// Like `draw`, limited to the named part and what hangs off it.
pub fn draw_part(rig: &Rig, m: Mat4, look: &Look, out: &mut Vec<Vertex>, part: &str) {
    if let Some(i) = rig.skeleton.index(part) {
        draw_filtered(rig, m, look, out, Some(i));
    }
}

fn draw_filtered(rig: &Rig, m: Mat4, look: &Look, out: &mut Vec<Vertex>, only: Option<u16>) {
    let skeleton = rig.skeleton;
    // Parents come before children, so one pass resolves every matrix.
    let mut mats: Vec<Mat4> = Vec::with_capacity(skeleton.parts.len());
    let mut hidden: Vec<bool> = Vec::with_capacity(skeleton.parts.len());
    let mut included: Vec<bool> = Vec::with_capacity(skeleton.parts.len());
    let s = &look.sprite;
    let (du, dv) = (s.u1 - s.u0, s.v1 - s.v0);
    let light = [look.light[0], look.light[1], look.overlay[0], look.overlay[1]];
    for (i, part) in skeleton.parts.iter().enumerate() {
        let pose = &rig.poses[i];
        let parent = part.parent.map(|p| p as usize);
        let hide = !pose.visible || parent.is_some_and(|p| hidden[p]);
        hidden.push(hide);
        let inc = match only {
            None => true,
            Some(o) => i == o as usize || parent.is_some_and(|p| included[p]),
        };
        included.push(inc);
        let mat = parent.map_or(m, |p| mats[p]) * pose.matrix();
        mats.push(mat);
        if hide || pose.skip_draw || !inc {
            continue;
        }
        for q in &part.quads {
            let n = mat.transform_vector3(q.normal).normalize_or_zero();
            let shade = if look.lit { entity_shade(n) } else { 1.0 };
            let c = |v: u8| (v as f32 * shade) as u8;
            let color = [c(look.color[0]), c(look.color[1]), c(look.color[2]), look.color[3]];
            for k in 0..4 {
                let p = mat.transform_point3(q.pos[k]);
                let uv = q.uv[k];
                out.push(Vertex {
                    pos: p.to_array(),
                    uv: [s.u0 + (uv[0] + look.scroll[0]) * du, s.v0 + (uv[1] + look.scroll[1]) * dv],
                    color,
                    light,
                });
            }
        }
    }
}
