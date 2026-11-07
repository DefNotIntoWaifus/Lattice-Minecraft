//! The Play page: the place you were last, then every world and server.

use super::{Files, Launcher, PingState, Screen};
use crate::app::frame_path;
use crate::config::SavedServer;
use crate::ui::theme::*;
use crate::ui::{column, widgets};
use egui::{Align2, Color32, CornerRadius, Key, Rect, RichText, Sense, UiBuilder, Vec2, vec2};

/// What the big picture at the top of the page is about.
#[derive(Clone, Copy, PartialEq)]
enum Spot {
    World(usize),
    Server(usize),
}

/// The world or server played most recently.
fn last_spot(app: &Launcher) -> Option<Spot> {
    let world = app.worlds.iter().enumerate().max_by_key(|(_, w)| w.last_played.unwrap_or(0)).map(|(i, w)| (Spot::World(i), w.last_played.unwrap_or(0) as i64));
    let server = app.servers.iter().enumerate().max_by_key(|(_, s)| s.last_joined.unwrap_or(0)).map(|(i, s)| (Spot::Server(i), s.last_joined.unwrap_or(0)));
    match (world, server) {
        (Some((w, a)), Some((s, b))) => Some(if b > a { s } else { w }),
        (Some((w, _)), None) => Some(w),
        (None, Some((s, _))) => Some(s),
        (None, None) => None,
    }
}

fn world_address(w: &super::worlds::World) -> Option<String> {
    w.port.map(|p| format!("127.0.0.1:{p}"))
}

/// The last frame seen at an address, loaded once.
fn frame_texture(app: &mut Launcher, ui: &egui::Ui, address: &str) -> Option<egui::TextureHandle> {
    let path = frame_path(&app.paths.cache, address);
    let key = format!("frame:{}", path.display());
    if let Some(t) = app.textures.get(&key) {
        return Some(t.clone());
    }
    let img = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok())?;
    let small = img.resize(1280, 720, image::imageops::FilterType::Triangle).to_rgba8();
    let ci = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
    let tex = ui.ctx().load_texture(key.clone(), ci, egui::TextureOptions::LINEAR);
    app.textures.insert(key, tex.clone());
    Some(tex)
}

fn seed_of(text: &str) -> u32 {
    text.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

fn world_summary(w: &super::worlds::World) -> String {
    let mode = super::worlds::GAME_MODES.iter().find(|m| m.0 == w.game_mode).map_or(w.game_mode.as_str(), |m| m.1);
    let mode = if w.hardcore { "Hardcore" } else { mode };
    let when = w.last_played.map_or("not played yet".to_string(), |t| format!("played {}", super::worlds::ago(t)));
    format!("{mode}, {when}")
}

pub fn draw(app: &mut Launcher, ui: &mut egui::Ui) {
    let spot = last_spot(app);
    // Enter plays the place in the picture, unless a control has the focus.
    let nothing_focused = ui.ctx().memory(|m| m.focused().is_none());
    let enter = nothing_focused && ui.input(|i| i.key_pressed(Key::Enter));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        hero(app, ui, spot, enter);
        ui.spacing_mut().item_spacing.y = 8.0;
        column(ui, |ui| {
            ui.add_space(20.0);
            update_status(app, ui);
            files_status(app, ui);
            world_status(app, ui);
            running_now(app, ui);
            worlds_section(app, ui);
            ui.add_space(32.0);
            servers_section(app, ui);
            ui.add_space(40.0);
        });
    });
}

/// A newer Lattice, offered once and never in the way.
fn update_status(app: &mut Launcher, ui: &mut egui::Ui) {
    use super::update::Update;
    match app.updater.state.clone() {
        Update::None => return,
        Update::Available(r) => {
            widgets::subheading(ui, format!("{} is available", r.title));
            ui.horizontal(|ui| {
                if widgets::primary(ui, "Install update").clicked() {
                    app.updater.install();
                }
                if widgets::secondary(ui, "What's new").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(r.page.clone()));
                }
            });
        }
        Update::Downloading(r) => {
            widgets::subheading(ui, format!("Downloading {}", r.title));
            widgets::busy(ui);
        }
        Update::Installed(version) => {
            widgets::subheading(ui, format!("Lattice {version} is installed"));
            widgets::muted(ui, "Close Lattice and open it again to start using it.");
        }
        Update::Failed(why) => {
            widgets::subheading(ui, "The update didn't install");
            widgets::muted(ui, why);
        }
    }
    ui.add_space(16.0);
}

