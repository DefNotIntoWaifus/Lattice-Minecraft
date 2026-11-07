//! One game window: joins one server and plays. Started by the launcher, or
//! directly with `lattice --join`. Each window is its own process.

use lattice_proto::ItemStack;
use crate::assets::{AssetState, start_loading};
use crate::config::{Config, Paths};
use crate::game::meshing::Mesher;
use crate::game::player::TickInput;
use crate::input::{Action, Binds};
use crate::game::{Phase, Session, SessionEnd};
use crate::ipc::{self, Control, LaunchSpec, SpecProfile, Status};
use crate::ui;
use crate::ui::Toast;
use crossbeam_channel::{Receiver, unbounded};
use glam::{DVec3, Quat, Vec3};
use lattice_net::{ConnectOptions, Connection, Disconnect, Profile, ServerAddress};
use lattice_proto::Text;
use lattice_proto::Translator;
use lattice_proto::packets::config::ClientInformation;
use lattice_render::renderer::Batches;
use lattice_render::{Camera, ColorVertex, EguiFrame, FrameWorld, Renderer};
use lattice_world::physics::Keys;
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

/// How long a clean disconnect may take before the window closes anyway.
const LEAVE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
pub enum Screen {
    /// Game files are being read.
    Loading,
    Connecting,
    Settings,
    Ended { title: String, reason: Option<Text>, detail: Option<String> },
    InGame,
}

/// Banner patterns as small textures, made as they're first shown.
#[derive(Default)]
pub struct BannerMasks(std::collections::HashMap<String, egui::TextureHandle>);

impl BannerMasks {
    pub fn get_or_make(&mut self, ctx: &egui::Context, assets: &crate::assets::GameAssets, name: &str) -> Option<egui::TextureId> {
        if let Some(t) = self.0.get(name) {
            return Some(t.id());
        }
        let img = assets.banner_masks.get(name).or_else(|| assets.screen_pictures.get(name))?;
        let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let handle = ctx.load_texture(format!("banner {name}"), color, egui::TextureOptions::NEAREST);
        let id = handle.id();
        self.0.insert(name.to_string(), handle);
        Some(id)
    }
}

#[derive(Default)]
pub struct Overlays {
    pub chat_open: bool,
    pub chat_input: String,
    pub chat_history_pos: Option<usize>,
    /// What tab completion has to offer for the chat box.
    pub suggest: crate::ui::hud::Suggest,
    /// What's just been heard, for those who want it written down.
    pub subtitles: Vec<crate::ui::hud::Subtitle>,
    pub chat_error: Option<String>,
    pub paused: bool,
    pub debug: bool,
    pub focus_chat: bool,
    pub hide_hud: bool,
}

#[derive(Default)]
pub struct Input {
    /// Actions held down right now.
    pub held: HashSet<Action>,
    /// The details key is down: other keys pressed with it are shortcuts.
    pub debug_down: bool,
    /// A shortcut was used, so letting go of it shouldn't toggle the details.
    pub debug_used: bool,
    pub captured: bool,
    pub sprint_toggled: bool,
    pub sneak_toggled: bool,
    pub attack_toggled: bool,
    pub use_toggled: bool,
    /// Button presses since the last tick, so a click between ticks isn't lost.
    pub attack_pressed: bool,
    pub use_pressed: bool,
    pub pick_pressed: bool,
    pub drop_pressed: bool,
    pub swap_pressed: bool,
    pub ctrl: bool,
}

/// Frame timing measured over short windows, so the numbers react quickly.
pub struct FrameStats {
    window_start: Instant,
    frames: u32,
    worst: f32,
    pub fps: f32,
    pub frame_ms: f32,
    pub worst_ms: f32,
}

impl FrameStats {
    fn new() -> Self {
        FrameStats { window_start: Instant::now(), frames: 0, worst: 0.0, fps: 0.0, frame_ms: 0.0, worst_ms: 0.0 }
    }

    fn record(&mut self, dt: f32) {
        self.frames += 1;
        self.worst = self.worst.max(dt);
        let elapsed = self.window_start.elapsed().as_secs_f32();
        if elapsed >= 0.25 {
            self.fps = self.frames as f32 / elapsed;
            self.frame_ms = elapsed * 1000.0 / self.frames as f32;
            self.worst_ms = self.worst * 1000.0;
            self.window_start = Instant::now();
            self.frames = 0;
            self.worst = 0.0;
        }
    }
}

pub struct App {
    /// `None` when running headless for tests.
    pub window: Option<Arc<Window>>,
    pub renderer: Renderer,
    pub egui_ctx: egui::Context,
    /// Hotbars saved in creative.
    pub toolbars: crate::game::creative::Toolbars,
    egui_state: Option<egui_winit::State>,
    /// Test commands and the interface events they make, when headless.
    test_commands: Receiver<crate::dev::TestCommand>,
    test_events: Vec<egui::Event>,
    test_modifiers: egui::Modifiers,
    test_frame_time: Option<Duration>,
    pub binds: Binds,
    pub menu: ui::inventory::MenuState,
    pub settings_panel: ui::settings::PanelState,
    scroll_acc: f32,
    /// Eased field of view multiplier, and the last tick's.
    fov_modifier: (f32, f32),
    /// 0 from the eyes, 1 from behind, 2 from the front.
    pub perspective: u8,
    /// How long a tick is right now, seconds.
    tick_len: f64,
    meshes_uploaded: u64,
    /// Moving block textures, advanced with game ticks.
    animations: Vec<lattice_render::atlas::SpriteAnimation>,
    pub skins: crate::skins::SkinStore,
    /// Sound: `None` until the game files are ready.
    pub audio: Option<crate::audio::Audio>,
    ambience: crate::game::ambience::Ambience,
    particles: crate::game::particles::Particles,
    pub banner_masks: BannerMasks,
    /// Pictures laid over the whole screen, made as they're first shown.
    pub screen_pictures: BannerMasks,
    pub face_textures: std::collections::HashMap<uuid::Uuid, egui::TextureHandle>,
    pub doll_texture: Option<(uuid::Uuid, egui::TextureHandle)>,
    /// How dark the edges of the screen are, following the light slowly.
    pub vignette: f32,
    /// How far a spyglass has come up.
    pub scope_scale: f32,
    map_textures: crate::bodies::maps::MapTextures,
    /// Geometry rebuilt every frame; kept to reuse its memory.
    batches: Batches,
    item_texture: Option<egui::TextureHandle>,
    /// What's in hand, and since when, for showing its name.
    pub held_name: (Option<(i32, usize)>, Instant),
    pub paths: Paths,
    pub config: Config,
    pub spec: LaunchSpec,
    pub screen: Screen,
    pub assets: AssetState,
    pub server_packs: crate::packs::ServerPacks,
    /// New assets on their way, with packs added or removed; the old ones stay
    /// in use until these are ready.
    asset_reload: Option<crate::assets::AssetLoader>,
    reload_again: bool,
    pub session: Option<Session>,
    pub mesher: Option<Mesher>,
    pub input: Input,
    pub look: (f32, f32),
    pub overlays: Overlays,
    pub toasts: Vec<Toast>,
    pub frame: FrameStats,
    pub focused: bool,
    control: Receiver<Control>,
    tick_acc: f64,
    last_frame: Instant,
    last_report: Instant,
    pending_end: Option<(SessionEnd, u8)>,
    leave_deadline: Option<Instant>,
    pub capture_request: Option<PathBuf>,
    pub screenshot_dir: PathBuf,
    /// Development aid: frames to save at given times (LATTICE_SHOTS="5:a.png,9:b.png").
    shots: Vec<(f32, PathBuf)>,
    exit_after: Option<f32>,
    started: Instant,
    /// Development aid: timed input after joining (LATTICE_SCRIPT="1:w+,3:yaw=90,4:chat=hi").
    script: Vec<(f32, String)>,
    script_start: Option<Instant>,
    exit: bool,
}

/// Where the last view of a server is kept; the launcher shows it behind the list.
pub fn frame_path(cache: &std::path::Path, address: &str) -> PathBuf {
    let safe: String = address.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    cache.join("frames").join(format!("{safe}.png"))
}

fn srgb_to_linear(c: u32) -> [f32; 3] {
    let f = |v: u32| ((v & 255) as f32 / 255.0).powf(2.2);
    [f(c >> 16), f(c >> 8), f(c)]
}

/// Reads stdin: launcher messages, and test commands when headless.
fn spawn_control_reader(reports: bool, headless: bool) -> (Receiver<Control>, Receiver<crate::dev::TestCommand>) {
    let (tx, rx) = unbounded();
    let (test_tx, test_rx) = unbounded();
    if !reports && !headless {
        return (rx, test_rx);
    }
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            // EOF means the launcher closed; this window just keeps running.
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if headless {
                    if let Ok(c) = serde_json::from_str::<crate::dev::TestCommand>(&line) {
                        if test_tx.send(c).is_err() {
                            return;
                        }
                        continue;
                    }
                }
                match serde_json::from_str::<Control>(&line) {
                    Ok(c) => {
                        if tx.send(c).is_err() {
                            return;
                        }
                    }
                    Err(e) => tracing::warn!("ignored a message on stdin: {e}"),
                }
            }
        })
        .expect("spawn control thread");
    (rx, test_rx)
}

