//! Item stacks on the wire.
//!
//! A stack's components have no length prefix when the server sends them, so every
//! one of the game's component types has to be walked exactly to find where the
//! next begins. The walkers below follow each type's stream codec in the 26.3 jar.
//! Their bytes are kept as they arrived, so stacks can be sent back unchanged and
//! interpreted only when something needs a value (see the accessors).

use crate::codec::{DResult, DecodeError, Reader, Writer};
use crate::nbt::Nbt;
use crate::text::Text;

/// Registry order of the data component types, so a type id says which walker to use.
pub const COMPONENT_NAMES: [&str; 122] = [
    "custom_data",
    "max_stack_size",
    "max_damage",
    "damage",
    "unbreakable",
    "use_effects",
    "custom_name",
    "minimum_attack_charge",
    "damage_type",
    "item_name",
    "item_model",
    "lore",
    "rarity",
    "enchantments",
    "can_place_on",
    "can_break",
    "attribute_modifiers",
    "custom_model_data",
    "tooltip_display",
    "repair_cost",
    "creative_slot_lock",
    "enchantment_glint_override",
    "intangible_projectile",
    "food",
    "consumable",
    "use_remainder",
    "use_cooldown",
    "damage_resistant",
    "tool",
    "weapon",
    "attack_range",
    "enchantable",
    "equippable",
    "repairable",
    "glider",
    "tooltip_style",
    "death_protection",
    "blocks_attacks",
    "piercing_weapon",
    "kinetic_weapon",
    "attack_animation",
    "interact_animation",
    "additional_trade_cost",
    "block_transformer",
    "villager_food",
    "stored_enchantments",
    "dye",
    "dyed_color",
    "map_id",
    "map_decorations",
    "map_post_processing",
    "charged_projectiles",
    "bundle_contents",
    "potion_contents",
    "potion_duration_scale",
    "suspicious_stew_effects",
    "writable_book_content",
    "written_book_content",
    "trim",
    "debug_stick_state",
    "entity_data",
    "bucket_entity_data",
    "block_entity_data",
    "instrument",
    "provides_trim_material",
    "ominous_bottle_amplifier",
    "jukebox_playable",
    "provides_banner_patterns",
    "recipes",
    "lodestone_tracker",
    "firework_explosion",
    "fireworks",
    "profile",
    "note_block_sound",
    "banner_patterns",
    "base_color",
    "pot_decorations",
    "container",
    "block_state",
    "bees",
    "sulfur_cube_content",
    "lock",
    "container_loot",
    "break_sound",
    "compostable",
    "cooking_fuel",
    "brewing_fuel",
    "mob_visibility",
    "villager/variant",
    "wolf/variant",
    "wolf/sound_variant",
    "wolf/collar",
    "fox/variant",
    "salmon/size",
    "parrot/variant",
    "tropical_fish/pattern",
    "tropical_fish/base_color",
    "tropical_fish/pattern_color",
    "mooshroom/variant",
    "rabbit/variant",
    "pig/variant",
    "pig/sound_variant",
    "cow/variant",
    "cow/sound_variant",
    "chicken/variant",
    "chicken/sound_variant",
    "zombie_nautilus/variant",
    "frog/variant",
    "horse/variant",
    "painting/variant",
    "llama/variant",
    "axolotl/variant",
    "cat/variant",
    "cat/sound_variant",
    "cat/collar",
    "sheep/color",
    "shulker/color",
    "provides_pottery_pattern",
    "sign_text_front",
    "sign_text_back",
    "waxed",
    "cushion/color",
];

/// Index of a component type in the registry.
pub fn component_id(name: &str) -> i32 {
    COMPONENT_NAMES.iter().position(|n| *n == name).map_or(-1, |i| i as i32)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub kind: i32,
    pub raw: Vec<u8>,
}

/// An item stack, never empty: empty slots are `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemStack {
    /// Index in the item registry.
    pub item: i32,
    pub count: i32,
    /// Components added or replaced compared with the item's defaults.
    pub added: Vec<Component>,
    /// Default components this stack doesn't have.
    pub removed: Vec<i32>,
}

