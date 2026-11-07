//! Sound: which file an event means, fetching and decoding it off the main thread, and a small
//! mixer that places each voice in the world around the listener.

use crossbeam_channel::{Receiver, Sender, unbounded};
use glam::Vec3;
use lattice_assets::AssetStore;
use rand::Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Sound categories, in the game's numbering.
pub const MASTER: usize = 0;
pub const MUSIC: usize = 1;
pub const WEATHER: usize = 3;
pub const BLOCKS: usize = 4;
pub const HOSTILE: usize = 5;
pub const NEUTRAL: usize = 6;
pub const PLAYERS: usize = 7;
pub const AMBIENT: usize = 8;
pub const UI: usize = 10;
pub const CATEGORIES: usize = 11;

/// One decoded sound, kept as 16-bit samples.
pub struct Pcm {
    samples: Vec<i16>,
    channels: u16,
    rate: u32,
}

struct Variant {
    /// On the first of an event's sounds: the key of the text saying what it is.
    subtitle: Option<String>,
    name: String,
    volume: f32,
    pitch: f32,
    weight: u32,
    stream: bool,
    event: bool,
}

struct Voice {
    pcm: Arc<Pcm>,
    /// Position in frames.
    at: f64,
    step: f64,
    volume: f32,
    category: usize,
    /// In the world, or right at the listener when `None`.
    pos: Option<Vec3>,
    range: f32,
    looping: Option<u64>,
    /// Fades in and out by this much per second, for loops that start and stop.
    fade: f32,
    fade_target: f32,
    fade_speed: f32,
}

struct Mixer {
    voices: Vec<Voice>,
    volumes: [f32; CATEGORIES],
    listener: Vec3,
    right: Vec3,
    out_rate: u32,
}

impl Mixer {
    /// Adds `frames` stereo frames into `out`.
    fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let dt = frames as f32 / self.out_rate.max(1) as f32;
        let master = self.volumes[MASTER];
        let (listener, right) = (self.listener, self.right);
        let volumes = self.volumes;
        self.voices.retain_mut(|v| {
            let total = v.pcm.samples.len() / v.pcm.channels.max(1) as usize;
            if total == 0 {
                return false;
            }
            let fade_from = v.fade;
            if v.fade != v.fade_target {
                let step = v.fade_speed * dt;
                v.fade = if v.fade < v.fade_target { (v.fade + step).min(v.fade_target) } else { (v.fade - step).max(v.fade_target) };
            }
            if v.fade <= 0.0 && v.fade_target <= 0.0 {
                return false;
            }
            let base = v.volume * volumes[v.category] * master;
            let (mut left_gain, mut right_gain) = (base, base);
            if let Some(p) = v.pos {
                let to = p - listener;
                let d = to.length();
                let fall = (1.0 - d / v.range.max(1.0)).clamp(0.0, 1.0);
                // Sources right on top of the listener are in both ears.
                let pan = if d > 0.5 { to.dot(right) / d } else { 0.0 };
                left_gain = base * fall * ((1.0 - pan) * 0.5).sqrt() * std::f32::consts::SQRT_2;
                right_gain = base * fall * ((1.0 + pan) * 0.5).sqrt() * std::f32::consts::SQRT_2;
            }
            let ch = v.pcm.channels.max(1) as usize;
            for (i, frame) in out.chunks_exact_mut(2).enumerate() {
                let i0 = v.at as usize;
                if i0 >= total {
                    if v.looping.is_some() {
                        v.at -= total as f64;
                        continue;
                    }
                    return false;
                }
                let f = (v.at - i0 as f64) as f32;
                let i1 = if i0 + 1 >= total { if v.looping.is_some() { 0 } else { i0 } } else { i0 + 1 };
                let sample = |n: usize, c: usize| v.pcm.samples[n * ch + c.min(ch - 1)] as f32 / 32768.0;
                let l = sample(i0, 0) * (1.0 - f) + sample(i1, 0) * f;
                let r = if ch > 1 { sample(i0, 1) * (1.0 - f) + sample(i1, 1) * f } else { l };
                let fade = fade_from + (v.fade - fade_from) * (i as f32 / frames.max(1) as f32);
                if v.pos.is_some() {
                    // In the world, stereo files are heard as one point.
                    let m = if ch > 1 { (l + r) * 0.5 } else { l };
                    frame[0] += m * left_gain * fade;
                    frame[1] += m * right_gain * fade;
                } else {
                    frame[0] += l * left_gain * fade;
                    frame[1] += r * right_gain * fade;
                }
                v.at += v.step;
            }
            true
        });
        for s in out.iter_mut() {
            // A soft limit rather than a hard clip when many things sound at once.
            *s = if s.abs() <= 0.8 { *s } else { s.signum() * (0.8 + (1.0 - (-(s.abs() - 0.8) * 5.0).exp()) * 0.2) };
        }
    }
}

