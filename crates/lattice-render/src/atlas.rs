//! Packs block textures into one mip-mapped atlas.

use image::RgbaImage;
use std::collections::HashMap;

pub const TILE: u32 = 16;
pub const MIP_LEVELS: u32 = 5;

#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    /// Normalised atlas coordinates.
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    /// 0 opaque, 1 has fully transparent pixels, 2 has partial alpha.
    pub alpha: u8,
}

impl Sprite {
    pub fn lerp(&self, u: f32, v: f32) -> [f32; 2] {
        [self.u0 + (self.u1 - self.u0) * u, self.v0 + (self.v1 - self.v0) * v]
    }
}

pub struct Atlas {
    pub size: u32,
    /// Level 0 first.
    pub mips: Vec<RgbaImage>,
    pub sprites: HashMap<String, Sprite>,
    pub missing: Sprite,
    pub animations: Vec<SpriteAnimation>,
}

/// How a texture's frames play, from its `.mcmeta` file.
#[derive(Debug, Clone, Default)]
pub struct AnimationInfo {
    pub frametime: u32,
    pub interpolate: bool,
    /// Frame index and its own time; empty means each frame in order.
    pub frames: Vec<(u32, Option<u32>)>,
    /// Size of a frame, when the file says it isn't square.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl AnimationInfo {
    pub fn parse(v: &serde_json::Value) -> Option<AnimationInfo> {
        let a = v.get("animation")?;
        let frames = a
            .get("frames")
            .and_then(|f| f.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|e| match e {
                        serde_json::Value::Number(n) => n.as_u64().map(|i| (i as u32, None)),
                        o => Some((o.get("index")?.as_u64()? as u32, o.get("time").and_then(|t| t.as_u64()).map(|t| t as u32))),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(AnimationInfo {
            frametime: a.get("frametime").and_then(|f| f.as_u64()).unwrap_or(1) as u32,
            interpolate: a.get("interpolate").and_then(|f| f.as_bool()).unwrap_or(false),
            frames,
            width: a.get("width").and_then(|f| f.as_u64()).map(|f| f as u32),
            height: a.get("height").and_then(|f| f.as_u64()).map(|f| f as u32),
        })
    }
}

/// One moving texture and where it sits in the atlas.
pub struct SpriteAnimation {
    pub x: u32,
    pub y: u32,
    /// Each frame's picture, with its smaller mip levels.
    frames: Vec<Vec<RgbaImage>>,
    /// Which frame for how many ticks, in order.
    order: Vec<(u32, u32)>,
    interpolate: bool,
    at: usize,
    ticks: u32,
    shown: Option<usize>,
}

impl SpriteAnimation {
    /// Advances one game tick. Returns pictures to write, by mip level, when something changed.
    pub fn tick(&mut self) -> Option<Vec<RgbaImage>> {
        self.ticks += 1;
        let (frame, duration) = self.order[self.at];
        if self.ticks >= duration {
            self.ticks = 0;
            self.at = (self.at + 1) % self.order.len();
            let (frame, _) = self.order[self.at];
            if !self.interpolate {
                if self.shown == Some(frame as usize) {
                    return None;
                }
                self.shown = Some(frame as usize);
                return self.frames.get(frame as usize).cloned();
            }
        } else if !self.interpolate {
            return None;
        }
        if !self.interpolate {
            return None;
        }
        // Blend towards the next frame, recomputing the smaller levels from the result.
        let (frame_now, duration) = if self.ticks == 0 { self.order[self.at] } else { (frame, duration) };
        let next = self.order[(self.at + 1) % self.order.len()].0;
        let t = self.ticks as f32 / duration.max(1) as f32;
        let (a, b) = (self.frames.get(frame_now as usize)?, self.frames.get(next as usize)?);
        let mut base = a[0].clone();
        for (p, q) in base.pixels_mut().zip(b[0].pixels()) {
            for c in 0..3 {
                p[c] = (p[c] as f32 * (1.0 - t) + q[c] as f32 * t) as u8;
            }
        }
        let mut levels = vec![base];
        for _ in 1..a.len() {
            levels.push(downsample(levels.last().unwrap()));
        }
        Some(levels)
    }
}

fn alpha_class(img: &RgbaImage) -> u8 {
    let mut class = 0;
    for p in img.pixels() {
        match p[3] {
            255 => {}
            0 => class = class.max(1),
            _ => return 2,
        }
    }
    class
}

/// A magenta/black checkerboard so missing textures are obvious.
fn missing_texture() -> RgbaImage {
    RgbaImage::from_fn(TILE, TILE, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 { image::Rgba([248, 0, 248, 255]) } else { image::Rgba([0, 0, 0, 255]) }
    })
}

impl Atlas {
    /// `textures` maps names to images; animated strips are cropped to their first frame.
    pub fn build(textures: Vec<(String, RgbaImage)>) -> Atlas {
        Self::build_animated(textures, &HashMap::new())
    }

