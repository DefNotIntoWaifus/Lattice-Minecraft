//! Settings. Every option here changes something real; options for features that aren't
//! built yet don't appear until they are. The launcher and the game window show the same
//! panel: a search box, sections down the side, and the options of the chosen section.

use super::theme::*;
use super::widgets;
use crate::app::{App, Screen};
use crate::config::{Config, Paths};
use crate::input::{self, Action, Bind, Binds};
use egui::{CornerRadius, Rect, RichText, Role, Sense, Stroke, StrokeKind, Vec2, WidgetInfo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Display,
    Quality,
    Camera,
    Controls,
    Keys,
    Audio,
    Chat,
    Player,
    Packs,
    Interface,
    Debug,
}

/// The sound section appears once there's sound for it to control.
const SOUND_BUILT: bool = true;

const SECTIONS: [(Section, &str); 11] = [
    (Section::Display, "Display"),
    (Section::Quality, "Quality"),
    (Section::Camera, "Camera and effects"),
    (Section::Controls, "Mouse and movement"),
    (Section::Keys, "Key bindings"),
    (Section::Audio, "Sound"),
    (Section::Chat, "Chat"),
    (Section::Player, "Player"),
    (Section::Packs, "Resource packs"),
    (Section::Interface, "Interface"),
    (Section::Debug, "Details"),
];

