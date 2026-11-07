//! The player's inventory and whatever menu is open, kept in step with the server.
//!
//! Slots of the player's own menu (container 0): 0 crafting result, 1–4 crafting
//! grid, 5–8 armour head to feet, 9–35 main, 36–44 hotbar, 45 offhand.

use super::items;
use lattice_proto::packets::play::{self, MerchantOffer, click};
use lattice_proto::{ItemStack, Text};
use std::collections::HashMap;

pub type Slot = Option<ItemStack>;

pub const PLAYER_SLOTS: usize = 46;
pub const HOTBAR_START: usize = 36;
pub const OFFHAND: usize = 45;
pub const ARMOR_HEAD: usize = 5;
pub const ARMOR_CHEST: usize = 6;
pub const ARMOR_LEGS: usize = 7;
pub const ARMOR_FEET: usize = 8;

/// A menu the server opened.
#[derive(Debug, Clone)]
pub struct Menu {
    pub id: i32,
    /// Name in the menu registry: "generic_9x3", "crafting", "merchant", ...
    pub kind: String,
    pub title: Text,
    pub slots: Vec<Slot>,
    pub state: i32,
    /// Bars and buttons, from `container_set_data`.
    pub data: HashMap<i16, i16>,
    pub merchant: Option<Merchant>,
    pub columns: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Merchant {
    pub offers: Vec<MerchantOffer>,
    pub level: i32,
    pub xp: i32,
    pub show_progress: bool,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    pub slots: Vec<Slot>,
    pub state: i32,
    pub cursor: Slot,
    pub menu: Option<Menu>,
    /// The inventory screen is open (the server doesn't know or care).
    pub own_screen_open: bool,
    /// Cooldown group to (ticks left, total ticks).
    pub cooldowns: HashMap<String, (i32, i32)>,
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory { slots: vec![None; PLAYER_SLOTS], state: 0, cursor: None, menu: None, own_screen_open: false, cooldowns: HashMap::new() }
    }
}

/// How many slots a menu has before the player's own 36 begin.
pub fn menu_own_slots(kind: &str, columns: i32) -> usize {
    match kind {
        "generic_9x1" => 9,
        "generic_9x2" => 18,
        "generic_9x3" | "shulker_box" => 27,
        "generic_9x4" => 36,
        "generic_9x5" => 45,
        "generic_9x6" => 54,
        // A crafter's result is slot 45, after the player's own.
        "generic_3x3" | "crafter_3x3" => 9,
        "anvil" | "smithing" if kind == "smithing" => 4,
        "anvil" => 3,
        "beacon" => 1,
        "blast_furnace" | "furnace" | "smoker" => 3,
        "brewing_stand" => 5,
        "crafting" => 10,
        "enchantment" => 2,
        "grindstone" => 3,
        "hopper" => 5,
        "lectern" => 1,
        "loom" => 4,
        "merchant" => 3,
        "cartography_table" => 3,
        "stonecutter" => 2,
        // Saddle and armour slots, plus the chest columns if there are any.
        "mount" => 2 + columns as usize * 3,
        _ => 0,
    }
}

impl Inventory {
    /// The stack in hotbar position `i`.
    pub fn hotbar(&self, i: usize) -> Option<&ItemStack> {
        self.slots.get(HOTBAR_START + i)?.as_ref()
    }

    pub fn offhand(&self) -> Option<&ItemStack> {
        self.slots[OFFHAND].as_ref()
    }

    /// Slots currently on show, with their state id and container id.
    pub fn view(&self) -> (i32, i32, &[Slot]) {
        match &self.menu {
            Some(m) => (m.id, m.state, &m.slots),
            None => (0, self.state, &self.slots),
        }
    }

    fn view_mut(&mut self) -> &mut Vec<Slot> {
        match &mut self.menu {
            Some(m) => &mut m.slots,
            None => &mut self.slots,
        }
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_some() || self.own_screen_open
    }

    pub fn open_menu(&mut self, id: i32, kind: String, title: Text) {
        self.own_screen_open = false;
        self.menu = Some(Menu { id, kind, title, slots: Vec::new(), state: 0, data: HashMap::new(), merchant: None, columns: 0 });
    }

