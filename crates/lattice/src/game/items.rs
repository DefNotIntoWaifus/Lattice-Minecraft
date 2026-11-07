//! What a stack is: its components, with the item's defaults filled in.

use lattice_data::GameData;
use lattice_proto::item::read_raw;
use lattice_proto::{ItemStack, Reader, Text, Translator};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// All block tags a server has told us about, as block indices.
#[derive(Debug, Default, Clone)]
pub struct BlockTags(pub HashMap<String, HashSet<u16>>);

impl BlockTags {
    pub fn contains(&self, tag: &str, block: u16) -> bool {
        self.0.get(tag.trim_start_matches("minecraft:")).is_some_and(|s| s.contains(&block))
    }
}

/// A set of blocks as a component states it.
#[derive(Debug, Clone)]
pub enum BlockSet {
    Tag(String),
    Blocks(Vec<u16>),
}

impl BlockSet {
    pub fn contains(&self, tags: &BlockTags, block: u16) -> bool {
        match self {
            BlockSet::Tag(t) => tags.contains(t, block),
            BlockSet::Blocks(list) => list.contains(&block),
        }
    }

    fn from_json(v: &Value, data: &GameData) -> Option<BlockSet> {
        let find = |n: &str| data.blocks.iter().position(|b| b.name == n.trim_start_matches("minecraft:")).map(|i| i as u16);
        match v {
            Value::String(s) if s.starts_with('#') => Some(BlockSet::Tag(s[1..].trim_start_matches("minecraft:").to_string())),
            Value::String(s) => Some(BlockSet::Blocks(find(s).into_iter().collect())),
            Value::Array(a) => Some(BlockSet::Blocks(a.iter().filter_map(|x| x.as_str().and_then(find)).collect())),
            _ => None,
        }
    }

    fn read(r: &mut Reader) -> Result<BlockSet, lattice_proto::DecodeError> {
        let n = r.var_i32()?;
        if n == 0 {
            return Ok(BlockSet::Tag(r.identifier()?.trim_start_matches("minecraft:").to_string()));
        }
        let mut list = Vec::with_capacity((n - 1).max(0) as usize);
        for _ in 0..(n - 1) {
            list.push(r.var_i32()? as u16);
        }
        Ok(BlockSet::Blocks(list))
    }
}

#[derive(Debug, Clone)]
pub struct ToolRule {
    pub blocks: BlockSet,
    pub speed: Option<f32>,
    pub correct: Option<bool>,
}

/// The `tool` component.
#[derive(Debug, Clone)]
pub struct Tool {
    pub rules: Vec<ToolRule>,
    pub default_speed: f32,
    pub can_destroy_in_creative: bool,
}

impl Tool {
    pub fn mining_speed(&self, tags: &BlockTags, block: u16) -> f32 {
        self.rules.iter().find(|r| r.speed.is_some() && r.blocks.contains(tags, block)).and_then(|r| r.speed).unwrap_or(self.default_speed)
    }

    pub fn correct_for_drops(&self, tags: &BlockTags, block: u16) -> bool {
        self.rules.iter().find(|r| r.correct.is_some() && r.blocks.contains(tags, block)).and_then(|r| r.correct).unwrap_or(false)
    }
}

fn default(stack: &ItemStack, name: &str) -> Option<&'static Value> {
    if stack.is_removed(name) {
        return None;
    }
    lattice_data::game().items.get(stack.item as usize)?.components.get(name)
}

/// How long this item's swing takes, before effects change it.
pub fn swing_ticks(stack: &ItemStack) -> u32 {
    default(stack, "swing_animation").and_then(|v| v.get("duration")).and_then(Value::as_u64).map_or(super::swing::NORMAL_TICKS, |d| d as u32)
}

pub fn item_name(stack: &ItemStack) -> &'static str {
    lattice_data::game().items.get(stack.item as usize).map_or("air", |i| i.name.as_str())
}