    /// Like `build`, keeping the frames of whatever's in `animated` so they can play.
    pub fn build_animated(textures: Vec<(String, RgbaImage)>, animated: &HashMap<String, AnimationInfo>) -> Atlas {
        // Name to the frames and timing of each moving texture.
        let mut moving: HashMap<String, (Vec<RgbaImage>, Vec<(u32, u32)>, bool)> = HashMap::new();
        let mut items: Vec<(String, RgbaImage)> = textures
            .into_iter()
            .map(|(name, img)| {
                let w = img.width();
                let info = animated.get(&name);
                let (fw, fh) = match info {
                    Some(a) => (a.width.unwrap_or(w), a.height.unwrap_or(w)),
                    None => (w, w),
                };
                if let Some(a) = info {
                    let count = (img.height() / fh.max(1)).max(1) * (img.width() / fw.max(1)).max(1);
                    if count > 1 {
                        let frames: Vec<RgbaImage> = (0..count).map(|i| image::imageops::crop_imm(&img, 0, i * fh, fw, fh).to_image()).collect();
                        let order: Vec<(u32, u32)> = if a.frames.is_empty() {
                            (0..count).map(|i| (i, a.frametime.max(1))).collect()
                        } else {
                            a.frames.iter().map(|(i, t)| (*i, t.unwrap_or(a.frametime).max(1))).collect()
                        };
                        moving.insert(name.clone(), (frames, order, a.interpolate));
                    }
                }
                let img = if img.height() > fh { image::imageops::crop_imm(&img, 0, 0, fw.min(w), fh).to_image() } else { img };
                (name, img)
            })
            .collect();
        items.push(("\u{0}missing".into(), missing_texture()));
        // Larger first; all sizes are multiples of 16 in practice, round up otherwise.
        items.sort_by_key(|(_, i)| std::cmp::Reverse((i.height(), i.width())));
        let round = |v: u32| v.div_ceil(TILE) * TILE;
        let area: u64 = items.iter().map(|(_, i)| (round(i.width()) * round(i.height())) as u64).sum();
        let mut size = 256u32;
        while (size as u64 * size as u64) < area * 5 / 4 {
            size *= 2;
        }
        loop {
            if let Some(placed) = Self::pack(&items, size) {
                return Self::compose(items, placed, size, moving);
            }
            size *= 2;
        }
    }

