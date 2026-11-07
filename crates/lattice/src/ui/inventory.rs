//! The inventory and every menu a server can open, in Lattice's own look. Layouts come
//! from what each menu's slots are for; the behaviour comes from the server. Everything
//! is placed on the grid from `chrome::unit()`: a slot is 18 units, its picture 16.

use super::chrome;
use super::items::{draw_stack, draw_stack_with, tooltip, tooltip_lines};
use super::theme::*;
use super::widgets;
use crate::app::App;
use crate::assets::GameAssets;
use crate::game::creative;
use crate::game::inventory::{Inventory, menu_own_slots};
use crate::input::Action;
use egui::{Align2, Color32, CornerRadius, Painter, PointerButton, Pos2, Rect, RichText, Sense, Vec2, vec2};
use lattice_proto::packets::play::{self, click};
use lattice_proto::{ItemStack, Translator};
use std::sync::Arc;
use std::time::Instant;

/// Slots side by side and the picture in each, in grid units.
const SLOT: f32 = 18.0;
const ICON: f32 = 16.0;
/// Space between a panel's edge and what's in it.
const PAD: f32 = 7.0;
/// The width of nine slots.
const WIDE: f32 = 9.0 * SLOT;
/// The trade list to the left of a trader's panel.
const TRADES: f32 = 84.0;
/// How long two clicks can be apart and count as one double click, in seconds.
const DOUBLE_CLICK: f32 = 0.25;

/// Which of the game's four recipe books goes with a menu.
fn book_index(kind: &str) -> i32 {
    match kind {
        "furnace" => 1,
        "blast_furnace" => 2,
        "smoker" => 3,
        _ => 0,
    }
}

/// What the player is doing with the menu right now.
#[derive(Default, Clone)]
pub struct MenuState {
    /// Slots dragged over with an item held: (button, slots).
    pub drag: Option<(u8, Vec<i16>)>,
    /// The press that picked something up: its release shouldn't put it down again.
    pub skip_release: bool,
    /// The last press: where, with what, and when, for telling double clicks.
    pub last_press: Option<(i16, u8, Instant)>,
    pub double_click: bool,
    /// What the last shift-click moved, for moving all the rest with a double click.
    pub last_moved: Option<ItemStack>,
    /// Keys pressed for the slot under the pointer.
    pub keys: Vec<Action>,
    pub rename: String,
    pub creative_tab: usize,
    pub creative_scroll: f32,
    pub creative_search: String,
    pub trade_scroll: f32,
    pub book: super::recipes::BookState,
    /// The powers picked in a beacon, not yet confirmed; -1 for none.
    pub beacon: Option<(i32, i32)>,
}

struct Placed {
    slot: i16,
    rect: Rect,
    hint: &'static str,
    output: bool,
}

/// Where everything goes in the open menu.
struct Plan {
    u: f32,
    slots: Vec<Placed>,
    panel: Rect,
    /// Left of the nine-slot column, and the top of the menu's own part.
    origin: Pos2,
    /// Where "Inventory" is written, if it is.
    label_y: f32,
    /// The height of the words in grid units.
    text_h: f32,
}

impl Plan {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.u
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.at(x, y), vec2(w, h) * self.u)
    }

    fn grid(&mut self, x: f32, y: f32, cols: usize, rows: usize, first: i16) {
        for r in 0..rows {
            for c in 0..cols {
                self.one(x + c as f32 * SLOT, y + r as f32 * SLOT, first + (r * cols + c) as i16, "");
            }
        }
    }

    fn one(&mut self, x: f32, y: f32, slot: i16, hint: &'static str) {
        let rect = self.rect(x, y, SLOT, SLOT);
        self.slots.push(Placed { slot, rect, hint, output: false });
    }

    /// The larger place for what comes out.
    fn out(&mut self, x: f32, y: f32, slot: i16) {
        let rect = self.rect(x, y, 26.0, 26.0);
        self.slots.push(Placed { slot, rect, hint: "", output: true });
    }

    /// The player's 27 + 9 slots, wherever they are in this menu.
    fn player(&mut self, y: f32, first: i16) {
        self.grid(0.0, y, 9, 3, first);
        self.grid(0.0, y + 3.0 * SLOT + 4.0, 9, 1, first + 27);
    }
}

fn title_of(inv: &Inventory, tr: &dyn Translator) -> String {
    match &inv.menu {
        Some(m) => {
            let t = m.title.to_plain(tr);
            if t.is_empty() { "Inventory".into() } else { t }
        }
        None => "Inventory".into(),
    }
}

/// The height of a menu's own part, above the player's slots, in units.
fn own_height(kind: &str, columns: i32) -> f32 {
    match kind {
        "" => 72.0,
        "generic_9x1" | "hopper" => SLOT,
        "generic_9x2" => 2.0 * SLOT,
        "generic_9x3" | "shulker_box" | "generic_3x3" | "crafter_3x3" | "crafting" | "furnace" | "smoker" | "blast_furnace" | "grindstone" | "cartography_table" | "smithing" => 3.0 * SLOT,
        "generic_9x4" => 4.0 * SLOT,
        "generic_9x5" => 5.0 * SLOT,
        "generic_9x6" => 6.0 * SLOT,
        "anvil" => 46.0,
        "brewing_stand" => 62.0,
        "enchantment" => 58.0,
        "stonecutter" | "loom" => 58.0,
        "merchant" => 40.0,
        "beacon" => 88.0,
        "lectern" => 128.0,
        "mount" => if columns > 0 { 3.0 * SLOT } else { 2.0 * SLOT },
        _ => 3.0 * SLOT,
    }
}

/// Lays out the slots for what's open.
fn plan(inv: &Inventory, screen: Rect, ppp: f32) -> Plan {
    let u = chrome::unit();
    let kind = inv.menu.as_ref().map_or("", |m| m.kind.as_str());
    let columns = inv.menu.as_ref().map_or(0, |m| m.columns);
    let text_h = (chrome::words_size() * 1.1 * if pixel_font() { 1.0 } else { 0.8 } / u).ceil();
    let own = own_height(kind, columns);
    // The player's own screen has no title: it's a picture of them.
    let title_h = if kind.is_empty() { 0.0 } else { text_h + 3.0 };
    let label_h = if kind.is_empty() { 4.0 } else { text_h + 3.0 };
    let player_h = 3.0 * SLOT + 4.0 + SLOT;
    let body = if kind == "lectern" { own } else { own + label_h + player_h };
    let side = if kind == "merchant" { TRADES + 5.0 } else { 0.0 };
    let (w, h) = (WIDE + 2.0 * PAD + side, PAD + title_h + body + PAD);
    // On whole pixels, or every hard edge in it goes soft.
    let at = chrome::snap(screen.center() - vec2(w, h) * u / 2.0, ppp);
    let panel = Rect::from_min_size(at, vec2(w, h) * u);
    let origin = at + vec2(PAD + side, PAD + title_h) * u;
    let mut l = Plan { u, slots: Vec::new(), panel, origin, label_y: origin.y + (own + 2.0) * u, text_h };
    let n = menu_own_slots(kind, columns) as i16;
    match kind {
        // The player's own screen: armour, you, offhand, crafting.
        "" => {
            for (i, (slot, hint)) in [(5, "Head"), (6, "Chest"), (7, "Legs"), (8, "Feet")].into_iter().enumerate() {
                l.one(0.0, i as f32 * SLOT, slot, hint);
            }
            l.one(4.0 * SLOT, 3.0 * SLOT, 45, "Offhand");
            l.grid(5.0 * SLOT + 2.0, SLOT, 2, 2, 1);
            l.out(136.0, SLOT + 5.0, 0);
            l.player(own + 4.0, 9);
            return l;
        }
        "generic_9x1" | "generic_9x2" | "generic_9x3" | "generic_9x4" | "generic_9x5" | "generic_9x6" | "shulker_box" => l.grid(0.0, 0.0, 9, n as usize / 9, 0),
        "generic_3x3" => l.grid(3.0 * SLOT, 0.0, 3, 3, 0),
        // Its result comes after the player's slots, in the game's own order.
        "crafter_3x3" => {
            l.grid(26.0, 0.0, 3, 3, 0);
            l.out(118.0, 14.0, 45);
        }
        "hopper" => l.grid(2.0 * SLOT, 0.0, 5, 1, 0),
        "crafting" => {
            l.grid(18.0, 0.0, 3, 3, 1);
            l.out(118.0, 14.0, 0);
        }
        "furnace" | "smoker" | "blast_furnace" => {
            l.one(44.0, 0.0, 0, "Input");
            l.one(44.0, 36.0, 1, "Fuel");
            l.out(106.0, 14.0, 2);
        }
        "brewing_stand" => {
            l.one(92.0, 0.0, 3, "Ingredient");
            l.one(70.0, 42.0, 0, "Bottle");
            l.one(92.0, 44.0, 1, "Bottle");
            l.one(114.0, 42.0, 2, "Bottle");
            l.one(20.0, 0.0, 4, "Blaze powder");
        }
        "anvil" => {
            l.one(14.0, 20.0, 0, "Item");
            l.one(50.0, 20.0, 1, "Material");
            l.out(118.0, 16.0, 2);
        }
        "smithing" => {
            l.one(8.0, 18.0, 0, "Template");
            l.one(26.0, 18.0, 1, "Base");
            l.one(44.0, 18.0, 2, "Material");
            l.out(106.0, 14.0, 3);
        }
        "grindstone" | "cartography_table" => {
            l.one(30.0, 0.0, 0, if kind == "grindstone" { "Item" } else { "Map" });
            l.one(30.0, 36.0, 1, if kind == "grindstone" { "Item" } else { "Paper or glass" });
            l.out(106.0, 14.0, 2);
        }
        "enchantment" => {
            l.one(4.0, 20.0, 0, "Item");
            l.one(26.0, 20.0, 1, "Lapis");
        }
        "stonecutter" => {
            l.one(2.0, 20.0, 0, "Input");
            l.out(136.0, 16.0, 1);
        }
        "loom" => {
            l.one(0.0, 0.0, 0, "Banner");
            l.one(18.0, 0.0, 1, "Dye");
            l.one(9.0, 20.0, 2, "Pattern");
            l.out(136.0, 16.0, 3);
        }
        "beacon" => l.one(90.0, 68.0, 0, "Payment"),
        "merchant" => {
            l.one(16.0, 14.0, 0, "");
            l.one(36.0, 14.0, 1, "");
            l.out(112.0, 10.0, 2);
        }
        "lectern" => return l,
        "mount" => {
            l.one(0.0, 0.0, 0, "Saddle");
            l.one(0.0, SLOT, 1, "Armour");
            if columns > 0 {
                l.grid(2.0 * SLOT, 0.0, columns as usize, 3, 2);
            }
        }
        // A menu added by a newer server version: show its slots plainly.
        _ => l.grid(0.0, 0.0, 9, n.max(0) as usize / 9, 0),
    }
    l.player(own + label_h, n);
    l
}

