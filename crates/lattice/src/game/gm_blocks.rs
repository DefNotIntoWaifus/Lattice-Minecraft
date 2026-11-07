//! Blocks only game masters can edit, and the screens the client opens for them itself: the
//! server never says "open this", it only sends what's in the block.

use lattice_proto::packets::{Encoded, play};
use lattice_proto::{BlockPos, Nbt, Text};

/// Blocks this module has a screen for.
pub fn has_screen(name: &str) -> bool {
    matches!(name, "command_block" | "chain_command_block" | "repeating_command_block" | "structure_block" | "jigsaw" | "test_block" | "test_instance_block")
}

#[derive(Debug, Clone)]
pub struct CommandScreen {
    /// `None` for a command block in a minecart, which has no modes.
    pub pos: Option<BlockPos>,
    pub minecart: Option<i32>,
    pub command: String,
    pub last_output: String,
    pub track_output: bool,
    /// 0 chain, 1 repeat, 2 impulse.
    pub mode: usize,
    pub conditional: bool,
    pub needs_redstone: bool,
    /// What the server was last asked to complete, and its answer.
    pub asked: String,
    pub question: i32,
    pub suggestions: Vec<String>,
    pub suggest_start: usize,
    pub suggest_length: usize,
    pub suggest_pick: usize,
}

#[derive(Debug, Clone)]
pub struct StructureScreen {
    pub pos: BlockPos,
    pub mode: usize,
    pub name: String,
    pub author: String,
    pub offset: [String; 3],
    pub size: [String; 3],
    pub mirror: usize,
    pub rotation: usize,
    pub data: String,
    pub integrity: String,
    pub seed: String,
    pub include_entities: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub strict: bool,
}

#[derive(Debug, Clone)]
pub struct JigsawScreen {
    pub pos: BlockPos,
    pub pool: String,
    pub name: String,
    pub target: String,
    pub final_state: String,
    pub rollable: bool,
    /// Only a block facing up or down can choose how the next piece turns.
    pub vertical: bool,
    pub selection_priority: String,
    pub placement_priority: String,
    pub levels: i32,
    pub keep_jigsaws: bool,
}

