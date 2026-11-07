//! Development aids for scripted test runs (see docs/DESIGN.md §13). Everything here
//! is switched on by environment variables and does nothing in normal use.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;

fn timed(var: &str) -> Vec<(f32, String)> {
    std::env::var(var)
        .ok()
        .map(|v| v.split(',').filter_map(|e| e.split_once(':')).filter_map(|(t, a)| Some((t.parse().ok()?, a.to_string()))).collect())
        .unwrap_or_default()
}

/// "5:a.png,9:b.png"
pub fn timed_paths(var: &str) -> Vec<(f32, PathBuf)> {
    timed(var).into_iter().map(|(t, p)| (t, PathBuf::from(p))).collect()
}

/// "1:w+,3:yaw=90"
pub fn timed_actions(var: &str) -> Vec<(f32, String)> {
    timed(var)
}

pub fn seconds(var: &str) -> Option<f32> {
    std::env::var(var).ok().and_then(|v| v.parse().ok())
}

/// `LATTICE_HEADLESS=1280x760`: run without a window, drawing to a texture.
pub fn headless_size() -> Option<(u32, u32)> {
    let v = std::env::var("LATTICE_HEADLESS").ok()?;
    let (w, h) = v.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Commands for a headless game, one JSON object per line on stdin.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum TestCommand {
    /// A key, by its winit name, going down or up. `text` is what it types.
    Key { code: String, down: bool, #[serde(default)] text: Option<String> },
    /// A mouse button while the mouse drives the camera.
    Button { button: u8, down: bool },
    Wheel { delta: i32 },
    Look { yaw: f32, pitch: f32 },
    /// A click on the interface at a position.
    Click { x: f32, y: f32, #[serde(default)] button: u8, #[serde(default)] shift: bool },
    /// Moves the pointer without clicking, for tooltips.
    Pointer { x: f32, y: f32 },
    /// Typed text going to whatever has focus.
    Type { text: String },
    /// A key press for the interface.
    UiKey { key: String },
    Chat { line: String },
    Capture { path: String, #[serde(default)] world_only: bool },
    Dump { id: u64 },
    /// Gives this player a made-up cape, to look at how capes hang.
    TestCape,
    Resize { w: u32, h: u32 },
    /// Holds the frame rate at this many frames per second (0: as fast as possible).
    Fps { fps: u32 },
    Quit,
}

/// One clickable thing on the interface, by label.
#[derive(Debug, Clone, Serialize)]
pub struct Widget {
    pub label: String,
    pub rect: [f32; 4],
}

thread_local! {
    static WIDGETS: RefCell<Option<Vec<Widget>>> = const { RefCell::new(None) };
}

/// Turns on recording of widget positions, for clicking by label.
pub fn enable_widget_log() {
    WIDGETS.with(|w| *w.borrow_mut() = Some(Vec::new()));
}

/// Notes where a control was drawn, in test runs.
pub fn note_widget(label: &str, rect: egui::Rect) {
    WIDGETS.with(|w| {
        if let Some(list) = w.borrow_mut().as_mut() {
            list.push(Widget { label: label.to_string(), rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y] });
        }
    });
}

/// The widgets drawn last frame.
pub fn take_widgets() -> Vec<Widget> {
    WIDGETS.with(|w| w.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize)]
pub struct DumpSlot {
    pub slot: usize,
    pub item: String,
    pub count: i32,
    pub damage: i32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DumpEntity {
    pub id: i32,
    pub kind: String,
    pub pos: [f64; 3],
    pub health: Option<f32>,
    pub hurt_ticks: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Dump {
    pub advancements: usize,
    pub advancements_done: usize,
    pub stats: usize,
    pub subtitles: Vec<String>,
    /// Fluid blocks close to the player, as this client knows them.
    pub fluid_near: u32,
    pub dirty_sections: usize,
    pub mesh_in_flight: usize,
    pub meshes_uploaded: u64,
    pub on_fire: bool,
    pub eye_in_water: bool,
    /// What the recipe book lists, and how many recipes are known of each kind.
    pub book_rows: Vec<String>,
    pub book_search: String,
    pub recipe_kinds: std::collections::BTreeMap<String, usize>,
    pub cut_choices: usize,
    pub chat_input: String,
    /// What's being ridden, and how it's steered if it's this player's to steer.
    pub vehicle: Option<String>,
    pub driving: Option<String>,
    pub vehicle_pos: Option<[f64; 3]>,
    pub jump_scale: f32,
    pub captured: bool,
    pub paused: bool,
    /// A sign or book screen, by kind.
    pub modal: Option<String>,
    /// The sign being written, or the book's pages.
    pub modal_text: Vec<String>,
    pub modal_page: usize,
    pub links: usize,
    pub waypoints: usize,
    pub boss_bars: Vec<String>,
    pub sidebar: Vec<String>,
    pub tab: Vec<String>,
    pub maps: usize,
    pub recipes: usize,
    pub cutter_recipes: usize,
    pub tick_rate: f32,
    pub border_size: f64,
    pub suggestions: Vec<String>,
    /// Sounds started, and files that couldn't be loaded.
    pub particles: usize,
    pub sounds_started: u32,
    pub sounds_failed: u32,
    pub dump: u64,
    pub screen: String,
    pub phase: String,
    pub pos: Option<[f64; 3]>,
    pub vel: Option<[f64; 3]>,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub swimming: bool,
    pub sprinting: bool,
    pub flying: bool,
    pub pose: String,
    pub game_mode: i32,
    pub health: f32,
    pub food: i32,
    pub dead: bool,
    pub selected_slot: i32,
    pub dimension: String,
    pub menu: Option<String>,
    pub menu_title: Option<String>,
    pub own_screen: bool,
    pub debug: bool,
    pub swing: f32,
    pub slots: Vec<DumpSlot>,
    pub cursor: Option<DumpSlot>,
    pub chat: Vec<String>,
    pub target_block: Option<String>,
    pub target_entity: Option<i32>,
    pub entities: Vec<DumpEntity>,
    pub widgets: Vec<Widget>,
    pub fps: f32,
    pub columns: usize,
    pub overlays: Vec<String>,
}

/// Writes a line for the test runner.
pub fn say(value: &impl Serialize) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if serde_json::to_writer(&mut out, value).is_ok() {
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}
