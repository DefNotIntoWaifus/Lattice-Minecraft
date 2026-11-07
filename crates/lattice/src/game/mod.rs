//! One connection to one server and everything known about it.

pub mod ambience;
pub mod block_entities;
pub mod gm_blocks;
pub mod chat;
pub mod creative;
pub mod effects;
pub mod entities;
pub mod inventory;
pub mod items;
pub mod meshing;
pub mod particles;
pub mod player;
pub mod riding;
pub mod scoreboard;
pub mod swing;

use chat::ChatLog;
use entities::Entities;
use inventory::Inventory;
use items::BlockTags;
use glam::DVec3;
use lattice_net::{Connection, Disconnect, Event, Stage};
use lattice_proto::packets::play::{self, Cb};
use lattice_proto::{Text, Translator};
use lattice_proto::DataValue;
use lattice_world::physics::{MoveTags, Pose};
use lattice_world::{Registries, World};
use player::LocalPlayer;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Connecting(Stage),
    /// Joined; waiting for the terrain around the player.
    LoadingTerrain,
    Playing,
}

#[derive(Debug, Clone)]
pub struct TabEntry {
    pub name: String,
    pub display: Option<Text>,
    pub latency: i32,
    pub game_mode: i32,
    pub listed: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Titles {
    pub title: Option<Text>,
    pub subtitle: Option<Text>,
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
    /// Ticks since the title was set.
    pub age: i32,
    pub action_bar: Option<(Text, Instant)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Vitals {
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub xp_bar: f32,
    pub xp_level: i32,
    pub known: bool,
}

#[derive(Debug, Clone)]
pub struct BossBar {
    pub name: Text,
    pub progress: f32,
    /// What's drawn, catching up with `progress` so jumps look like movement.
    pub shown: f32,
    pub color: i32,
    pub overlay: i32,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub struct MapData {
    pub colors: Vec<u8>,
    pub scale: i32,
    pub locked: bool,
    pub decorations: Vec<play::MapDecoration>,
    /// Bumped each time the picture changes, so it's only uploaded when needed.
    pub version: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct WorldBorder {
    pub x: f64,
    pub z: f64,
    pub size: f64,
    pub target: f64,
    pub lerp_ticks: f64,
}

impl Default for WorldBorder {
    fn default() -> Self {
        WorldBorder { x: 0.0, z: 0.0, size: 5.999997e7, target: 5.999997e7, lerp_ticks: 0.0 }
    }
}

/// A value a server's dialog asks for.
#[derive(Debug, Clone)]
pub struct DialogInput {
    pub key: String,
    pub label: Text,
    pub label_visible: bool,
    pub control: InputControl,
}

#[derive(Debug, Clone)]
pub enum InputControl {
    Text { value: String, max: usize, multiline: bool },
    Toggle { value: bool, on_true: String, on_false: String },
    Options { entries: Vec<(String, Text)>, index: usize },
    Number { value: f32, start: f32, end: f32, step: Option<f32> },
}

impl InputControl {
    /// The value as words, as a command template wants it.
    pub fn as_text(&self) -> String {
        match self {
            InputControl::Text { value, .. } => value.clone(),
            InputControl::Toggle { value, on_true, on_false } => {
                if *value { on_true.clone() } else { on_false.clone() }
            }
            InputControl::Options { entries, index } => entries.get(*index).map_or(String::new(), |e| e.0.clone()),
            InputControl::Number { value, .. } => {
                if value.fract() == 0.0 { format!("{}", *value as i64) } else { value.to_string() }
            }
        }
    }

    /// The value as a tag, as a custom action's payload wants it.
    pub fn as_nbt(&self) -> lattice_proto::Nbt {
        use lattice_proto::Nbt;
        match self {
            InputControl::Toggle { value, .. } => Nbt::Byte(*value as i8),
            InputControl::Number { value, .. } => Nbt::Float(*value),
            other => Nbt::String(other.as_text()),
        }
    }
}

/// `$(key)` in a command template, filled in from what's been entered.
pub fn fill_template(template: &str, inputs: &[DialogInput]) -> String {
    let mut out = template.to_string();
    for i in inputs {
        out = out.replace(&format!("$({})", i.key), &i.control.as_text());
    }
    out
}

/// What a dialog says about its inputs, read from the server's description.
fn parse_inputs(nbt: &lattice_proto::Nbt) -> Vec<DialogInput> {
    use lattice_proto::Nbt;
    let mut out = Vec::new();
    for n in nbt.get("inputs").and_then(Nbt::as_list).unwrap_or(&[]) {
        let Some(key) = n.get("key").and_then(Nbt::as_str) else { continue };
        let label = n.get("label").map_or_else(|| Text::literal(key), Text::from_nbt);
        let label_visible = n.get("label_visible").and_then(Nbt::as_i64).is_none_or(|v| v != 0);
        let text = |k: &str, d: &str| n.get(k).and_then(Nbt::as_str).unwrap_or(d).to_string();
        let number = |k: &str| n.get(k).and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))).map(|v| v as f32);
        let control = match n.get("type").and_then(Nbt::as_str).unwrap_or("").trim_start_matches("minecraft:") {
            "text" => InputControl::Text {
                value: text("initial", ""),
                max: n.get("max_length").and_then(Nbt::as_i64).unwrap_or(32).max(1) as usize,
                multiline: n.get("multiline").is_some(),
            },
            "boolean" => InputControl::Toggle {
                value: n.get("initial").and_then(Nbt::as_i64).is_some_and(|v| v != 0),
                on_true: text("on_true", "true"),
                on_false: text("on_false", "false"),
            },
            "single_option" => {
                let mut entries = Vec::new();
                let mut index = 0;
                for (i, o) in n.get("options").and_then(Nbt::as_list).unwrap_or(&[]).iter().enumerate() {
                    // An option is a bare id, or a compound that can name itself.
                    let id = o.as_str().or_else(|| o.get("id").and_then(Nbt::as_str)).unwrap_or("").to_string();
                    let display = o.get("display").map_or_else(|| Text::literal(id.clone()), Text::from_nbt);
                    if o.get("initial").and_then(Nbt::as_i64).is_some_and(|v| v != 0) {
                        index = i;
                    }
                    entries.push((id, display));
                }
                InputControl::Options { entries, index }
            }
            "number_range" => {
                let (start, end) = (number("start").unwrap_or(0.0), number("end").unwrap_or(1.0));
                InputControl::Number { value: number("initial").unwrap_or((start + end) / 2.0), start, end, step: number("step") }
            }
            _ => continue,
        };
        out.push(DialogInput { key: key.to_string(), label, label_visible, control });
    }
    out
}

/// The registries of this window's server, for things drawn without a session to hand.
static ACTIVE_REGISTRIES: std::sync::RwLock<Option<Arc<Registries>>> = std::sync::RwLock::new(None);

/// What a compass needs to know about where it's being held. Set each frame.
#[derive(Clone, Default)]
pub struct CompassView {
    pub yaw: f32,
    pub pos: DVec3,
    pub dimension: String,
    pub spawn: Option<(String, lattice_proto::BlockPos)>,
    pub death: Option<(String, lattice_proto::BlockPos)>,
    pub time: f32,
}

static COMPASS_VIEW: std::sync::Mutex<Option<CompassView>> = std::sync::Mutex::new(None);

pub fn set_compass_view(view: CompassView) {
    if let Ok(mut v) = COMPASS_VIEW.lock() {
        *v = Some(view);
    }
}

pub fn compass_view() -> Option<CompassView> {
    COMPASS_VIEW.lock().ok().and_then(|v| v.clone())
}

pub fn active_registries() -> Option<Arc<Registries>> {
    ACTIVE_REGISTRIES.read().ok()?.clone()
}

/// What a server's asked about resource packs, for the window to act on.
#[derive(Debug, Clone)]
pub enum PackRequest {
    Offer { id: Uuid, url: String, hash: String, forced: bool },
    Remove(Option<Uuid>),
}

/// A point on the locator bar.
#[derive(Debug, Clone, Copy)]
pub struct Waypoint {
    pub color: Option<u32>,
    pub place: play::WaypointPlace,
}

#[derive(Debug, Clone)]
pub struct DialogButton {
    pub label: Text,
    pub tooltip: Option<Text>,
    pub action: DialogAction,
}

#[derive(Debug, Clone)]
pub enum DialogAction {
    /// Just closes the screen.
    Close,
    Command(String),
    Suggest(String),
    Url(String),
    Custom(String, Option<lattice_proto::Nbt>),
    /// A command with `$(key)` holes for what's been typed.
    Template(String),
    /// A custom action carrying everything that's been entered.
    CustomAll(String, Option<lattice_proto::Nbt>),
    Copy(String),
    /// Opens another dialog given in full.
    Show(Box<lattice_proto::Nbt>),
}

/// An advancement and how far along it is.
#[derive(Debug, Clone)]
pub struct Advancement {
    pub entry: play::AdvancementEntry,
    pub done: std::collections::HashSet<String>,
}

impl Advancement {
    /// Each group needs one of its criteria done.
    pub fn complete(&self) -> bool {
        !self.entry.requirements.is_empty() && self.entry.requirements.iter().all(|group| group.iter().any(|c| self.done.contains(c)))
    }

