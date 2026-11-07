//! Settings, saved servers and accounts on disk.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn detect() -> Paths {
        if let Some(root) = std::env::var_os("LATTICE_HOME") {
            let root = PathBuf::from(root);
            return Paths { config: root.join("config"), cache: root.join("cache"), data: root.join("data") };
        }
        let d = directories::ProjectDirs::from("dev", "lattice", "lattice").expect("no home directory");
        // This client used to be called Patina: bring sign-ins, settings and
        // worlds along the first time it starts under the new name.
        if let Some(old) = directories::ProjectDirs::from("dev", "patina", "patina") {
            adopt_old_folder(old.config_dir(), d.config_dir());
            adopt_old_folder(old.data_dir(), d.data_dir());
            adopt_old_folder(old.cache_dir(), d.cache_dir());
        }
        Paths { config: d.config_dir().to_path_buf(), cache: d.cache_dir().to_path_buf(), data: d.data_dir().to_path_buf() }
    }

    pub fn ensure(&self) {
        for p in [&self.config, &self.cache, &self.data] {
            let _ = std::fs::create_dir_all(p);
        }
    }
}

/// Moves `old` to `new`, unless `new` already exists. A failure only means
/// starting with empty settings, so it's reported and not fatal.
fn adopt_old_folder(old: &Path, new: &Path) {
    if !old.is_dir() || new.exists() {
        return;
    }
    if let Some(parent) = new.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::rename(old, new) {
        eprintln!("Couldn't move {} to {}: {e}", old.display(), new.display());
    }
}

/// Writes via a temporary file so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    std::fs::rename(&tmp, path)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub video: Video,
    pub controls: Controls,
    pub audio: Audio,
    pub chat: Chat,
    pub player: Player,
    pub auth: Auth,
    pub interface: Interface,
    pub debug: Debug,
    pub worlds: Worlds,
}

/// Worlds kept on this computer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Worlds {
    /// Memory a running world may use.
    pub memory_mb: u32,
    /// The license of the server program has been accepted.
    pub eula_accepted: bool,
}

