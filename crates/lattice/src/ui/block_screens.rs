//! Screens for editing game-master blocks: forms on a stone slab, with what they set named
//! in the game's own words wherever the game has words for it.

use super::chrome;
use super::theme::*;
use super::widgets;
use crate::app::App;
use crate::game::Modal;
use crate::game::gm_blocks::{BlockScreen, CommandScreen, JigsawScreen, StructureScreen, TestInstanceScreen, TestScreen};
use egui::{Rect, Vec2};
use lattice_proto::Translator;
use lattice_proto::packets::{Encoded, play};

/// Numbers for questions about a command block's text, well clear of the chat box's.
const COMMAND_QUESTIONS: i32 = 1_000_000;

/// What a screen wants done once it's finished drawing.
#[derive(Default)]
struct Outcome {
    send: Vec<Encoded>,
    close: bool,
}

/// A label in front of a text box, on the stone.
fn labelled(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) -> egui::Response {
    let id = ui.label(egui::RichText::new(label).color(ink()).font(chrome::words()));
    let field = egui::TextEdit::singleline(value).desired_width(width).font(chrome::words()).margin(Vec2::new(6.0, 4.0));
    let resp = ui.add(field).labelled_by(id.id);
    crate::dev::note_widget(label, resp.rect);
    resp
}

/// Three numbers on one line, for sizes and positions.
fn triple(ui: &mut egui::Ui, label: &str, names: [&str; 3], values: &mut [String; 3], width: f32) {
    ui.label(egui::RichText::new(label).color(ink()).font(chrome::words()));
    ui.horizontal(|ui| {
        let each = (width - 2.0 * ui.spacing().item_spacing.x) / 3.0;
        for (i, name) in names.iter().enumerate() {
            let resp = ui.add(egui::TextEdit::singleline(&mut values[i]).desired_width(each - 12.0).font(chrome::words()).margin(Vec2::new(6.0, 4.0)));
            crate::dev::note_widget(&format!("{label} {name}"), resp.rect);
        }
    });
}

/// The game's words for something, without the colon they end in on its own screens.
fn tr_or(tr: &dyn Translator, key: &str, fallback: &str) -> String {
    tr.translate(key).unwrap_or(fallback).trim_end_matches(':').to_string()
}

/// A row of keys, the chosen one held down, under its own heading.
fn choice(ui: &mut egui::Ui, heading: &str, options: &[&str], selected: &mut usize) {
    ui.label(egui::RichText::new(heading).color(ink()).font(chrome::words()));
    ui.horizontal(|ui| {
        widgets::segmented(ui, heading, options, selected);
    });
}

fn done_row(ui: &mut egui::Ui, out: &mut Outcome, on_done: impl FnOnce(&mut Outcome)) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if widgets::primary(ui, "Done").clicked() || enter {
            on_done(out);
            out.close = true;
        }
        if widgets::secondary(ui, "Cancel").clicked() {
            out.close = true;
        }
    });
}

