//! The launcher: stays open beside the game, keeps accounts and servers, and
//! starts game windows as separate processes. It draws only when something
//! changes, so it costs next to nothing while you play.

pub mod accounts;
pub mod home;
pub mod panel;
pub mod sessions;
pub mod update;
pub mod worlds;

use crate::assets::{LoadProgress, ensure_files};
use crate::config::{Accounts, Config, Paths, SavedServer, StoredAccount};
use crate::ipc::{Control, LaunchSpec, SpecProfile};
use crate::ui::{self, Toast};
use crossbeam_channel::{Receiver, Sender, unbounded};
use lattice_net::{ServerAddress, ServerStatus};
use lattice_render::{EguiFrame, Renderer};
use sessions::{Exit, SessionProc, free_name};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use crate::a11y::UserEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

/// How often saved servers are asked for their status.
const PING_EVERY: Duration = Duration::from_secs(20);

pub enum PingState {
    Pending,
    Done(ServerStatus),
    Failed(String),
}

pub enum AuthUpdate {
    Code(lattice_auth::DeviceCode),
    Step(lattice_auth::Step),
    Done(lattice_auth::Session, String),
    Failed(lattice_auth::AuthError),
}

pub struct SignInFlow {
    pub rx: Receiver<AuthUpdate>,
    pub cancel: Arc<AtomicBool>,
    pub code: Option<(lattice_auth::DeviceCode, Instant)>,
    pub step: Option<lattice_auth::Step>,
    pub error: Option<String>,
}

/// A session waiting for a fresh Minecraft token.
pub struct PendingLaunch {
    pub server_name: String,
    pub address: String,
    pub account: String,
    rx: Receiver<Result<(lattice_auth::Session, String), lattice_auth::AuthError>>,
}

pub enum Files {
    Preparing(Arc<Mutex<LoadProgress>>, Receiver<Result<(), String>>),
    Ready,
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub enum Screen {
    Play,
    NewWorld(worlds::Form),
    EditServer { index: Option<usize>, name: String, address: String, error: Option<String> },
    Sessions,
    Accounts,
    Settings,
}

/// Who a new session plays as.
#[derive(Clone)]
pub enum Identity {
    Microsoft(StoredAccount),
    Offline(String),
}

/// What pressing Play would do right now.
pub struct Plan {
    pub name: String,
    pub label: &'static str,
    pub note: Option<String>,
}

pub struct Launcher {
    /// A newer Lattice, if there is one.
    pub updater: update::Updater,
    /// Worlds on this computer, and the one running if any.
    pub worlds: Vec<worlds::World>,
    pub host: Option<worlds::Host>,
    host_had_session: bool,
    pub window: Arc<Window>,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    pub paths: Paths,
    pub config: Config,
    pub settings_panel: ui::settings::PanelState,
    config_modified: Option<SystemTime>,
    pub servers: Vec<SavedServer>,
    pub accounts: Accounts,
    pub mc_sessions: HashMap<Uuid, lattice_auth::Session>,
    pub screen: Screen,
    /// The look now applied, so it only changes when something asks for it.
    applied_theme: Option<(ui::theme::Mode, bool)>,
    pub files: Files,
    pub pings: HashMap<String, PingState>,
    ping_rx: Receiver<(String, Result<ServerStatus, String>)>,
    ping_tx: Sender<(String, Result<ServerStatus, String>)>,
    last_ping: Instant,
    pub direct_address: String,
    pub sessions: Vec<SessionProc>,
    pub selected_session: Option<u64>,
    next_session: u64,
    pub pending: Vec<PendingLaunch>,
    pub sign_in: Option<SignInFlow>,
    pub toasts: Vec<Toast>,
    pub textures: HashMap<String, egui::TextureHandle>,
    face_rx: Receiver<(Uuid, Option<image::RgbaImage>)>,
    face_tx: Sender<(Uuid, Option<image::RgbaImage>)>,
    faces_requested: HashSet<Uuid>,
    repaint_at: Instant,
    dirty: bool,
    last_config_check: Instant,
    /// Development aids (see docs/DESIGN.md §13).
    shots: Vec<(f32, PathBuf)>,
    exit_after: Option<f32>,
    autolaunch: Vec<String>,
    started: Instant,
    exit: bool,
}

fn config_mtime(paths: &Paths) -> Option<SystemTime> {
    std::fs::metadata(paths.config.join("config.toml")).and_then(|m| m.modified()).ok()
}

pub fn address_key(address: &str) -> String {
    ServerAddress::parse(address).map(|a| a.display()).unwrap_or_else(|_| address.trim().to_string()).to_lowercase()
}

fn fetch_face(url: &str) -> Option<image::RgbaImage> {
    let url = url.replace("http://", "https://");
    let mut r = ureq::get(&url).call().ok()?;
    let bytes = r.body_mut().with_config().limit(1 << 20).read_to_vec().ok()?;
    let skin = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let mut face = image::imageops::crop_imm(&skin, 8, 8, 8, 8).to_image();
    if skin.height() >= 64 {
        let hat = image::imageops::crop_imm(&skin, 40, 8, 8, 8).to_image();
        image::imageops::overlay(&mut face, &hat, 0, 0);
    }
    Some(face)
}

impl Launcher {
    pub fn new(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<UserEvent>, paths: Paths, config: Config) -> anyhow::Result<Launcher> {
        let attrs = crate::desktop::identify(crate::desktop::use_launch_token(event_loop, Window::default_attributes()))
            .with_title("Lattice")
            .with_visible(false)
            .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 760.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(860.0, 560.0));
        // Development aid: open at a given size, to check how pages fit.
        let attrs = match std::env::var("LATTICE_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?)))) {
            Some((w, h)) => attrs.with_inner_size(winit::dpi::LogicalSize::new(w, h)),
            None => attrs,
        };
        let window = Arc::new(event_loop.create_window(attrs)?);
        let size = window.inner_size();
        let display: OwnedDisplayHandle = event_loop.owned_display_handle();
        let renderer = Renderer::new(window.clone(), Some(Box::new(display)), size.width, size.height, true, true).map_err(|e| anyhow::anyhow!("{e}"))?;
        let egui_ctx = egui::Context::default();
        ui::theme::install(&egui_ctx);
        egui_ctx.set_zoom_factor(config.interface.scale);
        let mut egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(renderer.device.limits().max_texture_dimension_2d as usize),
        );
        // Has to happen before the window is first shown.
        egui_state.init_accesskit(event_loop, &window, proxy);
        window.set_visible(true);
        if std::env::var_os("LATTICE_A11Y_DUMP").is_some() {
            egui_ctx.enable_accesskit();
        }

