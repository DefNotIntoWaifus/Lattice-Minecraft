//! Player looks: the bundled default for a player's id until their own skin has been fetched.
//! Skins come only from the texture host the game's own authentication service names.

use crate::bodies::mobs::default_skin;
use base64::Engine as _;
use crossbeam_channel::{Receiver, Sender, unbounded};
use image::RgbaImage;
use lattice_render::atlas::{Sheet, Sprite};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Only these hosts are trusted to serve skins, as with the official client.
const ALLOWED_HOSTS: [&str; 2] = [".minecraft.net", ".mojang.com"];

struct Fetched {
    uuid: Uuid,
    skin: RgbaImage,
    slim: bool,
}

#[derive(Debug, Clone, Copy)]
struct Own {
    slot: usize,
    slim: bool,
}

pub struct SkinStore {
    own: HashMap<Uuid, Own>,
    asked: HashSet<Uuid>,
    next_slot: usize,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
    /// Which sheet slot holds each player's cape.
    capes: HashMap<Uuid, usize>,
    /// Each fetched player's face, with the hat layer over it.
    faces: HashMap<Uuid, RgbaImage>,
    /// What's in each sheet slot, in case the sheet has to be rebuilt.
    kept: HashMap<usize, RgbaImage>,
    cape_tx: Sender<(Uuid, RgbaImage)>,
    cape_rx: Receiver<(Uuid, RgbaImage)>,
}

impl Default for SkinStore {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        let (cape_tx, cape_rx) = unbounded();
        SkinStore { own: HashMap::new(), asked: HashSet::new(), next_slot: 0, tx, rx, capes: HashMap::new(), faces: HashMap::new(), kept: HashMap::new(), cape_tx, cape_rx }
    }
}

/// Reads the skin address and arm shape out of a profile's `textures` property.
pub fn parse_textures(value: &str) -> Option<(String, bool)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(value.trim()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let skin = v.get("textures")?.get("SKIN")?;
    let url = skin.get("url")?.as_str()?.to_string();
    let slim = skin.get("metadata").and_then(|m| m.get("model")).and_then(Value::as_str) == Some("slim");
    Some((url, slim))
}