    pub fn open_mount(&mut self, id: i32, columns: i32) {
        self.own_screen_open = false;
        self.menu = Some(Menu { id, kind: "mount".into(), title: Text::literal(""), slots: Vec::new(), state: 0, data: HashMap::new(), merchant: None, columns });
    }

    pub fn set_content(&mut self, container: i32, state: i32, slots: Vec<Slot>, carried: Slot) {
        if container == 0 {
            self.state = state;
            let n = slots.len().min(PLAYER_SLOTS);
            self.slots[..n].clone_from_slice(&slots[..n]);
        } else if let Some(m) = &mut self.menu {
            if m.id == container {
                m.state = state;
                m.slots = slots;
                self.mirror_shared_slots();
            }
        }
        self.cursor = carried;
    }

    pub fn set_slot(&mut self, container: i32, state: i32, slot: i16, item: Slot) {
        let i = slot as usize;
        if container == 0 {
            self.state = state;
            if i < PLAYER_SLOTS {
                self.slots[i] = item;
            }
        } else if let Some(m) = &mut self.menu {
            if m.id == container {
                m.state = state;
                if i < m.slots.len() {
                    m.slots[i] = item;
                    self.mirror_shared_slots();
                }
            }
        }
    }

    /// `set_player_inventory` numbers slots by the player's inventory, not a menu.
    pub fn set_inventory_slot(&mut self, slot: i32, item: Slot) {
        let i = match slot {
            0..=8 => HOTBAR_START + slot as usize,
            9..=35 => slot as usize,
            36 => ARMOR_FEET,
            37 => ARMOR_LEGS,
            38 => ARMOR_CHEST,
            39 => ARMOR_HEAD,
            40 => OFFHAND,
            _ => return,
        };
        self.slots[i] = item;
    }

    /// While a menu is open its last 36 slots are the player's main and hotbar slots.
    fn mirror_shared_slots(&mut self) {
        let Some(m) = &self.menu else { return };
        let own = menu_own_slots(&m.kind, m.columns);
        if m.slots.len() == own + 36 {
            let shared: Vec<Slot> = m.slots[own..].to_vec();
            self.slots[9..45].clone_from_slice(&shared);
        }
    }

    pub fn set_data(&mut self, container: i32, property: i16, value: i16) {
        if let Some(m) = &mut self.menu {
            if m.id == container {
                m.data.insert(property, value);
            }
        }
    }

    /// Closes whatever is open on this client. Returns the container the server should hear about.
    pub fn close(&mut self) -> Option<i32> {
        self.own_screen_open = false;
        // Whatever was held on the cursor goes back or is dropped by the server, which then
        // tells us where it went.
        self.cursor = None;
        self.menu.take().map(|m| m.id).or(Some(0))
    }

    pub fn tick(&mut self) {
        self.cooldowns.retain(|_, (left, _)| {
            *left -= 1;
            *left > 0
        });
    }

    /// Clicks with the left or right button. The server's answer is what counts; this
    /// applies the plain cases right away so the menu feels immediate.
    pub fn click_pickup(&mut self, slot: i16, button: i8) -> play_packet::Encoded {
        let (id, state, _) = self.view();
        if slot < 0 {
            let carried = self.cursor.clone();
            let mut after = carried.clone();
            if button == 0 {
                after = None;
            } else if let Some(c) = &mut after {
                c.count -= 1;
                if c.count <= 0 {
                    after = None;
                }
            }
            self.cursor = after;
            return play::container_click(id, state, -999, button, click::PICKUP, &[], self.cursor.as_ref());
        }
        let i = slot as usize;
        let current = self.view_mut().get(i).cloned().flatten();
        let cursor = self.cursor.clone();
        // A bundle takes from or gives to what it's used on: only the server knows how much fits.
        let bundle = |s: &Slot| s.as_ref().is_some_and(|s| crate::game::items::has_component(s, "bundle_contents"));
        if bundle(&current) || bundle(&cursor) {
            return play::container_click(id, state, slot, button, click::PICKUP, &[], self.cursor.as_ref());
        }
        let result: Option<(Slot, Slot)> = match (&current, &cursor) {
            (None, None) => None,
            (None, Some(c)) => {
                let n = if button == 0 { c.count } else { 1 };
                let mut placed = c.clone();
                placed.count = n;
                let mut left = c.clone();
                left.count -= n;
                Some((Some(placed), (left.count > 0).then_some(left)))
            }
            (Some(s), None) => {
                let n = if button == 0 { s.count } else { (s.count + 1) / 2 };
                let mut taken = s.clone();
                taken.count = n;
                let mut left = s.clone();
                left.count -= n;
                Some(((left.count > 0).then_some(left), Some(taken)))
            }
            (Some(s), Some(c)) if s.same_item(c) => {
                let max = items::max_stack(s);
                let room = (max - s.count).max(0);
                let n = if button == 0 { c.count.min(room) } else { 1.min(room) };
                let mut merged = s.clone();
                merged.count += n;
                let mut left = c.clone();
                left.count -= n;
                Some((Some(merged), (left.count > 0).then_some(left)))
            }
            (Some(s), Some(c)) => Some((Some(c.clone()), Some(s.clone()))),
        };
        let mut changed = Vec::new();
        if let Some((new_slot, new_cursor)) = result {
            // Result and output slots behave differently; the server corrects those.
            if self.is_plain_slot(i) {
                if let Some(s) = self.view_mut().get_mut(i) {
                    *s = new_slot.clone();
                }
                self.mirror_shared_slots();
                changed.push((slot, new_slot));
                self.cursor = new_cursor;
            }
        }
        let (id, state, _) = self.view();
        play::container_click(id, state, slot, button, click::PICKUP, &changed, self.cursor.as_ref())
    }

