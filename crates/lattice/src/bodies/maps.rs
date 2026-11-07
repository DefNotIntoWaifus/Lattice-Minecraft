//! Maps: their pictures on the sheet, and drawing one held or in a frame.

use super::{Ctx, Pass};
use crate::game::MapData;
use glam::{Mat4, Vec3};
use lattice_render::atlas::{Sheet, Sprite};
use lattice_render::mesher::Vertex;
use std::collections::HashMap;

/// How many different maps can be shown at once.
pub const SLOTS: u32 = 24;

/// How each number's colour is shaded: the last two bits of a map byte.
const SHADES: [u32; 4] = [180, 220, 255, 135];

/// A map's picture from its colour numbers.
pub fn picture(colors: &[u8]) -> image::RgbaImage {
    let table = &lattice_data::game().map_colors;
    let mut img = image::RgbaImage::new(128, 128);
    for (i, c) in colors.iter().take(128 * 128).enumerate() {
        let id = (*c >> 2) as usize;
        if id == 0 {
            continue;
        }
        let shade = SHADES[(*c & 3) as usize];
        let rgb = table.get(id).copied().unwrap_or(0);
        let part = |s: u32| (((rgb >> s) & 255) * shade / 255) as u8;
        img.put_pixel(i as u32 % 128, i as u32 / 128, image::Rgba([part(16), part(8), part(0), 255]));
    }
    img
}

struct Slot {
    index: usize,
    version: u32,
    last_change: u64,
}

/// Which maps have their picture on the sheet.
#[derive(Default)]
pub struct MapTextures {
    slots: HashMap<i32, Slot>,
    clock: u64,
}

impl MapTextures {
    /// Puts any map that's new or has changed on the sheet.
    pub fn pump(&mut self, maps: &HashMap<i32, MapData>, sheet: &Sheet, mut write: impl FnMut(u32, u32, &image::RgbaImage)) {
        self.clock += 1;
        for (id, map) in maps {
            if self.slots.get(id).is_some_and(|s| s.version == map.version) {
                continue;
            }
            let index = match self.slots.get(id) {
                Some(s) => s.index,
                None => self.free_slot(sheet),
            };
            let Some(&(x, y)) = sheet.map_slots.get(index) else { continue };
            write(x, y, &picture(&map.colors));
            self.slots.insert(*id, Slot { index, version: map.version, last_change: self.clock });
        }
    }

    fn free_slot(&mut self, sheet: &Sheet) -> usize {
        let total = sheet.map_slots.len();
        if self.slots.len() < total {
            let used: Vec<usize> = self.slots.values().map(|s| s.index).collect();
            return (0..total).find(|i| !used.contains(i)).unwrap_or(0);
        }
        // Out of room: the map that hasn't changed for longest gives way.
        let oldest = self.slots.iter().min_by_key(|(_, s)| s.last_change).map(|(id, s)| (*id, s.index));
        match oldest {
            Some((id, index)) => {
                self.slots.remove(&id);
                index
            }
            None => 0,
        }
    }

    pub fn sprite(&self, id: i32, sheet: &Sheet) -> Option<Sprite> {
        sheet.map_sprite(self.slots.get(&id)?.index)
    }
}

/// A quad in the plane of `m`, from one corner to the other.
pub fn quad(ctx: &mut Ctx, m: Mat4, a: [f32; 2], b: [f32; 2], z: f32, s: Sprite, light: [u8; 2]) {
    let corners = [(a[0], b[1], s.u0, s.v1), (b[0], b[1], s.u1, s.v1), (b[0], a[1], s.u1, s.v0), (a[0], a[1], s.u0, s.v0)];
    let list = ctx.list(Pass::Cutout);
    for (x, y, u, v) in corners {
        list.push(Vertex { pos: m.transform_point3(Vec3::new(x, y, z)).to_array(), uv: [u, v], color: [255; 4], light: [light[0], light[1], 0, 0] });
    }
}

/// The picture and its markers, in a space where the map is 128 units wide.
pub fn draw(ctx: &mut Ctx, m: Mat4, id: i32, on_frame: bool, light: [u8; 2]) {
    let Some(map) = ctx.maps.get(&id) else { return };
    let bright = [light[0], 255];
    if let Some(s) = ctx.map_textures.sprite(id, &ctx.bodies.sheet) {
        quad(ctx, m, [0.0, 0.0], [128.0, 128.0], -0.25, s, bright);
    }
    let markers = &lattice_data::game().map_decorations;
    let mut names = Vec::new();
    for (i, d) in map.decorations.iter().enumerate() {
        let Some(info) = markers.get(d.kind as usize) else { continue };
        if on_frame && !info.on_frame {
            continue;
        }
        let at = m
            * Mat4::from_translation(Vec3::new(d.x as f32 / 2.0 + 64.0, d.y as f32 / 2.0 + 64.0, -0.5 - i as f32 * 0.01))
            * Mat4::from_rotation_z((d.rotation as f32 * 360.0 / 16.0).to_radians())
            * Mat4::from_scale(Vec3::new(4.0, 4.0, 3.0))
            * Mat4::from_translation(Vec3::new(-0.125, 0.125, 0.0));
        if let Some(s) = ctx.bodies.sheet.try_sprite(&format!("map/decorations/{}", info.asset)) {
            // Up in the picture is down in map space.
            quad(ctx, at, [-1.0, -1.0], [1.0, 1.0], 0.0, Sprite { v0: s.v1, v1: s.v0, ..s }, bright);
        }
        if let Some(name) = &d.name {
            names.push((d.x, d.y, name.spans(ctx.lang)));
        }
    }
    for (x, y, spans) in names {
        let width = ctx.bodies.font.width(&spans);
        let k = (25.0 / width.max(1.0)).clamp(0.0, 6.0 / 9.0);
        let at = m * Mat4::from_translation(Vec3::new(x as f32 / 2.0 + 64.0 - width * k / 2.0, y as f32 / 2.0 + 64.0 + 4.0, -0.75)) * Mat4::from_scale(Vec3::new(k, k, 1.0));
        super::text::draw_line(ctx, &spans, 0.0, 0.0, at, super::text::TextStyle { color: [255, 255, 255], alpha: 255, light: bright, pass: Pass::Cutout });
    }
}