    /// How many groups are done, of how many.
    pub fn progress(&self) -> (usize, usize) {
        let done = self.entry.requirements.iter().filter(|g| g.iter().any(|c| self.done.contains(c))).count();
        (done, self.entry.requirements.len())
    }
}

/// A screen the client opens itself.
#[derive(Debug, Clone)]
pub enum Modal {
    /// Asks before leaving for a web page a server offered.
    Link { url: String },
    /// The tree of what's been achieved, on one root's page.
    Advancements { tab: String, offset: (f32, f32) },
    /// Counts of everything done so far.
    Stats { tab: usize, search: String },
    /// A screen the server made: title, text and buttons with what each does.
    Dialog { title: Option<Text>, body: Vec<Text>, inputs: Vec<DialogInput>, buttons: Vec<DialogButton>, can_close: bool },
    /// The links this server offers.
    Links,
    Sign { pos: lattice_proto::BlockPos, front: bool, lines: [String; 4], limit: f32, focus: Option<usize> },
    Book { title: Option<String>, author: Option<String>, pages: Vec<Text>, page: usize },
    WriteBook { slot: i32, pages: Vec<String>, page: usize, title: String, signing: bool },
    /// Editing a command block, structure block or the like.
    Block(Box<gm_blocks::BlockScreen>),
    /// Choosing a game mode, opened with F3 and F4.
    GameMode { selected: usize },
    /// Each rule's name, its value on screen and what the server last said; then the world's difficulty.
    GameRules { rules: Vec<(String, String, String)>, search: String, difficulty: i32 },
    /// A spectator choosing who to go to.
    Teleport,
    /// The End's closing words; the server waits for these to finish.
    Credits { lines: Vec<String>, started: std::time::Instant },
}

/// A date like 2026-11-04T10:20:30.123Z, as milliseconds since 1970.
fn iso_millis(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let time = time.split('+').next()?;
    let (whole, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = whole.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next()??);
    let millis: i64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    // Days since 1970 by the civil calendar.
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + h * 3600 + min * 60 + s) * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_right() {
        assert_eq!(iso_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_millis("2000-03-01T00:00:01.5Z"), Some(951868801500));
        assert_eq!(iso_millis("2026-11-04T10:20:30.123456Z"), Some(1793787630123));
        assert_eq!(iso_millis("nonsense"), None);
    }
}

/// Reads a server's screen. Those asking the player to type things in aren't supported yet, and
/// show as what they say, with a way out.
pub fn parse_dialog(nbt: &lattice_proto::Nbt) -> Option<Modal> {
    use lattice_proto::Nbt;
    let kind = nbt.get("type")?.as_str()?.trim_start_matches("minecraft:");
    let title = nbt.get("title").map(Text::from_nbt);
    let mut body = Vec::new();
    let one_or_many = |n: &Nbt| -> Vec<Nbt> {
        match n.as_list() {
            Some(l) => l.to_vec(),
            None => vec![n.clone()],
        }
    };
    if let Some(b) = nbt.get("body") {
        for part in one_or_many(b) {
            if let Some(c) = part.get("contents") {
                body.push(Text::from_nbt(c));
            }
        }
    }
    let button = |n: &Nbt, fallback: &str| -> DialogButton {
        let label = n.get("label").map_or_else(|| Text::literal(fallback), Text::from_nbt);
        let tooltip = n.get("tooltip").map(Text::from_nbt);
        let action = match n.get("action") {
            None => DialogAction::Close,
            Some(a) => {
                let text = |key: &str| a.get(key).and_then(Nbt::as_str).unwrap_or("").to_string();
                match a.get("type").and_then(Nbt::as_str).unwrap_or("").trim_start_matches("minecraft:") {
                    "run_command" => DialogAction::Command(text("command")),
                    "suggest_command" => DialogAction::Suggest(text("command")),
                    "open_url" => DialogAction::Url(text("url")),
                    "custom" => DialogAction::Custom(text("id"), a.get("payload").cloned()),
                    "dynamic/custom" => DialogAction::CustomAll(text("id"), a.get("additions").cloned()),
                    "dynamic/run_command" => DialogAction::Template(text("template")),
                    "copy_to_clipboard" => DialogAction::Copy(text("value")),
                    "show_dialog" => match a.get("dialog") {
                        Some(d) if d.as_str().is_none() => DialogAction::Show(Box::new(d.clone())),
                        _ => DialogAction::Close,
                    },
                    _ => DialogAction::Close,
                }
            }
        };
        DialogButton { label, tooltip, action }
    };
    let mut buttons = Vec::new();
    match kind {
        "notice" => buttons.push(nbt.get("action").map_or_else(|| DialogButton { label: Text::translate("gui.ok", Vec::new()), tooltip: None, action: DialogAction::Close }, |a| button(a, "OK"))),
        "confirmation" => {
            if let Some(y) = nbt.get("yes") {
                buttons.push(button(y, "Yes"));
            }
            if let Some(n) = nbt.get("no") {
                buttons.push(button(n, "No"));
            }
        }
        // The screen that lists a server's links, which this client already knows.
        "server_links" => return Some(Modal::Links),
        "dialog_list" => {
            for d in nbt.get("dialogs").and_then(Nbt::as_list).map(|l| l.to_vec()).unwrap_or_else(|| nbt.get("dialogs").map_or(Vec::new(), |d| vec![d.clone()])) {
                // Only dialogs sent in full can be opened; named ones live on the server.
                if d.as_str().is_some() {
                    continue;
                }
                let label = d.get("external_title").or(d.get("title")).map_or_else(|| Text::literal("Open"), Text::from_nbt);
                buttons.push(DialogButton { label, tooltip: None, action: DialogAction::Show(Box::new(d.clone())) });
            }
            if let Some(e) = nbt.get("exit_action") {
                buttons.push(button(e, "Back"));
            }
        }
        "multi_action" => {
            for a in nbt.get("actions").and_then(Nbt::as_list).unwrap_or(&[]) {
                buttons.push(button(a, ""));
            }
            if let Some(e) = nbt.get("exit_action") {
                buttons.push(button(e, "Back"));
            }
        }
        _ => {}
    }
    if buttons.is_empty() {
        buttons.push(DialogButton { label: Text::literal("Close"), tooltip: None, action: DialogAction::Close });
    }
    let can_close = nbt.get("can_close_with_escape").and_then(Nbt::as_i64).is_none_or(|v| v != 0);
    Some(Modal::Dialog { title, body, inputs: parse_inputs(nbt), buttons, can_close })
}

impl Session {
    /// What's in a tag of some registry, as last sent.
    pub fn tag(&self, registry: &str, tag: &str) -> &[i32] {
        let late = self.extra_tags.tag(registry, tag);
        if !late.is_empty() {
            return late;
        }
        self.registries.as_ref().map_or(&[], |r| r.tag(registry, tag))
    }

    /// Whether an item is one of a recipe's possibilities.
    pub fn item_in_set(&self, set: &lattice_proto::recipes::ItemSet, item: i32) -> bool {
        match &set.tag {
            Some(t) => self.tag("item", t).contains(&item),
            None => set.items.contains(&item),
        }
    }

    /// The stack a recipe's slot shows, if it can be worked out.
    pub fn display_stack(&self, slot: &lattice_proto::recipes::SlotDisplay) -> Option<lattice_proto::ItemStack> {
        slot.stack(&|t| self.tag("item", t).first().copied())
    }
}

/// One particle wanted at a place, with its start speed.
#[derive(Debug, Clone)]
pub struct ParticleRequest {
    pub particle: lattice_proto::item::Particle,
    pub pos: DVec3,
    pub vel: DVec3,
}

/// A sound to play: an event name, its category, and where.
#[derive(Debug, Clone)]
pub struct SoundCue {
    pub name: String,
    pub category: usize,
    pub pos: Option<DVec3>,
    pub volume: f32,
    pub pitch: f32,
}

impl SoundCue {
    /// A sound with no place: heard the same whichever way you face.
    pub fn own(name: &str, category: usize, volume: f32, pitch: f32) -> SoundCue {
        SoundCue { name: name.to_string(), category, pos: None, volume, pitch }
    }

    /// A sound at a precise spot, such as where someone's standing.
    pub fn around(name: &str, category: usize, pos: DVec3, volume: f32, pitch: f32) -> SoundCue {
        SoundCue { name: name.to_string(), category, pos: Some(pos), volume, pitch }
    }

    pub fn at(name: &str, category: usize, pos: lattice_proto::BlockPos, volume: f32, pitch: f32) -> SoundCue {
        SoundCue { name: name.to_string(), category, pos: Some(DVec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5)), volume, pitch }
    }
}

fn sound_name(s: &play::SoundRef) -> Option<String> {
    match s {
        play::SoundRef::Id(id) => lattice_data::game().registry("sound_event").get(*id as usize).cloned(),
        play::SoundRef::Named(n) => Some(n.trim_start_matches("minecraft:").to_string()),
    }
}

