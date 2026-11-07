//! The local player: input → actions → physics → movement packets.

use super::entities::Entities;
use super::inventory::{ARMOR_CHEST, ARMOR_FEET, Inventory};
use super::items::{self, BlockTags};
use lattice_proto::ItemStack;
use glam::DVec3;
use lattice_data::{GameData, flags};
use lattice_net::Connection;
use lattice_proto::BlockPos;
use lattice_proto::packets::play::{self, action, command, input};
use lattice_world::World;
use lattice_world::physics::{Aabb, Body, Keys, MoveCtx, MoveTags};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default)]
pub struct TickInput {
    pub keys: Keys,
    pub yaw: f32,
    pub pitch: f32,
    pub attack_held: bool,
    /// The attack button went down since the last tick.
    pub attack_pressed: bool,
    pub use_held: bool,
    pub use_pressed: bool,
    pub pick_pressed: bool,
    /// Copy block data along with the picked item (Ctrl held, creative mode).
    pub pick_with_data: bool,
    pub drop_pressed: bool,
    pub drop_stack: bool,
    pub swap_hands_pressed: bool,
    /// Auto-jump option.
    pub auto_jump: bool,
    pub sprint_window: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct HitResult {
    pub pos: BlockPos,
    /// Face index (0 down, 1 up, 2 north, 3 south, 4 west, 5 east).
    pub face: u8,
    pub cursor: [f32; 3],
}

const SETTLE_TICKS: u8 = 3;

struct Digging {
    pos: BlockPos,
    face: u8,
    progress: f32,
    ticks: u32,
}

/// Status effects on the player: effect id to (amplifier, duration, flags).
pub type ActiveEffects = HashMap<i32, (i32, i32, u8)>;

pub struct LocalPlayer {
    pub body: Body,
    pub prev_pos: DVec3,
    pub prev_eye_height: f32,
    pub eye_height: f32,
    pub entity_id: i32,
    /// From the server's attributes, for the HUD.
    pub armor: f64,
    pub max_health: f64,
    pub game_mode: i32,
    pub selected_slot: i32,
    pub instant_break: bool,
    pub received_position: bool,
    pub effects: ActiveEffects,
    /// Using an item, as the server says: `Some(hand)`.
    pub using: Option<i32>,
    /// A rotation the server set, for the camera to pick up.
    pub look_override: Option<(f32, f32)>,
    pub attack_strength_ticks: u32,
    pub attack_speed: f64,
    pub mining_efficiency: f64,
    pub block_break_speed: f64,
    pub submerged_mining_speed: f64,
    pub swing: super::swing::Swing,
    pub vehicle: Option<i32>,
    /// Where the server just put the vehicle being driven.
    pub vehicle_pos: Option<(DVec3, f32)>,
    /// The vehicle being steered, when it's this player's to steer.
    pub ride: Option<super::riding::Ride>,
    /// Degrees the vehicle turned the player this tick, eased in over the tick for the camera.
    pub turned: f32,
    /// Where the world border stands: west, east, north, south.
    pub border: Option<[f64; 4]>,
    /// Where the cape's catching up from.
    pub cloak: DVec3,
    pub prev_cloak: DVec3,
    /// Just went into the water at this speed, for the spray.
    pub splashed: Option<DVec3>,
    /// A book and quill was just used in this hand.
    pub write_book: Option<i32>,
    /// A game-master block was just used and should get its screen: where, and what state it's in.
    pub open_block: Option<(BlockPos, u32)>,
    pub open_minecart: Option<i32>,
    /// 0 to 4, what the server's told this player they're allowed.
    pub permission: u8,
    /// What to go back to after being a spectator.
    pub previous_game_mode: i32,
    /// Sounds this player's own actions make, which the server doesn't send back.
    pub cues: Vec<super::SoundCue>,
    /// Blocks broken this tick, for the shower of bits.
    pub broke: Vec<(BlockPos, u32)>,
    /// Ticks since joining, for animations.
    pub age: u32,
    /// How the body looks to others: walk cycle and which way it faces.
    pub limb_position: f32,
    pub limb_speed: f32,
    pub prev_limb_speed: f32,
    own_body_yaw: f32,
    prev_own_body_yaw: f32,
    /// Ticks spent using the current item.
    pub use_ticks: u32,
    /// What the arms are holding on screen, which lags the real thing to dip and come back.
    pub shown_main: Option<ItemStack>,
    pub shown_off: Option<ItemStack>,
    pub main_height: f32,
    pub prev_main_height: f32,
    pub off_height: f32,
    pub prev_off_height: f32,
    move_dist: f32,
    next_step: f32,
    was_in_water: bool,
    pub bob: f32,
    bob_prev: f32,
    pub walk_dist: f32,
    walk_dist_prev: f32,
    loaded_sent: bool,
    ticks_since_spawn: u32,
    last_sent_pos: DVec3,
    last_sent_rot: (f32, f32),
    last_on_ground: bool,
    last_wall: bool,
    /// Teleports waiting to be confirmed on the next tick: id, position, yaw, pitch.
    pending_accepts: Vec<(i32, [f64; 3], f32, f32)>,
    /// A teleport needs echoing with a position packet on the next tick.
    echo_position: bool,
    /// Ticks to wait after arriving before answering the server at all.
    settle_ticks: u8,
    ticks_since_pos: u32,
    last_input: u8,
    sprint_sent: bool,
    sequence: i32,
    digging: Option<Digging>,
    miss_time: u8,
    destroy_delay: u8,
    right_click_delay: u8,
    was_using: bool,
    pub target: Option<HitResult>,
    /// Entity under the crosshair and where the look ray meets it.
    pub target_entity: Option<(i32, DVec3)>,
}

/// Blocks with something to do when used, so the use doesn't fall through to the held item.
fn block_interactive(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "crafting_table", "furnace", "blast_furnace", "smoker", "chest", "trapped_chest", "ender_chest", "barrel", "dispenser", "dropper", "hopper",
        "anvil", "chipped_anvil", "damaged_anvil", "brewing_stand", "enchanting_table", "grindstone", "smithing_table", "stonecutter", "loom",
        "cartography_table", "beacon", "lectern", "lever", "repeater", "comparator", "note_block", "jukebox", "daylight_detector", "bell", "cake",
        "composter", "crafter", "command_block", "chain_command_block", "repeating_command_block", "structure_block", "jigsaw", "respawn_anchor",
        "lodestone", "flower_pot", "decorated_pot", "chiseled_bookshelf", "dragon_egg", "redstone_ore", "deepslate_redstone_ore", "vault", "test_block", "test_instance_block",
    ];
    EXACT.contains(&name)
        || name.ends_with("_door") && name != "iron_door"
        || name.ends_with("_trapdoor") && name != "iron_trapdoor"
        || name.ends_with("_fence_gate")
        || name.ends_with("_button")
        || name.ends_with("_bed")
        || name.ends_with("shulker_box")
        || name.ends_with("_sign")
        || name.ends_with("_hanging_sign")
        || name.ends_with("_shelf")
        || name.ends_with("copper_chest")
        || name.ends_with("copper_golem_statue")
        || name.ends_with("cauldron")
        || name.ends_with("campfire")
        || name.starts_with("potted_")
        || name.ends_with("_candle_cake")
}

