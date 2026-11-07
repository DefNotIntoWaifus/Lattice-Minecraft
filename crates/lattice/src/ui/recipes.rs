//! The recipe book beside a crafting grid or furnace: what can be made, one click to set it up.

use super::items::{draw_stack, tooltip};
use super::theme::*;
use super::widgets;
use crate::app::App;
use crate::assets::GameAssets;
use crate::game::items;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, RichText, Sense, Vec2};
use lattice_proto::packets::play;
use lattice_proto::ItemStack;
use std::collections::HashMap;

const COLUMNS: usize = 6;

/// The panel's width on the current grid.
pub fn width() -> f32 {
    136.0 * super::chrome::unit()
}

/// One recipe as the panel shows it.
#[derive(Clone)]
pub struct Row {
    id: i32,
    stack: ItemStack,
    name: String,
    /// Each ingredient's possible items. Empty when the server didn't say.
    needs: Vec<Vec<i32>>,
    can_make: bool,
}

/// What the panel remembers while a menu is open.
#[derive(Default, Clone)]
pub struct BookState {
    pub open: bool,
    pub search: String,
    pub craftable_only: bool,
    pub scroll: f32,
    built_for: (String, usize),
    rows: Vec<Row>,
    since_check: u32,
}

impl BookState {
    pub fn names(&self) -> Vec<String> {
        self.rows.iter().map(|r| r.name.clone()).collect()
    }

    /// What carries over when this menu closes and another opens.
    pub fn for_next_menu(&self) -> BookState {
        BookState { open: self.open, craftable_only: self.craftable_only, ..Default::default() }
    }
}

/// Which menus have a book, by menu kind; the empty kind is the player's own inventory.
pub fn has_book(kind: &str) -> bool {
    matches!(kind, "" | "crafting" | "furnace" | "smoker" | "blast_furnace")
}

fn category_fits(kind: &str, category: &str) -> bool {
    let category = category.trim_start_matches("minecraft:");
    match kind {
        "" | "crafting" => category.starts_with("crafting_"),
        "furnace" => category.starts_with("furnace_"),
        "smoker" => category == "smoker_food",
        "blast_furnace" => category.starts_with("blast_furnace_"),
        _ => false,
    }
}

/// The ingredients could all come from these counts, each item used only once.
fn can_make(needs: &[Vec<i32>], have: &HashMap<i32, i32>) -> bool {
    if needs.is_empty() {
        return false;
    }
    let mut left = have.clone();
    let mut order: Vec<&Vec<i32>> = needs.iter().collect();
    // Fussy ingredients pick first, so a general one doesn't use up what they need.
    order.sort_by_key(|s| s.len());
    order.iter().all(|set| match set.iter().find(|i| left.get(i).is_some_and(|n| *n > 0)) {
        Some(i) => {
            if let Some(n) = left.get_mut(i) {
                *n -= 1;
            }
            true
        }
        None => false,
    })
}

