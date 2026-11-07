//! Synchronized registries sent by the server during configuration. The client
//! declares no known packs, so every entry arrives with its full NBT.

use lattice_proto::Nbt;
use lattice_proto::text::Style;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DimensionType {
    pub name: String,
    pub min_y: i32,
    pub height: i32,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    pub ambient_light: f32,
    pub sky_color: Option<u32>,
    pub fog_color: Option<u32>,
    pub cloud_height: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrassModifier {
    None,
    DarkForest,
    Swamp,
}

#[derive(Debug, Clone)]
pub struct Biome {
    pub name: String,
    pub temperature: f32,
    pub downfall: f32,
    pub has_precipitation: bool,
    pub water_color: u32,
    pub grass_color: Option<u32>,
    pub foliage_color: Option<u32>,
    pub grass_modifier: GrassModifier,
    pub sky_color: Option<u32>,
    pub fog_color: Option<u32>,
    pub water_fog_color: Option<u32>,
    pub sounds: BiomeSounds,
}

/// What a place sounds like.
#[derive(Debug, Clone, Default)]
pub struct BiomeSounds {
    pub ambient_loop: Option<String>,
    pub mood: Option<MoodSound>,
    /// An extra sound and its chance per tick.
    pub additions: Option<(String, f64)>,
    /// Event, shortest and longest wait after it, in ticks.
    pub music: Option<(String, i32, i32)>,
    pub music_creative: Option<(String, i32, i32)>,
    pub music_underwater: Option<(String, i32, i32)>,
    pub music_replaces: bool,
}

#[derive(Debug, Clone)]
pub struct MoodSound {
    pub sound: String,
    pub tick_delay: i32,
    pub offset: f64,
}

/// A sound named in registry data: a plain name, or a compound with a `sound_id`.
fn sound_name(tag: Option<&Nbt>) -> Option<String> {
    let t = tag?;
    let name = t.as_str().map(str::to_string).or_else(|| t.get("sound_id").and_then(Nbt::as_str).map(str::to_string))?;
    Some(name.trim_start_matches("minecraft:").to_string())
}

fn read_music(tag: Option<&Nbt>) -> Option<(String, i32, i32)> {
    let t = tag?;
    let sound = sound_name(t.get("sound"))?;
    let min = t.get("min_delay").and_then(Nbt::as_i64).unwrap_or(12000) as i32;
    let max = t.get("max_delay").and_then(Nbt::as_i64).unwrap_or(24000) as i32;
    Some((sound, min, max))
}

fn read_biome_sounds(t: &Nbt) -> BiomeSounds {
    let mut out = BiomeSounds::default();
    if let Some(a) = attr(t, "minecraft:audio/ambient_sounds") {
        out.ambient_loop = sound_name(a.get("loop"));
        if let Some(m) = a.get("mood") {
            if let Some(sound) = sound_name(m.get("sound")) {
                out.mood = Some(MoodSound {
                    sound,
                    tick_delay: m.get("tick_delay").and_then(Nbt::as_i64).unwrap_or(6000) as i32,
                    offset: m.get("offset").and_then(Nbt::as_f64).unwrap_or(2.0),
                });
            }
        }
        if let Some(list) = a.get("additions").and_then(Nbt::as_list) {
            if let Some(first) = list.first() {
                if let Some(sound) = sound_name(first.get("sound")) {
                    out.additions = Some((sound, first.get("tick_chance").and_then(Nbt::as_f64).unwrap_or(0.0)));
                }
            }
        }
    }
    if let Some(m) = attr(t, "minecraft:audio/background_music") {
        out.music = read_music(m.get("default"));
        out.music_creative = read_music(m.get("creative"));
        out.music_underwater = read_music(m.get("underwater"));
        out.music_replaces = m.get("default").and_then(|d| d.get("replace_current_music")).and_then(Nbt::as_bool).unwrap_or(false);
    }
    out
}

#[derive(Debug, Clone)]
pub struct ChatType {
    pub name: String,
    pub translation_key: String,
    /// "sender", "target", "content"
    pub parameters: Vec<String>,
    pub style: Style,
}

#[derive(Debug, Clone, Default)]
pub struct Registries {
    pub dimension_types: Vec<DimensionType>,
    pub biomes: Vec<Biome>,
    pub chat_types: Vec<ChatType>,
    /// registry → tag → IDs
    pub tags: HashMap<String, HashMap<String, Vec<i32>>>,
    /// Raw entry names of every registry, for anything not modelled above.
    pub names: HashMap<String, Vec<String>>,
}

/// Colours appear as "#rrggbb"/"#aarrggbb" strings or as integers.
fn color(tag: Option<&Nbt>) -> Option<u32> {
    match tag? {
        Nbt::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16).ok().map(|c| c & 0xFFFFFF),
        other => other.as_i64().map(|v| v as u32 & 0xFFFFFF),
    }
}