impl App {
    pub fn new(event_loop: &ActiveEventLoop, proxy: winit::event_loop::EventLoopProxy<crate::a11y::UserEvent>, paths: Paths, config: Config, spec: LaunchSpec) -> anyhow::Result<App> {
        let title = format!("{} on {} - Lattice (Minecraft {})", spec.profile.name(), spec.server_name, lattice_proto::GAME_VERSION);
        let attrs = crate::desktop::identify(Window::default_attributes())
            .with_title(title)
            .with_visible(false)
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 760.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(640.0, 420.0));
        let window = Arc::new(event_loop.create_window(attrs)?);
        if config.video.fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let size = window.inner_size();
        let display: OwnedDisplayHandle = event_loop.owned_display_handle();
        let renderer = Renderer::new(window.clone(), Some(Box::new(display)), size.width, size.height, config.video.vsync, false)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let egui_ctx = egui::Context::default();
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
        Ok(Self::build(Some(window), Some(egui_state), renderer, egui_ctx, paths, config, spec))
    }

    /// A game with no window, driven by test commands on stdin.
    pub fn new_headless(paths: Paths, config: Config, spec: LaunchSpec, width: u32, height: u32) -> anyhow::Result<App> {
        let renderer = Renderer::new_offscreen(width, height).map_err(|e| anyhow::anyhow!("{e}"))?;
        crate::dev::enable_widget_log();
        Ok(Self::build(None, None, renderer, egui::Context::default(), paths, config, spec))
    }

    fn build(window: Option<Arc<Window>>, egui_state: Option<egui_winit::State>, renderer: Renderer, egui_ctx: egui::Context, paths: Paths, config: Config, spec: LaunchSpec) -> App {
        ui::theme::install(&egui_ctx);
        ui::theme::use_game_look(&egui_ctx, config.interface.pixel_font);
        ui::theme::apply(&egui_ctx, ui::theme::resolve(&config.interface.theme, false, true), config.interface.reduce_motion);
        egui_ctx.set_zoom_factor(config.interface.scale);
        let server_packs = crate::packs::ServerPacks::default();
        let first_packs = crate::packs::layers(&paths, &config.player.resource_packs, &server_packs);
        let assets = AssetState::Loading(start_loading(paths.cache.clone(), config.player.locale.clone(), first_packs));
        let (control, test_commands) = spawn_control_reader(spec.reports, window.is_none());
        let screenshot_dir = paths.data.join("screenshots");
        let binds = Binds::from_config(&config.controls.binds);
        App {
            toolbars: crate::game::creative::Toolbars::load(&paths.data),
            window,
            renderer,
            egui_ctx,
            egui_state,
            test_commands,
            test_events: Vec::new(),
            test_modifiers: egui::Modifiers::NONE,
            test_frame_time: None,
            binds,
            menu: ui::inventory::MenuState::default(),
            settings_panel: ui::settings::PanelState::default(),
            scroll_acc: 0.0,
            fov_modifier: (1.0, 1.0),
            perspective: 0,
            tick_len: 0.05,
            meshes_uploaded: 0,
            animations: Vec::new(),
            skins: Default::default(),
            audio: None,
            ambience: Default::default(),
            particles: Default::default(),
            banner_masks: Default::default(),
            screen_pictures: Default::default(),
            face_textures: std::collections::HashMap::new(),
            doll_texture: None,
            vignette: 0.0,
            scope_scale: 0.5,
            map_textures: Default::default(),
            batches: Batches::default(),
            item_texture: None,
            held_name: (None, Instant::now()),
            paths,
            config,
            spec,
            screen: Screen::Loading,
            assets,
            server_packs,
            asset_reload: None,
            reload_again: false,
            session: None,
            mesher: None,
            input: Input::default(),
            look: (0.0, 0.0),
            overlays: Overlays::default(),
            toasts: Vec::new(),
            frame: FrameStats::new(),
            focused: true,
            control,
            tick_acc: 0.0,
            last_frame: Instant::now(),
            last_report: Instant::now(),
            pending_end: None,
            leave_deadline: None,
            capture_request: None,
            screenshot_dir,
            shots: crate::dev::timed_paths("LATTICE_SHOTS"),
            exit_after: crate::dev::seconds("LATTICE_EXIT_AFTER"),
            started: Instant::now(),
            script: crate::dev::timed_actions("LATTICE_SCRIPT"),
            script_start: None,
            exit: false,
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, at: Instant::now() });
    }

    /// The game files, once they've loaded.
    pub fn ready_assets(&self) -> Option<Arc<crate::assets::GameAssets>> {
        match &self.assets {
            AssetState::Ready(a) => Some(a.clone()),
            _ => None,
        }
    }

    /// The icon sheet as a texture, made the first time something needs it.
    pub fn item_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if let Some(t) = &self.item_texture {
            return t.id();
        }
        let Some(assets) = self.ready_assets() else { return egui::TextureId::default() };
        let img = &assets.items.atlas;
        let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let handle = ctx.load_texture("item icons", color, egui::TextureOptions::NEAREST);
        let id = handle.id();
        self.item_texture = Some(handle);
        id
    }

    /// The picture of this player's skin for egui, and whether their arms are slim.
    pub fn doll_skin(&mut self, ctx: &egui::Context) -> Option<(egui::TextureId, bool)> {
        let uuid = self.session.as_ref()?.self_uuid();
        if let Some((img, slim)) = self.skins.skin_image(uuid) {
            if self.doll_texture.as_ref().is_none_or(|(id, ..)| *id != uuid) {
                let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                self.doll_texture = Some((uuid, ctx.load_texture("own skin", color, egui::TextureOptions::NEAREST)));
            }
            return self.doll_texture.as_ref().map(|(_, t)| (t.id(), slim));
        }
        let assets = self.ready_assets()?;
        let (name, slim) = crate::bodies::mobs::default_skin(uuid);
        self.screen_pictures.get_or_make(ctx, &assets, &format!("skin/{name}")).map(|t| (t, slim))
    }

    pub fn lang(&self) -> &dyn Translator {
        match &self.assets {
            AssetState::Ready(a) => &a.lang,
            _ => &lattice_proto::text::NoTranslations,
        }
    }

    // -------------------------------------------------------------- session

    /// Starts building the assets again with the packs now wanted.
    fn reload_assets(&mut self) {
        if self.asset_reload.is_some() {
            self.reload_again = true;
            return;
        }
        let packs = crate::packs::layers(&self.paths, &self.config.player.resource_packs, &self.server_packs);
        self.asset_reload = Some(start_loading(self.paths.cache.clone(), self.config.player.locale.clone(), packs));
    }

    /// What servers have asked about packs, and the fetches under way.
    fn pump_packs(&mut self) {
        use crate::game::PackRequest;
        use crate::packs::*;
        let requests = self.session.as_mut().map_or(Vec::new(), |s| std::mem::take(&mut s.pack_requests));
        let mut changed = false;
        for r in requests {
            match r {
                PackRequest::Offer { id, url, hash, forced } => {
                    if self.config.player.server_packs == 1 {
                        if let Some(s) = &mut self.session {
                            s.conn.pack_status(id, STATUS_DECLINED);
                            s.notice(if forced { "This server requires its resource pack, and your settings decline them, so it may disconnect you." } else { "Declined this server's resource pack, as your settings ask." });
                        }
                        continue;
                    }
                    if let Some(s) = &self.session {
                        s.conn.pack_status(id, STATUS_ACCEPTED);
                    }
                    self.server_packs.fetch(id, url, hash, &self.paths.cache);
                }
                PackRequest::Remove(id) => {
                    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.server_packs.active).into_iter().partition(|(p, _)| id.is_none_or(|i| i == *p));
                    self.server_packs.active = kept;
                    changed |= !gone.is_empty();
                    if let Some(s) = &self.session {
                        for (p, _) in gone {
                            s.conn.pack_status(p, STATUS_DISCARDED);
                        }
                    }
                }
            }
        }
        for (id, result) in self.server_packs.finished() {
            match result {
                Ok(path) => {
                    if let Some(s) = &self.session {
                        s.conn.pack_status(id, STATUS_DOWNLOADED);
                    }
                    self.server_packs.active.retain(|(p, _)| *p != id);
                    self.server_packs.active.push((id, path));
                    self.server_packs.awaiting_load.push(id);
                    changed = true;
                }
                Err(e) => {
                    tracing::warn!("couldn't fetch a resource pack: {}", e.message);
                    if let Some(s) = &mut self.session {
                        s.conn.pack_status(id, e.status);
                        s.notice(format!("Couldn't fetch this server's resource pack: {}.", e.message));
                    }
                }
            }
        }
        if changed {
            self.reload_assets();
        }
        let Some(loader) = &self.asset_reload else { return };
        let Ok(result) = loader.result.try_recv() else { return };
        self.asset_reload = None;
        match result {
            Ok(a) => self.swap_assets(a),
            Err(e) => {
                tracing::warn!("couldn't apply resource packs: {e}");
                if let Some(s) = &mut self.session {
                    s.notice(format!("Couldn't apply the resource packs: {e}"));
                }
            }
        }
        if std::mem::take(&mut self.reload_again) {
            self.reload_assets();
        } else if let Some(s) = &self.session {
            for id in self.server_packs.awaiting_load.drain(..) {
                s.conn.pack_status(id, STATUS_LOADED);
            }
        }
    }

    /// Puts freshly built assets in place while the game goes on.
    fn swap_assets(&mut self, mut a: crate::assets::GameAssets) {
        if let Some(mut atlas) = a.atlas.take() {
            self.renderer.upload_atlas(&atlas);
            self.animations = std::mem::take(&mut atlas.animations);
        }
        if let Some(image) = a.sheet_image.take() {
            self.renderer.upload_sheet(&a.bodies.sheet, &image);
        }
        if let Some((item, armor)) = a.glint_images.take() {
            self.renderer.upload_glint(&item, &armor);
        }
        if let Some((portal, creeper, wither)) = a.effect_images.take() {
            self.renderer.upload_effects(&portal, &creeper, &wither);
        }
        if let Some((rain, snow)) = a.weather_images.take() {
            self.renderer.upload_weather(&rain, &snow);
        }
        self.particles.load(&a.particle_sets);
        self.skins.restore(&a.bodies.sheet, &mut self.renderer);
        if let Some(m) = &mut self.mesher {
            m.set_context(Arc::new(a.mesh.clone()));
        }
        if let Some(w) = self.session.as_mut().and_then(|s| s.world.as_mut()) {
            w.mark_all_dirty();
        }
        self.item_texture = None;
        self.banner_masks = Default::default();
        self.screen_pictures = Default::default();
        self.assets = AssetState::Ready(Arc::new(a));
    }

    fn pump_assets(&mut self) {
        let AssetState::Loading(loader) = &self.assets else { return };
        let Ok(result) = loader.result.try_recv() else { return };
        match result {
            Ok(mut a) => {
                if let Some(mut atlas) = a.atlas.take() {
                    self.renderer.upload_atlas(&atlas);
                    self.animations = std::mem::take(&mut atlas.animations);
                }
                if let Some(image) = a.sheet_image.take() {
                    self.renderer.upload_sheet(&a.bodies.sheet, &image);
                }
                if let Some((item, armor)) = a.glint_images.take() {
                    self.renderer.upload_glint(&item, &armor);
                }
                if let Some((portal, creeper, wither)) = a.effect_images.take() {
                    self.renderer.upload_effects(&portal, &creeper, &wither);
                }
                if let Some((rain, snow)) = a.weather_images.take() {
                    self.renderer.upload_weather(&rain, &snow);
                }
                self.assets = AssetState::Ready(Arc::new(a));
                self.audio = Some(crate::audio::Audio::start_on(&self.config.audio.device, self.paths.cache.clone()));
                if let AssetState::Ready(a) = &self.assets {
                    self.particles.load(&a.particle_sets);
                }
                self.rejoin();
            }
            Err(e) => self.assets = AssetState::Failed(e),
        }
    }

    /// Connects (or reconnects) to the server this window was started for.
    pub fn rejoin(&mut self) {
        match ServerAddress::parse(&self.spec.address) {
            Ok(a) => self.connect(a, false),
            Err(e) => {
                self.screen = Screen::Ended { title: format!("Couldn't join {}", self.spec.server_name), reason: None, detail: Some(e) };
            }
        }
    }

    fn connect(&mut self, address: ServerAddress, transfer: bool) {
        // A server's packs last as long as its connection.
        if !self.server_packs.active.is_empty() {
            self.server_packs.active.clear();
            self.server_packs.awaiting_load.clear();
            self.reload_assets();
        }
        let profile = match &self.spec.profile {
            SpecProfile::Offline { name } => Profile::Offline { name: name.clone() },
            SpecProfile::Online { name, uuid, access_token } => {
                let (token, id) = (access_token.clone(), *uuid);
                Profile::Online { name: name.clone(), uuid: id, join: Arc::new(move |hash: &str| lattice_auth::join_server(&token, id, hash)) }
            }
        };
        let self_uuid = profile.uuid();
        let conn = Connection::connect(ConnectOptions { address: address.clone(), profile, client_info: self.client_info(), transfer });
        if let AssetState::Ready(a) = &self.assets {
            self.mesher = Some(Mesher::new(Arc::new(a.mesh.clone())));
        }
        self.apply_graphics();
        self.renderer.clear_world();
        let mut session = Session::new(conn, self.spec.server_name.clone(), address.display(), self_uuid);
        // With a real account, chat can be signed, once Mojang's handed over the keys.
        if let SpecProfile::Online { access_token, .. } = &self.spec.profile {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let token = access_token.clone();
            let spawned = std::thread::Builder::new().name("chat keys".into()).spawn(move || {
                let _ = tx.send(lattice_auth::profile_keys(&token));
            });
            if spawned.is_ok() {
                session.keys_coming = Some(rx);
            }
        }
        self.session = Some(session);
        self.overlays = Overlays::default();
        self.script_start = None;
        self.screen = Screen::Connecting;
    }

    pub fn client_info(&self) -> ClientInformation {
        ClientInformation {
            locale: self.config.player.locale.clone(),
            view_distance: self.config.video.render_distance.clamp(2, 32) as u8,
            main_hand: self.config.player.main_hand,
            chat_mode: self.config.chat.mode,
            chat_colors: self.config.chat.colors,
            skin_parts: self.config.player.skin_parts(),
            text_filtering: self.config.chat.text_filtering,
            allow_listing: self.config.player.allow_listing,
            ..Default::default()
        }
    }

    /// Disconnects and closes this window, keeping a last view of the server.
    pub fn leave(&mut self) {
        match &self.session {
            Some(s) if s.phase == Phase::Playing => {
                self.capture_request = Some(frame_path(&self.paths.cache, &s.address));
                s.conn.close();
                self.leave_deadline = Some(Instant::now() + LEAVE_TIMEOUT);
            }
            _ => self.exit = true,
        }
    }

    fn end_session(&mut self, end: SessionEnd) {
        let Some(session) = self.session.take() else { return };
        self.mesher = None;
        self.renderer.clear_world();
        self.release_mouse();
        let name = session.server_name.clone();
        match end {
            SessionEnd::Transfer(host, port) => match ServerAddress::parse(&format!("{host}:{port}")) {
                Ok(a) => self.connect(a, true),
                Err(e) => self.screen = Screen::Ended { title: format!("Couldn't follow the transfer from {name}"), reason: None, detail: Some(e) },
            },
            SessionEnd::Disconnected(Disconnect::Closed) => self.exit = true,
            SessionEnd::Disconnected(d) => {
                let (title, reason, detail) = match d {
                    Disconnect::Kicked(t) => (format!("Disconnected by {name}"), Some(t), None),
                    Disconnect::Failed(s) => (format!("Couldn't join {name}"), None, Some(s)),
                    Disconnect::Lost(s) => (format!("Lost the connection to {name}"), None, Some(s)),
                    Disconnect::Closed => unreachable!(),
                };
                self.screen = Screen::Ended { title, reason, detail };
            }
        }
    }

    /// Applies settings that changed, whoever changed them.
    pub fn apply_config(&mut self, before: &Config) {
        let c = self.config.clone();
        if c.video.vsync != before.video.vsync && self.focused {
            self.renderer.set_vsync(c.video.vsync);
        }
        if c.video.fullscreen != before.video.fullscreen {
            if let Some(w) = &self.window {
                w.set_fullscreen(c.video.fullscreen.then_some(Fullscreen::Borderless(None)));
            }
        }
        if c.controls.binds != before.controls.binds {
            self.binds = Binds::from_config(&c.controls.binds);
        }
        if c.player.resource_packs != before.player.resource_packs {
            self.reload_assets();
        }
        if c.interface.theme != before.interface.theme || c.interface.reduce_motion != before.interface.reduce_motion {
            ui::theme::apply(&self.egui_ctx, ui::theme::resolve(&c.interface.theme, false, true), c.interface.reduce_motion);
        }
        if c.interface.pixel_font != before.interface.pixel_font {
            ui::theme::use_game_look(&self.egui_ctx, c.interface.pixel_font);
            ui::theme::apply(&self.egui_ctx, ui::theme::resolve(&c.interface.theme, false, true), c.interface.reduce_motion);
        }
        if c.interface.scale != before.interface.scale {
            self.egui_ctx.set_zoom_factor(c.interface.scale);
        }
        let info = |c: &Config| (c.video.render_distance, c.player.main_hand, c.chat.mode, c.chat.colors, c.player.skin_parts(), c.chat.text_filtering, c.player.allow_listing);
        if info(&c) != info(before) {
            if let Some(s) = &self.session {
                s.conn.send(lattice_proto::packets::play::client_information(&self.client_info()));
            }
        }
        self.apply_graphics();
    }

    /// The world's least brightness. The darkness effect pushes this below zero in a slow pulse.
    fn ambient_floor(&self) -> f32 {
        let floor = 0.02 + self.config.video.brightness * 0.06;
        let Some(session) = self.session.as_ref() else { return floor };
        if !session.player.has_effect("darkness") {
            return floor;
        }
        let pulse = ((session.ticks as f32 + self.tick_partial()) * std::f32::consts::PI * 0.025).cos().max(0.0) * 0.45;
        -(0.35 + pulse)
    }

    /// Applies quality settings that live in the renderer and the mesher.
    fn apply_graphics(&mut self) {
        let v = &self.config.video;
        let aniso = match v.anisotropy {
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => 1,
        };
        self.renderer.set_filtering(v.mipmap_levels.clamp(0, 4) as u32, aniso);
        let opts = lattice_render::mesher::MeshOptions {
            smooth_lighting: v.smooth_lighting,
            ambient_occlusion: v.ambient_occlusion.clamp(0, 100) as f32 / 100.0,
            biome_blend: v.biome_blend,
            detailed_leaves: v.detailed_leaves,
        };
        if let Some(m) = &mut self.mesher {
            if m.options() != opts {
                m.set_options(opts);
                if let Some(w) = self.session.as_mut().and_then(|s| s.world.as_mut()) {
                    w.mark_all_dirty();
                }
            }
        }
    }

    fn pump_control(&mut self) {
        while let Ok(c) = self.control.try_recv() {
            match c {
                Control::Stop => self.leave(),
                Control::ReloadConfig => {
                    let before = std::mem::replace(&mut self.config, Config::load(&self.paths.config));
                    self.apply_config(&before);
                }
            }
        }
    }

    fn report(&mut self) {
        if !self.spec.reports || self.last_report.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_report = Instant::now();
        let session = self.session.as_ref();
        let (phase, detail) = match &self.screen {
            Screen::Ended { title, reason, detail } => {
                let why = reason.as_ref().map(|r| r.to_plain(self.lang())).or_else(|| detail.clone()).unwrap_or_default();
                (ipc::Phase::Ended, if why.is_empty() { title.clone() } else { format!("{title}: {why}") })
            }
            Screen::Loading => match &self.assets {
                AssetState::Loading(l) => (ipc::Phase::Loading, l.progress.lock().unwrap().step.clone()),
                AssetState::Failed(e) => (ipc::Phase::Ended, format!("Couldn't prepare the game files: {e}")),
                AssetState::Ready(_) => (ipc::Phase::Loading, String::new()),
            },
            _ => match session.map(|s| &s.phase) {
                Some(Phase::Connecting(stage)) => (ipc::Phase::Connecting, ui::session::stage_text(*stage).to_string()),
                Some(Phase::LoadingTerrain) => (ipc::Phase::Terrain, "Loading terrain…".to_string()),
                Some(Phase::Playing) => (ipc::Phase::Playing, "In game".to_string()),
                None => (ipc::Phase::Connecting, String::new()),
            },
        };
        let status = Status {
            phase,
            detail,
            fps: self.frame.fps,
            frame_ms: self.frame.frame_ms,
            worst_ms: self.frame.worst_ms,
            focused: self.focused,
            ping: session.and_then(|s| s.tab.get(&s.self_uuid)).map(|e| e.latency),
            players: session.map(|s| s.tab.values().filter(|e| e.listed).count()).filter(|n| *n > 0),
            pos: session.filter(|s| s.player.received_position).map(|s| s.player.body.pos.to_array()),
            dimension: session.and_then(|s| s.world.as_ref()).map(|w| w.dimension.name.clone()),
        };
        let mut out = std::io::stdout().lock();
        // A closed pipe only means the launcher went away.
        if serde_json::to_writer(&mut out, &status).is_ok() {
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }

    // -------------------------------------------------------------- input

    pub fn capture_mouse(&mut self) {
        if self.input.captured {
            return;
        }
        let Some(window) = &self.window else {
            self.input.captured = true;
            return;
        };
        let ok = window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
        window.set_cursor_visible(false);
        self.input.captured = ok;
        if !ok {
            window.set_cursor_visible(true);
        }
    }

    pub fn release_mouse(&mut self) {
        if let Some(window) = &self.window {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.input.captured = false;
        self.input.held.clear();
    }

    /// How far through the current tick this frame is.
    pub fn tick_partial(&self) -> f32 {
        (self.tick_acc / self.tick_len) as f32
    }

    /// How many ticks this frame took.
    pub fn frame_ticks(&self) -> f32 {
        (self.frame.frame_ms / 50.0).clamp(0.0, 1.0) as f32
    }

    pub fn wants_game_input(&self) -> bool {
        self.screen == Screen::InGame
            && !self.overlays.chat_open
            && !self.overlays.paused
            && self.session.as_ref().is_some_and(|s| s.dead.is_none() && s.code_of_conduct.is_none() && !s.inventory.is_open() && s.modal.is_none() && s.player.body.pose != lattice_world::physics::Pose::Sleeping)
    }

    fn scripted(&self) -> bool {
        self.script_start.is_some() || !self.script.is_empty()
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        // A covered window must never stall the game loop waiting to present,
        // so VSync only applies while the window has focus.
        self.renderer.set_vsync(focused && self.config.video.vsync);
        if !focused && !self.scripted() {
            if self.screen == Screen::InGame && !self.overlays.chat_open {
                self.overlays.paused = true;
            }
            self.release_mouse();
        }
    }

    /// Time between frames when this window is being held back.
    fn frame_limit(&self) -> Option<Duration> {
        let v = &self.config.video;
        if !self.focused && v.limit_background_fps {
            return Some(Duration::from_secs_f64(1.0 / v.background_fps.max(1) as f64));
        }
        (v.max_fps > 0).then(|| Duration::from_secs_f64(1.0 / v.max_fps as f64))
    }

    /// Closes the inventory or menu on show, and tells the server if it was the server's.
    pub fn close_inventory(&mut self) {
        if let Some(s) = &mut self.session {
            // The cursor's contents go back to the inventory when the menu closes.
            if let Some(id) = s.inventory.close() {
                s.conn.send(lattice_proto::packets::play::container_close(id));
            }
        }
        // Whether the recipe book is open and how it's filtered carry over to the next menu.
        let book = self.menu.book.for_next_menu();
        self.menu = ui::inventory::MenuState { creative_tab: self.menu.creative_tab, book, ..Default::default() };
        self.capture_mouse();
    }

    /// Closes a sign or book screen. `save` keeps what was written.
    pub fn close_modal(&mut self, save: bool) {
        if let Some(s) = &mut self.session {
            match s.modal.take() {
                Some(crate::game::Modal::Advancements { .. }) => s.conn.send(lattice_proto::packets::play::seen_advancements(None)),
                Some(crate::game::Modal::Sign { pos, front, lines, .. }) => {
                    // A sign always takes what's on it, as there's no way to back out.
                    s.conn.send(lattice_proto::packets::play::sign_update(pos, front, &lines));
                }
                Some(crate::game::Modal::WriteBook { slot, mut pages, .. }) if save => {
                    ui::modal::trim_pages(&mut pages);
                    s.conn.send(lattice_proto::packets::play::edit_book(slot, &pages, None));
                }
                _ => {}
            }
        }
        self.capture_mouse();
    }

    fn on_key(&mut self, code: KeyCode, typed: Option<char>, pressed: bool) {
        if pressed && crate::input::capturing().is_some() {
            crate::input::finish_capture(crate::input::bind_from_key(code, typed));
            return;
        }
        if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight) {
            self.input.ctrl = pressed;
        }
        if self.screen != Screen::InGame {
            return;
        }
        let inventory_open = self.session.as_ref().is_some_and(|s| s.inventory.is_open());
        if pressed && code == KeyCode::Escape {
            if self.overlays.chat_open {
                if !self.overlays.suggest.take_escape() {
                    self.overlays.chat_open = false;
                    self.capture_mouse();
                }
            } else if self.session.as_ref().is_some_and(|s| s.modal.is_some()) {
                // A server's screen can insist on an answer.
                let fixed = self.session.as_ref().is_some_and(|s| matches!(s.modal, Some(crate::game::Modal::Dialog { can_close: false, .. })));
                if !fixed {
                    self.close_modal(true);
                }
            } else if inventory_open {
                self.close_inventory();
            } else if self.session.as_ref().is_some_and(|s| s.dead.is_none()) {
                self.overlays.paused = !self.overlays.paused;
                if self.overlays.paused { self.release_mouse() } else { self.capture_mouse() }
            }
            return;
        }
        if pressed && self.input.debug_down && self.debug_shortcut(code) {
            return;
        }
        for action in self.binds.actions_for_key(code, typed) {
            self.on_action(action, pressed);
        }
    }

    fn on_mouse_button(&mut self, button: u8, pressed: bool) {
        for action in self.binds.actions_for_mouse(button) {
            self.on_action(action, pressed);
        }
    }

    /// Keeps the hotbar's items in one of the nine saved rows.
    fn save_toolbar(&mut self, row: usize) {
        let Some(s) = self.session.as_ref() else { return };
        let items: Vec<Option<ItemStack>> = (0..9).map(|i| s.inventory.hotbar(i).cloned()).collect();
        self.toolbars.set(row, items);
        self.toolbars.save(&self.paths.data);
        self.toast(format!("Saved your hotbar as row {}", row + 1), false);
    }

    /// Puts a saved row back on the hotbar.
    fn load_toolbar(&mut self, row: usize) {
        let Some(s) = self.session.as_mut() else { return };
        for (i, stack) in self.toolbars.row(row).iter().enumerate() {
            let slot = 36 + i;
            s.conn.send(lattice_proto::packets::play::set_creative_mode_slot(slot as i16, stack.as_ref()));
            if let Some(entry) = s.inventory.slots.get_mut(slot) {
                *entry = stack.clone();
            }
        }
    }

    /// A key pressed with the details key held. True if it meant something.
    fn debug_shortcut(&mut self, code: KeyCode) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        match code {
            // Shows mob and object boxes.
            KeyCode::KeyB => {
                self.config.debug.hitboxes = !self.config.debug.hitboxes;
                let on = self.config.debug.hitboxes;
                self.toast(format!("Hit boxes {}", if on { "shown" } else { "hidden" }), false);
            }
            KeyCode::KeyG => {
                self.config.debug.chunk_borders = !self.config.debug.chunk_borders;
                let on = self.config.debug.chunk_borders;
                self.toast(format!("Chunk borders {}", if on { "shown" } else { "hidden" }), false);
            }
            KeyCode::KeyH => {
                let on = !crate::ui::items::advanced_tooltips();
                crate::ui::items::set_advanced_tooltips(on);
                self.toast(format!("Item details {}", if on { "shown" } else { "hidden" }), false);
            }
            KeyCode::KeyD => {
                s.chat = Default::default();
            }
            KeyCode::KeyC => {
                let b = &s.player.body;
                let text = format!("/execute in {} run tp @s {:.2} {:.2} {:.2} {:.2} {:.2}", s.dimension_name, b.pos.x, b.pos.y, b.pos.z, b.yaw, b.pitch);
                self.egui_ctx.copy_text(text);
                self.toast("Copied your position".to_string(), false);
            }
            // Copies what's in the block or mob in view, as a command that recreates it.
            KeyCode::KeyI => {
                if s.player.permission < 2 {
                    return true;
                }
                s.copy_transaction += 1;
                let id = s.copy_transaction;
                if let Some(hit) = s.player.target {
                    let state = s.world.as_ref().and_then(|w| w.block_at(hit.pos)).unwrap_or(0);
                    let name = &lattice_data::game().block(state).name;
                    s.copy_prefix = Some(format!("/setblock {} {} {} minecraft:{name}", hit.pos.x, hit.pos.y, hit.pos.z));
                    s.conn.send(lattice_proto::packets::play::block_entity_tag_query(id, hit.pos));
                } else if let Some((entity, _)) = s.player.target_entity {
                    let kind = s.entities.map.get(&entity).map_or("pig", |e| e.type_name());
                    let p = s.entities.map.get(&entity).map_or(DVec3::ZERO, |e| e.pos);
                    s.copy_prefix = Some(format!("/summon minecraft:{kind} {:.2} {:.2} {:.2}", p.x, p.y, p.z));
                    s.conn.send(lattice_proto::packets::play::entity_tag_query(id, entity));
                }
            }
            KeyCode::KeyN => {
                if s.player.permission >= 2 {
                    let mode = if s.player.game_mode == 3 { s.player.previous_game_mode.clamp(0, 2) } else { 3 };
                    s.conn.send(lattice_proto::packets::play::change_game_mode(mode));
                }
            }
            KeyCode::F4 => {
                if s.player.permission >= 2 {
                    let next = match &s.modal {
                        Some(crate::game::Modal::GameMode { selected }) => (selected + 1) % 4,
                        _ => s.player.game_mode.clamp(0, 3) as usize,
                    };
                    s.modal = Some(crate::game::Modal::GameMode { selected: next });
                    s.screen_opened = true;
                }
            }
            KeyCode::KeyT => self.reload_assets(),
            KeyCode::KeyA => {
                if let Some(w) = s.world.as_mut() {
                    w.mark_all_dirty();
                }
                self.toast("Redrawing the world".to_string(), false);
            }
            _ => return false,
        }
        self.input.debug_used = true;
        self.config.save(&self.paths.config);
        true
    }

    /// An action's key or button went down or up.
    pub fn on_action(&mut self, action: Action, pressed: bool) {
        if !pressed && action == Action::Debug && self.input.debug_down {
            self.input.debug_down = false;
            // Letting go chooses the game mode being pointed at.
            if let Some(s) = self.session.as_mut() {
                if let Some(crate::game::Modal::GameMode { selected }) = s.modal {
                    s.conn.send(lattice_proto::packets::play::change_game_mode(selected as i32));
                    s.modal = None;
                    self.capture_mouse();
                    return;
                }
            }
            if !self.input.debug_used {
                self.overlays.debug = !self.overlays.debug;
            }
            return;
        }
        if pressed {
            match action {
                Action::Debug => {
                    self.input.debug_down = true;
                    self.input.debug_used = false;
                    return;
                }
                Action::Screenshot => {
                    let _ = std::fs::create_dir_all(&self.screenshot_dir);
                    let name = format!("{}.png", now_unix());
                    self.renderer.capture_next_frame(self.screenshot_dir.join(&name));
                    self.toast(format!("Saved screenshot {name}"), false);
                    return;
                }
                Action::Fullscreen => {
                    self.config.video.fullscreen = !self.config.video.fullscreen;
                    if let Some(w) = &self.window {
                        w.set_fullscreen(self.config.video.fullscreen.then_some(Fullscreen::Borderless(None)));
                    }
                    self.config.save(&self.paths.config);
                    return;
                }
                Action::HideHud => {
                    self.overlays.hide_hud = !self.overlays.hide_hud;
                    return;
                }
                Action::Perspective => {
                    self.perspective = (self.perspective + 1) % 3;
                    return;
                }
                Action::Inventory => {
                    let open = self.session.as_ref().is_some_and(|s| s.inventory.is_open());
                    // The letter for this key can be what someone's typing, into an anvil or a search.
                    if open && !self.overlays.chat_open && !self.egui_ctx.egui_wants_keyboard_input() {
                        self.close_inventory();
                        return;
                    }
                }
                _ => {}
            }
        }
        if !self.wants_game_input() {
            if !pressed {
                self.input.held.remove(&action);
            } else if self.session.as_ref().is_some_and(|s| s.inventory.is_open()) && !self.egui_ctx.egui_wants_keyboard_input() {
                // Keys that act on the slot under the pointer, following whatever they're bound to.
                self.menu.keys.push(action);
            }
            return;
        }
        if pressed {
            match action {
                Action::Chat => {
                    self.open_chat(String::new());
                    return;
                }
                Action::Command => {
                    self.open_chat("/".into());
                    return;
                }
                Action::Advancements => {
                    if let Some(s) = &mut self.session {
                        ui::modal::open_advancements(s);
                    }
                    self.release_mouse();
                    return;
                }
                Action::Inventory => {
                    if let Some(s) = &mut self.session {
                        if s.player.game_mode != 3 {
                            let vehicle_screen = s.player.vehicle.and_then(|v| s.entities.map.get(&v)).is_some_and(|v| crate::game::riding::has_own_screen(v.type_name()));
                            if vehicle_screen {
                                let id = s.player.entity_id;
                                s.conn.send(lattice_proto::packets::play::player_command(id, lattice_proto::packets::play::command::OPEN_INVENTORY));
                            } else {
                                s.inventory.own_screen_open = true;
                            }
                        }
                    }
                    self.release_mouse();
                    return;
                }
                Action::Sprint if !self.config.controls.hold_to_sprint => self.input.sprint_toggled = !self.input.sprint_toggled,
                Action::Sneak if !self.config.controls.hold_to_sneak => self.input.sneak_toggled = !self.input.sneak_toggled,
                Action::Hotbar(n) => {
                    let saving = self.input.held.contains(&Action::SaveToolbar);
                    let loading = self.input.held.contains(&Action::LoadToolbar);
                    let creative = self.session.as_ref().is_some_and(|s| s.player.game_mode == 1);
                    let spectator = self.session.as_ref().is_some_and(|s| s.player.game_mode == 3);
                    if spectator {
                        // A spectator has no hotbar: these keys offer who to go to instead.
                        if let Some(s) = &mut self.session {
                            s.modal = Some(crate::game::Modal::Teleport);
                            s.screen_opened = true;
                        }
                    } else if creative && saving {
                        self.save_toolbar(n as usize);
                    } else if creative && loading {
                        self.load_toolbar(n as usize);
                    } else if let Some(s) = &mut self.session {
                        s.player.select_slot(&s.conn, n as i32);
                    }
                }
                Action::Attack => {
                    self.input.attack_pressed = true;
                    self.input.attack_toggled = !self.input.attack_toggled;
                }
                Action::Use => {
                    self.input.use_pressed = true;
                    self.input.use_toggled = !self.input.use_toggled;
                }
                Action::PickBlock => self.input.pick_pressed = true,
                Action::Drop => self.input.drop_pressed = true,
                Action::SwapHands => self.input.swap_pressed = true,
                _ => {}
            }
            self.input.held.insert(action);
        } else {
            self.input.held.remove(&action);
        }
    }

    pub fn open_chat_with(&mut self, prefill: String) {
        self.open_chat(prefill);
    }

    fn open_chat(&mut self, prefill: String) {
        self.overlays.chat_open = true;
        self.overlays.chat_input = prefill;
        self.overlays.chat_history_pos = None;
        self.overlays.suggest = Default::default();
        self.overlays.chat_error = None;
        self.overlays.focus_chat = true;
        self.release_mouse();
    }

    /// The input for one tick, consuming the presses waiting for it.
    fn tick_input(&mut self) -> TickInput {
        let playing = self.wants_game_input();
        let held = &self.input.held;
        let h = |a: Action| playing && held.contains(&a);
        let c = &self.config.controls;
        let keys = Keys {
            forward: h(Action::Forward),
            back: h(Action::Back),
            left: h(Action::Left),
            right: h(Action::Right),
            jump: h(Action::Jump),
            shift: if c.hold_to_sneak { h(Action::Sneak) } else { playing && self.input.sneak_toggled },
            sprint: if c.hold_to_sprint { h(Action::Sprint) } else { playing && self.input.sprint_toggled },
        };
        let t = TickInput {
            keys,
            yaw: self.look.0,
            pitch: self.look.1,
            attack_held: (if c.hold_to_attack { h(Action::Attack) } else { playing && self.input.attack_toggled }) && self.input.captured,
            attack_pressed: playing && self.input.attack_pressed,
            use_held: (if c.hold_to_use { h(Action::Use) } else { playing && self.input.use_toggled }) && self.input.captured,
            use_pressed: playing && self.input.use_pressed,
            pick_pressed: playing && self.input.pick_pressed,
            pick_with_data: self.input.ctrl,
            drop_pressed: playing && self.input.drop_pressed,
            drop_stack: self.input.ctrl,
            swap_hands_pressed: playing && self.input.swap_pressed,
            auto_jump: c.auto_jump,
            sprint_window: c.sprint_window,
        };
        self.input.attack_pressed = false;
        self.input.use_pressed = false;
        self.input.pick_pressed = false;
        self.input.drop_pressed = false;
        self.input.swap_pressed = false;
        t
    }

    // -------------------------------------------------------------- frame

    fn update_game(&mut self, dt: f64) {
        let Some(session) = &mut self.session else { return };
        let lang: &dyn Translator = match &self.assets {
            AssetState::Ready(a) => &a.lang,
            _ => &lattice_proto::text::NoTranslations,
        };
        if let Some(end) = session.pump(lang) {
            // Keep the world on screen for one more frame so it can be captured.
            if session.phase == Phase::Playing && self.pending_end.is_none() && self.leave_deadline.is_none() {
                self.capture_request = Some(frame_path(&self.paths.cache, &session.address));
            }
            self.pending_end = Some((end, 2));
        }
        if session.dimension_changed {
            session.dimension_changed = false;
            self.renderer.clear_world();
            // New place, new surroundings to hear.
            if let Some(audio) = &mut self.audio {
                self.ambience.reset(audio);
            }
            if let (Some(m), Some(reg)) = (&mut self.mesher, &session.registries) {
                m.clear();
                if let AssetState::Ready(a) = &self.assets {
                    m.set_context(Arc::new(a.mesh.with_biomes(reg.biomes.clone())));
                }
            }
        }
        for (uuid, textures) in session.wanted_skins.drain(..) {
            self.skins.request(uuid, &textures);
        }
        if let AssetState::Ready(a) = &self.assets {
            self.skins.pump(&a.bodies.sheet, &mut self.renderer);
            let renderer = &mut self.renderer;
            self.map_textures.pump(&session.maps, &a.bodies.sheet, |x, y, img| renderer.write_sheet(x, y, img));
        }
        for (x, z) in session.removed_columns.drain(..) {
            self.renderer.remove_column(x, z);
            if let Some(m) = &mut self.mesher {
                m.forget_column(x, z);
            }
        }
        let mut entered = false;
        if matches!(session.phase, Phase::Playing | Phase::LoadingTerrain) && self.screen == Screen::Connecting {
            self.screen = Screen::InGame;
            self.look = (session.player.body.yaw, session.player.body.pitch);
            entered = true;
        }

        // Fixed 20 Hz ticks. A slow window catches up rather than slowing the player.
        self.tick_acc = (self.tick_acc + dt).min(1.0);
        // The server can run its world faster or slower than usual.
        self.tick_len = 1.0 / session.tick_rate.clamp(1.0, 10000.0) as f64;
        while self.tick_acc >= self.tick_len {
            self.tick_acc -= self.tick_len;
            let input = self.tick_input();
            let Some(session) = &mut self.session else { return };
            session.tick(input);
            if let Some(look) = session.player.look_override.take() {
                self.look = look;
            }
            let target = session.player.fov_target(self.config.video.fov_effects as f32 / 100.0, self.perspective == 0);
            // Edges of the screen darken slowly in the dark.
            if let Some(w) = &session.world {
                let eye = session.player.body.pos + DVec3::new(0.0, session.player.body.eye_height() as f64, 0.0);
                let (sky, block) = w.light(eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
                let lit = crate::ui::hud::light_brightness(sky.max(block), w.dimension.ambient_light);
                self.vignette += ((1.0 - lit).clamp(0.0, 1.0) - self.vignette) * 0.01;
            }
            self.fov_modifier.0 = self.fov_modifier.1;
            self.fov_modifier.1 += (target - self.fov_modifier.1) * 0.5;
            for a in &mut self.animations {
                if let Some(levels) = a.tick() {
                    self.renderer.write_atlas(a.x, a.y, &levels);
                }
            }
            if let Some(audio) = &mut self.audio {
                if ui::widgets::take_button_press() {
                    audio.play("ui.button.click", crate::audio::UI, None, 0.25, 1.0);
                }
                self.ambience.music_frequency = self.config.audio.music_frequency;
                audio.use_device(&self.config.audio.device);
                self.ambience.tick(session, audio);
            }
            if let AssetState::Ready(assets) = &self.assets {
                if self.config.video.particles {
                    crate::game::effects::tick(session, &mut self.particles, assets);
                    if let Some(world) = &session.world {
                        self.particles.tick(world);
                    }
                } else {
                    session.particle_requests.clear();
                    session.player.broke.clear();
                    session.broke_elsewhere.clear();
                    session.entities.events.clear();
                    self.particles.clear();
                }
            }
        }
        let Some(session) = &mut self.session else { return };
        if let Some(look) = session.player.look_override.take() {
            self.look = look;
        }
        if let Some(audio) = &mut self.audio {
            audio.pump();
            let a = &self.config.audio;
            audio.set_volumes([a.master, a.music, a.records, a.weather, a.blocks, a.hostile, a.neutral, a.players, a.ambient, a.voice, a.ui]);
            let p = session.player.body.pos + DVec3::new(0.0, session.player.eye_height as f64, 0.0);
            audio.set_listener(p.as_vec3(), self.look.0);
            for cue in session.cues.drain(..) {
                audio.play(&cue.name, cue.category.min(crate::audio::CATEGORIES - 1), cue.pos.map(|p| p.as_vec3()), cue.volume, cue.pitch);
            }
            for (source, _name) in session.sound_stops.drain(..) {
                audio.stop_category(source.map(|s| s as usize));
            }
            let heard = audio.take_heard();
            if self.config.audio.subtitles {
                let now = Instant::now();
                for (key, pos) in heard {
                    match self.overlays.subtitles.iter_mut().find(|s| s.key == key) {
                        Some(s) => {
                            s.pos = pos;
                            s.at = now;
                        }
                        None => self.overlays.subtitles.push(ui::hud::Subtitle { key, pos, at: now }),
                    }
                }
                self.overlays.subtitles.retain(|s| s.at.elapsed().as_secs_f32() < 3.0);
            } else {
                self.overlays.subtitles.clear();
            }
        } else {
            session.cues.clear();
            session.sound_stops.clear();
        }

        // Menus the server opened or closed.
        if std::mem::take(&mut session.wants_credits) {
            // PLAYERNAME is the game's mark for who's being spoken to. Lines that start with the
            // game's scrambling code are the unreadable parts of the conversation.
            let me = session.player_name();
            let poem = self.ready_assets().map_or_else(String::new, |a| a.end_poem.clone());
            let lines: Vec<String> = poem.lines().map(|l| strip_codes(&l.replace("PLAYERNAME", &me))).collect();
            if let Some(session) = self.session.as_mut() {
                session.modal = Some(crate::game::Modal::Credits { lines, started: Instant::now() });
                session.screen_opened = true;
            }
        }
        let Some(session) = self.session.as_mut() else { return };
        if let Some(text) = session.clipboard.take() {
            self.egui_ctx.copy_text(text);
        }
        let Some(session) = self.session.as_mut() else { return };
        if std::mem::take(&mut session.screen_opened) {
            self.overlays.chat_open = false;
            self.input.held.clear();
            if let Some(w) = &self.window {
                let _ = w.set_cursor_grab(CursorGrabMode::None);
                w.set_cursor_visible(true);
            }
            self.input.captured = false;
        }
        if std::mem::take(&mut session.screen_closed) && !self.overlays.paused && !self.overlays.chat_open && self.screen == Screen::InGame {
            if session.dead.is_none() {
                if let Some(w) = &self.window {
                    let ok = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
                    w.set_cursor_visible(false);
                    self.input.captured = ok;
                } else {
                    self.input.captured = true;
                }
            }
        }

        // Meshing.
        if let (Some(world), Some(mesher)) = (&mut session.world, &mut self.mesher) {
            let p = session.player.body.pos;
            let centre = [(p.x.floor() as i32) >> 4, (p.y.floor() as i32) >> 4, (p.z.floor() as i32) >> 4];
            mesher.schedule(world, centre);
            for mesh in mesher.collect() {
                self.renderer.upload_section(mesh);
                self.meshes_uploaded += 1;
            }
        }
        if entered {
            self.script_start = Some(Instant::now());
            // Switched away while it was loading: wait on the pause menu.
            if self.focused || !self.script.is_empty() {
                self.capture_mouse();
            } else {
                self.overlays.paused = true;
            }
        }
    }

    fn run_script(&mut self) {
        let Some(start) = self.script_start else { return };
        let t = start.elapsed().as_secs_f32();
        while let Some(i) = self.script.iter().position(|(at, _)| t >= *at) {
            let (_, action) = self.script.remove(i);
            tracing::info!("script: {action}");
            let key = |name: &str| match name {
                "w" => Some(Action::Forward),
                "a" => Some(Action::Left),
                "s" => Some(Action::Back),
                "d" => Some(Action::Right),
                "jump" => Some(Action::Jump),
                "sneak" => Some(Action::Sneak),
                "sprint" => Some(Action::Sprint),
                _ => None,
            };
            if let Some(v) = action.strip_prefix("yaw=") {
                self.look.0 = v.parse().unwrap_or(self.look.0);
            } else if let Some(v) = action.strip_prefix("pitch=") {
                self.look.1 = v.parse().unwrap_or(self.look.1);
            } else if let Some(v) = action.strip_prefix("chat=") {
                if let Some(s) = &mut self.session {
                    if let Err(e) = s.send_chat(v) {
                        tracing::warn!("script chat: {e}");
                    }
                }
            } else if action == "f3" {
                self.overlays.debug = !self.overlays.debug;
            } else if action == "attack+" || action == "attack-" {
                self.on_action(Action::Attack, action.ends_with('+'));
            } else if action == "use+" || action == "use-" {
                self.on_action(Action::Use, action.ends_with('+'));
            } else if action == "disconnect" {
                self.leave();
            } else if let Some(k) = action.strip_suffix('+').and_then(key) {
                self.input.held.insert(k);
            } else if let Some(k) = action.strip_suffix('-').and_then(key) {
                self.input.held.remove(&k);
            } else {
                tracing::warn!("unknown script action {action}");
            }
        }
    }

    fn world_frame(&self, batches: &mut Batches) -> Option<(Camera, WorldLook, Vec<ColorVertex>, Vec<ColorVertex>)> {
        let session = self.session.as_ref()?;
        let world = session.world.as_ref()?;
        if !session.player.received_position {
            return None;
        }
        let partial = self.tick_acc / self.tick_len;
        let eye = session.player.eye(partial);
        let v = &self.config.video;
        let mut fov = self.config.video.fov * (self.fov_modifier.0 + (self.fov_modifier.1 - self.fov_modifier.0) * partial as f32);
        let effects = v.fov_effects as f32 / 100.0;
        if session.player.body.eye_in_water {
            fov *= 1.0 + (6.0 / 7.0 - 1.0) * effects;
        }
        // Steps move the camera a little, and being hurt tips it.
        let (mut roll, mut nod, mut shift) = (0.0f32, 0.0f32, Vec3::ZERO);
        if v.view_bobbing {
            let (b, t) = session.player.view_bob(partial as f32);
            let st = (t * std::f32::consts::PI).sin();
            let ct = (t * std::f32::consts::PI).cos();
            shift = Vec3::new(st * b * 0.5, -(ct * b).abs(), 0.0);
            roll += st * b * 3.0;
            nod += ((t * std::f32::consts::PI - 0.2).cos() * b).abs() * 5.0;
        }
        if session.hurt_ticks < 10 {
            let f = 1.0 - (session.hurt_ticks as f32 + partial as f32) / 10.0;
            roll -= (f * f * f * f * std::f32::consts::PI).sin() * 14.0 * (v.damage_tilt as f32 / 100.0);
        }
        // From the eyes, or pulled back along the look line until something's in the way.
        let (mut cam_yaw, mut cam_pitch) = self.look;
        // A turning boat takes the view round with it, a little each frame.
        cam_yaw -= session.player.turned * (1.0 - partial as f32);
        let mut eye = eye;
        if self.perspective != 0 {
            if self.perspective == 2 {
                cam_yaw += 180.0;
                cam_pitch = -cam_pitch;
            }
            let (y, p) = (cam_yaw.to_radians() as f64, cam_pitch.to_radians() as f64);
            let back = -DVec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos());
            let mut dist: f64 = 4.0;
            let data = lattice_data::game();
            for i in 0..8 {
                let off = DVec3::new(if i & 1 == 0 { -0.1 } else { 0.1 }, if i & 2 == 0 { -0.1 } else { 0.1 }, if i & 4 == 0 { -0.1 } else { 0.1 });
                if let Some(hit) = crate::game::player::raycast(world, data, eye + off, back, dist) {
                    let at = DVec3::new(hit.pos.x as f64 + hit.cursor[0] as f64, hit.pos.y as f64 + hit.cursor[1] as f64, hit.pos.z as f64 + hit.cursor[2] as f64);
                    dist = dist.min(at.distance(eye + off));
                }
            }
            eye += back * dist;
            // The camera shakes with steps only from the eyes.
            (roll, nod, shift) = (0.0, 0.0, Vec3::ZERO);
        }
        // Portals and nausea warp the whole view, as much as the setting allows.
        let warp = {
            let portal = session.portal_effect.0 + (session.portal_effect.1 - session.portal_effect.0) * partial as f32;
            let nausea = session.player.effect_duration("nausea").map_or(0.0, |d| if d < 0 { 1.0 } else { (d as f32 / 20.0).min(1.0) });
            let k = v.distortion as f32 / 100.0;
            let amount = portal.max(nausea) * k * k;
            (amount > 0.0).then(|| {
                let spin = session.spin_angle + session.spin_speed * partial as f32;
                let squeeze = 5.0 / (amount * amount + 5.0) - amount * 0.04;
                (spin, squeeze * squeeze)
            })
        };
        let camera = Camera { pos: eye, yaw: cam_yaw, pitch: cam_pitch, fov_y_deg: fov.clamp(1.0, 170.0), roll, nod, shift, warp };

        // Sky: real day time and the server's biome/dimension colours.
        let (bx, by, bz) = (eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
        let dim = &world.dimension;
        let biome = session.biome_at(bx, by, bz);
        let sky_hex = biome.and_then(|b| b.sky_color).or(dim.sky_color).unwrap_or(0x78A7FF);
        let fog_hex = biome.and_then(|b| b.fog_color).or(dim.fog_color).unwrap_or(0xC0D8FF);
        let f = session.day_fraction();
        let angle = f * std::f64::consts::TAU;
        let sun = Vec3::new(angle.cos() as f32, angle.sin() as f32, 0.0).normalize();
        // Brightness follows the sun's height; weather takes some of it away.
        let rain = session.rain_level;
        let thunder = session.thunder_level * rain;
        let day_k = (sun.y * 2.0 + 0.5).clamp(0.0, 1.0);
        let boss = session.boss_darkness_prev + (session.boss_darkness - session.boss_darkness_prev) * self.tick_partial();
        let daylight = if dim.has_skylight { day_k * (1.0 - rain * 5.0 / 16.0) * (1.0 - thunder * 5.0 / 16.0) } else { 0.6 } * (1.0 - boss * 0.35);
        // Colours dim as the eye sees them, so nights are properly dark.
        let scale = |hex: u32, k: f32| {
            let c = [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0];
            c.map(|v| v * k)
        };
        // Rain greys the sky.
        let grey = |c: [f32; 3], amount: f32| {
            let g = (c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11) * 0.6;
            c.map(|v| v * (1.0 - amount) + g * amount)
        };
        let to_linear = |c: [f32; 3]| c.map(|v| v.powf(2.2));
        let sky_k = if dim.has_skylight { day_k } else { 1.0 };
        let mut zenith = to_linear(grey(grey(scale(sky_hex, sky_k), rain * 0.75), thunder * 0.75));
        let weather_fog = (1.0 - rain * 0.5) * (1.0 - thunder * 0.5);
        let mut horizon = to_linear(scale(fog_hex, (0.06 + 0.94 * sky_k) * weather_fog));
        let rd = self.config.video.render_distance as f32 * 16.0;
        let (mut fog_start, mut fog_end) = if v.fog { (rd * (v.fog_start as f32 / 100.0).min(0.98), rd * 0.98) } else { (rd * 4.0, rd * 5.0) };
        if !dim.has_skylight {
            zenith = srgb_to_linear(fog_hex);
            horizon = zenith;
        }
        // A lightning flash whitens the sky for a moment.
        if session.sky_flash > 0 && dim.has_skylight && !v.no_lightning_flash {
            let flash = (session.sky_flash as f32 - partial as f32).min(1.0) * 0.45;
            let white = srgb_to_linear(0xCCCCFF);
            for i in 0..3 {
                zenith[i] += (white[i] - zenith[i]) * flash;
                horizon[i] += (white[i] - horizon[i]) * flash;
            }
        }
        let data = lattice_data::game();
        if world.block(bx, by, bz).is_some_and(|s| data.state(s).fluid == 1) {
            let water_fog = biome.and_then(|b| b.water_fog_color).unwrap_or(0x050533);
            horizon = to_linear(scale(water_fog, 0.2 + 0.8 * day_k));
            zenith = horizon;
            (fog_start, fog_end) = (2.0, 48.0);
        }
        // Some things swallow the view in a fog of their own.
        let eye_block = world.block(bx, by, bz);
        if session.player.game_mode != 3 {
            if eye_block.is_some_and(|s| data.state(s).fluid == 2) {
                let fire_proof = session.player.has_effect("fire_resistance");
                (fog_start, fog_end) = if fire_proof { (0.0, 5.0) } else { (0.25, 1.0) };
                horizon = srgb_to_linear(0x991900);
                zenith = horizon;
            } else if eye_block.is_some_and(|s| data.block(s).name == "powder_snow") {
                (fog_start, fog_end) = (0.0, 2.0);
                horizon = srgb_to_linear(0x9FBBCC);
                zenith = horizon;
            }
        }
        if let Some(duration) = session.player.effect_duration("blindness") {
            let d = if duration < 0 { 5.0 } else { rd + (5.0 - rd) * (duration as f32 / 20.0).min(1.0) };
            (fog_start, fog_end) = (d * 0.25, d);
            horizon = [0.0, 0.0, 0.0];
            zenith = horizon;
        } else if session.player.has_effect("darkness") {
            (fog_start, fog_end) = (15.0 * 0.75, 15.0);
            horizon = [0.0, 0.0, 0.0];
            zenith = horizon;
        }
        // A boss's fog closes in, whatever else is going on.
        if session.boss_bars.iter().any(|(_, b)| b.flags & 4 != 0) {
            fog_start = fog_start.min(10.0);
            fog_end = fog_end.min(96.0);
        }

        // Outline of the targeted block.
        let mut lines = Vec::new();
        if let Some(hit) = session.player.target {
            if let Some(state) = world.block_at(hit.pos) {
                let off = lattice_data::block_offset(data.block(state), hit.pos.x, hit.pos.y, hit.pos.z);
                let base = DVec3::new(hit.pos.x as f64, hit.pos.y as f64, hit.pos.z as f64) + DVec3::from(off) - eye;
                for b in data.outline(state) {
                    push_box_lines(&mut lines, base + DVec3::from(b.min), base + DVec3::from(b.max), if v.bold_outline { [255, 255, 255, 255] } else { [16, 22, 20, 200] });
                }
            }
        }
        // Details the settings can turn on.
        if self.config.debug.hitboxes {
            for e in session.entities.map.values() {
                let (w, h) = e.dimensions();
                let p = e.prev_pos.lerp(e.pos, partial) - eye;
                push_box_lines(&mut lines, p - DVec3::new(w / 2.0, 0.0, w / 2.0), p + DVec3::new(w / 2.0, h, w / 2.0), [255, 255, 255, 200]);
            }
        }
        if self.config.debug.chunk_borders {
            let (cx, cz) = ((eye.x.floor() as i32) >> 4, (eye.z.floor() as i32) >> 4);
            let (y0, y1) = (world.geometry.min_y as f64, (world.geometry.min_y + world.geometry.height) as f64);
            for (ox, oz) in [(0, 0), (16, 0), (0, 16), (16, 16)] {
                let base = DVec3::new((cx * 16 + ox) as f64, 0.0, (cz * 16 + oz) as f64) - eye;
                let c = |y: f64| ColorVertex { pos: [base.x as f32, (y - eye.y) as f32, base.z as f32], color: [230, 200, 80, 255] };
                lines.push(c(y0));
                lines.push(c(y1));
            }
            let y = eye.y.floor() - eye.y;
            let min = DVec3::new((cx * 16) as f64, 0.0, (cz * 16) as f64) - DVec3::new(eye.x, 0.0, eye.z);
            push_box_lines(&mut lines, min + DVec3::new(0.0, y, 0.0), min + DVec3::new(16.0, y + 16.0, 16.0), [230, 200, 80, 120]);
        }
        // Everything with a body.
        let tris = Vec::new();
        batches.clear();
        if let AssetState::Ready(assets) = &self.assets {
            let rot = Quat::from_rotation_y((180.0 - cam_yaw).to_radians()) * Quat::from_rotation_x((-cam_pitch).to_radians());
            let registries = session.registries.as_deref();
            let mut ctx = crate::bodies::Ctx {
                bodies: &assets.bodies,
                items: Some(&assets.items),
                out: batches,
                registries,
                skins: &self.skins,
                partial: partial as f32,
                day_fraction: session.day_fraction() as f32,
                blocks: &assets.mesh.models,
                camera_rot: rot,
                camera_yaw_rad: (180.0 - cam_yaw).to_radians(),
                camera_pitch_rad: (-cam_pitch).to_radians(),
                camera_yaw_deg: cam_yaw,
                fire: assets.flames,
                to_hand: false,
                lang: &assets.lang,
                unlit: false,
                maps: &session.maps,
                map_textures: &self.map_textures,
                shine: crate::bodies::Shine::at(self.started.elapsed().as_millis() as f64, v.glint_speed as f32 / 100.0, v.glint_strength as f32 / 100.0, assets.atlas_scale),
                shining: false,
                shadows: v.entity_shadows,
                tag_background: 0.25,
            };
            // What's overhead.
            crate::bodies::sky::draw_celestial(&mut ctx, sun, session.day_time, daylight, rain, dim.has_skylight);
            if v.clouds && dim.cloud_height.is_some() {
                let cell_range = v.cloud_range * 16 / 12;
                crate::bodies::sky::draw_clouds(&mut ctx, eye, dim.cloud_height.unwrap_or(192.0), session.ticks as f64 + partial, daylight, rain, cell_range);
            }
            if v.weather {
                crate::bodies::sky::draw_weather(&mut ctx, session, world, eye, rain, session.ticks, partial as f32, v.weather_radius);
            }
            // Cracks on blocks being broken.
            if let Some((pos, progress)) = session.player.digging_progress() {
                if let Some(state) = world.block_at(pos) {
                    let stage = (progress * 10.0) as u8;
                    crate::bodies::sky::draw_cracks(&mut ctx, state, glam::IVec3::new(pos.x, pos.y, pos.z), stage, eye, &assets.cracks);
                }
            }
            for (who, (pos, stage)) in &session.cracks {
                if *who == session.player.entity_id {
                    continue;
                }
                if let Some(state) = world.block_at(*pos) {
                    crate::bodies::sky::draw_cracks(&mut ctx, state, glam::IVec3::new(pos.x, pos.y, pos.z), *stage, eye, &assets.cracks);
                }
            }
            self.particles.draw(&mut ctx, world, eye, partial as f32);
            crate::bodies::sky::draw_border(&mut ctx, session, eye, rd as f64, self.started.elapsed().as_millis() as f64);
            let reach = rd as f64 * v.entity_distance as f64 / 100.0;
            crate::bodies::draw_entities(&mut ctx, session, eye, self.perspective != 0, reach.max(16.0));
            let elapsed = self.started.elapsed().as_secs_f64();
            crate::bodies::blockent::draw_all(&mut ctx, session, world, eye, (rd as f64).min(64.0).max(16.0), elapsed);
            crate::bodies::overlay::draw(&mut ctx, session, world, self.perspective == 0, self.look, assets.flames[1], &assets.mesh.models);
            if self.perspective == 0 && !self.overlays.hide_hud && session.player.game_mode != 3 {
                let p = &session.player;
                let at = p.prev_pos.lerp(p.body.pos, partial) + DVec3::new(0.0, p.eye_height as f64, 0.0);
                let light = crate::bodies::light_at(world, at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
                let (skin, slim) = self.skins.look_of(session.self_uuid(), &assets.bodies.sheet);
                let swing = p.swing.progress(partial as f32);
                let (main_swing, off_swing) = if p.swing.off_hand() { (0.0, swing) } else { (swing, 0.0) };
                let height = |prev: f32, now: f32| 1.0 - (prev + (now - prev) * partial as f32);
                let using = p.using.map(|hand| {
                    let main = hand == 0;
                    let stack = if main { p.shown_main.as_ref() } else { p.shown_off.as_ref() };
                    (main, p.use_ticks as f32 + partial as f32, stack.map_or(72000.0, crate::game::items::use_duration))
                });
                let scoping = using.is_some_and(|(main, _, _)| {
                    let stack = if main { p.shown_main.as_ref() } else { p.shown_off.as_ref() };
                    stack.is_some_and(|s| crate::game::items::item_name(s) == "spyglass")
                });
                let hand = crate::bodies::held::HandState {
                    main: p.shown_main.as_ref(),
                    off: p.shown_off.as_ref(),
                    main_equip: height(p.prev_main_height, p.main_height),
                    off_equip: height(p.prev_off_height, p.off_height),
                    main_swing,
                    off_swing,
                    using,
                    left_handed: self.config.player.main_hand == 0,
                    light,
                    skin,
                    slim,
                    sleeves: self.config.player.skin_parts() & 0x08 != 0,
                    invisible: self.self_invisible(),
                    scoping,
                    pitch: self.look.1,
                };
                crate::bodies::held::draw_first_person(&mut ctx, &hand);
            }
        }
        Some((camera, WorldLook { zenith, horizon, sun, daylight, fog_start, fog_end }, lines, tris))
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        self.frame.record(dt as f32);

        self.pump_assets();
        self.pump_packs();
        self.pump_control();
        self.pump_test_commands();
        self.run_script();
        self.update_game(dt);
        self.report();
        if let Some((end, frames)) = self.pending_end.take() {
            if frames == 0 {
                self.end_session(end);
            } else {
                self.pending_end = Some((end, frames - 1));
            }
        }
        if self.leave_deadline.is_some_and(|d| now >= d) {
            self.exit = true;
        }

        let raw = match (&mut self.egui_state, &self.window) {
            (Some(state), Some(window)) => state.take_egui_input(window),
            _ => self.test_input(),
        };
        let ctx = self.egui_ctx.clone();
        let mut output = ctx.run_ui(raw, |ui| crate::ui::draw(self, ui));
        if let (Some(state), Some(window)) = (&mut self.egui_state, &self.window) {
            state.handle_platform_output(window, output.platform_output);
        }
        let primitives = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);

        let show_world = matches!(self.screen, Screen::InGame | Screen::Settings) || self.pending_end.is_some();
        let mut batches = std::mem::take(&mut self.batches);
        let world = if show_world { self.world_frame(&mut batches) } else { None };
        let elapsed = self.started.elapsed().as_secs_f32();
        if let Some(i) = self.shots.iter().position(|(t, _)| elapsed >= *t) {
            let (_, path) = self.shots.remove(i);
            self.renderer.capture_next_frame(path);
        }
        if self.exit_after.is_some_and(|t| elapsed >= t) {
            self.exit_after = None;
            self.leave();
        }
        if let Some(path) = self.capture_request.take() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            self.renderer.capture_next_world(path);
        }
        let frame_world = world.as_ref().map(|(camera, look, lines, tris)| FrameWorld {
            camera,
            sky_zenith: look.zenith,
            sky_horizon: look.horizon,
            sun_dir: look.sun,
            daylight: look.daylight,
            fog_start: look.fog_start,
            fog_end: look.fog_end,
            min_brightness: self.ambient_floor(),
            gamma: self.config.video.brightness,
            lines,
            triangles: tris,
            batches: &batches,
            hand_fov: 70.0,
        });
        let g = ui::theme::ground();
        let clear = [g.r(), g.g(), g.b()].map(|c| c as f64 / 255.0);
        self.renderer.render(
            frame_world.as_ref(),
            EguiFrame { primitives: &primitives, textures: &mut output.textures_delta, pixels_per_point: output.pixels_per_point },
            clear,
        );
        drop(frame_world);
        drop(world);
        self.batches = batches;
    }

    fn self_invisible(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.self_flags & 0x20 != 0)
    }

    /// Closes the window without waiting for anything.
    pub fn close_now(&mut self) {
        self.exit = true;
    }

    // -------------------------------------------------------------- headless

    fn test_input(&mut self) -> egui::RawInput {
        let (w, h) = self.renderer.size();
        // A held modifier lasts for the frame of its click.
        if self.test_events.is_empty() && !self.test_modifiers.is_none() {
            self.test_modifiers = egui::Modifiers::NONE;
            self.test_events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        }
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
            events: std::mem::take(&mut self.test_events),
            time: Some(self.started.elapsed().as_secs_f64()),
            focused: true,
            ..Default::default()
        }
    }

    fn pump_test_commands(&mut self) {
        use crate::dev::TestCommand as T;
        while let Ok(c) = self.test_commands.try_recv() {
            match c {
                T::Key { code, down, text } => {
                    let typed = text.as_deref().and_then(|t| t.chars().next());
                    // A real key reaches the interface as well as the game: the same
                    // press has to be seen by both, or tests miss them disagreeing.
                    if let Some(key) = egui::Key::from_name(code.strip_prefix("Key").or(code.strip_prefix("Digit")).unwrap_or(&code)) {
                        self.test_events.push(egui::Event::Key { key, physical_key: Some(key), pressed: down, repeat: false, modifiers: egui::Modifiers::NONE });
                    }
                    if let Some(k) = crate::input::key_by_name(&code) {
                        self.on_key(k, typed, down);
                    } else if code == "Escape" {
                        self.on_key(KeyCode::Escape, None, down);
                    } else {
                        tracing::warn!("test command names an unknown key {code}");
                    }
                }
                T::Button { button, down } => self.on_mouse_button(button, down),
                T::Wheel { delta } => {
                    let menu = self.session.as_ref().is_some_and(|s| s.inventory.is_open() || s.modal.is_some());
                    if menu {
                        self.test_events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, -(delta as f32)), phase: egui::TouchPhase::Move, modifiers: Default::default(), source: egui::MouseWheelSource::Unknown });
                    } else if let Some(s) = &mut self.session {
                        let slot = s.player.selected_slot + delta;
                        s.player.select_slot(&s.conn, slot);
                    }
                }
                T::Look { yaw, pitch } => self.look = (yaw, pitch),
                T::TestCape => {
                    if let (AssetState::Ready(a), Some(s)) = (&self.assets, &self.session) {
                        let img = image::RgbaImage::from_fn(64, 32, |x, y| image::Rgba([120 + (x * 2) as u8, 40, 60 + (y * 4) as u8, 255]));
                        self.skins.set_test_cape(s.self_uuid(), &img, &a.bodies.sheet, &mut self.renderer);
                    }
                }
                T::Click { x, y, button, shift } => {
                    let pos = egui::pos2(x, y);
                    let b = match button {
                        1 => egui::PointerButton::Secondary,
                        2 => egui::PointerButton::Middle,
                        _ => egui::PointerButton::Primary,
                    };
                    let modifiers = egui::Modifiers { shift, ..Default::default() };
                    self.test_modifiers = modifiers;
                    self.test_events.push(egui::Event::ModifiersChanged(modifiers));
                    self.test_events.push(egui::Event::PointerMoved(pos));
                    self.test_events.push(egui::Event::PointerButton { pos, button: b, pressed: true, modifiers });
                    self.test_events.push(egui::Event::PointerButton { pos, button: b, pressed: false, modifiers });
                }
                T::Pointer { x, y } => self.test_events.push(egui::Event::PointerMoved(egui::pos2(x, y))),
                T::Type { text } => self.test_events.push(egui::Event::Text(text)),
                T::UiKey { key } => {
                    let k = match key.as_str() {
                        "Enter" => Some(egui::Key::Enter),
                        "Escape" => Some(egui::Key::Escape),
                        "Tab" => Some(egui::Key::Tab),
                        "Up" => Some(egui::Key::ArrowUp),
                        "Down" => Some(egui::Key::ArrowDown),
                        "Backspace" => Some(egui::Key::Backspace),
                        "End" => Some(egui::Key::End),
                        _ => None,
                    };
                    if let Some(k) = k {
                        self.test_events.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
                        self.test_events.push(egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
                    }
                }
                T::Chat { line } => {
                    if let Some(s) = &mut self.session {
                        if let Err(e) = s.send_chat(&line) {
                            tracing::warn!("test chat: {e}");
                        }
                    }
                }
                T::Capture { path, world_only } => {
                    if let Some(p) = std::path::Path::new(&path).parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    if world_only {
                        self.renderer.capture_next_world(PathBuf::from(path));
                    } else {
                        self.renderer.capture_next_frame(PathBuf::from(path));
                    }
                }
                T::Dump { id } => crate::dev::say(&self.dump(id)),
                T::Resize { w, h } => self.renderer.resize(w, h),
                T::Fps { fps } => self.test_frame_time = Some(if fps > 0 { Duration::from_secs_f64(1.0 / fps as f64) } else { Duration::ZERO }),
                T::Quit => self.leave(),
            }
        }
    }

    fn dump(&self, id: u64) -> crate::dev::Dump {
        use crate::dev::{Dump, DumpEntity, DumpSlot};
        use crate::game::items;
        let tr = self.lang();
        let mut d = Dump { dump: id, fps: self.frame.fps, ..Default::default() };
        d.screen = match &self.screen {
            Screen::Loading => "loading".into(),
            Screen::Connecting => "connecting".into(),
            Screen::Settings => "settings".into(),
            Screen::Ended { title, reason, detail } => {
                let why = reason.as_ref().map(|r| r.to_plain(tr)).or_else(|| detail.clone()).unwrap_or_default();
                format!("ended: {title}: {why}")
            }
            Screen::InGame => "in_game".into(),
        };
        d.widgets = crate::dev::take_widgets();
        for (flag, name) in [(self.overlays.chat_open, "chat"), (self.overlays.paused, "paused"), (self.overlays.debug, "debug")] {
            if flag {
                d.overlays.push(name.to_string());
            }
        }
        let Some(s) = &self.session else { return d };
        d.phase = format!("{:?}", s.phase);
        let b = &s.player.body;
        if s.player.received_position {
            d.pos = Some(b.pos.to_array());
            d.vel = Some(b.vel.to_array());
        }
        d.yaw = b.yaw;
        d.pitch = b.pitch;
        d.on_ground = b.on_ground;
        d.in_water = b.in_water;
        d.swimming = b.is_swimming();
        d.sprinting = b.sprinting;
        d.flying = b.flying;
        d.pose = format!("{:?}", b.pose);
        d.game_mode = s.player.game_mode;
        d.health = s.vitals.health;
        d.food = s.vitals.food;
        d.dead = s.dead.is_some();
        d.selected_slot = s.player.selected_slot;
        d.dimension = s.world.as_ref().map(|w| w.dimension.name.clone()).unwrap_or_default();
        d.menu = s.inventory.menu.as_ref().map(|m| m.kind.clone());
        d.menu_title = s.inventory.menu.as_ref().map(|m| m.title.to_plain(tr));
        d.debug = self.overlays.debug;
        d.own_screen = s.inventory.own_screen_open;
        d.swing = s.player.swing.progress(1.0);
        let dump_stack = |slot: usize, stack: &lattice_proto::ItemStack| DumpSlot {
            slot,
            item: items::item_name(stack).to_string(),
            count: stack.count,
            damage: items::damage(stack),
            name: items::display_name(stack, tr),
        };
        let (_, _, view) = s.inventory.view();
        d.slots = view.iter().enumerate().filter_map(|(i, x)| x.as_ref().map(|st| dump_stack(i, st))).collect();
        d.cursor = s.inventory.cursor.as_ref().map(|st| dump_stack(0, st));
        if let Some(a) = &self.audio {
            d.sounds_started = a.started.load(std::sync::atomic::Ordering::Relaxed);
            d.sounds_failed = a.failed.load(std::sync::atomic::Ordering::Relaxed);
        }
        d.particles = self.particles.count();
        d.captured = self.input.captured;
        d.advancements = s.advancements.len();
        d.advancements_done = s.advancements.values().filter(|a| a.complete()).count();
        d.stats = s.stats.len();
        d.subtitles = self.overlays.subtitles.iter().map(|x| x.key.clone()).collect();
        d.meshes_uploaded = self.meshes_uploaded;
        if let Some(w) = &s.world {
            let at = s.player.body.block_pos();
            let mut water = 0;
            for x in -5..=5 {
                for y in -2..=5 {
                    for z in -5..=5 {
                        if w.block(at[0] + x, at[1] + y, at[2] + z).is_some_and(|b| lattice_data::game().state(b).fluid != 0) {
                            water += 1;
                        }
                    }
                }
            }
            d.fluid_near = water;
            d.dirty_sections = w.dirty_count();
        }
        d.mesh_in_flight = self.mesher.as_ref().map_or(0, |m| m.in_flight());
        d.on_fire = s.self_flags & 1 != 0;
        d.eye_in_water = s.player.body.eye_in_water;
        d.book_rows = self.menu.book.names();
        d.book_search = self.menu.book.search.clone();
        d.recipe_kinds = {
            let categories = lattice_data::game().registry("recipe_book_category");
            let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
            for r in &s.recipes {
                *counts.entry(categories.get(r.category as usize).cloned().unwrap_or_else(|| format!("#{}", r.category))).or_default() += 1;
            }
            counts
        };
        d.cut_choices = s.inventory.menu.as_ref().and_then(|m| m.slots.first().cloned().flatten()).map_or(0, |stack| s.cutter.iter().filter(|c| s.item_in_set(&c.input, stack.item)).count());
        d.chat_input = self.overlays.chat_input.clone();
        d.suggestions = self.overlays.suggest.list.iter().map(|(t, _)| t.clone()).collect();
        let ridden = s.player.vehicle.and_then(|v| s.entities.map.get(&v));
        d.vehicle = ridden.map(|e| e.type_name().to_string());
        d.vehicle_pos = ridden.map(|e| e.pos.to_array());
        d.driving = s.player.ride.as_ref().map(|r| format!("{:?}", r.kind));
        d.jump_scale = s.player.ride.as_ref().map_or(0.0, |r| r.jump_scale);
        d.paused = self.overlays.paused;
        d.modal = s.modal.as_ref().map(|m| {
            match m {
                crate::game::Modal::Sign { .. } => "sign",
                crate::game::Modal::Book { .. } => "book",
                crate::game::Modal::WriteBook { .. } => "write_book",
                crate::game::Modal::Advancements { .. } => "advancements",
                crate::game::Modal::Stats { .. } => "stats",
                crate::game::Modal::Dialog { .. } => "dialog",
                crate::game::Modal::Links => "links",
                crate::game::Modal::Link { .. } => "link",
                crate::game::Modal::Block(_) => "block",
                crate::game::Modal::GameMode { .. } => "game_mode",
                crate::game::Modal::GameRules { .. } => "game_rules",
                crate::game::Modal::Credits { .. } => "credits",
                crate::game::Modal::Teleport => "teleport",
            }
            .to_string()
        });
        d.modal_text = match &s.modal {
            Some(crate::game::Modal::Sign { lines, .. }) => lines.to_vec(),
            Some(crate::game::Modal::Book { pages, .. }) => pages.iter().map(|p| p.to_plain(tr)).collect(),
            Some(crate::game::Modal::WriteBook { pages, .. }) => pages.clone(),
            Some(crate::game::Modal::Dialog { body, .. }) => body.iter().map(|t| t.to_plain(tr)).collect(),
            _ => Vec::new(),
        };
        d.modal_page = match &s.modal {
            Some(crate::game::Modal::Book { page, .. }) => *page,
            _ => 0,
        };
        d.links = s.server_links.len();
        d.waypoints = s.waypoints.len();
        d.boss_bars = s.boss_bars.iter().map(|(_, b)| format!("{} {:.2}", b.name.to_plain(tr), b.progress)).collect();
        if let Some((title, rows)) = s.scoreboard.sidebar(tr) {
            let plain = |spans: &[lattice_proto::Span]| spans.iter().map(|x| x.text.as_str()).collect::<String>();
            d.sidebar.push(plain(&title));
            for (name, score) in rows {
                d.sidebar.push(format!("{} {}", plain(&name), score.map_or(String::new(), |x| plain(&x))));
            }
        }
        d.tab = s.tab.values().map(|e| s.scoreboard.decorated(&e.name, tr).iter().map(|x| x.text.as_str()).collect::<String>()).collect();
        d.maps = s.maps.len();
        d.recipes = s.recipes.len();
        d.cutter_recipes = s.cutter.len();
        d.tick_rate = s.tick_rate;
        d.border_size = s.border.size;
        d.chat = s.chat.lines.iter().rev().take(40).map(|l| l.text.to_plain(tr)).collect::<Vec<_>>().into_iter().rev().collect();
        if let (Some(hit), Some(w)) = (s.player.target, &s.world) {
            d.target_block = w.block_at(hit.pos).map(|st| lattice_data::game().block(st).name.clone());
        }
        d.target_entity = s.player.target_entity.map(|(id, _)| id);
        d.entities = s
            .entities
            .map
            .iter()
            .map(|(id, e)| DumpEntity { id: *id, kind: e.type_name().to_string(), pos: e.pos.to_array(), health: e.float_field("DATA_HEALTH_ID"), hurt_ticks: e.hurt_ticks })
            .collect();
        d.columns = s.world.as_ref().map_or(0, |w| w.column_count());
        d
    }
}