fn command(ui: &mut egui::Ui, s: &mut CommandScreen, tr: &dyn Translator, width: f32, out: &mut Outcome) {
    widgets::heading(ui, if s.minecart.is_some() { "Minecart with Command Block" } else { "Command Block" });
    ui.add_space(4.0);
    // Tab takes the suggestion, and the arrow keys choose between them.
    let (tab, up, down) = ui.input_mut(|i| {
        if s.suggestions.is_empty() {
            (false, false, false)
        } else {
            (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
        }
    });
    let n = s.suggestions.len().max(1);
    if up {
        s.suggest_pick = (s.suggest_pick + n - 1) % n;
    }
    if down {
        s.suggest_pick = (s.suggest_pick + 1) % n;
    }
    let cursor = if tab { s.accept_suggestion() } else { None };
    let resp = labelled(ui, &tr_or(tr, "advMode.command", "Console Command"), &mut s.command, width);
    if !resp.has_focus() && (s.command.is_empty() || tab) {
        resp.request_focus();
    }
    if let Some(at) = cursor {
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
            state.store(ui.ctx(), resp.id);
        }
    }
    // What the server offers for the word being typed.
    if resp.has_focus() && !s.suggestions.is_empty() {
        let row_h = chrome::words_size() * 1.2;
        let shown = s.suggestions.len().min(6);
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, row_h * shown as f32 + 2.0 * chrome::unit()), egui::Sense::hover());
        chrome::well(ui.painter(), area, false);
        let first = s.suggest_pick.saturating_sub(shown - 1);
        for (row, line) in s.suggestions.iter().enumerate().skip(first).take(shown) {
            let at = area.left_top() + Vec2::new(4.0 * chrome::unit(), chrome::unit() + (row - first) as f32 * row_h);
            let color = if row == s.suggest_pick { egui::Color32::from_rgb(0xFF, 0xD8, 0x7A) } else { chrome::TIP_MUTED };
            ui.painter().text(at, egui::Align2::LEFT_TOP, line, chrome::words(), color);
        }
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr_or(tr, "advMode.previousOutput", "Previous Output")).color(ink()).font(chrome::words()));
    let shown = if s.track_output { if s.last_output.is_empty() { "No output yet".to_string() } else { s.last_output.clone() } } else { "Not kept".to_string() };
    ui.label(egui::RichText::new(shown).color(ink_muted()).font(chrome::words()));
    widgets::switch_line(ui, &mut s.track_output, &tr_or(tr, "advMode.trackOutput", "Track output"));
    if s.pos.is_some() {
        ui.add_space(6.0);
        let (chain, repeat, impulse) = (tr_or(tr, "advMode.mode.sequence", "Chain"), tr_or(tr, "advMode.mode.auto", "Repeat"), tr_or(tr, "advMode.mode.redstone", "Impulse"));
        choice(ui, "Mode", &[chain.as_str(), repeat.as_str(), impulse.as_str()], &mut s.mode);
        let (always, conditional) = (tr_or(tr, "advMode.mode.unconditional", "Unconditional"), tr_or(tr, "advMode.mode.conditional", "Conditional"));
        let mut cond = usize::from(s.conditional);
        choice(ui, "Runs", &[always.as_str(), conditional.as_str()], &mut cond);
        s.conditional = cond == 1;
        let (needs, active) = (tr_or(tr, "advMode.mode.redstoneTriggered", "Needs Redstone"), tr_or(tr, "advMode.mode.autoexec.bat", "Always Active"));
        let mut auto = usize::from(!s.needs_redstone);
        choice(ui, "Triggered by", &[needs.as_str(), active.as_str()], &mut auto);
        s.needs_redstone = auto == 0;
    }
    let packet = s.packet();
    done_row(ui, out, |out| out.send.extend(packet));
}

fn structure(ui: &mut egui::Ui, s: &mut StructureScreen, tr: &dyn Translator, width: f32, out: &mut Outcome) {
    widgets::heading(ui, "Structure Block");
    ui.add_space(4.0);
    let modes: Vec<String> = ["save", "load", "corner", "data"].iter().map(|m| tr_or(tr, &format!("structure_block.mode.{m}"), m)).collect();
    let refs: Vec<&str> = modes.iter().map(String::as_str).collect();
    ui.horizontal(|ui| {
        widgets::segmented(ui, "Mode", &refs, &mut s.mode);
    });
    let xyz = ["x", "y", "z"];
    let half = (width - ui.spacing().item_spacing.x) / 2.0;
    match s.mode {
        // Save and load share most of their fields.
        0 | 1 => {
            labelled(ui, &tr_or(tr, "structure_block.structure_name", "Structure Name"), &mut s.name, width);
            ui.columns(2, |c| {
                triple(&mut c[0], &tr_or(tr, "structure_block.position", "Relative Position"), xyz, &mut s.offset, half);
                if s.mode == 0 {
                    triple(&mut c[1], &tr_or(tr, "structure_block.size", "Structure Size"), xyz, &mut s.size, half);
                } else {
                    labelled(&mut c[1], &tr_or(tr, "structure_block.integrity", "Structure Integrity and Seed"), &mut s.integrity, half - 8.0);
                    c[1].add(egui::TextEdit::singleline(&mut s.seed).desired_width(half - 20.0).font(chrome::words()).margin(Vec2::new(6.0, 4.0)));
                }
            });
            if s.mode == 1 {
                let (mirrors, rotations) = (["None", "Left to right", "Front to back"], ["0", "90", "180", "270"]);
                ui.columns(2, |c| {
                    choice(&mut c[0], "Mirror", &mirrors, &mut s.mirror);
                    choice(&mut c[1], "Rotation", &rotations, &mut s.rotation);
                });
            }
            ui.columns(2, |c| {
                widgets::switch_line(&mut c[0], &mut s.include_entities, &tr_or(tr, "structure_block.include_entities", "Include Entities"));
                widgets::switch_line(&mut c[1], &mut s.show_bounding_box, &tr_or(tr, "structure_block.show_boundingbox", "Show Bounding Box"));
                if s.mode == 0 {
                    widgets::switch_line(&mut c[0], &mut s.show_air, &tr_or(tr, "structure_block.show_air", "Show Invisible Blocks"));
                } else {
                    widgets::switch_line(&mut c[0], &mut s.strict, &tr_or(tr, "structure_block.strict", "Strict Placement"));
                }
            });
            ui.horizontal(|ui| {
                if s.mode == 0 {
                    if widgets::secondary(ui, &tr_or(tr, "structure_block.button.detect_size", "Detect")).clicked() {
                        out.send.push(s.packet(3));
                    }
                    if widgets::secondary(ui, &tr_or(tr, "structure_block.button.save", "Save")).clicked() {
                        out.send.push(s.packet(1));
                        out.close = true;
                    }
                } else if widgets::secondary(ui, &tr_or(tr, "structure_block.button.load", "Load")).clicked() {
                    out.send.push(s.packet(2));
                    out.close = true;
                }
            });
        }
        2 => {
            labelled(ui, &tr_or(tr, "structure_block.structure_name", "Structure Name"), &mut s.name, width);
            widgets::muted(ui, "Marks a corner for a block in Save mode to find its size by.");
        }
        _ => {
            labelled(ui, &tr_or(tr, "structure_block.custom_data", "Custom Data Tag Name"), &mut s.data, width);
        }
    }
    let packet = s.packet(0);
    done_row(ui, out, |out| out.send.push(packet));
}