        // Fetch the game files here, so several sessions never race to download them.
        let progress = Arc::new(Mutex::new(LoadProgress { step: "Checking game files".into(), fraction: None }));
        let (done_tx, done_rx) = unbounded();
        {
            let (cache, progress) = (paths.cache.clone(), progress.clone());
            std::thread::Builder::new().name("files".into()).spawn(move || {
                let _ = done_tx.send(ensure_files(&cache, &progress).map(|_| ()));
            })?;
        }

        let servers = crate::config::load_servers(&paths.config);
        let accounts = Accounts::load(&paths.config);
        let (ping_tx, ping_rx) = unbounded();
        let (face_tx, face_rx) = unbounded();
        let config_modified = config_mtime(&paths);
        let paths_for_worlds = paths.clone();
        let mut app = Launcher {
            updater: {
                let mut u = update::Updater::new();
                if config.interface.check_updates {
                    u.check();
                }
                u
            },
            window,
            renderer,
            egui_ctx,
            egui_state,
            paths,
            config,
            settings_panel: ui::settings::PanelState::default(),
            config_modified,
            worlds: worlds::list(&paths_for_worlds),
            host: None,
            host_had_session: false,
            servers,
            accounts,
            mc_sessions: HashMap::new(),
            screen: Screen::Play,
            applied_theme: None,
            files: Files::Preparing(progress, done_rx),
            pings: HashMap::new(),
            ping_rx,
            ping_tx,
            last_ping: Instant::now(),
            direct_address: String::new(),
            sessions: Vec::new(),
            selected_session: None,
            next_session: 1,
            pending: Vec::new(),
            sign_in: None,
            toasts: Vec::new(),
            textures: HashMap::new(),
            face_rx,
            face_tx,
            faces_requested: HashSet::new(),
            repaint_at: Instant::now(),
            dirty: true,
            last_config_check: Instant::now(),
            shots: crate::dev::timed_paths("LATTICE_SHOTS"),
            exit_after: crate::dev::seconds("LATTICE_EXIT_AFTER"),
            autolaunch: std::env::var("LATTICE_AUTOLAUNCH").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default(),
            started: Instant::now(),
            exit: false,
        };
        app.ping_all();
        match std::env::var("LATTICE_SCREEN").as_deref() {
            Ok("accounts") => app.screen = Screen::Accounts,
            Ok("settings") => app.screen = Screen::Settings,
            Ok("sessions") => app.screen = Screen::Sessions,
            Ok("add_server") => app.screen = Screen::EditServer { index: None, name: String::new(), address: String::new(), error: None },
            Ok("new_world") => app.screen = Screen::NewWorld(worlds::Form::default()),
            _ => {}
        }
        Ok(app)
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, at: Instant::now() });
        self.dirty = true;
    }

    pub fn files_ready(&self) -> bool {
        matches!(self.files, Files::Ready)
    }

    // -------------------------------------------------------------- pings

    pub fn ping_all(&mut self) {
        self.last_ping = Instant::now();
        let addrs: Vec<String> = self.servers.iter().map(|s| s.address.clone()).collect();
        for a in addrs {
            self.ping(a);
        }
    }

    pub fn ping(&mut self, address: String) {
        let Ok(parsed) = ServerAddress::parse(&address) else {
            self.pings.insert(address, PingState::Failed("The address isn't valid.".into()));
            return;
        };
        // Keep showing the last result while a refresh is on its way.
        self.pings.entry(address.clone()).or_insert(PingState::Pending);
        let tx = self.ping_tx.clone();
        std::thread::spawn(move || {
            let r = lattice_net::ping(&parsed, Duration::from_secs(6));
            let _ = tx.send((address, r));
        });
    }

    // -------------------------------------------------------------- accounts

    pub fn face_texture(&mut self, uuid: Uuid, skin_url: Option<&str>) -> Option<egui::TextureHandle> {
        let key = format!("face:{uuid}");
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        if let (Some(url), true) = (skin_url, self.faces_requested.insert(uuid)) {
            let (url, tx) = (url.to_string(), self.face_tx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((uuid, fetch_face(&url)));
            });
        }
        None
    }

    pub fn start_sign_in(&mut self) {
        let client_id = self.config.client_id();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            let send = |u| {
                let _ = tx.send(u);
            };
            let code = match lattice_auth::start_device_code(&client_id) {
                Ok(c) => c,
                Err(e) => return send(AuthUpdate::Failed(e)),
            };
            send(AuthUpdate::Code(code.clone()));
            let mut interval = code.interval.max(1);
            let deadline = Instant::now() + Duration::from_secs(code.expires_in);
            let tokens = loop {
                for _ in 0..interval * 10 {
                    if c2.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if Instant::now() > deadline {
                    return send(AuthUpdate::Failed(lattice_auth::AuthError::Expired));
                }
                match lattice_auth::poll_device_code(&client_id, &code) {
                    Ok(lattice_auth::Poll::Pending) => {}
                    Ok(lattice_auth::Poll::SlowDown) => interval += 5,
                    Ok(lattice_auth::Poll::Done(t)) => break t,
                    Err(e) => return send(AuthUpdate::Failed(e)),
                }
            };
            match lattice_auth::minecraft_session(&tokens.access_token, &|s| send(AuthUpdate::Step(s))) {
                Ok(session) => send(AuthUpdate::Done(session, tokens.refresh_token)),
                Err(e) => send(AuthUpdate::Failed(e)),
            }
        });
        self.sign_in = Some(SignInFlow { rx, cancel, code: None, step: None, error: None });
    }

    // -------------------------------------------------------------- launching

    pub fn identity(&self) -> Identity {
        match self.accounts.active() {
            Some(a) => Identity::Microsoft(a.clone()),
            None => Identity::Offline(self.config.offline_name()),
        }
    }

    fn names_on(&self, key: &str) -> Vec<String> {
        self.sessions.iter().filter(|s| s.on_server() && s.address_key == key).map(|s| s.name().to_string()).collect()
    }

    pub fn sessions_on(&self, address: &str) -> usize {
        let key = address_key(address);
        self.sessions.iter().filter(|s| s.on_server() && s.address_key == key).count()
    }

    pub fn plan(&self, address: &str) -> Plan {
        let used = self.names_on(&address_key(address));
        match self.identity() {
            Identity::Microsoft(a) => {
                let note = used
                    .iter()
                    .any(|u| u.eq_ignore_ascii_case(&a.name))
                    .then(|| format!("{} is already on this server. A second session with the same account disconnects the first.", a.name));
                Plan { name: a.name, label: "Play", note }
            }
            Identity::Offline(base) => {
                let name = free_name(&base, &used);
                if name == base {
                    Plan { name, label: "Play", note: None }
                } else {
                    let note = format!("{base} is already on this server, so this session joins as {name}.");
                    Plan { name, label: "Open another session", note: Some(note) }
                }
            }
        }
    }

    pub fn launch(&mut self, server_name: String, address: String) {
        let address = address.trim().to_string();
        if let Err(e) = ServerAddress::parse(&address) {
            self.toast(e, true);
            return;
        }
        if !self.files_ready() {
            self.toast("The game files are still being prepared. Try again in a moment.", true);
            return;
        }
        match self.identity() {
            Identity::Offline(_) => {
                let name = self.plan(&address).name;
                self.spawn(server_name, address, SpecProfile::Offline { name });
            }
            Identity::Microsoft(acc) => {
                if let Some(profile) = self.online_profile(acc.uuid) {
                    self.spawn(server_name, address, profile);
                    return;
                }
                // The Minecraft token is missing or about to expire: get a new one first.
                let (tx, rx) = unbounded();
                let client_id = self.config.client_id();
                let (refresh, account) = (acc.refresh_token.clone(), acc.name.clone());
                std::thread::spawn(move || {
                    let r = lattice_auth::refresh(&client_id, &refresh)
                        .and_then(|t| lattice_auth::minecraft_session(&t.access_token, &|_| {}).map(|s| (s, t.refresh_token)));
                    let _ = tx.send(r);
                });
                self.pending.push(PendingLaunch { server_name, address, account, rx });
            }
        }
    }

    fn online_profile(&self, uuid: Uuid) -> Option<SpecProfile> {
        let s = self.mc_sessions.get(&uuid).filter(|s| !s.needs_refresh())?;
        Some(SpecProfile::Online { name: s.profile.name.clone(), uuid, access_token: s.access_token.clone() })
    }

    fn spawn(&mut self, server_name: String, address: String, profile: SpecProfile) {
        let id = self.next_session;
        self.next_session += 1;
        let key = address_key(&address);
        let spec = LaunchSpec { server_name, address, profile, reports: true };
        match SessionProc::spawn(id, spec, key) {
            Ok(s) => {
                self.selected_session = Some(id);
                self.sessions.push(s);
            }
            Err(e) => self.toast(format!("Couldn't start a game window: {e}"), true),
        }
    }

    /// Starts a finished or failed session again with the same identity.
    pub fn restart(&mut self, id: u64) {
        let Some(i) = self.sessions.iter().position(|s| s.id == id) else { return };
        let s = self.sessions.remove(i);
        let (server, address) = (s.spec.server_name.clone(), s.spec.address.clone());
        match s.spec.profile.clone() {
            SpecProfile::Offline { name } => self.spawn(server, address, SpecProfile::Offline { name }),
            SpecProfile::Online { uuid, .. } => {
                let active = self.accounts.active.replace(uuid);
                self.launch(server, address);
                self.accounts.active = active;
            }
        }
    }

    pub fn dismiss(&mut self, id: u64) {
        self.sessions.retain(|s| s.id != id);
    }

    pub fn stop(&mut self, id: u64) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.stop();
        }
    }

    pub fn reload_running_sessions(&mut self) {
        for s in self.sessions.iter_mut().filter(|s| s.running()) {
            s.send(Control::ReloadConfig);
        }
    }

    pub fn save_config(&mut self) {
        self.config.save(&self.paths.config);
        self.config_modified = config_mtime(&self.paths);
    }

    // -------------------------------------------------------------- background work

    fn pump(&mut self) {
        while let Ok((addr, r)) = self.ping_rx.try_recv() {
            self.dirty = true;
            self.pings.insert(addr, match r {
                Ok(s) => PingState::Done(s),
                Err(e) => PingState::Failed(e),
            });
        }
        while let Ok((uuid, img)) = self.face_rx.try_recv() {
            if let Some(img) = img {
                let ci = egui::ColorImage::from_rgba_unmultiplied([8, 8], img.as_raw());
                let t = self.egui_ctx.load_texture(format!("face:{uuid}"), ci, egui::TextureOptions::NEAREST);
                self.textures.insert(format!("face:{uuid}"), t);
                self.dirty = true;
            }
        }
        self.pump_sign_in();
        self.pump_pending();
        self.pump_files();
        self.pump_sessions();
        self.pump_world();
        if self.last_ping.elapsed() > PING_EVERY {
            self.ping_all();
        }
        // Settings may have been changed from a game window.
        if self.last_config_check.elapsed() > Duration::from_secs(1) {
            self.last_config_check = Instant::now();
            let now = config_mtime(&self.paths);
            if now != self.config_modified && self.screen != Screen::Settings {
                self.config_modified = now;
                self.config = Config::load(&self.paths.config);
                self.dirty = true;
            }
        }
    }

    fn pump_sign_in(&mut self) {
        let Some(flow) = &mut self.sign_in else { return };
        let mut finished = None;
        while let Ok(u) = flow.rx.try_recv() {
            self.dirty = true;
            match u {
                AuthUpdate::Code(c) => flow.code = Some((c, Instant::now())),
                AuthUpdate::Step(s) => flow.step = Some(s),
                AuthUpdate::Failed(e) => flow.error = Some(e.to_string()),
                AuthUpdate::Done(session, refresh) => {
                    finished = Some((session, refresh));
                    break;
                }
            }
        }
        if let Some((session, refresh)) = finished {
            let p = session.profile.clone();
            self.accounts.upsert(StoredAccount { uuid: p.id, name: p.name.clone(), refresh_token: refresh, skin_url: p.skin_url.clone() });
            self.accounts.save(&self.paths.config);
            self.mc_sessions.insert(p.id, session);
            self.sign_in = None;
            self.toast(format!("Signed in as {}", p.name), false);
        }
    }

    fn pump_pending(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            let Ok(r) = self.pending[i].rx.try_recv() else {
                i += 1;
                continue;
            };
            let p = self.pending.remove(i);
            match r {
                Ok((session, refresh)) => {
                    let id = session.profile.id;
                    if let Some(a) = self.accounts.accounts.iter_mut().find(|a| a.uuid == id) {
                        a.refresh_token = refresh;
                        a.name = session.profile.name.clone();
                    }
                    self.accounts.save(&self.paths.config);
                    self.mc_sessions.insert(id, session);
                    if let Some(profile) = self.online_profile(id) {
                        self.spawn(p.server_name, p.address, profile);
                    }
                }
                Err(e) => self.toast(format!("Couldn't sign in {}: {e}", p.account), true),
            }
            self.dirty = true;
        }
    }

    fn pump_files(&mut self) {
        let Files::Preparing(_, rx) = &self.files else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.files = match r {
            Ok(()) => Files::Ready,
            Err(e) => Files::Failed(e),
        };
        self.dirty = true;
        for address in std::mem::take(&mut self.autolaunch) {
            let name = self.servers.iter().find(|s| s.address == address).map_or(address.clone(), |s| s.name.clone());
            self.launch(name, address);
        }
    }

    fn pump_sessions(&mut self) {
        let mut newly_playing = Vec::new();
        let mut newly_ended = Vec::new();
        let mut any_exit = false;
        for s in &mut self.sessions {
            let was_playing = s.reached_play;
            let was_running = s.running();
            let was_ended = s.ended();
            self.dirty |= s.poll();
            if !was_playing && s.reached_play {
                newly_playing.push(s.spec.address.clone());
            }
            if !was_ended && s.ended() {
                newly_ended.push(s.spec.address.clone());
            }
            any_exit |= was_running && !s.running();
        }
        for a in newly_ended {
            self.ping(a);
        }
        if !newly_playing.is_empty() {
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            for server in self.servers.iter_mut().filter(|s| newly_playing.iter().any(|a| address_key(a) == address_key(&s.address))) {
                server.last_joined = Some(now);
            }
            crate::config::save_servers(&self.paths.config, &self.servers);
            // The player count just changed.
            for a in newly_playing {
                self.ping(a);
            }
        }
        if any_exit {
            // Closing cleanly needs no row. Crashes stay, so the log can be read.
            let selected = self.selected_session;
            self.sessions.retain(|s| s.exit != Some(Exit::Clean));
            if selected.is_some_and(|id| !self.sessions.iter().any(|s| s.id == id)) {
                self.selected_session = self.sessions.last().map(|s| s.id);
            }
            // A session that closed has saved a new last view of its server.
            self.textures.retain(|k, _| !k.starts_with("frame:"));
            self.ping_all();
            self.dirty = true;
        }
    }

    /// How long nothing needs checking.
    /// Opens a world: starts it if need be, then a game window into it.
    pub fn play_world(&mut self, world: worlds::World) {
        if !self.files_ready() {
            self.toast("The game files are still being prepared. Try again in a moment.", true);
            return;
        }
        if let Some(host) = &self.host {
            if host.world.folder != world.folder {
                self.toast(format!("{} is still open. Close it first.", host.world.name), true);
                return;
            }
            if let Some(address) = host.address() {
                let name = self.plan(&address).name;
                self.spawn(world.name.clone(), address, SpecProfile::Offline { name });
            }
            return;
        }
        let owner = match self.identity() {
            Identity::Microsoft(a) => a.name,
            Identity::Offline(n) => n,
        };
        self.host = Some(worlds::Host::start(&self.paths, world, owner, self.config.worlds.memory_mb));
        self.host_had_session = false;
        self.dirty = true;
    }

    fn pump_world(&mut self) {
        let Some(host) = &mut self.host else { return };
        let mut open_window = None;
        let mut finished = false;
        while let Some(e) = host.pump() {
            self.dirty = true;
            match e {
                worlds::HostEvent::Ready(_) => open_window = host.address().map(|a| (host.world.name.clone(), a)),
                worlds::HostEvent::Failed(why) => self.toasts.push(ui::Toast { text: why, error: true, at: Instant::now() }),
                worlds::HostEvent::Stopped => finished = true,
                worlds::HostEvent::Step(..) => {}
            }
        }
        if finished {
            self.host = None;
            self.worlds = worlds::list(&self.paths);
            return;
        }
        if let Some((name, address)) = open_window {
            let owner = match self.identity() {
                Identity::Microsoft(a) => a.name,
                Identity::Offline(n) => n,
            };
            self.spawn(name, address, SpecProfile::Offline { name: owner });
            self.host_had_session = true;
            self.worlds = worlds::list(&self.paths);
            return;
        }
        // Everyone's left: the world saves and closes.
        if let Some(address) = host_address(&self.host) {
            if self.host_had_session && self.sessions_on(&address) == 0 {
                if let Some(h) = &mut self.host {
                    h.stop();
                }
                self.dirty = true;
            }
        }
    }

    fn poll_interval(&self) -> Duration {
        if !self.sessions.is_empty() || !self.pending.is_empty() || self.sign_in.is_some() || matches!(self.files, Files::Preparing(..)) {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        }
    }

    pub fn running_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.running()).count()
    }

    /// Matches the look to the settings and, for "System", the desktop.
    fn sync_theme(&mut self) {
        let light = self.window.theme() == Some(winit::window::Theme::Light);
        let wanted = (ui::theme::resolve(&self.config.interface.theme, light, false), self.config.interface.reduce_motion);
        if self.applied_theme != Some(wanted) {
            self.applied_theme = Some(wanted);
            ui::theme::apply(&self.egui_ctx, wanted.0, wanted.1);
        }
    }

    fn redraw(&mut self) {
        self.dirty = false;
        self.sync_theme();
        let raw = self.egui_state.take_egui_input(&self.window);
        let ctx = self.egui_ctx.clone();
        let mut output = ctx.run_ui(raw, |ui| draw(self, ui));
        if let (Some(path), Some(update)) = (std::env::var_os("LATTICE_A11Y_DUMP"), &output.platform_output.accesskit_update) {
            crate::a11y::dump(update, std::path::Path::new(&path));
        }
        self.egui_state.handle_platform_output(&self.window, output.platform_output);
        let delay = output.viewport_output.get(&egui::ViewportId::ROOT).map_or(Duration::from_secs(1), |v| v.repaint_delay);
        self.repaint_at = Instant::now() + delay.min(Duration::from_secs(1));
        let primitives = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);

        let elapsed = self.started.elapsed().as_secs_f32();
        if let Some(i) = self.shots.iter().position(|(t, _)| elapsed >= *t) {
            let (_, path) = self.shots.remove(i);
            self.renderer.capture_next_frame(path);
        }
        if self.exit_after.is_some_and(|t| elapsed >= t) {
            self.exit = true;
        }
        let g = ui::theme::ground();
        let clear = [g.r(), g.g(), g.b()].map(|c| c as f64 / 255.0);
        self.renderer.render(
            None,
            EguiFrame { primitives: &primitives, textures: &mut output.textures_delta, pixels_per_point: output.pixels_per_point },
            clear,
        );
    }
}