#[derive(Debug, Clone)]
pub struct TestScreen {
    pub pos: BlockPos,
    pub mode: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct TestInstanceScreen {
    pub pos: BlockPos,
    pub id: String,
    pub size: [String; 3],
    pub rotation: usize,
    pub include_entities: bool,
    pub status: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum BlockScreen {
    Command(CommandScreen),
    Structure(StructureScreen),
    Jigsaw(JigsawScreen),
    Test(TestScreen),
    TestInstance(TestInstanceScreen),
}

fn text_of(data: Option<&Nbt>, key: &str) -> String {
    data.and_then(|d| d.get(key)).and_then(Nbt::as_str).unwrap_or_default().to_string()
}

fn bool_of(data: Option<&Nbt>, key: &str, default: bool) -> bool {
    data.and_then(|d| d.get(key)).and_then(Nbt::as_bool).unwrap_or(default)
}

fn int_of(data: Option<&Nbt>, key: &str, default: i64) -> i64 {
    data.and_then(|d| d.get(key)).and_then(Nbt::as_i64).unwrap_or(default)
}

const ROTATIONS: [&str; 4] = ["NONE", "CLOCKWISE_90", "CLOCKWISE_180", "COUNTERCLOCKWISE_90"];
const MIRRORS: [&str; 3] = ["NONE", "LEFT_RIGHT", "FRONT_BACK"];
const STRUCTURE_MODES: [&str; 4] = ["SAVE", "LOAD", "CORNER", "DATA"];
const TEST_MODES: [&str; 4] = ["start", "log", "fail", "accept"];

fn index_of(list: &[&str], value: &str) -> usize {
    list.iter().position(|v| v.eq_ignore_ascii_case(value)).unwrap_or(0)
}

/// What's written in the block's last output, which can be a text component.
fn output_of(data: Option<&Nbt>, tr: &dyn lattice_proto::Translator) -> String {
    data.and_then(|d| d.get("LastOutput")).map_or_else(String::new, |t| Text::from_nbt(t).to_plain(tr))
}

impl BlockScreen {
    /// The screen for a block, filled in with what's known about it so far.
    pub fn open(pos: BlockPos, state: u32, data: Option<&Nbt>, tr: &dyn lattice_proto::Translator) -> Option<BlockScreen> {
        let game = lattice_data::game();
        let name = game.block(state).name.as_str();
        let props = game.properties(state);
        let prop = |key: &str| props.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        let mut screen = match name {
            "command_block" | "chain_command_block" | "repeating_command_block" => BlockScreen::Command(CommandScreen {
                pos: Some(pos),
                minecart: None,
                command: String::new(),
                last_output: String::new(),
                track_output: true,
                mode: match name {
                    "chain_command_block" => 0,
                    "repeating_command_block" => 1,
                    _ => 2,
                },
                conditional: prop("conditional") == Some("true"),
                needs_redstone: true,
                asked: String::new(),
                question: 0,
                suggestions: Vec::new(),
                suggest_start: 0,
                suggest_length: 0,
                suggest_pick: 0,
            }),
            "structure_block" => BlockScreen::Structure(StructureScreen {
                pos,
                mode: prop("mode").map_or(1, |m| index_of(&STRUCTURE_MODES, m)),
                name: String::new(),
                author: String::new(),
                offset: ["0".into(), "1".into(), "0".into()],
                size: ["0".into(), "0".into(), "0".into()],
                mirror: 0,
                rotation: 0,
                data: String::new(),
                integrity: "1.0".into(),
                seed: "0".into(),
                include_entities: true,
                show_air: false,
                show_bounding_box: true,
                strict: false,
            }),
            "jigsaw" => BlockScreen::Jigsaw(JigsawScreen {
                pos,
                pool: "minecraft:empty".into(),
                name: "minecraft:empty".into(),
                target: "minecraft:empty".into(),
                final_state: "minecraft:air".into(),
                rollable: true,
                vertical: prop("orientation").is_some_and(|o| o.starts_with("up_") || o.starts_with("down_")),
                selection_priority: "0".into(),
                placement_priority: "0".into(),
                levels: 0,
                keep_jigsaws: false,
            }),
            "test_block" => BlockScreen::Test(TestScreen { pos, mode: prop("mode").map_or(0, |m| index_of(&TEST_MODES, m)), message: String::new() }),
            "test_instance_block" => BlockScreen::TestInstance(TestInstanceScreen { pos, id: String::new(), size: ["5".into(), "5".into(), "5".into()], rotation: 0, include_entities: false, status: Vec::new() }),
            _ => return None,
        };
        if let Some(data) = data {
            screen.take_data(data, tr);
        }
        Some(screen)
    }

    /// A command block on a minecart: what it runs is on the entity, not a block.
    pub fn minecart(entity: i32, command: String, last_output: String) -> BlockScreen {
        BlockScreen::Command(CommandScreen { pos: None, minecart: Some(entity), command, last_output, track_output: true, mode: 2, conditional: false, needs_redstone: true, asked: String::new(), question: 0, suggestions: Vec::new(), suggest_start: 0, suggest_length: 0, suggest_pick: 0 })
    }

    pub fn pos(&self) -> Option<BlockPos> {
        match self {
            BlockScreen::Command(s) => s.pos,
            BlockScreen::Structure(s) => Some(s.pos),
            BlockScreen::Jigsaw(s) => Some(s.pos),
            BlockScreen::Test(s) => Some(s.pos),
            BlockScreen::TestInstance(s) => Some(s.pos),
        }
    }

    /// Fills the screen in from what the server says is in the block.
    pub fn take_data(&mut self, data: &Nbt, tr: &dyn lattice_proto::Translator) {
        let d = Some(data);
        match self {
            BlockScreen::Command(s) => {
                s.command = text_of(d, "Command");
                s.last_output = output_of(d, tr);
                s.track_output = bool_of(d, "TrackOutput", true);
                s.needs_redstone = !bool_of(d, "auto", false);
            }
            BlockScreen::Structure(s) => {
                s.name = text_of(d, "name");
                s.author = text_of(d, "author");
                s.mode = index_of(&STRUCTURE_MODES, &text_of(d, "mode"));
                s.offset = [int_of(d, "posX", 0).to_string(), int_of(d, "posY", 1).to_string(), int_of(d, "posZ", 0).to_string()];
                s.size = [int_of(d, "sizeX", 0).to_string(), int_of(d, "sizeY", 0).to_string(), int_of(d, "sizeZ", 0).to_string()];
                s.mirror = index_of(&MIRRORS, &text_of(d, "mirror"));
                s.rotation = index_of(&ROTATIONS, &text_of(d, "rotation"));
                s.data = text_of(d, "metadata");
                s.integrity = data.get("integrity").and_then(Nbt::as_f64).unwrap_or(1.0).to_string();
                s.seed = int_of(d, "seed", 0).to_string();
                s.include_entities = !bool_of(d, "ignoreEntities", false);
                s.show_air = bool_of(d, "showair", false);
                s.show_bounding_box = bool_of(d, "showboundingbox", true);
                s.strict = bool_of(d, "strict", false);
            }
            BlockScreen::Jigsaw(s) => {
                s.pool = text_of(d, "pool");
                s.name = text_of(d, "name");
                s.target = text_of(d, "target");
                s.final_state = text_of(d, "final_state");
                s.rollable = text_of(d, "joint") != "aligned";
                s.selection_priority = int_of(d, "selection_priority", 0).to_string();
                s.placement_priority = int_of(d, "placement_priority", 0).to_string();
            }
            BlockScreen::Test(s) => {
                s.mode = index_of(&TEST_MODES, &text_of(d, "mode"));
                s.message = text_of(d, "message");
            }
            BlockScreen::TestInstance(s) => {
                s.id = data.get("data").and_then(Nbt::as_str).unwrap_or_default().to_string();
                if let Some(size) = data.get("size") {
                    let read = |key: &str, i: usize| size.get(key).and_then(Nbt::as_i64).or_else(|| match size {
                        Nbt::IntArray(v) => v.get(i).map(|n| *n as i64),
                        _ => None,
                    });
                    for (i, key) in ["x", "y", "z"].into_iter().enumerate() {
                        if let Some(n) = read(key, i) {
                            s.size[i] = n.to_string();
                        }
                    }
                }
                s.rotation = data.get("rotation").and_then(Nbt::as_str).map_or(0, |r| {
                    let r = r.to_ascii_uppercase();
                    ROTATIONS.iter().position(|v| *v == r || (r == "NONE" && *v == "NONE")).unwrap_or(0)
                });
                s.include_entities = !bool_of(d, "ignore_entities", true);
            }
        }
    }

    /// Takes what a size-finding sweep or a report changed, leaving what's being typed.
    pub fn take_new_size(&mut self, data: &Nbt) {
        if let BlockScreen::Structure(s) = self {
            let d = Some(data);
            s.offset = [int_of(d, "posX", 0).to_string(), int_of(d, "posY", 1).to_string(), int_of(d, "posZ", 0).to_string()];
            s.size = [int_of(d, "sizeX", 0).to_string(), int_of(d, "sizeY", 0).to_string(), int_of(d, "sizeZ", 0).to_string()];
        }
    }
}

fn byte_of(text: &str, min: i32, max: i32) -> i8 {
    text.trim().parse::<i32>().unwrap_or(0).clamp(min, max) as i8
}

impl StructureScreen {
    /// `action`: 0 just store, 1 save the area, 2 load it, 3 find its size.
    pub fn packet(&self, action: i32) -> Encoded {
        let offset = [byte_of(&self.offset[0], -48, 48), byte_of(&self.offset[1], -48, 48), byte_of(&self.offset[2], -48, 48)];
        let size = [byte_of(&self.size[0], 0, 48), byte_of(&self.size[1], 0, 48), byte_of(&self.size[2], 0, 48)];
        play::set_structure_block(&play::StructureBlock {
            pos: self.pos,
            action,
            mode: self.mode as i32,
            name: &self.name,
            offset,
            size,
            mirror: self.mirror as i32,
            rotation: self.rotation as i32,
            data: &self.data,
            integrity: self.integrity.trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
            seed: self.seed.trim().parse::<i64>().unwrap_or(0),
            ignore_entities: !self.include_entities,
            show_air: self.show_air,
            show_bounding_box: self.show_bounding_box,
            strict: self.strict,
        })
    }
}

impl CommandScreen {
    /// Puts the chosen suggestion into the command. Returns where the cursor should go.
    pub fn accept_suggestion(&mut self) -> Option<usize> {
        let choice = self.suggestions.get(self.suggest_pick)?.clone();
        let chars: Vec<char> = self.command.chars().collect();
        let start = self.suggest_start.min(chars.len());
        let end = (self.suggest_start + self.suggest_length).min(chars.len());
        let mut out: String = chars[..start].iter().collect();
        out.push_str(&choice);
        let cursor = out.chars().count();
        out.extend(&chars[end..]);
        self.command = out;
        self.suggestions.clear();
        Some(cursor)
    }

    pub fn packet(&self) -> Option<Encoded> {
        match (self.pos, self.minecart) {
            (Some(pos), _) => Some(play::set_command_block(pos, &self.command, self.mode as i32, self.track_output, self.conditional, !self.needs_redstone)),
            (None, Some(entity)) => Some(play::set_command_minecart(entity, &self.command, self.track_output)),
            _ => None,
        }
    }
}

impl JigsawScreen {
    pub fn packet(&self) -> Encoded {
        let joint = if self.rollable || !self.vertical { "rollable" } else { "aligned" };
        play::set_jigsaw_block(self.pos, &self.name, &self.target, &self.pool, &self.final_state, joint, self.selection_priority.trim().parse().unwrap_or(0), self.placement_priority.trim().parse().unwrap_or(0))
    }
}

impl TestInstanceScreen {
    pub fn packet(&self, action: i32) -> Encoded {
        let size = [self.size[0].trim().parse().unwrap_or(1).clamp(1, 48), self.size[1].trim().parse().unwrap_or(1).clamp(1, 48), self.size[2].trim().parse().unwrap_or(1).clamp(1, 48)];
        let id = (!self.id.trim().is_empty()).then_some(self.id.trim());
        play::test_instance_block_action(self.pos, action, id, size, self.rotation as i32, !self.include_entities, 0)
    }
}