/// How a setting is shown.
enum Control {
    Toggle(fn(&mut Config) -> &mut bool),
    SliderF(fn(&mut Config) -> &mut f32, f32, f32, f32, &'static str),
    SliderI(fn(&mut Config) -> &mut i32, i32, i32, &'static str),
    Choice(fn(&mut Config) -> &mut i32, &'static [&'static str]),
    /// Like Choice, for settings stored by name: (stored, shown).
    Named(fn(&mut Config) -> &mut String, &'static [(&'static str, &'static str)]),
}

struct Setting {
    section: Section,
    label: &'static str,
    help: &'static str,
    control: Control,
}

macro_rules! toggle {
    ($s:expr, $l:expr, $h:expr, $($f:tt).+) => {
        Setting { section: $s, label: $l, help: $h, control: Control::Toggle(|c| &mut c.$($f).+) }
    };
}
macro_rules! slider {
    ($s:expr, $l:expr, $h:expr, $($f:tt).+, $min:expr, $max:expr, $step:expr, $suffix:expr) => {
        Setting { section: $s, label: $l, help: $h, control: Control::SliderF(|c| &mut c.$($f).+, $min, $max, $step, $suffix) }
    };
}
macro_rules! slider_i {
    ($s:expr, $l:expr, $h:expr, $($f:tt).+, $min:expr, $max:expr, $suffix:expr) => {
        Setting { section: $s, label: $l, help: $h, control: Control::SliderI(|c| &mut c.$($f).+, $min, $max, $suffix) }
    };
}
macro_rules! choice {
    ($s:expr, $l:expr, $h:expr, $($f:tt).+, $opts:expr) => {
        Setting { section: $s, label: $l, help: $h, control: Control::Choice(|c| &mut c.$($f).+, $opts) }
    };
}

fn settings() -> Vec<Setting> {
    use Section::*;
    vec![
        // ---------------------------------------------------- display
        toggle!(Display, "Fullscreen", "Also on F11.", video.fullscreen),
        toggle!(Display, "Sync to display (VSync)", "Without it the frame rate isn't held to your display's refresh rate. Turn it off to measure what the GPU can really do.", video.vsync),
        slider_i!(Display, "Frame rate limit", "0 means no limit.", video.max_fps, 0, 500, " fps"),
        toggle!(Display, "Limit background windows", "Windows without focus draw at the rate below, so side-by-side sessions don't take GPU time from the one you're playing.", video.limit_background_fps),
        slider_i!(Display, "Background frame rate", "Frame rate of a window without focus.", video.background_fps, 1, 120, " fps"),
        toggle!(Display, "Show frame rate", "Frame rate and frame time in the corner of the game window.", video.show_fps),
        // ---------------------------------------------------- quality
        slider_i!(Quality, "Render distance", "Chunks in every direction. Servers can send fewer than you ask for.", video.render_distance, 2, 32, " chunks"),
        slider_i!(Quality, "Entity distance", "How far mobs and objects are drawn, as a percentage of the render distance.", video.entity_distance, 10, 500, "%"),
        toggle!(Quality, "Shadows", "Soft dark circles on the ground under mobs and items.", video.entity_shadows),
        toggle!(Quality, "Clouds", "Blocky clouds drifting overhead.", video.clouds),
        toggle!(Quality, "Particles", "Smoke, sparks, bits of broken block and the like.", video.particles),
        slider_i!(Quality, "Cloud distance", "How far out clouds are drawn, in chunks.", video.cloud_range, 4, 64, ""),
        slider_i!(Quality, "Rain and snow distance", "How far around you it falls, in blocks.", video.weather_radius, 3, 10, ""),
        toggle!(Quality, "Rain and snow", "Falling weather near you. The server decides when it happens.", video.weather),
        toggle!(Quality, "Smooth lighting", "Light blends across block faces. Off is flat and a little faster to build.", video.smooth_lighting),
        slider_i!(Quality, "Corner shadows", "Strength of the shading in the corners between blocks. Needs smooth lighting.", video.ambient_occlusion, 0, 100, "%"),
        slider_i!(Quality, "Biome blend", "How far grass and water colours blend across biome borders, in steps of four blocks. 0 shows hard edges.", video.biome_blend, 0, 7, ""),
        toggle!(Quality, "Detailed leaves", "Draws the leaf faces inside trees. Off is faster, and trees look more solid.", video.detailed_leaves),
        slider_i!(Quality, "Mipmap levels", "Smooths distant textures. 0 keeps every pixel sharp, which shimmers at a distance.", video.mipmap_levels, 0, 4, ""),
        choice!(Quality, "Texture filtering", "Anisotropic keeps ground textures clear at low angles, and softens them up close.", video.anisotropy, &["Off", "2×", "4×", "8×", "16×"]),
        toggle!(Quality, "Fog", "Fades the world's edge into the sky. Off shows where the terrain stops.", video.fog),
        slider_i!(Quality, "Fog start", "Where fog begins, as a percentage of the render distance.", video.fog_start, 0, 100, "%"),
        // ---------------------------------------------------- camera
        slider!(Camera, "Field of view", "Vertical angle in degrees.", video.fov, 30.0, 110.0, 1.0, "°"),
        slider_i!(Camera, "Field of view effects", "How much sprinting and speed effects widen the view.", video.fov_effects, 0, 100, "%"),
        slider_i!(Camera, "Screen distortion", "How much portals and nausea warp the view. Turned down, a pale tint shows instead.", video.distortion, 0, 100, "%"),
        toggle!(Camera, "Bold block outline", "A bright outline around the block you're looking at.", video.bold_outline),
        toggle!(Camera, "Hide lightning flashes", "The sky stays as it is when lightning strikes.", video.no_lightning_flash),
        toggle!(Camera, "View bobbing", "The camera moves with your steps.", video.view_bobbing),
        slider_i!(Camera, "Enchantment shine speed", "How fast the shine moves across enchanted things.", video.glint_speed, 0, 100, "%"),
        slider_i!(Camera, "Enchantment shine strength", "How bright it is.", video.glint_strength, 0, 100, "%"),
        toggle!(Camera, "Dark screen edges", "The edges of the view darken in dark places.", video.vignette),
        slider_i!(Camera, "Damage tilt", "How much the camera tips when you're hurt.", video.damage_tilt, 0, 100, "%"),
        slider!(Camera, "Brightness", "From moody to bright. Lifts dark areas more than light ones.", video.brightness, 0.0, 1.0, 0.01, ""),
        // ---------------------------------------------------- controls
        slider!(Controls, "Mouse sensitivity", "Degrees turned per pixel of mouse movement.", controls.sensitivity, 0.02, 0.5, 0.01, ""),
        toggle!(Controls, "Invert vertical look", "Moving the mouse up looks down.", controls.invert_y),
        toggle!(Controls, "Invert horizontal look", "Moving the mouse right looks left.", controls.invert_x),
        slider!(Controls, "Scroll sensitivity", "Hotbar slots moved per step of the wheel.", controls.wheel_sensitivity, 0.25, 4.0, 0.25, "×"),
        toggle!(Controls, "Hold to sprint", "Off: the sprint key toggles sprinting.", controls.hold_to_sprint),
        toggle!(Controls, "Hold to attack", "Off: click once to start, again to stop.", controls.hold_to_attack),
        toggle!(Controls, "Hold to use", "Off: click once to start, again to stop.", controls.hold_to_use),
        toggle!(Controls, "One slot per wheel notch", "For wheels that report many small steps.", controls.whole_wheel_steps),
        toggle!(Controls, "Hold to sneak", "Off: the sneak key toggles sneaking.", controls.hold_to_sneak),
        slider_i!(Controls, "Double-tap sprint window", "Ticks allowed between two taps of forward to start sprinting. 0 turns it off.", controls.sprint_window, 0, 20, " ticks"),
        toggle!(Controls, "Auto-jump", "Jumps up one-block steps as you walk into them.", controls.auto_jump),
        // ---------------------------------------------------- audio: filled in by the sound section
        // ---------------------------------------------------- chat
        choice!(Chat, "Chat is", "Hidden still shows messages from commands you ran, if the server sends them that way. This is also what the server is told.", chat.mode, &["Shown", "Commands only", "Hidden"]),
        toggle!(Chat, "Suggest while typing", "Off: the list of what can come next waits for Tab.", chat.command_suggestions),
        toggle!(Chat, "Clickable links", "Links in chat open in your browser when clicked.", chat.links),
        toggle!(Chat, "Ask before opening links", "Shows where a link goes first.", chat.link_prompt),
        slider!(Chat, "Delay", "How long new lines wait before they show.", chat.delay, 0.0, 6.0, 0.1, " s"),
        toggle!(Chat, "Colours", "Show server colours and formatting.", chat.colors),
        slider!(Chat, "Hide lines after", "Seconds a line stays on screen when chat is closed.", chat.fade_after, 3.0, 60.0, 1.0, " s"),
        slider_i!(Chat, "Lines when closed", "", chat.lines_closed, 1, 40, ""),
        slider_i!(Chat, "Lines when open", "", chat.lines_open, 1, 40, ""),
        slider_i!(Chat, "Width", "", chat.width, 260, 900, " px"),
        slider!(Chat, "Text size", "", chat.text_size, 10.0, 24.0, 1.0, " px"),
        slider_i!(Chat, "Text opacity", "", chat.opacity, 10, 100, "%"),
        slider_i!(Chat, "Background opacity", "", chat.background, 0, 100, "%"),
        slider!(Chat, "Line spacing", "", chat.line_spacing, 0.0, 12.0, 1.0, " px"),
        toggle!(Chat, "Filter text", "Asks the server to filter messages, if it can.", chat.text_filtering),
        // ---------------------------------------------------- player
        choice!(Player, "Main hand", "Which hand holds your items, and swings.", player.main_hand, &["Left", "Right"]),
        toggle!(Player, "Cape", "Skin layers others see on you.", player.skin_cape),
        toggle!(Player, "Jacket", "", player.skin_jacket),
        toggle!(Player, "Left sleeve", "", player.skin_left_sleeve),
        toggle!(Player, "Right sleeve", "", player.skin_right_sleeve),
        toggle!(Player, "Left trouser leg", "", player.skin_left_leg),
        toggle!(Player, "Right trouser leg", "", player.skin_right_leg),
        toggle!(Player, "Hat", "", player.skin_hat),
        toggle!(Player, "Appear on server lists", "Lets servers show your name in their player samples.", player.allow_listing),
        // ---------------------------------------------------- interface
        Setting {
            section: Interface,
            label: "Theme",
            help: "System follows your desktop. The game window stays dark over the world, unless you choose high contrast.",
            control: Control::Named(|c| &mut c.interface.theme, &[("system", "System"), ("dark", "Dark"), ("light", "Light"), ("high_contrast", "High contrast")]),
        },
        slider!(Interface, "Interface size", "Size of menus and the HUD, on top of your desktop's scale.", interface.scale, 0.75, 2.0, 0.05, "×"),
        toggle!(Interface, "Reduce motion", "Menus switch instantly and nothing in them moves by itself.", interface.reduce_motion),
        toggle!(Interface, "Check for updates", "When the launcher starts it asks GitHub for the newest Lattice, and offers it if there is one. Nothing else is sent.", interface.check_updates),
        toggle!(Interface, "Pixel font in game", "Menus and chat in the game window use blocky letters. Turn this off for plainer ones.", interface.pixel_font),
        toggle!(Interface, "Attack indicator", "A bar under the crosshair shows how charged your next attack is.", interface.attack_indicator),
        toggle!(Interface, "Show coordinates", "Your position on the HUD, without opening the details.", interface.show_coordinates),
        slider!(Interface, "Item name time", "Seconds the held item's name shows when it changes. 0 turns it off.", interface.held_name_time, 0.0, 10.0, 0.5, " s"),
        slider!(Interface, "Notification time", "Seconds messages from Lattice stay on screen.", interface.toast_time, 2.0, 30.0, 1.0, " s"),
        // ---------------------------------------------------- details
        toggle!(Debug, "Show entity boxes", "Outlines of mobs and objects, as the crosshair sees them.", debug.hitboxes),
        toggle!(Debug, "Show chunk borders", "Lines around the chunk you're in.", debug.chunk_borders),
    ]
}

fn reset_setting(s: &Setting, c: &mut Config, d: &mut Config) {
    match &s.control {
        Control::Toggle(f) => *f(c) = *f(d),
        Control::SliderF(f, ..) => *f(c) = *f(d),
        Control::SliderI(f, ..) => *f(c) = *f(d),
        Control::Choice(f, _) => *f(c) = *f(d),
        Control::Named(f, _) => *f(c) = f(d).clone(),
    }
}

fn is_default(s: &Setting, c: &mut Config, d: &mut Config) -> bool {
    match &s.control {
        Control::Toggle(f) => *f(c) == *f(d),
        Control::SliderF(f, ..) => *f(c) == *f(d),
        Control::SliderI(f, ..) => *f(c) == *f(d),
        Control::Choice(f, _) => *f(c) == *f(d),
        Control::Named(f, _) => *f(c) == *f(d),
    }
}

/// Width of the column controls sit in, so they line up from row to row.
const CONTROL_W: f32 = 300.0;
const RESET_W: f32 = 64.0;

/// A settings line: words on the left, the control in a fixed column on the
/// right. In a narrow window the control moves under the words.
fn line(ui: &mut egui::Ui, label: &str, help: &str, control: impl FnOnce(&mut egui::Ui, egui::Id), reset: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let narrow = width < 560.0;
    let words = |ui: &mut egui::Ui, w: f32| -> egui::Id {
        ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing.y = 2.0;
            let id = ui.add(egui::Label::new(RichText::new(label).color(ink())).wrap()).id;
            if !help.is_empty() {
                ui.add(egui::Label::new(RichText::new(help).color(ink_muted()).font(font(SIZE_SMALL))).wrap());
            }
            id
        })
        .inner
    };
    if narrow {
        let id = words(ui, width);
        ui.horizontal(|ui| {
            control(ui, id);
            reset(ui);
        });
    } else {
        let label_w = width - CONTROL_W - RESET_W - 24.0;
        ui.horizontal_top(|ui| {
            let id = words(ui, label_w);
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(egui::vec2(CONTROL_W, TARGET), egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.set_min_width(CONTROL_W);
                control(ui, id);
            });
            ui.allocate_ui_with_layout(egui::vec2(RESET_W, TARGET), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(TARGET);
                reset(ui);
            });
        });
    }
    ui.add_space(10.0);
}

