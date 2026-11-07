//! What's overhead and falling: sun, moon, stars, clouds, rain and snow; and the cracks on
//! blocks being broken and the flames on burning things, which share the same quads.

use super::{Ctx, Pass};
use glam::{DVec3, Mat4, Vec3};
use lattice_render::atlas::Sprite;
use lattice_render::mesher::Vertex;
use std::sync::OnceLock;

/// Directions of the stars: scattered over the sphere, the same each time.
fn star_field() -> &'static Vec<(Vec3, f32)> {
    static STARS: OnceLock<Vec<(Vec3, f32)>> = OnceLock::new();
    STARS.get_or_init(|| {
        let mut state = 10842u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f32 / (1u64 << 53) as f32
        };
        let mut out = Vec::with_capacity(1500);
        while out.len() < 1500 {
            let v = Vec3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0);
            let d = v.length_squared();
            if !(0.01..=1.0).contains(&d) {
                continue;
            }
            out.push((v.normalize(), 0.15 + next() * 0.1));
        }
        out
    })
}

fn push_quad(list: &mut Vec<Vertex>, pos: [Vec3; 4], uv: [[f32; 2]; 4], color: [u8; 4], light: [u8; 4]) {
    for i in 0..4 {
        list.push(Vertex { pos: pos[i].to_array(), uv: uv[i], color, light });
    }
}

fn whole(s: Sprite) -> [[f32; 2]; 4] {
    [[s.u0, s.v0], [s.u0, s.v1], [s.u1, s.v1], [s.u1, s.v0]]
}

/// Sun, moon and stars. `sun` points at the sun; the sky rotates around the north-south line.
pub fn draw_celestial(ctx: &mut Ctx, sun: Vec3, day_time: f64, daylight: f32, rain: f32, has_sky: bool) {
    if !has_sky {
        return;
    }
    let clear = (1.0 - rain).clamp(0.0, 1.0);
    let bodies = ctx.bodies;
    let across = Vec3::Z;
    let up = across.cross(sun).normalize_or_zero();
    let distance = 100.0;
    let white = 255;
    if let Some(s) = bodies.sheet.try_sprite("environment/celestial/sun") {
        let size = 30.0;
        let c = sun * distance;
        let pos = [c - across * size + up * size, c + across * size + up * size, c + across * size - up * size, c - across * size - up * size];
        let a = (clear * 255.0) as u8;
        push_quad(&mut ctx.out.celestial, pos, whole(s), [white, white, white, a], [0; 4]);
    }
    const PHASES: [&str; 8] = ["full_moon", "waning_gibbous", "third_quarter", "waning_crescent", "new_moon", "waxing_crescent", "first_quarter", "waxing_gibbous"];
    let phase = ((day_time / 24000.0).floor() as i64).rem_euclid(8) as usize;
    if let Some(s) = bodies.sheet.try_sprite(&format!("environment/celestial/moon/{}", PHASES[phase])) {
        let size = 20.0;
        let c = -sun * distance;
        let pos = [c + across * size + up * size, c - across * size + up * size, c - across * size - up * size, c + across * size - up * size];
        let a = (clear * 255.0) as u8;
        push_quad(&mut ctx.out.celestial, pos, whole(s), [white, white, white, a], [0; 4]);
    }
    // Stars come out as the day goes.
    let g = (1.0 - (sun.y * 2.0 + 0.25)).clamp(0.0, 1.0);
    let brightness = g * g * 0.5 * clear;
    let _ = daylight;
    if brightness > 0.004 {
        let turn = sun.y.atan2(sun.x);
        let (sn, cs) = turn.sin_cos();
        let w = bodies.sheet.white;
        let t = [(w.u0 + w.u1) / 2.0, (w.v0 + w.v1) / 2.0];
        let color = [255, 255, 255, (brightness * 255.0) as u8];
        for (d, size) in star_field() {
            // Stars wheel with the sun.
            let dir = Vec3::new(d.x * cs - d.y * sn, d.x * sn + d.y * cs, d.z);
            let side = if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }.cross(dir).normalize();
            let top = dir.cross(side);
            let c = dir * distance;
            let s = *size;
            push_quad(&mut ctx.out.celestial, [c - side * s + top * s, c + side * s + top * s, c + side * s - top * s, c - side * s - top * s], [t, t, t, t], color, [0; 4]);
        }
    }
}