fn jigsaw(ui: &mut egui::Ui, s: &mut JigsawScreen, tr: &dyn Translator, width: f32, out: &mut Outcome) {
    widgets::heading(ui, "Jigsaw Block");
    ui.add_space(4.0);
    let half = (width - ui.spacing().item_spacing.x) / 2.0 - 4.0;
    labelled(ui, &tr_or(tr, "jigsaw_block.pool", "Target Pool"), &mut s.pool, width);
    ui.columns(2, |c| {
        labelled(&mut c[0], &tr_or(tr, "jigsaw_block.name", "Name"), &mut s.name, half);
        labelled(&mut c[1], &tr_or(tr, "jigsaw_block.target", "Target Name"), &mut s.target, half);
        labelled(&mut c[0], &tr_or(tr, "jigsaw_block.final_state", "Turns into"), &mut s.final_state, half);
        if s.vertical {
            let (rollable, aligned) = (tr_or(tr, "jigsaw_block.joint.rollable", "Rollable"), tr_or(tr, "jigsaw_block.joint.aligned", "Aligned"));
            let mut joint = usize::from(!s.rollable);
            choice(&mut c[1], &tr_or(tr, "jigsaw_block.joint_label", "Joint Type"), &[rollable.as_str(), aligned.as_str()], &mut joint);
            s.rollable = joint == 0;
        }
        labelled(&mut c[0], &tr_or(tr, "jigsaw_block.selection_priority", "Selection Priority"), &mut s.selection_priority, half);
        labelled(&mut c[1], &tr_or(tr, "jigsaw_block.placement_priority", "Placement Priority"), &mut s.placement_priority, half);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Levels: {}", s.levels)).color(ink()).font(chrome::words()));
        ui.add(egui::Slider::new(&mut s.levels, 0..=20).show_value(false));
        widgets::switch_line(ui, &mut s.keep_jigsaws, &tr_or(tr, "jigsaw_block.keep_jigsaws", "Keep Jigsaws"));
        if widgets::secondary(ui, &tr_or(tr, "jigsaw_block.generate", "Generate")).clicked() {
            out.send.push(s.packet());
            out.send.push(play::jigsaw_generate(s.pos, s.levels, s.keep_jigsaws));
            out.close = true;
        }
    });
    let packet = s.packet();
    done_row(ui, out, |out| out.send.push(packet));
}

fn test(ui: &mut egui::Ui, s: &mut TestScreen, tr: &dyn Translator, width: f32, out: &mut Outcome) {
    widgets::heading(ui, "Test Block");
    ui.add_space(4.0);
    let modes: Vec<String> = ["start", "log", "fail", "accept"].iter().map(|m| tr_or(tr, &format!("test_block.mode.{m}"), m)).collect();
    let refs: Vec<&str> = modes.iter().map(String::as_str).collect();
    ui.horizontal(|ui| {
        widgets::segmented(ui, "Mode", &refs, &mut s.mode);
    });
    labelled(ui, &tr_or(tr, "test_block.message", "Message"), &mut s.message, width);
    let (pos, mode, message) = (s.pos, s.mode as i32, s.message.clone());
    done_row(ui, out, |out| out.send.push(play::set_test_block(pos, mode, &message)));
}