/// The big picture: where you were, and the button that takes you back.
fn hero(app: &mut Launcher, ui: &mut egui::Ui, spot: Option<Spot>, enter: bool) {
    let width = ui.available_width();
    let height = (ui.ctx().content_rect().height() * 0.46).clamp(260.0, 460.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let (title, line, seed, address) = match spot {
        Some(Spot::World(i)) => {
            let w = &app.worlds[i];
            (w.name.clone(), world_summary(w), seed_of(&w.name), world_address(w))
        }
        Some(Spot::Server(i)) => {
            let s = &app.servers[i];
            let when = s.last_joined.map_or("Not joined yet".to_string(), |t| format!("Last joined {}", ago(t)));
            let status = match app.pings.get(&s.address) {
                Some(PingState::Done(st)) => match (st.players_online, st.players_max) {
                    (Some(o), Some(m)) => format!("{when}, {} of {} players online", group(o), group(m)),
                    _ => format!("{when}, online"),
                },
                Some(PingState::Failed(_)) => format!("{when}, not responding right now"),
                _ => when,
            };
            (s.name.clone(), status, seed_of(&s.address), Some(s.address.clone()))
        }
        None => ("Build your first world".to_string(), "It runs on this computer, just for you. You can also join a server below.".to_string(), 7, None),
    };
    let frame = address.as_deref().and_then(|a| frame_texture(app, ui, a));
    let painter = ui.painter_at(rect);
    // Over a picture the text is always white on a dark fade; over the drawing
    // it uses the theme's own colours.
    let (title_color, line_color, fade) = match &frame {
        Some(t) => {
            widgets::image_cover(&painter, rect, t);
            (Color32::WHITE, Color32::from_rgb(0xE8, 0xEE, 0xF2), if mode() == Mode::Contrast { HIGH_CONTRAST.ground } else { CYANOTYPE.ground })
        }
        None => {
            widgets::lattice_field(&painter, rect, seed, 30.0);
            (ink(), ink(), ground())
        }
    };
    let strip = Rect::from_min_max(rect.left_bottom() - vec2(0.0, 170.0_f32.min(height)), rect.right_bottom());
    let solid = Rect::from_min_max(rect.left_bottom() - vec2(0.0, 92.0), rect.right_bottom());
    widgets::scrim(&painter, Rect::from_min_max(strip.min, solid.right_top()), fade, 0, 225);
    painter.rect_filled(solid, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(fade.r(), fade.g(), fade.b(), 225));

    let inner = rect.shrink2(vec2(32.0, 22.0));
    let mut play = enter && spot.is_some();
    let mut new_world = false;
    let mut add_server = false;
    let ready = app.files_ready();
    let label = match spot {
        Some(Spot::World(i)) => {
            let open = app.host.as_ref().is_some_and(|h| h.world.folder == app.worlds[i].folder);
            if open { "Open another window" } else { "Play" }
        }
        Some(Spot::Server(i)) => app.plan(&app.servers[i].address.clone()).label,
        None => "New world",
    };
    // Buttons first, at the bottom right; the words get what's left.
    let buttons_h = 52.0;
    let buttons_rect = Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - buttons_h), inner.right_bottom());
    let mut buttons = ui.new_child(UiBuilder::new().max_rect(buttons_rect).layout(egui::Layout::right_to_left(egui::Align::Max)));
    if spot.is_some() {
        let r = widgets::primary_large(&mut buttons, label, ready);
        let spoken = format!("{label}: {title}");
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, &spoken));
        if r.clicked() {
            play = true;
        }
    } else {
        if widgets::primary_large(&mut buttons, label, true).clicked() {
            new_world = true;
        }
        if widgets::secondary_large(&mut buttons, "Add server").clicked() {
            add_server = true;
        }
    }
    let used = buttons.min_rect().width();
    let words_w = (inner.width() - used - 28.0).max(120.0);
    let line_h = SIZE_BODY * 1.4;
    let title_h = SIZE_DISPLAY * 1.25;
    let line_rect = Rect::from_min_size(egui::pos2(inner.left(), inner.bottom() - line_h), vec2(words_w, line_h));
    let title_rect = Rect::from_min_size(egui::pos2(inner.left(), line_rect.top() - title_h), vec2(words_w, title_h));
    for (r, text) in [(title_rect, RichText::new(&title).font(font_bold(SIZE_DISPLAY)).color(title_color)), (line_rect, RichText::new(&line).size(SIZE_BODY).color(line_color))] {
        let mut words = ui.new_child(UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Max)));
        words.add(egui::Label::new(text).truncate());
    }
    if new_world {
        open_new_world(app);
    }
    if add_server {
        open_add_server(app);
    }
    if play {
        match spot {
            Some(Spot::World(i)) => {
                let w = app.worlds[i].clone();
                app.play_world(w);
            }
            Some(Spot::Server(i)) => {
                let s = app.servers[i].clone();
                app.launch(s.name, s.address);
            }
            None => {}
        }
    }
}