/// The game's cloud picture, one cell per pixel.
pub struct CloudMap {
    pub cells: Vec<bool>,
    pub size: u32,
}

impl CloudMap {
    pub fn from_image(img: &image::RgbaImage) -> CloudMap {
        CloudMap { cells: img.pixels().map(|p| p[3] >= 10).collect(), size: img.width() }
    }

    fn at(&self, x: i32, z: i32) -> bool {
        let s = self.size as i32;
        self.cells[(x.rem_euclid(s) + z.rem_euclid(s) * s) as usize]
    }
}

const CELL: f32 = 12.0;
const CLOUD_HEIGHT: f32 = 4.0;

/// Blocky clouds drifting west, as thick slabs with their inside faces left out.
pub fn draw_clouds(ctx: &mut Ctx, camera: DVec3, height: f32, ticks: f64, daylight: f32, rain: f32, range_cells: i32) {
    let bodies = ctx.bodies;
    let map = &bodies.clouds;
    if map.cells.is_empty() {
        return;
    }
    let drift = ticks * 0.03;
    let x = camera.x + drift;
    let z = camera.z + 0.33;
    let (cx, cz) = ((x / CELL as f64).floor() as i32, (z / CELL as f64).floor() as i32);
    let (ox, oz) = ((x - cx as f64 * CELL as f64) as f32, (z - cz as f64 * CELL as f64) as f32);
    let y0 = height - camera.y as f32;
    let y1 = y0 + CLOUD_HEIGHT;
    // Colour by the time of day: a dim blue at night.
    let dim = 1.0 - rain * 0.5;
    let shade = |k: f32| {
        let c = |base: f32, floor: f32| ((base * daylight * (1.0 - floor) + floor) * k * dim * 255.0) as u8;
        [c(1.0, 0.1), c(1.0, 0.1), c(1.0, 0.15), 204]
    };
    let w = bodies.sheet.white;
    let t = [(w.u0 + w.u1) / 2.0, (w.v0 + w.v1) / 2.0];
    let uv = [t, t, t, t];
    let list = &mut ctx.out.clouds;
    let below = y0 > 0.0;
    let above = y1 < 0.0;
    for dz in -range_cells..=range_cells {
        for dx in -range_cells..=range_cells {
            if dx * dx + dz * dz > range_cells * range_cells {
                continue;
            }
            let (gx, gz) = (cx + dx, cz + dz);
            if !map.at(gx, gz) {
                continue;
            }
            let (a, b) = (dx as f32 * CELL - ox, dz as f32 * CELL - oz);
            let (c, d) = (a + CELL, b + CELL);
            if !above {
                push_quad(list, [Vec3::new(a, y0, d), Vec3::new(a, y0, b), Vec3::new(c, y0, b), Vec3::new(c, y0, d)], uv, shade(0.7), [255, 255, 0, 0]);
            }
            if !below {
                push_quad(list, [Vec3::new(a, y1, b), Vec3::new(a, y1, d), Vec3::new(c, y1, d), Vec3::new(c, y1, b)], uv, shade(1.0), [255, 255, 0, 0]);
            }
            if !map.at(gx, gz - 1) && b > 0.0 {
                push_quad(list, [Vec3::new(c, y1, b), Vec3::new(c, y0, b), Vec3::new(a, y0, b), Vec3::new(a, y1, b)], uv, shade(0.9), [255, 255, 0, 0]);
            }
            if !map.at(gx, gz + 1) && d < 0.0 {
                push_quad(list, [Vec3::new(a, y1, d), Vec3::new(a, y0, d), Vec3::new(c, y0, d), Vec3::new(c, y1, d)], uv, shade(0.9), [255, 255, 0, 0]);
            }
            if !map.at(gx - 1, gz) && a > 0.0 {
                push_quad(list, [Vec3::new(a, y1, b), Vec3::new(a, y0, b), Vec3::new(a, y0, d), Vec3::new(a, y1, d)], uv, shade(0.8), [255, 255, 0, 0]);
            }
            if !map.at(gx + 1, gz) && c < 0.0 {
                push_quad(list, [Vec3::new(c, y1, d), Vec3::new(c, y0, d), Vec3::new(c, y0, b), Vec3::new(c, y1, b)], uv, shade(0.8), [255, 255, 0, 0]);
            }
        }
    }
}