impl ItemStack {
    pub fn new(item: i32, count: i32) -> Self {
        ItemStack { item, count, added: Vec::new(), removed: Vec::new() }
    }

    /// Same item and same components; such stacks can merge.
    pub fn same_item(&self, other: &ItemStack) -> bool {
        self.item == other.item && self.added == other.added && self.removed == other.removed
    }

    pub fn raw(&self, name: &str) -> Option<&[u8]> {
        let id = component_id(name);
        self.added.iter().find(|c| c.kind == id).map(|c| c.raw.as_slice())
    }

    pub fn is_removed(&self, name: &str) -> bool {
        self.removed.contains(&component_id(name))
    }

    pub fn set_raw(&mut self, name: &str, raw: Vec<u8>) {
        let kind = component_id(name);
        match self.added.iter_mut().find(|c| c.kind == kind) {
            Some(c) => c.raw = raw,
            None => self.added.push(Component { kind, raw }),
        }
    }

    /// `ItemStack.OPTIONAL_STREAM_CODEC`.
    pub fn read(r: &mut Reader) -> DResult<Option<ItemStack>> {
        let count = r.var_i32()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = r.var_i32()?;
        let (added, removed) = read_patch(r)?;
        Ok(Some(ItemStack { item, count, added, removed }))
    }

    /// `ItemStackTemplate.STREAM_CODEC`: the item comes first and the count is always there.
    pub fn read_template(r: &mut Reader) -> DResult<ItemStack> {
        let item = r.var_i32()?;
        let count = r.var_i32()?;
        let (added, removed) = read_patch(r)?;
        Ok(ItemStack { item, count, added, removed })
    }

    /// Reads back what `write_delimited` wrote.
    pub fn read_delimited(r: &mut Reader) -> DResult<Option<ItemStack>> {
        let count = r.var_i32()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = r.var_i32()?;
        let (n_added, n_removed) = (r.var_i32()?, r.var_i32()?);
        let mut added = Vec::new();
        for _ in 0..n_added {
            let kind = r.var_i32()?;
            let len = r.var_i32()?;
            let raw = r.take(len.max(0) as usize)?.to_vec();
            added.push(Component { kind, raw });
        }
        let mut removed = Vec::new();
        for _ in 0..n_removed {
            removed.push(r.var_i32()?);
        }
        Ok(Some(ItemStack { item, count, added, removed }))
    }

    /// `ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC`, for creative inventory packets: the
    /// same thing, but each component has a length so the server can check it safely.
    pub fn write_delimited(w: &mut Writer, stack: Option<&ItemStack>) {
        let Some(s) = stack else {
            w.var_i32(0);
            return;
        };
        w.var_i32(s.count).var_i32(s.item).var_i32(s.added.len() as i32).var_i32(s.removed.len() as i32);
        for c in &s.added {
            w.var_i32(c.kind).var_i32(c.raw.len() as i32).bytes(&c.raw);
        }
        for k in &s.removed {
            w.var_i32(*k);
        }
    }

    /// `HashedStack.STREAM_CODEC`, used when clicking in menus. A hash that doesn't match
    /// only makes the server send the slot again, so components are described by hash 0
    /// when they aren't plain; the server's copy stays authoritative either way.
    pub fn write_hashed(w: &mut Writer, stack: Option<&ItemStack>) {
        let Some(s) = stack else {
            w.bool(false);
            return;
        };
        w.bool(true).var_i32(s.item).var_i32(s.count).var_i32(s.added.len() as i32);
        for c in &s.added {
            w.var_i32(c.kind).i32(0);
        }
        w.var_i32(s.removed.len() as i32);
        for k in &s.removed {
            w.var_i32(*k);
        }
    }
}

type Patch = (Vec<Component>, Vec<i32>);

fn read_patch(r: &mut Reader) -> DResult<Patch> {
    let added_n = r.len()?;
    let removed_n = r.len()?;
    let mut added = Vec::with_capacity(added_n.min(64));
    for _ in 0..added_n {
        let kind = r.var_i32()?;
        let start = r.position();
        walk_component(kind, r)?;
        added.push(Component { kind, raw: r.since(start).to_vec() });
    }
    let mut removed = Vec::with_capacity(removed_n.min(64));
    for _ in 0..removed_n {
        removed.push(r.var_i32()?);
    }
    Ok((added, removed))
}