/// The whole window: the sessions panel is always there, screens change beside it.
fn host_address(host: &Option<worlds::Host>) -> Option<String> {
    host.as_ref().filter(|h| h.stopping.is_none()).and_then(|h| h.address())
}

fn draw(app: &mut Launcher, ui: &mut egui::Ui) {
    let before = app.screen.clone();
    shortcuts(app, ui);
    egui::Panel::left("navigation")
        .exact_size(224.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(ui::theme::surface()).inner_margin(egui::Margin::same(12)))
        .show(ui, |ui| sidebar(app, ui));
    let page = egui::Frame::new().fill(ui::theme::ground());
    egui::CentralPanel::no_frame().frame(page).show(ui, |ui| match app.screen.clone() {
        Screen::Play => home::draw(app, ui),
        Screen::EditServer { .. } => home::edit_server(app, ui),
        Screen::NewWorld(form) => home::new_world(app, ui, form),
        Screen::Sessions => panel::draw(app, ui),
        Screen::Accounts => accounts::draw(app, ui),
        Screen::Settings => settings(app, ui),
    });
    // Leaving settings is the moment they're saved and open windows hear of them.
    if before == Screen::Settings && app.screen != Screen::Settings {
        app.save_config();
        app.reload_running_sessions();
    }
    let toast_time = app.config.interface.toast_time;
    ui::toasts(&mut app.toasts, ui, toast_time);
}