pub fn open_new_world(app: &mut Launcher) {
    let form = super::worlds::Form { eula: app.config.worlds.eula_accepted, ..Default::default() };
    app.screen = Screen::NewWorld(form);
}

fn open_add_server(app: &mut Launcher) {
    app.screen = Screen::EditServer { index: None, name: String::new(), address: String::new(), error: None };
}

/// First-run download, or why it failed.
fn files_status(app: &Launcher, ui: &mut egui::Ui) {
    match &app.files {
        Files::Preparing(p, _) => {
            let p = p.lock().unwrap().clone();
            ui.label(RichText::new(&p.step).font(font_bold(SIZE_BODY)));
            match p.fraction {
                Some(f) => widgets::progress(ui, f, ui.available_width().min(520.0)),
                None => {
                    crate::ui::widgets::busy(ui);
                }
            }
            widgets::muted(ui, "Lattice doesn't ship Mojang's art. Block textures come from the official game files, downloaded once from Mojang and kept on this computer.");
            ui.add_space(20.0);
        }
        Files::Failed(e) => {
            widgets::error_text(ui, format!("Couldn't download the game files: {e}"));
            widgets::muted(ui, "Check your connection, then close and reopen Lattice.");
            ui.add_space(20.0);
        }
        Files::Ready => {}
    }
}

/// How the world being opened or closed is doing.
fn world_status(app: &Launcher, ui: &mut egui::Ui) {
    let Some(host) = &app.host else { return };
    ui.label(RichText::new(format!("{}: {}", host.world.name, host.step)).font(font_bold(SIZE_BODY)));
    match host.fraction {
        Some(f) => widgets::progress(ui, f, ui.available_width().min(520.0)),
        None if host.port.is_none() || host.stopping.is_some() => {
            crate::ui::widgets::busy(ui);
        }
        None => {}
    }
    ui.add_space(20.0);
}

/// One line per open game window, so a running session is never out of sight.
fn running_now(app: &mut Launcher, ui: &mut egui::Ui) {
    let mut stop = None;
    let mut any = false;
    for s in app.sessions.iter().filter(|s| s.running()) {
        any = true;
        egui::Frame::new().fill(surface()).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, if s.stopping() { "Stopping" } else if s.ended() { "Close" } else { "Stop" }).clicked() {
                        stop = Some(s.id);
                    }
                    if let Some(st) = s.status.as_ref().filter(|st| st.fps > 0.0 && s.on_server()) {
                        ui.label(RichText::new(format!("{:.0} fps", st.fps)).font(font_bold(SIZE_BODY)).color(accent()));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("{} on {}", s.name(), s.spec.server_name)).font(font_bold(SIZE_BODY))).truncate());
                        ui.add(egui::Label::new(RichText::new(s.state_text()).size(SIZE_SMALL).color(ink_muted())).truncate());
                    });
                });
            });
        });
    }
    if any {
        ui.add_space(24.0);
    }
    if let Some(id) = stop {
        app.stop(id);
    }
}