fn decode(bytes: Vec<u8>) -> Result<Pcm, String> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let channels = reader.ident_hdr.audio_channels as u16;
    let rate = reader.ident_hdr.audio_sample_rate;
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| e.to_string())? {
        samples.extend_from_slice(&packet);
    }
    Ok(Pcm { samples, channels: channels.max(1), rate })
}

struct Job {
    name: String,
}

struct Done {
    name: String,
    pcm: Option<Arc<Pcm>>,
}

/// A request waiting for its file.
struct Waiting {
    file: String,
    since: Instant,
    start: StartVoice,
}

#[derive(Clone)]
struct StartVoice {
    volume: f32,
    pitch: f32,
    category: usize,
    pos: Option<Vec3>,
    range: f32,
    looping: Option<u64>,
    fade_in: f32,
    is_music: bool,
}

pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    _stream: Option<cpal::Stream>,
    pub device: Option<String>,
    events: HashMap<String, Vec<Variant>>,
    cache: HashMap<String, Arc<Pcm>>,
    cache_bytes: usize,
    jobs: Option<Sender<Job>>,
    heard: Vec<(String, Option<Vec3>)>,
    wanted_device: String,
    done: Option<Receiver<Done>>,
    asked: HashMap<String, Instant>,
    waiting: Vec<Waiting>,
    /// For tests: how many voices have started and files have failed.
    pub started: Arc<AtomicU32>,
    pub failed: Arc<AtomicU32>,
    next_loop: u64,
    opening: Option<OpenState>,
}

struct OpenState {
    events: Receiver<Option<(Arc<AssetStore>, HashMap<String, Vec<Variant>>)>>,
    jobs: Receiver<Job>,
    done: Sender<Done>,
    failed: Arc<AtomicU32>,
}