    /// Shelf packing on a 16-pixel grid so mip levels never mix neighbours.
    fn pack(items: &[(String, RgbaImage)], size: u32) -> Option<Vec<(u32, u32)>> {
        let round = |v: u32| v.div_ceil(TILE) * TILE;
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        let mut out = Vec::with_capacity(items.len());
        for (_, img) in items {
            let (w, h) = (round(img.width()), round(img.height()));
            if x + w > size {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            if y + h > size || w > size {
                return None;
            }
            out.push((x, y));
            x += w;
            shelf = shelf.max(h);
        }
        Some(out)
    }

    fn compose(items: Vec<(String, RgbaImage)>, placed: Vec<(u32, u32)>, size: u32, mut moving: HashMap<String, (Vec<RgbaImage>, Vec<(u32, u32)>, bool)>) -> Atlas {
        let mut base = RgbaImage::new(size, size);
        let mut sprites = HashMap::with_capacity(items.len());
        let s = size as f32;
        let mut animations = Vec::new();
        for ((name, img), (x, y)) in items.into_iter().zip(placed) {
            image::imageops::replace(&mut base, &img, x as i64, y as i64);
            if let Some((frames, order, interpolate)) = moving.remove(&name) {
                let frames = frames
                    .into_iter()
                    .map(|f| {
                        let mut levels = vec![f];
                        for _ in 1..MIP_LEVELS {
                            levels.push(downsample(levels.last().unwrap()));
                        }
                        levels
                    })
                    .collect();
                let shown = order.first().map(|(f, _)| *f as usize);
                animations.push(SpriteAnimation { x, y, frames, order, interpolate, at: 0, ticks: 0, shown });
            }
            sprites.insert(
                name,
                Sprite {
                    u0: x as f32 / s,
                    v0: y as f32 / s,
                    u1: (x + img.width()) as f32 / s,
                    v1: (y + img.height()) as f32 / s,
                    alpha: alpha_class(&img),
                },
            );
        }
        let missing = sprites.remove("\u{0}missing").unwrap();
        let mut mips = vec![base];
        for _ in 1..MIP_LEVELS {
            let prev = mips.last().unwrap();
            mips.push(downsample(prev));
        }
        Atlas { size, mips, sprites, missing, animations }
    }

    pub fn sprite(&self, name: &str) -> Sprite {
        let key = name.trim_start_matches("minecraft:");
        *self.sprites.get(key).unwrap_or(&self.missing)
    }
}

/// One unfiltered sheet of textures that don't share the 16-pixel grid of blocks: mobs,
/// block entities, armour, paintings. Part of it is kept free for player skins, which arrive
/// while playing.
pub struct Sheet {
    pub width: u32,
    pub height: u32,
    pub image: RgbaImage,
    pub sprites: HashMap<String, Sprite>,
    pub missing: Sprite,
    /// A little plain white, for coloured rectangles.
    pub white: Sprite,
    /// Top-left corners of the 64×64 areas held back for skins.
    pub skin_slots: Vec<(u32, u32)>,
    /// Top-left corners of the 128×128 areas held back for maps.
    pub map_slots: Vec<(u32, u32)>,
}

impl Sheet {
    pub fn build(textures: Vec<(String, RgbaImage)>, skin_slots: u32, map_slots: u32) -> Sheet {
        const WIDTH: u32 = 4096;
        const GAP: u32 = 2;
        let mut items = textures;
        items.push(("\u{0}missing".into(), missing_texture()));
        items.push(("\u{0}white".into(), RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]))));
        items.sort_by_key(|(n, i)| (std::cmp::Reverse(i.height()), n.clone()));
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        let mut placed = Vec::with_capacity(items.len());
        for (_, img) in &items {
            if x + img.width() > WIDTH {
                x = 0;
                y += shelf + GAP;
                shelf = 0;
            }
            placed.push((x, y));
            x += img.width() + GAP;
            shelf = shelf.max(img.height());
        }
        let mut slots = Vec::with_capacity(skin_slots as usize);
        y += shelf + GAP;
        let (mut sx, mut sy) = (0u32, y);
        for _ in 0..skin_slots {
            if sx + 64 > WIDTH {
                sx = 0;
                sy += 64 + GAP;
            }
            slots.push((sx, sy));
            sx += 64 + GAP;
        }
        // Maps go after the skins, and only change as they're explored.
        sy += 64 + GAP;
        sx = 0;
        let mut maps = Vec::with_capacity(map_slots as usize);
        for _ in 0..map_slots {
            if sx + 128 > WIDTH {
                sx = 0;
                sy += 128 + GAP;
            }
            maps.push((sx, sy));
            sx += 128 + GAP;
        }
        let height = (sy + 128 + GAP).next_multiple_of(64).max(64);
        let mut image = RgbaImage::new(WIDTH, height);
        let (w, h) = (WIDTH as f32, height as f32);
        let mut sprites = HashMap::with_capacity(items.len());
        for ((name, img), (px, py)) in items.into_iter().zip(placed) {
            image::imageops::replace(&mut image, &img, px as i64, py as i64);
            sprites.insert(
                name,
                Sprite { u0: px as f32 / w, v0: py as f32 / h, u1: (px + img.width()) as f32 / w, v1: (py + img.height()) as f32 / h, alpha: alpha_class(&img) },
            );
        }
        let missing = sprites.remove("\u{0}missing").unwrap();
        let white = sprites.remove("\u{0}white").unwrap();
        Sheet { width: WIDTH, height, image, sprites, missing, white, skin_slots: slots, map_slots: maps }
    }

    pub fn sprite(&self, name: &str) -> Sprite {
        *self.sprites.get(name.trim_start_matches("minecraft:")).unwrap_or(&self.missing)
    }

    pub fn try_sprite(&self, name: &str) -> Option<Sprite> {
        self.sprites.get(name.trim_start_matches("minecraft:")).copied()
    }

    /// Where a map slot sits, for drawing.
    pub fn map_sprite(&self, slot: usize) -> Option<Sprite> {
        let (x, y) = *self.map_slots.get(slot)?;
        let (w, h) = (self.width as f32, self.height as f32);
        Some(Sprite { u0: x as f32 / w, v0: y as f32 / h, u1: (x + 128) as f32 / w, v1: (y + 128) as f32 / h, alpha: 1 })
    }

    /// Where a skin slot sits, for drawing.
    pub fn slot_sprite(&self, slot: usize) -> Option<Sprite> {
        let (x, y) = *self.skin_slots.get(slot)?;
        let (w, h) = (self.width as f32, self.height as f32);
        Some(Sprite { u0: x as f32 / w, v0: y as f32 / h, u1: (x + 64) as f32 / w, v1: (y + 64) as f32 / h, alpha: 1 })
    }
}

/// 2×2 box filter that weights colour by alpha, so cut-out edges don't darken.
fn downsample(src: &RgbaImage) -> RgbaImage {
    let (w, h) = (src.width() / 2, src.height() / 2);
    RgbaImage::from_fn(w.max(1), h.max(1), |x, y| {
        let mut acc = [0u32; 3];
        let (mut a_sum, mut a_max, mut n) = (0u32, 0u8, 0u32);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let p = src.get_pixel((2 * x + dx).min(src.width() - 1), (2 * y + dy).min(src.height() - 1));
            let a = p[3] as u32;
            for c in 0..3 {
                acc[c] += p[c] as u32 * a;
            }
            a_sum += a;
            a_max = a_max.max(p[3]);
            n += 1;
        }
        if a_sum == 0 {
            return image::Rgba([0, 0, 0, 0]);
        }
        // Keep cut-out textures (leaves, grass) from fading away at distance:
        // use the max alpha when any texel is fully opaque.
        let alpha = if a_max == 255 && a_sum >= 255 * 2 { 255 } else { (a_sum / n) as u8 };
        image::Rgba([(acc[0] / a_sum) as u8, (acc[1] / a_sum) as u8, (acc[2] / a_sum) as u8, alpha])
    })
}