    /// Slots where placing and taking follow the plain rules, so the outcome can be predicted.
    fn is_plain_slot(&self, i: usize) -> bool {
        match &self.menu {
            None => (9..=44).contains(&i),
            Some(m) => {
                let own = menu_own_slots(&m.kind, m.columns);
                i >= own || m.kind.starts_with("generic_") || m.kind == "shulker_box" || m.kind == "hopper"
            }
        }
    }

    /// Any other kind of click: only the server knows what comes of it.
    pub fn click(&self, slot: i16, button: i8, input: i32) -> play_packet::Encoded {
        let (id, state, _) = self.view();
        play::container_click(id, state, slot, button, input, &[], self.cursor.as_ref())
    }
}

/// Re-exported so the signatures above read plainly.
mod play_packet {
    pub use lattice_proto::packets::Encoded;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: i32, count: i32) -> Slot {
        Some(ItemStack::new(item, count))
    }

    #[test]
    fn hotbar_positions() {
        let mut inv = Inventory::default();
        inv.set_inventory_slot(0, stack(5, 1));
        inv.set_inventory_slot(8, stack(6, 2));
        inv.set_inventory_slot(40, stack(7, 3));
        assert_eq!(inv.hotbar(0).unwrap().item, 5);
        assert_eq!(inv.hotbar(8).unwrap().count, 2);
        assert_eq!(inv.offhand().unwrap().item, 7);
    }

    #[test]
    fn left_click_picks_up_and_places() {
        let mut inv = Inventory::default();
        inv.slots[9] = stack(1, 10);
        inv.click_pickup(9, 0);
        assert!(inv.slots[9].is_none());
        assert_eq!(inv.cursor.as_ref().unwrap().count, 10);
        inv.click_pickup(10, 0);
        assert_eq!(inv.slots[10].as_ref().unwrap().count, 10);
        assert!(inv.cursor.is_none());
    }

    #[test]
    fn right_click_splits_and_places_one() {
        let mut inv = Inventory::default();
        inv.slots[9] = stack(1, 9);
        inv.click_pickup(9, 1);
        assert_eq!(inv.cursor.as_ref().unwrap().count, 5);
        assert_eq!(inv.slots[9].as_ref().unwrap().count, 4);
        inv.click_pickup(10, 1);
        assert_eq!(inv.slots[10].as_ref().unwrap().count, 1);
        assert_eq!(inv.cursor.as_ref().unwrap().count, 4);
    }

    #[test]
    fn menu_shares_the_players_slots() {
        let mut inv = Inventory::default();
        inv.open_menu(3, "generic_9x1".into(), Text::literal("Chest"));
        let mut slots = vec![None; 9 + 36];
        slots[9] = stack(1, 4);
        slots[9 + 35] = stack(2, 8);
        inv.set_content(3, 1, slots, None);
        assert_eq!(inv.slots[9].as_ref().unwrap().item, 1);
        assert_eq!(inv.hotbar(8).unwrap().item, 2);
    }
}