impl Audio {
    /// Starts up on the named output, or the system's default, with the background loaders.
    /// Having no sound device, or no list of sounds, is fine: everything else keeps working in
    /// silence.
    pub fn start_on(wanted: &str, cache: PathBuf) -> Audio {
        let mixer = Arc::new(Mutex::new(Mixer { voices: Vec::new(), volumes: [1.0; CATEGORIES], listener: Vec3::ZERO, right: Vec3::X, out_rate: 44100 }));
        let (stream, device) = open_output(mixer.clone(), wanted);
        let (jobs, job_rx) = unbounded::<Job>();
        let (done_tx, done) = unbounded::<Done>();
        let failed = Arc::new(AtomicU32::new(0));
        let (events_tx, events_rx) = crossbeam_channel::bounded::<Option<(Arc<AssetStore>, HashMap<String, Vec<Variant>>)>>(1);
        // The list of files may need downloading, so that can't hold up the window either.
        let opener = std::thread::Builder::new().name("sound list".into()).spawn(move || {
            let store = match AssetStore::open(&cache) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!("sound won't play: {e}");
                    let _ = events_tx.send(None);
                    return;
                }
            };
            let events = store.read("minecraft/sounds.json").ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(|v| parse_events(&v)).unwrap_or_default();
            tracing::info!("sound: {} events", events.len());
            let _ = events_tx.send(Some((store, events)));
        });
        if let Err(e) = opener {
            tracing::warn!("couldn't start the sound loader: {e}");
        }
        let mut audio = Audio {
            mixer,
            _stream: stream,
            device,
            events: HashMap::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            jobs: Some(jobs),
            heard: Vec::new(),
            wanted_device: wanted.to_string(),
            done: Some(done),
            asked: HashMap::new(),
            waiting: Vec::new(),
            started: Arc::new(AtomicU32::new(0)),
            failed: failed.clone(),
            next_loop: 1,
            opening: None,
        };
        audio.opening = Some(OpenState { events: events_rx, jobs: job_rx, done: done_tx, failed });
        audio
    }

    pub fn set_volumes(&self, v: [f32; CATEGORIES]) {
        if let Ok(mut m) = self.mixer.lock() {
            m.volumes = v;
        }
    }

    /// Where sounds are heard from.
    pub fn set_listener(&self, pos: Vec3, yaw_degrees: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.listener = pos;
            let y = yaw_degrees.to_radians();
            // Looking along (−sin, cos): right is one quarter turn from that.
            m.right = Vec3::new(-y.cos(), 0.0, -y.sin());
        }
    }

    fn pick<'a>(&'a self, event: &str, depth: u32) -> Option<(&'a Variant, f32, f32)> {
        let list = self.events.get(event)?;
        let total: u32 = list.iter().map(|v| v.weight).sum();
        if total == 0 {
            return None;
        }
        let mut r = rand::rng().random_range(0..total);
        for v in list {
            if r < v.weight {
                if v.event && depth < 8 {
                    let (inner, vol, pitch) = self.pick(&v.name, depth + 1)?;
                    return Some((inner, vol * v.volume, pitch * v.pitch));
                }
                return Some((v, v.volume, v.pitch));
            }
            r -= v.weight;
        }
        None
    }

    /// Plays a sound event. `pos` is in the world; `None` plays it right at the listener.
    pub fn play(&mut self, event: &str, category: usize, pos: Option<Vec3>, volume: f32, pitch: f32) {
        let _ = self.start_event(event, category, pos, volume, pitch, None, 0.0);
    }

    /// Starts a looping sound that fades in; stop it with `stop_loop`. Returns its handle.
    pub fn play_loop(&mut self, event: &str, category: usize, volume: f32, fade_in: f32) -> u64 {
        let id = self.next_loop;
        self.next_loop += 1;
        self.start_event(event, category, None, volume, 1.0, Some(id), fade_in);
        id
    }

    pub fn stop_loop(&mut self, id: u64, fade_seconds: f32) {
        self.waiting.retain(|w| w.start.looping != Some(id));
        if let Ok(mut m) = self.mixer.lock() {
            for v in &mut m.voices {
                if v.looping == Some(id) {
                    v.fade_target = 0.0;
                    v.fade_speed = 1.0 / fade_seconds.max(0.01);
                }
            }
        }
    }

    /// Stops what a server asks: a category, a named sound, or everything.
    pub fn stop_category(&mut self, category: Option<usize>) {
        self.waiting.retain(|w| category.is_some_and(|c| w.start.category != c));
        if let Ok(mut m) = self.mixer.lock() {
            m.voices.retain(|v| category.is_some_and(|c| v.category != c));
        }
    }

    pub fn music_playing(&self) -> bool {
        self.waiting.iter().any(|w| w.start.is_music) || self.mixer.lock().is_ok_and(|m| m.voices.iter().any(|v| v.category == MUSIC))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_event(&mut self, event: &str, category: usize, pos: Option<Vec3>, volume: f32, pitch: f32, looping: Option<u64>, fade_in: f32) -> bool {
        let event = event.trim_start_matches("minecraft:");
        if let Some(key) = self.events.get(event).and_then(|l| l.first()).and_then(|v| v.subtitle.clone()) {
            // What's out of earshot isn't heard, so it isn't written down either.
            let range = (volume.max(1.0) * 16.0).max(1.0);
            if pos.is_none_or(|p| p.distance(self.listener_pos()) <= range) {
                self.heard.push((key, pos));
            }
        }
        let Some((variant, v_volume, v_pitch)) = self.pick(event, 0) else { return false };
        let file = format!("minecraft/sounds/{}.ogg", variant.name.trim_start_matches("minecraft:"));
        let stream = variant.stream;
        let range = (volume.max(1.0) * 16.0).max(1.0);
        let start = StartVoice {
            volume: (volume * v_volume).clamp(0.0, 1.0),
            pitch: (pitch * v_pitch).clamp(0.5, 2.0),
            category,
            pos,
            range,
            looping,
            fade_in,
            is_music: category == MUSIC,
        };
        if let Some(pcm) = self.cache.get(&file).cloned() {
            self.start_voice(pcm, &start);
            return true;
        }
        self.waiting.push(Waiting { file: file.clone(), since: Instant::now(), start });
        let fresh = self.asked.get(&file).is_none_or(|t| t.elapsed().as_secs() > 30);
        if fresh {
            self.asked.insert(file.clone(), Instant::now());
            if let Some(j) = &self.jobs {
                let _ = j.send(Job { name: file });
            }
        }
        let _ = stream;
        true
    }

    fn start_voice(&self, pcm: Arc<Pcm>, s: &StartVoice) {
        let Ok(mut m) = self.mixer.lock() else { return };
        // When too much is sounding, the oldest thing that isn't music or a loop gives way.
        if m.voices.len() >= 96 {
            if let Some(i) = m.voices.iter().position(|v| v.category != MUSIC && v.looping.is_none()) {
                m.voices.remove(i);
            }
        }
        let step = s.pitch as f64 * pcm.rate as f64 / m.out_rate.max(1) as f64;
        let fade = if s.fade_in > 0.0 { 0.0 } else { 1.0 };
        m.voices.push(Voice { pcm, at: 0.0, step, volume: s.volume, category: s.category, pos: s.pos, range: s.range, looping: s.looping, fade, fade_target: 1.0, fade_speed: 1.0 / s.fade_in.max(0.01) });
        self.started.fetch_add(1, Ordering::Relaxed);
    }

    /// Moves to another output if the choice has changed.
    pub fn use_device(&mut self, wanted: &str) {
        if self.wanted_device == wanted {
            return;
        }
        self.wanted_device = wanted.to_string();
        let (stream, device) = open_output(self.mixer.clone(), wanted);
        self._stream = stream;
        self.device = device;
    }

    /// What's been heard since last asked: the text's key, and where from.
    pub fn take_heard(&mut self) -> Vec<(String, Option<Vec3>)> {
        std::mem::take(&mut self.heard)
    }

    fn listener_pos(&self) -> Vec3 {
        self.mixer.lock().map_or(Vec3::ZERO, |m| m.listener)
    }

    /// Picks up what the background threads have finished. Call every frame.
    pub fn pump(&mut self) {
        if let Some(open) = &self.opening {
            match open.events.try_recv() {
                Ok(Some((store, events))) => {
                    self.events = events;
                    let open = self.opening.take().unwrap();
                    prefetch_effects(store.clone());
                    for _ in 0..3 {
                        let (rx, tx, store, failed) = (open.jobs.clone(), open.done.clone(), store.clone(), open.failed.clone());
                        let spawned = std::thread::Builder::new().name("sound loader".into()).spawn(move || {
                            while let Ok(job) = rx.recv() {
                                let pcm = match store.read(&job.name).map_err(|e| e.to_string()).and_then(decode) {
                                    Ok(p) => Some(Arc::new(p)),
                                    Err(e) => {
                                        tracing::debug!("couldn't load {}: {e}", job.name);
                                        failed.fetch_add(1, Ordering::Relaxed);
                                        None
                                    }
                                };
                                let _ = tx.send(Done { name: job.name, pcm });
                            }
                        });
                        if let Err(e) = spawned {
                            tracing::warn!("couldn't start a sound loader: {e}");
                        }
                    }
                }
                Ok(None) => {
                    self.opening = None;
                    self.jobs = None;
                }
                Err(_) => {}
            }
        }
        let finished: Vec<Done> = self.done.as_ref().map(|d| d.try_iter().collect()).unwrap_or_default();
        for d in finished {
            let Some(pcm) = d.pcm else {
                self.waiting.retain(|w| w.file != d.name);
                continue;
            };
            let waiting = std::mem::take(&mut self.waiting);
            for w in waiting {
                if w.file == d.name {
                    // Late effects are worse than none; music and loops still want to start.
                    if w.start.is_music || w.start.looping.is_some() || w.since.elapsed().as_secs_f32() < 1.5 {
                        self.start_voice(pcm.clone(), &w.start);
                    }
                } else {
                    self.waiting.push(w);
                }
            }
            let size = pcm.samples.len() * 2;
            // Long tracks aren't worth keeping in memory once they've finished.
            if size < 8 * 1024 * 1024 {
                if self.cache_bytes + size > 256 * 1024 * 1024 {
                    self.cache.clear();
                    self.cache_bytes = 0;
                }
                self.cache_bytes += size;
                self.cache.insert(d.name, pcm);
            }
        }
        self.waiting.retain(|w| w.start.is_music || w.start.looping.is_some() || w.since.elapsed().as_secs() < 5);
    }

}

