//! The creative inventory: pages of everything in the game. The client picks from these
//! itself and tells the server what ended up where.

use super::inventory::Slot;
use super::items;
use crate::app::App;
use crate::assets::GameAssets;
use crate::ui::chrome;
use crate::ui::items::{draw_stack, tooltip, tooltip_lines};
use crate::ui::theme::*;
use egui::{Align2, CornerRadius, Pos2, Rect, RichText, Sense, Vec2};
use lattice_proto::packets::play;
use lattice_proto::{ItemStack, Writer};
use lattice_world::Registries;
use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// A slot and its picture, in grid units.
const SLOT: f32 = 18.0;
const ICON: f32 = 16.0;
const ROWS: usize = 6;

/// The nine hotbars kept in creative. Item ids change between versions of the game, so a
/// saved set only counts for the version it was saved in.
#[derive(Default)]
pub struct Toolbars {
    rows: Vec<Vec<Option<ItemStack>>>,
}

impl Toolbars {
    fn path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join(format!("hotbars-{}.bin", lattice_proto::PROTOCOL_VERSION))
    }

    pub fn load(dir: &std::path::Path) -> Toolbars {
        let mut out = Toolbars::default();
        let Ok(bytes) = std::fs::read(Self::path(dir)) else { return out };
        let mut r = lattice_proto::Reader::new(&bytes);
        let mut read = || -> lattice_proto::codec::DResult<Vec<Vec<Option<ItemStack>>>> {
            let mut rows = Vec::new();
            for _ in 0..9 {
                let mut row = Vec::new();
                for _ in 0..9 {
                    row.push(ItemStack::read_delimited(&mut r)?);
                }
                rows.push(row);
            }
            Ok(rows)
        };
        if let Ok(rows) = read() {
            out.rows = rows;
        }
        out
    }

    pub fn save(&self, dir: &std::path::Path) {
        let mut w = Writer::new();
        for row in 0..9 {
            let items = self.row(row);
            for stack in &items {
                ItemStack::write_delimited(&mut w, stack.as_ref());
            }
        }
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(Self::path(dir), &w.buf)) {
            tracing::warn!("couldn't save hotbars: {e}");
        }
    }

    pub fn set(&mut self, row: usize, items: Vec<Option<ItemStack>>) {
        self.rows.resize(9, vec![None; 9]);
        self.rows[row] = items;
    }

    pub fn row(&self, row: usize) -> Vec<Option<ItemStack>> {
        self.rows.get(row).cloned().unwrap_or_else(|| vec![None; 9])
    }
}

#[derive(serde::Deserialize, Clone)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub kind: String,
    pub row: String,
    pub column: i32,
    pub items: Vec<Vec<String>>,
}

pub fn tabs() -> &'static [Tab] {
    static TABS: OnceLock<Vec<Tab>> = OnceLock::new();
    TABS.get_or_init(|| serde_json::from_str(include_str!("../../../lattice-data/data/creative.json")).unwrap_or_default())
}

fn id_of(names: Option<&Vec<String>>, name: &str) -> Option<i32> {
    let n = name.trim_start_matches("minecraft:");
    names?.iter().position(|x| x.trim_start_matches("minecraft:") == n).map(|i| i as i32)
}