/// Mobs and objects that take the use themselves, whatever is in hand.
fn entity_interactive(name: &str) -> bool {
    matches!(
        name,
        "villager" | "wandering_trader" | "horse" | "donkey" | "mule" | "llama" | "trader_llama" | "skeleton_horse" | "zombie_horse" | "camel" | "camel_husk"
            | "pig" | "strider" | "armor_stand" | "item_frame" | "glow_item_frame" | "minecart" | "chest_minecart" | "furnace_minecart" | "hopper_minecart"
            | "command_block_minecart" | "interaction" | "player"
    ) || name.ends_with("_boat")
        || name.ends_with("_raft")
        || name.ends_with("_chest_boat")
        || name.ends_with("_chest_raft")
}

impl LocalPlayer {
    pub fn new() -> Self {
        let data = lattice_data::game();
        LocalPlayer {
            body: Body::new(data),
            prev_pos: DVec3::ZERO,
            prev_eye_height: data.player.eye,
            eye_height: data.player.eye,
            entity_id: -1,
            armor: 0.0,
            max_health: 20.0,
            game_mode: 0,
            selected_slot: 0,
            instant_break: false,
            received_position: false,
            effects: HashMap::new(),
            using: None,
            look_override: None,
            attack_strength_ticks: 100,
            attack_speed: 4.0,
            mining_efficiency: 0.0,
            block_break_speed: 1.0,
            submerged_mining_speed: 0.2,
            swing: Default::default(),
            vehicle: None,
            vehicle_pos: None,
            ride: None,
            turned: 0.0,
            border: None,
            cloak: DVec3::ZERO,
            prev_cloak: DVec3::ZERO,
            splashed: None,
            write_book: None,
            open_block: None,
            open_minecart: None,
            permission: 0,
            previous_game_mode: 0,
            cues: Vec::new(),
            broke: Vec::new(),
            move_dist: 0.0,
            next_step: 1.0,
            was_in_water: false,
            age: 0,
            limb_position: 0.0,
            limb_speed: 0.0,
            prev_limb_speed: 0.0,
            own_body_yaw: 0.0,
            prev_own_body_yaw: 0.0,
            use_ticks: 0,
            shown_main: None,
            shown_off: None,
            main_height: 0.0,
            prev_main_height: 0.0,
            off_height: 0.0,
            prev_off_height: 0.0,
            bob: 0.0,
            bob_prev: 0.0,
            walk_dist: 0.0,
            walk_dist_prev: 0.0,
            loaded_sent: false,
            ticks_since_spawn: 0,
            last_sent_pos: DVec3::ZERO,
            last_sent_rot: (0.0, 0.0),
            last_on_ground: false,
            last_wall: false,
            pending_accepts: Vec::new(),
            echo_position: false,
            settle_ticks: SETTLE_TICKS,
            ticks_since_pos: 0,
            last_input: 0,
            sprint_sent: false,
            sequence: 0,
            digging: None,
            miss_time: 0,
            destroy_delay: 0,
            right_click_delay: 0,
            was_using: false,
            target: None,
            target_entity: None,
        }
    }

    pub fn reset_for_spawn(&mut self) {
        self.received_position = false;
        self.loaded_sent = false;
        self.settle_ticks = SETTLE_TICKS;
        self.ticks_since_spawn = 0;
        self.digging = None;
        self.body.vel = DVec3::ZERO;
        self.using = None;
        self.vehicle = None;
        self.ride = None;
        self.body.passenger = false;
        self.body.fall_flying = false;
        self.body.fall_distance = 0.0;
    }

    /// Where the eyes are, with the height eased as the game eases it between poses.
    pub fn eye(&self, partial: f64) -> DVec3 {
        let p = self.prev_pos.lerp(self.body.pos, partial);
        let h = self.prev_eye_height as f64 + (self.eye_height - self.prev_eye_height) as f64 * partial;
        p + DVec3::new(0.0, h, 0.0)
    }

    fn eye_now(&self) -> DVec3 {
        self.body.pos + DVec3::new(0.0, self.body.eye_height() as f64, 0.0)
    }

    pub fn apply_attributes(&mut self, attrs: &[play::Attribute]) {
        let names = lattice_data::game().registry("attribute");
        for a in attrs {
            let Some(name) = names.get(a.id as usize) else { continue };
            // Apply modifiers except the sprint one, which physics adds itself.
            let mut v = a.base;
            let mods = || a.modifiers.iter().filter(|m| !m.id.ends_with("sprinting"));
            v += mods().filter(|m| m.operation == 0).map(|m| m.amount).sum::<f64>();
            let base = v;
            v += base * mods().filter(|m| m.operation == 1).map(|m| m.amount).sum::<f64>();
            for m in mods().filter(|m| m.operation == 2) {
                v *= 1.0 + m.amount;
            }
            let at = &mut self.body.attrs;
            match name.as_str() {
                "movement_speed" => at.movement_speed = v,
                "jump_strength" => at.jump_strength = v,
                "gravity" => at.gravity = v,
                "step_height" => at.step_height = v,
                "sneaking_speed" => at.sneaking_speed = v,
                "water_movement_efficiency" => at.water_movement_efficiency = v,
                "movement_efficiency" => at.movement_efficiency = v,
                "friction_modifier" => at.friction_modifier = v,
                "air_drag_modifier" => at.air_drag_modifier = v,
                "armor" => self.armor = v,
                "max_health" => self.max_health = v,
                "scale" => at.scale = v,
                "block_interaction_range" => at.block_range = v,
                "entity_interaction_range" => at.entity_range = v,
                "attack_speed" => self.attack_speed = v,
                "mining_efficiency" => self.mining_efficiency = v,
                "block_break_speed" => self.block_break_speed = v,
                "submerged_mining_speed" => self.submerged_mining_speed = v,
                _ => {}
            }
        }
    }

    /// A status effect changed; movement cares about a few of them.
    pub fn set_effect(&mut self, effect: i32, value: Option<(i32, i32, u8)>) {
        match value {
            Some(v) => {
                self.effects.insert(effect, v);
            }
            None => {
                self.effects.remove(&effect);
            }
        }
        let names = lattice_data::game().registry("mob_effect");
        let amp = |name: &str| names.iter().position(|n| n == name).and_then(|i| self.effects.get(&(i as i32))).map(|v| v.0);
        let e = &mut self.body.effects;
        e.jump_boost = amp("jump_boost");
        e.levitation = amp("levitation");
        e.slow_falling = amp("slow_falling").is_some();
        e.dolphins_grace = amp("dolphins_grace").is_some();
        e.blindness = amp("blindness").is_some();
        e.weaving = amp("weaving").is_some();
    }

    pub fn has_effect(&self, name: &str) -> bool {
        self.effect_amp(name).is_some()
    }

    /// Ticks left of an effect; negative when it lasts forever.
    pub fn effect_duration(&self, name: &str) -> Option<i32> {
        let names = lattice_data::game().registry("mob_effect");
        names.iter().position(|n| n == name).and_then(|i| self.effects.get(&(i as i32))).map(|v| v.1)
    }