fn attr<'a>(tag: &'a Nbt, name: &str) -> Option<&'a Nbt> {
    let attrs = tag.get("attributes")?;
    attrs.get(name).or_else(|| attrs.get(name.trim_start_matches("minecraft:")))
}

fn f32_of(tag: &Nbt, key: &str, default: f32) -> f32 {
    tag.get(key).and_then(Nbt::as_f64).map(|v| v as f32).unwrap_or(default)
}

impl Registries {
    pub fn insert(&mut self, registry: &str, entries: Vec<(String, Option<Nbt>)>) {
        let registry = registry.trim_start_matches("minecraft:");
        self.names.insert(registry.to_string(), entries.iter().map(|(n, _)| n.clone()).collect());
        let empty = Nbt::Compound(vec![]);
        match registry {
            "dimension_type" => {
                self.dimension_types = entries
                    .into_iter()
                    .map(|(name, tag)| {
                        let t = tag.as_ref().unwrap_or(&empty);
                        DimensionType {
                            name,
                            min_y: t.get("min_y").and_then(Nbt::as_i64).unwrap_or(0) as i32,
                            height: t.get("height").and_then(Nbt::as_i64).unwrap_or(256) as i32,
                            has_skylight: t.get("has_skylight").and_then(Nbt::as_bool).unwrap_or(true),
                            has_ceiling: t.get("has_ceiling").and_then(Nbt::as_bool).unwrap_or(false),
                            ambient_light: f32_of(t, "ambient_light", 0.0),
                            sky_color: color(attr(t, "minecraft:visual/sky_color")),
                            fog_color: color(attr(t, "minecraft:visual/fog_color")),
                            cloud_height: attr(t, "minecraft:visual/cloud_height").and_then(Nbt::as_f64).map(|v| v as f32),
                        }
                    })
                    .collect();
            }
            "worldgen/biome" => {
                self.biomes = entries
                    .into_iter()
                    .map(|(name, tag)| {
                        let t = tag.as_ref().unwrap_or(&empty);
                        let fx = t.get("effects").unwrap_or(&empty);
                        Biome {
                            name,
                            temperature: f32_of(t, "temperature", 0.5),
                            downfall: f32_of(t, "downfall", 0.5),
                            has_precipitation: t.get("has_precipitation").and_then(Nbt::as_bool).unwrap_or(true),
                            water_color: color(fx.get("water_color")).unwrap_or(0x3F76E4),
                            grass_color: color(fx.get("grass_color")),
                            foliage_color: color(fx.get("foliage_color")),
                            grass_modifier: match fx.get("grass_color_modifier").and_then(Nbt::as_str) {
                                Some("dark_forest") => GrassModifier::DarkForest,
                                Some("swamp") => GrassModifier::Swamp,
                                _ => GrassModifier::None,
                            },
                            sky_color: color(attr(t, "minecraft:visual/sky_color")),
                            fog_color: color(attr(t, "minecraft:visual/fog_color")),
                            water_fog_color: color(attr(t, "minecraft:visual/water_fog_color")),
                            sounds: read_biome_sounds(t),
                        }
                    })
                    .collect();
            }
            "chat_type" => {
                self.chat_types = entries
                    .into_iter()
                    .map(|(name, tag)| {
                        let t = tag.as_ref().unwrap_or(&empty);
                        let chat = t.get("chat").unwrap_or(&empty);
                        ChatType {
                            name,
                            translation_key: chat.get("translation_key").and_then(Nbt::as_str).unwrap_or("%s").to_string(),
                            parameters: chat
                                .get("parameters")
                                .and_then(Nbt::as_list)
                                .map(|l| l.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
                                .unwrap_or_default(),
                            style: chat
                                .get("style")
                                .map(|s| lattice_proto::Text::from_nbt(s).style)
                                .unwrap_or_default(),
                        }
                    })
                    .collect();
            }
            _ => {}
        }
    }

    pub fn set_tags(&mut self, tags: Vec<(String, Vec<(String, Vec<i32>)>)>) {
        for (registry, list) in tags {
            let entry = self.tags.entry(registry.trim_start_matches("minecraft:").to_string()).or_default();
            for (tag, ids) in list {
                entry.insert(tag.trim_start_matches("minecraft:").to_string(), ids);
            }
        }
    }

    pub fn tag(&self, registry: &str, tag: &str) -> &[i32] {
        self.tags.get(registry).and_then(|t| t.get(tag)).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn biome_bits(&self) -> u32 {
        lattice_data::ceil_log2(self.biomes.len().max(1) as u32)
    }
}