fn slider_width(ui: &mut egui::Ui) {
    // Leaves room for the number box beside the rail.
    ui.spacing_mut().slider_width = (ui.available_width().min(CONTROL_W) - 96.0).max(100.0);
    widgets::slider_look(ui);
}

fn row(ui: &mut egui::Ui, s: &Setting, c: &mut Config, defaults: &mut Config) {
    let default = is_default(s, c, defaults);
    let tag = format!("setting:{}", s.label);
    let mut reset = false;
    line(
        ui,
        s.label,
        s.help,
        |ui, label| match &s.control {
            Control::Toggle(f) => {
                let r = widgets::switch(ui, f(c), s.label);
                crate::dev::note_widget(&tag, r.rect);
            }
            Control::SliderF(f, min, max, step, suffix) => {
                slider_width(ui);
                let decimals = if *step >= 1.0 { 0 } else if *step >= 0.1 { 1 } else { 2 };
                let r = ui.add(egui::Slider::new(f(c), *min..=*max).step_by(*step as f64).fixed_decimals(decimals).suffix(*suffix)).labelled_by(label);
                crate::dev::note_widget(&tag, r.rect);
                widgets::focus_ring(ui, &r);
            }
            Control::SliderI(f, min, max, suffix) => {
                slider_width(ui);
                let r = ui.add(egui::Slider::new(f(c), *min..=*max).suffix(*suffix)).labelled_by(label);
                crate::dev::note_widget(&tag, r.rect);
                widgets::focus_ring(ui, &r);
            }
            Control::Choice(f, options) => {
                let v = f(c);
                if options.len() <= 3 {
                    let mut i = (*v).max(0) as usize;
                    if widgets::segmented(ui, s.label, options, &mut i) {
                        *v = i as i32;
                    }
                    crate::dev::note_widget(&tag, ui.min_rect());
                } else {
                    let current = options.get(*v as usize).copied().unwrap_or("");
                    let r = egui::ComboBox::from_id_salt(s.label).selected_text(current).width(CONTROL_W - 8.0).show_ui(ui, |ui| {
                        for (i, o) in options.iter().enumerate() {
                            ui.selectable_value(v, i as i32, *o);
                        }
                    });
                    crate::dev::note_widget(&tag, r.response.rect);
                }
            }
            Control::Named(f, options) => {
                let v = f(c);
                let mut i = options.iter().position(|o| o.0 == v.as_str()).unwrap_or(0);
                let names: Vec<&str> = options.iter().map(|o| o.1).collect();
                if widgets::segmented(ui, s.label, &names, &mut i) {
                    *v = options[i].0.to_string();
                }
                crate::dev::note_widget(&tag, ui.min_rect());
            }
        },
        |ui| {
            if !default && reset_button(ui, s.label) {
                reset = true;
            }
        },
    );
    if reset {
        reset_setting(s, c, defaults);
    }
}