    fn effect_amp(&self, name: &str) -> Option<i32> {
        let names = lattice_data::game().registry("mob_effect");
        names.iter().position(|n| n == name).and_then(|i| self.effects.get(&(i as i32))).map(|v| v.0)
    }

    pub fn teleport(&mut self, id: i32, pos: [f64; 3], vel: [f64; 3], yaw: f32, pitch: f32, flags: i32) {
        let b = &mut self.body;
        let rel = |bit: i32| flags & bit != 0;
        let p = DVec3::new(
            if rel(0x01) { b.pos.x + pos[0] } else { pos[0] },
            if rel(0x02) { b.pos.y + pos[1] } else { pos[1] },
            if rel(0x04) { b.pos.z + pos[2] } else { pos[2] },
        );
        let new_yaw = if rel(0x08) { b.yaw + yaw } else { yaw };
        let new_pitch = if rel(0x10) { b.pitch + pitch } else { pitch };
        let v = DVec3::new(
            if rel(0x20) { b.vel.x + vel[0] } else { vel[0] },
            if rel(0x40) { b.vel.y + vel[1] } else { vel[1] },
            if rel(0x80) { b.vel.z + vel[2] } else { vel[2] },
        );
        b.pos = p;
        b.vel = v;
        b.yaw = new_yaw;
        b.pitch = new_pitch.clamp(-90.0, 90.0);
        b.on_ground = false;
        self.prev_pos = p;
        self.received_position = true;
        self.look_override = Some((b.yaw, b.pitch));
        // Both the confirmation and the position echo wait for the next tick.
        // The server runs its movement check on the position inside the
        // confirmation, which fails if it hasn't ticked this player yet, and it
        // disconnects a client that sends two position packets without a tick
        // end between them.
        self.pending_accepts.push((id, p.to_array(), b.yaw, b.pitch));
        self.echo_position = true;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        conn: &Connection,
        world: &mut World,
        inv: &mut Inventory,
        entities: &mut Entities,
        inp: TickInput,
        tags: &MoveTags,
        block_tags: &BlockTags,
        food: i32,
        fast_lava: bool,
        playing: bool,
        game_time: i64,
    ) {
        let data = lattice_data::game();
        self.prev_pos = self.body.pos;
        self.prev_eye_height = self.eye_height;
        self.ticks_since_spawn += 1;
        if !self.received_position {
            conn.send(play::client_tick_end());
            return;
        }
        self.body.yaw = inp.yaw;
        self.body.pitch = inp.pitch.clamp(-90.0, 90.0);
        self.body.spectator = self.game_mode == 3;

        // Give the server a moment after the player arrives; see `teleport`.
        if self.settle_ticks > 0 {
            self.settle_ticks -= 1;
            conn.send(play::client_tick_end());
            return;
        }
        for (id, pos, yaw, pitch) in self.pending_accepts.drain(..) {
            conn.send(play::accept_teleportation(id, pos, yaw, pitch));
        }
        // Must come before the first simulated tick: until then the server
        // ignores movement, and it only tracks positions from here on.
        if playing && !self.loaded_sent {
            conn.send(play::player_loaded());
            self.loaded_sent = true;
        }
        let playing = playing && self.loaded_sent;

        // Actions come first, as the game handles key presses before moving the player.
        if playing {
            self.update_target(world, data, entities);
            self.handle_actions(conn, world, inv, entities, data, block_tags, &inp);
        }

        // Input state for the server (sneaking is reported here since 1.21.2).
        let k = inp.keys;
        let mut flags_now = 0u8;
        if k.forward { flags_now |= input::FORWARD; }
        if k.back { flags_now |= input::BACKWARD; }
        if k.left { flags_now |= input::LEFT; }
        if k.right { flags_now |= input::RIGHT; }
        if k.jump { flags_now |= input::JUMP; }
        if k.shift { flags_now |= input::SNEAK; }
        if k.sprint { flags_now |= input::SPRINT; }
        if flags_now != self.last_input {
            conn.send(play::player_input(flags_now));
            self.last_input = flags_now;
        }

        self.turned = 0.0;
        if !self.body.passenger {
            self.ride = None;
            entities.driven = None;
        }
        if playing {
            let using_stack = self.using.and_then(|hand| if hand == 0 { inv.hotbar(self.selected_slot as usize) } else { inv.offhand() });
            let (use_speed, use_sprint) = using_stack.map_or((None, false), |s| {
                let (speed, sprint) = items::use_effects(s);
                (Some(speed), sprint)
            });
            let ctx = MoveCtx {
                data,
                tags,
                fast_lava,
                auto_jump: inp.auto_jump,
                sprint_window: inp.sprint_window,
                enough_food: food > 6 || self.body.may_fly,
                leather_boots: inv.slots[ARMOR_FEET].as_ref().is_some_and(|s| items::item_name(s) == "leather_boots"),
                can_glide: inv.slots[ARMOR_CHEST].as_ref().is_some_and(items::glides),
                item_use_speed: use_speed,
                item_use_can_sprint: use_sprint,
                min_y: world.geometry.min_y,
                border: self.border,
            };
            if self.body.passenger {
                self.tick_riding(conn, world, inv, entities, k, &ctx, game_time);
            } else {
                self.body.tick(k, world, &ctx);
                self.movement_sounds(world, &ctx, block_tags);
            }
            if std::mem::take(&mut self.body.abilities_changed) {
                conn.send(play::player_abilities(self.body.flying));
            }
            if std::mem::take(&mut self.body.start_gliding) {
                conn.send(play::player_command(self.entity_id, command::START_FALL_FLYING));
            }
        }
        // How the body looks: legs follow walking, the torso follows legs and head.
        self.age = self.age.wrapping_add(1);
        let moved = self.body.pos - self.prev_pos;
        let square = (moved.x * moved.x + moved.z * moved.z) as f32;
        self.prev_limb_speed = self.limb_speed;
        self.limb_speed += ((square.sqrt() * 4.0).min(1.0) - self.limb_speed) * 0.4;
        self.limb_position += self.limb_speed;
        self.prev_own_body_yaw = self.own_body_yaw;
        let mut facing = self.own_body_yaw;
        if square > 0.0025000002 {
            let f = (moved.z as f32).atan2(moved.x as f32).to_degrees() - 90.0;
            let off = (wrap_degrees(self.body.yaw) - f).abs();
            facing = if 95.0 < off && off < 265.0 { f - 180.0 } else { f };
        }
        if self.swing.active() {
            facing = self.body.yaw;
        }
        self.own_body_yaw += wrap_degrees(facing - self.own_body_yaw) * 0.3;
        let off = wrap_degrees(self.body.yaw - self.own_body_yaw);
        if off.abs() > 50.0 {
            self.own_body_yaw += off - 50.0 * off.signum();
        }
        self.use_ticks = if self.using.is_some() { self.use_ticks + 1 } else { 0 };
        // The held item dips while switching and after a hit, then comes back up.
        self.prev_main_height = self.main_height;
        self.prev_off_height = self.off_height;
        let real_main = inv.hotbar(self.selected_slot.clamp(0, 8) as usize).cloned();
        let real_off = inv.offhand().cloned();
        let same = |a: &Option<ItemStack>, b: &Option<ItemStack>| match (a, b) {
            (None, None) => true,
            (Some(x), Some(y)) => x.item == y.item,
            _ => false,
        };
        let charge = self.attack_charge(1.0);
        let main_target = if same(&self.shown_main, &real_main) { charge * charge * charge } else { 0.0 };
        self.main_height += (main_target - self.main_height).clamp(-0.4, 0.4);
        let off_target = if same(&self.shown_off, &real_off) { 1.0 } else { 0.0 };
        self.off_height += (off_target - self.off_height).clamp(-0.4, 0.4);
        if self.main_height < 0.1 || same(&self.shown_main, &real_main) {
            self.shown_main = real_main;
        }
        if self.off_height < 0.1 || same(&self.shown_off, &real_off) {
            self.shown_off = real_off;
        }

        // A cape trails behind, and doesn't try to follow a long way.
        self.prev_cloak = self.cloak;
        let trail = self.body.pos - self.cloak;
        if trail.abs().max_element() > 10.0 {
            self.cloak = self.body.pos;
            self.prev_cloak = self.body.pos;
        } else {
            self.cloak += trail * 0.25;
        }
        // Walking, for view bobbing.
        self.walk_dist_prev = self.walk_dist;
        self.walk_dist += (moved.x * moved.x + moved.z * moved.z).sqrt() as f32 * 0.6;
        self.bob_prev = self.bob;
        let v = self.body.vel;
        let target = if self.body.on_ground && !self.body.is_swimming() { ((v.x * v.x + v.z * v.z).sqrt() as f32).min(0.1) } else { 0.0 };
        self.bob += (target - self.bob) * 0.4;
        // Eyes ease towards the height of the current pose.
        let target = self.body.eye_height();
        self.eye_height += (target - self.eye_height) * 0.5;
        if (target - self.eye_height).abs() < 0.001 {
            self.eye_height = target;
        }

        self.attack_strength_ticks = self.attack_strength_ticks.saturating_add(1);
        self.swing.tick();
        if self.body.sprinting != self.sprint_sent {
            conn.send(play::player_command(self.entity_id, if self.body.sprinting { command::START_SPRINTING } else { command::STOP_SPRINTING }));
            self.sprint_sent = self.body.sprinting;
        }

        // Movement packets, as the game sends them.
        let b = &self.body;
        self.ticks_since_pos += 1;
        let moved = b.pos.distance_squared(self.last_sent_pos) > 2.0e-4 * 2.0e-4 || self.ticks_since_pos >= 20;
        let rotated = (b.yaw, b.pitch) != self.last_sent_rot;
        let mut mflags = 0u8;
        if b.on_ground {
            mflags |= play::ON_GROUND;
        }
        if b.horizontal_collision {
            mflags |= play::AGAINST_WALL;
        }
        let p = b.pos.to_array();
        if self.echo_position {
            conn.send(play::move_player_pos_rot(p, b.yaw, b.pitch, mflags));
            self.echo_position = false;
            self.last_sent_pos = b.pos;
            self.last_sent_rot = (b.yaw, b.pitch);
            self.ticks_since_pos = 0;
        } else if !self.loaded_sent {
            // Nothing else before the server expects it.
        } else if b.passenger {
            // Riding: only the look is ours to report, and it's sent every tick.
            conn.send(play::move_player_rot(b.yaw, b.pitch, mflags));
        } else if moved && rotated {
            conn.send(play::move_player_pos_rot(p, b.yaw, b.pitch, mflags));
        } else if moved {
            conn.send(play::move_player_pos(p, mflags));
        } else if rotated {
            conn.send(play::move_player_rot(b.yaw, b.pitch, mflags));
        } else if b.on_ground != self.last_on_ground || b.horizontal_collision != self.last_wall {
            conn.send(play::move_player_status_only(mflags));
        }
        if self.loaded_sent {
            if moved {
                self.last_sent_pos = b.pos;
                self.ticks_since_pos = 0;
            }
            if rotated {
                self.last_sent_rot = (b.yaw, b.pitch);
            }
            self.last_on_ground = b.on_ground;
            self.last_wall = b.horizontal_collision;
        }
        conn.send(play::client_tick_end());
    }