fn column_hash(x: i32, z: i32) -> u32 {
    (x as u32).wrapping_mul(x as u32).wrapping_mul(3121).wrapping_add((x as u32).wrapping_mul(45238971)).wrapping_add((z as u32).wrapping_mul(z as u32).wrapping_mul(418711)).wrapping_add((z as u32).wrapping_mul(13761))
}

/// Falling rain or snow in the columns around the camera. `surface` gives the block that stops it.
#[allow(clippy::too_many_arguments)]
pub fn draw_weather(ctx: &mut Ctx, session: &crate::game::Session, world: &lattice_world::World, camera: DVec3, level: f32, ticks: i64, partial: f32, radius: i32) {
    if level <= 0.0 || !world.dimension.has_skylight {
        return;
    }
    let data = lattice_data::game();
    let r = radius.clamp(3, 10);
    let (cx, cy, cz) = (camera.x.floor() as i32, camera.y.floor() as i32, camera.z.floor() as i32);
    let min_y = cy - r;
    let max_y = cy + r;
    for z in cz - r..=cz + r {
        for x in cx - r..=cx + r {
            let (dx, dz) = ((x - cx) as f32, (z - cz) as f32);
            let d2 = dx * dx + dz * dz;
            if d2 > (r * r) as f32 {
                continue;
            }
            // The first thing rain lands on, looking down from above the view.
            let mut floor = world.geometry.min_y;
            for y in (min_y..=max_y + 40).rev() {
                if let Some(s) = world.block(x, y, z) {
                    let st = data.state(s);
                    if st.has(lattice_data::flags::SOLID) || st.fluid != 0 {
                        floor = y + 1;
                        break;
                    }
                }
            }
            let top = max_y.max(floor);
            let bottom = min_y.max(floor);
            if top <= bottom {
                continue;
            }
            let biome = session.biome_at(x, bottom, z);
            let Some(biome) = biome else { continue };
            if !biome.has_precipitation {
                continue;
            }
            let snowing = biome.temperature - (bottom - 64).max(0) as f32 * 0.00125 < 0.15;
            let hash = column_hash(x, z);
            let alpha = ((1.0 - d2 / (r * r) as f32) * 0.5 + 0.5) * level;
            // Perpendicular to the line of sight to this column.
            let len = (dx * dx + dz * dz).sqrt().max(0.001);
            let (sx, sz) = (dz / len * 0.5, -dx / len * 0.5);
            let (fx, fz) = ((x as f64 + 0.5 - camera.x) as f32, (z as f64 + 0.5 - camera.z) as f32);
            let (y0, y1) = ((bottom as f64 - camera.y) as f32, (top as f64 - camera.y) as f32);
            let (u0, u1, v0, v1);
            if snowing {
                let t = ((ticks & 511) as f32 + partial) / 512.0;
                let spin = (hash >> 8 & 255) as f32 / 256.0;
                let slide = (hash >> 16 & 255) as f32 / 256.0 + ticks as f32 * 0.002;
                (u0, u1) = (spin + slide.fract(), spin + slide.fract() + 1.0);
                (v0, v1) = (bottom as f32 * 0.25 - t, top as f32 * 0.25 - t);
            } else {
                let t = (((ticks as u32).wrapping_add(hash) & 31) as f32 + partial) / 32.0 * (3.0 + (hash >> 8 & 255) as f32 / 256.0);
                (u0, u1) = (0.0, 1.0);
                (v0, v1) = (bottom as f32 * 0.25 + t, top as f32 * 0.25 + t);
            }
            let (sky, block) = world.light(x, floor.max(cy), z);
            let lit = [sky.saturating_mul(17), block.saturating_mul(17), 0, 0];
            let color = [255, 255, 255, (alpha * 255.0) as u8];
            let pos = [Vec3::new(fx - sx, y1, fz - sz), Vec3::new(fx - sx, y0, fz - sz), Vec3::new(fx + sx, y0, fz + sz), Vec3::new(fx + sx, y1, fz + sz)];
            let uv = [[u0, v1], [u0, v0], [u1, v0], [u1, v1]];
            let list = if snowing { &mut ctx.out.snow } else { &mut ctx.out.rain };
            push_quad(list, pos, uv, color, lit);
        }
    }
}