fn nbt(r: &mut Reader) -> DResult<()> {
    r.nbt().map(|_| ())
}

fn id(r: &mut Reader) -> DResult<()> {
    r.identifier().map(|_| ())
}

fn varint(r: &mut Reader) -> DResult<()> {
    r.var_i32().map(|_| ())
}

fn floats(r: &mut Reader, n: usize) -> DResult<()> {
    r.take(4 * n).map(|_| ())
}

fn optional(r: &mut Reader, f: impl FnOnce(&mut Reader) -> DResult<()>) -> DResult<()> {
    r.option(f).map(|_| ())
}

fn list(r: &mut Reader, mut f: impl FnMut(&mut Reader) -> DResult<()>) -> DResult<()> {
    let n = r.len()?;
    for _ in 0..n {
        f(r)?;
    }
    Ok(())
}

/// `ByteBufCodecs.holderSet`: a tag name, or a list of registry ids.
fn holder_set(r: &mut Reader) -> DResult<()> {
    let n = r.var_i32()?;
    if n == 0 {
        return id(r);
    }
    for _ in 0..(n - 1) {
        varint(r)?;
    }
    Ok(())
}

/// `ByteBufCodecs.holder`: a registry id plus one, or 0 and the value itself.
fn holder(r: &mut Reader, direct: impl FnOnce(&mut Reader) -> DResult<()>) -> DResult<()> {
    if r.var_i32()? == 0 { direct(r) } else { Ok(()) }
}

fn sound_event(r: &mut Reader) -> DResult<()> {
    holder(r, |r| {
        id(r)?;
        optional(r, |r| floats(r, 1))
    })
}

fn mob_effect_details(r: &mut Reader) -> DResult<()> {
    varint(r)?;
    varint(r)?;
    r.take(3)?;
    optional(r, mob_effect_details)
}

fn mob_effect_instance(r: &mut Reader) -> DResult<()> {
    varint(r)?;
    mob_effect_details(r)
}

fn consume_effect(r: &mut Reader) -> DResult<()> {
    match r.var_i32()? {
        0 => {
            list(r, mob_effect_instance)?;
            floats(r, 1)
        }
        1 => holder_set(r),
        2 => Ok(()),
        3 => {
            floats(r, 1)?;
            r.take(1).map(|_| ())
        }
        4 => sound_event(r),
        v => Err(DecodeError::Invalid { field: "consume effect type", value: v as i64 }),
    }
}

fn template(r: &mut Reader) -> DResult<()> {
    ItemStack::read_template(r).map(|_| ())
}

fn filterable_string(r: &mut Reader) -> DResult<()> {
    r.string(32767)?;
    optional(r, |r| r.string(32767).map(|_| ()))
}

fn filterable_text(r: &mut Reader) -> DResult<()> {
    nbt(r)?;
    optional(r, nbt)
}

fn typed_entity_data(r: &mut Reader) -> DResult<()> {
    varint(r)?;
    nbt(r)
}

fn game_profile_properties(r: &mut Reader) -> DResult<()> {
    list(r, |r| {
        r.string(64)?;
        r.string(32767)?;
        optional(r, |r| r.string(1024).map(|_| ()))
    })
}

fn resolvable_profile(r: &mut Reader) -> DResult<()> {
    if r.bool()? {
        r.uuid()?;
        r.string(16)?;
        game_profile_properties(r)?;
    } else {
        optional(r, |r| r.string(16).map(|_| ()))?;
        optional(r, |r| r.uuid().map(|_| ()))?;
        game_profile_properties(r)?;
    }
    // PlayerSkin.Patch: body, cape, elytra, model.
    optional(r, id)?;
    optional(r, id)?;
    optional(r, id)?;
    optional(r, |r| r.take(1).map(|_| ()))
}