/// Sound effects of world events, by the event's number: event name, volume, pitch.
fn level_event_sound(event: i32) -> Option<(&'static str, f32, f32)> {
    Some(match event {
        1000 => ("block.dispenser.dispense", 1.0, 1.0),
        1001 => ("block.dispenser.fail", 1.0, 1.2),
        1002 => ("block.dispenser.launch", 1.0, 1.2),
        1004 => ("entity.firework_rocket.shoot", 1.0, 1.2),
        1009 => ("block.fire.extinguish", 0.5, 2.6),
        1015 => ("entity.ghast.warn", 10.0, 1.0),
        1016 => ("entity.ghast.shoot", 10.0, 1.0),
        1017 => ("entity.ender_dragon.shoot", 10.0, 1.0),
        1018 => ("entity.blaze.shoot", 2.0, 1.0),
        1019 => ("entity.zombie.attack_wooden_door", 2.0, 1.0),
        1020 => ("entity.zombie.attack_iron_door", 2.0, 1.0),
        1021 => ("entity.zombie.break_wooden_door", 2.0, 1.0),
        1022 => ("entity.wither.break_block", 2.0, 1.0),
        1024 => ("entity.wither.shoot", 2.0, 1.0),
        1025 => ("entity.bat.takeoff", 0.05, 1.0),
        1026 => ("entity.zombie.infect", 2.0, 1.0),
        1027 => ("entity.zombie_villager.converted", 2.0, 1.0),
        1029 => ("block.anvil.destroy", 1.0, 1.0),
        1030 => ("block.anvil.use", 1.0, 1.0),
        1031 => ("block.anvil.land", 0.3, 1.0),
        1032 => ("block.portal.travel", 1.0, 1.0),
        1033 => ("block.chorus_flower.grow", 1.0, 1.0),
        1034 => ("block.chorus_flower.death", 1.0, 1.0),
        1035 => ("block.brewing_stand.brew", 1.0, 1.0),
        1039 => ("entity.phantom.bite", 0.3, 1.0),
        1040 => ("entity.zombie.converted_to_drowned", 2.0, 1.0),
        1041 => ("entity.husk.converted_to_zombie", 2.0, 1.0),
        1042 => ("block.grindstone.use", 1.0, 1.0),
        1043 => ("item.book.page_turn", 1.0, 1.0),
        1044 => ("block.smithing_table.use", 1.0, 1.0),
        1045 => ("block.pointed_dripstone.land", 2.0, 1.0),
        1046 => ("block.pointed_dripstone.drip_lava_into_cauldron", 2.0, 1.0),
        1047 => ("block.pointed_dripstone.drip_water_into_cauldron", 2.0, 1.0),
        1048 => ("entity.skeleton.converted_to_stray", 2.0, 1.0),
        1049 => ("block.crafter.craft", 1.0, 1.0),
        1050 => ("block.crafter.fail", 1.0, 1.0),
        _ => return None,
    })
}

pub struct Session {
    /// The slot of the bundle the pointer is on, and which of its items is picked.
    pub bundle_pick: Option<(i16, i32)>,
    /// How the server remembers each recipe book, until the next menu takes it in.
    pub recipe_book_settings: Option<[(bool, bool); 4]>,
    /// When a totem last saved this player, for showing it fly up.
    pub totem_shown: Option<std::time::Instant>,
    /// Where a compass with nothing else to go on points, and which dimension that's in.
    pub spawn_point: Option<(String, lattice_proto::BlockPos)>,
    /// Where this player last died, for a recovery compass.
    pub death_point: Option<(String, lattice_proto::BlockPos)>,
    /// The open block screen has had its answer from the server.
    pub block_screen_filled: bool,
    pub immediate_respawn: bool,
    /// The story at the End was asked for and needs reading from the game's files.
    pub wants_credits: bool,
    /// Who's been told what by the server, for showing in chat.
    pub copy_transaction: i32,
    /// The next tag-query answer goes to the clipboard, with this command in front.
    pub copy_prefix: Option<String>,
    pub clipboard: Option<String>,
    pub conn: Connection,
    pub server_name: String,
    pub address: String,
    pub phase: Phase,
    pub registries: Option<Arc<Registries>>,
    pub world: Option<World>,
    pub player: LocalPlayer,
    pub entities: Entities,
    pub block_entities: block_entities::BlockEntities,
    /// Signs what this player says, where they've got keys to do it with.
    signer: Option<lattice_proto::signing::ChatSigner>,
    seen: lattice_proto::signing::SeenMessages,
    pub commands: play::CommandTree,
    /// Keys on their way from Mojang.
    pub keys_coming: Option<crossbeam_channel::Receiver<Result<lattice_auth::ProfileKeys, String>>>,
    pub advancements: HashMap<String, Advancement>,
    /// Just achieved, for a note on the screen: title and when.
    pub achieved: Vec<(Text, Instant)>,
    /// Statistics as last sent, by type and thing.
    pub stats: HashMap<(i32, i32), i32>,
    /// Extra words the server offers for completing in chat.
    pub chat_completions: Vec<String>,
    pub scoreboard: scoreboard::Scoreboard,
    /// Tags sent while playing, which replace the ones from when the connection was set up.
    extra_tags: lattice_world::registries::Registries,
    /// Boss bars, in the order they arrived.
    pub boss_bars: Vec<(Uuid, BossBar)>,
    pub boss_darkness: f32,
    pub boss_darkness_prev: f32,
    /// Maps seen so far, by number.
    pub maps: HashMap<i32, MapData>,
    /// Recipes known, for the book, and what the stonecutter can make.
    pub recipes: Vec<lattice_proto::recipes::RecipeEntry>,
    pub cutter: Vec<lattice_proto::recipes::CutterChoice>,
    /// A recipe's outline shown in the grid because its ingredients aren't at hand.
    pub ghost: Option<(i32, lattice_proto::recipes::RecipeDisplay)>,
    /// A screen that isn't a server menu: a sign or a book.
    pub modal: Option<Modal>,
    /// Answers to tab completion, waiting for the chat box.
    pub suggestions: Option<(i32, i32, i32, Vec<(String, Option<Text>)>)>,
    /// Rate of game ticks per second, usually 20, and whether time is frozen.
    pub tick_rate: f32,
    pub frozen: bool,
    pub camera_entity: Option<i32>,
    pub border: WorldBorder,
    /// Particles the server sent, as they were asked for, waiting to be made.
    pub particle_requests: Vec<ParticleRequest>,
    /// Sounds waiting to be played.
    pub cues: Vec<SoundCue>,
    /// Blocks others broke, for the shower of bits.
    pub broke_elsewhere: Vec<(lattice_proto::BlockPos, u32)>,
    /// What a server asked to stop: a category, a named sound.
    pub sound_stops: Vec<(Option<i32>, Option<String>)>,
    /// Profiles whose skins haven't been asked for yet: id and `textures` value.
    pub wanted_skins: Vec<(Uuid, String)>,
    /// Ticks since joining.
    pub ticks: i64,
    /// How hard it's raining and thundering, 0..1.
    pub rain_level: f32,
    pub thunder_level: f32,
    /// Blocks being broken by anyone: the breaker's id to where and how far along.
    pub cracks: HashMap<i32, (lattice_proto::BlockPos, u8)>,
    pub profile_id: Uuid,
    pub inventory: Inventory,
    pub chat: ChatLog,
    pub tab: HashMap<Uuid, TabEntry>,
    pub server_links: Vec<lattice_proto::packets::config::ServerLink>,
    pub pack_requests: Vec<PackRequest>,
    pub waypoints: HashMap<play::WaypointId, Waypoint>,
    pub tab_header: Option<Text>,
    pub tab_footer: Option<Text>,
    pub titles: Titles,
    pub vitals: Vitals,
    pub dead: Option<Text>,
    pub code_of_conduct: Option<String>,
    pub notices: Vec<(String, Instant)>,
    pub secure_chat_required: bool,
    pub online_mode: bool,
    pub hardcore: bool,
    /// Ticks of the dimension's clock, with fraction.
    pub day_time: f64,
    /// Ticks the world has been running, as the server counts them.
    pub game_time: i64,
    day_clock: Option<i32>,
    day_rate: f32,
    pub move_tags: MoveTags,
    pub block_tags: BlockTags,
    pub joined_at: Instant,
    pub chunks_received: u64,
    /// Columns removed since the last frame (renderer must drop their meshes).
    pub removed_columns: Vec<(i32, i32)>,
    pub dimension_changed: bool,
    /// The name of the dimension the player's in.
    pub dimension_name: String,
    pub self_uuid: Uuid,
    /// The server opened a menu since the last frame.
    pub screen_opened: bool,
    /// The server closed the menu since the last frame.
    pub screen_closed: bool,
    /// Ticks since the player was hurt.
    pub hurt_ticks: u32,
    /// Ticks left of the flash from a lightning bolt.
    pub sky_flash: u32,
    /// Ticks spent freezing in powder snow.
    pub frozen_ticks: i32,
    /// Breath left, and extra health from absorption, for the HUD.
    pub air_supply: i32,
    pub absorption: f32,
    /// Which way the warp from a portal or nausea is turned, degrees, and how fast.
    pub spin_angle: f32,
    pub spin_speed: f32,
    /// How far the swirl of a nether portal has come over the view, 0 to 1.
    pub portal_effect: (f32, f32),
    /// Names of the data fields seen on the player's own entity, for the F3 screen.
    pub self_flags: u8,
    pub difficulty: Option<(i32, bool)>,
    pub server_motd: Option<Text>,
}

pub enum SessionEnd {
    Disconnected(Disconnect),
    Transfer(String, i32),
}