fn data_of(inv: &Inventory, key: i16) -> i32 {
    inv.menu.as_ref().and_then(|m| m.data.get(&key)).map_or(0, |v| *v as i32)
}

/// What a trade really costs right now: the game's sum, from demand and discounts.
pub fn trade_cost(base: &ItemStack, offer_special: i32, demand: i32, multiplier: f32) -> i32 {
    let count = base.count;
    let extra = (((count * demand) as f32 * multiplier).floor() as i32).max(0);
    (count + extra + offer_special).clamp(1, crate::game::items::max_stack(base))
}

/// A row of words in the panel's voice.
fn words(p: &Painter, at: Pos2, align: Align2, text: impl ToString, color: Color32) {
    chrome::text(p, at, align, text, chrome::words(), color);
}

/// Draws what isn't a slot.
fn draw_extras(app: &mut App, ui: &mut egui::Ui, p: &Painter, l: &Plan, screen: Rect, assets: &Arc<GameAssets>, tex: egui::TextureId) {
    let Some(session) = app.session.as_mut() else { return };
    let kind = session.inventory.menu.as_ref().map_or(String::new(), |m| m.kind.clone());
    let id = session.inventory.menu.as_ref().map_or(0, |m| m.id);
    let u = l.u;
    let tr: &dyn Translator = &assets.lang;
    let inv = &session.inventory;
    match kind.as_str() {
        "furnace" | "smoker" | "blast_furnace" => {
            let lit = data_of(inv, 0) as f32 / data_of(inv, 1).max(1) as f32;
            let cook = data_of(inv, 2) as f32 / data_of(inv, 3).max(1) as f32;
            chrome::flame(p, l.rect(44.0, 18.0, SLOT, SLOT), if data_of(inv, 0) > 0 { lit } else { 0.0 });
            chrome::arrow(p, l.rect(70.0, 18.0, 28.0, SLOT), cook);
        }
        "brewing_stand" => {
            let brew = data_of(inv, 0);
            let fuel = data_of(inv, 1) as f32 / 20.0;
            words(p, l.at(20.0 + SLOT / 2.0, SLOT + 1.0), Align2::CENTER_TOP, "Fuel", ink_muted());
            chrome::gauge(p, l.rect(14.0, SLOT + 1.0 + l.text_h, 30.0, 6.0), fuel, hex_of(0xD9641E), false);
            // Tubes from the ingredient down to each bottle.
            let tube = if mode() == Mode::Contrast { Color32::WHITE } else { hex_of(0x5A5344) };
            let line = |r: Rect| p.rect_filled(r, CornerRadius::ZERO, tube);
            line(l.rect(100.0, SLOT, 2.0, 12.0));
            line(l.rect(78.0, SLOT + 11.0, 46.0, 2.0));
            line(l.rect(78.0, SLOT + 11.0, 2.0, 13.0));
            line(l.rect(100.0, SLOT + 11.0, 2.0, 15.0));
            line(l.rect(122.0, SLOT + 11.0, 2.0, 13.0));
            // Fills as a brew finishes.
            let track = l.rect(134.0, 0.0, 8.0, 56.0);
            chrome::gauge(p, track, if brew > 0 { 1.0 - brew as f32 / 400.0 } else { 0.0 }, hex_of(0x5FB8E8), true);
        }
        "crafting" => chrome::arrow(p, l.rect(76.0, 18.0, 38.0, SLOT), 0.0),
        "crafter_3x3" => {
            chrome::arrow(p, l.rect(82.0, 18.0, 32.0, SLOT), 0.0);
            // Lit while it's getting a redstone signal.
            let powered = data_of(inv, 9) == 1;
            let dot = l.rect(92.0, 2.0, 8.0, 8.0);
            chrome::well(p, dot, false);
            p.rect_filled(dot.shrink(u), CornerRadius::ZERO, if powered { hex_of(0xE8402A) } else { hex_of(0x2A2620) });
        }
        "grindstone" | "cartography_table" => {
            chrome::picture(p, l.rect(30.0, SLOT, SLOT, SLOT), &chrome::PLUS, u, ink_muted(), |_, _| true);
            chrome::arrow(p, l.rect(58.0, 18.0, 40.0, SLOT), 0.0);
        }
        "smithing" => chrome::arrow(p, l.rect(68.0, 18.0, 32.0, SLOT), 0.0),
        "anvil" => {
            chrome::picture(p, l.rect(32.0, 20.0, SLOT, SLOT), &chrome::PLUS, u, ink_muted(), |_, _| true);
            chrome::arrow(p, l.rect(74.0, 20.0, 36.0, SLOT), 0.0);
            let rect = l.rect(0.0, 0.0, WIDE, 15.0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
            let mut name = app.menu.rename.clone();
            let r = child.add(egui::TextEdit::singleline(&mut name).hint_text(RichText::new("Type a new name").color(ink_muted())).desired_width(rect.width()).font(chrome::words()).char_limit(50));
            crate::dev::note_widget("rename", r.rect);
            if r.changed() {
                session.conn.send(play::rename_item(&name));
            }
            app.menu.rename = name;
            let cost = data_of(&session.inventory, 0);
            if cost > 0 {
                let creative = session.player.game_mode == 1;
                let (line, color) = if cost >= 40 && !creative {
                    ("Too expensive".to_string(), danger())
                } else if session.vitals.xp_level >= cost || creative {
                    (format!("Costs {cost} {}", if cost == 1 { "level" } else { "levels" }), accent())
                } else {
                    (format!("Costs {cost} levels, you have {}", session.vitals.xp_level), danger())
                };
                words(p, Pos2::new(l.at(WIDE, 0.0).x, l.label_y), Align2::RIGHT_TOP, line, color);
            }
        }
        "enchantment" => {
            let xp = session.vitals.xp_level;
            let lapis = session.inventory.menu.as_ref().and_then(|m| m.slots.get(1).cloned().flatten()).map_or(0, |s| s.count);
            for i in 0..3i32 {
                let cost = data_of(&session.inventory, i as i16);
                let ench = data_of(&session.inventory, 4 + i as i16);
                let level = data_of(&session.inventory, 7 + i as i16);
                let rect = l.rect(50.0, i as f32 * 19.0 + 1.0, WIDE - 50.0, 18.0);
                let can = cost > 0 && (session.player.game_mode == 1 || (xp >= cost && lapis >= i + 1));
                let resp = ui.interact(rect, egui::Id::unique(("enchant", i)), if can { Sense::click() } else { Sense::hover() });
                crate::dev::note_widget(&format!("enchant:{i}"), rect);
                if cost <= 0 {
                    chrome::well(p, rect, false);
                    continue;
                }
                chrome::button(p, rect, if can { chrome::Face::Plain } else { chrome::Face::Off }, can && resp.hovered(), resp.is_pointer_button_down_on());
                let name = enchant_name(session, ench, tr);
                let line = if ench >= 0 { format!("{name} {}", roman(level)) } else { "Unknown".to_string() };
                let color = if can { ink() } else { ink_muted() };
                let y = rect.center().y - chrome::key_lift();
                let galley = p.layout(line, chrome::words(), color, rect.width() - 34.0 * u);
                p.galley(Pos2::new(rect.left() + 4.0 * u, y - galley.size().y / 2.0), galley, color);
                words(p, Pos2::new(rect.right() - 4.0 * u, y), Align2::RIGHT_CENTER, cost, if can { accent() } else { danger() });
                if resp.hovered() && !can {
                    let why = if xp < cost { format!("Needs {cost} levels") } else { format!("Needs {} lapis", i + 1) };
                    if let Some(q) = ui.ctx().pointer_latest_pos() {
                        tooltip(&tip_layer(ui), q, screen, &[(why, chrome::TIP_INK)]);
                    }
                }
                if resp.clicked() {
                    session.conn.send(play::container_button_click(id, i));
                }
            }
        }
        "merchant" => merchant(app, ui, p, l, screen, assets, tex),
        "beacon" => beacon(app, ui, p, l, id, assets),
        "lectern" => {
            let page = data_of(&session.inventory, 0);
            let book = session.inventory.menu.as_ref().and_then(|m| m.slots.first().cloned().flatten());
            let pages = book.as_ref().map_or_else(Vec::new, book_pages);
            let r = l.rect(0.0, 0.0, WIDE, 108.0);
            // A sheet of paper in the stone.
            chrome::well(p, r, false);
            let sheet = r.shrink(u);
            p.rect_filled(sheet, CornerRadius::ZERO, hex_of(if mode() == Mode::Contrast { 0x000000 } else { 0xEFE7D2 }));
            let ink_on_sheet = if mode() == Mode::Contrast { Color32::WHITE } else { hex_of(0x241D12) };
            let text = pages.get(page as usize).cloned().unwrap_or_default();
            p.galley(sheet.left_top() + vec2(5.0, 4.0) * u, p.layout(text, chrome::words(), ink_on_sheet, sheet.width() - 10.0 * u), ink_on_sheet);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(l.rect(0.0, 112.0, WIDE, 16.0)).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if widgets::secondary(&mut child, "Back").clicked() {
                session.conn.send(play::container_button_click(id, 1));
            }
            child.label(RichText::new(format!("Page {} of {}", page + 1, pages.len().max(1))).color(ink_muted()));
            if widgets::secondary(&mut child, "Next").clicked() {
                session.conn.send(play::container_button_click(id, 2));
            }
            if widgets::secondary(&mut child, "Take book").clicked() {
                session.conn.send(play::container_button_click(id, 3));
            }
        }
        "stonecutter" => {
            let input = session.inventory.menu.as_ref().and_then(|m| m.slots.first().cloned().flatten());
            let choices: Vec<ItemStack> = match &input {
                Some(stack) => session.cutter.iter().filter(|c| session.item_in_set(&c.input, stack.item)).filter_map(|c| session.display_stack(&c.output)).collect(),
                None => Vec::new(),
            };
            let selected = data_of(&session.inventory, 0);
            chrome::arrow(p, l.rect(112.0, 20.0, 22.0, SLOT), 0.0);
            let grid = l.rect(24.0, 0.0, 86.0, 58.0);
            chrome::well(p, grid, false);
            if choices.is_empty() {
                let hint = if input.is_some() { "Nothing can be cut from this" } else { "Put a block in to see what it can be cut into" };
                chrome::note_in(p, grid, hint);
            }
            let pointer = ui.ctx().pointer_latest_pos();
            let per_row = 4;
            let cell = 20.0 * u;
            let rows_shown = 2 + 1;
            let visible = rows_shown * per_row;
            let inner = Rect::from_min_size(grid.min + vec2(u, u), vec2(cell * per_row as f32, cell * 2.8));
            let rows = choices.len().div_ceil(per_row);
            if pointer.is_some_and(|q| grid.contains(q)) {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                app.menu.trade_scroll = (app.menu.trade_scroll - wheel / cell).clamp(0.0, rows.saturating_sub(rows_shown) as f32);
            }
            let first = app.menu.trade_scroll.floor() as usize * per_row;
            let mut tip = None;
            let clip = p.with_clip_rect(grid.shrink(u));
            for (i, stack) in choices.iter().enumerate().skip(first).take(visible) {
                let at = i - first;
                let r = Rect::from_min_size(inner.min + vec2((at % per_row) as f32 * cell, (at / per_row) as f32 * cell * 0.92), Vec2::splat(cell));
                let resp = ui.interact(r, egui::Id::unique(("choice", i)), Sense::click());
                crate::dev::note_widget(&format!("choice:{}", crate::game::items::item_name(stack)), r);
                chrome::button(&clip, r, chrome::Face::Plain, resp.hovered() && selected != i as i32, selected == i as i32);
                draw_stack(&clip, icon_in(r, u), tex, assets, stack, 0.0);
                if resp.hovered() {
                    tip = Some(crate::game::items::display_name(stack, tr));
                }
                if resp.clicked() {
                    session.conn.send(play::container_button_click(id, i as i32));
                }
            }
            if rows > rows_shown {
                let track = Rect::from_min_size(grid.right_top() - vec2(6.0 * u, 0.0), vec2(6.0 * u, grid.height()));
                chrome::scroller(p, track, app.menu.trade_scroll / rows.saturating_sub(rows_shown).max(1) as f32, rows_shown as f32 / rows as f32);
            }
            if let (Some(name), Some(q)) = (tip, pointer) {
                tooltip(&tip_layer(ui), q, screen, &[(name, chrome::TIP_INK)]);
            }
        }
        "loom" => loom(app, ui, p, l, id, screen, assets),
        _ => {}
    }
}

fn hex_of(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Roman numerals for enchantment levels, as the game writes them.
fn roman(n: i32) -> String {
    const NAMES: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    usize::try_from(n - 1).ok().and_then(|i| NAMES.get(i)).map_or_else(|| n.to_string(), |s| s.to_string())
}

fn tip_layer(ui: &egui::Ui) -> Painter {
    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("menu_tip")))
}