fn block_predicate(r: &mut Reader) -> DResult<()> {
    optional(r, holder_set)?;
    optional(r, |r| {
        list(r, |r| {
            r.string(32767)?;
            if r.bool()? {
                r.string(32767).map(|_| ())
            } else {
                optional(r, |r| r.string(32767).map(|_| ()))?;
                optional(r, |r| r.string(32767).map(|_| ()))
            }
        })
    })?;
    optional(r, nbt)?;
    // DataComponentMatchers: exact components, then partial predicates.
    list(r, |r| {
        let kind = r.var_i32()?;
        walk_component(kind, r)
    })?;
    let partial = r.len()?;
    if partial > 0 {
        // Each predicate type has its own format and none has a length. Vanilla worlds
        // don't put these on items; a failure here costs this one stack, not the session.
        return Err(DecodeError::Invalid { field: "item component predicate", value: partial as i64 });
    }
    Ok(())
}

fn firework_explosion(r: &mut Reader) -> DResult<()> {
    varint(r)?;
    list(r, |r| floats(r, 1))?;
    list(r, |r| floats(r, 1))?;
    r.take(2).map(|_| ())
}

fn resolvable_int(r: &mut Reader) -> DResult<()> {
    if r.bool()? { floats(r, 1) } else { id(r) }
}

fn sign_lines(r: &mut Reader) -> DResult<()> {
    for _ in 0..4 {
        nbt(r)?;
    }
    Ok(())
}