/// Short sounds should play the moment they're asked for, so they're fetched ahead of time, a few
/// at a time, in the background. Music is left until it's wanted, as it's by far the largest part.
fn prefetch_effects(store: Arc<AssetStore>) {
    let names: Vec<String> = store
        .names_under("minecraft/sounds/")
        .into_iter()
        .filter(|n| !n.starts_with("minecraft/sounds/music/") && !n.starts_with("minecraft/sounds/records/") && !store.is_cached(n))
        .map(|n| n.to_string())
        .collect();
    if names.is_empty() {
        return;
    }
    tracing::info!("fetching {} sound effects", names.len());
    let (tx, rx) = unbounded::<String>();
    for n in names {
        let _ = tx.send(n);
    }
    drop(tx);
    let misses = Arc::new(std::sync::atomic::AtomicU32::new(0));
    for _ in 0..6 {
        let (rx, store, misses) = (rx.clone(), store.clone(), misses.clone());
        let spawned = std::thread::Builder::new().name("sound fetch".into()).spawn(move || {
            while let Ok(name) = rx.recv() {
                if misses.load(Ordering::Relaxed) > 10 {
                    return;
                }
                if let Err(e) = store.read(&name) {
                    // Offline, most likely: stop trying, and sounds are fetched as they're needed.
                    if misses.fetch_add(1, Ordering::Relaxed) == 10 {
                        tracing::info!("stopped fetching sounds ahead of time: {e}");
                    }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("couldn't start fetching sounds: {e}");
        }
    }
}

fn parse_events(v: &Value) -> HashMap<String, Vec<Variant>> {
    let mut out = HashMap::new();
    let Some(obj) = v.as_object() else { return out };
    for (name, def) in obj {
        let mut list = Vec::new();
        let subtitle = def.get("subtitle").and_then(Value::as_str).map(str::to_string);
        for s in def.get("sounds").and_then(Value::as_array).into_iter().flatten() {
            match s {
                Value::String(n) => list.push(Variant { name: n.clone(), volume: 1.0, pitch: 1.0, weight: 1, stream: false, event: false, subtitle: None }),
                o => {
                    let Some(n) = o.get("name").and_then(Value::as_str) else { continue };
                    list.push(Variant {
                        name: n.to_string(),
                        volume: o.get("volume").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                        pitch: o.get("pitch").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                        weight: o.get("weight").and_then(Value::as_u64).unwrap_or(1) as u32,
                        stream: o.get("stream").and_then(Value::as_bool).unwrap_or(false),
                        event: o.get("type").and_then(Value::as_str) == Some("event"),
                        subtitle: None,
                    });
                }
            }
        }
        if let (Some(first), Some(key)) = (list.first_mut(), subtitle) {
            first.subtitle = Some(key);
        }
        out.insert(name.clone(), list);
    }
    out
}

/// The names of the outputs that could be used.
pub fn device_names() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    // Asking can be slow, so it's only done now and then.
    static CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed().as_secs() < 10 {
            return list.clone();
        }
    }
    let list: Vec<String> = cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    *cache = Some((Instant::now(), list.clone()));
    list
}

fn open_output(mixer: Arc<Mutex<Mixer>>, wanted: &str) -> (Option<cpal::Stream>, Option<String>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    if std::env::var_os("LATTICE_HEADLESS").is_some() {
        return (None, None);
    }
    let host = cpal::default_host();
    let chosen = (!wanted.is_empty()).then(|| host.output_devices().ok()?.find(|d| d.name().is_ok_and(|n| n == wanted))).flatten();
    let Some(device) = chosen.or_else(|| host.default_output_device()) else {
        tracing::warn!("no sound output device; running in silence");
        return (None, None);
    };
    let name = device.name().ok();
    let Ok(config) = device.default_output_config() else {
        tracing::warn!("the sound device has no usable format; running in silence");
        return (None, None);
    };
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    if let Ok(mut m) = mixer.lock() {
        m.out_rate = rate;
    }
    let cfg: cpal::StreamConfig = config.config();
    let mut scratch: Vec<f32> = Vec::new();
    let error = |e| tracing::warn!("sound output error: {e}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &cfg,
            move |out: &mut [f32], _| {
                let frames = out.len() / channels.max(1);
                scratch.resize(frames * 2, 0.0);
                if let Ok(mut m) = mixer.try_lock() {
                    m.mix(&mut scratch);
                } else {
                    scratch.fill(0.0);
                }
                for (i, frame) in out.chunks_exact_mut(channels.max(1)).enumerate() {
                    frame[0] = scratch[i * 2];
                    if channels > 1 {
                        frame[1] = scratch[i * 2 + 1];
                    }
                    for extra in frame.iter_mut().skip(2) {
                        *extra = 0.0;
                    }
                }
            },
            error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &cfg,
            move |out: &mut [i16], _| {
                let frames = out.len() / channels.max(1);
                scratch.resize(frames * 2, 0.0);
                if let Ok(mut m) = mixer.try_lock() {
                    m.mix(&mut scratch);
                } else {
                    scratch.fill(0.0);
                }
                for (i, frame) in out.chunks_exact_mut(channels.max(1)).enumerate() {
                    frame[0] = (scratch[i * 2] * 32767.0) as i16;
                    if channels > 1 {
                        frame[1] = (scratch[i * 2 + 1] * 32767.0) as i16;
                    }
                    for extra in frame.iter_mut().skip(2) {
                        *extra = 0;
                    }
                }
            },
            error,
            None,
        ),
        other => {
            tracing::warn!("sound device uses an unsupported format ({other:?}); running in silence");
            return (None, name);
        }
    };
    match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                tracing::warn!("couldn't start sound: {e}");
                return (None, name);
            }
            tracing::info!("sound output: {} at {rate} Hz", name.clone().unwrap_or_default());
            (Some(s), name)
        }
        Err(e) => {
            tracing::warn!("couldn't open the sound device: {e}");
            (None, name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Arc<Pcm> {
        Arc::new(Pcm { samples: (0..4410).map(|i| ((i as f32 * 0.1).sin() * 20000.0) as i16).collect(), channels: 1, rate: 44100 })
    }

    fn mixer() -> Mixer {
        Mixer { voices: Vec::new(), volumes: [1.0; CATEGORIES], listener: Vec3::ZERO, right: Vec3::X, out_rate: 44100 }
    }

    fn voice(pos: Option<Vec3>) -> Voice {
        Voice { pcm: tone(), at: 0.0, step: 1.0, volume: 1.0, category: BLOCKS, pos, range: 16.0, looping: None, fade: 1.0, fade_target: 1.0, fade_speed: 1.0 }
    }

    fn energy(out: &[f32], channel: usize) -> f32 {
        out.iter().skip(channel).step_by(2).map(|s| s * s).sum()
    }

    #[test]
    fn sounds_on_the_right_are_louder_on_the_right() {
        let mut m = mixer();
        m.voices.push(voice(Some(Vec3::new(8.0, 0.0, 0.0))));
        let mut out = vec![0.0; 2048];
        m.mix(&mut out);
        assert!(energy(&out, 1) > energy(&out, 0) * 10.0, "{} {}", energy(&out, 1), energy(&out, 0));
    }

    #[test]
    fn far_sounds_are_silent_and_near_ones_are_not() {
        let mut far = mixer();
        far.voices.push(voice(Some(Vec3::new(100.0, 0.0, 0.0))));
        let mut out = vec![0.0; 1024];
        far.mix(&mut out);
        assert_eq!(energy(&out, 0) + energy(&out, 1), 0.0);
        let mut near = mixer();
        near.voices.push(voice(Some(Vec3::new(0.0, 2.0, 0.0))));
        near.mix(&mut out);
        assert!(energy(&out, 0) > 0.0);
    }

    #[test]
    fn voices_end_and_category_volume_applies() {
        let mut m = mixer();
        m.volumes[BLOCKS] = 0.0;
        m.voices.push(voice(None));
        let mut out = vec![0.0; 1024];
        m.mix(&mut out);
        assert_eq!(energy(&out, 0), 0.0);
        let mut long = vec![0.0; 44100];
        m.mix(&mut long);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn stereo_files_keep_both_channels_when_not_placed() {
        let mut m = mixer();
        let pcm = Arc::new(Pcm { samples: vec![10000, -10000].repeat(4410), channels: 2, rate: 44100 });
        let mut v = voice(None);
        v.pcm = pcm;
        m.voices.push(v);
        let mut out = vec![0.0; 64];
        m.mix(&mut out);
        assert!(out[0] > 0.0 && out[1] < 0.0);
    }

    /// Needs a sound device, so it isn't part of normal runs. Plays silence for a moment.
    #[test]
    #[ignore]
    fn real_device_opens() {
        let m = Arc::new(Mutex::new(mixer()));
        m.lock().unwrap().volumes = [0.0; CATEGORIES];
        let (stream, name) = open_output(m.clone(), "");
        println!("device: {name:?}, opened: {}", stream.is_some());
        assert!(stream.is_some());
        std::thread::sleep(std::time::Duration::from_millis(500));
        drop(stream);
    }

    #[test]
    fn event_lists_parse() {
        let v: Value = serde_json::from_str(r#"{"a":{"sounds":["x/y",{"name":"z","volume":0.5,"weight":3,"stream":true},{"name":"b","type":"event"}]}}"#).unwrap();
        let e = parse_events(&v);
        let list = &e["a"];
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].weight, 3);
        assert!(list[1].stream && !list[0].stream);
        assert!(list[2].event);
    }
}