/// The picture's place in the middle of a slot or button.
fn icon_in(r: Rect, u: f32) -> Rect {
    let at = r.center() - Vec2::splat(ICON * u / 2.0);
    let at = Pos2::new((at.x / u).round() * u, (at.y / u).round() * u);
    Rect::from_min_size(at, Vec2::splat(ICON * u))
}

/// A trader: their stock on the left, the deal being made on the right.
fn merchant(app: &mut App, ui: &mut egui::Ui, p: &Painter, l: &Plan, screen: Rect, assets: &Arc<GameAssets>, tex: egui::TextureId) {
    let u = l.u;
    let Some(session) = app.session.as_mut() else { return };
    let tr: &dyn Translator = &assets.lang;
    let names: Vec<String> = session.registries.as_ref().and_then(|r| r.names.get("enchantment")).cloned().unwrap_or_default();
    let Some(m) = session.inventory.menu.as_mut() else { return };
    let Some(merchant) = m.merchant.as_mut() else {
        return;
    };

    // How far this trader's come, and how far to the next level. Those who don't level up
    // (wandering traders) have no bar.
    let bar = l.rect(0.0, 0.0, WIDE, 7.0);
    let offers = merchant.offers.clone();
    let selected = merchant.selected;
    let pointer = ui.ctx().pointer_latest_pos();
    if merchant.show_progress && (1..=5).contains(&merchant.level) {
        const LEVEL_XP: [i32; 6] = [0, 10, 70, 150, 250, 250];
        let at = merchant.level as usize - 1;
        chrome::well(p, bar, false);
        let inner = bar.shrink(u);
        let span = (LEVEL_XP[at + 1] - LEVEL_XP[at]).max(1) as f32;
        let done = ((merchant.xp - LEVEL_XP[at]) as f32 / span).clamp(0.0, 1.0);
        let done = if merchant.level >= 5 { 1.0 } else { done };
        let mut fill = inner;
        fill.set_width(inner.width() * done);
        p.rect_filled(fill, CornerRadius::ZERO, hex_of(0x6FBF4A));
        // What the chosen trade would add.
        if let Some(offer) = offers.get(selected) {
            if merchant.level < 5 && offer.xp > 0 {
                let more = ((merchant.xp + offer.xp - LEVEL_XP[at]) as f32 / span).clamp(0.0, 1.0);
                let mut extra = inner;
                extra.set_left(inner.left() + inner.width() * done);
                extra.set_right(inner.left() + inner.width() * more);
                p.rect_filled(extra, CornerRadius::ZERO, hex_of(0xB5E58F));
            }
        }
    }

    // The list of what's on offer.
    let list = Rect::from_min_size(l.panel.left_top() + vec2(PAD, PAD) * u, vec2(TRADES * u, l.panel.height() - 2.0 * PAD * u));
    chrome::well(p, list, false);
    let row_h = 20.0 * u;
    let inner = list.shrink(u);
    let shown = (inner.height() / row_h).floor() as usize;
    let extra = offers.len().saturating_sub(shown);
    if pointer.is_some_and(|q| list.contains(q)) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.menu.trade_scroll = (app.menu.trade_scroll - wheel / row_h).clamp(0.0, extra as f32);
    }
    app.menu.trade_scroll = app.menu.trade_scroll.min(extra as f32);
    let first = app.menu.trade_scroll.round() as usize;
    let row_w = inner.width() - if extra > 0 { 7.0 * u } else { 0.0 };
    let mut chosen = None;
    let mut tip: Option<Vec<(String, Color32)>> = None;
    for (i, o) in offers.iter().enumerate().skip(first).take(shown) {
        let r = Rect::from_min_size(inner.min + vec2(0.0, (i - first) as f32 * row_h), vec2(row_w, row_h));
        let resp = ui.interact(r, egui::Id::unique(("trade", i)), Sense::click());
        crate::dev::note_widget(&format!("trade:{i}"), r);
        let on = i == selected;
        chrome::button(p, r, if o.out_of_stock { chrome::Face::Off } else { chrome::Face::Plain }, resp.hovered() && !on, on);
        if on {
            p.rect_filled(r.shrink(2.0 * u), CornerRadius::ZERO, accent_dim());
        }
        let y = r.center().y - ICON * u / 2.0 - if on { 0.0 } else { chrome::key_lift() };
        let y = (y / u).round() * u;
        let icon = |x: f32| Rect::from_min_size(Pos2::new(r.left() + x * u, y), Vec2::splat(ICON * u));
        // What it costs today, in the colour of whether that's more or less than usual.
        let cost_a = trade_cost(&o.cost_a, o.special_price, o.demand, o.price_multiplier);
        let mut paying = o.cost_a.clone();
        paying.count = cost_a;
        let tint = match cost_a.cmp(&o.cost_a.count) {
            std::cmp::Ordering::Less => Some(hex_of(0x8CF06E)),
            std::cmp::Ordering::Greater => Some(hex_of(0xFF9D8A)),
            std::cmp::Ordering::Equal => None,
        };
        draw_stack_with(p, icon(2.0), tex, assets, &paying, 0.0, tint);
        if let Some(b) = &o.cost_b {
            draw_stack(p, icon(20.0), tex, assets, b, 0.0);
        }
        let mid = Pos2::new(r.left() + 45.0 * u, r.center().y - if on { 0.0 } else { chrome::key_lift() });
        let mark = Rect::from_center_size(mid, Vec2::splat(9.0 * u));
        if o.out_of_stock {
            chrome::picture(p, mark, &chrome::CROSS, u, danger(), |_, _| true);
        } else {
            chrome::picture(p, mark, &SMALL_ARROW, u, ink_muted(), |_, _| true);
        }
        if let Some(res) = &o.result {
            draw_stack(p, icon(52.0), tex, assets, res, 0.0);
        }
        if resp.clicked() {
            chosen = Some(i);
        }
        if resp.hovered() {
            // Whichever of the three the pointer's on.
            if let Some(q) = pointer {
                let stack = if q.x < r.left() + 19.0 * u {
                    Some(&paying)
                } else if q.x < r.left() + 38.0 * u {
                    o.cost_b.as_ref()
                } else {
                    o.result.as_ref()
                };
                if let Some(stack) = stack {
                    let mut lines = tooltip_lines(stack, tr, &names);
                    if q.x < r.left() + 19.0 * u && cost_a != o.cost_a.count {
                        lines.push((format!("Usually {}", o.cost_a.count), chrome::TIP_MUTED));
                    }
                    if o.out_of_stock {
                        lines.push(("Sold out until the trader restocks".to_string(), hex_of(0xFF9D8A)));
                    } else if q.x >= r.left() + 38.0 * u {
                        let left = o.max_uses - o.uses;
                        lines.push((format!("{left} {} left", if left == 1 { "trade" } else { "trades" }), chrome::TIP_MUTED));
                    }
                    tip = Some(lines);
                }
            }
        }
    }
    if extra > 0 {
        let track = Rect::from_min_max(Pos2::new(inner.right() - 6.0 * u, inner.top()), inner.right_bottom());
        chrome::scroller(p, track, first as f32 / extra as f32, shown as f32 / offers.len().max(1) as f32);
    }
    if offers.is_empty() {
        chrome::note_in(p, list, "Nothing for sale yet");
    }
    if let Some(i) = chosen {
        if let Some(merchant) = session.inventory.menu.as_mut().and_then(|m| m.merchant.as_mut()) {
            merchant.selected = i;
        }
        session.conn.send(play::select_trade(i as i32));
    }

    // The deal itself.
    let sold_out = offers.get(selected).is_some_and(|o| o.out_of_stock);
    let arrow = l.rect(58.0, 14.0, 50.0, SLOT);
    if sold_out {
        chrome::picture(p, arrow, &chrome::CROSS, u * 2.0, danger(), |_, _| true);
    } else {
        chrome::arrow(p, arrow, 0.0);
    }
    if let (Some(lines), Some(q)) = (tip, pointer) {
        tooltip(&tip_layer(ui), q, screen, &lines);
    }
}