/// Where a player's cape is, if they've got one.
pub fn parse_cape(value: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(value.trim()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("textures")?.get("CAPE")?.get("url")?.as_str().map(str::to_string)
}

fn fetch_cape(url: &str) -> Result<RgbaImage, String> {
    let mut resp = ureq::get(url).header("User-Agent", concat!("lattice/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| e.to_string())?;
    let bytes = resp.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?.to_rgba8();
    // A cape is 64×32, and sits in the top of a skin's space on the sheet.
    if img.width() == 64 && img.height() >= 32 { Ok(image::imageops::crop_imm(&img, 0, 0, 64, 32).to_image()) } else { Err("not a cape-sized image".to_string()) }
}

fn host_allowed(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let Some(rest) = rest else { return false };
    let host = rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("").to_ascii_lowercase();
    ALLOWED_HOSTS.iter().any(|h| host.ends_with(h))
}

/// Old skins are 64×32 with no left limbs or outer layers: the game mirrors the right ones over.
fn normalise(img: RgbaImage) -> Option<RgbaImage> {
    match (img.width(), img.height()) {
        (64, 64) => Some(img),
        (64, 32) => {
            let mut out = RgbaImage::new(64, 64);
            image::imageops::replace(&mut out, &img, 0, 0);
            let mut copy = |sx: u32, sy: u32, w: u32, h: u32, dx: i32, dy: i32| {
                for y in 0..h {
                    for x in 0..w {
                        let p = *out.get_pixel(sx + x, sy + y);
                        // Mirrored left to right, as the game's skin processing does.
                        out.put_pixel(((sx + (w - 1 - x)) as i32 + dx) as u32, ((sy + y) as i32 + dy) as u32, p);
                    }
                }
            };
            copy(4, 16, 4, 4, 16, 32);
            copy(8, 16, 4, 4, 16, 32);
            copy(0, 20, 4, 12, 24, 32);
            copy(4, 20, 4, 12, 16, 32);
            copy(8, 20, 4, 12, 8, 32);
            copy(12, 20, 4, 12, 16, 32);
            copy(44, 16, 4, 4, -8, 32);
            copy(48, 16, 4, 4, -8, 32);
            copy(40, 20, 4, 12, 0, 32);
            copy(44, 20, 4, 12, -8, 32);
            copy(48, 20, 4, 12, -16, 32);
            copy(52, 20, 4, 12, -8, 32);
            Some(out)
        }
        _ => None,
    }
}

fn fetch(url: &str) -> Result<RgbaImage, String> {
    let mut resp = ureq::get(url).header("User-Agent", concat!("lattice/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| e.to_string())?;
    let bytes = resp.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    normalise(img.to_rgba8()).ok_or_else(|| "not a skin-sized image".to_string())
}

/// The 8×8 face of a 64×64 skin, with the hat layer over it.
pub fn face_from_skin(skin: &RgbaImage) -> RgbaImage {
    let mut face = image::imageops::crop_imm(skin, 8, 8, 8, 8).to_image();
    let hat = image::imageops::crop_imm(skin, 40, 8, 8, 8).to_image();
    image::imageops::overlay(&mut face, &hat, 0, 0);
    face
}

impl SkinStore {
    /// Starts fetching a player's skin, once.
    pub fn request(&mut self, uuid: Uuid, textures: &str) {
        if !self.asked.insert(uuid) {
            return;
        }
        let Some((url, slim)) = parse_textures(textures) else { return };
        if !host_allowed(&url) {
            tracing::warn!("ignoring a skin from an unexpected host");
            return;
        }
        if let Some(cape) = parse_cape(textures).filter(|u| host_allowed(u)) {
            let tx = self.cape_tx.clone();
            let spawned = std::thread::Builder::new().name("cape".into()).spawn(move || match fetch_cape(&cape) {
                Ok(img) => {
                    let _ = tx.send((uuid, img));
                }
                Err(e) => tracing::info!("couldn't fetch a cape: {e}"),
            });
            if let Err(e) = spawned {
                tracing::warn!("couldn't start fetching a cape: {e}");
            }
        }
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("skin".into()).spawn(move || match fetch(&url) {
            Ok(skin) => {
                let _ = tx.send(Fetched { uuid, skin, slim });
            }
            Err(e) => tracing::info!("couldn't fetch a skin: {e}"),
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start fetching a skin: {e}");
        }
    }

    /// Moves skins that have arrived onto the sheet.
    pub fn pump(&mut self, sheet: &Sheet, renderer: &mut lattice_render::renderer::Renderer) {
        while let Ok((uuid, img)) = self.cape_rx.try_recv() {
            if self.next_slot < sheet.skin_slots.len() {
                let (x, y) = sheet.skin_slots[self.next_slot];
                renderer.write_sheet(x, y, &img);
                self.capes.insert(uuid, self.next_slot);
                self.kept.insert(self.next_slot, img);
                self.next_slot += 1;
            }
        }
        while let Ok(f) = self.rx.try_recv() {
            let slot = match self.own.get(&f.uuid) {
                Some(o) => o.slot,
                None => {
                    if self.next_slot >= sheet.skin_slots.len() {
                        continue;
                    }
                    self.next_slot += 1;
                    self.next_slot - 1
                }
            };
            let (x, y) = sheet.skin_slots[slot];
            renderer.write_sheet(x, y, &f.skin);
            self.faces.insert(f.uuid, face_from_skin(&f.skin));
            self.kept.insert(slot, f.skin.clone());
            self.own.insert(f.uuid, Own { slot, slim: f.slim });
        }
    }

    /// Puts every player's look back on a sheet that's just been replaced.
    pub fn restore(&self, sheet: &Sheet, renderer: &mut lattice_render::renderer::Renderer) {
        for (slot, img) in &self.kept {
            if let Some((x, y)) = sheet.skin_slots.get(*slot) {
                renderer.write_sheet(*x, *y, img);
            }
        }
    }

    /// A player's own face, once their skin has arrived.
    pub fn face_of(&self, uuid: Uuid) -> Option<&RgbaImage> {
        self.faces.get(&uuid)
    }

    /// A player's cape, if they've got one.
    pub fn cape_of(&self, uuid: Uuid, sheet: &Sheet) -> Option<Sprite> {
        let slot = self.capes.get(&uuid).copied()?;
        let s = sheet.slot_sprite(slot)?;
        // Only the top half of the space is used.
        Some(Sprite { v1: s.v0 + (s.v1 - s.v0) * 0.5, ..s })
    }

    /// Gives everyone this cape, for trying out the drawing with no real ones to hand.
    pub fn set_test_cape(&mut self, uuid: Uuid, img: &RgbaImage, sheet: &Sheet, renderer: &mut lattice_render::renderer::Renderer) {
        if self.next_slot < sheet.skin_slots.len() {
            let (x, y) = sheet.skin_slots[self.next_slot];
            renderer.write_sheet(x, y, img);
            self.capes.insert(uuid, self.next_slot);
            self.next_slot += 1;
        }
    }

    /// A fetched player's whole skin, and whether their arms are slim.
    pub fn skin_image(&self, uuid: Uuid) -> Option<(&RgbaImage, bool)> {
        let own = self.own.get(&uuid)?;
        Some((self.kept.get(&own.slot)?, own.slim))
    }

    /// The sprite to draw a player with, and whether their arms are slim.
    pub fn look_of(&self, uuid: Uuid, sheet: &Sheet) -> (Sprite, bool) {
        if let Some(o) = self.own.get(&uuid) {
            if let Some(s) = sheet.slot_sprite(o.slot) {
                return (s, o.slim);
            }
        }
        let (name, slim) = default_skin(uuid);
        (sheet.try_sprite(&name).unwrap_or(sheet.missing), slim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(host_allowed("https://textures.minecraft.net/texture/abc"));
        assert!(host_allowed("http://textures.minecraft.net/texture/abc"));
        assert!(!host_allowed("https://minecraft.net.evil.example/x"));
        assert!(!host_allowed("file:///etc/passwd"));
        assert!(!host_allowed("https://example.com/.minecraft.net"));
    }

    #[test]
    fn texture_property() {
        let json = r#"{"textures":{"SKIN":{"url":"https://textures.minecraft.net/texture/x","metadata":{"model":"slim"}}}}"#;
        let v = base64::engine::general_purpose::STANDARD.encode(json);
        assert_eq!(parse_textures(&v), Some(("https://textures.minecraft.net/texture/x".to_string(), true)));
    }

    #[test]
    fn default_look_follows_the_id() {
        // The classic Steve/Alex test ids keep landing on one of the bundled looks.
        let (name, _) = default_skin(Uuid::from_u128(0x853c80ef3c3749fdaa49938b674adae6));
        assert!(name.starts_with("entity/player/"));
    }

    #[test]
    fn old_skins_get_left_limbs() {
        let mut img = RgbaImage::new(64, 32);
        img.put_pixel(4, 20, image::Rgba([1, 2, 3, 255]));
        let out = normalise(img).unwrap();
        assert_eq!((out.width(), out.height()), (64, 64));
        assert_eq!(out.get_pixel(23, 52)[0], 1);
        assert!(normalise(RgbaImage::new(32, 32)).is_none());
    }
}