fn test_instance(ui: &mut egui::Ui, s: &mut TestInstanceScreen, tr: &dyn Translator, width: f32, out: &mut Outcome) {
    widgets::heading(ui, "Test Instance Block");
    ui.add_space(4.0);
    labelled(ui, &tr_or(tr, "test_instance_block.test_id", "Test Instance ID"), &mut s.id, width);
    triple(ui, &tr_or(tr, "test_instance_block.size", "Test Structure Size"), ["x", "y", "z"], &mut s.size, width);
    choice(ui, &tr_or(tr, "test_instance_block.rotation", "Test Structure Rotation"), &["0", "90", "180", "270"], &mut s.rotation);
    widgets::switch_line(ui, &mut s.include_entities, &tr_or(tr, "test_instance_block.entities", "Include Entities"));
    for line in &s.status {
        ui.label(egui::RichText::new(line).color(ink_muted()).font(chrome::words()));
    }
    ui.horizontal(|ui| {
        for (action, key, words) in [(3, "test_instance.action.reset", "Reset"), (4, "test_instance.action.save", "Save"), (5, "", "Export Structure"), (6, "test_instance.action.run", "Run")] {
            let label = if key.is_empty() { words.to_string() } else { tr_or(tr, key, words) };
            if widgets::secondary(ui, &label).clicked() {
                out.send.push(s.packet(2));
                out.send.push(s.packet(action));
                if action == 6 {
                    out.close = true;
                }
            }
        }
    });
    let packet = s.packet(2);
    done_row(ui, out, |out| out.send.push(packet));
}

/// The screen for whichever block is open.
pub fn draw(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(assets) = app.ready_assets() else { return };
    let tr: &dyn Translator = &assets.lang;
    let Some(session) = app.session.as_mut() else { return };
    let Some(Modal::Block(block)) = session.modal.as_mut() else { return };
    let shade = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("block_shade")));
    chrome::dim(&shade, screen, egui::Color32::from_black_alpha(165));
    let u = chrome::unit();
    let width = (screen.width() - 80.0 - 14.0 * u).clamp(320.0, 760.0);
    let mut out = Outcome::default();
    // Ask the server what could finish what's typed, and take in its answer.
    if let BlockScreen::Command(s) = block.as_mut() {
        if s.command != s.asked {
            s.asked = s.command.clone();
            s.question += 1;
            s.suggestions.clear();
            s.suggest_pick = 0;
            if !s.command.is_empty() {
                out.send.push(play::command_suggestion(COMMAND_QUESTIONS + s.question, &s.command));
            }
        }
        if let Some((id, start, length, entries)) = session.suggestions.take() {
            if id == COMMAND_QUESTIONS + s.question {
                s.suggestions = entries.into_iter().map(|(text, _)| text).collect();
                s.suggest_start = start.max(0) as usize;
                s.suggest_length = length.max(0) as usize;
            }
        }
    }
    let block_box = block.as_mut();
    egui::Area::new(egui::Id::unique("block_screen")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_CENTER).fixed_pos(screen.center()).show(ui.ctx(), |ui| {
        chrome::window(ui, (7.0 * u).round() as i8, |ui| {
            ui.set_width(width);
            match block_box {
                BlockScreen::Command(s) => command(ui, s, tr, width, &mut out),
                BlockScreen::Structure(s) => structure(ui, s, tr, width, &mut out),
                BlockScreen::Jigsaw(s) => jigsaw(ui, s, tr, width, &mut out),
                BlockScreen::Test(s) => test(ui, s, tr, width, &mut out),
                BlockScreen::TestInstance(s) => test_instance(ui, s, tr, width, &mut out),
            }
        });
    });
    for packet in out.send {
        session.conn.send(packet);
    }
    if out.close {
        session.modal = None;
        app.capture_mouse();
    }
}

