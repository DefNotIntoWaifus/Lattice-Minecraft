//! Writing in the world, with the game's own bitmap font: name tags, signs, floating text.

use super::{Ctx, Pass};
use glam::{Mat4, Vec3};
use lattice_assets::Resources;
use lattice_proto::text::Span;
use lattice_render::atlas::Sprite;
use lattice_render::mesher::Vertex;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// Where it is on its picture, in pixels.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// The picture's name on the sheet.
    pub sheet: u8,
    pub advance: f32,
    /// Pixels above the top of the line.
    pub top: f32,
    /// World pixels per picture pixel.
    pub scale: f32,
}

#[derive(Default)]
pub struct Font {
    glyphs: HashMap<char, Glyph>,
    pictures: Vec<(String, u32, u32)>,
}

pub const LINE_HEIGHT: f32 = 9.0;

impl Font {
    /// Reads the bitmap parts of the default font. Pictures are added to `textures` for the sheet.
    pub fn load(res: &Resources, textures: &mut Vec<(String, image::RgbaImage)>) -> Font {
        let mut font = Font::default();
        let Some(def) = res.json("font/include/default.json") else { return font };
        for provider in def.get("providers").and_then(Value::as_array).into_iter().flatten() {
            if provider.get("type").and_then(Value::as_str) != Some("bitmap") {
                continue;
            }
            let Some(file) = provider.get("file").and_then(Value::as_str) else { continue };
            let file = file.trim_start_matches("minecraft:").trim_end_matches(".png");
            let Some(img) = res.image(&format!("textures/{file}.png")) else { continue };
            let Some(rows) = provider.get("chars").and_then(Value::as_array) else { continue };
            let ascent = provider.get("ascent").and_then(Value::as_i64).unwrap_or(7) as f32;
            let height = provider.get("height").and_then(Value::as_i64).unwrap_or(8) as f32;
            let columns = rows.first().and_then(Value::as_str).map_or(1, |r| r.chars().count()) as u32;
            let (cell_w, cell_h) = (img.width() / columns.max(1), img.height() / rows.len().max(1) as u32);
            let scale = height / cell_h as f32;
            let sheet = font.pictures.len() as u8;
            font.pictures.push((file.to_string(), img.width(), img.height()));
            for (row, line) in rows.iter().enumerate() {
                for (col, ch) in line.as_str().unwrap_or("").chars().enumerate() {
                    if ch == '\0' || font.glyphs.contains_key(&ch) {
                        continue;
                    }
                    let (x, y) = (col as u32 * cell_w, row as u32 * cell_h);
                    // The width is up to the last column with anything in it.
                    let mut w = 0;
                    for px in (0..cell_w).rev() {
                        if (0..cell_h).any(|py| img.get_pixel(x + px, y + py)[3] != 0) {
                            w = px + 1;
                            break;
                        }
                    }
                    let advance = (0.5 + w as f32 * scale).floor() + 1.0;
                    font.glyphs.insert(ch, Glyph { x, y, w, h: cell_h, sheet, advance, top: 7.0 - ascent, scale });
                }
            }
            textures.push((file.to_string(), img));
        }
        font
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    fn advance(&self, c: char) -> f32 {
        if c == ' ' {
            return 4.0;
        }
        self.glyph(c).map_or(4.0, |g| g.advance)
    }

    pub fn width(&self, spans: &[Span]) -> f32 {
        spans
            .iter()
            .map(|s| s.text.chars().map(|c| self.advance(c) + if s.bold { 1.0 } else { 0.0 }).sum::<f32>())
            .sum()
    }
}

/// Default colour and how the text is laid down.
#[derive(Clone, Copy)]
pub struct TextStyle {
    pub color: [u8; 3],
    pub alpha: u8,
    pub light: [u8; 2],
    pub pass: Pass,
}

/// Writes a line starting at (x, y) in the plane of `m`, where one unit is a font pixel.
pub fn draw_line(ctx: &mut Ctx, spans: &[Span], mut x: f32, y: f32, m: Mat4, style: TextStyle) {
    let bodies = ctx.bodies;
    let font = &bodies.font;
    let light = [style.light[0], style.light[1], 0, 0];
    for span in spans {
        let color = span.color.map_or(style.color, |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        for c in span.text.chars() {
            if c == ' ' {
                x += 4.0 + if span.bold { 1.0 } else { 0.0 };
                continue;
            }
            let Some(g) = font.glyph(c) else { continue };
            let (name, pw, ph) = &font.pictures[g.sheet as usize];
            let Some(sheet) = bodies.sheet.try_sprite(name) else { continue };
            let (u0, v0) = (sheet.u0 + (sheet.u1 - sheet.u0) * g.x as f32 / *pw as f32, sheet.v0 + (sheet.v1 - sheet.v0) * g.y as f32 / *ph as f32);
            let (u1, v1) = (sheet.u0 + (sheet.u1 - sheet.u0) * (g.x + g.w) as f32 / *pw as f32, sheet.v0 + (sheet.v1 - sheet.v0) * (g.y + g.h) as f32 / *ph as f32);
            let (w, h) = (g.w as f32 * g.scale, g.h as f32 * g.scale);
            let top = y + g.top;
            let times = if span.bold { 2 } else { 1 };
            let rgba = [color[0], color[1], color[2], style.alpha];
            for i in 0..times {
                let dx = i as f32;
                let quad = [(x + dx, top, u0, v0), (x + dx, top + h, u0, v1), (x + dx + w, top + h, u1, v1), (x + dx + w, top, u1, v0)];
                let list = ctx.list(style.pass);
                for (px, py, u, v) in quad {
                    list.push(Vertex { pos: m.transform_point3(Vec3::new(px, py, 0.0)).to_array(), uv: [u, v], color: rgba, light });
                }
            }
            x += g.advance + if span.bold { 1.0 } else { 0.0 };
        }
    }
}

/// A plain coloured rectangle in the plane of `m`.
pub fn draw_rect(ctx: &mut Ctx, x0: f32, y0: f32, x1: f32, y1: f32, z: f32, m: Mat4, color: [u8; 4], light: [u8; 2], pass: Pass) {
    let s: Sprite = ctx.bodies.sheet.white;
    let quad = [(x0, y0), (x0, y1), (x1, y1), (x1, y0)];
    let list = ctx.list(pass);
    for (px, py) in quad {
        list.push(Vertex { pos: m.transform_point3(Vec3::new(px, py, z)).to_array(), uv: [(s.u0 + s.u1) * 0.5, (s.v0 + s.v1) * 0.5], color, light: [light[0], light[1], 0, 0] });
    }
}

/// A label floating over something, always facing the camera.
pub fn draw_name_tag(ctx: &mut Ctx, spans: &[Span], pos: Vec3, discrete: bool, light: [u8; 2], background: f32) {
    let m = Mat4::from_translation(pos) * Mat4::from_quat(ctx.camera_rot) * Mat4::from_scale(Vec3::new(0.025, -0.025, 0.025));
    let width = ctx.bodies.font.width(spans);
    let x = -width / 2.0;
    let back = [0, 0, 0, (background * 255.0) as u8];
    if !discrete {
        // Faintly through walls, then properly where nothing's in the way.
        draw_rect(ctx, x - 1.0, -1.0, x + width, 8.0, 0.0, m, back, light, Pass::Through);
        draw_line(ctx, spans, x, 0.0, m, TextStyle { color: [255, 255, 255], alpha: 32, light, pass: Pass::Through });
        draw_line(ctx, spans, x, 0.0, m, TextStyle { color: [255, 255, 255], alpha: 255, light, pass: Pass::Cutout });
    } else {
        draw_rect(ctx, x - 1.0, -1.0, x + width, 8.0, 0.0, m, back, light, Pass::Translucent);
        draw_line(ctx, spans, x, 0.0, m, TextStyle { color: [255, 255, 255], alpha: 32, light, pass: Pass::Translucent });
    }
}
