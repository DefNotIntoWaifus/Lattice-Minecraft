//! Game resources, loaded in the background by each game window.

use crate::ui::widgets::format_bytes;
use crossbeam_channel::{Receiver, unbounded};
use lattice_proto::Translator;
use lattice_render::mesher::MeshContext;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct Lang(pub HashMap<String, String>);

impl Translator for Lang {
    fn translate(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

pub struct GameAssets {
    pub lang: Lang,
    /// What's read at the end of the game, from the game's own files.
    pub end_poem: String,
    pub mesh: MeshContext,
    pub atlas: Option<lattice_render::atlas::Atlas>,
    pub items: lattice_render::items::ItemVisuals,
    pub bodies: crate::bodies::Bodies,
    pub flames: [lattice_render::atlas::Sprite; 2],
    /// Flat item pictures in the block atlas, by item name, for bits of broken items.
    pub atlas_item: HashMap<String, lattice_render::atlas::Sprite>,
    /// Pictures of each particle kind.
    pub particle_sets: Vec<(String, Vec<lattice_render::atlas::Sprite>)>,
    pub cracks: [lattice_render::atlas::Sprite; 10],
    /// Rain and snow pictures, handed to the renderer once.
    pub weather_images: Option<(image::RgbaImage, image::RgbaImage)>,
    /// Handed to the renderer once, then dropped.
    pub sheet_image: Option<image::RgbaImage>,
    /// The flag part of each banner pattern, as white on clear.
    pub banner_masks: HashMap<String, image::RgbaImage>,
    /// The shine on enchanted items and armour, for the renderer.
    pub glint_images: Option<(image::RgbaImage, image::RgbaImage)>,
    /// The End portal's stars, and the swirls on charged creepers and withers.
    pub effect_images: Option<(image::RgbaImage, image::RgbaImage, image::RgbaImage)>,
    /// How much wider the block atlas is than usual, which the shine's scale depends on.
    pub atlas_scale: f32,
    /// Pictures laid over the whole screen.
    pub screen_pictures: HashMap<String, image::RgbaImage>,
}

#[derive(Clone, Default)]
pub struct LoadProgress {
    pub step: String,
    pub fraction: Option<f32>,
}

pub struct AssetLoader {
    pub progress: Arc<Mutex<LoadProgress>>,
    pub result: Receiver<Result<GameAssets, String>>,
}

pub enum AssetState {
    Loading(AssetLoader),
    Ready(Arc<GameAssets>),
    Failed(String),
}

/// Makes sure the verified client jar is in the cache, reporting download progress.
pub fn ensure_files(cache: &std::path::Path, progress: &Mutex<LoadProgress>) -> Result<PathBuf, String> {
    lattice_assets::ensure_client_jar(cache, &|p| {
        let total = p.total.unwrap_or(0);
        let mut g = progress.lock().unwrap();
        g.step = format!("Downloading game files from Mojang ({} of {})", format_bytes(p.done), format_bytes(total));
        g.fraction = (total > 0).then(|| p.done as f32 / total as f32);
    })
    .map_err(|e| e.to_string())
}

fn load(cache: PathBuf, locale: String, packs: Vec<PathBuf>, progress: &Mutex<LoadProgress>) -> Result<GameAssets, String> {
    let started = std::time::Instant::now();
    let set = |step: &str| {
        let mut p = progress.lock().unwrap();
        p.step = step.to_string();
        p.fraction = None;
    };
    set("Checking game files");
    let jar = ensure_files(&cache, progress)?;
    set("Reading textures");
    let res = lattice_assets::Resources::open_with_packs(&jar, &packs).map_err(|e| e.to_string())?;
    let item_defs = lattice_render::items::load_definitions(&res);
    let mut names = lattice_render::models::collect_textures(&res);
    names.extend(lattice_render::items::collect_item_textures(&res, &item_defs));
    names.sort();
    names.dedup();
    let textures: Vec<_> = names.iter().filter_map(|n| lattice_render::items::load_sprite_or_trim(&res, n).map(|i| (n.clone(), i))).collect();
    let mut moving = HashMap::new();
    for n in &names {
        if let Some(info) = res.json(&format!("textures/{n}.png.mcmeta")).and_then(|v| lattice_render::atlas::AnimationInfo::parse(&v)) {
            moving.insert(n.clone(), info);
        }
    }
    set("Building the texture atlas");
    let atlas = lattice_render::atlas::Atlas::build_animated(textures, &moving);
    set("Preparing block models");
    let data = lattice_data::game();
    let models = lattice_render::models::BlockModels::load(&res, data, &atlas);
    if models.missing_models > 0 {
        tracing::warn!("{} blocks have no blockstate file", models.missing_models);
    }
    let mesh = MeshContext::new(data, models, &res, &atlas);
    let lang = Lang(res.translations(&locale));
    let end_poem = res.read("assets/minecraft/texts/end.txt").map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    set("Reading mob shapes");
    let mut bodies = crate::bodies::Bodies::load(&jar, &res);
    let sheet_image = std::mem::replace(&mut bodies.sheet.image, image::RgbaImage::new(1, 1));
    let mut banner_masks = HashMap::new();
    for (name, sprite) in &bodies.sheet.sprites {
        let Some(pattern) = name.strip_prefix("entity/banner/") else { continue };
        let (w, h) = (sheet_image.width() as f32, sheet_image.height() as f32);
        let k = (sprite.u1 - sprite.u0) * w / 64.0;
        // The front of the flag, in the shape's own layout.
        let (x, y) = ((sprite.u0 * w + k) as u32, (sprite.v0 * h + k) as u32);
        let (cw, ch) = ((20.0 * k) as u32, (40.0 * k) as u32);
        banner_masks.insert(pattern.to_string(), image::imageops::crop_imm(&sheet_image, x, y, cw, ch).to_image());
    }
    let banner_pattern_names: Vec<String> = banner_masks.keys().filter(|p| !matches!(p.as_str(), "base" | "banner_base")).cloned().collect();
    set("Drawing item icons");
    let special = |kind: &str, texture: Option<&str>, item: &str| crate::bodies::blockent::special_icon(&bodies, &mesh.models, kind, texture, item, &sheet_image, &atlas.mips[0]);
    let items = lattice_render::items::ItemVisuals::load(&res, item_defs, &atlas, &special, &banner_pattern_names);
    let sheet_image = Some(sheet_image);
    let glint_images = res.image("textures/misc/enchanted_glint_item.png").zip(res.image("textures/misc/enchanted_glint_armor.png"));
    let effect_images = res.image("textures/entity/end_portal/end_portal.png").zip(res.image("textures/entity/creeper/creeper_armor.png")).zip(res.image("textures/entity/wither/wither_armor.png")).map(|((a, b), c)| (a, b, c));
    let atlas_scale = atlas.size as f32 / 1024.0;
    let mut screen_pictures = HashMap::new();
    // The faces of the bundled looks, for the player list.
    for path in res.list("textures/entity/player/", ".png") {
        if let Some(img) = res.image(&path) {
            if img.width() >= 64 && img.height() >= 64 {
                let name = path.trim_start_matches("textures/").trim_end_matches(".png");
                screen_pictures.insert(format!("face/{name}"), crate::skins::face_from_skin(&img));
                screen_pictures.insert(format!("skin/{name}"), img);
            }
        }
    }
    for name in ["misc/vignette", "misc/pumpkinblur", "misc/spyglass_scope", "misc/powder_snow_outline", "misc/nausea"] {
        if let Some(mut img) = res.image(&format!("textures/{name}.png")) {
            // The game multiplies what's on screen by the inverse of this; dark and see-through
            // comes to the same with ordinary blending.
            if name == "misc/vignette" {
                for p in img.pixels_mut() {
                    *p = image::Rgba([0, 0, 0, p[0]]);
                }
            }
            screen_pictures.insert(name.to_string(), img);
        }
    }
    // Armour, for the character in the inventory.
    for dir in ["humanoid", "humanoid_leggings"] {
        for path in res.list(&format!("textures/entity/equipment/{dir}/"), ".png") {
            if let Some(img) = res.image(&path) {
                screen_pictures.insert(path.trim_start_matches("textures/").trim_end_matches(".png").to_string(), img);
            }
        }
    }
    for path in res.list("textures/mob_effect/", ".png") {
        if let Some(img) = res.image(&path) {
            let name = path.trim_start_matches("textures/").trim_end_matches(".png").to_string();
            screen_pictures.insert(name, img);
        }
    }
    if let Some(img) = res.image("textures/block/nether_portal.png") {
        screen_pictures.insert("block/nether_portal".to_string(), img);
    }
    tracing::info!("game files ready in {:.2} s", started.elapsed().as_secs_f32());
    let flames = [atlas.sprite("block/fire_0"), atlas.sprite("block/fire_1")];
    let cracks = std::array::from_fn(|i| atlas.sprite(&format!("block/destroy_stage_{i}")));
    let weather_images = bodies.weather.take();
    let mut atlas_item = HashMap::new();
    for item in &lattice_data::game().items {
        if let Some(s) = atlas.sprites.get(&format!("item/{}", item.name)).or_else(|| atlas.sprites.get(&format!("block/{}", item.name))) {
            atlas_item.insert(item.name.clone(), *s);
        }
    }
    let mut particle_sets = Vec::new();
    for path in res.list("particles/", ".json") {
        let name = path.trim_start_matches("particles/").trim_end_matches(".json").to_string();
        let Some(v) = res.json(&path) else { continue };
        let list: Vec<_> = v
            .get("textures")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str())
            .filter_map(|t| bodies.sheet.try_sprite(&format!("particle/{}", t.trim_start_matches("minecraft:"))))
            .collect();
        particle_sets.push((name, list));
    }
    Ok(GameAssets { lang, end_poem, mesh, atlas: Some(atlas), items, bodies, flames, atlas_item, particle_sets, cracks, weather_images, sheet_image, banner_masks, screen_pictures, glint_images, effect_images, atlas_scale })
}

pub fn start_loading(cache: PathBuf, locale: String, packs: Vec<PathBuf>) -> AssetLoader {
    let progress = Arc::new(Mutex::new(LoadProgress::default()));
    let (tx, result) = unbounded();
    let p = progress.clone();
    std::thread::Builder::new()
        .name("assets".into())
        .spawn(move || {
            let _ = tx.send(load(cache, locale, packs, &p));
        })
        .expect("spawn asset thread");
    AssetLoader { progress, result }
}