/// Walks one component's value.
pub fn walk_component(kind: i32, r: &mut Reader) -> DResult<()> {
    let name = *COMPONENT_NAMES.get(kind as usize).ok_or(DecodeError::Invalid { field: "data component type", value: kind as i64 })?;
    match name {
        // No stream codec of their own: sent as NBT.
        "custom_data" | "intangible_projectile" | "map_decorations" | "debug_stick_state" | "recipes" | "lock" | "container_loot" => nbt(r),
        "bucket_entity_data" => nbt(r),
        // Numbers and simple enums.
        "max_stack_size" | "max_damage" | "damage" | "repair_cost" | "additional_trade_cost" | "rarity" | "dye" | "map_id"
        | "map_post_processing" | "ominous_bottle_amplifier" | "base_color" | "enchantable" | "villager_food" | "block_transformer"
        | "villager/variant" | "wolf/variant" | "wolf/sound_variant" | "wolf/collar" | "fox/variant" | "salmon/size"
        | "parrot/variant" | "tropical_fish/pattern" | "tropical_fish/base_color" | "tropical_fish/pattern_color"
        | "mooshroom/variant" | "rabbit/variant" | "pig/variant" | "pig/sound_variant" | "cow/variant" | "cow/sound_variant"
        | "chicken/variant" | "chicken/sound_variant" | "zombie_nautilus/variant" | "frog/variant" | "horse/variant"
        | "llama/variant" | "axolotl/variant" | "cat/variant" | "cat/sound_variant" | "cat/collar" | "sheep/color"
        | "shulker/color" | "provides_pottery_pattern" | "cushion/color" => varint(r),
        "unbreakable" | "creative_slot_lock" | "glider" | "waxed" => Ok(()),
        "minimum_attack_charge" | "potion_duration_scale" | "dyed_color" => floats(r, 1),
        "enchantment_glint_override" => r.take(1).map(|_| ()),
        "item_model" | "tooltip_style" | "note_block_sound" => id(r),
        "custom_name" | "item_name" => nbt(r),
        "use_effects" => {
            r.take(2)?;
            floats(r, 1)
        }
        "damage_type" => holder(r, |r| {
            id(r)?;
            varint(r)?;
            floats(r, 1)?;
            varint(r)?;
            varint(r)
        }),
        "lore" => list(r, nbt),
        "enchantments" | "stored_enchantments" => list(r, |r| {
            varint(r)?;
            varint(r)
        }),
        "can_place_on" | "can_break" => list(r, block_predicate),
        "attribute_modifiers" => list(r, |r| {
            varint(r)?;
            id(r)?;
            r.take(8)?;
            varint(r)?;
            varint(r)?;
            match r.var_i32()? {
                2 => nbt(r),
                _ => Ok(()),
            }
        }),
        "custom_model_data" => {
            list(r, |r| floats(r, 1))?;
            list(r, |r| r.take(1).map(|_| ()))?;
            list(r, |r| r.string(32767).map(|_| ()))?;
            list(r, |r| floats(r, 1))
        }
        "tooltip_display" => {
            r.take(1)?;
            list(r, varint)
        }
        "food" => {
            varint(r)?;
            floats(r, 1)?;
            r.take(1).map(|_| ())
        }
        "consumable" => {
            floats(r, 1)?;
            varint(r)?;
            sound_event(r)?;
            r.take(1)?;
            list(r, consume_effect)
        }
        "use_remainder" | "sulfur_cube_content" => template(r),
        "use_cooldown" => {
            floats(r, 1)?;
            optional(r, id)
        }
        "damage_resistant" | "repairable" | "provides_banner_patterns" => holder_set(r),
        "tool" => {
            list(r, |r| {
                holder_set(r)?;
                optional(r, |r| floats(r, 1))?;
                optional(r, |r| r.take(1).map(|_| ()))
            })?;
            floats(r, 1)?;
            varint(r)?;
            r.take(1).map(|_| ())
        }
        "weapon" => {
            varint(r)?;
            floats(r, 1)
        }
        "attack_range" => floats(r, 6),
        "equippable" => {
            varint(r)?;
            sound_event(r)?;
            optional(r, id)?;
            optional(r, id)?;
            optional(r, holder_set)?;
            r.take(5)?;
            sound_event(r)
        }
        "death_protection" => list(r, consume_effect),
        "blocks_attacks" => {
            floats(r, 2)?;
            list(r, |r| {
                floats(r, 1)?;
                optional(r, holder_set)?;
                floats(r, 2)
            })?;
            floats(r, 3)?;
            optional(r, holder_set)?;
            optional(r, sound_event)?;
            optional(r, sound_event)
        }
        "piercing_weapon" => {
            r.take(2)?;
            optional(r, sound_event)?;
            optional(r, sound_event)
        }
        "kinetic_weapon" => {
            varint(r)?;
            varint(r)?;
            for _ in 0..3 {
                optional(r, |r| {
                    varint(r)?;
                    floats(r, 2)
                })?;
            }
            floats(r, 2)?;
            optional(r, sound_event)?;
            optional(r, sound_event)
        }
        "attack_animation" | "interact_animation" => {
            varint(r)?;
            varint(r)
        }
        "charged_projectiles" | "bundle_contents" => list(r, template),
        "potion_contents" => {
            optional(r, varint)?;
            optional(r, |r| floats(r, 1))?;
            list(r, mob_effect_instance)?;
            optional(r, |r| r.string(32767).map(|_| ()))
        }
        "suspicious_stew_effects" => list(r, |r| {
            varint(r)?;
            varint(r)
        }),
        "writable_book_content" => list(r, filterable_string),
        "written_book_content" => {
            filterable_string(r)?;
            r.string(32767)?;
            varint(r)?;
            list(r, filterable_text)?;
            r.take(1).map(|_| ())
        }
        "trim" => {
            trim_material(r)?;
            holder(r, |r| {
                id(r)?;
                nbt(r)?;
                r.take(1).map(|_| ())
            })
        }
        "provides_trim_material" => trim_material(r),
        "entity_data" | "block_entity_data" => typed_entity_data(r),
        "instrument" => holder(r, |r| {
            sound_event(r)?;
            floats(r, 2)?;
            varint(r)?;
            nbt(r)
        }),
        "jukebox_playable" => holder(r, |r| {
            sound_event(r)?;
            nbt(r)?;
            floats(r, 1)?;
            varint(r)
        }),
        "lodestone_tracker" => {
            optional(r, |r| {
                id(r)?;
                r.take(8).map(|_| ())
            })?;
            r.take(1).map(|_| ())
        }
        "firework_explosion" => firework_explosion(r),
        "fireworks" => {
            varint(r)?;
            list(r, firework_explosion)
        }
        "profile" => resolvable_profile(r),
        "banner_patterns" => list(r, |r| {
            holder(r, |r| {
                id(r)?;
                r.string(32767).map(|_| ())
            })?;
            varint(r)
        }),
        "pot_decorations" => {
            for _ in 0..4 {
                optional(r, template)?;
            }
            Ok(())
        }
        "container" => list(r, |r| optional(r, template)),
        "block_state" => list(r, |r| {
            r.string(32767)?;
            r.string(32767).map(|_| ())
        }),
        "bees" => list(r, |r| {
            typed_entity_data(r)?;
            varint(r)?;
            varint(r)
        }),
        "break_sound" => sound_event(r),
        "compostable" => resolvable_int(r),
        "cooking_fuel" | "brewing_fuel" => {
            resolvable_int(r)?;
            resolvable_int(r)
        }
        "mob_visibility" => {
            holder_set(r)?;
            floats(r, 1)
        }
        "painting/variant" => holder(r, |r| {
            varint(r)?;
            varint(r)?;
            id(r)?;
            optional(r, nbt)?;
            optional(r, nbt)
        }),
        "sign_text_front" | "sign_text_back" => {
            sign_lines(r)?;
            optional(r, sign_lines)?;
            varint(r)?;
            r.take(1).map(|_| ())
        }
        other => Err(DecodeError::Invalid { field: "unhandled component", value: component_id(other) as i64 }),
    }
}