fn section_header(ui: &mut egui::Ui, title: &str, button: &str) -> bool {
    let mut pressed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(TARGET);
        widgets::subheading(ui, title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pressed = widgets::secondary(ui, button).clicked();
        });
    });
    pressed
}

/// The worlds kept on this computer.
fn worlds_section(app: &mut Launcher, ui: &mut egui::Ui) {
    if section_header(ui, "Your worlds", "New world") {
        open_new_world(app);
    }
    ui.add_space(4.0);
    if app.worlds.is_empty() {
        widgets::muted(ui, "A new world runs on this computer, just for you, and stays here until you delete it.");
        return;
    }
    let can_launch = app.files_ready();
    let open_folder = app.host.as_ref().map(|h| h.world.folder.clone());
    let mut play = None;
    let mut remove = None;
    for (i, w) in app.worlds.clone().iter().enumerate() {
        let thumb = world_address(w).and_then(|a| frame_texture(app, ui, &a));
        egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(112.0, 63.0), Sense::hover());
                match &thumb {
                    Some(t) => widgets::image_cover(ui.painter(), r, t),
                    None => widgets::lattice_field(ui.painter(), r, seed_of(&w.name), 9.0),
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_open = open_folder.as_ref() == Some(&w.folder);
                    let confirm_id = egui::Id::unique("confirm_delete_world");
                    let confirming = ui.ctx().memory(|m| m.data.get_temp::<usize>(confirm_id)) == Some(i);
                    if !is_open && !confirming {
                        let r = widgets::danger_button(ui, "Delete");
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Delete {}", w.name)));
                        if r.clicked() {
                            ui.ctx().memory_mut(|m| m.data.insert_temp(confirm_id, i));
                        }
                    }
                    if confirming {
                        if widgets::secondary(ui, "Keep").clicked() {
                            ui.ctx().memory_mut(|m| m.data.remove::<usize>(confirm_id));
                        }
                        if widgets::danger_button(ui, "Delete for good").clicked() {
                            remove = Some(i);
                            ui.ctx().memory_mut(|m| m.data.remove::<usize>(confirm_id));
                        }
                    } else {
                        let can = can_launch && (open_folder.is_none() || is_open);
                        let r = widgets::secondary_enabled(ui, if is_open { "Open another window" } else { "Play" }, can);
                        crate::dev::note_widget(&format!("play world:{}", w.name), r.rect);
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can, format!("Play {}", w.name)));
                        if r.clicked() {
                            play = Some(i);
                        }
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add_space(10.0);
                        ui.add(egui::Label::new(RichText::new(&w.name).font(font_bold(SIZE_BODY))).truncate());
                        if confirming {
                            ui.add(egui::Label::new(RichText::new("This deletes the world from this computer.").size(SIZE_SMALL).color(danger())).truncate());
                        } else {
                            ui.add(egui::Label::new(RichText::new(world_summary(w)).size(SIZE_SMALL).color(ink_muted())).truncate());
                        }
                    });
                });
            });
        });
        ui.add_space(-6.0);
    }
    if let Some(i) = play {
        let w = app.worlds[i].clone();
        app.play_world(w);
    }
    if let Some(i) = remove {
        let w = app.worlds.remove(i);
        // Only ever a folder Lattice made under its own worlds folder.
        if w.folder.starts_with(super::worlds::worlds_dir(&app.paths)) && w.folder.join("lattice-world.json").exists() {
            match std::fs::remove_dir_all(&w.folder) {
                Ok(()) => app.toast(format!("Deleted {}", w.name), false),
                Err(e) => app.toast(format!("Couldn't delete {}: {e}", w.name), true),
            }
        }
    }
}

enum RowAction {
    Play,
    Edit,
    Remove,
}