struct WorldLook {
    zenith: [f32; 3],
    horizon: [f32; 3],
    sun: Vec3,
    daylight: f32,
    fog_start: f32,
    fog_end: f32,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Takes the game's formatting codes out of a line of its own text.
fn strip_codes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn push_box_lines(out: &mut Vec<ColorVertex>, min: DVec3, max: DVec3, color: [u8; 4]) {
    let e = 0.002;
    let (a, b) = (min - DVec3::splat(e), max + DVec3::splat(e));
    let c = |x: f64, y: f64, z: f64| ColorVertex { pos: [x as f32, y as f32, z as f32], color };
    let corners = [
        c(a.x, a.y, a.z),
        c(b.x, a.y, a.z),
        c(b.x, a.y, b.z),
        c(a.x, a.y, b.z),
        c(a.x, b.y, a.z),
        c(b.x, b.y, a.z),
        c(b.x, b.y, b.z),
        c(a.x, b.y, b.z),
    ];
    for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
        out.push(corners[i]);
        out.push(corners[j]);
    }
}

struct Runner {
    app: Option<App>,
    start: Option<(Paths, Config, LaunchSpec)>,
    proxy: winit::event_loop::EventLoopProxy<crate::a11y::UserEvent>,
}

/// Runs a game window until it's closed.
pub fn run(paths: Paths, config: Config, spec: LaunchSpec) -> anyhow::Result<()> {
    let event_loop = EventLoop::<crate::a11y::UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner { app: None, start: Some((paths, config, spec)), proxy: event_loop.create_proxy() };
    event_loop.run_app(&mut runner)?;
    Ok(())
}