/// A small link-like reset, named for the setting so a screen reader can tell them apart.
fn reset_button(ui: &mut egui::Ui, what: &str) -> bool {
    let r = ui.add(egui::Button::new(RichText::new("Reset").font(font(SIZE_SMALL)).color(ink())).frame(false).min_size(egui::vec2(52.0, 32.0)));
    widgets::focus_ring(ui, &r);
    r.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Reset {what}")));
    r.clicked()
}

/// One action's row in the key list.
fn key_row(ui: &mut egui::Ui, a: Action, c: &mut Config) {
    let binds = Binds::from_config(&c.controls.binds);
    let bind = binds.get(a);
    let conflicts = binds.conflicts(a);
    let help = if conflicts.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = conflicts.iter().map(|o| o.label()).collect();
        format!("Same key as {}", names.join(", "))
    };
    let mut reset = false;
    line(
        ui,
        &a.label(),
        &help,
        |ui, _| {
            let capturing = input::capturing() == Some(a);
            let label = if capturing { "Press a key…".to_string() } else { bind.label() };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(180.0, TARGET), Sense::click());
            crate::dev::note_widget(&format!("bind:{}", a.id()), rect);
            let fill = if capturing {
                accent_dim()
            } else if resp.hovered() {
                hover()
            } else {
                raised()
            };
            let stroke = if conflicts.is_empty() { Stroke::new(1.0, edge()) } else { Stroke::new(2.0, danger()) };
            if game_look() {
                super::chrome::button(ui.painter(), rect, super::chrome::Face::Plain, resp.hovered(), capturing);
                if capturing {
                    ui.painter().rect_filled(rect.shrink(4.0), CornerRadius::ZERO, fill);
                }
                if !conflicts.is_empty() {
                    ui.painter().rect_stroke(rect.shrink(2.0), CornerRadius::ZERO, stroke, StrokeKind::Inside);
                }
            } else {
                ui.painter().rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), fill);
                ui.painter().rect_stroke(rect, CornerRadius::same(CONTROL_RADIUS), stroke, StrokeKind::Inside);
            }
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &label, font(SIZE_BODY), ink());
            widgets::focus_ring(ui, &resp);
            let spoken = format!("{}: {label}", a.label());
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &spoken));
            if resp.clicked() {
                input::start_capture(a);
            }
        },
        |ui| {
            if bind != input::default_bind(a) && reset_button(ui, &a.label()) {
                reset = true;
            }
        },
    );
    if reset {
        c.controls.binds.remove(&a.id());
    }
}