/// The item's id in the registry by name.
pub fn item_id(name: &str) -> Option<i32> {
    lattice_data::game().items.iter().position(|i| i.name == name.trim_start_matches("minecraft:")).map(|i| i as i32)
}

pub fn max_stack(stack: &ItemStack) -> i32 {
    stack.var_int("max_stack_size").or_else(|| default(stack, "max_stack_size").and_then(Value::as_i64).map(|v| v as i32)).unwrap_or(1)
}

pub fn max_damage(stack: &ItemStack) -> i32 {
    stack.var_int("max_damage").or_else(|| default(stack, "max_damage").and_then(Value::as_i64).map(|v| v as i32)).unwrap_or(0)
}

pub fn damage(stack: &ItemStack) -> i32 {
    stack.var_int("damage").unwrap_or(0)
}

pub fn has_component(stack: &ItemStack, name: &str) -> bool {
    stack.raw(name).is_some() || default(stack, name).is_some()
}

/// How full the durability bar is, for damaged items.
pub fn durability(stack: &ItemStack) -> Option<f32> {
    let max = max_damage(stack);
    let d = damage(stack);
    (max > 0 && d > 0 && !has_component(stack, "unbreakable")).then(|| 1.0 - d as f32 / max as f32)
}

/// The name shown for a stack.
pub fn display_name(stack: &ItemStack, tr: &dyn Translator) -> String {
    if let Some(t) = stack.text("custom_name") {
        return t.to_plain(tr);
    }
    if let Some(t) = stack.text("item_name") {
        return t.to_plain(tr);
    }
    let key = default(stack, "item_name").and_then(|v| v.get("translate")).and_then(Value::as_str);
    match key {
        Some(k) => tr.translate(k).map(str::to_string).unwrap_or_else(|| k.to_string()),
        None => item_name(stack).replace('_', " "),
    }
}

pub fn rarity(stack: &ItemStack) -> &'static str {
    let index = stack.var_int("rarity");
    match index {
        Some(1) => "uncommon",
        Some(2) => "rare",
        Some(3) => "epic",
        Some(_) => "common",
        None => match default(stack, "rarity").and_then(Value::as_str) {
            Some("uncommon") => "uncommon",
            Some("rare") => "rare",
            Some("epic") => "epic",
            _ => "common",
        },
    }
}