/// Runs a game with no window until it's told to quit. See dev.rs.
pub fn run_headless(paths: Paths, config: Config, spec: LaunchSpec, width: u32, height: u32) -> anyhow::Result<()> {
    let mut app = App::new_headless(paths, config, spec, width, height)?;
    while !app.exit {
        app.redraw();
        let wait = app.test_frame_time.unwrap_or(Duration::from_millis(16));
        if wait.is_zero() {
            // As fast as it'll go, and counting what the graphics card does too.
            app.renderer.wait_idle();
        } else {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}

impl ApplicationHandler<crate::a11y::UserEvent> for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        let Some((paths, config, spec)) = self.start.take() else { return };
        match App::new(event_loop, self.proxy.clone(), paths, config, spec) {
            Ok(app) => self.app = Some(app),
            Err(e) => {
                tracing::error!("couldn't start: {e:#}");
                eprintln!("Lattice couldn't start: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: crate::a11y::UserEvent) {
        let Some(app) = &mut self.app else { return };
        if let Some(state) = &mut app.egui_state {
            crate::a11y::handle(state, &app.egui_ctx, event);
        }
    }

    /// Paces drawing: as fast as allowed with focus, 30 fps without.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = &self.app else { return };
        if let Some(limit) = app.frame_limit() {
            let next = app.last_frame + limit;
            if Instant::now() < next {
                event_loop.set_control_flow(ControlFlow::WaitUntil(next));
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        if let Some(w) = &app.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(app) = &mut self.app else { return };
        let captured = app.input.captured;
        // While the mouse drives the camera, egui doesn't see the pointer.
        let ui_response = if captured && matches!(event, WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }) {
            None
        } else if let (Some(state), Some(window)) = (&mut app.egui_state, &app.window) {
            Some(state.on_window_event(window, &event))
        } else {
            None
        };
        match event {
            WindowEvent::CloseRequested => app.leave(),
            WindowEvent::Resized(s) => app.renderer.resize(s.width, s.height),
            WindowEvent::RedrawRequested => {
                app.redraw();
                if app.exit {
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(focused) => app.set_focused(focused),
            WindowEvent::KeyboardInput { event, .. } => {
                let consumed = ui_response.is_some_and(|r| r.consumed) && !app.input.captured;
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if !event.repeat && (!consumed || code == KeyCode::Escape || !pressed) {
                        let typed = event.text.as_ref().and_then(|t| t.chars().next());
                        app.on_key(code, typed, pressed);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed && crate::input::capturing().is_some() {
                    crate::input::finish_capture(crate::input::bind_from_mouse(button));
                } else if app.input.captured {
                    let index = match button {
                        MouseButton::Left => Some(0),
                        MouseButton::Right => Some(1),
                        MouseButton::Middle => Some(2),
                        MouseButton::Back => Some(3),
                        MouseButton::Forward => Some(4),
                        MouseButton::Other(n) => Some(n.min(32) as u8),
                    };
                    if let Some(i) = index {
                        app.on_mouse_button(i, pressed);
                    }
                } else if !pressed {
                    // A press that ended a captured state still has to let go.
                } else if app.wants_game_input() && !ui_response.is_some_and(|r| r.consumed) {
                    app.capture_mouse();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if app.input.captured => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 40.0,
                };
                let amount = if app.config.controls.whole_wheel_steps && amount != 0.0 { amount.signum() } else { amount };
                app.scroll_acc += amount * app.config.controls.wheel_sensitivity;
                let d = app.scroll_acc.trunc() as i32;
                if d != 0 {
                    app.scroll_acc -= d as f32;
                    if let Some(s) = &mut app.session {
                        let slot = s.player.selected_slot + d;
                        s.player.select_slot(&s.conn, slot);
                    }
                }
            }
            _ => {}
        }
    }

    /// Drop the renderer while the display connection still exists; tearing
    /// down a Vulkan surface after Wayland disconnects crashes the driver.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        drop(self.app.take());
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(app) = &mut self.app else { return };
        if let DeviceEvent::MouseMotion { delta } = event {
            if app.input.captured && app.wants_game_input() {
                let s = app.config.controls.sensitivity;
                let inv_y = if app.config.controls.invert_y { -1.0 } else { 1.0 };
                let inv_x = if app.config.controls.invert_x { -1.0 } else { 1.0 };
                app.look.0 += delta.0 as f32 * s * inv_x;
                app.look.1 = (app.look.1 + delta.1 as f32 * s * inv_y).clamp(-90.0, 90.0);
            }
        }
    }
}