/// A small list for switching game mode, opened with F3 and F4.
pub fn draw_game_modes(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(assets) = app.ready_assets() else { return };
    let tex = app.item_texture(ui.ctx());
    let Some(session) = app.session.as_mut() else { return };
    let Some(Modal::GameMode { selected }) = session.modal.as_mut() else { return };
    let shade = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("mode_shade")));
    chrome::dim(&shade, screen, egui::Color32::from_black_alpha(120));
    let u = chrome::unit();
    const MODES: [(&str, &str); 4] = [("Survival", "iron_sword"), ("Creative", "grass_block"), ("Adventure", "map"), ("Spectator", "ender_eye")];
    let tile = 30.0 * u;
    let total = Vec2::new(tile * 4.0 + 3.0 * u + 14.0 * u, tile + 14.0 * u + 12.0 * u);
    let panel = Rect::from_center_size(chrome::snap(screen.center(), ui.ctx().pixels_per_point()), total);
    let mut pick = None;
    let selected_now = *selected;
    egui::Area::new(egui::Id::unique("mode_screen")).order(egui::Order::Foreground).fixed_pos(egui::Pos2::ZERO).show(ui.ctx(), |ui| {
        on_panel(ui, |ui| {
            let p = ui.painter().clone();
            chrome::panel(&p, panel);
            for (i, (name, icon)) in MODES.iter().enumerate() {
                let r = Rect::from_min_size(panel.min + Vec2::new(7.0 * u + i as f32 * (tile + u), 7.0 * u), Vec2::splat(tile));
                let resp = ui.interact(r, egui::Id::unique(("game_mode", i)), egui::Sense::click());
                crate::dev::note_widget(&format!("mode:{name}"), r);
                chrome::button(&p, r, chrome::Face::Plain, resp.hovered(), i == selected_now);
                if i == selected_now {
                    p.rect_filled(r.shrink(2.0 * u), egui::CornerRadius::ZERO, accent_dim());
                }
                if let Some(id) = crate::game::items::item_id(icon) {
                    super::items::draw_stack(&p, Rect::from_center_size(r.center(), Vec2::splat(16.0 * u)), tex, &assets, &lattice_proto::ItemStack::new(id, 1), 0.0);
                }
                if resp.clicked() {
                    pick = Some(i);
                }
            }
            chrome::text(&p, egui::Pos2::new(panel.center().x, panel.bottom() - 7.0 * u), egui::Align2::CENTER_BOTTOM, MODES[selected_now].0, chrome::words(), ink());
        });
    });
    if let Some(i) = pick {
        session.conn.send(play::change_game_mode(i as i32));
        session.modal = None;
        app.capture_mouse();
    }
}

/// Who a spectator can go to: everyone playing who isn't also watching.
pub fn draw_teleport(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(session) = app.session.as_mut() else { return };
    let me = session.self_uuid();
    let mut people: Vec<(String, uuid::Uuid)> = session.tab.iter().filter(|(id, e)| **id != me && e.game_mode != 3).map(|(id, e)| (e.name.clone(), *id)).collect();
    people.sort();
    let shade = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("teleport_shade")));
    chrome::dim(&shade, screen, egui::Color32::from_black_alpha(140));
    let u = chrome::unit();
    let width = 150.0 * u;
    let mut go = None;
    let mut close = false;
    egui::Area::new(egui::Id::unique("teleport_screen")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_CENTER).fixed_pos(screen.center()).show(ui.ctx(), |ui| {
        chrome::window(ui, (7.0 * u).round() as i8, |ui| {
            ui.set_width(width);
            widgets::heading(ui, "Teleport to a player");
            if people.is_empty() {
                widgets::muted(ui, "Nobody else is playing here.");
            }
            egui::ScrollArea::vertical().max_height((screen.height() - 200.0).max(120.0)).show(ui, |ui| {
                for (name, id) in &people {
                    if widgets::menu_button(ui, name, false, width - 12.0).clicked() {
                        go = Some(*id);
                    }
                }
            });
            ui.add_space(4.0);
            if widgets::secondary(ui, "Cancel").clicked() {
                close = true;
            }
        });
    });
    if let Some(id) = go {
        session.conn.send(play::teleport_to_entity(id));
        close = true;
    }
    if close {
        session.modal = None;
        app.capture_mouse();
    }
}