/// Turns one of the tab's entries into a stack. The components listed in tabs are few,
/// and only their network form is needed here.
pub fn stack_of(entry: &[String], reg: Option<&Registries>) -> Option<ItemStack> {
    let mut s = ItemStack::new(items::item_id(entry.first()?)?, 1);
    let Some(json) = entry.get(1) else { return Some(s) };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else { return Some(s) };
    let data = lattice_data::game();
    let names = |r: &str| reg.and_then(|x| x.names.get(r));
    for (k, v) in map {
        let key = k.trim_start_matches("minecraft:");
        let mut w = Writer::new();
        match key {
            "potion_contents" => {
                let potion = v.get("potion").and_then(Value::as_str).and_then(|p| data.registry("potion").iter().position(|n| *n == p.trim_start_matches("minecraft:")));
                match potion {
                    Some(p) => w.bool(true).var_i32(p as i32),
                    None => w.bool(false),
                };
                w.bool(false).var_i32(0).bool(false);
            }
            "stored_enchantments" => {
                let Value::Object(m) = &v else { continue };
                let list: Vec<(i32, i32)> = m.iter().filter_map(|(e, l)| Some((id_of(names("enchantment"), e)?, l.as_i64()? as i32))).collect();
                w.var_i32(list.len() as i32);
                for (e, l) in list {
                    w.var_i32(e).var_i32(l);
                }
            }
            "painting/variant" => {
                let Some(id) = v.as_str().and_then(|n| id_of(names("painting_variant"), n)) else { continue };
                w.var_i32(id + 1);
            }
            "instrument" => {
                let Some(id) = v.as_str().and_then(|n| id_of(names("instrument"), n)) else { continue };
                w.var_i32(id + 1);
            }
            "block_state" => {
                let Value::Object(m) = &v else { continue };
                w.var_i32(m.len() as i32);
                for (a, b) in m {
                    w.string(a).string(b.as_str().unwrap_or(""));
                }
            }
            "suspicious_stew_effects" => {
                let Value::Array(list) = &v else { continue };
                w.var_i32(list.len() as i32);
                for e in list {
                    let id = e.get("id").and_then(Value::as_str).and_then(|n| data.registry("mob_effect").iter().position(|x| *x == n.trim_start_matches("minecraft:"))).unwrap_or(0);
                    w.var_i32(id as i32).var_i32(e.get("duration").and_then(Value::as_i64).unwrap_or(160) as i32);
                }
            }
            "ominous_bottle_amplifier" => {
                w.var_i32(v.as_i64().unwrap_or(0) as i32);
            }
            "fireworks" => {
                w.var_i32(v.get("flight_duration").and_then(Value::as_i64).unwrap_or(1) as i32).var_i32(0);
            }
            _ => continue,
        }
        s.set_raw(key, w.buf);
    }
    Some(s)
}

/// Slots of the player's menu shown below the page: hotbar 36..=44.
fn slot_rect(left: f32, y: f32, col: usize) -> Rect {
    let step = SLOT * chrome::unit();
    Rect::from_min_size(Pos2::new(left + col as f32 * step, y), Vec2::splat(step))
}

/// The picture's place in the middle of a slot.
fn icon_in(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), Vec2::splat(ICON * chrome::unit()))
}