/// Takes a finished key capture into the settings.
fn apply_capture(c: &mut Config) {
    if let Some((a, b)) = input::take_captured() {
        if b == input::default_bind(a) {
            c.controls.binds.remove(&a.id());
        } else {
            c.controls.binds.insert(a.id(), b.to_config());
        }
    }
}

/// What's chosen and typed in the panel, kept while it's open.
#[derive(Clone)]
pub struct PanelState {
    pub section: Section,
    pub search: String,
}

impl Default for PanelState {
    fn default() -> Self {
        PanelState { section: Section::Display, search: String::new() }
    }
}

fn sound_section(ui: &mut egui::Ui, c: &mut Config) {
    let channels: [(&str, fn(&mut Config) -> &mut f32); 11] = [
        ("Master volume", |c| &mut c.audio.master),
        ("Music", |c| &mut c.audio.music),
        ("Jukeboxes and note blocks", |c| &mut c.audio.records),
        ("Weather", |c| &mut c.audio.weather),
        ("Blocks", |c| &mut c.audio.blocks),
        ("Hostile creatures", |c| &mut c.audio.hostile),
        ("Friendly creatures", |c| &mut c.audio.neutral),
        ("Players", |c| &mut c.audio.players),
        ("Ambient and caves", |c| &mut c.audio.ambient),
        ("Voice", |c| &mut c.audio.voice),
        ("Menus", |c| &mut c.audio.ui),
    ];
    for (label, f) in channels {
        line(
            ui,
            label,
            "",
            |ui, id| {
                slider_width(ui);
                let v = f(c);
                let mut pct = (*v * 100.0).round();
                let r = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").fixed_decimals(0)).labelled_by(id);
                crate::dev::note_widget(&format!("setting:{label}"), r.rect);
                widgets::focus_ring(ui, &r);
                if r.changed() {
                    *v = pct / 100.0;
                }
            },
            |_| {},
        );
    }
    ui.add_space(8.0);
    line(
        ui,
        "Subtitles",
        "Shows what you can hear, and from which side, on the HUD.",
        |ui, _| {
            widgets::switch(ui, &mut c.audio.subtitles, "Subtitles");
        },
        |_| {},
    );
    line(
        ui,
        "Music plays",
        "",
        |ui, _| {
            const OFTEN: [&str; 3] = ["Now and then", "Often", "All the time"];
            let mut i = c.audio.music_frequency.clamp(0, 2) as usize;
            if widgets::segmented(ui, "Music plays", &OFTEN, &mut i) {
                c.audio.music_frequency = i as i32;
            }
        },
        |_| {},
    );
    line(
        ui,
        "Sound comes out of",
        "",
        |ui, _| {
            let shown = if c.audio.device.is_empty() { "The system's default".to_string() } else { c.audio.device.clone() };
            egui::ComboBox::from_id_salt("sound_device").width(CONTROL_W - 8.0).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut c.audio.device, String::new(), "The system's default");
                for name in crate::audio::device_names() {
                    ui.selectable_value(&mut c.audio.device, name.clone(), name.as_str());
                }
            });
        },
        |_| {},
    );
}