const SMALL_ARROW: [&str; 7] = ["....#....", "....##...", "#########", "##########", "#########", "....##...", "....#...."];

/// A beacon: choose its powers and pay for them.
fn beacon(app: &mut App, ui: &mut egui::Ui, p: &Painter, l: &Plan, id: i32, assets: &Arc<GameAssets>) {
    let u = l.u;
    let ctx = ui.ctx().clone();
    let Some(session) = app.session.as_ref() else { return };
    let levels = data_of(&session.inventory, 0);
    let (now_primary, now_secondary) = (data_of(&session.inventory, 1) - 1, data_of(&session.inventory, 2) - 1);
    let paid = session.inventory.menu.as_ref().and_then(|m| m.slots.first().cloned().flatten()).is_some();
    let (mut primary, mut secondary) = app.menu.beacon.unwrap_or((now_primary, now_secondary));
    let data = lattice_data::game();
    let by_name = |n: &str| data.mob_effects.iter().position(|e| e.name == n).map_or(-1, |i| i as i32);
    let tile = 22.0;
    let label = |p: &Painter, x: f32, y: f32, text: &str| words(p, l.at(x, y), Align2::LEFT_TOP, text, ink());
    label(p, 0.0, 0.0, "Primary power");
    label(p, 90.0, 0.0, "Secondary");
    let top = l.text_h + 1.0;
    let rows: [(i32, &[&str]); 3] = [(1, &["speed", "haste"]), (2, &["resistance", "jump_boost"]), (3, &["strength"])];
    let mut tiles: Vec<(Rect, i32, bool, i32, &str)> = Vec::new();
    for (row, (need, list)) in rows.iter().enumerate() {
        for (i, name) in list.iter().enumerate() {
            tiles.push((l.rect(i as f32 * (tile + 1.0), top + row as f32 * (tile + 1.0), tile, tile), *need, false, by_name(name), name));
        }
    }
    // The second power: regeneration, or the first one made stronger.
    tiles.push((l.rect(90.0, top, tile, tile), 4, true, by_name("regeneration"), "regeneration"));
    let upgrade = data.mob_effects.get(primary.max(0) as usize).map(|e| e.name.clone()).unwrap_or_default();
    if primary >= 0 {
        tiles.push((l.rect(90.0 + tile + 1.0, top, tile, tile), 4, true, primary, &upgrade));
    }
    let mut change: Option<(bool, i32)> = None;
    for (r, need, second, effect, name) in &tiles {
        let on = levels >= *need;
        let resp = ui.interact(*r, egui::Id::unique(("beacon", *second, *name)), if on { Sense::click() } else { Sense::hover() });
        crate::dev::note_widget(&format!("beacon:{}{name}", if *second { "2:" } else { "" }), *r);
        let picked = if *second { secondary == *effect } else { primary == *effect };
        chrome::button(p, *r, if on { chrome::Face::Plain } else { chrome::Face::Off }, on && resp.hovered(), picked);
        if picked {
            p.rect_filled(r.shrink(2.0 * u), CornerRadius::ZERO, accent_dim());
        }
        if let Some(tex) = app.screen_pictures.get_or_make(&ctx, assets, &format!("mob_effect/{name}")) {
            let picture = icon_in(*r, u);
            let shade = if on { Color32::WHITE } else { Color32::from_gray(90) };
            p.image(tex, picture, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), shade);
        }
        if *second && *effect == primary {
            words(p, r.right_bottom() - vec2(2.0, 2.0) * u, Align2::RIGHT_BOTTOM, "II", ink());
        }
        let name_shown = assets.lang.0.get(&format!("effect.minecraft.{name}")).cloned().unwrap_or_else(|| name.to_string());
        let hint = if on { name_shown } else { format!("{name_shown}, needs a pyramid of {need} levels") };
        if resp.hovered() {
            if let Some(q) = ctx.pointer_latest_pos() {
                tooltip(&tip_layer(ui), q, ctx.content_rect(), &[(hint, chrome::TIP_INK)]);
            }
        }
        if resp.clicked() {
            change = Some((*second, *effect));
        }
    }
    match change {
        Some((false, e)) => {
            primary = e;
            // A first power changing makes a matching second one meaningless.
            if secondary >= 0 && secondary != by_name("regeneration") {
                secondary = -1;
            }
        }
        Some((true, e)) => secondary = e,
        None => {}
    }
    app.menu.beacon = Some((primary, secondary));
    words(p, l.at(90.0, top + tile + 2.0), Align2::LEFT_TOP, format!("Pyramid: {} of 4", levels.min(4)), ink_muted());
    let ready = paid && primary >= 0;
    let done = l.rect(112.0, 66.0, 50.0, 22.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(done).layout(egui::Layout::top_down(egui::Align::Center)));
    let hint = if paid { "" } else { "Needs an iron ingot, gold ingot, emerald, diamond or netherite ingot" };
    let resp = widgets::primary_enabled(&mut child, "Done", ready);
    if !hint.is_empty() {
        resp.clone().on_hover_text(hint);
    }
    if resp.clicked() {
        if let Some(session) = app.session.as_mut() {
            session.conn.send(play::set_beacon((primary >= 0).then_some(primary), (secondary >= 0).then_some(secondary)));
            session.conn.send(play::container_close(id));
        }
        app.close_inventory();
    }
}