/// Cracks on a block being broken, stage 0 to 9.
pub fn draw_cracks(ctx: &mut Ctx, state: u32, pos: glam::IVec3, stage: u8, camera: DVec3, cracks: &[Sprite; 10]) {
    let s = cracks[stage.min(9) as usize];
    let origin = (DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera).as_vec3();
    let m = Mat4::from_translation(origin + Vec3::splat(0.5)) * Mat4::from_scale(Vec3::splat(1.002)) * Mat4::from_translation(Vec3::splat(-0.5));
    let models = ctx.blocks;
    for model in models.pick(state, 0) {
        for q in &model.quads {
            let n = (q.pos[1] - q.pos[0]).cross(q.pos[3] - q.pos[0]);
            // The picture lies on the block's own grid, whichever way the face points.
            let plane = |p: Vec3| {
                if n.y.abs() > n.x.abs() && n.y.abs() > n.z.abs() {
                    (p.x, p.z)
                } else if n.x.abs() > n.z.abs() {
                    (p.z, 1.0 - p.y)
                } else {
                    (p.x, 1.0 - p.y)
                }
            };
            let list = ctx.block_list(true);
            for k in 0..4 {
                let (u, v) = plane(q.pos[k]);
                list.push(Vertex { pos: m.transform_point3(q.pos[k]).to_array(), uv: s.lerp(u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)), color: [255, 255, 255, 200], light: [255, 255, 0, 0] });
            }
        }
    }
}

/// Flames around something that's burning.
pub fn draw_flames(ctx: &mut Ctx, pos: Vec3, width: f32, height: f32, light: [u8; 2], camera_yaw: f32, fire: [Sprite; 2]) {
    let scale = width * 1.4;
    let m = Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_rotation_y((180.0 - camera_yaw).to_radians());
    let m = m * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.3 - (height / scale).floor() * 0.02));
    let (mut half, mut left, mut lift, mut z, mut i) = (0.5f32, height / scale, 0.0f32, 0.0f32, 0);
    let lit = [light[0], 255, 0, 0];
    while left > 0.0 {
        let s = fire[i % 2];
        let (mut a, mut b) = (s.u0, s.u1);
        if (i / 2) % 2 == 0 {
            std::mem::swap(&mut a, &mut b);
        }
        let corners = [(-half, -lift, a, s.v1), (half, -lift, b, s.v1), (half, 1.4 - lift, b, s.v0), (-half, 1.4 - lift, a, s.v0)];
        let list = ctx.block_list(false);
        for (x, y, u, v) in corners {
            list.push(Vertex { pos: m.transform_point3(Vec3::new(x, y, z)).to_array(), uv: [u, v], color: [255, 255, 255, 255], light: lit });
        }
        left -= 0.45;
        lift -= 0.45;
        half *= 0.9;
        z -= 0.03;
        i += 1;
    }
    let _ = Pass::Cutout;
}