/// Ctrl+1 to Ctrl+4 go to the four pages.
fn shortcuts(app: &mut Launcher, ui: &egui::Ui) {
    use egui::{Key, Modifiers};
    let pressed = |k: Key| ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, k));
    if pressed(Key::Num1) {
        app.screen = Screen::Play;
    }
    if pressed(Key::Num2) {
        app.screen = Screen::Sessions;
    }
    if pressed(Key::Num3) {
        app.screen = Screen::Accounts;
    }
    if pressed(Key::Num4) {
        app.screen = Screen::Settings;
    }
}

/// The app's frame: where you can go, and who you are.
fn sidebar(app: &mut Launcher, ui: &mut egui::Ui) {
    use ui::theme::*;
    use ui::widgets::{self, Icon};
    ui.horizontal(|ui| {
        ui.set_min_height(48.0);
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
        widgets::mark(ui.painter(), r.center(), 30.0);
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Lattice").font(font_bold(SIZE_LEAD)));
    });
    ui.add_space(12.0);
    ui.spacing_mut().item_spacing.y = 4.0;
    let running = app.running_count();
    let badge = (running > 0).then(|| format!("{running} open"));
    let on_play = matches!(app.screen, Screen::Play | Screen::NewWorld(_) | Screen::EditServer { .. });
    if widgets::nav_item(ui, Icon::Play, "Play", None, on_play).clicked() {
        app.screen = Screen::Play;
    }
    if widgets::nav_item(ui, Icon::Sessions, "Sessions", badge.as_deref(), app.screen == Screen::Sessions).clicked() {
        app.screen = Screen::Sessions;
    }
    if widgets::nav_item(ui, Icon::Account, "Accounts", None, app.screen == Screen::Accounts).clicked() {
        app.screen = Screen::Accounts;
    }
    if widgets::nav_item(ui, Icon::Settings, "Settings", None, app.screen == Screen::Settings).clicked() {
        app.screen = Screen::Settings;
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::muted(ui, format!("Plays Minecraft {}", lattice_proto::GAME_VERSION));
        ui.add_space(6.0);
        identity_picker(app, ui);
        ui.label(egui::RichText::new("Playing as").size(SIZE_SMALL).color(ink_muted()));
    });
}