pub fn tool(stack: &ItemStack, data: &GameData) -> Option<Tool> {
    if let Some(raw) = stack.raw("tool") {
        return read_raw(raw, |r| {
            let rules = r.array(|r| {
                let blocks = BlockSet::read(r)?;
                let speed = r.option(|r| r.f32())?;
                let correct = r.option(|r| r.bool())?;
                Ok(ToolRule { blocks, speed, correct })
            })?;
            let default_speed = r.f32()?;
            let _damage = r.var_i32()?;
            let can_destroy_in_creative = r.bool()?;
            Ok(Tool { rules, default_speed, can_destroy_in_creative })
        });
    }
    let v = default(stack, "tool")?;
    let rules = v.get("rules")?.as_array()?;
    let rules = rules
        .iter()
        .filter_map(|r| {
            Some(ToolRule {
                blocks: BlockSet::from_json(r.get("blocks")?, data)?,
                speed: r.get("speed").and_then(Value::as_f64).map(|s| s as f32),
                correct: r.get("correct_for_drops").and_then(Value::as_bool),
            })
        })
        .collect();
    Some(Tool {
        rules,
        default_speed: v.get("default_mining_speed").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        can_destroy_in_creative: v.get("can_destroy_blocks_in_creative").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Wears in this slot ("head", "chest", "legs", "feet", "body", ...).
pub fn equip_slot(stack: &ItemStack) -> Option<String> {
    const SLOTS: [&str; 8] = ["mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle"];
    if let Some(raw) = stack.raw("equippable") {
        return read_raw(raw, |r| r.var_i32()).and_then(|i| SLOTS.get(i as usize)).map(|s| s.to_string());
    }
    default(stack, "equippable")?.get("slot")?.as_str().map(str::to_string)
}

/// The patterns a banner pattern item offers, when it hasn't been changed from how it's made.
pub fn default_banner_patterns(stack: &ItemStack, session: &super::Session) -> Vec<i32> {
    let name = item_name(stack);
    let Some(kind) = name.strip_suffix("_banner_pattern") else { return Vec::new() };
    session.tag("banner_pattern", &format!("pattern_item/{kind}")).to_vec()
}

/// The colour a shield's been dyed, by applying a banner to it.
pub fn shield_base_color(stack: &ItemStack) -> Option<u32> {
    let dye = stack.raw("base_color").and_then(|raw| read_raw(raw, |r| r.var_i32()))?;
    crate::bodies::mobs::DYE_COLORS.get(dye as usize).copied()
}

/// A banner item's colour and pattern layers, by pattern name.
pub fn banner_look(stack: &ItemStack, names: &[String]) -> (u32, Vec<(String, u32)>) {
    let base = crate::bodies::mobs::dye_color(item_name(stack).trim_end_matches("_banner"));
    let layers = stack
        .raw("banner_patterns")
        .and_then(|raw| {
            read_raw(raw, |r| {
                r.array(|r| {
                    let holder = r.var_i32()?;
                    let name = if holder == 0 {
                        let asset = r.identifier()?;
                        r.string(32767)?;
                        asset
                    } else {
                        names.get(holder as usize - 1).cloned().unwrap_or_default()
                    };
                    let dye = r.var_i32()?;
                    let color = crate::bodies::mobs::DYE_COLORS.get(dye as usize).copied().unwrap_or(0xFFFFFF);
                    Ok((name.trim_start_matches("minecraft:").to_string(), color))
                })
            })
        })
        .unwrap_or_default();
    (base, layers)
}

/// Whether a stack has the shine of an enchanted thing.
pub fn has_glint(stack: &ItemStack) -> bool {
    if let Some(raw) = stack.raw("enchantment_glint_override") {
        return raw.first() == Some(&1);
    }
    if let Some(v) = default(stack, "enchantment_glint_override") {
        return v.as_bool().unwrap_or(false);
    }
    // A count of nothing is a single zero byte.
    stack.raw("enchantments").is_some_and(|r| r.first().is_some_and(|n| *n != 0)) || default(stack, "enchantments").is_some_and(|v| v.as_object().is_some_and(|m| !m.is_empty()))
}

/// The picture a worn item lays over the view, if any.
pub fn camera_overlay(stack: &ItemStack) -> Option<String> {
    default(stack, "equippable")?.get("camera_overlay")?.as_str().map(|s| s.trim_start_matches("minecraft:").to_string())
}

/// A signed book: title, author and pages.
pub fn written_book(stack: &ItemStack) -> Option<(String, String, Vec<Text>)> {
    let raw = stack.raw("written_book_content")?;
    read_raw(raw, |r| {
        let title = r.string(32767)?;
        r.option(|r| r.string(32767))?;
        let author = r.string(32767)?;
        r.var_i32()?;
        let pages = r.array(|r| {
            let t = r.text()?;
            r.option(|r| r.nbt())?;
            Ok(t)
        })?;
        Ok((title, author, pages))
    })
}

/// Text of an unsigned book's pages.
pub fn writable_pages(stack: &ItemStack) -> Vec<String> {
    let Some(raw) = stack.raw("writable_book_content") else { return Vec::new() };
    read_raw(raw, |r| {
        r.array(|r| {
            let t = r.string(32767)?;
            r.option(|r| r.string(32767))?;
            Ok(t)
        })
    })
    .unwrap_or_default()
}

/// The look a worn item takes: a name under `equipment/`.
pub fn equipment_asset(stack: &ItemStack) -> Option<String> {
    if let Some(raw) = stack.raw("equippable") {
        return read_raw(raw, |r| {
            r.var_i32()?; // slot
            // The equip sound: a registry id, or a whole definition when it's zero.
            let sound = r.var_i32()?;
            if sound == 0 {
                r.identifier()?;
                r.option(|r| r.f32())?;
            }
            r.option(|r| r.identifier())
        })
        .flatten()
        .map(|s| s.trim_start_matches("minecraft:").to_string());
    }
    default(stack, "equippable")?.get("asset_id")?.as_str().map(|s| s.trim_start_matches("minecraft:").to_string())
}

/// How long using the stack takes, in ticks, when it's consumed.
pub fn use_duration(stack: &ItemStack) -> f32 {
    if let Some(raw) = stack.raw("consumable") {
        if let Some(seconds) = read_raw(raw, |r| r.f32()) {
            return seconds * 20.0;
        }
    }
    default(stack, "consumable").map_or(72000.0, |c| c.get("consume_seconds").and_then(Value::as_f64).unwrap_or(1.6) as f32 * 20.0)
}

/// How the stack is used: "eat", "drink", "bow", "block", ...
pub fn use_animation(stack: &ItemStack) -> Option<String> {
    let name = item_name(stack);
    match name {
        "bow" => return Some("bow".into()),
        "crossbow" => return Some("crossbow".into()),
        "shield" => return Some("block".into()),
        "trident" => return Some("trident".into()),
        "spyglass" => return Some("spyglass".into()),
        "goat_horn" => return Some("toot_horn".into()),
        "brush" => return Some("brush".into()),
        _ => {}
    }
    const KINDS: [&str; 13] = ["none", "eat", "drink", "block", "bow", "trident", "crossbow", "spyglass", "toot_horn", "brush", "bundle", "spear", "none"];
    if let Some(raw) = stack.raw("consumable") {
        return read_raw(raw, |r| {
            r.f32()?;
            r.var_i32()
        })
        .and_then(|i| KINDS.get(i as usize))
        .map(|s| s.to_string());
    }
    default(stack, "consumable").map(|c| c.get("animation").and_then(Value::as_str).unwrap_or("eat").to_string())
}

/// Whether this stack lets the player glide when worn in the chest slot.
pub fn glides(stack: &ItemStack) -> bool {
    has_component(stack, "glider")
}

/// (speed multiplier while using, can sprint while using).
pub fn use_effects(stack: &ItemStack) -> (f32, bool) {
    if let Some(raw) = stack.raw("use_effects") {
        if let Some((sprint, _, speed)) = read_raw(raw, |r| Ok((r.bool()?, r.bool()?, r.f32()?))) {
            return (speed, sprint);
        }
    }
    let v = default(stack, "use_effects");
    let speed = v.and_then(|v| v.get("speed_multiplier")).and_then(Value::as_f64).unwrap_or(0.2) as f32;
    let sprint = v.and_then(|v| v.get("can_sprint")).and_then(Value::as_bool).unwrap_or(false);
    (speed, sprint)
}

/// The block this item places, if any.
pub fn places_block(stack: &ItemStack, data: &GameData) -> Option<u16> {
    let name = item_name(stack);
    data.blocks.iter().position(|b| b.item.as_deref() == Some(name)).map(|i| i as u16)
}

/// Which item a bundle in `slot` is set to hand over next, moved by the wheel. The game's own
/// order: the wheel going up goes back through the list, and it wraps round.
pub fn bundle_selection(session: &mut super::Session, slot: i16, inside: i32, up: bool) -> i32 {
    let now = shown_bundle_selection(session, slot);
    let next = match (now, up) {
        (i, _) if i < 0 => if up { inside - 1 } else { 0 },
        (i, true) => (i - 1).rem_euclid(inside),
        (i, false) => (i + 1).rem_euclid(inside),
    };
    session.bundle_pick = Some((slot, next));
    next
}

/// The item picked in the bundle at `slot`, or -1 for none.
pub fn shown_bundle_selection(session: &super::Session, slot: i16) -> i32 {
    session.bundle_pick.filter(|(s, _)| *s == slot).map_or(-1, |(_, i)| i)
}