fn rebuild(session: &crate::game::Session, kind: &str, tr: &dyn lattice_proto::Translator) -> Vec<Row> {
    let categories = lattice_data::game().registry("recipe_book_category");
    let mut rows = Vec::new();
    for e in &session.recipes {
        let category = categories.get(e.category as usize).map_or("", |s| s.as_str());
        if !category_fits(kind, category) {
            continue;
        }
        // The inventory's grid only takes what fits in two by two.
        if kind.is_empty() {
            let d = &e.display;
            let small = match d.kind {
                1 => d.width <= 2 && d.height <= 2,
                _ => d.slots.len() <= 4,
            };
            if !small {
                continue;
            }
        }
        let Some(stack) = session.display_stack(&e.display.result) else { continue };
        let needs = e
            .requirements
            .iter()
            .flatten()
            .map(|set| match &set.tag {
                Some(t) => session.tag("item", t).to_vec(),
                None => set.items.clone(),
            })
            .collect();
        rows.push(Row { id: e.id, name: items::display_name(&stack, tr), stack, needs, can_make: false });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// Draws the panel to the left of the menu.
pub fn draw(app: &mut App, ui: &mut egui::Ui, p: &Painter, panel: Rect, screen: Rect, assets: &GameAssets, tex: egui::TextureId) {
    let Some(session) = app.session.as_ref() else { return };
    let kind = session.inventory.menu.as_ref().map_or(String::new(), |m| m.kind.clone());
    let container = session.inventory.menu.as_ref().map_or(0, |m| m.id);
    let book = &mut app.menu.book;
    let tr: &dyn lattice_proto::Translator = &assets.lang;
    if book.built_for != (kind.clone(), session.recipes.len()) {
        book.rows = rebuild(session, &kind, tr);
        book.built_for = (kind.clone(), session.recipes.len());
        book.since_check = u32::MAX;
    }
    // What's at hand changes rarely, and working it out isn't free.
    book.since_check = book.since_check.saturating_add(1);
    if book.since_check > 15 {
        book.since_check = 0;
        let (_, _, view) = session.inventory.view();
        let mut have: HashMap<i32, i32> = HashMap::new();
        // Slot 0 of these menus is the result or the input being cooked.
        for s in view.iter().skip(1).flatten() {
            *have.entry(s.item).or_default() += s.count;
        }
        for row in &mut book.rows {
            row.can_make = can_make(&row.needs, &have);
        }
    }

    let u = super::chrome::unit();
    let cell_size = 20.0 * u;
    let rect = Rect::from_min_size(Pos2::new(panel.left() - width() - 2.0 * u, panel.top()), Vec2::new(width(), panel.height()));
    super::chrome::panel(p, rect);
    let pad = 7.0 * u;
    let line = super::chrome::words_size() * 1.2;
    super::chrome::text(p, rect.left_top() + Vec2::new(pad, pad), egui::Align2::LEFT_TOP, "Recipes", super::chrome::words(), ink());
    let controls = Rect::from_min_size(rect.left_top() + Vec2::new(pad, pad + line), Vec2::new(rect.width() - 2.0 * pad, 40.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(controls).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let search = child.add(egui::TextEdit::singleline(&mut book.search).hint_text(RichText::new("Search").color(ink_muted())).desired_width(rect.width() - 2.0 * pad - 120.0).font(super::chrome::words()));
    crate::dev::note_widget("recipe search", search.rect);
    if search.changed() {
        book.scroll = 0.0;
    }
    if widgets::secondary(&mut child, if book.craftable_only { "All" } else { "Can make" }).clicked() {
        book.craftable_only = !book.craftable_only;
        book.scroll = 0.0;
    }

    let needle = book.search.to_lowercase();
    let shown: Vec<usize> = (0..book.rows.len())
        .filter(|&i| {
            let r = &book.rows[i];
            (!book.craftable_only || r.can_make) && (needle.is_empty() || r.name.to_lowercase().contains(&needle))
        })
        .collect();
    let grid = Rect::from_min_max(controls.left_bottom() + Vec2::new(0.0, 8.0), Pos2::new(rect.right() - pad, rect.bottom() - pad));
    super::chrome::well(p, grid, false);
    if shown.is_empty() {
        let text = if session.recipes.is_empty() { "No recipes unlocked yet" } else { "Nothing matches" };
        super::chrome::note_in(p, grid, text);
    }
    let visible_rows = ((grid.height() - 2.0 * u) / cell_size).floor() as usize;
    let rows_total = shown.len().div_ceil(COLUMNS);
    let pointer = ui.ctx().pointer_latest_pos();
    if pointer.is_some_and(|q| grid.contains(q)) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        book.scroll = (book.scroll - wheel / cell_size).clamp(0.0, rows_total.saturating_sub(visible_rows) as f32);
    }
    let first_row = book.scroll.floor() as usize;
    let mut hovered = None;
    let mut chosen = None;
    for (n, &i) in shown.iter().enumerate().skip(first_row * COLUMNS).take(visible_rows * COLUMNS) {
        let at = n - first_row * COLUMNS;
        let cell = Rect::from_min_size(grid.min + Vec2::splat(u) + Vec2::new((at % COLUMNS) as f32 * cell_size, (at / COLUMNS) as f32 * cell_size), Vec2::splat(cell_size));
        let row = &book.rows[i];
        crate::dev::note_widget(&format!("recipe:{}", items::item_name(&row.stack)), cell);
        let resp = ui.interact(cell, egui::Id::unique(("recipe", row.id)), Sense::click());
        if resp.hovered() {
            p.rect_filled(cell, CornerRadius::ZERO, Color32::from_white_alpha(40));
            hovered = Some(i);
        }
        draw_stack(p, Rect::from_center_size(cell.center(), Vec2::splat(16.0 * u)), tex, assets, &row.stack, 0.0);
        if !row.can_make {
            p.rect_filled(cell, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x3F, 0x39, 0x2F, 170));
        }
        if resp.clicked() {
            chosen = Some(row.id);
        }
    }
    if rows_total > visible_rows {
        // In the panel's margin, beside the list.
        let track = Rect::from_min_max(Pos2::new(grid.right() + u, grid.top()), Pos2::new(grid.right() + 6.0 * u, grid.bottom()));
        super::chrome::scroller(p, track, book.scroll / (rows_total - visible_rows) as f32, visible_rows as f32 / rows_total as f32);
    }
    if let (Some(i), Some(q)) = (hovered, pointer) {
        let row = &book.rows[i];
        let mut lines = vec![(row.name.clone(), super::chrome::TIP_INK)];
        if !row.can_make {
            lines.push(("Some ingredients are missing".to_string(), super::chrome::TIP_MUTED));
        }
        let tip = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("recipe_tip")));
        tooltip(&tip, q, screen, &lines);
    }
    if let Some(id) = chosen {
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(session) = app.session.as_mut() {
            session.ghost = None;
            session.conn.send(play::place_recipe(container, id, shift));
        }
    }
}