/// Saved servers, and joining one by its address.
fn servers_section(app: &mut Launcher, ui: &mut egui::Ui) {
    if section_header(ui, "Servers", "Add server") {
        open_add_server(app);
    }
    ui.add_space(4.0);
    let mut join_direct = false;
    let direct = app.direct_address.trim().to_string();
    let plan = (!direct.is_empty()).then(|| app.plan(&direct));
    let can_launch = app.files_ready();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 110.0).min(420.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut app.direct_address)
                .hint_text(RichText::new("Join by address, such as play.example.net").color(ink_muted()))
                .desired_width(width)
                .margin(vec2(10.0, 8.0))
                .font(font(SIZE_BODY)),
        );
        crate::dev::note_widget("Server address", r.rect);
        widgets::focus_ring(ui, &r);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Join by address"));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            join_direct = true;
        }
        if widgets::secondary_enabled(ui, "Join", can_launch && !direct.is_empty()).clicked() {
            join_direct = true;
        }
    });
    if let Some(note) = plan.as_ref().and_then(|p| p.note.as_ref()) {
        widgets::muted(ui, note);
    }
    if join_direct && !direct.is_empty() {
        let name = app.servers.iter().find(|s| s.address == direct).map_or(direct.clone(), |s| s.name.clone());
        app.launch(name, direct);
    }
    ui.add_space(4.0);

    if app.servers.is_empty() {
        widgets::muted(ui, "Saved servers show here with who's online. Add one, or type an address above to join without saving it.");
        return;
    }
    let mut action = None;
    for i in 0..app.servers.len() {
        if let Some(a) = server_row(app, ui, i) {
            action = Some((i, a));
        }
        ui.add_space(-6.0);
    }
    match action {
        Some((i, RowAction::Play)) => {
            let s = app.servers[i].clone();
            app.launch(s.name, s.address);
        }
        Some((i, RowAction::Edit)) => {
            let s = &app.servers[i];
            app.screen = Screen::EditServer { index: Some(i), name: s.name.clone(), address: s.address.clone(), error: None };
        }
        Some((i, RowAction::Remove)) => {
            let s = app.servers.remove(i);
            crate::config::save_servers(&app.paths.config, &app.servers);
            app.toast(format!("Removed {}", s.name), false);
        }
        None => {}
    }
}