pub fn draw(app: &mut App, ui: &mut egui::Ui, screen: Rect, assets: &Arc<GameAssets>, tex: egui::TextureId) {
    let list = tabs();
    let shown: Vec<&Tab> = list.iter().collect();
    let u = chrome::unit();
    let step = SLOT * u;
    let tab_h = 24.0 * u;
    let line = chrome::words_size() * 1.2;
    let w = (7.0 + 9.0 * SLOT + 3.0 + 7.0 + 7.0) * u;
    let h = 7.0 * u + line + (ROWS as f32 * SLOT + 4.0 + SLOT + 7.0) * u;
    // On whole pixels, or every hard edge in it goes soft.
    let ppp = ui.ctx().pixels_per_point();
    let panel = Rect::from_min_size(chrome::snap(screen.center() - Vec2::new(w, h) / 2.0, ppp), Vec2::new(w, h));
    egui::Area::new(egui::Id::unique("creative")).order(egui::Order::Middle).fixed_pos(Pos2::ZERO).show(ui.ctx(), |ui| {
        ui.set_clip_rect(screen);
        on_panel(ui, |ui| {
        let p = ui.painter().clone();

        // Tabs stick out of the panel's top and bottom. Only the open one joins it.
        let mut change = None;
        let selected = app.menu.creative_tab.min(shown.len().saturating_sub(1));
        let tab_w = ((w - 2.0 * u) / 7.0 / u).floor() * u;
        let tab_rect = |t: &Tab| {
            let x = panel.left() + u + t.column as f32 * tab_w;
            if t.row == "top" {
                Rect::from_min_size(Pos2::new(x, panel.top() - tab_h + 2.0 * u), Vec2::new(tab_w - u, tab_h))
            } else {
                Rect::from_min_size(Pos2::new(x, panel.bottom() - 2.0 * u), Vec2::new(tab_w - u, tab_h))
            }
        };
        let mut draw_tab = |ui: &mut egui::Ui, i: usize, t: &Tab| {
            let r = tab_rect(t);
            let resp = ui.interact(r, egui::Id::unique(("creative_tab", i)), Sense::click());
            crate::dev::note_widget(&format!("tab:{}", t.id), r);
            if i == selected {
                chrome::button(&p, r, chrome::Face::Slab, false, false);
                // Runs on into the panel, with no edge between them.
                let join = if t.row == "top" { Rect::from_min_size(Pos2::new(r.left() + 2.0 * u, r.bottom() - 4.0 * u), Vec2::new(r.width() - 4.0 * u, 5.0 * u)) } else { Rect::from_min_size(Pos2::new(r.left() + 2.0 * u, r.top() - u), Vec2::new(r.width() - 4.0 * u, 5.0 * u)) };
                p.rect_filled(join, CornerRadius::ZERO, surface());
            } else {
                chrome::button(&p, r, chrome::Face::Plain, resp.hovered(), false);
            }
            if let Some(icon) = items::item_id(&t.icon).map(|id| ItemStack::new(id, 1)) {
                let toward = if t.row == "top" { -u } else { u };
                draw_stack(&p, Rect::from_center_size(r.center() + Vec2::new(0.0, toward), Vec2::splat(ICON * u)), tex, assets, &icon, 0.0);
            }
            let name = assets.lang.0.get(&t.title).cloned().unwrap_or_else(|| t.title.clone());
            if resp.on_hover_text(name).clicked() {
                change = Some(i);
            }
        };
        for (i, t) in shown.iter().enumerate() {
            if i != selected {
                draw_tab(ui, i, t);
            }
        }
        chrome::panel(&p, panel);
        if let Some(t) = shown.get(selected) {
            draw_tab(ui, selected, t);
        }
        if let Some(i) = change {
            app.menu.creative_tab = i;
            app.menu.creative_scroll = 0.0;
        }
        let tab = shown[app.menu.creative_tab.min(shown.len().saturating_sub(1))];
        let left = panel.left() + 7.0 * u;
        let mut y = panel.top() + 7.0 * u;
        let title = assets.lang.0.get(&tab.title).cloned().unwrap_or_else(|| tab.title.clone());
        chrome::text(&p, Pos2::new(left, y), Align2::LEFT_TOP, &title, chrome::words(), ink());
        let is_search = tab.kind == "search";
        if is_search {
            let rect = Rect::from_min_size(Pos2::new(left + 60.0 * u, y - u), Vec2::new(w - 74.0 * u, line));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
            let r = child.add(egui::TextEdit::singleline(&mut app.menu.creative_search).hint_text(RichText::new("Search").color(ink_muted())).desired_width(rect.width()).font(chrome::words()));
            crate::dev::note_widget("search", r.rect);
            if r.changed() {
                app.menu.creative_scroll = 0.0;
            }
            if !r.has_focus() && app.menu.creative_search.is_empty() {
                r.request_focus();
            }
        }
        y += line;

        let pointer = ui.ctx().pointer_latest_pos();
        let reg = app.session.as_ref().and_then(|s| s.registries.clone());
        let mut hovered: Option<ItemStack> = None;
        let mut picked: Option<(ItemStack, bool)> = None;
        let mut put_back = false;
        let cols = 9usize;
        if tab.kind == "inventory" {
            // The survival slots, moved by telling the server what's in each.
            let Some(session) = app.session.as_mut() else { return };
            let mut layout: Vec<(i16, Rect)> = Vec::new();
            for (i, s) in [5i16, 6, 7, 8, 45].into_iter().enumerate() {
                layout.push((s, slot_rect(left, y, i)));
            }
            for r in 0..3 {
                for c in 0..cols {
                    layout.push(((9 + r * cols + c) as i16, slot_rect(left, y + (r + 1) as f32 * step + 4.0 * u, c)));
                }
            }
            for c in 0..cols {
                layout.push(((36 + c) as i16, panel_hotbar(slot_rect(left, panel.bottom() - 7.0 * u - step, c))));
            }
            draw_creative_slots(ui, &p, &layout, session, tex, assets, pointer, &mut hovered);
        } else if tab.kind != "hotbar" {
            let needle = app.menu.creative_search.to_lowercase();
            let entries: Vec<&Vec<String>> = if is_search && !needle.is_empty() {
                tab.items
                    .iter()
                    .filter(|e| {
                        e.first().and_then(|n| items::item_id(n)).is_some_and(|id| {
                            let name = items::display_name(&ItemStack::new(id, 1), &assets.lang).to_lowercase();
                            name.contains(&needle) || e[0].contains(&needle.replace(' ', "_"))
                        })
                    })
                    .collect()
            } else {
                tab.items.iter().collect()
            };
            let rows_total = entries.len().div_ceil(cols);
            let max_scroll = rows_total.saturating_sub(ROWS) as f32;
            let grid = Rect::from_min_size(Pos2::new(left, y), Vec2::new(cols as f32 * step, ROWS as f32 * step));
            if pointer.is_some_and(|q| grid.contains(q)) {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                app.menu.creative_scroll = (app.menu.creative_scroll - wheel / step).clamp(0.0, max_scroll);
            }
            let first_row = app.menu.creative_scroll.round() as usize;
            for row in 0..ROWS {
                for col in 0..cols {
                    let r = slot_rect(left, y + row as f32 * step, col);
                    let hot = pointer.is_some_and(|q| r.contains(q));
                    let index = (first_row + row) * cols + col;
                    chrome::well(&p, r, hot && entries.get(index).is_some());
                    let Some(entry) = entries.get(index) else { continue };
                    let Some(stack) = stack_of(entry, reg.as_deref()) else { continue };
                    let resp = ui.interact(r, egui::Id::unique(("creative_item", index)), Sense::click());
                    crate::dev::note_widget(&format!("item:{}", entry[0]), r);
                    if hot {
                        hovered = Some(stack.clone());
                    }
                    draw_stack(&p, icon_in(r), tex, assets, &stack, 0.0);
                    if resp.clicked() {
                        picked = Some((stack, false));
                    } else if resp.secondary_clicked() {
                        picked = Some((stack, true));
                    }
                }
            }
            if rows_total > ROWS {
                let track = Rect::from_min_size(Pos2::new(left + cols as f32 * step + 3.0 * u, y), Vec2::new(7.0 * u, ROWS as f32 * step));
                chrome::scroller(&p, track, app.menu.creative_scroll / max_scroll.max(1.0), ROWS as f32 / rows_total as f32);
            }
            // Clicking the page with something in hand throws it away.
            if pointer.is_some_and(|q| grid.contains(q)) && ui.input(|i| i.pointer.any_click()) {
                put_back = true;
            }
        } else {
            // Nine saved rows, each with its number. Taken from like any other page.
            chrome::text(&p, Pos2::new(panel.right() - 7.0 * u, panel.top() + 7.0 * u), Align2::RIGHT_TOP, "C and a number saves, X loads", chrome::words(), ink_muted());
            let rows_total = 9usize;
            let max_scroll = (rows_total - ROWS) as f32;
            let grid = Rect::from_min_size(Pos2::new(left, y), Vec2::new(cols as f32 * step, ROWS as f32 * step));
            if pointer.is_some_and(|q| grid.contains(q)) {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                app.menu.creative_scroll = (app.menu.creative_scroll - wheel / step).clamp(0.0, max_scroll);
            }
            let first_row = app.menu.creative_scroll.round() as usize;
            for row in 0..ROWS {
                let saved = app.toolbars.row(first_row + row);
                for (col, stack) in saved.iter().enumerate() {
                    let r = slot_rect(left, y + row as f32 * step, col);
                    let hot = pointer.is_some_and(|q| r.contains(q));
                    chrome::well(&p, r, hot && stack.is_some());
                    let Some(stack) = stack else { continue };
                    let resp = ui.interact(r, egui::Id::unique(("saved_item", first_row + row, col)), Sense::click());
                    crate::dev::note_widget(&format!("saved:{}:{col}", first_row + row), r);
                    if hot {
                        hovered = Some(stack.clone());
                    }
                    draw_stack(&p, icon_in(r), tex, assets, stack, 0.0);
                    if resp.clicked() {
                        picked = Some((stack.clone(), false));
                    } else if resp.secondary_clicked() {
                        picked = Some((stack.clone(), true));
                    }
                }
            }
            let track = Rect::from_min_size(Pos2::new(left + cols as f32 * step + 3.0 * u, y), Vec2::new(7.0 * u, ROWS as f32 * step));
            chrome::scroller(&p, track, app.menu.creative_scroll / max_scroll, ROWS as f32 / rows_total as f32);
        }

        let Some(session) = app.session.as_mut() else { return };
        if tab.kind != "inventory" {
            let hot_y = panel.bottom() - 7.0 * u - step;
            let layout: Vec<(i16, Rect)> = (0..9).map(|c| ((36 + c) as i16, slot_rect(left, hot_y, c))).collect();
            draw_creative_slots(ui, &p, &layout, session, tex, assets, pointer, &mut hovered);
        }
        if let Some((stack, one)) = picked {
            match &session.inventory.cursor {
                Some(c) if c.same_item(&stack) && !one => {
                    let max = items::max_stack(c);
                    if let Some(c) = &mut session.inventory.cursor {
                        c.count = (c.count + 1).min(max);
                    }
                }
                Some(_) => session.inventory.cursor = None,
                None => {
                    let mut s = stack;
                    s.count = if one { 1 } else { items::max_stack(&s) };
                    session.inventory.cursor = Some(s);
                }
            }
        } else if put_back && session.inventory.cursor.is_some() {
            session.inventory.cursor = None;
        }
        // Dropping outside the panel throws the stack into the world.
        if pointer.is_some_and(|q| !panel.contains(q)) && ui.input(|i| i.pointer.any_click()) {
            if let Some(c) = session.inventory.cursor.take() {
                session.conn.send(play::set_creative_mode_slot(-1, Some(&c)));
            }
        }
        let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("menu_top")));
        if let (Some(session), Some(q)) = (app.session.as_ref(), pointer) {
            if let Some(c) = &session.inventory.cursor {
                draw_stack(&top, Rect::from_center_size(q, Vec2::splat(ICON * u)), tex, assets, c, 0.0);
            } else if let Some(stack) = &hovered {
                let names = session.registries.as_ref().and_then(|r| r.names.get("enchantment")).cloned().unwrap_or_default();
                tooltip(&top, q, screen, &tooltip_lines(stack, &assets.lang, &names));
            }
        }
        });
    });
}