fn trim_material(r: &mut Reader) -> DResult<()> {
    holder(r, |r| {
        id(r)?;
        nbt(r)
    })
}

// ---------------------------------------------------------------- values

/// Reads a component's value out of its raw bytes.
pub fn read_raw<T>(raw: &[u8], f: impl FnOnce(&mut Reader) -> DResult<T>) -> Option<T> {
    f(&mut Reader::new(raw)).ok()
}

impl ItemStack {
    pub fn var_int(&self, name: &str) -> Option<i32> {
        read_raw(self.raw(name)?, |r| r.var_i32())
    }

    pub fn text(&self, name: &str) -> Option<Text> {
        read_raw(self.raw(name)?, |r| r.text())
    }

    pub fn lore(&self) -> Vec<Text> {
        self.raw("lore").and_then(|raw| read_raw(raw, |r| r.array(|r| r.text()))).unwrap_or_default()
    }

    /// (registry id, level) pairs.
    pub fn enchantments(&self, name: &str) -> Vec<(i32, i32)> {
        self.raw(name).and_then(|raw| read_raw(raw, |r| r.array(|r| Ok((r.var_i32()?, r.var_i32()?))))).unwrap_or_default()
    }

    pub fn dyed_color(&self) -> Option<i32> {
        read_raw(self.raw("dyed_color")?, |r| r.i32())
    }

    /// `item_model`, when the stack overrides it.
    pub fn item_model(&self) -> Option<String> {
        read_raw(self.raw("item_model")?, |r| r.identifier())
    }

    /// Items inside a bundle.
    pub fn bundle_contents(&self) -> Vec<ItemStack> {
        self.raw("bundle_contents").and_then(|raw| read_raw(raw, |r| r.array(ItemStack::read_template))).unwrap_or_default()
    }

    /// Slots of a shulker box or similar, empty slots included.
    pub fn container_contents(&self) -> Vec<Option<ItemStack>> {
        self.raw("container").and_then(|raw| read_raw(raw, |r| r.array(|r| r.option(ItemStack::read_template)))).unwrap_or_default()
    }

    /// Potion colour source: (potion registry id, custom colour).
    pub fn potion_contents(&self) -> Option<PotionContents> {
        read_raw(self.raw("potion_contents")?, |r| {
            let potion = r.option(|r| r.var_i32())?;
            let color = r.option(|r| r.i32())?;
            let effects = r.array(|r| {
                let effect = r.var_i32()?;
                let d = read_effect_details(r)?;
                Ok(EffectInstance { effect, amplifier: d.0, duration: d.1 })
            })?;
            let name = r.option(|r| r.string(32767))?;
            Ok(PotionContents { potion, color, effects, name })
        })
    }
}

fn read_effect_details(r: &mut Reader) -> DResult<(i32, i32)> {
    let amplifier = r.var_i32()?;
    let duration = r.var_i32()?;
    r.take(3)?;
    optional(r, mob_effect_details)?;
    Ok((amplifier, duration))
}