impl Session {
    pub fn new(conn: Connection, server_name: String, address: String, self_uuid: Uuid) -> Session {
        Session {
            bundle_pick: None,
            recipe_book_settings: None,
            totem_shown: None,
            spawn_point: None,
            death_point: None,
            block_screen_filled: false,
            immediate_respawn: false,
            wants_credits: false,
            copy_transaction: 0,
            copy_prefix: None,
            clipboard: None,
            conn,
            server_name,
            address,
            phase: Phase::Connecting(Stage::Resolving),
            registries: None,
            world: None,
            player: LocalPlayer::new(),
            entities: Entities::default(),
            inventory: Inventory::default(),
            chat: ChatLog::default(),
            tab: HashMap::new(),
            server_links: Vec::new(),
            pack_requests: Vec::new(),
            waypoints: HashMap::new(),
            tab_header: None,
            tab_footer: None,
            titles: Titles::default(),
            vitals: Vitals::default(),
            dead: None,
            code_of_conduct: None,
            notices: Vec::new(),
            secure_chat_required: false,
            online_mode: false,
            hardcore: false,
            day_time: 6000.0,
            game_time: 0,
            day_clock: None,
            day_rate: 1.0,
            move_tags: MoveTags::defaults(lattice_data::game()),
            block_tags: BlockTags::default(),
            joined_at: Instant::now(),
            chunks_received: 0,
            removed_columns: Vec::new(),
            block_entities: Default::default(),
            signer: None,
            seen: Default::default(),
            commands: Default::default(),
            keys_coming: None,
            advancements: HashMap::new(),
            achieved: Vec::new(),
            stats: HashMap::new(),
            chat_completions: Vec::new(),
            scoreboard: Default::default(),
            extra_tags: Default::default(),
            boss_bars: Vec::new(),
            boss_darkness: 0.0,
            boss_darkness_prev: 0.0,
            maps: HashMap::new(),
            recipes: Vec::new(),
            cutter: Vec::new(),
            ghost: None,
            modal: None,
            suggestions: None,
            tick_rate: 20.0,
            frozen: false,
            camera_entity: None,
            border: WorldBorder::default(),
            particle_requests: Vec::new(),
            cues: Vec::new(),
            broke_elsewhere: Vec::new(),
            sound_stops: Vec::new(),
            wanted_skins: Vec::new(),
            ticks: 0,
            rain_level: 0.0,
            thunder_level: 0.0,
            cracks: HashMap::new(),
            profile_id: Uuid::nil(),
            dimension_changed: false,
            dimension_name: "minecraft:overworld".to_string(),
            self_uuid,
            screen_opened: false,
            screen_closed: false,
            hurt_ticks: u32::MAX,
            sky_flash: 0,
            frozen_ticks: 0,
            air_supply: 300,
            absorption: 0.0,
            spin_angle: 0.0,
            spin_speed: 0.0,
            portal_effect: (0.0, 0.0),
            self_flags: 0,
            difficulty: None,
            server_motd: None,
        }
    }

