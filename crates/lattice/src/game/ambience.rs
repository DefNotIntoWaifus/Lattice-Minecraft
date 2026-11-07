//! Sounds nobody sends: music, the air of a place, rain on the ground, the hush underwater.
//! Decides what should be playing; `crate::audio` makes it heard.

use super::Session;
use crate::audio::{self, Audio};
use glam::Vec3;
use lattice_world::registries::{Biome, BiomeSounds};
use rand::Rng;

const MUSIC_USUAL: i32 = 24000;
const MUSIC_FREQUENT: i32 = 12000;
const MUSIC_CONSTANT: i32 = 100;

pub struct Ambience {
    music_wait: i32,
    /// 0 usual, 1 frequent, 2 all the time.
    pub music_frequency: i32,
    current_music: Option<String>,
    underwater: bool,
    underwater_loop: Option<u64>,
    /// The biome loop now playing, by the event that started it.
    biome_loop: Option<(String, u64)>,
    /// Builds up in the dark until something stirs.
    mood: f32,
    rain_wait: i32,
}

impl Default for Ambience {
    fn default() -> Self {
        // A little while after arriving, not straight away.
        Ambience { music_frequency: 0, music_wait: 100, current_music: None, underwater: false, underwater_loop: None, biome_loop: None, mood: 0.0, rain_wait: 0 }
    }
}

impl Ambience {
    pub fn reset(&mut self, audio: &mut Audio) {
        if let Some(id) = self.underwater_loop.take() {
            audio.stop_loop(id, 0.2);
        }
        if let Some((_, id)) = self.biome_loop.take() {
            audio.stop_loop(id, 0.2);
        }
        self.current_music = None;
        self.music_wait = 100;
        self.mood = 0.0;
    }

    /// One game tick.
    pub fn tick(&mut self, session: &Session, audio: &mut Audio) {
        let Some(world) = &session.world else { return };
        let p = session.player.body.pos;
        let eye = p + glam::DVec3::new(0.0, session.player.eye_height as f64, 0.0);
        let (bx, by, bz) = (eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
        let biome: Option<&Biome> = session.biome_at(bx, by, bz);
        let sounds: Option<&BiomeSounds> = biome.map(|b| &b.sounds);
        let mut rng = rand::rng();

        // Underwater: everything goes quiet and muffled-sounding.
        let under = session.player.body.eye_in_water;
        if under != self.underwater {
            self.underwater = under;
            if under {
                audio.play("ambient.underwater.enter", audio::AMBIENT, None, 1.0, 1.0);
                self.underwater_loop = Some(audio.play_loop("ambient.underwater.loop", audio::AMBIENT, 0.65, 2.0));
            } else {
                audio.play("ambient.underwater.exit", audio::AMBIENT, None, 1.0, 1.0);
                if let Some(id) = self.underwater_loop.take() {
                    audio.stop_loop(id, 1.5);
                }
            }
        }

        // The biome's own air.
        let wanted = sounds.and_then(|s| s.ambient_loop.clone());
        if self.biome_loop.as_ref().map(|(n, _)| n) != wanted.as_ref() {
            if let Some((_, id)) = self.biome_loop.take() {
                audio.stop_loop(id, 2.0);
            }
            if let Some(event) = wanted {
                let id = audio.play_loop(&event, audio::AMBIENT, 1.0, 2.0);
                self.biome_loop = Some((event, id));
            }
        }
        if let Some(extra) = sounds.and_then(|s| s.additions.as_ref()) {
            if rng.random::<f64>() < extra.1 {
                audio.play(&extra.0, audio::AMBIENT, None, 1.0, 1.0);
            }
        }
        // The dark gets to you: an odd noise now and then in caves.
        if let Some(mood) = sounds.and_then(|s| s.mood.as_ref()) {
            let (sky, block) = world.light(bx, by, bz);
            let dark = 15 - sky.max(block) as i32;
            let dark_enough = sky == 0 && block < 8;
            if dark_enough {
                self.mood += dark as f32 / mood.tick_delay.max(1) as f32;
            } else {
                self.mood = (self.mood - 0.0025).max(0.0);
            }
            if self.mood >= 1.0 {
                self.mood = 0.0;
                let dir = Vec3::new(rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0)).normalize_or_zero();
                let at = eye.as_vec3() + dir * mood.offset as f32;
                audio.play(&mood.sound, audio::AMBIENT, Some(at), 1.0, 1.0);
            }
        }

        // Rain somewhere close by.
        if session.rain_level > 0.0 && world.dimension.has_skylight && biome.is_some_and(|b| b.has_precipitation && b.temperature >= 0.15) {
            self.rain_wait -= 1;
            if self.rain_wait <= 0 {
                self.rain_wait = rng.random_range(3..=5);
                let (x, z) = (bx + rng.random_range(-10..=10), bz + rng.random_range(-10..=10));
                let data = lattice_data::game();
                let mut top = None;
                for y in (world.geometry.min_y..=by + 30).rev() {
                    if world.block(x, y, z).is_some_and(|s| data.state(s).has(lattice_data::flags::SOLID) || data.state(s).fluid != 0) {
                        top = Some(y + 1);
                        break;
                    }
                }
                if let Some(y) = top {
                    let at = Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5);
                    let (event, volume, pitch) = if y > by + 1 { ("weather.rain.above", 0.1, 0.5) } else { ("weather.rain", 0.2, 1.0) };
                    audio.play(event, audio::WEATHER, Some(at), volume * session.rain_level, pitch);
                }
            }
        }

        // Music: a track, then a long quiet.
        let (track, min_delay, max_delay) = music_choice(session, sounds, under);
        if audio.music_playing() {
            // A place with music of its own can cut in, as the game lets it.
            if self.current_music.as_deref() != Some(track.as_str()) && sounds.is_some_and(|s| s.music_replaces) {
                audio.stop_category(Some(audio::MUSIC));
                self.music_wait = 0;
            }
        } else if self.music_wait > 0 {
            self.music_wait -= 1;
        } else {
            audio.play(&track, audio::MUSIC, None, 1.0, 1.0);
            self.current_music = Some(track);
            // How often is up to the player.
            let (most, constant) = match self.music_frequency {
                1 => (MUSIC_FREQUENT, false),
                2 => (MUSIC_CONSTANT, true),
                _ => (MUSIC_USUAL, false),
            };
            self.music_wait = if constant { 100 } else { rng.random_range(min_delay.min(most)..=max_delay.min(most).max(min_delay.min(most))) };
        }
    }
}

fn music_choice(session: &Session, sounds: Option<&BiomeSounds>, under: bool) -> (String, i32, i32) {
    let creative = session.player.game_mode == 1;
    if let Some(s) = sounds {
        if under {
            if let Some(m) = &s.music_underwater {
                return m.clone();
            }
        }
        if creative {
            if let Some(m) = &s.music_creative {
                return m.clone();
            }
        }
        if let Some(m) = &s.music {
            return m.clone();
        }
    }
    if creative { ("music.creative".to_string(), 1200, 3600) } else { ("music.game".to_string(), 12000, 24000) }
}