fn packs_section(ui: &mut egui::Ui, c: &mut Config, paths: &Paths) {
    line(
        ui,
        "When a server offers a pack",
        "Servers can send their own textures and sounds. Packs only change how things look and sound, and some servers require theirs.",
        |ui, _| {
            let mut i = c.player.server_packs.clamp(0, 1) as usize;
            if widgets::segmented(ui, "When a server offers a pack", &["Use it", "Decline it"], &mut i) {
                c.player.server_packs = i as i32;
            }
        },
        |_| {},
    );
    ui.add_space(8.0);
    ui.label(RichText::new("Your packs").font(font_bold(SIZE_BODY)));
    let folder = crate::packs::user_folder(paths);
    let found = crate::packs::user_packs(paths);
    if found.is_empty() {
        ui.label(RichText::new("Put resource pack files in the packs folder, and they'll appear here to switch on.").color(ink_muted()));
    }
    for name in found {
        let mut on = c.player.resource_packs.contains(&name);
        line(
            ui,
            &name,
            "",
            |ui, _| {
                let r = widgets::switch(ui, &mut on, &name);
                crate::dev::note_widget(&format!("setting:{name}"), r.rect);
                if r.changed() {
                    if on {
                        c.player.resource_packs.push(name.clone());
                    } else {
                        c.player.resource_packs.retain(|n| n != &name);
                    }
                }
            },
            |_| {},
        );
    }
    ui.add_space(8.0);
    if widgets::secondary(ui, "Open the packs folder").clicked() {
        let _ = std::fs::create_dir_all(&folder);
        if let Err(e) = webbrowser::open(&format!("file://{}", folder.display())) {
            tracing::warn!("couldn't open {}: {e}", folder.display());
        }
    }
    widgets::muted(ui, format!("{}", folder.display()));
}