/// Which slot each ingredient of a ghost recipe goes in, centred in the grid as the game does.
pub fn ghost_slots(width: usize, height: usize, recipe_w: usize, recipe_h: usize, count: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut y = 0;
    while y < height {
        let short = (recipe_h as f32) < height as f32 / 2.0;
        let line = (height as f32 / 2.0 - recipe_h as f32 / 2.0).floor() as usize;
        if short && line > y {
            i += width;
            y += 1;
        }
        for x in 0..width {
            if out.len() == count {
                return out;
            }
            let narrow = (recipe_w as f32) < width as f32 / 2.0;
            let offset = (width as f32 / 2.0 - recipe_w as f32 / 2.0).floor() as usize;
            let (end, take) = if narrow { (offset + recipe_w, offset <= x && x < offset + recipe_w) } else { (recipe_w, x < recipe_w) };
            if take {
                out.push(i);
            } else if end == x {
                i += width - x;
                break;
            }
            i += 1;
        }
        y += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingredients_are_each_used_once() {
        let mut have = HashMap::new();
        have.insert(1, 2);
        have.insert(2, 1);
        assert!(can_make(&[vec![1], vec![1], vec![2]], &have));
        assert!(!can_make(&[vec![1], vec![1], vec![1]], &have));
        // The general ingredient leaves the only 2 for the fussy one.
        assert!(can_make(&[vec![1, 2], vec![2]], &have));
        assert!(!can_make(&[], &have));
    }

    #[test]
    fn ghosts_sit_where_the_game_puts_them() {
        // A full three by three.
        assert_eq!(ghost_slots(3, 3, 3, 3, 9), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // One item in a three by three goes in the middle.
        assert_eq!(ghost_slots(3, 3, 1, 1, 1), vec![4]);
        // Two wide, one high, starts at the left of the middle row.
        assert_eq!(ghost_slots(3, 3, 2, 1, 2), vec![3, 4]);
        // A two by two in the player's grid fills it.
        assert_eq!(ghost_slots(2, 2, 2, 2, 4), vec![0, 1, 2, 3]);
    }
}