#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub effect: i32,
    pub amplifier: i32,
    pub duration: i32,
}

#[derive(Debug, Clone)]
pub struct PotionContents {
    pub potion: Option<i32>,
    pub color: Option<i32>,
    pub effects: Vec<EffectInstance>,
    pub name: Option<String>,
}

// ---------------------------------------------------------------- particles

/// Walks the options of a particle type.
fn walk_particle_options(name: &str, r: &mut Reader) -> DResult<()> {
    match name {
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" | "shriek" => varint(r),
        "dust" => floats(r, 2),
        "dust_color_transition" => floats(r, 3),
        "effect" | "instant_effect" => floats(r, 2),
        "entity_effect" | "tinted_leaves" | "flash" => floats(r, 1),
        "dragon_breath" | "sculk_charge" => floats(r, 1),
        "item" => template(r),
        "vibration" => {
            match r.var_i32()? {
                0 => {
                    r.take(8)?;
                }
                _ => {
                    varint(r)?;
                    floats(r, 1)?;
                }
            }
            varint(r)
        }
        "trail" => {
            r.take(24)?;
            floats(r, 1)?;
            varint(r)
        }
        // Geyser particles aren't drawn from the network with any options in vanilla play.
        _ => Ok(()),
    }
}

/// A particle reference with its options kept raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Particle {
    pub kind: i32,
    pub raw: Vec<u8>,
}

/// Reads one particle. `names` is the particle type registry.
pub fn read_particle(r: &mut Reader, names: &[String]) -> DResult<Particle> {
    let kind = r.var_i32()?;
    let start = r.position();
    let name = names.get(kind as usize).map(String::as_str).unwrap_or("");
    walk_particle_options(name, r)?;
    Ok(Particle { kind, raw: r.since(start).to_vec() })
}

// ---------------------------------------------------------------- entity data

/// A value of an entity's synced data.
#[derive(Debug, Clone, PartialEq)]
pub enum DataValue {
    Byte(u8),
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    Text(Nbt),
    OptText(Option<Nbt>),
    Item(Option<ItemStack>),
    Bool(bool),
    Rotations([f32; 3]),
    Pos(crate::BlockPos),
    OptPos(Option<crate::BlockPos>),
    Direction(i32),
    OptEntity(Option<uuid::Uuid>),
    BlockState(i32),
    OptBlockState(Option<i32>),
    Particle(Particle),
    Particles(Vec<Particle>),
    VillagerData { kind: i32, profession: i32, level: i32 },
    OptUnsignedInt(Option<i32>),
    Pose(i32),
    /// Any of the registry-backed variant types.
    Variant(i32),
    OptGlobalPos(Option<(String, crate::BlockPos)>),
    /// An enum sent as an index.
    Enum(i32),
    Vector3([f32; 3]),
    Quaternion([f32; 4]),
    Profile(Vec<u8>),
}