    fn set_block_tags<'a>(&mut self, list: impl Iterator<Item = (&'a str, &'a [i32])>) {
        for (tag, ids) in list {
            let tag = tag.trim_start_matches("minecraft:");
            let set: std::collections::HashSet<u16> = ids.iter().map(|&b| b as u16).collect();
            self.move_tags.set_from_server(tag, set.iter().copied());
            self.block_tags.0.insert(tag.to_string(), set);
        }
        // Walls and fence gates are one group for movement.
        let mut group = self.block_tags.0.get("walls").cloned().unwrap_or_default();
        group.extend(self.block_tags.0.get("fence_gates").into_iter().flatten().copied());
        if !group.is_empty() {
            self.move_tags.walls_and_gates = group;
        }
    }

    /// Synced data of the player's own entity.
    fn self_data(&mut self, values: Vec<(u8, DataValue)>) {
        for (index, value) in values {
            match (index, value) {
                // Shared flags: bit 7 is gliding.
                (0, DataValue::Byte(v)) => {
                    self.self_flags = v;
                    self.player.body.fall_flying = v & 0x80 != 0;
                }
                // Breath left, out of 300.
                (1, DataValue::Int(v)) => self.air_supply = v,
                // How frozen, from standing in powder snow.
                (7, DataValue::Int(v)) => self.frozen_ticks = v,
                // Extra health from absorption.
                (17, DataValue::Float(v)) => self.absorption = v,
                // The living entity flags: bit 0 using an item, bit 1 which hand.
                (8, DataValue::Byte(v)) => self.player.set_using_from_server(v),
                // Pose: sleeping is the server's decision.
                (6, DataValue::Pose(p)) => {
                    if p == 2 {
                        // Gets the pointer back, for the way out of bed.
                        if self.player.body.pose != Pose::Sleeping {
                            self.screen_opened = true;
                        }
                        self.player.body.pose = Pose::Sleeping;
                        self.player.body.immobile = true;
                    } else if self.player.body.pose == Pose::Sleeping {
                        self.player.body.pose = Pose::Standing;
                        self.player.body.immobile = false;
                        self.screen_closed = true;
                    }
                }
                _ => {}
            }
        }
    }

    pub fn notice(&mut self, s: impl Into<String>) {
        self.notices.push((s.into(), Instant::now()));
    }

    /// Drains network events. Returns `Some` when the session is over.
    pub fn pump(&mut self, tr: &dyn Translator) -> Option<SessionEnd> {
        let mut budget = 4000;
        while budget > 0 {
            let Some(ev) = self.conn.try_event() else { break };
            budget -= 1;
            match ev {
                Event::Stage(s) => self.phase = Phase::Connecting(s),
                Event::LoggedIn(profile) => self.profile_id = profile.uuid,
                Event::CodeOfConduct(text) => self.code_of_conduct = Some(text),
                Event::ResourcePackOffer { id, url, hash, forced } => self.pack_requests.push(PackRequest::Offer { id, url, hash, forced }),
                Event::ResourcePackRemoved(id) => self.pack_requests.push(PackRequest::Remove(id)),
                Event::Notice(n) => self.notice(n),
                Event::ServerLinks(links) => self.server_links = links,
                Event::Configured(reg) => {
                    if let Ok(mut active) = ACTIVE_REGISTRIES.write() {
                        *active = Some(reg.clone());
                    }
                    self.set_block_tags(reg.tags.get("block").into_iter().flatten().map(|(t, ids)| (t.as_str(), ids.as_slice())));
                    self.registries = Some(reg);
                }
                Event::Reconfiguring => {
                    self.phase = Phase::Connecting(Stage::Configuring);
                }
                Event::Chunk(col, block_entities) => {
                    self.chunks_received += 1;
                    let (cx, cz) = (col.x, col.z);
                    self.block_entities.forget_column(cx, cz);
                    for be in block_entities {
                        let x = (cx << 4) + (be.packed_xz >> 4) as i32;
                        let z = (cz << 4) + (be.packed_xz & 15) as i32;
                        let pos = lattice_proto::BlockPos { x, y: be.y as i32, z };
                        self.block_entities.set_data(pos, be.data);
                    }
                    if let Some(w) = &mut self.world {
                        w.insert_column(col);
                    }
                }
                Event::Play(p) => self.handle_play(*p, tr),
                Event::Transfer { host, port } => return Some(SessionEnd::Transfer(host, port)),
                Event::Disconnected(d) => return Some(SessionEnd::Disconnected(d)),
            }
        }
        None
    }

    fn enter_dimension(&mut self, spawn: &play::SpawnInfo) {
        let Some(reg) = &self.registries else { return };
        let Some(dim) = reg.dimension_types.get(spawn.dimension_type as usize).cloned() else {
            tracing::warn!("unknown dimension type {}", spawn.dimension_type);
            return;
        };
        // Which clock drives this dimension's day: the overworld clock if present.
        self.day_clock = reg
            .names
            .get("world_clock")
            .and_then(|n| n.iter().position(|c| c.ends_with(if dim.name.ends_with("the_end") { "the_end" } else { "overworld" })))
            .map(|i| i as i32);
        if let Some(w) = &self.world {
            self.removed_columns.extend(w.columns().map(|c| (c.x, c.z)));
        }
        self.block_entities.clear();
        self.world = Some(World::new(dim));
        self.entities.clear();
        self.player.game_mode = spawn.game_mode;
        self.player.reset_for_spawn();
        self.dimension_changed = true;
        self.dimension_name = spawn.dimension_name.clone();
        self.death_point = spawn.death_location.clone();
        self.phase = Phase::LoadingTerrain;
    }

    fn handle_play(&mut self, p: Cb, tr: &dyn Translator) {
        if let Cb::EntityEvent { id, event } = &p {
            if *id == self.player.entity_id && (24..=28).contains(event) {
                self.player.permission = (*event - 24) as u8;
            }
        }
        match p {
            Cb::Login(j) => {
                self.player.entity_id = j.entity_id;
                self.hardcore = j.hardcore;
                self.secure_chat_required = j.enforces_secure_chat;
                self.online_mode = j.online_mode;
                self.hardcore = j.hardcore;
                self.enter_dimension(&j.spawn);
                self.joined_at = Instant::now();
            }
            Cb::Respawn { spawn, data_kept } => {
                self.dead = None;
                self.enter_dimension(&spawn);
                // Bit 1 keeps attributes, bit 2 keeps entity data; otherwise it all starts over.
                if data_kept & 2 == 0 {
                    self.player.effects.clear();
                    self.player.set_effect(-1, None);
                }
                self.inventory.menu = None;
                self.inventory.own_screen_open = false;
                self.screen_closed = true;
            }
            Cb::PlayerPosition { teleport_id, pos, velocity, yaw, pitch, flags } => {
                self.player.teleport(teleport_id, pos, velocity, yaw, pitch, flags);
            }
            Cb::ForgetChunk { x, z } => {
                self.block_entities.forget_column(x, z);
                if let Some(w) = &mut self.world {
                    if w.remove_column(x, z) {
                        self.removed_columns.push((x, z));
                    }
                }
            }
            // A block that's only there for a moment, from the client's point of view a normal change.
            Cb::AddTransientBlock { pos, state } => {
                if let Some(w) = &mut self.world {
                    w.set_block(pos, state as u32);
                }
            }
            Cb::BlockUpdate { pos, state } => {
                tracing::debug!("block update {pos:?} -> {}", lattice_data::game().block(state as u32).name);
                self.block_entities.block_changed(pos, state as u32);
                if let Some(w) = &mut self.world {
                    w.set_block(pos, state as u32);
                }
            }
            Cb::SectionBlocksUpdate { section, blocks } => {
                for (packed, state) in &blocks {
                    let (x, y, z) = ((*packed >> 8 & 15) as i32, (*packed & 15) as i32, (*packed >> 4 & 15) as i32);
                    let pos = lattice_proto::BlockPos { x: (section[0] << 4) + x, y: (section[1] << 4) + y, z: (section[2] << 4) + z };
                    self.block_entities.block_changed(pos, *state as u32);
                }
                if let Some(w) = &mut self.world {
                    w.set_section_blocks(section, &blocks);
                }
            }
            Cb::BlockEntityData { pos, data, .. } => {
                // The answer to opening a block screen, or to asking it to find its size.
                if let Some(Modal::Block(screen)) = &mut self.modal {
                    if screen.pos() == Some(pos) {
                        if self.block_screen_filled {
                            screen.take_new_size(&data);
                        } else {
                            screen.take_data(&data, tr);
                            self.block_screen_filled = true;
                        }
                    }
                }
                self.block_entities.set_data(pos, data);
            }
            Cb::BossEvent { id, op } => match op {
                play::BossOp::Add { name, progress, color, overlay, flags } => self.boss_bars.push((id, BossBar { name, progress, shown: progress, color, overlay, flags })),
                play::BossOp::Remove => self.boss_bars.retain(|(i, _)| *i != id),
                other => {
                    if let Some((_, b)) = self.boss_bars.iter_mut().find(|(i, _)| *i == id) {
                        match other {
                            play::BossOp::Progress(v) => b.progress = v,
                            play::BossOp::Name(n) => b.name = n,
                            play::BossOp::Style { color, overlay } => {
                                b.color = color;
                                b.overlay = overlay;
                            }
                            play::BossOp::Flags(f) => b.flags = f,
                            _ => {}
                        }
                    }
                }
            },
            Cb::SetObjective { name, method, details } => self.scoreboard.set_objective(name, method, details),
            Cb::SetDisplayObjective { slot, name } => self.scoreboard.set_display(slot, name),
            Cb::SetScore { owner, objective, score, display, format } => self.scoreboard.set_score(owner, objective, score, display, format),
            Cb::ResetScore { owner, objective } => self.scoreboard.reset_score(&owner, objective.as_deref()),
            Cb::SetPlayerTeam { name, method, params, players } => self.scoreboard.set_team(name, method, params, players),
            Cb::MapItemData { id, scale, locked, decorations, patch } => {
                let map = self.maps.entry(id).or_insert_with(|| MapData { colors: vec![0; 128 * 128], scale, locked, decorations: Vec::new(), version: 0 });
                map.scale = scale;
                map.locked = locked;
                if let Some(d) = decorations {
                    map.decorations = d;
                }
                if let Some(p) = patch {
                    for y in 0..p.height as usize {
                        for x in 0..p.width as usize {
                            let (tx, ty) = (x + p.x as usize, y + p.y as usize);
                            if tx < 128 && ty < 128 {
                                if let Some(c) = p.colors.get(x + y * p.width as usize) {
                                    map.colors[tx + ty * 128] = *c;
                                }
                            }
                        }
                    }
                    map.version += 1;
                }
            }
            Cb::UpdateRecipes { cutter } => self.cutter = cutter,
            Cb::RecipeBookAdd { entries, replace } => {
                if replace {
                    self.recipes.clear();
                }
                for (e, _) in entries {
                    self.recipes.retain(|r| r.id != e.id);
                    self.recipes.push(e);
                }
            }
            Cb::RecipeBookRemove(ids) => self.recipes.retain(|r| !ids.contains(&r.id)),
            Cb::PlaceGhostRecipe { container, display } => self.ghost = Some((container, display)),
            Cb::Commands(tree) => self.commands = tree,
            Cb::UpdateAdvancements { reset, added, removed, progress, show } => {
                if reset {
                    self.advancements.clear();
                }
                for id in removed {
                    self.advancements.remove(&id);
                }
                for entry in added {
                    let id = entry.id.clone();
                    let done = self.advancements.remove(&id).map_or_else(Default::default, |a| a.done);
                    self.advancements.insert(id, Advancement { entry, done });
                }
                for (id, criteria) in progress {
                    let Some(a) = self.advancements.get_mut(&id) else { continue };
                    let was = a.complete();
                    a.done = criteria.into_iter().filter(|(_, when)| when.is_some()).map(|(c, _)| c).collect();
                    if !was && a.complete() && show {
                        if let Some(d) = a.entry.display.as_ref().filter(|d| d.show_toast) {
                            self.achieved.push((d.title.clone(), Instant::now()));
                        }
                    }
                }
            }
            Cb::AwardStats(list) => {
                for (kind, thing, count) in list {
                    self.stats.insert((kind, thing), count);
                }
            }
            Cb::CookieRequest(key) => self.conn.send(play::cookie_response_empty(&key)),
            Cb::CustomChatCompletions { action, entries } => match action {
                0 => self.chat_completions.extend(entries),
                1 => self.chat_completions.retain(|e| !entries.contains(e)),
                _ => self.chat_completions = entries,
            },
            Cb::ShowDialog(nbt) => {
                if let Some(d) = nbt.as_ref().and_then(parse_dialog) {
                    self.modal = Some(d);
                    self.screen_opened = true;
                }
            }
            Cb::ServerLinks(links) => self.server_links = links,
            Cb::Waypoint { op, id, color, place } => {
                if op == 1 {
                    self.waypoints.remove(&id);
                } else {
                    let w = self.waypoints.entry(id).or_insert(Waypoint { color, place });
                    w.place = place;
                    w.color = color.or(w.color);
                }
            }
            Cb::ClearDialog => {
                if matches!(self.modal, Some(Modal::Dialog { .. })) {
                    self.modal = None;
                    self.screen_closed = true;
                }
            }
            Cb::CommandSuggestions { id, start, length, entries } => self.suggestions = Some((id, start, length, entries)),
            Cb::PlayerLookAt { from_eyes, target, entity } => {
                let eye = self.player.body.pos + DVec3::new(0.0, self.player.eye_height as f64, 0.0);
                let aim = entity
                    .and_then(|(id, eyes)| self.entities.map.get(&id).map(|e| (e, eyes)))
                    .map(|(e, eyes)| e.pos + DVec3::new(0.0, if eyes { e.info().map_or(1.6, |i| i.eye as f64) } else { 0.0 }, 0.0))
                    .unwrap_or_else(|| DVec3::from(target));
                let from = if from_eyes { eye } else { self.player.body.pos };
                let d = aim - from;
                let flat = (d.x * d.x + d.z * d.z).sqrt();
                let yaw = (d.z.atan2(d.x).to_degrees() - 90.0) as f32;
                let pitch = (-d.y.atan2(flat).to_degrees()) as f32;
                self.player.body.yaw = yaw;
                self.player.body.pitch = pitch;
                self.player.look_override = Some((yaw, pitch));
            }
            Cb::SetCamera(id) => self.camera_entity = (id != self.player.entity_id).then_some(id),
            Cb::TickingState { rate, frozen } => {
                self.tick_rate = rate.max(0.1);
                self.frozen = frozen;
            }
            Cb::TickingStep(_) => {}
            Cb::MoveVehicle { pos, yaw, pitch } => self.player.server_moved_vehicle(pos, yaw, pitch),
            Cb::ChangeDifficulty { difficulty, locked } => self.difficulty = Some((difficulty, locked)),
            Cb::ServerData { motd } => self.server_motd = Some(motd),
            Cb::ChunksBiomes(list) => {
                let bits = self.registries.as_ref().map_or(7, |r| r.biome_bits());
                if let Some(w) = &mut self.world {
                    for (x, z, data) in list {
                        w.update_biomes(x, z, &data, bits);
                    }
                }
            }
            Cb::InitializeBorder { x, z, old_size, new_size, lerp_ms, .. } => {
                self.border = WorldBorder { x, z, size: old_size, target: new_size, lerp_ticks: lerp_ms as f64 / 50.0 };
            }
            Cb::SetBorderCenter { x, z } => {
                self.border.x = x;
                self.border.z = z;
            }
            Cb::SetBorderLerpSize { old_size, new_size, lerp_ms } => {
                self.border.size = old_size;
                self.border.target = new_size;
                self.border.lerp_ticks = lerp_ms as f64 / 50.0;
            }
            Cb::SetBorderSize(size) => {
                self.border.size = size;
                self.border.target = size;
                self.border.lerp_ticks = 0.0;
            }
            Cb::OpenSignEditor { pos, front } => {
                let mut lines: [String; 4] = Default::default();
                let hanging = self.world.as_ref().and_then(|w| w.block_at(pos)).is_some_and(|s| lattice_data::game().block(s).name.contains("hanging"));
                let side = if front { "front_text" } else { "back_text" };
                if let Some(list) = self.block_entities.get(pos).and_then(|b| b.data.as_ref()).and_then(|d| d.get(side)).and_then(|s| s.get("messages")).and_then(lattice_proto::Nbt::as_list) {
                    for (i, m) in list.iter().take(4).enumerate() {
                        lines[i] = Text::from_nbt(m).to_plain(tr);
                    }
                }
                self.modal = Some(Modal::Sign { pos, front, lines, limit: if hanging { 60.0 } else { 90.0 }, focus: Some(0) });
                self.screen_opened = true;
            }
            Cb::OpenBook { hand } => {
                let stack = if hand == 0 { self.inventory.hotbar(self.player.selected_slot.clamp(0, 8) as usize) } else { self.inventory.offhand() };
                if let Some(stack) = stack {
                    if let Some((title, author, pages)) = items::written_book(stack) {
                        self.modal = Some(Modal::Book { title: Some(title), author: Some(author), pages, page: 0 });
                        self.screen_opened = true;
                    }
                }
            }
            Cb::Sound { sound, source, pos, volume, pitch, .. } => {
                if let Some(name) = sound_name(&sound) {
                    self.cues.push(SoundCue { name, category: source as usize, pos: Some(DVec3::from(pos)), volume, pitch });
                }
            }
            Cb::SoundEntity { sound, source, entity, volume, pitch, .. } => {
                if let Some(name) = sound_name(&sound) {
                    let at = if entity == self.player.entity_id { Some(self.player.body.pos) } else { self.entities.map.get(&entity).map(|e| e.pos) };
                    if let Some(at) = at {
                        self.cues.push(SoundCue { name, category: source as usize, pos: Some(at), volume, pitch });
                    }
                }
            }
            Cb::StopSound { source, name } => self.sound_stops.push((source, name)),
            Cb::LevelParticles { particle, pos, dist, speed, count, kind, .. } => {
                let pos = DVec3::from(pos);
                let mut rng = rand::rng();
                let gauss = |r: &mut rand::rngs::ThreadRng| -> f64 {
                    use rand::Rng;
                    let (a, b): (f64, f64) = (r.random::<f64>().max(1e-9), r.random());
                    (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
                };
                if count == 0 {
                    let v = DVec3::new((speed[0] * dist[0]) as f64, (speed[1] * dist[1]) as f64, (speed[2] * dist[2]) as f64);
                    self.particle_requests.push(ParticleRequest { particle, pos, vel: v });
                } else {
                    use rand::Rng;
                    for _ in 0..count.min(4096) {
                        let (at, v) = if kind != 0 {
                            let at = DVec3::new(rng.random::<f64>() * dist[0] as f64, rng.random::<f64>() * dist[1] as f64, rng.random::<f64>() * dist[2] as f64);
                            let scale = if kind == 2 { DVec3::new(rng.random(), rng.random(), rng.random()) } else { DVec3::ONE };
                            (at, DVec3::new(speed[0] as f64, speed[1] as f64, speed[2] as f64) * scale)
                        } else {
                            (
                                DVec3::new(gauss(&mut rng) * dist[0] as f64, gauss(&mut rng) * dist[1] as f64, gauss(&mut rng) * dist[2] as f64),
                                DVec3::new(gauss(&mut rng) * speed[0] as f64, gauss(&mut rng) * speed[1] as f64, gauss(&mut rng) * speed[2] as f64),
                            )
                        };
                        self.particle_requests.push(ParticleRequest { particle: particle.clone(), pos: pos + at, vel: v });
                    }
                }
            }
            Cb::Explode { center, knockback, particle, sound, play_sound, .. } => {
                if let Some(k) = knockback {
                    self.player.body.vel += DVec3::from(k);
                }
                if play_sound {
                    if let Some(name) = sound_name(&sound) {
                        use rand::Rng;
                        let mut r = rand::rng();
                        let pitch = (1.0 + (r.random::<f32>() - r.random::<f32>()) * 0.2) * 0.7;
                        self.cues.push(SoundCue { name, category: crate::audio::BLOCKS, pos: Some(DVec3::from(center)), volume: 4.0, pitch });
                    }
                }
                self.particle_requests.push(ParticleRequest { particle, pos: DVec3::from(center), vel: DVec3::new(1.0, 0.0, 0.0) });
            }
            Cb::LevelEvent { event, pos, data, .. } => {
                let game = lattice_data::game();
                if event == 2001 {
                    // A block breaking, somewhere else.
                    let block = game.block(data as u32);
                    if let Some(st) = game.sound_types.get(block.sound as usize) {
                        self.cues.push(SoundCue::at(&st.break_sound, crate::audio::BLOCKS, pos, (st.volume + 1.0) / 2.0, st.pitch * 0.8));
                    }
                    self.broke_elsewhere.push((pos, data as u32));
                } else if let Some((name, volume, pitch)) = level_event_sound(event) {
                    self.cues.push(SoundCue::at(name, crate::audio::BLOCKS, pos, volume, pitch));
                }
            }
            Cb::TakeItemEntity { item, .. } => {
                if let Some(e) = self.entities.map.get(&item) {
                    let orb = e.type_name() == "experience_orb";
                    let r = |seed: i32| (seed.wrapping_mul(1103515245) as u32 >> 16) as f32 / 65536.0;
                    let (name, volume, pitch) = if orb {
                        ("entity.experience_orb.pickup", 0.1, (r(item) - r(item + 1)) * 0.35 + 0.9)
                    } else {
                        ("entity.item.pickup", 0.2, (r(item) - r(item + 1)) * 1.4 + 2.0)
                    };
                    self.cues.push(SoundCue { name: name.to_string(), category: crate::audio::PLAYERS, pos: Some(e.pos), volume, pitch });
                }
            }
            Cb::BlockEvent { pos, a, b, .. } => self.block_entities.event(pos, a, b),
            Cb::LightUpdate { x, z, light } => {
                if let Some(w) = &mut self.world {
                    w.apply_light(x, z, &light);
                }
            }
            Cb::GameEvent { event, value } => match event {
                // "You have no home bed or charged respawn anchor."
                0 => self.chat.push(Text::translate("block.minecraft.spawn.not_valid", Vec::new())),
                1 => self.rain_level = 0.0,
                2 => self.rain_level = 1.0,
                3 => {
                    self.player.previous_game_mode = self.player.game_mode;
                    self.player.game_mode = value as i32;
                }
                // Leaving the End: the server waits for the client before moving the player.
                4 => self.wants_credits = true,
                6 => self.cues.push(SoundCue::own("entity.arrow.hit_player", crate::audio::PLAYERS, 0.18, 0.45)),
                7 => self.rain_level = value.clamp(0.0, 1.0),
                8 => self.thunder_level = value.clamp(0.0, 1.0),
                9 => self.cues.push(SoundCue::own("entity.puffer_fish.sting", crate::audio::NEUTRAL, 1.0, 1.0)),
                10 => self.cues.push(SoundCue::own("entity.elder_guardian.curse", crate::audio::HOSTILE, 1.0, 1.0)),
                11 => self.immediate_respawn = value == 1.0,
                _ => {}
            },
            Cb::SetDefaultSpawnPosition { dimension, pos, .. } => self.spawn_point = Some((dimension, pos)),
            Cb::GameRuleValues(values) => {
                if let Some(Modal::GameRules { rules, .. }) = &mut self.modal {
                    *rules = values.iter().map(|(k, v)| (k.clone(), v.clone(), v.clone())).collect();
                    rules.sort();
                }
            }
            Cb::LowDiskSpace => self.chat.push(Text::literal("The server is running low on disk space.".to_string())),
            Cb::TestInstanceStatus { status, size } => {
                if let Some(Modal::Block(screen)) = &mut self.modal {
                    if let gm_blocks::BlockScreen::TestInstance(s) = screen.as_mut() {
                        s.status = status.to_plain(tr).lines().map(str::to_string).collect();
                        if let Some(size) = size {
                            s.size = size.map(|n| n.to_string());
                        }
                    }
                }
            }
            Cb::SelectAdvancementsTab(tab) => {
                if let (Some(Modal::Advancements { tab: open, .. }), Some(tab)) = (&mut self.modal, tab) {
                    *open = tab;
                }
            }
            Cb::TagQuery { transaction, data } => {
                if transaction == self.copy_transaction {
                    if let Some(prefix) = self.copy_prefix.take() {
                        self.clipboard = Some(format!("{prefix} {}", data.to_snbt()));
                        self.chat.push(Text::literal("Copied what's stored there.".to_string()));
                    }
                }
            }
            // Taking back a message that a server now wants unsaid.
            Cb::DeleteChat { signature: Some(sig), .. } => self.chat.delete(&sig),
            Cb::RecipeBookSettings(books) => self.recipe_book_settings = Some(books),
            // The red tint uses the border's own numbers, which it already has from starting up.
            Cb::DeleteChat { .. } | Cb::BorderWarningDelay(_) | Cb::BorderWarningDistance(_) => {}
            Cb::BlockDestruction { entity, pos, stage } => {
                if stage < 10 {
                    self.cracks.insert(entity, (pos, stage));
                } else {
                    self.cracks.remove(&entity);
                }
            }
            Cb::PlayerAbilities { flags, flying_speed, .. } => {
                let b = &mut self.player.body;
                b.flying = flags & 0x02 != 0;
                b.may_fly = flags & 0x04 != 0;
                b.fly_speed = flying_speed;
                self.player.instant_break = flags & 0x08 != 0;
            }
            Cb::SetHealth { health, food, saturation } => {
                self.vitals.health = health;
                self.vitals.food = food;
                self.vitals.saturation = saturation;
                self.vitals.known = true;
            }
            Cb::SetExperience { bar, level, .. } => {
                self.vitals.xp_bar = bar;
                self.vitals.xp_level = level;
            }
            Cb::CombatDeath { player, message } if player == self.player.entity_id => {
                if self.immediate_respawn {
                    // The doImmediateRespawn rule: no death screen.
                    self.respawn();
                } else {
                    self.dead = Some(message);
                }
                self.modal = None;
            }
            Cb::SetHeldSlot(s) => self.player.selected_slot = s.clamp(0, 8),
            Cb::UpdateAttributes { entity, attributes } if entity == self.player.entity_id => {
                self.player.apply_attributes(&attributes);
            }
            Cb::SetEntityMotion { id, velocity } if id == self.player.entity_id => {
                self.player.body.vel = DVec3::from(velocity);
            }
            Cb::PlayerRotation { yaw, relative_yaw, pitch, relative_pitch } => {
                let b = &mut self.player.body;
                b.yaw = if relative_yaw { b.yaw + yaw } else { yaw };
                b.pitch = if relative_pitch { b.pitch + pitch } else { pitch };
                self.player.look_override = Some((b.yaw, b.pitch));
            }
            Cb::SetTime { clocks, world_age } => {
                self.game_time = world_age;
                if let Some(c) = clocks.iter().find(|c| Some(c.id) == self.day_clock).or(clocks.first()) {
                    self.day_time = c.time as f64 + c.fraction as f64;
                    self.day_rate = c.rate;
                }
            }
            Cb::SystemChat { content, overlay } => {
                if overlay {
                    self.titles.action_bar = Some((content, Instant::now()));
                } else {
                    self.chat.push(content);
                }
            }
            Cb::PlayerChat(pc) => {
                // Whatever's signed goes on the record of what's been seen.
                if let Some(sig) = pc.signature.as_deref().and_then(|s| <[u8; 256]>::try_from(s).ok()) {
                    if let Some(count) = self.seen.add(sig) {
                        self.conn.send(play::chat_ack(count));
                    }
                }
                let content = pc.unsigned_content.clone().unwrap_or_else(|| Text::literal(pc.message.clone()));
                let msg = self.format_chat(&pc.chat_type, pc.sender_name.clone(), pc.target_name.clone(), content);
                self.chat.push_signed(msg, pc.signature.clone());
            }
            Cb::DisguisedChat { message, chat_type, sender_name, target_name } => {
                let msg = self.format_chat(&chat_type, sender_name, target_name, message);
                self.chat.push(msg);
            }
            Cb::PlayerInfoUpdate { entries, .. } => {
                for e in entries {
                    let entry = self.tab.entry(e.uuid).or_insert_with(|| TabEntry {
                        name: String::new(),
                        display: None,
                        latency: -1,
                        game_mode: 0,
                        listed: false,
                        priority: 0,
                    });
                    if let Some(n) = e.name {
                        entry.name = n;
                    }
                    for prop in e.properties.iter().flatten() {
                        if prop.name == "textures" {
                            self.wanted_skins.push((e.uuid, prop.value.clone()));
                        }
                    }
                    if let Some(d) = e.display_name {
                        entry.display = d;
                    }
                    if let Some(l) = e.latency {
                        entry.latency = l;
                    }
                    if let Some(g) = e.game_mode {
                        entry.game_mode = g;
                    }
                    if let Some(l) = e.listed {
                        entry.listed = l;
                    }
                    if let Some(p) = e.priority {
                        entry.priority = p;
                    }
                }
            }
            Cb::PlayerInfoRemove(ids) => {
                for id in ids {
                    self.tab.remove(&id);
                }
            }
            Cb::TabList { header, footer } => {
                self.tab_header = Some(header).filter(|t| !t.to_plain(tr).trim().is_empty());
                self.tab_footer = Some(footer).filter(|t| !t.to_plain(tr).trim().is_empty());
            }
            Cb::SetActionBarText(t) => self.titles.action_bar = Some((t, Instant::now())),
            Cb::SetTitleText(t) => {
                self.titles.title = Some(t);
                self.titles.age = 0;
                if self.titles.stay == 0 {
                    (self.titles.fade_in, self.titles.stay, self.titles.fade_out) = (10, 70, 20);
                }
            }
            Cb::SetSubtitleText(t) => self.titles.subtitle = Some(t),
            Cb::SetTitlesAnimation { fade_in, stay, fade_out } => {
                (self.titles.fade_in, self.titles.stay, self.titles.fade_out) = (fade_in, stay, fade_out);
            }
            Cb::ClearTitles { reset } => {
                self.titles.title = None;
                self.titles.subtitle = None;
                if reset {
                    (self.titles.fade_in, self.titles.stay, self.titles.fade_out) = (10, 70, 20);
                }
            }
            Cb::UpdateTags(tags) => {
                for (registry, list) in &tags {
                    if registry.trim_start_matches("minecraft:") == "block" {
                        self.set_block_tags(list.iter().map(|(t, ids)| (t.as_str(), ids.as_slice())));
                    }
                }
                self.extra_tags.set_tags(tags);
            }
            Cb::BlockChangedAck(seq) => tracing::debug!("server acknowledged block change {seq}"),
            // Inventories
            Cb::ContainerSetContent { container, state, slots, carried } => self.inventory.set_content(container, state, slots, carried),
            Cb::ContainerSetSlot { container, state, slot, item } => self.inventory.set_slot(container, state, slot, item),
            Cb::ContainerSetData { container, property, value } => self.inventory.set_data(container, property, value),
            Cb::SetCursorItem(item) => self.inventory.cursor = item,
            Cb::SetPlayerInventory { slot, item } => self.inventory.set_inventory_slot(slot, item),
            Cb::OpenScreen { container, menu, title } => {
                let kind = lattice_data::game().registry("menu").get(menu as usize).cloned().unwrap_or_default();
                self.inventory.open_menu(container, kind, title);
                self.screen_opened = true;
            }
            Cb::MountScreenOpen { container, columns, entity: _ } => {
                self.inventory.open_mount(container, columns);
                self.screen_opened = true;
            }
            Cb::ContainerClose(container) => {
                if self.inventory.menu.as_ref().is_some_and(|m| m.id == container) || container == 0 {
                    self.inventory.menu = None;
                    self.inventory.own_screen_open = false;
                    self.screen_closed = true;
                }
            }
            Cb::MerchantOffers { container, offers, level, xp, show_progress, .. } => {
                if let Some(m) = &mut self.inventory.menu {
                    if m.id == container {
                        let selected = m.merchant.as_ref().map_or(0, |x| x.selected);
                        m.merchant = Some(inventory::Merchant { offers, level, xp, show_progress, selected });
                    }
                }
            }
            Cb::Cooldown { group, ticks } => {
                if ticks > 0 {
                    self.inventory.cooldowns.insert(group, (ticks, ticks));
                } else {
                    self.inventory.cooldowns.remove(&group);
                }
            }
            Cb::UpdateMobEffect { id, effect, amplifier, duration, flags } if id == self.player.entity_id => {
                self.player.set_effect(effect, Some((amplifier, duration, flags)));
            }
            Cb::RemoveMobEffect { id, effect } if id == self.player.entity_id => {
                self.player.set_effect(effect, None);
            }
            Cb::SetEntityData { id, values } if id == self.player.entity_id => self.self_data(values),
            Cb::SetPassengers { vehicle, passengers } => {
                let me = self.player.entity_id;
                if passengers.contains(&me) {
                    self.player.vehicle = Some(vehicle);
                    self.player.body.passenger = true;
                } else if self.player.vehicle == Some(vehicle) {
                    self.player.vehicle = None;
                    self.player.body.passenger = false;
                    self.player.ride = None;
                    self.entities.driven = None;
                }
                self.entities.handle(Cb::SetPassengers { vehicle, passengers }, me);
            }
            Cb::Animate { id, action } if id == self.player.entity_id => {
                match action {
                    1 => self.hurt_ticks = 0,
                    0 | 3 => self.player.swing.start(action == 3, swing::NORMAL_TICKS),
                    _ => {}
                }
            }
            Cb::SwingAnimation { id, hand, duration, .. } if id == self.player.entity_id => self.player.swing.start(hand == 1, duration.max(1) as u32),
            Cb::HurtAnimation { id, .. } | Cb::DamageEvent { id, .. } if id == self.player.entity_id => self.hurt_ticks = 0,
            other => self.entities.handle(other, self.player.entity_id),
        }
    }

    fn format_chat(&self, kind: &play::ChatTypeRef, sender: Text, target: Option<Text>, content: Text) -> Text {
        let (key, params, style) = match kind {
            play::ChatTypeRef::Registry(id) => match self.registries.as_ref().and_then(|r| r.chat_types.get(*id as usize)) {
                Some(ct) => (ct.translation_key.clone(), ct.parameters.clone(), ct.style.clone()),
                None => ("chat.type.text".into(), vec!["sender".into(), "content".into()], Default::default()),
            },
            play::ChatTypeRef::Inline { translation_key, parameters, style } => (
                translation_key.clone(),
                parameters.iter().map(|p| ["sender", "target", "content"].get(*p as usize).unwrap_or(&"content").to_string()).collect(),
                Text::from_nbt(style).style,
            ),
        };
        let args = params
            .iter()
            .map(|p| match p.as_str() {
                "sender" => sender.clone(),
                "target" => target.clone().unwrap_or_else(|| Text::literal("")),
                _ => content.clone(),
            })
            .collect();
        let mut t = Text::translate(key, args);
        t.style = style;
        t
    }

    /// 20 Hz game tick.
    pub fn tick(&mut self, input: player::TickInput) {
        self.day_time += self.day_rate as f64;
        self.game_time += 1;
        self.titles.age += 1;
        self.hurt_ticks = self.hurt_ticks.saturating_add(1);
        self.ticks += 1;
        // Frozen by the server: everything but the players stays put.
        if !self.frozen {
            self.entities.tick(self.world.as_ref());
            self.block_entities.tick();
        }
        self.inventory.tick();
        let Some(world) = &mut self.world else { return };
        if self.phase == Phase::LoadingTerrain {
            let p = self.player.body.pos;
            let (cx, cz) = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
            let ready = world.has_column(cx, cz) && self.player.received_position;
            if ready || self.joined_at.elapsed().as_secs() > 30 {
                self.phase = Phase::Playing;
            }
        }
        let playing = self.phase == Phase::Playing && self.dead.is_none();
        let input = if playing { input } else { player::TickInput { yaw: input.yaw, pitch: input.pitch, sprint_window: input.sprint_window, ..Default::default() } };
        let fast_lava = world.dimension.name.ends_with("the_nether");
        let half = self.border.size / 2.0;
        self.player.border = (half < 2.9e7).then(|| [self.border.x - half, self.border.x + half, self.border.z - half, self.border.z + half]);
        self.player.tick(&self.conn, world, &mut self.inventory, &mut self.entities, input, &self.move_tags, &self.block_tags, self.vitals.food, fast_lava, playing, self.game_time);
        self.cues.append(&mut self.player.cues);
        // Starting to write in a book opens right here, with nothing asked of the server.
        if let Some((pos, state)) = self.player.open_block.take() {
            let data = self.block_entities.get(pos).and_then(|b| b.data.clone());
            let tr = lattice_proto::text::NoTranslations;
            if let Some(screen) = gm_blocks::BlockScreen::open(pos, state, data.as_ref(), &tr) {
                self.modal = Some(Modal::Block(Box::new(screen)));
                self.block_screen_filled = false;
                self.screen_opened = true;
            }
        }
        if let Some(id) = self.player.open_minecart.take() {
            if let Some(e) = self.entities.map.get(&id) {
                let tr = lattice_proto::text::NoTranslations;
                let command = match e.field("DATA_ID_COMMAND_NAME") {
                    Some(lattice_proto::item::DataValue::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let output = match e.field("DATA_ID_LAST_OUTPUT") {
                    Some(lattice_proto::item::DataValue::Text(t)) => Text::from_nbt(t).to_plain(&tr),
                    _ => String::new(),
                };
                self.modal = Some(Modal::Block(Box::new(gm_blocks::BlockScreen::minecart(id, command, output))));
                self.block_screen_filled = true;
                self.screen_opened = true;
            }
        }
        if let Some(hand) = self.player.write_book.take() {
            let stack = if hand == 0 { self.inventory.hotbar(self.player.selected_slot.clamp(0, 8) as usize) } else { self.inventory.offhand() };
            let mut pages = stack.map_or_else(Vec::new, items::writable_pages);
            if pages.is_empty() {
                pages.push(String::new());
            }
            self.modal = Some(Modal::WriteBook { slot: if hand == 0 { self.player.selected_slot.clamp(0, 8) } else { 40 }, pages, page: 0, title: String::new(), signing: false });
            self.screen_opened = true;
        }
        // Standing in a nether portal slowly swirls over the view.
        let in_portal = {
            let p = self.player.body.block_pos();
            let eye = (self.player.body.pos.y + self.player.body.eye_height() as f64).floor() as i32;
            [p[1], eye].iter().any(|y| world.block(p[0], *y, p[2]).is_some_and(|s| lattice_data::game().block(s).name == "nether_portal"))
        };
        // The view's warp turns slowly while there's anything causing it.
        let nausea = self.player.effect_duration("nausea").map_or(0.0, |d| if d < 0 { 1.0 } else { (d as f32 / 20.0).min(1.0) });
        let portal = self.portal_effect.1;
        if portal > 0.0 || nausea > 0.0 {
            self.spin_speed = (portal * 20.0 + nausea * 7.0) / (portal + nausea);
            self.spin_angle += self.spin_speed;
        } else {
            self.spin_speed = 0.0;
        }
        self.portal_effect.0 = self.portal_effect.1;
        if in_portal {
            self.portal_effect.1 = (self.portal_effect.1 + 0.0125).min(1.0);
        } else if self.portal_effect.1 > 0.0 {
            self.portal_effect.1 = (self.portal_effect.1 - 0.05).max(0.0);
        }
        // Effects run down by themselves; the server only says when they end.
        for v in self.player.effects.values_mut() {
            if v.1 > 0 {
                v.1 -= 1;
            }
        }
        // Chat keys, once they've arrived and there's a server to tell.
        if self.player.entity_id != 0 {
            if let Some(result) = self.keys_coming.as_ref().and_then(|rx| rx.try_recv().ok()) {
                self.keys_coming = None;
                match result.and_then(|keys| self.start_signing(&keys)) {
                    Ok(()) => tracing::info!("chat signing is ready"),
                    Err(e) => tracing::warn!("chat won't be signed: {e}"),
                }
            }
        }
        // A bolt lights up the whole sky while it's there.
        if self.entities.map.values().any(|e| e.type_name() == "lightning_bolt") {
            self.sky_flash = 2;
        } else if self.sky_flash > 0 {
            self.sky_flash -= 1;
        }
        for (_, bar) in &mut self.boss_bars {
            bar.shown += (bar.progress - bar.shown) * 0.5;
        }
        // A boss can darken the world, which comes on faster than it fades.
        self.boss_darkness_prev = self.boss_darkness;
        self.boss_darkness = if self.boss_bars.iter().any(|(_, b)| b.flags & 1 != 0) { (self.boss_darkness + 0.05).min(1.0) } else { (self.boss_darkness - 0.0125).max(0.0) };
        if let Some(me) = self.tab.get(&self.self_uuid) {
            let name = me.name.clone();
            self.scoreboard.set_own_name(&name);
        }
        if self.border.lerp_ticks > 0.0 {
            self.border.size += (self.border.target - self.border.size) / self.border.lerp_ticks;
            self.border.lerp_ticks -= 1.0;
        }
    }

    /// A steadily counting clock, for slow animations such as banners.
    pub fn game_time(&self) -> i64 {
        self.ticks
    }

    pub fn self_uuid(&self) -> Uuid {
        self.profile_id
    }

    /// Which skin layers the local player's body shows.
    pub fn self_customisation(&self) -> i32 {
        0x7F
    }

    pub fn send_chat(&mut self, line: &str) -> Result<(), String> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        if let Some(cmd) = line.strip_prefix('/') {
            self.conn.send(play::chat_command(cmd));
            return Ok(());
        }
        if self.secure_chat_required {
            return Err("This server only accepts signed chat messages, which Lattice can't send yet. Commands still work.".into());
        }
        let msg: String = line.chars().take(256).collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        self.conn.send(play::chat(&msg, now, rand::random(), None, 0, [0; 3], 1));
        Ok(())
    }

    fn start_signing(&mut self, keys: &lattice_auth::ProfileKeys) -> Result<(), String> {
        use base64::Engine;
        let private = lattice_proto::signing::private_key_from_pem(&keys.private_key_pem)?;
        let public = lattice_proto::signing::der_from_pem(&keys.public_key_pem)?;
        let signature = base64::engine::general_purpose::STANDARD.decode(keys.key_signature.trim()).map_err(|e| e.to_string())?;
        let expires = iso_millis(&keys.expires_at).ok_or("the keys' expiry date couldn't be read")?;
        let session = Uuid::new_v4();
        self.conn.send(play::chat_session_update(session, expires, &public, &signature));
        self.signer = Some(lattice_proto::signing::ChatSigner::new(private, self.self_uuid, session));
        Ok(())
    }

    pub fn accept_code_of_conduct(&mut self) {
        self.conn.send_config(lattice_proto::packets::config::accept_code_of_conduct());
        self.code_of_conduct = None;
    }

    /// The name this player goes by, as the tab list has it.
    pub fn player_name(&self) -> String {
        self.tab.get(&self.self_uuid()).map_or_else(|| "you".to_string(), |p| p.name.clone())
    }

    pub fn respawn(&mut self) {
        self.conn.send(play::client_command(0));
    }

    /// The biome at a block position, if that column is loaded.
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> Option<&lattice_world::registries::Biome> {
        let (world, reg) = (self.world.as_ref()?, self.registries.as_ref()?);
        let col = world.column(x >> 4, z >> 4)?;
        let s = ((y - world.geometry.min_y) >> 4).clamp(0, col.sections.len() as i32 - 1) as usize;
        let id = col.sections[s].biomes.get(((((y & 15) >> 2) << 4) | (((z & 15) >> 2) << 2) | ((x & 15) >> 2)) as usize);
        reg.biomes.get(id as usize)
    }

    /// Hours/minutes-free day progress: 0 = sunrise, 0.25 = noon, 0.5 = sunset.
    pub fn day_fraction(&self) -> f64 {
        (self.day_time / 24000.0).rem_euclid(1.0)
    }
}