/// A loom: banner and dye on the left, the patterns they can make beside them.
fn loom(app: &mut App, ui: &mut egui::Ui, p: &Painter, l: &Plan, id: i32, screen: Rect, assets: &Arc<GameAssets>) {
    let u = l.u;
    let tr: &dyn Translator = &assets.lang;
    let Some(session) = app.session.as_mut() else { return };
    let slots = session.inventory.menu.as_ref().map(|m| m.slots.clone()).unwrap_or_default();
    let banner = slots.first().cloned().flatten();
    let dye = slots.get(1).cloned().flatten();
    let pattern_item = slots.get(2).cloned().flatten();
    let result = slots.get(3).cloned().flatten();
    let names = session.registries.as_ref().and_then(|r| r.names.get("banner_pattern")).cloned().unwrap_or_default();
    // With a pattern item, what it offers; without one, what needs no item.
    let offered: Vec<i32> = if banner.is_none() || dye.is_none() {
        Vec::new()
    } else {
        match &pattern_item {
            Some(stack) => match stack.raw("provides_banner_patterns").and_then(|raw| lattice_proto::item::read_raw(raw, lattice_proto::recipes::ItemSet::read)) {
                Some(set) => match &set.tag {
                    Some(t) => session.tag("banner_pattern", t).to_vec(),
                    None => set.items.clone(),
                },
                None => crate::game::items::default_banner_patterns(stack, session),
            },
            None => session.tag("banner_pattern", "no_item_required").to_vec(),
        }
    };
    let base = banner.as_ref().map_or(0xFFFFFF, |b| crate::bodies::mobs::dye_color(crate::game::items::item_name(b).trim_end_matches("_banner")));
    let dye_rgb = dye.as_ref().map_or(0xFFFFFF, |d| crate::bodies::mobs::dye_color(crate::game::items::item_name(d).trim_end_matches("_dye")));
    let selected = data_of(&session.inventory, 0);
    let grid = l.rect(40.0, 0.0, 74.0, 58.0);
    chrome::well(p, grid, false);
    if offered.is_empty() {
        chrome::note_in(p, grid, "Put in a banner and a dye");
    }
    let per_row = 5;
    let (cell_w, cell_h) = (14.0 * u, 28.0 * u);
    let inner = grid.shrink(u);
    let pointer = ui.ctx().pointer_latest_pos();
    let rows = offered.len().div_ceil(per_row);
    if pointer.is_some_and(|q| grid.contains(q)) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.menu.trade_scroll = (app.menu.trade_scroll - wheel / cell_h).clamp(0.0, rows.saturating_sub(2) as f32);
    }
    let first = app.menu.trade_scroll.floor() as usize * per_row;
    let mut tip = None;
    let mut pick = None;
    let mut textures = Vec::new();
    for (i, pattern) in offered.iter().enumerate().skip(first).take(2 * per_row) {
        let name = names.get(*pattern as usize).map_or(String::new(), |n| n.trim_start_matches("minecraft:").to_string());
        textures.push((i, name));
    }
    let ctx = ui.ctx().clone();
    for (i, name) in textures {
        let at = i - first;
        let r = Rect::from_min_size(inner.min + vec2((at % per_row) as f32 * cell_w, (at / per_row) as f32 * cell_h), vec2(cell_w, cell_h));
        let resp = ui.interact(r, egui::Id::unique(("choice", i)), Sense::click());
        crate::dev::note_widget(&format!("choice:{name}"), r);
        chrome::button(p, r, chrome::Face::Plain, resp.hovered() && selected != i as i32, selected == i as i32);
        let flag = Rect::from_center_size(r.center(), vec2(10.0, 20.0) * u);
        p.rect_filled(flag, CornerRadius::ZERO, rgb(base));
        if let Some(mask) = app.banner_masks.get_or_make(&ctx, assets, &name) {
            p.image(mask, flag, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), rgb(dye_rgb));
        }
        if resp.hovered() {
            let key = format!("block.minecraft.banner.{name}.{}", crate::game::items::item_name(dye.as_ref().unwrap_or(&ItemStack::new(0, 1))).trim_end_matches("_dye"));
            tip = Some(tr.translate(&key).map_or(name.clone(), str::to_string));
        }
        if resp.clicked() {
            pick = Some(i as i32);
        }
    }
    if rows > 2 {
        let track = Rect::from_min_size(Pos2::new(inner.right() - 2.0 * u, inner.top()), vec2(2.0 * u, inner.height()));
        p.rect_filled(track, CornerRadius::ZERO, Color32::from_black_alpha(90));
        let span = rows.saturating_sub(2).max(1) as f32;
        let h = track.height() * 2.0 / rows as f32;
        let y = track.top() + (track.height() - h) * app.menu.trade_scroll.floor() / span;
        p.rect_filled(Rect::from_min_size(Pos2::new(track.left(), y), vec2(track.width(), h)), CornerRadius::ZERO, chrome::TIP_MUTED);
    }
    chrome::arrow(p, l.rect(116.0, 20.0, 18.0, SLOT), 0.0);
    if let Some(i) = pick {
        if let Some(session) = app.session.as_mut() {
            session.conn.send(play::container_button_click(id, i));
        }
    }
    if let (Some(name), Some(q)) = (tip, pointer) {
        tooltip(&tip_layer(ui), q, screen, &[(name, chrome::TIP_INK)]);
    }
    // The whole banner as it would come out.
    if let Some(result) = &result {
        let flag = l.rect(144.0, 44.0, 11.0, 22.0);
        let (base, layers) = crate::game::items::banner_look(result, &names);
        p.rect_filled(flag, CornerRadius::ZERO, rgb(base));
        for (name, color) in layers {
            if let Some(mask) = app.banner_masks.get_or_make(&ctx, assets, &name) {
                p.image(mask, flag, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), rgb(color));
            }
        }
    }
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn enchant_name(session: &crate::game::Session, id: i32, tr: &dyn Translator) -> String {
    let name = session.registries.as_ref().and_then(|r| r.names.get("enchantment")).and_then(|n| n.get(id.max(0) as usize)).cloned().unwrap_or_default();
    let key = format!("enchantment.minecraft.{}", name.trim_start_matches("minecraft:"));
    tr.translate(&key).map(str::to_string).unwrap_or(name)
}