fn server_row(app: &mut Launcher, ui: &mut egui::Ui, i: usize) -> Option<RowAction> {
    let server: SavedServer = app.servers[i].clone();
    // The server's own favicon, or its initial on a plain tile.
    let tex_key = format!("favicon:{}", server.address);
    let favicon = match app.pings.get(&server.address) {
        Some(PingState::Done(s)) => s.favicon.clone(),
        _ => None,
    };
    if let Some(png) = favicon {
        if !app.textures.contains_key(&tex_key) {
            if let Ok(img) = image::load_from_memory(&png) {
                let img = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                app.textures.insert(tex_key.clone(), ui.ctx().load_texture(tex_key.clone(), ci, egui::TextureOptions::NEAREST));
            }
        }
    }
    let icon = app.textures.get(&tex_key).cloned();
    let tr = &lattice_proto::text::NoTranslations;
    // Status is always said in words, never by colour alone.
    let (motd, status, status_color) = match app.pings.get(&server.address) {
        None | Some(PingState::Pending) => (None, "Checking…".to_string(), ink_muted()),
        Some(PingState::Failed(e)) => (None, format!("Not responding: {e}"), danger()),
        Some(PingState::Done(s)) => {
            let players = match (s.players_online, s.players_max) {
                (Some(o), Some(m)) => format!("{} of {} players", group(o), group(m)),
                _ => "Online".to_string(),
            };
            if s.compatible() {
                (Some(s.description.clone()), format!("{players}, {} ms", s.ping_ms), ink_muted())
            } else {
                (Some(s.description.clone()), format!("Runs {}. Lattice plays {}, so it can't join.", plain_version(&s.version_name), lattice_proto::GAME_VERSION), warn())
            }
        }
    };
    let plan = app.plan(&server.address);
    let here = app.sessions_on(&server.address);
    let ready = app.files_ready();
    let confirm_id = egui::Id::unique("confirm_remove_server");
    let mut action = None;
    let frame = egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            match &icon {
                Some(t) => {
                    ui.painter().image(t.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, raised());
                    let initial = server.name.chars().next().unwrap_or('?').to_uppercase().to_string();
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, font_bold(SIZE_TITLE), ink());
                }
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let confirming = ui.ctx().memory(|m| m.data.get_temp::<usize>(confirm_id)) == Some(i);
                if confirming {
                    if widgets::secondary(ui, "Keep").clicked() {
                        ui.ctx().memory_mut(|m| m.data.remove::<usize>(confirm_id));
                    }
                    if widgets::danger_button(ui, "Remove from list").clicked() {
                        action = Some(RowAction::Remove);
                        ui.ctx().memory_mut(|m| m.data.remove::<usize>(confirm_id));
                    }
                } else {
                    let r = widgets::secondary(ui, "Remove");
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Remove {}", server.name)));
                    if r.clicked() {
                        ui.ctx().memory_mut(|m| m.data.insert_temp(confirm_id, i));
                    }
                    let r = widgets::secondary(ui, "Edit");
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Edit {}", server.name)));
                    if r.clicked() {
                        action = Some(RowAction::Edit);
                    }
                    let r = widgets::secondary_enabled(ui, plan.label, ready);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, format!("{}: {}", plan.label, server.name)));
                    if r.clicked() {
                        action = Some(RowAction::Play);
                    }
                }
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(&server.name).font(font_bold(SIZE_BODY))).truncate());
                    if let Some(m) = &motd {
                        let job = widgets::spans_job(&m.spans(tr), SIZE_SMALL, ink_muted(), ui.available_width());
                        ui.add(egui::Label::new(egui::WidgetText::LayoutJob(job.into())).truncate());
                    }
                    ui.add(egui::Label::new(RichText::new(&status).size(SIZE_SMALL).color(status_color)).truncate());
                    let extra = match (here, server.last_joined) {
                        (1, _) => Some("1 of your sessions is here".to_string()),
                        (n, _) if n > 1 => Some(format!("{n} of your sessions are here")),
                        (_, Some(t)) if motd.is_none() => Some(format!("Last joined {}", ago(t))),
                        _ => None,
                    };
                    if let Some(extra) = extra {
                        ui.add(egui::Label::new(RichText::new(extra).size(SIZE_SMALL).color(accent())).truncate());
                    }
                });
            });
        });
        if let Some(note) = &plan.note {
            widgets::muted(ui, note);
        }
    });
    crate::dev::note_widget(&format!("server:{}", server.name), frame.response.rect);
    action
}