/// The settings themselves. The caller decides what a change means. `back` adds
/// a button in the header; the result says whether it was pressed.
pub fn panel(ui: &mut egui::Ui, c: &mut Config, paths: &Paths, state: &mut PanelState, note: Option<&str>, back: Option<&str>) -> bool {
    apply_capture(c);
    let list = settings();
    let mut defaults = Config::default();
    let mut back_pressed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(TARGET);
        widgets::heading(ui, "Settings");
        ui.add_space(16.0);
        let r = ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(RichText::new("Search settings").color(ink_muted())).desired_width(240.0).font(font(SIZE_BODY)).margin(egui::vec2(10.0, 8.0)));
        crate::dev::note_widget("settings search", r.rect);
        widgets::focus_ring(ui, &r);
        r.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Search settings"));
        if let Some(label) = back {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                back_pressed = widgets::primary(ui, label).clicked();
            });
        }
    });
    if let Some(n) = note {
        widgets::muted(ui, n);
    }
    ui.add_space(12.0);
    let needle = state.search.trim().to_lowercase();
    let avail = ui.available_rect_before_wrap();
    let nav_w = if game_look() { 244.0 } else { 176.0 };
    let nav = Rect::from_min_size(avail.min, Vec2::new(nav_w + 8.0, avail.height()));
    let body = Rect::from_min_max(avail.min + Vec2::new(nav_w + 28.0, 0.0), avail.max);

    // Sections down the side.
    let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(nav).layout(egui::Layout::top_down(egui::Align::Min)));
    nav_ui.spacing_mut().item_spacing.y = 2.0;
    egui::ScrollArea::vertical().id_salt("settings nav").auto_shrink([false, false]).show(&mut nav_ui, |nav_ui| {
        for (s, label) in SECTIONS {
            if s == Section::Audio && !SOUND_BUILT {
                continue;
            }
            let (rect, resp) = nav_ui.allocate_exact_size(Vec2::new(nav_w, TARGET), Sense::click());
            crate::dev::note_widget(&format!("section:{label}"), rect);
            let on = state.section == s && needle.is_empty();
            if game_look() {
                // Stone keys: the open section stays pressed in.
                super::chrome::button(nav_ui.painter(), rect, super::chrome::Face::Plain, resp.hovered() && !on, on);
                if on {
                    nav_ui.painter().rect_filled(rect.shrink(4.0), egui::CornerRadius::ZERO, accent_dim());
                }
            } else if on {
                nav_ui.painter().rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), accent_dim());
            } else if resp.hovered() {
                nav_ui.painter().rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), hover());
            }
            let f = if on { font_bold(SIZE_BODY) } else { font(SIZE_BODY) };
            nav_ui.painter().text(rect.left_center() + Vec2::new(10.0, 0.0), egui::Align2::LEFT_CENTER, label, f, if on { ink() } else { ink_muted() });
            widgets::focus_ring(nav_ui, &resp);
            resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, label));
            if resp.clicked() {
                state.section = s;
                state.search.clear();
            }
        }
        nav_ui.add_space(14.0);
        let confirm = egui::Id::unique("confirm_reset_settings");
        let confirming = nav_ui.ctx().memory(|m| m.data.get_temp::<bool>(confirm)).unwrap_or(false);
        if confirming {
            widgets::muted(nav_ui, "Your names and sign-ins stay.");
            if widgets::danger_button(nav_ui, "Reset all settings").clicked() {
                let keep_player = (c.player.offline_name.clone(), c.player.offline_names.clone());
                let keep_auth = c.auth.clone();
                *c = Config::default();
                (c.player.offline_name, c.player.offline_names) = keep_player;
                c.auth = keep_auth;
                nav_ui.ctx().memory_mut(|m| m.data.remove::<bool>(confirm));
            }
            if widgets::secondary(nav_ui, "Keep my settings").clicked() {
                nav_ui.ctx().memory_mut(|m| m.data.remove::<bool>(confirm));
            }
        } else if widgets::secondary(nav_ui, "Reset everything").clicked() {
            nav_ui.ctx().memory_mut(|m| m.data.insert_temp(confirm, true));
        }
    });

    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt("settings body").auto_shrink([false, false]).show(&mut body_ui, |ui| {
        ui.set_width(ui.available_width() - 16.0);
        if !needle.is_empty() {
            let mut any = false;
            for s in &list {
                if s.label.to_lowercase().contains(&needle) || s.help.to_lowercase().contains(&needle) {
                    row(ui, s, c, &mut defaults);
                    any = true;
                }
            }
            for a in Action::all() {
                if a.label().to_lowercase().contains(&needle) {
                    key_row(ui, a, c);
                    any = true;
                }
            }
            if !any {
                ui.label(format!("No setting matches \"{}\". Try a shorter word.", state.search.trim()));
            }
            return;
        }
        let section_name = SECTIONS.iter().find(|(s, _)| *s == state.section).map_or("", |(_, n)| n);
        widgets::subheading(ui, section_name);
        ui.add_space(10.0);
        match state.section {
            Section::Keys => {
                for (group, actions) in Action::groups() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(group).font(font_bold(SIZE_BODY)));
                    ui.add_space(4.0);
                    for a in actions {
                        key_row(ui, a, c);
                    }
                }
                ui.add_space(10.0);
                if widgets::secondary(ui, "Reset all keys").clicked() {
                    c.controls.binds.clear();
                }
            }
            Section::Audio => sound_section(ui, c),
            Section::Packs => packs_section(ui, c, paths),
            Section::Debug => {
                for s in list.iter().filter(|s| s.section == Section::Debug) {
                    row(ui, s, c, &mut defaults);
                }
                ui.add_space(14.0);
                ui.label(RichText::new("Files").font(font_bold(SIZE_BODY)));
                widgets::muted(ui, format!("Settings, servers and accounts: {}", paths.config.display()));
                widgets::muted(ui, format!("Downloaded game files: {}", paths.cache.display()));
                widgets::muted(ui, format!("Screenshots and logs: {}", paths.data.display()));
                ui.add_space(14.0);
                ui.label(RichText::new("Version").font(font_bold(SIZE_BODY)));
                widgets::muted(ui, format!("Lattice {}, plays Minecraft {} (protocol {})", env!("CARGO_PKG_VERSION"), lattice_proto::GAME_VERSION, lattice_proto::PROTOCOL_VERSION));
            }
            section => {
                for s in list.iter().filter(|s| s.section == section) {
                    row(ui, s, c, &mut defaults);
                }
            }
        }
        ui.add_space(30.0);
    });
    let _ = Bind::None;
    back_pressed
}

/// Settings opened from the pause menu, over the running game.
pub fn in_game(app: &mut App, ui: &mut egui::Ui) {
    let before = app.config.clone();
    let mut leave = false;
    egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(egui::Color32::from_black_alpha(150))).show(ui, |ui| {
        // A big stone slab over the world, so changes to it show around the edges.
        let screen = ui.max_rect();
        let size = egui::vec2((screen.width() - 32.0).min(1240.0), screen.height() - 32.0);
        let slab = egui::Rect::from_min_size((screen.center() - size / 2.0).floor(), size);
        super::chrome::panel(ui.painter(), slab);
        let inner = slab.shrink(22.0);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        let capturing = input::capturing().is_some();
        if !capturing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            leave = true;
        }
        on_panel(&mut ui, |ui| {
            // Scroll areas fade out towards the page's colour, which here is the stone.
            ui.visuals_mut().panel_fill = surface();
            let mut state = std::mem::take(&mut app.settings_panel);
            leave |= panel(ui, &mut app.config, &app.paths, &mut state, None, Some("Back to game"));
            app.settings_panel = state;
        });
    });
    app.apply_config(&before);
    if leave {
        app.config.save(&app.paths.config);
        app.screen = Screen::InGame;
        app.capture_mouse();
    }
}