    pub fn look_dir(&self) -> DVec3 {
        let (y, p) = ((self.body.yaw as f64).to_radians(), (self.body.pitch as f64).to_radians());
        DVec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos())
    }

    fn next_sequence(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }

    /// What the crosshair is on: the nearer of the block and entity the look ray meets.
    fn update_target(&mut self, world: &World, data: &GameData, entities: &Entities) {
        let eye = self.eye_now();
        let dir = self.look_dir();
        let block_range = self.body.attrs.block_range;
        let entity_range = self.body.attrs.entity_range;
        let block = raycast(world, data, eye, dir, block_range);
        let block_dist = block.map(|h| {
            let c = DVec3::new(h.pos.x as f64, h.pos.y as f64, h.pos.z as f64) + DVec3::from([h.cursor[0] as f64, h.cursor[1] as f64, h.cursor[2] as f64]);
            c.distance(eye)
        });
        let mut best: Option<(i32, DVec3, f64)> = None;
        for (id, e) in &entities.map {
            if !e.is_pickable() {
                continue;
            }
            let (w, h) = e.dimensions();
            let half = w / 2.0;
            let min = e.pos - DVec3::new(half, 0.0, half);
            let max = e.pos + DVec3::new(half, h, half);
            if let Some((t, _)) = ray_box(eye, dir, min, max) {
                if t <= entity_range && best.is_none_or(|(_, _, d)| t < d) {
                    best = Some((*id, eye + dir * t, t));
                }
            }
        }
        self.target_entity = None;
        self.target = block;
        if let Some((id, hit, d)) = best {
            if block_dist.is_none_or(|bd| d < bd) {
                self.target_entity = Some((id, hit));
                self.target = None;
            }
        }
    }

    pub fn swing(&mut self, conn: &Connection, inv: &Inventory) {
        self.start_swing(inv, false);
        conn.send(play::punch());
    }

    /// Moves the arm. What's held and some effects change how long that takes.
    fn start_swing(&mut self, inv: &Inventory, off_hand: bool) {
        let held = if off_hand { inv.offhand() } else { inv.hotbar(self.selected_slot as usize) };
        let normal = held.map_or(super::swing::NORMAL_TICKS, items::swing_ticks) as i32;
        let quicker = self.effect_amp("haste").into_iter().chain(self.effect_amp("conduit_power")).max();
        let ticks = match (quicker, self.effect_amp("mining_fatigue")) {
            (Some(amp), _) => normal - (1 + amp),
            (None, Some(amp)) => normal + (1 + amp) * 2,
            (None, None) => normal,
        };
        self.swing.start(off_hand, ticks.max(1) as u32);
    }

    /// How far a block breaks per tick with what's in hand.
    fn destroy_progress(&self, state: u32, inv: &Inventory, data: &GameData, tags: &BlockTags) -> f32 {
        let block = data.block(state);
        let hardness = block.hardness;
        if hardness < 0.0 {
            return 0.0;
        }
        if hardness == 0.0 {
            return 1.0;
        }
        let idx = data.block_index(state) as u16;
        let held = inv.hotbar(self.selected_slot as usize);
        let tool = held.and_then(|s| items::tool(s, data));
        let mut speed: f32 = tool.as_ref().map_or(1.0, |t| t.mining_speed(tags, idx));
        if speed > 1.0 {
            speed += self.mining_efficiency as f32;
        }
        let haste = self.effect_amp("haste");
        let conduit = self.effect_amp("conduit_power");
        if haste.is_some() || conduit.is_some() {
            let level = haste.unwrap_or(-1).max(conduit.unwrap_or(-1));
            speed *= 1.0 + (level + 1) as f32 * 0.2;
        }
        if let Some(a) = self.effect_amp("mining_fatigue") {
            speed *= match a {
                0 => 0.3,
                1 => 0.09,
                2 => 0.0027,
                _ => 8.1e-4,
            };
        }
        speed *= self.block_break_speed as f32;
        if self.body.eye_in_water {
            speed *= self.submerged_mining_speed as f32;
        }
        if !self.body.on_ground {
            speed /= 5.0;
        }
        let correct = !block.tool || tool.as_ref().is_some_and(|t| t.correct_for_drops(tags, idx));
        speed / hardness / if correct { 30.0 } else { 100.0 }
    }

    fn predict_break(&mut self, world: &mut World, data: &GameData, pos: BlockPos) {
        let Some(state) = world.block_at(pos) else { return };
        let block = data.block(state);
        if let Some(st) = data.sound_types.get(block.sound as usize) {
            self.cues.push(super::SoundCue::at(&st.break_sound, crate::audio::BLOCKS, pos, (st.volume + 1.0) / 2.0, st.pitch * 0.8));
        }
        self.broke.push((pos, state));
        // What's left behind: water stays if the block was waterlogged.
        let info = data.state(state);
        let left = if info.fluid == 1 && info.has(flags::FLUID_SOURCE) {
            data.block_by_name("water").map_or(0, |w| w.default)
        } else {
            0
        };
        world.set_block(pos, left);
    }

    /// Shows a plain block as soon as it's placed, in time with the arm. The server
    /// answers every placement with what's really there, which corrects a wrong guess.
    /// Blocks whose look depends on how they're placed (stairs, logs, doors) wait for it.
    fn predict_place(&mut self, world: &mut World, entities: &Entities, data: &GameData, hit: HitResult, block: &lattice_data::BlockInfo) {
        if block.count != 1 || self.game_mode == 2 || !data.state(block.default).has(flags::FULL_COLLISION) {
            return;
        }
        let Some(clicked) = world.block_at(hit.pos) else { return };
        let at = if data.state(clicked).has(flags::REPLACEABLE) {
            hit.pos
        } else {
            let (dx, dy, dz) = face_step(hit.face);
            BlockPos::new(hit.pos.x + dx, hit.pos.y + dy, hit.pos.z + dz)
        };
        let Some(there) = world.block_at(at) else { return };
        if !data.state(there).has(flags::REPLACEABLE) {
            return;
        }
        let min = DVec3::new(at.x as f64, at.y as f64, at.z as f64);
        let cube = Aabb::new(min, min + DVec3::ONE);
        let overlaps = |b: &Aabb| b.min.x < cube.max.x && b.max.x > cube.min.x && b.min.y < cube.max.y && b.max.y > cube.min.y && b.min.z < cube.max.z && b.max.z > cube.min.z;
        let someone_there = entities.map.values().any(|e| {
            let (w, h) = e.dimensions();
            let half = DVec3::new(w / 2.0, 0.0, w / 2.0);
            overlaps(&Aabb::new(e.pos - half, e.pos + half + DVec3::new(0.0, h, 0.0)))
        });
        if overlaps(&self.body.aabb()) || someone_there {
            return;
        }
        world.set_block(at, block.default);
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_actions(&mut self, conn: &Connection, world: &mut World, inv: &mut Inventory, entities: &Entities, data: &GameData, tags: &BlockTags, inp: &TickInput) {
        if self.miss_time > 0 {
            self.miss_time -= 1;
        }
        if self.right_click_delay > 0 {
            self.right_click_delay -= 1;
        }
        // Releasing use ends charging a bow, eating and so on.
        if self.using.is_some() && !inp.use_held {
            conn.send(play::player_action(action::RELEASE_USE_ITEM, BlockPos::new(0, 0, 0), 0, 0));
            self.using = None;
        }
        if inp.attack_pressed && self.using.is_none() {
            self.start_attack(conn, world, inv, entities, data, tags);
        }
        if inp.use_pressed || (inp.use_held && self.right_click_delay == 0) {
            if self.using.is_none() {
                self.start_use(conn, world, inv, entities, data, inp);
            }
        }
        if inp.pick_pressed {
            if let Some((id, _)) = self.target_entity {
                conn.send(play::pick_item_from_entity(id, inp.pick_with_data));
            } else if let Some(hit) = self.target {
                conn.send(play::pick_item_from_block(hit.pos, inp.pick_with_data));
            }
        }
        if inp.drop_pressed && self.game_mode != 3 {
            let status = if inp.drop_stack { action::DROP_ALL_ITEMS } else { action::DROP_ITEM };
            conn.send(play::player_action(status, BlockPos::new(0, 0, 0), 0, 0));
            // The server assumes the client took it out of the slot and won't say so.
            let slot = &mut inv.slots[super::inventory::HOTBAR_START + self.selected_slot as usize];
            if let Some(s) = slot {
                s.count -= if inp.drop_stack { s.count } else { 1 };
                if s.count <= 0 {
                    *slot = None;
                }
                self.start_swing(inv, false);
            }
        }
        if inp.swap_hands_pressed && self.game_mode != 3 {
            conn.send(play::player_action(action::SWAP_WITH_OFFHAND, BlockPos::new(0, 0, 0), 0, 0));
        }
        // Breaking continues while the button is held.
        if !inp.attack_held {
            self.miss_time = 0;
        }
        if self.miss_time == 0 && self.using.is_none() {
            if let (true, Some(hit)) = (inp.attack_held, self.target) {
                self.continue_destroy(conn, world, inv, data, tags, hit);
            } else if let Some(d) = self.digging.take() {
                let seq = self.next_sequence();
                conn.send(play::player_action(action::ABORT_DESTROY, d.pos, d.face, seq));
            }
        }
    }

    fn can_break(&self) -> bool {
        // Adventure mode needs an item allowed to; the server decides, and spectators can't.
        self.game_mode != 3
    }

    #[allow(clippy::too_many_arguments)]
    fn start_attack(&mut self, conn: &Connection, world: &mut World, inv: &Inventory, entities: &Entities, data: &GameData, tags: &BlockTags) {
        if self.miss_time > 0 {
            return;
        }
        if self.game_mode == 3 {
            // Spectators take the view of an entity by clicking it.
            let target = self.target_entity.map(|(id, _)| id).filter(|id| entities.map.contains_key(id));
            conn.send(play::spectator_action(target));
            return;
        }
        if let Some((id, _)) = self.target_entity {
            conn.send(play::attack(id));
            self.attack_strength_ticks = 0;
            if self.game_mode == 1 {
                self.destroy_delay = 5;
            }
        } else if let Some(hit) = self.target {
            self.start_destroy(conn, world, inv, data, tags, hit);
        } else {
            if self.game_mode != 1 {
                self.miss_time = 10;
            }
            self.attack_strength_ticks = 0;
        }
        self.swing(conn, inv);
    }

    fn start_destroy(&mut self, conn: &Connection, world: &mut World, inv: &Inventory, data: &GameData, tags: &BlockTags, hit: HitResult) {
        if !self.can_break() {
            return;
        }
        let state = world.block_at(hit.pos).unwrap_or(0);
        if state == 0 {
            return;
        }
        if self.game_mode == 1 {
            let can = inv.hotbar(self.selected_slot as usize).and_then(|s| items::tool(s, data)).is_none_or(|t| t.can_destroy_in_creative);
            let seq = self.next_sequence();
            conn.send(play::player_action(action::START_DESTROY, hit.pos, hit.face, seq));
            if can {
                self.predict_break(world, data, hit.pos);
            }
            self.destroy_delay = 5;
            return;
        }
        if let Some(d) = self.digging.take() {
            let seq = self.next_sequence();
            conn.send(play::player_action(action::ABORT_DESTROY, d.pos, d.face, seq));
        }
        let progress = self.destroy_progress(state, inv, data, tags);
        let seq = self.next_sequence();
        conn.send(play::player_action(action::START_DESTROY, hit.pos, hit.face, seq));
        if progress >= 1.0 {
            self.predict_break(world, data, hit.pos);
            self.destroy_delay = 5;
        } else {
            self.digging = Some(Digging { pos: hit.pos, face: hit.face, progress: 0.0, ticks: 0 });
        }
    }

    fn continue_destroy(&mut self, conn: &Connection, world: &mut World, inv: &Inventory, data: &GameData, tags: &BlockTags, hit: HitResult) {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
            self.swing(conn, inv);
            return;
        }
        if !self.can_break() {
            return;
        }
        if self.game_mode == 1 {
            let can = inv.hotbar(self.selected_slot as usize).and_then(|s| items::tool(s, data)).is_none_or(|t| t.can_destroy_in_creative);
            self.destroy_delay = 5;
            let seq = self.next_sequence();
            conn.send(play::player_action(action::START_DESTROY, hit.pos, hit.face, seq));
            if can {
                self.predict_break(world, data, hit.pos);
            }
            self.swing(conn, inv);
            return;
        }
        let same = self.digging.as_ref().is_some_and(|d| d.pos == hit.pos);
        if !same {
            self.start_destroy(conn, world, inv, data, tags, hit);
            self.swing(conn, inv);
            return;
        }
        let state = world.block_at(hit.pos).unwrap_or(0);
        if state == 0 {
            self.digging = None;
            return;
        }
        let per_tick = self.destroy_progress(state, inv, data, tags);
        let Some(d) = &mut self.digging else { return };
        // Knocking at a block, every few ticks.
        if d.ticks % 4 == 0 {
            if let Some(st) = data.sound_types.get(data.block(state).sound as usize) {
                self.cues.push(super::SoundCue::at(&st.hit, crate::audio::BLOCKS, d.pos, (st.volume + 1.0) / 8.0, st.pitch * 0.5));
            }
        }
        d.progress += per_tick;
        d.ticks += 1;
        if d.progress >= 1.0 {
            let (pos, face) = (d.pos, d.face);
            self.digging = None;
            let seq = self.next_sequence();
            conn.send(play::player_action(action::STOP_DESTROY, pos, face, seq));
            self.predict_break(world, data, pos);
            self.destroy_delay = 5;
        }
        self.swing(conn, inv);
    }

    /// Creative and allowed by the server, the same test the game makes.
    pub fn can_use_game_master_blocks(&self) -> bool {
        self.game_mode == 1 && self.permission >= 2
    }

    /// Right button: entity, then block, then the item itself, for each hand in turn.
    fn start_use(&mut self, conn: &Connection, world: &mut World, inv: &Inventory, entities: &Entities, data: &GameData, inp: &TickInput) {
        if self.digging.is_some() {
            return;
        }
        self.right_click_delay = 4;
        let sneaking = inp.keys.shift;
        let main = inv.hotbar(self.selected_slot as usize);
        let off = inv.offhand();
        for hand in [0i32, 1] {
            let stack = if hand == 0 { main } else { off };
            if let Some((id, hit)) = self.target_entity {
                if let Some(e) = entities.map.get(&id) {
                    let at = hit - e.pos;
                    conn.send(play::interact(id, hand, at.to_array(), sneaking));
                    if e.type_name() == "command_block_minecart" && self.can_use_game_master_blocks() {
                        self.open_minecart = Some(id);
                    }
                    if self.game_mode == 3 {
                        return;
                    }
                    if entity_interactive(e.type_name()) {
                        self.start_swing(inv, hand == 1);
                        return;
                    }
                }
            } else if let Some(hit) = self.target {
                let state = world.block_at(hit.pos).unwrap_or(0);
                let name = data.block(state).name.as_str();
                let eye = self.eye_now();
                let inside = world.block(eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32).is_some_and(|s| !data.collision(s).is_empty());
                let seq = self.next_sequence();
                conn.send(play::use_item_on(hand, hit.pos, hit.face as i32, hit.cursor, inside, seq));
                let holds_any = main.is_some() || off.is_some();
                let opens = block_interactive(name) && !(sneaking && holds_any);
                if opens && super::gm_blocks::has_screen(name) && self.can_use_game_master_blocks() {
                    self.open_block = Some((hit.pos, state));
                }
                let places = stack.is_some_and(|s| items::places_block(s, data).is_some() || items::has_component(s, "block_transformer"));
                if opens || places {
                    self.start_swing(inv, hand == 1);
                    // The server leaves this sound to whoever's placing.
                    if !opens {
                        if let Some(block) = stack.and_then(|s| items::places_block(s, data)) {
                            let b = &data.blocks[block as usize];
                            self.predict_place(world, entities, data, hit, b);
                            if let Some(st) = data.sound_types.get(b.sound as usize) {
                                let (dx, dy, dz) = face_step(hit.face);
                                let at = BlockPos::new(hit.pos.x + dx, hit.pos.y + dy, hit.pos.z + dz);
                                self.cues.push(super::SoundCue::at(&st.place, crate::audio::BLOCKS, at, (st.volume + 1.0) / 2.0, st.pitch * 0.8));
                            }
                        }
                    }
                    return;
                }
            }
            if self.game_mode != 3 {
                if let Some(s) = stack {
                    let seq = self.next_sequence();
                    conn.send(play::use_item(hand, seq, self.body.yaw, self.body.pitch));
                    if items::item_name(s) == "writable_book" {
                        self.write_book = Some(hand);
                    }
                    return;
                }
            }
        }
    }

    pub fn dig_progress(&self) -> Option<(BlockPos, f32)> {
        self.digging.as_ref().map(|d| (d.pos, d.progress.min(1.0)))
    }

    pub fn select_slot(&mut self, conn: &Connection, slot: i32) {
        let slot = slot.rem_euclid(9);
        if slot != self.selected_slot {
            self.selected_slot = slot;
            conn.send(play::set_carried_item(slot as i16));
        }
    }

    /// How much sprinting, flying and speed effects widen the view, scaled by the setting.
    pub fn fov_target(&self, scale: f32, first_person: bool) -> f32 {
        let mut f = 1.0f32;
        if self.body.flying {
            f *= 1.1;
        }
        let speed = if self.body.sprinting { self.body.attrs.movement_speed * 1.3 } else { self.body.attrs.movement_speed } as f32;
        f *= (speed / 0.1 + 1.0) / 2.0;
        if let Some(hand) = self.using {
            let stack = if hand == 0 { self.shown_main.as_ref() } else { self.shown_off.as_ref() };
            match stack.map(items::item_name) {
                Some("bow") => {
                    let pull = (self.use_ticks as f32 / 20.0).min(1.0);
                    f *= 1.0 - pull * pull * 0.15;
                }
                // Looking through a spyglass zooms however the effects are set.
                Some("spyglass") if first_person => return 0.1,
                _ => {}
            }
        }
        1.0 + (f - 1.0) * scale
    }

    /// Step amount and position along the walk, for view bobbing.
    /// Footsteps and splashes. The server only tells other players about these.
    fn movement_sounds(&mut self, world: &World, ctx: &MoveCtx, tags: &BlockTags) {
        let data = ctx.data;
        let moved = self.body.pos - self.prev_pos;
        let random = |seed: f32| (seed * 12.9898).sin().rem_euclid(1.0);
        // Getting wet.
        let wet = self.body.in_water;
        if wet && !self.was_in_water && self.game_mode != 3 && self.ticks_since_spawn > 20 {
            let v = self.body.vel;
            let volume = (((v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2).sqrt() as f32) * 0.2).min(1.0);
            let event = if volume < 0.25 { "entity.player.splash" } else { "entity.player.splash.high_speed" };
            let pitch = 1.0 + (random(self.age as f32) - random(self.age as f32 + 1.0)) * 0.4;
            self.cues.push(super::SoundCue::around(event, crate::audio::PLAYERS, self.body.pos, volume, pitch));
            self.splashed = Some(self.body.vel);
        }
        self.was_in_water = wet;
        if self.body.flying || self.game_mode == 3 {
            return;
        }
        let (exact, legacy) = self.body.step_blocks(world, ctx);
        let state_at = |p: [i32; 3]| world.block(p[0], p[1], p[2]).unwrap_or(0);
        let on = state_at(exact);
        let climbing = self.body.on_climbable(world, ctx);
        let horizontal = ((moved.x * moved.x + moved.z * moved.z).sqrt() * 0.6) as f32;
        self.move_dist += if climbing { (moved.length() * 0.6) as f32 } else { horizontal };
        if self.move_dist <= self.next_step || data.state(on).is_air() {
            return;
        }
        // Crouching along the ground makes no noise.
        if self.body.on_ground && self.body.crouching {
            return;
        }
        let under = state_at(legacy);
        let stepped = !data.state(under).is_air() && (self.body.on_ground || climbing) && !self.body.is_swimming();
        if stepped {
            self.next_step = self.move_dist.floor() + 1.0;
            let at = self.body.pos;
            let sound = |state: u32, volume: f32, pitch: f32, cues: &mut Vec<super::SoundCue>| {
                if let Some(st) = data.sound_types.get(data.block(state).sound as usize) {
                    cues.push(super::SoundCue::around(&st.step, crate::audio::PLAYERS, at, st.volume * volume, st.pitch * pitch));
                }
            };
            if wet {
                self.swim_sound();
                sound(under, 0.05, 0.8, &mut self.cues);
                return;
            }
            // Thin things on top of a block, like carpet or snow, change what a step sounds like.
            let above = [legacy[0], legacy[1] + 1, legacy[2]];
            let above_state = state_at(above);
            let index = data.block_index(above_state) as u16;
            let has = |name: &str| tags.0.get(name).is_some_and(|set| set.contains(&index));
            if has("combination_step_sound_blocks") {
                sound(above_state, 0.15, 1.0, &mut self.cues);
                sound(under, 0.05, 0.8, &mut self.cues);
            } else if has("inside_step_sound_blocks") {
                sound(above_state, 0.15, 1.0, &mut self.cues);
            } else {
                sound(under, 0.15, 1.0, &mut self.cues);
            }
        } else if wet {
            self.next_step = self.move_dist.floor() + 1.0;
            self.swim_sound();
        }
    }

    fn swim_sound(&mut self) {
        let v = self.body.vel;
        let volume = (((v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2).sqrt() as f32) * 0.35).min(1.0);
        let s = (self.age as f32 * 12.9898).sin().rem_euclid(1.0);
        self.cues.push(super::SoundCue::around("entity.player.swim", crate::audio::PLAYERS, self.body.pos, volume, 0.8 + s * 0.4));
    }

    /// Being carried: the vehicle moves, by this player's keys if it's theirs to steer, and
    /// the seat goes with it.
    #[allow(clippy::too_many_arguments)]
    fn tick_riding(&mut self, conn: &Connection, world: &World, inv: &Inventory, entities: &mut Entities, k: Keys, ctx: &MoveCtx, game_time: i64) {
        self.body.sit_tick(world, ctx);
        let Some(id) = self.vehicle else { return };
        let Some(e) = entities.map.get(&id) else { return };
        let me = self.entity_id;
        let main = inv.hotbar(self.selected_slot.clamp(0, 8) as usize);
        let off = inv.offhand();
        let holding = |name: &str| main.is_some_and(|s| items::item_name(s) == name) || off.is_some_and(|s| items::item_name(s) == name);
        let kind = if e.passengers.first() == Some(&me) { super::riding::controls(e, &holding) } else { None };
        match kind {
            Some(kind) => {
                if self.ride.as_ref().is_none_or(|r| r.vehicle != id || r.kind != kind) {
                    self.ride = Some(super::riding::Ride::new(id, kind, e, lattice_data::game()));
                }
            }
            None => self.ride = None,
        }
        entities.driven = self.ride.as_ref().map(|r| r.vehicle);
        let axes = self.body.rider_axes(k, ctx);
        if let Some(ride) = self.ride.as_mut() {
            if let Some((pos, yaw)) = self.vehicle_pos.take() {
                ride.snap_to(pos, yaw);
            }
            let out = ride.tick(e, self.body.yaw, self.body.pitch, k, axes, world, ctx, game_time, conn, me);
            ride.report(conn);
            let (turned, is_boat) = (ride.turned, ride.kind == super::riding::Kind::Boat);
            entities.drive(id, out.pos, out.yaw, out.pitch, out.on_ground);
            if is_boat {
                // A boat carries its rider round with it, and only lets them look so far back.
                let wrapped = super::riding::wrap(self.body.yaw + turned - out.yaw);
                let clamped = wrapped.clamp(-105.0, 105.0);
                let yaw = self.body.yaw + turned + clamped - wrapped;
                self.turned = yaw - self.body.yaw;
                self.body.yaw = yaw;
                self.look_override = Some((yaw, self.body.pitch));
            }
        } else {
            self.vehicle_pos = None;
        }
        let Some(e) = entities.map.get(&id) else { return };
        let index = e.passengers.iter().position(|p| *p == me).unwrap_or(0);
        static RIDES_AT: std::sync::OnceLock<[f64; 3]> = std::sync::OnceLock::new();
        let at = *RIDES_AT.get_or_init(|| lattice_data::game().entities.iter().find(|e| e.name == "player").map_or([0.0, 0.6, 0.0], |i| i.rides_at));
        self.body.pos = super::riding::seat(e, e.yaw, index, e.passengers.len(), false, at);
        self.body.vel = DVec3::ZERO;
    }

    /// The server moved the vehicle being driven.
    pub fn server_moved_vehicle(&mut self, pos: [f64; 3], yaw: f32, _pitch: f32) {
        self.vehicle_pos = Some((DVec3::from(pos), yaw));
    }

    pub fn is_sprinting(&self) -> bool {
        self.body.sprinting
    }

    /// The block being broken and how far along, 0..1.
    pub fn digging_progress(&self) -> Option<(BlockPos, f32)> {
        self.digging.as_ref().map(|d| (d.pos, d.progress))
    }

    /// Which way the body faces, degrees.
    pub fn body_yaw(&self, partial: f32) -> f32 {
        self.prev_own_body_yaw + wrap_degrees(self.own_body_yaw - self.prev_own_body_yaw) * partial
    }

    pub fn view_bob(&self, partial: f32) -> (f32, f32) {
        let b = self.bob_prev + (self.bob - self.bob_prev) * partial;
        let t = -(self.walk_dist + (self.walk_dist - self.walk_dist_prev) * partial);
        (b, t)
    }

    /// Whether the attack is fully charged: 0..1, as the attack indicator shows.
    pub fn attack_charge(&self, partial: f32) -> f32 {
        let per_second = self.attack_speed.max(0.001);
        let delay = (1.0 / per_second * 20.0) as f32;
        ((self.attack_strength_ticks as f32 + partial) / delay).clamp(0.0, 1.0)
    }

    /// The server's view of using an item changed.
    pub fn set_using_from_server(&mut self, flags: u8) {
        self.using = (flags & 1 != 0).then_some(if flags & 2 != 0 { 1 } else { 0 });
        self.was_using = self.using.is_some();
    }
}