/// Reads one value of the serializer named `serializer`.
pub fn read_data_value(r: &mut Reader, serializer: &str) -> DResult<DataValue> {
    let particle_names = lattice_data::game().registry("particle_type");
    Ok(match serializer {
        "BYTE" => DataValue::Byte(r.u8()?),
        "INT" => DataValue::Int(r.var_i32()?),
        "LONG" => DataValue::Long(r.var_i64()?),
        "FLOAT" => DataValue::Float(r.f32()?),
        "STRING" => DataValue::String(r.string(32767)?),
        "COMPONENT" => DataValue::Text(r.nbt()?),
        "OPTIONAL_COMPONENT" => DataValue::OptText(r.option(|r| r.nbt())?),
        "ITEM_STACK" => DataValue::Item(ItemStack::read(r)?),
        "BOOLEAN" => DataValue::Bool(r.bool()?),
        "ROTATIONS" => DataValue::Rotations([r.f32()?, r.f32()?, r.f32()?]),
        "BLOCK_POS" => DataValue::Pos(r.block_pos()?),
        "OPTIONAL_BLOCK_POS" => DataValue::OptPos(r.option(|r| r.block_pos())?),
        "DIRECTION" => DataValue::Direction(r.var_i32()?),
        "OPTIONAL_LIVING_ENTITY_REFERENCE" => DataValue::OptEntity(r.option(|r| r.uuid())?),
        "BLOCK_STATE" => DataValue::BlockState(r.var_i32()?),
        "OPTIONAL_BLOCK_STATE" => {
            let v = r.var_i32()?;
            DataValue::OptBlockState((v != 0).then_some(v))
        }
        "PARTICLE" => DataValue::Particle(read_particle(r, particle_names)?),
        "PARTICLES" => DataValue::Particles(r.array(|r| read_particle(r, particle_names))?),
        "VILLAGER_DATA" => DataValue::VillagerData { kind: r.var_i32()?, profession: r.var_i32()?, level: r.var_i32()? },
        "OPTIONAL_UNSIGNED_INT" => {
            let v = r.var_i32()?;
            DataValue::OptUnsignedInt((v != 0).then_some(v - 1))
        }
        "POSE" => DataValue::Pose(r.var_i32()?),
        "CAT_VARIANT" | "CAT_SOUND_VARIANT" | "COW_VARIANT" | "COW_SOUND_VARIANT" | "WOLF_VARIANT" | "WOLF_SOUND_VARIANT" | "FROG_VARIANT"
        | "PIG_VARIANT" | "PIG_SOUND_VARIANT" | "CHICKEN_VARIANT" | "CHICKEN_SOUND_VARIANT" | "ZOMBIE_NAUTILUS_VARIANT" => {
            DataValue::Variant(r.var_i32()?)
        }
        "PAINTING_VARIANT" => {
            let v = r.var_i32()?;
            if v == 0 {
                varint(r)?;
                varint(r)?;
                id(r)?;
                optional(r, nbt)?;
                optional(r, nbt)?;
                DataValue::Variant(-1)
            } else {
                DataValue::Variant(v - 1)
            }
        }
        "OPTIONAL_GLOBAL_POS" => DataValue::OptGlobalPos(r.option(|r| Ok((r.identifier()?, r.block_pos()?)))?),
        "SNIFFER_STATE" | "ARMADILLO_STATE" | "COPPER_GOLEM_STATE" | "WEATHERING_COPPER_STATE" | "HUMANOID_ARM" | "DYE_COLOR" => DataValue::Enum(r.var_i32()?),
        "VECTOR3" => DataValue::Vector3([r.f32()?, r.f32()?, r.f32()?]),
        "QUATERNION" => DataValue::Quaternion([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        "RESOLVABLE_PROFILE" => {
            let start = r.position();
            resolvable_profile(r)?;
            DataValue::Profile(r.since(start).to_vec())
        }
        _ => return Err(DecodeError::Invalid { field: "entity data serializer", value: -1 }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_stack_round_trips() {
        let mut w = Writer::new();
        w.var_i32(5).var_i32(42).var_i32(0).var_i32(0);
        let stack = ItemStack::read(&mut Reader::new(&w.buf)).unwrap().unwrap();
        assert_eq!((stack.item, stack.count), (42, 5));
        let mut out = Writer::new();
        ItemStack::write_delimited(&mut out, Some(&stack));
        assert_eq!(out.buf, w.buf);
    }

    #[test]
    fn empty_stack() {
        assert!(ItemStack::read(&mut Reader::new(&[0])).unwrap().is_none());
    }

    #[test]
    fn components_keep_their_bytes() {
        // A damage component (id 3) of 7, and removal of the lore component.
        let mut w = Writer::new();
        w.var_i32(1).var_i32(10).var_i32(1).var_i32(1).var_i32(3).var_i32(7).var_i32(component_id("lore"));
        let mut r = Reader::new(&w.buf);
        let stack = ItemStack::read(&mut r).unwrap().unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!(stack.var_int("damage"), Some(7));
        assert!(stack.is_removed("lore"));
    }

    #[test]
    fn component_list_matches_the_game_registry() {
        let reg = lattice_data::game().registry("data_component_type");
        assert_eq!(reg.len(), COMPONENT_NAMES.len());
        for (a, b) in reg.iter().zip(COMPONENT_NAMES) {
            assert_eq!(a, b);
        }
    }
}