/// Every identity a new session could use.
fn identity_picker(app: &mut Launcher, ui: &mut egui::Ui) {
    let identity = app.identity();
    let (name, face) = match &identity {
        Identity::Microsoft(a) => (a.name.clone(), app.face_texture(a.uuid, a.skin_url.as_deref())),
        Identity::Offline(n) => (n.clone(), None),
    };
    let accounts = app.accounts.accounts.clone();
    let names = app.config.all_offline_names();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(34.0), egui::Sense::hover());
        ui::widgets::face_tile(ui, rect, face.as_ref());
        egui::ComboBox::from_id_salt("identity").width(ui.available_width() - 8.0).selected_text(name).show_ui(ui, |ui| {
            ui.set_min_width(240.0);
            for a in &accounts {
                let selected = app.accounts.active == Some(a.uuid);
                if ui.selectable_label(selected, format!("{} (Microsoft account)", a.name)).clicked() {
                    app.accounts.active = Some(a.uuid);
                    app.accounts.save(&app.paths.config);
                }
            }
            for n in &names {
                let selected = app.accounts.active.is_none() && app.config.offline_name().eq_ignore_ascii_case(n);
                if ui.selectable_label(selected, format!("{n} (offline name)")).clicked() {
                    app.accounts.active = None;
                    app.accounts.save(&app.paths.config);
                    app.config.player.offline_name = n.clone();
                    app.save_config();
                }
            }
        });
    });
}