/// Text of a book's pages.
fn book_pages(stack: &ItemStack) -> Vec<String> {
    use lattice_proto::item::read_raw;
    if let Some(raw) = stack.raw("written_book_content") {
        return read_raw(raw, |r| {
            r.string(32767)?;
            r.option(|r| r.string(32767))?;
            r.string(32767)?;
            r.var_i32()?;
            let pages = r.array(|r| {
                let t = r.text()?;
                r.option(|r| r.nbt())?;
                Ok(t.to_plain(&lattice_proto::text::NoTranslations))
            })?;
            Ok(pages)
        })
        .unwrap_or_default();
    }
    if let Some(raw) = stack.raw("writable_book_content") {
        return read_raw(raw, |r| {
            r.array(|r| {
                let t = r.string(32767)?;
                r.option(|r| r.string(32767))?;
                Ok(t)
            })
        })
        .unwrap_or_default();
    }
    Vec::new()
}

/// The outline of a recipe waiting for its ingredients: which slot shows what.
fn ghost_stacks(session: &crate::game::Session, kind: &str) -> std::collections::HashMap<i16, ItemStack> {
    let mut out = std::collections::HashMap::new();
    if kind == "merchant" {
        if let Some(o) = session.inventory.menu.as_ref().and_then(|m| m.merchant.as_ref()).and_then(|m| m.offers.get(m.selected)) {
            let mut paying = o.cost_a.clone();
            paying.count = trade_cost(&o.cost_a, o.special_price, o.demand, o.price_multiplier);
            out.insert(0, paying);
            if let Some(b) = &o.cost_b {
                out.insert(1, b.clone());
            }
            if let Some(result) = &o.result {
                out.insert(2, result.clone());
            }
        }
        return out;
    }
    let Some((container, d)) = &session.ghost else { return out };
    if *container != session.inventory.menu.as_ref().map_or(0, |m| m.id) {
        return out;
    }
    let stacks: Vec<Option<ItemStack>> = d.slots.iter().map(|s| session.display_stack(s)).collect();
    match kind {
        "" | "crafting" => {
            let size = if kind.is_empty() { 2 } else { 3 };
            let (w, h) = if d.kind == 1 {
                (d.width as usize, d.height as usize)
            } else {
                let n = stacks.len();
                let side = (1..=size).find(|s| s * s >= n).unwrap_or(size);
                (side, side)
            };
            for (cell, stack) in super::recipes::ghost_slots(size, size, w, h, stacks.len()).into_iter().zip(stacks) {
                if let Some(stack) = stack {
                    out.insert(cell as i16 + 1, stack);
                }
            }
            if let Some(result) = session.display_stack(&d.result) {
                out.insert(0, result);
            }
        }
        "furnace" | "smoker" | "blast_furnace" => {
            if let Some(Some(input)) = stacks.first() {
                out.insert(0, input.clone());
            }
            if let Some(Some(fuel)) = stacks.get(1) {
                out.insert(1, fuel.clone());
            }
            if let Some(result) = session.display_stack(&d.result) {
                out.insert(2, result);
            }
        }
        _ => {}
    }
    out
}


/// The armour on the player, as pictures for the character in the inventory.
fn worn_pieces(app: &mut App, ctx: &egui::Context, assets: &Arc<GameAssets>) -> Vec<super::doll::Worn> {
    use super::doll::ArmourKind;
    let Some(session) = app.session.as_ref() else { return Vec::new() };
    let slots = &session.inventory.slots;
    let mut wanted = Vec::new();
    // Feet first, so what covers them draws over the top.
    for (slot, kind) in [(8, ArmourKind::Feet), (7, ArmourKind::Legs), (6, ArmourKind::Chest), (5, ArmourKind::Head)] {
        let Some(stack) = slots.get(slot).cloned().flatten() else { continue };
        let Some(asset) = crate::game::items::equipment_asset(&stack) else { continue };
        let Some(equipment) = assets.bodies.equipment(&asset) else { continue };
        let body = if kind == ArmourKind::Legs { "humanoid_leggings" } else { "humanoid" };
        let Some(layers) = equipment.layers.get(body) else { continue };
        for layer in layers {
            let tint = if layer.dyeable { stack.dyed_color().map(|c| c as u32).or(layer.undyed).map_or(Color32::WHITE, rgb) } else { Color32::WHITE };
            wanted.push((format!("entity/equipment/{body}/{}", layer.texture), kind, tint));
        }
    }
    wanted.into_iter().filter_map(|(path, kind, tint)| app.screen_pictures.get_or_make(ctx, assets, &path).map(|texture| super::doll::Worn { texture, kind, tint })).collect()
}

/// The player's active effects, down the right of the menu.
fn effect_list(app: &mut App, ui: &egui::Ui, panel: Rect, screen: Rect, assets: &Arc<GameAssets>) {
    let Some(session) = app.session.as_ref() else { return };
    let game = lattice_data::game();
    let mut list: Vec<(String, i32, i32, bool)> = session.player.effects.iter().filter_map(|(id, v)| game.mob_effects.get(*id as usize).map(|e| (e.name.clone(), v.0, v.1, e.category != "harmful"))).collect();
    if list.is_empty() {
        return;
    }
    list.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(&b.0)));
    let u = chrome::unit();
    let ctx = ui.ctx().clone();
    let x = panel.right() + 2.0 * u;
    let wide = screen.right() - x >= 84.0 * u;
    let (w, h) = (if wide { 84.0 } else { 26.0 }, 26.0);
    let p = ui.painter().clone();
    let room = ((panel.height() / ((h + 1.0) * u)).floor() as usize).max(1);
    for (i, (name, amp, ticks, _)) in list.iter().enumerate().take(room) {
        let r = Rect::from_min_size(Pos2::new(x, panel.top() + i as f32 * (h + 1.0) * u), vec2(w, h) * u);
        chrome::panel(&p, r);
        let picture = Rect::from_min_size(r.min + vec2(5.0, 5.0) * u, Vec2::splat(ICON * u));
        if let Some(tex) = app.screen_pictures.get_or_make(&ctx, assets, &format!("mob_effect/{name}")) {
            p.image(tex, picture, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        if wide {
            let shown = assets.lang.0.get(&format!("effect.minecraft.{name}")).cloned().unwrap_or_else(|| name.clone());
            let level = if *amp > 0 { format!(" {}", roman(*amp + 1)) } else { String::new() };
            let time = if *ticks < 0 { "Forever".to_string() } else { let s = *ticks / 20; format!("{}:{:02}", s / 60, s % 60) };
            let cx = r.left() + 23.0 * u;
            let clip = p.with_clip_rect(r.shrink(2.0 * u));
            words(&clip, Pos2::new(cx, r.top() + 3.0 * u), Align2::LEFT_TOP, format!("{shown}{level}"), ink());
            words(&clip, Pos2::new(cx, r.bottom() - 3.0 * u), Align2::LEFT_BOTTOM, time, ink_muted());
        }
    }
}

/// What the slot would hold if the drag in progress ended now.
fn spread_preview(stack: Option<&ItemStack>, carried: &ItemStack, each: i32) -> ItemStack {
    let max = crate::game::items::max_stack(carried);
    let mut s = carried.clone();
    s.count = (stack.map_or(0, |h| h.count) + each).min(max);
    s
}

/// The inventory or whichever menu is open.
pub fn draw(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(assets) = app.ready_assets() else { return };
    let tex = app.item_texture(ui.ctx());
    let creative = app.session.as_ref().is_some_and(|s| s.player.game_mode == 1 && s.inventory.menu.is_none());
    let doll = app.doll_skin(ui.ctx());
    // Under the panel but over the world: the HUD layer is Background, and this comes after it.
    let dim = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("menu_dim")));
    dim.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(165));
    let was = panel_mode(true);
    if creative {
        creative::draw(app, ui, screen, &assets, tex);
        panel_mode(was);
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let Some(session) = app.session.as_ref() else {
        panel_mode(was);
        return;
    };
    let kind = session.inventory.menu.as_ref().map_or(String::new(), |m| m.kind.clone());
    let book_kind = super::recipes::has_book(&kind);
    // How the book was left last time, which the server remembers per world.
    if let Some(books) = session.recipe_book_settings {
        let (open, filtering) = books[book_index(&kind) as usize];
        app.menu.book.open = open;
        app.menu.book.craftable_only = filtering;
        if let Some(session) = app.session.as_mut() {
            session.recipe_book_settings = None;
        }
    }
    let Some(session) = app.session.as_ref() else {
        panel_mode(was);
        return;
    };
    let u = chrome::unit();
    // Room for the book beside the menu, when it's open.
    let stage = if book_kind && app.menu.book.open { screen.translate(vec2((super::recipes::width() + 2.0 * u) / 2.0, 0.0)) } else { screen };
    let l = plan(&session.inventory, stage, ppp);
    let title = title_of(&session.inventory, &assets.lang);
    egui::Area::new(egui::Id::unique("menu_area")).order(egui::Order::Middle).fixed_pos(Pos2::ZERO).show(ui.ctx(), |ui| {
        ui.set_clip_rect(screen);
        on_panel(ui, |ui| {
            let p = ui.painter().clone();
            draw_menu(app, ui, &p, &l, screen, &kind, &title, book_kind, doll, &assets, tex);
        });
    });
    panel_mode(was);
}