/// The form for a new world.
pub fn new_world(app: &mut Launcher, ui: &mut egui::Ui, mut form: super::worlds::Form) {
    use super::worlds::{DIFFICULTIES, GAME_MODES, KINDS};
    let mut create = false;
    let mut cancel = ui.input(|i| i.key_pressed(Key::Escape));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        column(ui, |ui| {
            ui.add_space(28.0);
            widgets::heading(ui, "New world");
            ui.add_space(14.0);
            let r = widgets::field(ui, "Name", &mut form.name, "", 420.0);
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                create = true;
            }
            ui.add_space(12.0);
            form_label(ui, "Game mode");
            let modes: Vec<&str> = GAME_MODES.iter().map(|m| m.1).collect();
            ui.horizontal(|ui| {
                widgets::segmented(ui, "Game mode", &modes, &mut form.game_mode);
            });
            widgets::muted(ui, GAME_MODES[form.game_mode].2);
            ui.add_space(12.0);
            let hardcore = GAME_MODES[form.game_mode].0 == "hardcore";
            form_label(ui, "Difficulty");
            if hardcore {
                widgets::muted(ui, "Hardcore is always played on Hard.");
            } else {
                let names: Vec<&str> = DIFFICULTIES.iter().map(|d| d.1).collect();
                ui.horizontal(|ui| {
                    widgets::segmented(ui, "Difficulty", &names, &mut form.difficulty);
                });
            }
            ui.add_space(12.0);
            form_label(ui, "World type");
            let kinds: Vec<&str> = KINDS.iter().map(|k| k.1).collect();
            ui.horizontal(|ui| {
                widgets::segmented(ui, "World type", &kinds, &mut form.kind);
            });
            ui.add_space(12.0);
            widgets::field(ui, "Seed", &mut form.seed, "Leave empty for a random world", 420.0);
            ui.add_space(14.0);
            widgets::switch_line(ui, &mut form.allow_commands, "Allow commands such as /give and /gamemode");
            ui.add_space(4.0);
            widgets::switch_line(ui, &mut form.eula, "I accept the Minecraft End User License Agreement");
            ui.hyperlink_to(RichText::new("Read the agreement").size(SIZE_SMALL).color(accent()).underline(), "https://aka.ms/MinecraftEULA");
            widgets::muted(ui, "Worlds run on Mojang's own server program. Lattice downloads it the first time, about 170 MB with the Java it needs. The license is theirs.");
            if let Some(e) = &form.error {
                ui.add_space(6.0);
                widgets::error_text(ui, e);
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if widgets::primary(ui, "Create and play").clicked() {
                    create = true;
                }
                if widgets::secondary(ui, "Cancel").clicked() {
                    cancel = true;
                }
            });
            ui.add_space(32.0);
        });
    });
    if cancel {
        app.screen = Screen::Play;
        return;
    }
    if create {
        match super::worlds::create(&app.paths, &form) {
            Ok(world) => {
                app.config.worlds.eula_accepted = true;
                app.save_config();
                app.worlds = super::worlds::list(&app.paths);
                app.screen = Screen::Play;
                app.play_world(world);
                return;
            }
            Err(e) => form.error = Some(e),
        }
    }
    app.screen = Screen::NewWorld(form);
}

fn form_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(SIZE_SMALL).color(ink()));
}

fn plain_version(v: &str) -> String {
    lattice_proto::Text::literal(v).to_plain(&lattice_proto::text::NoTranslations)
}

fn group(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn ago(unix: i64) -> String {
    super::worlds::ago(unix.max(0) as u64)
}

pub fn edit_server(app: &mut Launcher, ui: &mut egui::Ui) {
    let Screen::EditServer { index, mut name, mut address, mut error } = app.screen.clone() else { return };
    let mut done = false;
    let mut cancel = ui.input(|i| i.key_pressed(Key::Escape));
    column(ui, |ui| {
        ui.add_space(28.0);
        widgets::heading(ui, if index.is_some() { "Edit server" } else { "Add server" });
        ui.add_space(14.0);
        widgets::field(ui, "Name", &mut name, "My server", 420.0);
        ui.add_space(8.0);
        let r = widgets::field(ui, "Address", &mut address, "play.example.net or 192.168.1.20:25565", 420.0);
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            done = true;
        }
        widgets::muted(ui, "Leave out the port unless the server owner gave you one.");
        if let Some(e) = &error {
            ui.add_space(4.0);
            widgets::error_text(ui, e);
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if widgets::primary(ui, "Save server").clicked() {
                done = true;
            }
            if widgets::secondary(ui, "Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if cancel {
        app.screen = Screen::Play;
        return;
    }
    if done {
        let addr = address.trim().to_string();
        match lattice_net::ServerAddress::parse(&addr) {
            Err(e) => error = Some(e),
            Ok(_) => {
                let entry = SavedServer {
                    name: if name.trim().is_empty() { addr.clone() } else { name.trim().to_string() },
                    address: addr.clone(),
                    last_joined: index.and_then(|i| app.servers.get(i)).and_then(|s| s.last_joined),
                };
                match index {
                    Some(i) if i < app.servers.len() => app.servers[i] = entry,
                    _ => app.servers.push(entry),
                }
                crate::config::save_servers(&app.paths.config, &app.servers);
                app.ping(addr);
                app.screen = Screen::Play;
                return;
            }
        }
    }
    app.screen = Screen::EditServer { index, name, address, error };
}