fn settings(app: &mut Launcher, ui: &mut egui::Ui) {
    let before_scale = app.config.interface.scale;
    let running = app.running_count() > 0;
    ui::column(ui, |ui| {
        ui.add_space(28.0);
        ui::settings::panel(ui, &mut app.config, &app.paths, &mut app.settings_panel, running.then_some("Open sessions update when you leave this page."), None);
    });
    if app.config.interface.scale != before_scale {
        ui.ctx().set_zoom_factor(app.config.interface.scale);
    }
}

// ------------------------------------------------------------------ winit glue

struct Runner {
    app: Option<Launcher>,
    start: Option<(Paths, Config)>,
    proxy: EventLoopProxy<UserEvent>,
}

/// Runs the launcher until its window is closed. Sessions it started keep running.
pub fn run(paths: Paths, config: Config, mut instance: crate::desktop::Instance) -> anyhow::Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    instance.on_second_launch(move || {
        let _ = proxy.send_event(UserEvent::Raise);
    });
    let mut runner = Runner { app: None, start: Some((paths, config)), proxy: event_loop.create_proxy() };
    event_loop.run_app(&mut runner)?;
    Ok(())
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        let Some((paths, config)) = self.start.take() else { return };
        let log = paths.data.join("logs").join("launcher.log");
        match Launcher::new(event_loop, self.proxy.clone(), paths, config) {
            Ok(app) => {
                app.window.request_redraw();
                self.app = Some(app);
            }
            Err(e) => {
                crate::desktop::report_start_failure(&format!("{e:#}"), &log);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(app) = &mut self.app else { return };
        if matches!(event, UserEvent::Raise) {
            // Wayland doesn't let a window bring itself forward, only ask for
            // attention, which most desktops show on its taskbar entry.
            app.window.focus_window();
            app.window.request_user_attention(Some(winit::window::UserAttentionType::Critical));
            return;
        }
        crate::a11y::handle(&mut app.egui_state, &app.egui_ctx, event);
        app.dirty = true;
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = &mut self.app else { return };
        app.pump();
        if app.updater.poll() {
            app.dirty = true;
        }
        let now = Instant::now();
        if app.dirty || app.repaint_at <= now {
            app.window.request_redraw();
        }
        let wake = app.repaint_at.max(now + Duration::from_millis(8)).min(now + app.poll_interval());
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // The settings are waiting for a key to bind.
        if crate::input::capturing().is_some() {
            match &event {
                WindowEvent::KeyboardInput { event: k, .. } if k.state == winit::event::ElementState::Pressed => {
                    if let winit::keyboard::PhysicalKey::Code(code) = k.physical_key {
                        crate::input::finish_capture(crate::input::bind_from_key(code, k.text.as_ref().and_then(|t| t.chars().next())));
                    }
                }
                WindowEvent::MouseInput { state: winit::event::ElementState::Pressed, button, .. } => {
                    crate::input::finish_capture(crate::input::bind_from_mouse(*button));
                }
                _ => {}
            }
        }
        let Some(app) = &mut self.app else { return };
        let response = app.egui_state.on_window_event(&app.window, &event);
        if response.repaint {
            app.dirty = true;
        }
        match event {
            WindowEvent::CloseRequested => {
                if app.screen == Screen::Settings {
                    app.save_config();
                    app.reload_running_sessions();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(s) => {
                app.renderer.resize(s.width, s.height);
                app.dirty = true;
            }
            WindowEvent::RedrawRequested => {
                app.redraw();
                if app.exit {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    /// Drop the renderer while the display connection still exists (see app.rs).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        drop(self.app.take());
    }
}