/// Keeps a slot rectangle as it is; the hotbar sits on the panel's last row in every tab.
fn panel_hotbar(r: Rect) -> Rect {
    r
}

#[allow(clippy::too_many_arguments)]
fn draw_creative_slots(ui: &mut egui::Ui, p: &egui::Painter, layout: &[(i16, Rect)], session: &mut crate::game::Session, tex: egui::TextureId, assets: &Arc<GameAssets>, pointer: Option<Pos2>, hovered: &mut Option<ItemStack>) {
    for (slot, r) in layout {
        crate::dev::note_widget(&format!("slot:{slot}"), *r);
        chrome::well(p, *r, pointer.is_some_and(|q| r.contains(q)));
        let resp = ui.interact(*r, egui::Id::unique(("creative_slot", *slot)), Sense::click());
        let here: Slot = session.inventory.slots.get(*slot as usize).cloned().flatten();
        let hot = pointer.is_some_and(|q| r.contains(q));
        if hot {
            if here.is_some() {
                *hovered = here.clone();
            }
        }
        if let Some(s) = &here {
            draw_stack(p, icon_in(*r), tex, assets, s, 0.0);
        }
        if resp.clicked() || resp.secondary_clicked() {
            let cursor = session.inventory.cursor.take();
            let right = resp.secondary_clicked();
            let (new_here, new_cursor) = match (here, cursor) {
                (None, None) => (None, None),
                (Some(s), None) => (None, Some(s)),
                (None, Some(c)) if right => {
                    let mut one = c.clone();
                    one.count = 1;
                    let mut left = c;
                    left.count -= 1;
                    (Some(one), (left.count > 0).then_some(left))
                }
                (None, Some(c)) => (Some(c), None),
                (Some(mut s), Some(c)) if s.same_item(&c) => {
                    let room = (items::max_stack(&s) - s.count).max(0);
                    let n = if right { 1.min(room) } else { c.count.min(room) };
                    s.count += n;
                    let mut left = c;
                    left.count -= n;
                    (Some(s), (left.count > 0).then_some(left))
                }
                (Some(s), Some(c)) => (Some(c), Some(s)),
            };
            session.conn.send(play::set_creative_mode_slot(*slot, new_here.as_ref()));
            if let Some(entry) = session.inventory.slots.get_mut(*slot as usize) {
                *entry = new_here;
            }
            session.inventory.cursor = new_cursor;
        }
    }
}