/// Game rules for whoever's allowed to change them.
pub fn draw_game_rules(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(assets) = app.ready_assets() else { return };
    let tr: &dyn Translator = &assets.lang;
    let Some(session) = app.session.as_mut() else { return };
    let Some(Modal::GameRules { rules, search, difficulty }) = session.modal.as_mut() else { return };
    let shade = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("rules_shade")));
    chrome::dim(&shade, screen, egui::Color32::from_black_alpha(165));
    let u = chrome::unit();
    let width = (screen.width() - 80.0 - 14.0 * u).clamp(320.0, 760.0);
    let mut send = Vec::new();
    let mut close = false;
    let rows_h = (screen.height() - 300.0).max(160.0);
    egui::Area::new(egui::Id::unique("rules_screen")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_CENTER).fixed_pos(screen.center()).show(ui.ctx(), |ui| {
        chrome::window(ui, (7.0 * u).round() as i8, |ui| {
            ui.set_width(width);
            widgets::heading(ui, tr_or(tr, "editGamerule.title", "Game Rules"));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Difficulty").color(ink()).font(chrome::words()));
                let mut chosen = (*difficulty).clamp(0, 3) as usize;
                if widgets::segmented(ui, "Difficulty", &["Peaceful", "Easy", "Normal", "Hard"], &mut chosen) {
                    *difficulty = chosen as i32;
                    send.push(play::change_difficulty(chosen as i32));
                }
            });
            let find = ui.add(egui::TextEdit::singleline(search).hint_text(egui::RichText::new("Search").color(ink_muted())).desired_width(width).font(chrome::words()).margin(Vec2::new(6.0, 4.0)));
            crate::dev::note_widget("rule search", find.rect);
            let needle = search.to_lowercase();
            egui::ScrollArea::vertical().max_height(rows_h).auto_shrink([false, true]).show(ui, |ui| {
                if rules.is_empty() {
                    widgets::muted(ui, "Waiting for the server to send them.");
                }
                for rule in rules.iter_mut() {
                    let key = rule.0.trim_start_matches("minecraft:");
                    let camel = camel_case(key);
                    let shown = tr.translate(&format!("gamerule.{key}")).or_else(|| tr.translate(&format!("gamerule.{camel}"))).map_or_else(|| plain_name(key), str::to_string);
                    if !needle.is_empty() && !shown.to_lowercase().contains(&needle) && !key.contains(&needle) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        let is_bool = rule.1 == "true" || rule.1 == "false";
                        if is_bool {
                            let mut on = rule.1 == "true";
                            if widgets::switch(ui, &mut on, &shown).changed() {
                                rule.1 = on.to_string();
                            }
                        } else {
                            let resp = ui.add(egui::TextEdit::singleline(&mut rule.1).desired_width(90.0).font(chrome::words()).margin(Vec2::new(6.0, 2.0)));
                            crate::dev::note_widget(&format!("rule:{key}"), resp.rect);
                        }
                        ui.label(egui::RichText::new(shown).color(ink()).font(chrome::words()));
                    });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::primary(ui, "Done").clicked() {
                    let changed: Vec<(String, String)> = rules.iter().filter(|r| r.1 != r.2).map(|r| (r.0.clone(), r.1.clone())).collect();
                    if !changed.is_empty() {
                        send.push(play::set_game_rules(&changed));
                    }
                    close = true;
                }
                if widgets::secondary(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
    });
    for packet in send {
        session.conn.send(packet);
    }
    if close {
        session.modal = None;
    }
}

/// `advance_time` as "Advance time", for rules with no name of their own in the game's words.
fn plain_name(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn camel_case(key: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in key.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The story at the end of the End, read from the game's own files, then back to the world.
pub fn draw_credits(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(session) = app.session.as_mut() else { return };
    let Some(Modal::Credits { lines, started }) = session.modal.as_mut() else { return };
    let shade = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("credits_shade")));
    chrome::dim(&shade, screen, egui::Color32::from_black_alpha(235));
    let elapsed = started.elapsed().as_secs_f32();
    let speed = if ui.input(|i| i.modifiers.shift || i.key_down(egui::Key::Space)) { 120.0 } else { 26.0 };
    let line_h = chrome::words_size() * 1.3;
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("credits_text")));
    let top = screen.bottom() - elapsed * speed;
    for (i, line) in lines.iter().enumerate() {
        let y = top + i as f32 * line_h;
        if y > screen.top() - line_h && y < screen.bottom() {
            let was = panel_mode(false);
            chrome::text(&p, egui::Pos2::new(screen.center().x, y), egui::Align2::CENTER_TOP, line, chrome::words(), chrome::TIP_INK);
            panel_mode(was);
        }
    }
    let finished = top + lines.len() as f32 * line_h < screen.top();
    let mut leave = finished;
    egui::Area::new(egui::Id::unique("credits_skip")).order(egui::Order::Foreground).anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0)).show(ui.ctx(), |ui| {
        on_panel(ui, |ui| {
            if widgets::secondary(ui, "Skip").clicked() {
                leave = true;
            }
        });
    });
    if !finished {
        ui.ctx().request_repaint();
    }
    if leave {
        session.modal = None;
        session.respawn();
        app.capture_mouse();
    }
}