/// The step from a block to the space next to the given face.
fn face_step(face: u8) -> (i32, i32, i32) {
    match face {
        0 => (0, -1, 0),
        1 => (0, 1, 0),
        2 => (0, 0, -1),
        3 => (0, 0, 1),
        4 => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

fn wrap_degrees(mut d: f32) -> f32 {
    d %= 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// Finds the first block outline hit along a ray (voxel walk + per-box test).
pub fn raycast(world: &World, data: &GameData, from: DVec3, dir: DVec3, range: f64) -> Option<HitResult> {
    let mut best: Option<(f64, HitResult)> = None;
    let steps = (range * 3.0) as i32 + 2;
    let mut cell = from.floor();
    let step = dir.signum();
    let inv = DVec3::new(1.0 / dir.x.abs().max(1e-12), 1.0 / dir.y.abs().max(1e-12), 1.0 / dir.z.abs().max(1e-12));
    let mut t_max = DVec3::new(
        if dir.x > 0.0 { cell.x + 1.0 - from.x } else { from.x - cell.x } * inv.x,
        if dir.y > 0.0 { cell.y + 1.0 - from.y } else { from.y - cell.y } * inv.y,
        if dir.z > 0.0 { cell.z + 1.0 - from.z } else { from.z - cell.z } * inv.z,
    );
    for _ in 0..steps {
        let (x, y, z) = (cell.x as i32, cell.y as i32, cell.z as i32);
        if let Some(state) = world.block(x, y, z) {
            if state != 0 {
                let block = data.block(state);
                let off = lattice_data::block_offset(block, x, y, z);
                for b in data.outline(state) {
                    let min = cell + DVec3::from(b.min) + DVec3::from(off);
                    let max = cell + DVec3::from(b.max) + DVec3::from(off);
                    if let Some((t, face)) = ray_box(from, dir, min, max) {
                        if t <= range && best.as_ref().is_none_or(|(bt, _)| t < *bt) {
                            let hit = from + dir * t;
                            let c = hit - cell;
                            best = Some((t, HitResult { pos: BlockPos::new(x, y, z), face, cursor: [c.x as f32, c.y as f32, c.z as f32] }));
                        }
                    }
                }
            }
        }
        if best.is_some() {
            break;
        }
        if t_max.x < t_max.y && t_max.x < t_max.z {
            cell.x += step.x;
            t_max.x += inv.x;
        } else if t_max.y < t_max.z {
            cell.y += step.y;
            t_max.y += inv.y;
        } else {
            cell.z += step.z;
            t_max.z += inv.z;
        }
    }
    best.map(|(_, h)| h)
}

fn ray_box(o: DVec3, d: DVec3, min: DVec3, max: DVec3) -> Option<(f64, u8)> {
    let mut t0 = 0.0f64;
    let mut t1 = f64::MAX;
    let mut face = 0u8;
    for a in 0..3 {
        if d[a].abs() < 1e-12 {
            if o[a] < min[a] || o[a] > max[a] {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((min[a] - o[a]) / d[a], (max[a] - o[a]) / d[a]);
        let mut f = match a {
            0 => 4,
            1 => 0,
            _ => 2,
        };
        if near > far {
            std::mem::swap(&mut near, &mut far);
            f += 1;
        }
        if near > t0 {
            t0 = near;
            face = f;
        }
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, face))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_faces() {
        // Looking down at a unit box from above hits the top face (1).
        let (t, f) = ray_box(DVec3::new(0.5, 3.0, 0.5), DVec3::NEG_Y, DVec3::ZERO, DVec3::ONE).unwrap();
        assert!((t - 2.0).abs() < 1e-9);
        assert_eq!(f, 1);
        // Looking east (+X) from the west hits the west face (4).
        let (_, f) = ray_box(DVec3::new(-2.0, 0.5, 0.5), DVec3::X, DVec3::ZERO, DVec3::ONE).unwrap();
        assert_eq!(f, 4);
        // Looking north (−Z) from the south hits the south face (3).
        let (_, f) = ray_box(DVec3::new(0.5, 0.5, 3.0), DVec3::NEG_Z, DVec3::ZERO, DVec3::ONE).unwrap();
        assert_eq!(f, 3);
    }

    #[test]
    fn interactive_blocks() {
        assert!(block_interactive("chest"));
        assert!(block_interactive("oak_door"));
        assert!(!block_interactive("iron_door"));
        assert!(!block_interactive("stone"));
    }
}