impl Default for Worlds {
    fn default() -> Self {
        Worlds { memory_mb: 3072, eula_accepted: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    /// Chunks, 2–32.
    pub render_distance: i32,
    /// Percent of the render distance entities are drawn within.
    pub entity_distance: i32,
    pub entity_shadows: bool,
    pub clouds: bool,
    pub weather: bool,
    pub particles: bool,
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// Percent of the sprint and speed effect on the field of view.
    pub fov_effects: i32,
    /// Darkens the edges of the screen in the dark.
    pub vignette: bool,
    /// How far out clouds are drawn, in chunks.
    pub cloud_range: i32,
    /// How far around you rain and snow fall, in blocks.
    pub weather_radius: i32,
    /// Percent of the warping from nausea and portals.
    pub distortion: i32,
    /// The sky doesn't flash with lightning.
    pub no_lightning_flash: bool,
    /// The outline of the block you're looking at stands out more.
    pub bold_outline: bool,
    /// How fast and how bright the shine on enchanted things is, percent.
    pub glint_speed: i32,
    pub glint_strength: i32,
    pub view_bobbing: bool,
    /// Percent of the camera tip when hurt.
    pub damage_tilt: i32,
    /// 0 moody to 1 bright.
    pub brightness: f32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// 0 is no limit.
    pub max_fps: i32,
    /// Live frame rate in the corner of the game window.
    pub show_fps: bool,
    /// Windows without focus draw at a lower rate, so side-by-side sessions
    /// don't take GPU time from the one being played.
    pub limit_background_fps: bool,
    pub background_fps: i32,
    pub smooth_lighting: bool,
    /// Percent strength of corner shading.
    pub ambient_occlusion: i32,
    /// Biome colour blend radius, in steps of four blocks.
    pub biome_blend: i32,
    pub detailed_leaves: bool,
    pub mipmap_levels: i32,
    /// 0 off, then 2×, 4×, 8×, 16×.
    pub anisotropy: i32,
    pub fog: bool,
    /// Percent of the render distance where fog starts.
    pub fog_start: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Controls {
    /// Degrees of rotation per pixel of mouse movement.
    pub sensitivity: f32,
    pub invert_y: bool,
    pub invert_x: bool,
    pub wheel_sensitivity: f32,
    /// The sprint key sprints while held (false: it toggles).
    pub hold_to_sprint: bool,
    /// The sneak key sneaks while held (false: it toggles).
    pub hold_to_sneak: bool,
    /// The attack and use buttons work while held (false: they toggle).
    pub hold_to_attack: bool,
    pub hold_to_use: bool,
    /// Each notch of the wheel moves one slot, however fast a wheel reports it.
    pub whole_wheel_steps: bool,
    pub auto_jump: bool,
    /// Ticks allowed between two taps of forward to start sprinting (0 turns it off).
    pub sprint_window: i32,
    /// Action name to binding; actions not listed use the default.
    pub binds: std::collections::BTreeMap<String, String>,
}

/// Volumes, 0 to 1, by the game's sound channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub master: f32,
    pub music: f32,
    pub records: f32,
    pub weather: f32,
    pub blocks: f32,
    pub hostile: f32,
    pub neutral: f32,
    pub players: f32,
    pub ambient: f32,
    pub voice: f32,
    pub ui: f32,
    pub subtitles: bool,
    /// 0 usual, 1 frequent, 2 all the time.
    pub music_frequency: i32,
    /// Which output to use; empty means the system's default.
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    /// 0 shown, 1 commands only, 2 hidden. The server is told.
    pub mode: i32,
    pub colors: bool,
    /// Seconds a line stays on screen after it arrives.
    pub fade_after: f32,
    pub lines_closed: i32,
    pub lines_open: i32,
    pub width: i32,
    pub text_size: f32,
    /// Percent.
    pub opacity: i32,
    /// Percent.
    pub background: i32,
    pub line_spacing: f32,
    pub text_filtering: bool,
    pub command_suggestions: bool,
    /// Links in chat can be clicked.
    pub links: bool,
    /// Asks before opening one.
    pub link_prompt: bool,
    /// Seconds new lines wait before they show.
    pub delay: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
    /// Selected name for offline-mode servers (empty: derive one from the system user).
    pub offline_name: String,
    /// Other saved offline names.
    pub offline_names: Vec<String>,
    /// 0 left, 1 right.
    pub main_hand: i32,
    pub locale: String,
    pub skin_cape: bool,
    pub skin_jacket: bool,
    pub skin_left_sleeve: bool,
    pub skin_right_sleeve: bool,
    pub skin_left_leg: bool,
    pub skin_right_leg: bool,
    pub skin_hat: bool,
    pub allow_listing: bool,
    /// What to do when a server offers a resource pack: 0 use it, 1 decline it.
    pub server_packs: i32,
    /// The player's own packs in use, lowest first, by file name.
    pub resource_packs: Vec<String>,
}

impl Player {
    /// The bit field the protocol uses for skin layers.
    pub fn skin_parts(&self) -> u8 {
        [self.skin_cape, self.skin_jacket, self.skin_left_sleeve, self.skin_right_sleeve, self.skin_left_leg, self.skin_right_leg, self.skin_hat]
            .iter()
            .enumerate()
            .fold(0, |acc, (i, on)| if *on { acc | 1 << i } else { acc })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Auth {
    /// Azure application ID approved for Minecraft sign-in.
    pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    pub scale: f32,
    /// "system", "dark", "light" or "high_contrast".
    pub theme: String,
    /// Menus snap instead of animating, and nothing in them moves by itself.
    pub reduce_motion: bool,
    /// Game windows use the blocky pixel face; off, the plainer one the launcher uses.
    pub pixel_font: bool,
    /// The launcher looks for a newer Lattice when it starts.
    pub check_updates: bool,
    /// Bar under the crosshair showing how charged the next attack is.
    pub attack_indicator: bool,
    pub show_coordinates: bool,
    /// Seconds the held item's name shows when it changes.
    pub held_name_time: f32,
    /// Seconds Lattice's own messages stay on screen.
    pub toast_time: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Debug {
    pub hitboxes: bool,
    pub chunk_borders: bool,
}

impl Default for Video {
    fn default() -> Self {
        Video {
            render_distance: 10,
            entity_distance: 100,
            entity_shadows: true,
            clouds: true,
            weather: true,
            particles: true,
            fov: 70.0,
            fov_effects: 100,
            vignette: true,
            cloud_range: 24,
            weather_radius: 10,
            distortion: 100,
            no_lightning_flash: false,
            bold_outline: false,
            glint_speed: 50,
            glint_strength: 75,
            view_bobbing: true,
            damage_tilt: 100,
            brightness: 0.5,
            vsync: true,
            fullscreen: false,
            max_fps: 0,
            show_fps: true,
            limit_background_fps: true,
            background_fps: 30,
            smooth_lighting: true,
            ambient_occlusion: 100,
            biome_blend: 1,
            detailed_leaves: true,
            mipmap_levels: 4,
            anisotropy: 0,
            fog: true,
            fog_start: 72,
        }
    }
}
impl Default for Controls {
    fn default() -> Self {
        Controls {
            sensitivity: 0.12,
            invert_y: false,
            invert_x: false,
            wheel_sensitivity: 1.0,
            hold_to_sprint: true,
            hold_to_sneak: true,
            hold_to_attack: true,
            hold_to_use: true,
            whole_wheel_steps: false,
            auto_jump: false,
            sprint_window: 7,
            binds: Default::default(),
        }
    }
}
impl Default for Audio {
    fn default() -> Self {
        Audio { master: 1.0, music: 1.0, records: 1.0, weather: 1.0, blocks: 1.0, hostile: 1.0, neutral: 1.0, players: 1.0, ambient: 1.0, voice: 1.0, ui: 1.0, subtitles: false, music_frequency: 0, device: String::new() }
    }
}
impl Default for Chat {
    fn default() -> Self {
        Chat {
            mode: 0,
            colors: true,
            fade_after: 10.0,
            lines_closed: 10,
            lines_open: 20,
            width: 520,
            text_size: 15.0,
            opacity: 100,
            background: 55,
            line_spacing: 0.0,
            text_filtering: false,
            command_suggestions: true,
            links: true,
            link_prompt: true,
            delay: 0.0,
        }
    }
}
impl Default for Player {
    fn default() -> Self {
        Player {
            offline_name: String::new(),
            offline_names: Vec::new(),
            main_hand: 1,
            locale: "en_us".into(),
            skin_cape: true,
            skin_jacket: true,
            skin_left_sleeve: true,
            skin_right_sleeve: true,
            skin_left_leg: true,
            skin_right_leg: true,
            skin_hat: true,
            allow_listing: true,
            server_packs: 0,
            resource_packs: Vec::new(),
        }
    }
}
impl Default for Interface {
    fn default() -> Self {
        Interface { scale: 1.0, theme: "system".into(), reduce_motion: false, pixel_font: true, check_updates: true, attack_indicator: true, show_coordinates: false, held_name_time: 2.5, toast_time: 6.0 }
    }
}

impl Config {
    pub fn load(dir: &Path) -> Config {
        let path = dir.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("config.toml is invalid ({e}); using defaults for unreadable values");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Saves, keeping keys this version doesn't know about.
    pub fn save(&self, dir: &Path) {
        let path = dir.join("config.toml");
        let mut table: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
        if let Ok(toml::Value::Table(ours)) = toml::Value::try_from(self) {
            merge(&mut table, ours);
        }
        if let Ok(text) = toml::to_string_pretty(&table) {
            if let Err(e) = write_atomic(&path, text.as_bytes(), false) {
                tracing::warn!("couldn't save settings: {e}");
            }
        }
    }

    /// The name used for offline play right now.
    pub fn offline_name(&self) -> String {
        let n = self.player.offline_name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        let cleaned: String = user.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(16).collect();
        if cleaned.len() >= 3 { cleaned } else { "Player".into() }
    }

    /// Every saved offline name, the selected one included.
    pub fn all_offline_names(&self) -> Vec<String> {
        let mut names = vec![self.offline_name()];
        for n in &self.player.offline_names {
            if !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                names.push(n.clone());
            }
        }
        names
    }

    /// The application ID from the environment overrides the file.
    pub fn client_id(&self) -> String {
        std::env::var("LATTICE_CLIENT_ID").unwrap_or_else(|_| self.auth.client_id.clone())
    }
}

fn merge(base: &mut toml::Table, ours: toml::Table) {
    for (k, v) in ours {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedServer {
    pub name: String,
    pub address: String,
    /// Unix seconds of the last successful join.
    #[serde(default)]
    pub last_joined: Option<i64>,
}

pub fn load_servers(dir: &Path) -> Vec<SavedServer> {
    std::fs::read(dir.join("servers.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_servers(dir: &Path, servers: &[SavedServer]) {
    if let Ok(b) = serde_json::to_vec_pretty(servers) {
        if let Err(e) = write_atomic(&dir.join("servers.json"), &b, false) {
            tracing::warn!("couldn't save the server list: {e}");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub uuid: Uuid,
    pub name: String,
    pub refresh_token: String,
    #[serde(default)]
    pub skin_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Accounts {
    pub active: Option<Uuid>,
    pub accounts: Vec<StoredAccount>,
}

impl Accounts {
    pub fn load(dir: &Path) -> Accounts {
        std::fs::read(dir.join("accounts.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Stored with owner-only permissions; it contains refresh tokens.
    pub fn save(&self, dir: &Path) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            if let Err(e) = write_atomic(&dir.join("accounts.json"), &b, true) {
                tracing::warn!("couldn't save accounts: {e}");
            }
        }
    }

    pub fn active(&self) -> Option<&StoredAccount> {
        let id = self.active?;
        self.accounts.iter().find(|a| a.uuid == id)
    }

    pub fn upsert(&mut self, acc: StoredAccount) {
        self.active = Some(acc.uuid);
        match self.accounts.iter_mut().find(|a| a.uuid == acc.uuid) {
            Some(a) => *a = acc,
            None => self.accounts.push(acc),
        }
    }

    pub fn remove(&mut self, id: Uuid) {
        self.accounts.retain(|a| a.uuid != id);
        if self.active == Some(id) {
            self.active = self.accounts.first().map(|a| a.uuid);
        }
    }
}

/// Whether a server would accept this as a player name.
pub fn valid_player_name(n: &str) -> bool {
    (3..=16).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_names() {
        assert!(valid_player_name("Alex_2"));
        assert!(!valid_player_name("ab"));
        assert!(!valid_player_name("seventeen_chars_x"));
        assert!(!valid_player_name("no spaces"));
    }

    #[test]
    fn old_folders_move_to_the_new_name_once() {
        let root = std::env::temp_dir().join(format!("lattice-move-{}", std::process::id()));
        let (old, new) = (root.join("patina"), root.join("lattice"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("accounts.json"), "{}").unwrap();
        adopt_old_folder(&old, &new);
        assert!(new.join("accounts.json").exists());
        assert!(!old.exists());
        // Never over the top of what's already there.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("accounts.json"), "older").unwrap();
        adopt_old_folder(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("accounts.json")).unwrap(), "{}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_keys_survive_save() {
        let dir = std::env::temp_dir().join(format!("lattice-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "future_key = 1\n[video]\nfov = 90.0\nfuture_video = \"x\"\n").unwrap();
        let c = Config::load(&dir);
        assert_eq!(c.video.fov, 90.0);
        assert_eq!(c.video.render_distance, 10);
        c.save(&dir);
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(text.contains("future_key"));
        assert!(text.contains("future_video"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