/// The world border, once it's close enough to matter: a shimmering wall that strengthens as
/// you near it, blue when still, red when closing in, green when moving out.
pub fn draw_border(ctx: &mut Ctx, session: &crate::game::Session, camera: DVec3, render_blocks: f64, millis: f64) {
    let b = &session.border;
    let half = b.size / 2.0;
    if half >= 2.9e7 {
        return;
    }
    let (west, east, north, south) = (b.x - half, b.x + half, b.z - half, b.z + half);
    let to_edge = (camera.x - west).min(east - camera.x).min(camera.z - north).min(south - camera.z);
    if to_edge >= render_blocks {
        return;
    }
    let strength = ((1.0 - to_edge / render_blocks).clamp(0.0, 1.0) as f32).powi(4);
    let color = if b.lerp_ticks > 0.0 && b.target < b.size {
        [255, 48, 48]
    } else if b.lerp_ticks > 0.0 && b.target > b.size {
        [64, 255, 128]
    } else {
        [32, 160, 255]
    };
    let Some(s) = ctx.bodies.sheet.try_sprite("misc/forcefield") else { return };
    let alpha = (strength * 255.0) as u8;
    let shift = (millis % 3000.0 / 3000.0) as f32;
    let rgba = [color[0], color[1], color[2], alpha];
    let full = [255, 255, 0, 0];
    // One two-block tile after another, only as far round as can be noticed.
    const REACH: i32 = 32;
    let (cy, base_u) = (camera.y.floor() as i32, shift);
    let (u0, u1, v0, v1) = (s.u0, s.u1, s.v0, s.v1);
    let tile = |list: &mut Vec<Vertex>, a: DVec3, b: DVec3, h: f64| {
        let (pa, pb) = ((a - camera).as_vec3(), (b - camera).as_vec3());
        let up = Vec3::new(0.0, h as f32, 0.0);
        // Drift along the wall: start the picture part way through and wrap it at the seam.
        let cut = base_u;
        let mid = pa + (pb - pa) * (1.0 - cut);
        let split = u0 + (u1 - u0) * cut;
        for (p0, p1, t0, t1) in [(pa, mid, split, u1), (mid, pb, u0, split)] {
            list.push(Vertex { pos: (p0 + up).to_array(), uv: [t0, v0], color: rgba, light: full });
            list.push(Vertex { pos: p0.to_array(), uv: [t0, v1], color: rgba, light: full });
            list.push(Vertex { pos: p1.to_array(), uv: [t1, v1], color: rgba, light: full });
            list.push(Vertex { pos: (p1 + up).to_array(), uv: [t1, v0], color: rgba, light: full });
        }
    };
    let list = ctx.list(super::Pass::Glow);
    let steps = |c: f64| ((c / 2.0).floor() as i32 - REACH / 2..(c / 2.0).floor() as i32 + REACH / 2).map(|i| i as f64 * 2.0);
    for row in (cy / 2 - 12..cy / 2 + 12).map(|r| r as f64 * 2.0) {
        for (x, on) in [(west, camera.x - west < REACH as f64), (east, east - camera.x < REACH as f64)] {
            if on {
                for z in steps(camera.z).filter(|z| *z >= north && *z + 2.0 <= south) {
                    tile(list, DVec3::new(x, row, z), DVec3::new(x, row, z + 2.0), 2.0);
                }
            }
        }
        for (z, on) in [(north, camera.z - north < REACH as f64), (south, south - camera.z < REACH as f64)] {
            if on {
                for x in steps(camera.x).filter(|x| *x >= west && *x + 2.0 <= east) {
                    tile(list, DVec3::new(x, row, z), DVec3::new(x + 2.0, row, z), 2.0);
                }
            }
        }
    }
}