#[allow(clippy::too_many_arguments)]
fn draw_menu(app: &mut App, ui: &mut egui::Ui, p: &Painter, l: &Plan, screen: Rect, kind: &str, title: &str, book_kind: bool, doll: Option<(egui::TextureId, bool)>, assets: &Arc<GameAssets>, tex: egui::TextureId) {
    let u = l.u;
    let panel = l.panel;
    // The recipe book's tab sticks out of the panel's side, behind it.
    let mut tab_rect = None;
    if book_kind {
        let tab = Rect::from_min_size(panel.left_top() + vec2(-22.0, 10.0) * u, vec2(26.0, 24.0) * u);
        let resp = ui.interact(tab, egui::Id::unique("recipe_tab"), Sense::click());
        crate::dev::note_widget("Recipes", tab);
        chrome::button(p, tab, if app.menu.book.open { chrome::Face::Slab } else { chrome::Face::Plain }, resp.hovered(), false);
        chrome::picture(p, Rect::from_min_size(tab.min, vec2(22.0, 24.0) * u), &chrome::BOOK, u, ink_muted(), |_, _| true);
        if resp.clicked() {
            app.menu.book.open = !app.menu.book.open;
            if let Some(s) = app.session.as_ref() {
                s.conn.send(play::recipe_book_settings(book_index(kind), app.menu.book.open, app.menu.book.craftable_only));
            }
        }
        if resp.hovered() {
            if let Some(q) = ui.ctx().pointer_latest_pos() {
                tooltip(&tip_layer(ui), q, screen, &[(if app.menu.book.open { "Close the recipes".to_string() } else { "Recipes".to_string() }, chrome::TIP_INK)]);
            }
        }
        tab_rect = Some(tab);
    }
    chrome::panel(p, panel);
    if !kind.is_empty() {
        words(p, panel.left_top() + vec2(l.origin.x - panel.left(), PAD * u), Align2::LEFT_TOP, title, ink());
        if kind != "lectern" && !matches!(kind, "merchant") {
            words(p, Pos2::new(l.origin.x, l.label_y), Align2::LEFT_TOP, "Inventory", ink_muted());
        } else if kind == "merchant" {
            // A villager's level beside their job.
            let level = app.session.as_ref().and_then(|s| s.inventory.menu.as_ref()).and_then(|m| m.merchant.as_ref()).filter(|m| m.show_progress && (1..=5).contains(&m.level)).map(|m| m.level);
            if let Some(level) = level {
                let shown = assets.lang.0.get(&format!("merchant.level.{level}")).cloned().unwrap_or_else(|| format!("Level {level}"));
                words(p, Pos2::new(l.at(WIDE, 0.0).x, panel.top() + PAD * u), Align2::RIGHT_TOP, shown, ink_muted());
            }
            words(p, Pos2::new(l.origin.x, l.label_y), Align2::LEFT_TOP, "Inventory", ink_muted());
        }
    } else {
        // You, between your armour and what you're crafting.
        let stage = l.rect(SLOT, 0.0, 3.0 * SLOT, 4.0 * SLOT);
        chrome::well(p, stage, false);
        if let Some((skin, slim)) = doll {
            let figure = stage.shrink(2.0 * u);
            let worn = worn_pieces(app, ui.ctx(), assets);
            super::doll::draw(&p.with_clip_rect(stage.shrink(u)), figure, skin, slim, ui.ctx().pointer_latest_pos(), &worn);
        }
        words(p, l.at(5.0 * SLOT + 2.0, 3.0), Align2::LEFT_TOP, "Crafting", ink_muted());
        chrome::picture(p, l.rect(128.0, SLOT + 9.0, 8.0, SLOT), &SMALL_ARROW, u, ink_muted(), |_, _| true);
    }
    draw_extras(app, ui, p, l, screen, assets, tex);
    let mut book_rect = None;
    if book_kind && app.menu.book.open {
        super::recipes::draw(app, ui, p, panel, screen, assets, tex);
        let w = super::recipes::width();
        book_rect = Some(Rect::from_min_size(Pos2::new(panel.left() - w - 2.0 * u, panel.top()), vec2(w, panel.height())));
    } else {
        effect_list(app, ui, panel, screen, assets);
    }

    // Slots and clicks.
    let Some(session) = app.session.as_ref() else { return };
    let (_, _, view) = session.inventory.view();
    let pointer = ui.ctx().pointer_latest_pos();
    let cursor = session.inventory.cursor.clone();
    let drag = app.menu.drag.clone();
    // How much each slot in a drag would get, once there's more than one to share between.
    let spread: Option<(i32, Vec<i16>)> = match (&drag, &cursor) {
        (Some((button, slots)), Some(c)) if slots.len() > 1 => Some((if *button == 1 { 1 } else { c.count / slots.len() as i32 }, slots.clone())),
        _ => None,
    };
    let mut hovered: Option<(i16, Option<ItemStack>, bool)> = None;
    let mut empty_hint: Option<&str> = None;
    let ghost = ghost_stacks(session, kind);
    let disabled = |s: i16| kind == "crafter_3x3" && s < 9 && data_of(&session.inventory, s) == 1;
    for placed in &l.slots {
        let (slot, rect, hint) = (placed.slot, placed.rect, placed.hint);
        crate::dev::note_widget(&format!("slot:{slot}"), rect);
        let resp = ui.interact(rect, egui::Id::unique(("slot", slot)), Sense::hover());
        let stack = view.get(slot as usize).cloned().flatten();
        let hot = pointer.is_some_and(|q| rect.contains(q));
        resp.widget_info(|| {
            let what = match &stack {
                Some(s) => format!("{} {}", s.count, crate::game::items::display_name(s, &assets.lang)),
                None if hint.is_empty() => "Empty slot".to_string(),
                None => format!("{hint}, empty"),
            };
            egui::WidgetInfo::labeled(egui::Role::Button, true, what)
        });
        if hot && stack.is_none() && !hint.is_empty() {
            empty_hint = Some(hint);
        }
        if placed.output {
            chrome::output_well(p, rect, hot);
        } else {
            chrome::well(p, rect, hot);
        }
        let picture = icon_in(rect, u);
        if disabled(slot) && stack.is_none() {
            chrome::picture(p, rect, &chrome::CROSS, u * 2.0, hex_of(0xA33B2A), |_, _| true);
        }
        let dragged = spread.as_ref().filter(|(_, slots)| slots.contains(&slot)).map(|(each, _)| *each);
        match (&stack, dragged, &cursor) {
            (held, Some(each), Some(c)) => {
                let preview = spread_preview(held.as_ref(), c, each);
                p.rect_filled(rect.shrink(u), CornerRadius::ZERO, hex_of(0x756D5C));
                draw_stack(p, picture, tex, assets, &preview, 0.0);
            }
            (Some(s), _, _) => draw_stack(p, picture, tex, assets, s, 0.0),
            (None, _, _) => {
                if !disabled(slot) {
                    chrome::slot_hint(p, rect, hint);
                }
            }
        }
        if stack.is_none() {
            if let Some(g) = ghost.get(&slot) {
                // What it'll take, faintly, so a chosen trade reads at a glance.
                draw_stack(p, picture, tex, assets, g, 0.0);
                let (r0, g0, b0, _) = (0x3Fu8, 0x39u8, 0x2Fu8, 0u8);
                p.rect_filled(rect.shrink(u), CornerRadius::ZERO, Color32::from_rgba_unmultiplied(r0, g0, b0, 170));
            }
        }
        if hot {
            hovered = Some((slot, stack, placed.output));
        }
    }

    let pressed = ui.input(|i| {
        if i.pointer.button_pressed(PointerButton::Primary) {
            Some(0u8)
        } else if i.pointer.button_pressed(PointerButton::Secondary) {
            Some(1)
        } else if i.pointer.button_pressed(PointerButton::Middle) {
            Some(2)
        } else {
            None
        }
    });
    let released = ui.input(|i| {
        if i.pointer.button_released(PointerButton::Primary) {
            Some(0u8)
        } else if i.pointer.button_released(PointerButton::Secondary) {
            Some(1)
        } else {
            None
        }
    });
    let held = ui.input(|i| i.pointer.button_down(PointerButton::Primary) || i.pointer.button_down(PointerButton::Secondary));
    let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.ctrl));
    let keys = std::mem::take(&mut app.menu.keys);
    let in_book = |q: Pos2| book_rect.is_some_and(|r: Rect| r.contains(q)) || tab_rect.is_some_and(|r: Rect| r.contains(q));
    let outside = pointer.is_some_and(|q| !panel.contains(q) && !in_book(q));
    // Clicks on the menu's own buttons and fields aren't about slots.
    let over_control = ui.ctx().egui_is_using_pointer() && hovered.is_none();

    let Some(session) = app.session.as_mut() else { return };
    let now = Instant::now();
    if let Some(button) = pressed {
        let slot = hovered.as_ref().map(|h| h.0);
        app.menu.double_click = button == 0 && slot.is_some() && app.menu.last_press.as_ref().is_some_and(|(s, b, at)| Some(*s) == slot && *b == button && now.duration_since(*at).as_secs_f32() < DOUBLE_CLICK);
        app.menu.last_press = slot.map(|s| (s, button, now));
        if let (true, Some(s), Some((_, stack, _))) = (kind == "crafter_3x3", slot, &hovered) {
            // An empty crafter slot switches off when pressed with nothing in hand.
            if s < 9 && stack.is_none() {
                let off = data_of(&session.inventory, s) == 1;
                if off || session.inventory.cursor.is_none() {
                    let id = session.inventory.menu.as_ref().map_or(0, |m| m.id);
                    session.conn.send(play::container_slot_state_changed(s as i32, id, off));
                    if let Some(m) = session.inventory.menu.as_mut() {
                        m.data.insert(s, if off { 0 } else { 1 });
                    }
                }
            }
        }
        if button == 2 {
            if let Some(s) = slot {
                session.conn.send(session.inventory.click(s, 2, click::CLONE));
            }
        } else if session.inventory.cursor.is_none() {
            if let Some(s) = slot {
                if shift {
                    app.menu.last_moved = hovered.as_ref().and_then(|h| h.1.clone());
                    session.conn.send(session.inventory.click(s, button as i8, click::QUICK_MOVE));
                } else {
                    let packet = session.inventory.click_pickup(s, button as i8);
                    session.conn.send(packet);
                }
                app.menu.skip_release = true;
            }
        } else if !over_control {
            app.menu.drag = Some((button, Vec::new()));
            app.menu.skip_release = false;
        }
    }
    // An item spreads across the slots it's dragged over, as far as it'll go round.
    if held {
        if let (Some(drag), Some(c)) = (app.menu.drag.as_mut(), &session.inventory.cursor) {
            if let Some((s, stack, output)) = &hovered {
                let room = match stack {
                    None => true,
                    Some(h) => h.same_item(c) && h.count < crate::game::items::max_stack(c),
                };
                if room && !*output && !drag.1.contains(s) && (drag.1.len() as i32) < c.count {
                    drag.1.push(*s);
                }
            }
        }
    }
    if let Some(button) = released {
        let shared_slot = hovered.as_ref().map(|h| h.0);
        if app.menu.double_click && button == 0 && shared_slot.is_some() {
            let s = shared_slot.unwrap_or(0);
            app.menu.double_click = false;
            app.menu.last_press = None;
            app.menu.drag = None;
            if shift {
                // Everything else like what was just moved goes the same way.
                if let Some(moved) = app.menu.last_moved.clone() {
                    let (_, _, view) = session.inventory.view();
                    let list: Vec<i16> = l.slots.iter().filter(|e| !e.output && view.get(e.slot as usize).cloned().flatten().is_some_and(|v| v.same_item(&moved))).map(|e| e.slot).collect();
                    for other in list {
                        session.conn.send(session.inventory.click(other, 0, click::QUICK_MOVE));
                    }
                }
            } else if session.inventory.cursor.is_some() {
                session.conn.send(session.inventory.click(s, 0, click::PICKUP_ALL));
            }
        } else if app.menu.skip_release {
            app.menu.skip_release = false;
        } else if let Some((b, slots)) = app.menu.drag.take() {
            if b == button {
                if slots.len() > 1 {
                    let mask = (b as i8) << 2;
                    session.conn.send(session.inventory.click(-999, mask, click::QUICK_CRAFT));
                    for s in &slots {
                        session.conn.send(session.inventory.click(*s, mask | 1, click::QUICK_CRAFT));
                    }
                    session.conn.send(session.inventory.click(-999, mask | 2, click::QUICK_CRAFT));
                } else if let Some(s) = slots.first().copied().or(shared_slot) {
                    let packet = session.inventory.click_pickup(s, b as i8);
                    session.conn.send(packet);
                } else if outside {
                    let packet = session.inventory.click_pickup(-1, b as i8);
                    session.conn.send(packet);
                }
            }
        }
    }
    if let Some((slot, stack, _)) = &hovered {
        for key in keys {
            match key {
                Action::Hotbar(n) => session.conn.send(session.inventory.click(*slot, n as i8, click::SWAP)),
                Action::SwapHands => session.conn.send(session.inventory.click(*slot, 40, click::SWAP)),
                Action::Drop if stack.is_some() => session.conn.send(session.inventory.click(*slot, i8::from(ctrl), click::THROW)),
                _ => {}
            }
        }
        // The wheel picks which item a bundle hands over next.
        if let Some(stack) = stack {
            let inside = stack.bundle_contents().len() as i32;
            if inside > 0 {
                let wheel = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::MouseWheel { delta, .. } = e { Some(delta.y) } else { None }).sum::<f32>());
                if wheel != 0.0 {
                    let pick = crate::game::items::bundle_selection(session, *slot, inside, wheel > 0.0);
                    session.conn.send(play::bundle_item_selected(*slot as i32, pick));
                }
            }
        }
    }
    // What's in hand follows the pointer; hovering shows what a stack is.
    let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("menu_top")));
    if let (Some(session), Some(q)) = (app.session.as_ref(), pointer) {
        if let Some(c) = &session.inventory.cursor {
            let mut shown = c.clone();
            if let Some((each, slots)) = &spread {
                shown.count = (c.count - each * slots.len() as i32).max(0);
            }
            if shown.count > 0 {
                draw_stack(&top, Rect::from_center_size(q, Vec2::splat(ICON * u)), tex, assets, &shown, 0.0);
            }
        } else if let Some((slot, Some(stack), _)) = &hovered {
            let names = session.registries.as_ref().and_then(|r| r.names.get("enchantment")).cloned().unwrap_or_default();
            let selected = crate::game::items::shown_bundle_selection(session, *slot);
            if !super::items::bundle_tooltip(&top, q, screen, stack, selected, &assets.lang, tex, assets) {
                tooltip(&top, q, screen, &tooltip_lines(stack, &assets.lang, &names));
            }
        } else if let Some(hint) = empty_hint {
            tooltip(&top, q, screen, &[(hint.to_string(), chrome::TIP_INK)]);
        }
    }
}
