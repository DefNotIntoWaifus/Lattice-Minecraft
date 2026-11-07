//! How recipes are described to the client: what a slot shows, and the book's entries.

use crate::ItemStack;
use crate::codec::{DResult, Reader};

/// What a slot in a recipe shows.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotDisplay {
    Empty,
    AnyFuel,
    Item(i32),
    Stack(ItemStack),
    /// A list of items, or a tag's name.
    Tag(ItemSet),
    /// Wraps another, with extra details that only matter for showing it.
    Inner(Box<SlotDisplay>),
    Composite(Vec<SlotDisplay>),
}

/// A set of items: spelled out, or by tag.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemSet {
    pub tag: Option<String>,
    pub items: Vec<i32>,
}

impl ItemSet {
    pub fn read(r: &mut Reader) -> DResult<ItemSet> {
        let n = r.var_i32()?;
        if n == 0 {
            return Ok(ItemSet { tag: Some(r.identifier()?.trim_start_matches("minecraft:").to_string()), items: Vec::new() });
        }
        let mut items = Vec::with_capacity((n - 1).clamp(0, 256) as usize);
        for _ in 0..n - 1 {
            items.push(r.var_i32()?);
        }
        Ok(ItemSet { tag: None, items })
    }
}

impl SlotDisplay {
    pub fn read(r: &mut Reader) -> DResult<SlotDisplay> {
        Ok(match r.var_i32()? {
            0 => SlotDisplay::Empty,
            1 => SlotDisplay::AnyFuel,
            // with_any_potion: another display.
            2 => SlotDisplay::Inner(Box::new(SlotDisplay::read(r)?)),
            // only_with_component: a display and a component type.
            3 => {
                let inner = SlotDisplay::read(r)?;
                r.var_i32()?;
                SlotDisplay::Inner(Box::new(inner))
            }
            4 => SlotDisplay::Item(r.var_i32()?),
            5 => SlotDisplay::Stack(ItemStack::read_template(r)?),
            6 => SlotDisplay::Tag(ItemSet::read(r)?),
            // dyed: a dye and a target.
            7 => {
                let _ = SlotDisplay::read(r)?;
                SlotDisplay::Inner(Box::new(SlotDisplay::read(r)?))
            }
            // smithing_trim: base, material, and a pattern (by id, or written out).
            8 => {
                let base = SlotDisplay::read(r)?;
                let _ = SlotDisplay::read(r)?;
                let id = r.var_i32()?;
                if id == 0 {
                    // Asset id, description, decal flag.
                    r.identifier()?;
                    r.nbt()?;
                    r.bool()?;
                }
                SlotDisplay::Inner(Box::new(base))
            }
            // with_remainder: the input and what's left.
            9 => {
                let input = SlotDisplay::read(r)?;
                let _ = SlotDisplay::read(r)?;
                SlotDisplay::Inner(Box::new(input))
            }
            10 => {
                let mut list = Vec::new();
                for _ in 0..r.var_i32()? {
                    list.push(SlotDisplay::read(r)?);
                }
                SlotDisplay::Composite(list)
            }
            other => return Err(crate::codec::DecodeError::Invalid { field: "slot display", value: other as i64 }),
        })
    }

    /// The stack to show for this slot. `tag` looks up what's in an item tag.
    pub fn stack(&self, tag: &dyn Fn(&str) -> Option<i32>) -> Option<ItemStack> {
        match self {
            SlotDisplay::Item(i) => Some(ItemStack::new(*i, 1)),
            SlotDisplay::Stack(s) => Some(s.clone()),
            SlotDisplay::Tag(set) => match &set.tag {
                Some(t) => tag(t).map(|i| ItemStack::new(i, 1)),
                None => set.items.first().map(|i| ItemStack::new(*i, 1)),
            },
            SlotDisplay::Inner(d) => d.stack(tag),
            SlotDisplay::Composite(l) => l.iter().find_map(|d| d.stack(tag)),
            _ => None,
        }
    }
}

/// How a recipe shows itself: what goes where and what it makes.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeDisplay {
    /// 0 shapeless, 1 shaped, 2 furnace, 3 stonecutter, 4 smithing.
    pub kind: i32,
    /// What's needed, for shown ingredients.
    pub slots: Vec<SlotDisplay>,
    pub result: SlotDisplay,
    /// Crafting grid size when shaped.
    pub width: i32,
    pub height: i32,
}

impl RecipeDisplay {
    pub fn read(r: &mut Reader) -> DResult<RecipeDisplay> {
        let kind = r.var_i32()?;
        let (mut slots, mut width, mut height) = (Vec::new(), 0, 0);
        let result;
        match kind {
            0 => {
                for _ in 0..r.var_i32()? {
                    slots.push(SlotDisplay::read(r)?);
                }
                result = SlotDisplay::read(r)?;
                SlotDisplay::read(r)?; // crafting station
            }
            1 => {
                width = r.var_i32()?;
                height = r.var_i32()?;
                for _ in 0..r.var_i32()? {
                    slots.push(SlotDisplay::read(r)?);
                }
                result = SlotDisplay::read(r)?;
                SlotDisplay::read(r)?;
            }
            2 => {
                slots.push(SlotDisplay::read(r)?);
                slots.push(SlotDisplay::read(r)?); // fuel
                result = SlotDisplay::read(r)?;
                SlotDisplay::read(r)?;
                r.var_i32()?; // duration
                r.f32()?; // experience
            }
            3 => {
                slots.push(SlotDisplay::read(r)?);
                result = SlotDisplay::read(r)?;
                SlotDisplay::read(r)?;
            }
            4 => {
                slots.push(SlotDisplay::read(r)?);
                slots.push(SlotDisplay::read(r)?);
                slots.push(SlotDisplay::read(r)?);
                result = SlotDisplay::read(r)?;
                SlotDisplay::read(r)?;
            }
            other => return Err(crate::codec::DecodeError::Invalid { field: "recipe display", value: other as i64 }),
        }
        Ok(RecipeDisplay { kind, slots, result, width, height })
    }
}

/// A recipe in the player's recipe book.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeEntry {
    pub id: i32,
    pub display: RecipeDisplay,
    pub group: Option<i32>,
    pub category: i32,
    /// What must be at hand: each ingredient's possible items.
    pub requirements: Option<Vec<ItemSet>>,
}

impl RecipeEntry {
    pub fn read(r: &mut Reader) -> DResult<RecipeEntry> {
        let id = r.var_i32()?;
        let display = RecipeDisplay::read(r)?;
        // Group: an optional number, stored one higher.
        let group = match r.var_i32()? {
            0 => None,
            g => Some(g - 1),
        };
        let category = r.var_i32()?;
        let requirements = r.option(|r| {
            let mut list = Vec::new();
            for _ in 0..r.var_i32()? {
                list.push(ItemSet::read(r)?);
            }
            Ok(list)
        })?;
        Ok(RecipeEntry { id, display, group, category, requirements })
    }
}

/// A stonecutter choice: what goes in and what comes out.
#[derive(Debug, Clone, PartialEq)]
pub struct CutterChoice {
    pub input: ItemSet,
    pub output: SlotDisplay,
}
