use super::config::{ClientInformation, ResourcePack, Tags, read_resource_pack, read_tags};
use super::{Encoded, ProfileProperty, enc, read_properties};
use crate::codec::{BlockPos, DResult, Reader};
use crate::ids::play::{cb, sb};
use crate::item::{DataValue, ItemStack, read_data_value};
use crate::nbt::Nbt;
use crate::text::Text;
use uuid::Uuid;

// ======================= clientbound =======================

/// Fields shared by `login` and `respawn`.
#[derive(Debug, Clone)]
pub struct SpawnInfo {
    pub dimension_type: i32,
    pub dimension_name: String,
    pub hashed_seed: i64,
    pub game_mode: i32,
    /// -1 when undefined
    pub previous_game_mode: i32,
    pub is_debug: bool,
    pub is_flat: bool,
    pub death_location: Option<(String, BlockPos)>,
    pub portal_cooldown: i32,
    pub sea_level: i32,
}

fn read_spawn_info(r: &mut Reader) -> DResult<SpawnInfo> {
    Ok(SpawnInfo {
        dimension_type: r.var_i32()?,
        dimension_name: r.identifier()?,
        hashed_seed: r.i64()?,
        game_mode: r.var_i32()?,
        previous_game_mode: r.var_i32()? - 1,
        is_debug: r.bool()?,
        is_flat: r.bool()?,
        death_location: r.option(|r| Ok((r.identifier()?, r.block_pos()?)))?,
        portal_cooldown: r.var_i32()?,
        sea_level: r.var_i32()?,
    })
}

#[derive(Debug, Clone)]
pub struct JoinGame {
    pub entity_id: i32,
    pub hardcore: bool,
    pub dimensions: Vec<String>,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub reduced_debug_info: bool,
    pub respawn_screen: bool,
    pub spawn: SpawnInfo,
    pub online_mode: bool,
    pub enforces_secure_chat: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LightData {
    pub sky_mask: Vec<u64>,
    pub block_mask: Vec<u64>,
    pub empty_sky_mask: Vec<u64>,
    pub empty_block_mask: Vec<u64>,
    pub sky: Vec<Vec<u8>>,
    pub block: Vec<Vec<u8>>,
}

fn read_light(r: &mut Reader) -> DResult<LightData> {
    Ok(LightData {
        sky_mask: r.bitset()?,
        block_mask: r.bitset()?,
        empty_sky_mask: r.bitset()?,
        empty_block_mask: r.bitset()?,
        sky: r.array(|r| r.byte_array())?,
        block: r.array(|r| r.byte_array())?,
    })
}

#[derive(Debug, Clone)]
pub struct BlockEntity {
    /// (x & 15) << 4 | (z & 15)
    pub packed_xz: u8,
    pub y: i16,
    pub kind: i32,
    pub data: Nbt,
}

#[derive(Debug, Clone)]
pub struct ChunkData {
    pub x: i32,
    pub z: i32,
    /// Raw section data; decoded by the world crate, which knows the dimension height.
    pub data: Vec<u8>,
    pub block_entities: Vec<BlockEntity>,
    pub light: LightData,
}

/// One step of an entity movement path (26.3).
#[derive(Debug, Clone, Copy)]
pub struct MoveStep {
    pub tick_offset: i32,
    pub delta: [f64; 3],
}

#[derive(Debug, Clone)]
pub enum ChatTypeRef {
    Registry(i32),
    Inline { translation_key: String, parameters: Vec<i32>, style: Nbt },
}

#[derive(Debug, Clone)]
pub struct PlayerChat {
    pub sender: Uuid,
    pub index: i32,
    pub signature: Option<Vec<u8>>,
    pub message: String,
    pub timestamp: i64,
    pub salt: i64,
    pub unsigned_content: Option<Text>,
    /// 0 pass-through, 1 fully filtered, 2 partially filtered
    pub filter_type: i32,
    pub chat_type: ChatTypeRef,
    pub sender_name: Text,
    pub target_name: Option<Text>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerInfoEntry {
    pub uuid: Uuid,
    pub name: Option<String>,
    pub properties: Option<Vec<ProfileProperty>>,
    pub game_mode: Option<i32>,
    pub listed: Option<bool>,
    pub latency: Option<i32>,
    pub display_name: Option<Option<Text>>,
    pub priority: Option<i32>,
    pub show_hat: Option<bool>,
}

/// What a boss bar's update is.
#[derive(Debug, Clone)]
pub enum BossOp {
    Add { name: Text, progress: f32, color: i32, overlay: i32, flags: u8 },
    Remove,
    Progress(f32),
    Name(Text),
    Style { color: i32, overlay: i32 },
    Flags(u8),
}

/// One point in the tree of commands.
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// 0 root, 1 a literal word, 2 an argument.
    pub kind: u8,
    pub children: Vec<i32>,
    pub redirect: Option<i32>,
    pub name: String,
    /// An argument that takes the rest of the line and gets signed.
    pub signed: bool,
    /// An argument that's several words, up to the end: it can't be split on spaces.
    pub greedy: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CommandTree {
    pub nodes: Vec<CommandNode>,
    pub root: i32,
}

impl CommandTree {
    /// The signed parts of a command: name and text. Walks the words the way the server would,
    /// and gives up quietly, with nothing signed, on anything it can't follow.
    pub fn signed_arguments(&self, command: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut at = self.root;
        let mut rest = command.trim_start();
        for _ in 0..64 {
            let Some(node) = self.nodes.get(at as usize) else { return out };
            if rest.is_empty() {
                return out;
            }
            let (word, after) = match rest.find(' ') {
                Some(i) => (&rest[..i], rest[i + 1..].trim_start()),
                None => (rest, ""),
            };
            let followed = node.redirect.map_or(node, |r| self.nodes.get(r as usize).unwrap_or(node));
            let list = &followed.children;
            let literal = list.iter().find(|c| self.nodes.get(**c as usize).is_some_and(|n| n.kind == 1 && n.name == word));
            if let Some(next) = literal {
                at = *next;
                rest = after;
                continue;
            }
            let Some(next) = list.iter().find(|c| self.nodes.get(**c as usize).is_some_and(|n| n.kind == 2)) else { return out };
            let arg = &self.nodes[*next as usize];
            if arg.signed {
                out.push((arg.name.clone(), rest.to_string()));
                return out;
            }
            if arg.greedy {
                return out;
            }
            // A selector's brackets can hold spaces.
            let taken = if rest.starts_with('@') && rest.as_bytes().get(2) == Some(&b'[') { rest.find(']').map_or(rest.len(), |i| i + 1) } else { word.len() };
            rest = rest[taken.min(rest.len())..].trim_start();
            at = *next;
        }
        out
    }
}

fn read_command_node(r: &mut Reader, parsers: &[String]) -> DResult<CommandNode> {
    let flags = r.u8()?;
    let kind = flags & 3;
    let children = r.array(|r| r.var_i32())?;
    let redirect = if flags & 8 != 0 { Some(r.var_i32()?) } else { None };
    let name = if kind == 1 || kind == 2 { r.string(32767)? } else { String::new() };
    let (mut signed, mut greedy) = (false, false);
    if kind == 2 {
        let parser = r.var_i32()?;
        let parser_name = parsers.get(parser as usize).map_or("", |s| s.as_str());
        match parser_name {
            "brigadier:float" => {
                let f = r.u8()?;
                if f & 1 != 0 {
                    r.f32()?;
                }
                if f & 2 != 0 {
                    r.f32()?;
                }
            }
            "brigadier:double" => {
                let f = r.u8()?;
                if f & 1 != 0 {
                    r.f64()?;
                }
                if f & 2 != 0 {
                    r.f64()?;
                }
            }
            "brigadier:integer" => {
                let f = r.u8()?;
                if f & 1 != 0 {
                    r.i32()?;
                }
                if f & 2 != 0 {
                    r.i32()?;
                }
            }
            "brigadier:long" => {
                let f = r.u8()?;
                if f & 1 != 0 {
                    r.i64()?;
                }
                if f & 2 != 0 {
                    r.i64()?;
                }
            }
            "brigadier:string" => greedy = r.var_i32()? == 2,
            "entity" | "score_holder" => {
                r.u8()?;
            }
            "time" => {
                r.i32()?;
            }
            "resource_or_tag" | "resource_or_tag_key" | "resource" | "resource_key" | "resource_selector" => {
                r.identifier()?;
            }
            "message" => {
                signed = true;
                greedy = true;
            }
            _ => {}
        }
    }
    if flags & 0x10 != 0 {
        r.identifier()?;
    }
    Ok(CommandNode { kind, children, redirect, name, signed, greedy })
}

/// How an advancement shows itself.
#[derive(Debug, Clone)]
pub struct AdvancementDisplay {
    pub title: Text,
    pub description: Text,
    pub icon: crate::ItemStack,
    /// 0 task, 1 challenge, 2 goal.
    pub kind: i32,
    pub background: Option<String>,
    pub show_toast: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub struct AdvancementEntry {
    pub id: String,
    pub parent: Option<String>,
    pub display: Option<AdvancementDisplay>,
    pub requirements: Vec<Vec<String>>,
    pub x: f32,
    pub y: f32,
}

/// How a score is written.
#[derive(Debug, Clone)]
pub enum NumberFormat {
    Blank,
    Styled(Nbt),
    Fixed(Text),
}

fn read_number_format(r: &mut Reader) -> DResult<NumberFormat> {
    Ok(match r.var_i32()? {
        0 => NumberFormat::Blank,
        1 => NumberFormat::Styled(r.nbt()?),
        _ => NumberFormat::Fixed(r.text()?),
    })
}

#[derive(Debug, Clone)]
pub struct TeamParams {
    pub display: Text,
    pub prefix: Text,
    pub suffix: Text,
    /// 0 always, 1 never, 2 hide for other teams, 3 hide for own team.
    pub name_visibility: i32,
    pub collision: i32,
    pub color: Option<i32>,
    pub options: u8,
}

#[derive(Debug, Clone)]
pub struct MapDecoration {
    pub kind: i32,
    pub x: i8,
    pub y: i8,
    pub rotation: i8,
    pub name: Option<Text>,
}

#[derive(Debug, Clone)]
pub struct MapPatch {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
    pub colors: Vec<u8>,
}

/// A sound as packets name it: by registry number, or spelled out.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundRef {
    Id(i32),
    Named(String),
}

fn read_sound_ref(r: &mut Reader) -> DResult<SoundRef> {
    let id = r.var_i32()?;
    if id == 0 {
        let name = r.identifier()?;
        r.option(|r| r.f32())?;
        Ok(SoundRef::Named(name))
    } else {
        Ok(SoundRef::Id(id - 1))
    }
}

#[derive(Debug, Clone)]
pub struct AttributeModifier {
    pub id: String,
    pub amount: f64,
    pub operation: u8,
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub id: i32,
    pub base: f64,
    pub modifiers: Vec<AttributeModifier>,
}

impl Attribute {
    /// What it comes to with all its modifiers.
    pub fn value(&self) -> f64 {
        let mut v = self.base;
        v += self.modifiers.iter().filter(|m| m.operation == 0).map(|m| m.amount).sum::<f64>();
        let base = v;
        v += base * self.modifiers.iter().filter(|m| m.operation == 1).map(|m| m.amount).sum::<f64>();
        for m in self.modifiers.iter().filter(|m| m.operation == 2) {
            v *= 1.0 + m.amount;
        }
        v
    }
}

#[derive(Debug, Clone)]
pub struct WorldClock {
    pub id: i32,
    pub time: i64,
    pub fraction: f32,
    pub rate: f32,
}

#[derive(Debug, Clone)]
pub struct AddEntity {
    pub id: i32,
    pub uuid: Uuid,
    pub kind: i32,
    pub pos: [f64; 3],
    pub velocity: [f64; 3],
    pub pitch: f32,
    pub yaw: f32,
    pub head_yaw: f32,
    pub data: i32,
}

/// A villager's offer: costs and result.
#[derive(Debug, Clone)]
pub struct MerchantOffer {
    pub cost_a: ItemStack,
    pub cost_b: Option<ItemStack>,
    pub result: Option<ItemStack>,
    pub out_of_stock: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}

fn read_item_cost(r: &mut Reader) -> DResult<ItemStack> {
    let item = r.var_i32()?;
    let count = r.var_i32()?;
    // Exact components the offered stack has to have, with their values.
    let added = r.array(|r| {
        let kind = r.var_i32()?;
        let start = r.position();
        crate::item::walk_component(kind, r)?;
        Ok(crate::item::Component { kind, raw: r.since(start).to_vec() })
    })?;
    Ok(ItemStack { item, count, added, removed: Vec::new() })
}

/// Reads `set_entity_data`, keeping what could be read if a value is unknown.
fn read_entity_data(r: &mut Reader) -> DResult<Vec<(u8, DataValue)>> {
    let mut out = Vec::new();
    loop {
        let index = r.u8()?;
        if index == 0xFF {
            return Ok(out);
        }
        let serializer = r.var_i32()?;
        let name = ENTITY_SERIALIZERS.get(serializer as usize).copied().ok_or(crate::codec::DecodeError::Invalid { field: "entity data serializer", value: serializer as i64 })?;
        out.push((index, read_data_value(r, name)?));
    }
}

/// Wire ids of entity data serializers, in the game's registration order.
pub const ENTITY_SERIALIZERS: [&str; 44] = [
    "BYTE",
    "INT",
    "LONG",
    "FLOAT",
    "STRING",
    "COMPONENT",
    "OPTIONAL_COMPONENT",
    "ITEM_STACK",
    "BOOLEAN",
    "ROTATIONS",
    "BLOCK_POS",
    "OPTIONAL_BLOCK_POS",
    "DIRECTION",
    "OPTIONAL_LIVING_ENTITY_REFERENCE",
    "BLOCK_STATE",
    "OPTIONAL_BLOCK_STATE",
    "PARTICLE",
    "PARTICLES",
    "VILLAGER_DATA",
    "OPTIONAL_UNSIGNED_INT",
    "POSE",
    "CAT_VARIANT",
    "CAT_SOUND_VARIANT",
    "COW_VARIANT",
    "COW_SOUND_VARIANT",
    "WOLF_VARIANT",
    "WOLF_SOUND_VARIANT",
    "FROG_VARIANT",
    "PIG_VARIANT",
    "PIG_SOUND_VARIANT",
    "CHICKEN_VARIANT",
    "CHICKEN_SOUND_VARIANT",
    "ZOMBIE_NAUTILUS_VARIANT",
    "OPTIONAL_GLOBAL_POS",
    "PAINTING_VARIANT",
    "SNIFFER_STATE",
    "ARMADILLO_STATE",
    "COPPER_GOLEM_STATE",
    "WEATHERING_COPPER_STATE",
    "VECTOR3",
    "QUATERNION",
    "RESOLVABLE_PROFILE",
    "HUMANOID_ARM",
    "DYE_COLOR",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WaypointId {
    Player(uuid::Uuid),
    Named(String),
}

/// Where a tracked point on the locator bar is, as exactly as the server will say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaypointPlace {
    Unknown,
    Block(i32, i32, i32),
    Chunk(i32, i32),
    /// Compass direction, radians.
    Azimuth(f32),
}

#[derive(Debug, Clone)]
pub enum Cb {
    GameRuleValues(Vec<(String, String)>),
    LowDiskSpace,
    TestInstanceStatus { status: Text, size: Option<[i32; 3]> },
    SelectAdvancementsTab(Option<String>),
    TagQuery { transaction: i32, data: Nbt },
    BorderWarningDelay(i32),
    BorderWarningDistance(i32),
    /// A message taken back: by its place among recent signatures, or the signature itself.
    DeleteChat { index: i32, signature: Option<Vec<u8>> },
    RecipeBookSettings([(bool, bool); 4]),
    AddTransientBlock { pos: BlockPos, state: i32 },
    BundleDelimiter,
    AddEntity(AddEntity),
    BlockChangedAck(i32),
    BlockDestruction { entity: i32, pos: BlockPos, stage: u8 },
    BlockUpdate { pos: BlockPos, state: i32 },
    ChunkBatchFinished(i32),
    ChunkBatchStart,
    ClearTitles { reset: bool },
    CombatDeath { player: i32, message: Text },
    Disconnect(Text),
    DisguisedChat { message: Text, chat_type: ChatTypeRef, sender_name: Text, target_name: Option<Text> },
    EntityPositionSync { id: i32, path: Vec<([f64; 3], i32)>, yaw: f32, pitch: f32, on_ground: bool },
    ForgetChunk { x: i32, z: i32 },
    GameEvent { event: u8, value: f32 },
    KeepAlive(i64),
    LevelChunk(Box<ChunkData>),
    LightUpdate { x: i32, z: i32, light: LightData },
    Login(Box<JoinGame>),
    MoveEntity { id: i32, steps: Vec<MoveStep>, rotation: Option<(f32, f32)>, on_ground: bool },
    MoveEntityRot { id: i32, yaw: f32, pitch: f32, on_ground: bool },
    Ping(i32),
    PlayerAbilities { flags: u8, flying_speed: f32, fov_modifier: f32 },
    PlayerChat(Box<PlayerChat>),
    PlayerInfoRemove(Vec<Uuid>),
    PlayerInfoUpdate { actions: u8, entries: Vec<PlayerInfoEntry> },
    PlayerPosition { teleport_id: i32, pos: [f64; 3], velocity: [f64; 3], yaw: f32, pitch: f32, flags: i32 },
    PlayerRotation { yaw: f32, relative_yaw: bool, pitch: f32, relative_pitch: bool },
    RemoveEntities(Vec<i32>),
    ResourcePackPop(Option<Uuid>),
    ResourcePackPush(ResourcePack),
    Respawn { spawn: SpawnInfo, data_kept: u8 },
    RotateHead { id: i32, head_yaw: f32 },
    SectionBlocksUpdate { section: [i32; 3], blocks: Vec<(u16, i32)> },
    SetActionBarText(Text),
    SetChunkCacheCenter { x: i32, z: i32 },
    SetChunkCacheRadius(i32),
    SetDefaultSpawnPosition { dimension: String, pos: BlockPos, yaw: f32, pitch: f32 },
    SetEntityMotion { id: i32, velocity: [f64; 3] },
    SetExperience { bar: f32, level: i32, total: i32 },
    SetHealth { health: f32, food: i32, saturation: f32 },
    SetHeldSlot(i32),
    SetSimulationDistance(i32),
    SetSubtitleText(Text),
    SetTime { world_age: i64, clocks: Vec<WorldClock> },
    SetTitleText(Text),
    SetTitlesAnimation { fade_in: i32, stay: i32, fade_out: i32 },
    StartConfiguration,
    SystemChat { content: Text, overlay: bool },
    TabList { header: Text, footer: Text },
    TeleportEntity { id: i32, pos: [f64; 3], velocity: [f64; 3], yaw: f32, pitch: f32, flags: i32, on_ground: bool },
    UpdateAttributes { entity: i32, attributes: Vec<Attribute> },
    UpdateTags(Tags),
    // Inventories
    ContainerClose(i32),
    ContainerSetContent { container: i32, state: i32, slots: Vec<Option<ItemStack>>, carried: Option<ItemStack> },
    ContainerSetData { container: i32, property: i16, value: i16 },
    ContainerSetSlot { container: i32, state: i32, slot: i16, item: Option<ItemStack> },
    Cooldown { group: String, ticks: i32 },
    MerchantOffers { container: i32, offers: Vec<MerchantOffer>, level: i32, xp: i32, show_progress: bool, can_restock: bool },
    MountScreenOpen { container: i32, columns: i32, entity: i32 },
    OpenBook { hand: i32 },
    OpenScreen { container: i32, menu: i32, title: Text },
    OpenSignEditor { pos: BlockPos, front: bool },
    SetCursorItem(Option<ItemStack>),
    SetPlayerInventory { slot: i32, item: Option<ItemStack> },
    // Entities
    Animate { id: i32, action: u8 },
    DamageEvent { id: i32, kind: i32, cause: Option<i32>, direct: Option<i32>, position: Option<[f64; 3]> },
    EntityEvent { id: i32, event: i8 },
    HurtAnimation { id: i32, yaw: f32 },
    RemoveMobEffect { id: i32, effect: i32 },
    SetEntityData { id: i32, values: Vec<(u8, DataValue)> },
    SetEntityLink { source: i32, dest: i32 },
    SetEquipment { id: i32, slots: Vec<(u8, Option<ItemStack>)> },
    SetPassengers { vehicle: i32, passengers: Vec<i32> },
    SwingAnimation { id: i32, hand: i32, kind: i32, duration: i32 },
    TakeItemEntity { item: i32, collector: i32, count: i32 },
    UpdateMobEffect { id: i32, effect: i32, amplifier: i32, duration: i32, flags: u8 },
    // World
    BlockEntityData { pos: BlockPos, kind: i32, data: Nbt },
    BlockEvent { pos: BlockPos, a: u8, b: u8, block: i32 },
    BossEvent { id: Uuid, op: BossOp },
    SetObjective { name: String, method: i32, details: Option<(Text, i32, Option<NumberFormat>)> },
    SetDisplayObjective { slot: i32, name: String },
    SetScore { owner: String, objective: String, score: i32, display: Option<Text>, format: Option<NumberFormat> },
    ResetScore { owner: String, objective: Option<String> },
    SetPlayerTeam { name: String, method: i32, params: Option<TeamParams>, players: Vec<String> },
    MapItemData { id: i32, scale: i32, locked: bool, decorations: Option<Vec<MapDecoration>>, patch: Option<MapPatch> },
    UpdateRecipes { cutter: Vec<crate::recipes::CutterChoice> },
    RecipeBookAdd { entries: Vec<(crate::recipes::RecipeEntry, u8)>, replace: bool },
    RecipeBookRemove(Vec<i32>),
    UpdateAdvancements { reset: bool, added: Vec<AdvancementEntry>, removed: Vec<String>, progress: Vec<(String, Vec<(String, Option<i64>)>)>, show: bool },
    /// Type, thing and count of each statistic asked for.
    AwardStats(Vec<(i32, i32, i32)>),
    CookieRequest(String),
    CustomChatCompletions { action: i32, entries: Vec<String> },
    /// A screen the server defines; `None` when it's one of its own by number.
    ShowDialog(Option<Nbt>),
    ClearDialog,
    ServerLinks(Vec<super::config::ServerLink>),
    /// 0 start tracking, 1 stop, 2 update.
    Waypoint { op: i32, id: WaypointId, color: Option<u32>, place: WaypointPlace },
    /// The tree of commands, for finding which parts of a command need signing.
    Commands(CommandTree),
    /// The ingredients aren't at hand, so here's where they would have gone.
    PlaceGhostRecipe { container: i32, display: crate::recipes::RecipeDisplay },
    CommandSuggestions { id: i32, start: i32, length: i32, entries: Vec<(String, Option<Text>)> },
    PlayerLookAt { from_eyes: bool, target: [f64; 3], entity: Option<(i32, bool)> },
    SetCamera(i32),
    TickingState { rate: f32, frozen: bool },
    TickingStep(i32),
    MoveVehicle { pos: [f64; 3], yaw: f32, pitch: f32 },
    ChangeDifficulty { difficulty: i32, locked: bool },
    ServerData { motd: Text },
    ChunksBiomes(Vec<(i32, i32, Vec<u8>)>),
    InitializeBorder { x: f64, z: f64, old_size: f64, new_size: f64, lerp_ms: i64, portal_limit: i32, warning_blocks: i32, warning_time: i32 },
    SetBorderCenter { x: f64, z: f64 },
    SetBorderLerpSize { old_size: f64, new_size: f64, lerp_ms: i64 },
    SetBorderSize(f64),
    LevelParticles { particle: crate::item::Particle, override_limiter: bool, always_show: bool, pos: [f64; 3], dist: [f32; 3], speed: [f32; 3], count: i32, kind: i32 },
    Explode { center: [f64; 3], radius: f32, block_count: i32, knockback: Option<[f64; 3]>, particle: crate::item::Particle, sound: SoundRef, play_sound: bool },
    Sound { sound: SoundRef, source: i32, pos: [f64; 3], volume: f32, pitch: f32, seed: i64 },
    SoundEntity { sound: SoundRef, source: i32, entity: i32, volume: f32, pitch: f32, seed: i64 },
    StopSound { source: Option<i32>, name: Option<String> },
    LevelEvent { event: i32, pos: BlockPos, data: i32, global: bool },
    Unknown(i32),
}

fn read_chat_type(r: &mut Reader) -> DResult<ChatTypeRef> {
    let id = r.var_i32()?;
    if id != 0 {
        return Ok(ChatTypeRef::Registry(id - 1));
    }
    let translation_key = r.string(32767)?;
    let parameters = r.array(|r| r.var_i32())?;
    let style = r.nbt()?;
    // Narration decoration: same layout, unused for display.
    r.string(32767)?;
    r.array(|r| r.var_i32())?;
    r.nbt()?;
    Ok(ChatTypeRef::Inline { translation_key, parameters, style })
}

fn read_move_steps(r: &mut Reader, properties: i32) -> DResult<Vec<MoveStep>> {
    const SCALE: f64 = 4096.0;
    let count = (properties as u32 >> 1) as usize;
    let delta = |r: &mut Reader| -> DResult<[f64; 3]> {
        Ok([r.i16()? as f64 / SCALE, r.i16()? as f64 / SCALE, r.i16()? as f64 / SCALE])
    };
    if count == 0 {
        return Ok(vec![MoveStep { tick_offset: 0, delta: delta(r)? }]);
    }
    let mut steps = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let tick_offset = r.var_i32()?;
        steps.push(MoveStep { tick_offset, delta: delta(r)? });
    }
    Ok(steps)
}

fn read_player_info(r: &mut Reader) -> DResult<Cb> {
    let actions = r.u8()?;
    let entries = r.array(|r| {
        let mut e = PlayerInfoEntry { uuid: r.uuid()?, ..Default::default() };
        if actions & 0x01 != 0 {
            e.name = Some(r.string(16)?);
            e.properties = Some(read_properties(r)?);
        }
        if actions & 0x02 != 0 {
            r.option(|r| {
                r.uuid()?;
                r.i64()?;
                r.byte_array()?;
                r.byte_array()?;
                Ok(())
            })?;
        }
        if actions & 0x04 != 0 {
            e.game_mode = Some(r.var_i32()?);
        }
        if actions & 0x08 != 0 {
            e.listed = Some(r.bool()?);
        }
        if actions & 0x10 != 0 {
            e.latency = Some(r.var_i32()?);
        }
        if actions & 0x20 != 0 {
            e.display_name = Some(r.option(|r| r.text())?);
        }
        if actions & 0x40 != 0 {
            e.priority = Some(r.var_i32()?);
        }
        if actions & 0x80 != 0 {
            e.show_hat = Some(r.bool()?);
        }
        Ok(e)
    })?;
    Ok(Cb::PlayerInfoUpdate { actions, entries })
}

pub fn decode(id: i32, r: &mut Reader) -> DResult<Cb> {
    Ok(match id {
        cb::BUNDLE_DELIMITER => Cb::BundleDelimiter,
        cb::ADD_ENTITY => Cb::AddEntity(AddEntity {
            id: r.var_i32()?,
            uuid: r.uuid()?,
            kind: r.var_i32()?,
            pos: [r.f64()?, r.f64()?, r.f64()?],
            velocity: r.lp_vec3()?,
            pitch: r.angle()?,
            yaw: r.angle()?,
            head_yaw: r.angle()?,
            data: r.var_i32()?,
        }),
        cb::BLOCK_CHANGED_ACK => Cb::BlockChangedAck(r.var_i32()?),
        cb::BLOCK_DESTRUCTION => Cb::BlockDestruction { entity: r.var_i32()?, pos: r.block_pos()?, stage: r.u8()? },
        cb::BLOCK_UPDATE => Cb::BlockUpdate { pos: r.block_pos()?, state: r.var_i32()? },
        cb::CHUNK_BATCH_FINISHED => Cb::ChunkBatchFinished(r.var_i32()?),
        cb::CHUNK_BATCH_START => Cb::ChunkBatchStart,
        cb::CLEAR_TITLES => Cb::ClearTitles { reset: r.bool()? },
        cb::PLAYER_COMBAT_KILL => Cb::CombatDeath { player: r.var_i32()?, message: r.text()? },
        cb::DISCONNECT => Cb::Disconnect(r.text()?),
        cb::DISGUISED_CHAT => Cb::DisguisedChat {
            message: r.text()?,
            chat_type: read_chat_type(r)?,
            sender_name: r.text()?,
            target_name: r.option(|r| r.text())?,
        },
        cb::ENTITY_POSITION_SYNC => {
            let id = r.var_i32()?;
            let path = match r.var_i32()? {
                0 => vec![([r.f64()?, r.f64()?, r.f64()?], 0)],
                1 => r.array(|r| Ok(([r.f64()?, r.f64()?, r.f64()?], r.var_i32()?)))?,
                v => return Err(crate::codec::DecodeError::Invalid { field: "position path type", value: v as i64 }),
            };
            Cb::EntityPositionSync { id, path, yaw: r.f32()?, pitch: r.f32()?, on_ground: r.bool()? }
        }
        cb::FORGET_LEVEL_CHUNK => {
            let z = r.i32()?;
            let x = r.i32()?;
            Cb::ForgetChunk { x, z }
        }
        cb::GAME_EVENT => Cb::GameEvent { event: r.u8()?, value: r.f32()? },
        cb::KEEP_ALIVE => Cb::KeepAlive(r.i64()?),
        cb::LEVEL_CHUNK_WITH_LIGHT => {
            let x = r.i32()?;
            let z = r.i32()?;
            // Heightmaps: the client recomputes what it needs.
            r.array(|r| {
                r.var_i32()?;
                r.array(|r| r.i64())?;
                Ok(())
            })?;
            let data = r.byte_array()?;
            let block_entities = r.array(|r| {
                Ok(BlockEntity { packed_xz: r.u8()?, y: r.i16()?, kind: r.var_i32()?, data: r.nbt()? })
            })?;
            let light = read_light(r)?;
            Cb::LevelChunk(Box::new(ChunkData { x, z, data, block_entities, light }))
        }
        cb::LIGHT_UPDATE => Cb::LightUpdate { x: r.var_i32()?, z: r.var_i32()?, light: read_light(r)? },
        cb::LOGIN => {
            let entity_id = r.i32()?;
            let hardcore = r.bool()?;
            let dimensions = r.array(|r| r.identifier())?;
            let _max_players = r.var_i32()?;
            let view_distance = r.var_i32()?;
            let simulation_distance = r.var_i32()?;
            let reduced_debug_info = r.bool()?;
            let respawn_screen = r.bool()?;
            let _limited_crafting = r.bool()?;
            let spawn = read_spawn_info(r)?;
            let online_mode = r.bool()?;
            let enforces_secure_chat = r.bool()?;
            Cb::Login(Box::new(JoinGame {
                entity_id,
                hardcore,
                dimensions,
                view_distance,
                simulation_distance,
                reduced_debug_info,
                respawn_screen,
                spawn,
                online_mode,
                enforces_secure_chat,
            }))
        }
        cb::MOVE_ENTITY_POS => {
            let id = r.var_i32()?;
            let props = r.var_i32()?;
            Cb::MoveEntity { id, steps: read_move_steps(r, props)?, rotation: None, on_ground: props & 1 != 0 }
        }
        cb::MOVE_ENTITY_POS_ROT => {
            let id = r.var_i32()?;
            let props = r.var_i32()?;
            let steps = read_move_steps(r, props)?;
            let yaw = r.angle()?;
            let pitch = r.angle()?;
            Cb::MoveEntity { id, steps, rotation: Some((yaw, pitch)), on_ground: props & 1 != 0 }
        }
        cb::MOVE_ENTITY_ROT => {
            let id = r.var_i32()?;
            let on_ground = r.bool()?;
            Cb::MoveEntityRot { id, on_ground, yaw: r.angle()?, pitch: r.angle()? }
        }
        cb::PING => Cb::Ping(r.i32()?),
        cb::PLAYER_ABILITIES => Cb::PlayerAbilities { flags: r.u8()?, flying_speed: r.f32()?, fov_modifier: r.f32()? },
        cb::PLAYER_CHAT => {
            let _global_index = r.var_i32()?;
            let sender = r.uuid()?;
            let index = r.var_i32()?;
            let signature = r.option(|r| Ok(r.take(256)?.to_vec()))?;
            let message = r.string(256)?;
            let timestamp = r.i64()?;
            let salt = r.i64()?;
            let previous = r.len()?;
            for _ in 0..previous {
                if r.var_i32()? == 0 {
                    r.take(256)?;
                }
            }
            let unsigned_content = r.option(|r| r.text())?;
            let filter_type = r.var_i32()?;
            if filter_type == 2 {
                r.bitset()?;
            }
            Cb::PlayerChat(Box::new(PlayerChat {
                sender,
                index,
                signature,
                message,
                timestamp,
                salt,
                unsigned_content,
                filter_type,
                chat_type: read_chat_type(r)?,
                sender_name: r.text()?,
                target_name: r.option(|r| r.text())?,
            }))
        }
        cb::PLAYER_INFO_REMOVE => Cb::PlayerInfoRemove(r.array(|r| r.uuid())?),
        cb::PLAYER_INFO_UPDATE => read_player_info(r)?,
        cb::PLAYER_POSITION => Cb::PlayerPosition {
            teleport_id: r.var_i32()?,
            pos: [r.f64()?, r.f64()?, r.f64()?],
            velocity: [r.f64()?, r.f64()?, r.f64()?],
            yaw: r.f32()?,
            pitch: r.f32()?,
            flags: r.i32()?,
        },
        cb::PLAYER_ROTATION => {
            Cb::PlayerRotation { yaw: r.f32()?, relative_yaw: r.bool()?, pitch: r.f32()?, relative_pitch: r.bool()? }
        }
        cb::REMOVE_ENTITIES => Cb::RemoveEntities(r.array(|r| r.var_i32())?),
        cb::RESOURCE_PACK_POP => Cb::ResourcePackPop(r.option(|r| r.uuid())?),
        cb::RESOURCE_PACK_PUSH => Cb::ResourcePackPush(read_resource_pack(r)?),
        cb::RESPAWN => Cb::Respawn { spawn: read_spawn_info(r)?, data_kept: r.u8()? },
        cb::ROTATE_HEAD => Cb::RotateHead { id: r.var_i32()?, head_yaw: r.angle()? },
        cb::SECTION_BLOCKS_UPDATE => {
            let p = r.i64()?;
            let section = [(p >> 42) as i32, (p << 44 >> 44) as i32, (p << 22 >> 42) as i32];
            let blocks = r.array(|r| {
                let v = r.var_i64()?;
                Ok(((v & 0xFFF) as u16, (v >> 12) as i32))
            })?;
            Cb::SectionBlocksUpdate { section, blocks }
        }
        cb::SET_ACTION_BAR_TEXT => Cb::SetActionBarText(r.text()?),
        cb::SET_CHUNK_CACHE_CENTER => Cb::SetChunkCacheCenter { x: r.var_i32()?, z: r.var_i32()? },
        cb::SET_CHUNK_CACHE_RADIUS => Cb::SetChunkCacheRadius(r.var_i32()?),
        cb::SET_DEFAULT_SPAWN_POSITION => Cb::SetDefaultSpawnPosition {
            dimension: r.identifier()?,
            pos: r.block_pos()?,
            yaw: r.f32()?,
            pitch: r.f32()?,
        },
        cb::SET_ENTITY_MOTION => Cb::SetEntityMotion { id: r.var_i32()?, velocity: r.lp_vec3()? },
        cb::SET_EXPERIENCE => Cb::SetExperience { bar: r.f32()?, level: r.var_i32()?, total: r.var_i32()? },
        cb::SET_HEALTH => Cb::SetHealth { health: r.f32()?, food: r.var_i32()?, saturation: r.f32()? },
        cb::SET_HELD_SLOT => Cb::SetHeldSlot(r.var_i32()?),
        cb::SET_SIMULATION_DISTANCE => Cb::SetSimulationDistance(r.var_i32()?),
        cb::SET_SUBTITLE_TEXT => Cb::SetSubtitleText(r.text()?),
        cb::SET_TIME => Cb::SetTime {
            world_age: r.i64()?,
            clocks: r.array(|r| {
                Ok(WorldClock { id: r.var_i32()?, time: r.var_i64()?, fraction: r.f32()?, rate: r.f32()? })
            })?,
        },
        cb::SET_TITLE_TEXT => Cb::SetTitleText(r.text()?),
        cb::SET_TITLES_ANIMATION => {
            Cb::SetTitlesAnimation { fade_in: r.i32()?, stay: r.i32()?, fade_out: r.i32()? }
        }
        cb::START_CONFIGURATION => Cb::StartConfiguration,
        cb::SYSTEM_CHAT => Cb::SystemChat { content: r.text()?, overlay: r.bool()? },
        cb::TAB_LIST => Cb::TabList { header: r.text()?, footer: r.text()? },
        cb::TELEPORT_ENTITY => Cb::TeleportEntity {
            id: r.var_i32()?,
            pos: [r.f64()?, r.f64()?, r.f64()?],
            velocity: [r.f64()?, r.f64()?, r.f64()?],
            yaw: r.f32()?,
            pitch: r.f32()?,
            flags: r.i32()?,
            on_ground: r.bool()?,
        },
        cb::UPDATE_ATTRIBUTES => Cb::UpdateAttributes {
            entity: r.var_i32()?,
            attributes: r.array(|r| {
                Ok(Attribute {
                    id: r.var_i32()?,
                    base: r.f64()?,
                    modifiers: r.array(|r| {
                        Ok(AttributeModifier { id: r.identifier()?, amount: r.f64()?, operation: r.u8()? })
                    })?,
                })
            })?,
        },
        cb::UPDATE_TAGS => Cb::UpdateTags(read_tags(r)?),
        cb::CONTAINER_CLOSE => Cb::ContainerClose(r.var_i32()?),
        cb::CONTAINER_SET_CONTENT => Cb::ContainerSetContent {
            container: r.var_i32()?,
            state: r.var_i32()?,
            slots: r.array(ItemStack::read)?,
            carried: ItemStack::read(r)?,
        },
        cb::CONTAINER_SET_DATA => Cb::ContainerSetData { container: r.var_i32()?, property: r.i16()?, value: r.i16()? },
        cb::CONTAINER_SET_SLOT => Cb::ContainerSetSlot { container: r.var_i32()?, state: r.var_i32()?, slot: r.i16()?, item: ItemStack::read(r)? },
        cb::COOLDOWN => Cb::Cooldown { group: r.identifier()?, ticks: r.var_i32()? },
        cb::MERCHANT_OFFERS => Cb::MerchantOffers {
            container: r.var_i32()?,
            offers: r.array(|r| {
                Ok(MerchantOffer {
                    cost_a: read_item_cost(r)?,
                    result: ItemStack::read(r)?,
                    cost_b: r.option(read_item_cost)?,
                    out_of_stock: r.bool()?,
                    uses: r.i32()?,
                    max_uses: r.i32()?,
                    xp: r.i32()?,
                    special_price: r.i32()?,
                    price_multiplier: r.f32()?,
                    demand: r.i32()?,
                })
            })?,
            level: r.var_i32()?,
            xp: r.var_i32()?,
            show_progress: r.bool()?,
            can_restock: r.bool()?,
        },
        cb::MOUNT_SCREEN_OPEN => Cb::MountScreenOpen { container: r.var_i32()?, columns: r.var_i32()?, entity: r.i32()? },
        cb::OPEN_BOOK => Cb::OpenBook { hand: r.var_i32()? },
        cb::OPEN_SCREEN => Cb::OpenScreen { container: r.var_i32()?, menu: r.var_i32()?, title: r.text()? },
        cb::OPEN_SIGN_EDITOR => Cb::OpenSignEditor { pos: r.block_pos()?, front: r.bool()? },
        cb::SET_CURSOR_ITEM => Cb::SetCursorItem(ItemStack::read(r)?),
        cb::SET_PLAYER_INVENTORY => Cb::SetPlayerInventory { slot: r.var_i32()?, item: ItemStack::read(r)? },
        cb::ANIMATE => Cb::Animate { id: r.var_i32()?, action: r.u8()? },
        cb::DAMAGE_EVENT => {
            let id = r.var_i32()?;
            let kind = r.var_i32()?;
            let optional_id = |r: &mut Reader| -> DResult<Option<i32>> {
                let v = r.var_i32()?;
                Ok((v != 0).then_some(v - 1))
            };
            Cb::DamageEvent { id, kind, cause: optional_id(r)?, direct: optional_id(r)?, position: r.option(|r| Ok([r.f64()?, r.f64()?, r.f64()?]))? }
        }
        cb::ENTITY_EVENT => Cb::EntityEvent { id: r.i32()?, event: r.i8()? },
        cb::HURT_ANIMATION => Cb::HurtAnimation { id: r.var_i32()?, yaw: r.f32()? },
        cb::REMOVE_MOB_EFFECT => Cb::RemoveMobEffect { id: r.var_i32()?, effect: r.var_i32()? },
        cb::SET_ENTITY_DATA => Cb::SetEntityData { id: r.var_i32()?, values: read_entity_data(r)? },
        cb::SET_ENTITY_LINK => Cb::SetEntityLink { source: r.i32()?, dest: r.i32()? },
        cb::SET_EQUIPMENT => {
            let id = r.var_i32()?;
            let mut slots = Vec::new();
            loop {
                let b = r.u8()?;
                slots.push((b & 0x7F, ItemStack::read(r)?));
                if b & 0x80 == 0 {
                    break;
                }
            }
            Cb::SetEquipment { id, slots }
        }
        cb::SET_PASSENGERS => Cb::SetPassengers { vehicle: r.var_i32()?, passengers: r.array(|r| r.var_i32())? },
        cb::SWING_ANIMATION => Cb::SwingAnimation { id: r.var_i32()?, hand: r.var_i32()?, kind: r.var_i32()?, duration: r.var_i32()? },
        cb::TAKE_ITEM_ENTITY => Cb::TakeItemEntity { item: r.var_i32()?, collector: r.var_i32()?, count: r.var_i32()? },
        cb::UPDATE_MOB_EFFECT => Cb::UpdateMobEffect { id: r.var_i32()?, effect: r.var_i32()?, amplifier: r.var_i32()?, duration: r.var_i32()?, flags: r.u8()? },
        cb::BLOCK_ENTITY_DATA => Cb::BlockEntityData { pos: r.block_pos()?, kind: r.var_i32()?, data: r.nbt()? },
        cb::BOSS_EVENT => {
            let id = r.uuid()?;
            let op = match r.var_i32()? {
                0 => BossOp::Add { name: r.text()?, progress: r.f32()?, color: r.var_i32()?, overlay: r.var_i32()?, flags: r.u8()? },
                1 => BossOp::Remove,
                2 => BossOp::Progress(r.f32()?),
                3 => BossOp::Name(r.text()?),
                4 => BossOp::Style { color: r.var_i32()?, overlay: r.var_i32()? },
                _ => BossOp::Flags(r.u8()?),
            };
            Cb::BossEvent { id, op }
        }
        cb::SET_OBJECTIVE => {
            let name = r.string(32767)?;
            let method = r.i8()? as i32;
            let details = if method == 0 || method == 2 {
                let display = r.text()?;
                let render = r.var_i32()?;
                let format = r.option(read_number_format)?;
                Some((display, render, format))
            } else {
                None
            };
            Cb::SetObjective { name, method, details }
        }
        cb::SET_DISPLAY_OBJECTIVE => Cb::SetDisplayObjective { slot: r.var_i32()?, name: r.string(32767)? },
        cb::SET_SCORE => Cb::SetScore {
            owner: r.string(32767)?,
            objective: r.string(32767)?,
            score: r.var_i32()?,
            display: r.option(|r| r.text())?,
            format: r.option(read_number_format)?,
        },
        cb::RESET_SCORE => Cb::ResetScore { owner: r.string(32767)?, objective: r.option(|r| r.string(32767))? },
        cb::SET_PLAYER_TEAM => {
            let name = r.string(32767)?;
            let method = r.i8()? as i32;
            let params = if method == 0 || method == 2 {
                let (display, prefix, suffix) = (r.text()?, r.text()?, r.text()?);
                let name_visibility = r.var_i32()?;
                let collision = r.var_i32()?;
                let color = r.option(|r| r.var_i32())?;
                let options = r.u8()?;
                Some(TeamParams { display, prefix, suffix, name_visibility, collision, color, options })
            } else {
                None
            };
            let players = if method == 0 || method == 3 || method == 4 { r.array(|r| r.string(32767))? } else { Vec::new() };
            Cb::SetPlayerTeam { name, method, params, players }
        }
        cb::MAP_ITEM_DATA => {
            let id = r.var_i32()?;
            let scale = r.i8()? as i32;
            let locked = r.bool()?;
            let decorations = r.option(|r| {
                r.array(|r| {
                    Ok(MapDecoration { kind: r.var_i32()?, x: r.i8()?, y: r.i8()?, rotation: r.i8()?, name: r.option(|r| r.text())? })
                })
            })?;
            let width = r.u8()?;
            let patch = if width > 0 {
                let height = r.u8()?;
                let x = r.u8()?;
                let y = r.u8()?;
                let n = r.var_i32()?.max(0) as usize;
                Some(MapPatch { x, y, width, height, colors: r.take(n)?.to_vec() })
            } else {
                None
            };
            Cb::MapItemData { id, scale, locked, decorations, patch }
        }
        cb::UPDATE_RECIPES => {
            // Which items fit furnaces and such, which the server checks anyway.
            for _ in 0..r.var_i32()? {
                r.identifier()?;
                for _ in 0..r.var_i32()? {
                    r.var_i32()?;
                }
            }
            let cutter = r.array(|r| {
                let input = crate::recipes::ItemSet::read(r)?;
                let output = crate::recipes::SlotDisplay::read(r)?;
                Ok(crate::recipes::CutterChoice { input, output })
            })?;
            Cb::UpdateRecipes { cutter }
        }
        cb::RECIPE_BOOK_ADD => {
            let entries = r.array(|r| {
                let e = crate::recipes::RecipeEntry::read(r)?;
                Ok((e, r.u8()?))
            })?;
            Cb::RecipeBookAdd { entries, replace: r.bool()? }
        }
        cb::RECIPE_BOOK_REMOVE => Cb::RecipeBookRemove(r.array(|r| r.var_i32())?),
        cb::COMMANDS => {
            let parsers = lattice_data::game().registry("command_argument_type");
            let nodes = r.array(|r| read_command_node(r, parsers))?;
            Cb::Commands(CommandTree { nodes, root: r.var_i32()? })
        }
        cb::UPDATE_ADVANCEMENTS => {
            let reset = r.bool()?;
            let added = r.array(|r| {
                let id = r.identifier()?;
                let parent = r.option(|r| r.identifier())?;
                let display = r.option(|r| {
                    let title = r.text()?;
                    let description = r.text()?;
                    let icon = crate::ItemStack::read_template(r)?;
                    let kind = r.var_i32()?;
                    let flags = r.i32()?;
                    let background = if flags & 1 != 0 { Some(r.identifier()?) } else { None };
                    Ok(AdvancementDisplay { title, description, icon, kind, background, show_toast: flags & 2 != 0, hidden: flags & 4 != 0 })
                })?;
                let requirements = r.array(|r| r.array(|r| r.string(32767)))?;
                let _telemetry = r.bool()?;
                let (x, y) = (r.f32()?, r.f32()?);
                Ok(AdvancementEntry { id, parent, display, requirements, x, y })
            })?;
            let removed = r.array(|r| r.identifier())?;
            let progress = r.array(|r| {
                let id = r.identifier()?;
                let criteria = r.array(|r| Ok((r.string(32767)?, r.option(|r| r.i64())?)))?;
                Ok((id, criteria))
            })?;
            Cb::UpdateAdvancements { reset, added, removed, progress, show: r.bool()? }
        }
        cb::AWARD_STATS => Cb::AwardStats(r.array(|r| Ok((r.var_i32()?, r.var_i32()?, r.var_i32()?)))?),
        cb::COOKIE_REQUEST => Cb::CookieRequest(r.identifier()?),
        cb::CUSTOM_CHAT_COMPLETIONS => Cb::CustomChatCompletions { action: r.var_i32()?, entries: r.array(|r| r.string(32767))? },
        cb::SHOW_DIALOG => {
            let holder = r.var_i32()?;
            Cb::ShowDialog(if holder == 0 { Some(r.nbt()?) } else { None })
        }
        cb::CLEAR_DIALOG => Cb::ClearDialog,
        cb::SERVER_LINKS => Cb::ServerLinks(super::config::read_server_links(r)?),
        cb::WAYPOINT => {
            let op = r.var_i32()?;
            let id = if r.bool()? { WaypointId::Player(r.uuid()?) } else { WaypointId::Named(r.string(32767)?) };
            let _style = r.identifier()?;
            let color = if r.bool()? {
                let (red, green, blue) = (r.u8()? as u32, r.u8()? as u32, r.u8()? as u32);
                Some(red << 16 | green << 8 | blue)
            } else {
                None
            };
            let place = match r.var_i32()? {
                1 => WaypointPlace::Block(r.var_i32()?, r.var_i32()?, r.var_i32()?),
                2 => WaypointPlace::Chunk(r.var_i32()?, r.var_i32()?),
                3 => WaypointPlace::Azimuth(r.f32()?),
                _ => WaypointPlace::Unknown,
            };
            Cb::Waypoint { op, id, color, place }
        }
        cb::PLACE_GHOST_RECIPE => Cb::PlaceGhostRecipe { container: r.var_i32()?, display: crate::recipes::RecipeDisplay::read(r)? },
        cb::COMMAND_SUGGESTIONS => {
            let (id, start, length) = (r.var_i32()?, r.var_i32()?, r.var_i32()?);
            let entries = r.array(|r| Ok((r.string(32767)?, r.option(|r| r.text())?)))?;
            Cb::CommandSuggestions { id, start, length, entries }
        }
        cb::PLAYER_LOOK_AT => {
            let from_eyes = r.var_i32()? == 1;
            let target = [r.f64()?, r.f64()?, r.f64()?];
            let entity = if r.bool()? { Some((r.var_i32()?, r.var_i32()? == 1)) } else { None };
            Cb::PlayerLookAt { from_eyes, target, entity }
        }
        cb::SET_CAMERA => Cb::SetCamera(r.var_i32()?),
        cb::GAME_RULE_VALUES => {
            let n = r.var_i32()?;
            let mut values = Vec::new();
            for _ in 0..n {
                values.push((r.identifier()?, r.string(32767)?));
            }
            Cb::GameRuleValues(values)
        }
        cb::LOW_DISK_SPACE_WARNING => Cb::LowDiskSpace,
        cb::TEST_INSTANCE_BLOCK_STATUS => {
            let status = r.text()?;
            let size = if r.bool()? { Some([r.var_i32()?, r.var_i32()?, r.var_i32()?]) } else { None };
            Cb::TestInstanceStatus { status, size }
        }
        cb::SELECT_ADVANCEMENTS_TAB => Cb::SelectAdvancementsTab(r.option(|r| r.identifier())?),
        cb::TAG_QUERY => Cb::TagQuery { transaction: r.var_i32()?, data: r.nbt()? },
        cb::SET_BORDER_WARNING_DELAY => Cb::BorderWarningDelay(r.var_i32()?),
        cb::SET_BORDER_WARNING_DISTANCE => Cb::BorderWarningDistance(r.var_i32()?),
        cb::DELETE_CHAT => {
            let packed = r.var_i32()? - 1;
            let signature = if packed < 0 { Some(r.take(256)?.to_vec()) } else { None };
            Cb::DeleteChat { index: packed, signature }
        }
        cb::RECIPE_BOOK_SETTINGS => {
            // Open and filtering, for each of the four books.
            let mut books = [(false, false); 4];
            for b in &mut books {
                *b = (r.bool()?, r.bool()?);
            }
            Cb::RecipeBookSettings(books)
        }
        cb::ADD_TRANSIENT_BLOCK => Cb::AddTransientBlock { pos: r.block_pos()?, state: r.var_i32()? },
        cb::TICKING_STATE => Cb::TickingState { rate: r.f32()?, frozen: r.bool()? },
        cb::TICKING_STEP => Cb::TickingStep(r.var_i32()?),
        cb::MOVE_VEHICLE => Cb::MoveVehicle { pos: [r.f64()?, r.f64()?, r.f64()?], yaw: r.f32()?, pitch: r.f32()? },
        cb::CHANGE_DIFFICULTY => Cb::ChangeDifficulty { difficulty: r.var_i32()?, locked: r.bool()? },
        cb::SERVER_DATA => {
            let motd = r.text()?;
            // The icon follows, as a whole PNG; nothing in a running game shows it.
            let _icon = r.option(|r| {
                let n = r.var_i32()?.max(0) as usize;
                Ok(r.take(n)?.len())
            })?;
            Cb::ServerData { motd }
        }
        cb::CHUNKS_BIOMES => Cb::ChunksBiomes(r.array(|r| {
            let packed = r.i64()?;
            let n = r.var_i32()?.max(0) as usize;
            Ok((packed as i32, (packed >> 32) as i32, r.take(n)?.to_vec()))
        })?),
        cb::INITIALIZE_BORDER => Cb::InitializeBorder {
            x: r.f64()?,
            z: r.f64()?,
            old_size: r.f64()?,
            new_size: r.f64()?,
            lerp_ms: r.var_i64()?,
            portal_limit: r.var_i32()?,
            warning_blocks: r.var_i32()?,
            warning_time: r.var_i32()?,
        },
        cb::SET_BORDER_CENTER => Cb::SetBorderCenter { x: r.f64()?, z: r.f64()? },
        cb::SET_BORDER_LERP_SIZE => Cb::SetBorderLerpSize { old_size: r.f64()?, new_size: r.f64()?, lerp_ms: r.var_i64()? },
        cb::SET_BORDER_SIZE => Cb::SetBorderSize(r.f64()?),
        cb::LEVEL_PARTICLES => {
            let names = lattice_data::game().registry("particle_type");
            let particle = crate::item::read_particle(r, names)?;
            let (override_limiter, always_show) = (r.bool()?, r.bool()?);
            let pos = [r.f64()?, r.f64()?, r.f64()?];
            let dist = [r.f32()?, r.f32()?, r.f32()?];
            let speed = [r.f32()?, r.f32()?, r.f32()?];
            let count = r.var_i32()?;
            let kind = r.var_i32()?;
            Cb::LevelParticles { particle, override_limiter, always_show, pos, dist, speed, count, kind }
        }
        cb::EXPLODE => {
            let names = lattice_data::game().registry("particle_type");
            let center = [r.f64()?, r.f64()?, r.f64()?];
            let radius = r.f32()?;
            let block_count = r.i32()?;
            let knockback = r.option(|r| Ok([r.f64()?, r.f64()?, r.f64()?]))?;
            let particle = crate::item::read_particle(r, names)?;
            let sound = read_sound_ref(r)?;
            // The list of block particles: particle, scaling, speed, weight.
            let n = r.var_i32()?;
            for _ in 0..n {
                crate::item::read_particle(r, names)?;
                r.f32()?;
                r.f32()?;
                r.var_i32()?;
            }
            let play_sound = r.bool()?;
            Cb::Explode { center, radius, block_count, knockback, particle, sound, play_sound }
        }
        cb::SOUND => {
            let sound = read_sound_ref(r)?;
            let source = r.var_i32()?;
            let (x, y, z) = (r.i32()?, r.i32()?, r.i32()?);
            Cb::Sound { sound, source, pos: [x as f64 / 8.0, y as f64 / 8.0, z as f64 / 8.0], volume: r.f32()?, pitch: r.f32()?, seed: r.i64()? }
        }
        cb::SOUND_ENTITY => Cb::SoundEntity { sound: read_sound_ref(r)?, source: r.var_i32()?, entity: r.var_i32()?, volume: r.f32()?, pitch: r.f32()?, seed: r.i64()? },
        cb::STOP_SOUND => {
            let flags = r.u8()?;
            let source = if flags & 1 != 0 { Some(r.var_i32()?) } else { None };
            let name = if flags & 2 != 0 { Some(r.identifier()?) } else { None };
            Cb::StopSound { source, name }
        }
        cb::BLOCK_EVENT => Cb::BlockEvent { pos: r.block_pos()?, a: r.u8()?, b: r.u8()?, block: r.var_i32()? },
        cb::LEVEL_EVENT => Cb::LevelEvent { event: r.i32()?, pos: r.block_pos()?, data: r.i32()?, global: r.bool()? },
        other => Cb::Unknown(other),
    })
}

// ======================= serverbound =======================

pub fn accept_teleportation(id: i32, pos: [f64; 3], yaw: f32, pitch: f32) -> Encoded {
    enc(sb::ACCEPT_TELEPORTATION, |w| {
        w.var_i32(id).f64(pos[0]).f64(pos[1]).f64(pos[2]).f32(yaw).f32(pitch);
    })
}

pub fn keep_alive(id: i64) -> Encoded {
    enc(sb::KEEP_ALIVE, |w| {
        w.i64(id);
    })
}

pub fn pong(id: i32) -> Encoded {
    enc(sb::PONG, |w| {
        w.i32(id);
    })
}

pub fn chat_command(command: &str) -> Encoded {
    enc(sb::CHAT_COMMAND, |w| {
        w.string(command);
    })
}

/// Unsigned chat message. `acknowledged` is the 20-bit last-seen bitset.
pub fn chat(message: &str, timestamp: i64, salt: i64, signature: Option<&[u8; 256]>, seen: i32, acknowledged: [u8; 3], checksum: u8) -> Encoded {
    enc(sb::CHAT, |w| {
        w.string(message).i64(timestamp).i64(salt);
        match signature {
            Some(sig) => {
                w.bool(true).bytes(sig);
            }
            None => {
                w.bool(false);
            }
        }
        w.var_i32(seen).bytes(&acknowledged).u8(checksum);
    })
}

/// A command with signed parts.
pub fn chat_command_signed(command: &str, timestamp: i64, salt: i64, signatures: &[(String, crate::signing::Signature)], seen: i32, acknowledged: [u8; 3], checksum: u8) -> Encoded {
    enc(sb::CHAT_COMMAND_SIGNED, |w| {
        w.string(command).i64(timestamp).i64(salt);
        w.var_i32(signatures.len() as i32);
        for (name, sig) in signatures {
            w.string(name).bytes(sig);
        }
        w.var_i32(seen).bytes(&acknowledged).u8(checksum);
    })
}

/// Tells the server which key this player signs with.
pub fn chat_session_update(session: Uuid, expires_millis: i64, public_key: &[u8], key_signature: &[u8]) -> Encoded {
    enc(sb::CHAT_SESSION_UPDATE, |w| {
        w.uuid(session).i64(expires_millis);
        w.var_i32(public_key.len() as i32).bytes(public_key);
        w.var_i32(key_signature.len() as i32).bytes(key_signature);
    })
}

pub fn chat_ack(count: i32) -> Encoded {
    enc(sb::CHAT_ACK, |w| {
        w.var_i32(count);
    })
}

pub fn chunk_batch_received(chunks_per_tick: f32) -> Encoded {
    enc(sb::CHUNK_BATCH_RECEIVED, |w| {
        w.f32(chunks_per_tick);
    })
}

/// 0 respawn, 1 request stats, 2 request game rules
/// There are no cookies here, for any key.
pub fn cookie_response_empty(key: &str) -> Encoded {
    enc(sb::COOKIE_RESPONSE, |w| {
        w.string(key).bool(false);
    })
}

/// A button on a server's own screen, with what it wants sent back.
pub fn custom_click_action(id: &str, payload: Option<&Nbt>) -> Encoded {
    enc(sb::CUSTOM_CLICK_ACTION, |w| {
        w.string(id);
        match payload {
            Some(n) => {
                w.bool(true);
                crate::nbt::write_network(w, n);
            }
            None => {
                w.bool(false);
            }
        }
    })
}

pub fn client_command(action: i32) -> Encoded {
    enc(sb::CLIENT_COMMAND, |w| {
        w.var_i32(action);
    })
}

pub fn client_tick_end() -> Encoded {
    enc(sb::CLIENT_TICK_END, |_| {})
}

pub fn client_information(info: &ClientInformation) -> Encoded {
    enc(sb::CLIENT_INFORMATION, |w| info.write(w))
}

pub fn configuration_acknowledged() -> Encoded {
    enc(sb::CONFIGURATION_ACKNOWLEDGED, |_| {})
}

pub fn custom_payload(channel: &str, data: &[u8]) -> Encoded {
    enc(sb::CUSTOM_PAYLOAD, |w| {
        w.string(channel).bytes(data);
    })
}

pub const ON_GROUND: u8 = 0x01;
pub const AGAINST_WALL: u8 = 0x02;

pub fn move_player_pos(pos: [f64; 3], flags: u8) -> Encoded {
    enc(sb::MOVE_PLAYER_POS, |w| {
        w.f64(pos[0]).f64(pos[1]).f64(pos[2]).u8(flags);
    })
}

pub fn move_player_pos_rot(pos: [f64; 3], yaw: f32, pitch: f32, flags: u8) -> Encoded {
    enc(sb::MOVE_PLAYER_POS_ROT, |w| {
        w.f64(pos[0]).f64(pos[1]).f64(pos[2]).f32(yaw).f32(pitch).u8(flags);
    })
}

pub fn move_player_rot(yaw: f32, pitch: f32, flags: u8) -> Encoded {
    enc(sb::MOVE_PLAYER_ROT, |w| {
        w.f32(yaw).f32(pitch).u8(flags);
    })
}

pub fn move_player_status_only(flags: u8) -> Encoded {
    enc(sb::MOVE_PLAYER_STATUS_ONLY, |w| {
        w.u8(flags);
    })
}

pub mod input {
    pub const FORWARD: u8 = 0x01;
    pub const BACKWARD: u8 = 0x02;
    pub const LEFT: u8 = 0x04;
    pub const RIGHT: u8 = 0x08;
    pub const JUMP: u8 = 0x10;
    pub const SNEAK: u8 = 0x20;
    pub const SPRINT: u8 = 0x40;
}

pub fn player_input(flags: u8) -> Encoded {
    enc(sb::PLAYER_INPUT, |w| {
        w.u8(flags);
    })
}

pub fn player_loaded() -> Encoded {
    enc(sb::PLAYER_LOADED, |_| {})
}

/// `ServerboundPlayerCommandPacket.Action`.
pub mod command {
    pub const STOP_SLEEPING: i32 = 0;
    pub const START_SPRINTING: i32 = 1;
    pub const STOP_SPRINTING: i32 = 2;
    pub const START_RIDING_JUMP: i32 = 3;
    pub const STOP_RIDING_JUMP: i32 = 4;
    pub const OPEN_INVENTORY: i32 = 5;
    pub const START_FALL_FLYING: i32 = 6;
}

pub fn player_command(entity_id: i32, action: i32) -> Encoded {
    player_command_data(entity_id, action, 0)
}

/// `data` is how charged a riding jump is, 0 to 100.
pub fn player_command_data(entity_id: i32, action: i32, data: i32) -> Encoded {
    enc(sb::PLAYER_COMMAND, |w| {
        w.var_i32(entity_id).var_i32(action).var_i32(data);
    })
}

pub fn set_carried_item(slot: i16) -> Encoded {
    enc(sb::SET_CARRIED_ITEM, |w| {
        w.i16(slot);
    })
}

/// `Player Action` statuses, in the order of the 26.3 server's
/// `ServerboundPlayerActionPacket.Action` enum (read from its bytecode; 26.x
/// inserted CHANGE_DESTROY_DIRECTION at index 1, shifting the rest).
pub mod action {
    pub const START_DESTROY: i32 = 0;
    pub const CHANGE_DESTROY_DIRECTION: i32 = 1;
    pub const ABORT_DESTROY: i32 = 2;
    pub const STOP_DESTROY: i32 = 3;
    pub const DROP_ALL_ITEMS: i32 = 4;
    pub const DROP_ITEM: i32 = 5;
    pub const RELEASE_USE_ITEM: i32 = 6;
    pub const SWAP_WITH_OFFHAND: i32 = 7;
    pub const STAB: i32 = 8;
}

/// See [`action`] for `status` values.
pub fn player_action(status: i32, pos: BlockPos, face: u8, sequence: i32) -> Encoded {
    enc(sb::PLAYER_ACTION, |w| {
        w.var_i32(status).block_pos(pos).u8(face).var_i32(sequence);
    })
}

#[allow(clippy::too_many_arguments)]
pub fn use_item_on(hand: i32, pos: BlockPos, face: i32, cursor: [f32; 3], inside_block: bool, sequence: i32) -> Encoded {
    enc(sb::USE_ITEM_ON, |w| {
        w.var_i32(hand)
            .block_pos(pos)
            .var_i32(face)
            .f32(cursor[0])
            .f32(cursor[1])
            .f32(cursor[2])
            .bool(inside_block)
            .bool(false)
            .var_i32(sequence);
    })
}

pub fn use_item(hand: i32, sequence: i32, yaw: f32, pitch: f32) -> Encoded {
    enc(sb::USE_ITEM, |w| {
        w.var_i32(hand).var_i32(sequence).f32(yaw).f32(pitch);
    })
}

/// The arm swing that goes with an attack.
pub fn punch() -> Encoded {
    enc(sb::PUNCH, |_| {})
}

pub fn attack(entity_id: i32) -> Encoded {
    enc(sb::ATTACK, |w| {
        w.var_i32(entity_id);
    })
}

/// Using something on an entity. `at` is the point hit, relative to the entity's position.
pub fn interact(entity_id: i32, hand: i32, at: [f64; 3], sneaking: bool) -> Encoded {
    enc(sb::INTERACT, |w| {
        w.var_i32(entity_id).var_i32(hand).lp_vec3(at).bool(sneaking);
    })
}

/// `ContainerInput`: how a slot was clicked.
pub mod click {
    pub const PICKUP: i32 = 0;
    pub const QUICK_MOVE: i32 = 1;
    pub const SWAP: i32 = 2;
    pub const CLONE: i32 = 3;
    pub const THROW: i32 = 4;
    pub const QUICK_CRAFT: i32 = 5;
    pub const PICKUP_ALL: i32 = 6;
}

/// A click in a menu, with the slots that changed as this client predicts them.
pub fn container_click(container: i32, state: i32, slot: i16, button: i8, input: i32, changed: &[(i16, Option<ItemStack>)], carried: Option<&ItemStack>) -> Encoded {
    enc(sb::CONTAINER_CLICK, |w| {
        w.var_i32(container).var_i32(state).i16(slot).i8(button).var_i32(input).var_i32(changed.len() as i32);
        for (s, item) in changed {
            w.i16(*s);
            ItemStack::write_hashed(w, item.as_ref());
        }
        ItemStack::write_hashed(w, carried);
    })
}

pub fn container_close(container: i32) -> Encoded {
    enc(sb::CONTAINER_CLOSE, |w| {
        w.var_i32(container);
    })
}

pub fn container_button_click(container: i32, button: i32) -> Encoded {
    enc(sb::CONTAINER_BUTTON_CLICK, |w| {
        w.var_i32(container).var_i32(button);
    })
}

pub fn set_creative_mode_slot(slot: i16, item: Option<&ItemStack>) -> Encoded {
    enc(sb::SET_CREATIVE_MODE_SLOT, |w| {
        w.i16(slot);
        ItemStack::write_delimited(w, item);
    })
}

pub fn pick_item_from_block(pos: BlockPos, include_data: bool) -> Encoded {
    enc(sb::PICK_ITEM_FROM_BLOCK, |w| {
        w.block_pos(pos).bool(include_data);
    })
}

pub fn pick_item_from_entity(entity_id: i32, include_data: bool) -> Encoded {
    enc(sb::PICK_ITEM_FROM_ENTITY, |w| {
        w.var_i32(entity_id).bool(include_data);
    })
}

pub fn player_abilities(flying: bool) -> Encoded {
    enc(sb::PLAYER_ABILITIES, |w| {
        w.u8(if flying { 2 } else { 0 });
    })
}

pub fn sign_update(pos: BlockPos, front: bool, lines: &[String; 4]) -> Encoded {
    enc(sb::SIGN_UPDATE, |w| {
        w.block_pos(pos);
        for l in lines {
            w.string(l);
        }
        w.bool(front);
    })
}

pub fn rename_item(name: &str) -> Encoded {
    enc(sb::RENAME_ITEM, |w| {
        w.string(name);
    })
}

pub fn select_trade(index: i32) -> Encoded {
    enc(sb::SELECT_TRADE, |w| {
        w.var_i32(index);
    })
}

pub fn edit_book(slot: i32, pages: &[String], title: Option<&str>) -> Encoded {
    enc(sb::EDIT_BOOK, |w| {
        w.var_i32(slot).var_i32(pages.len() as i32);
        for p in pages {
            w.string(p);
        }
        match title {
            Some(t) => {
                w.bool(true).string(t);
            }
            None => {
                w.bool(false);
            }
        }
    })
}

pub fn set_beacon(primary: Option<i32>, secondary: Option<i32>) -> Encoded {
    enc(sb::SET_BEACON, |w| {
        for e in [primary, secondary] {
            match e {
                Some(v) => {
                    w.bool(true).var_i32(v);
                }
                None => {
                    w.bool(false);
                }
            }
        }
    })
}

pub fn paddle_boat(left: bool, right: bool) -> Encoded {
    enc(sb::PADDLE_BOAT, |w| {
        w.bool(left).bool(right);
    })
}

pub fn move_vehicle(pos: [f64; 3], yaw: f32, pitch: f32, on_ground: bool) -> Encoded {
    enc(sb::MOVE_VEHICLE, |w| {
        w.f64(pos[0]).f64(pos[1]).f64(pos[2]).f32(yaw).f32(pitch).bool(on_ground);
    })
}

pub fn teleport_to_entity(id: Uuid) -> Encoded {
    enc(sb::TELEPORT_TO_ENTITY, |w| {
        w.uuid(id);
    })
}

pub fn place_recipe(container: i32, recipe: i32, use_max: bool) -> Encoded {
    enc(sb::PLACE_RECIPE, |w| {
        w.var_i32(container).var_i32(recipe).bool(use_max);
    })
}

pub fn command_suggestion(transaction: i32, text: &str) -> Encoded {
    enc(sb::COMMAND_SUGGESTION, |w| {
        w.var_i32(transaction).string(text);
    })
}

/// `mode`: 0 chain, 1 repeating, 2 impulse, the game's own order.
pub fn set_command_block(pos: BlockPos, command: &str, mode: i32, track_output: bool, conditional: bool, automatic: bool) -> Encoded {
    enc(sb::SET_COMMAND_BLOCK, |w| {
        w.block_pos(pos).string(command).var_i32(mode);
        w.u8(u8::from(track_output) | u8::from(conditional) << 1 | u8::from(automatic) << 2);
    })
}

pub fn set_command_minecart(entity: i32, command: &str, track_output: bool) -> Encoded {
    enc(sb::SET_COMMAND_MINECART, |w| {
        w.var_i32(entity).string(command).bool(track_output);
    })
}

/// What's asked of a structure block, field for field.
pub struct StructureBlock<'a> {
    pub pos: BlockPos,
    /// 0 just store these, 1 save the area, 2 load it, 3 find its size.
    pub action: i32,
    /// 0 save, 1 load, 2 corner, 3 data.
    pub mode: i32,
    pub name: &'a str,
    pub offset: [i8; 3],
    pub size: [i8; 3],
    /// 0 none, 1 left-right, 2 front-back.
    pub mirror: i32,
    /// 0 none, 1 90, 2 180, 3 270 degrees.
    pub rotation: i32,
    pub data: &'a str,
    pub integrity: f32,
    pub seed: i64,
    pub ignore_entities: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub strict: bool,
}

pub fn set_structure_block(s: &StructureBlock) -> Encoded {
    enc(sb::SET_STRUCTURE_BLOCK, |w| {
        w.block_pos(s.pos).var_i32(s.action).var_i32(s.mode).string(s.name);
        for v in s.offset.iter().chain(s.size.iter()) {
            w.i8(*v);
        }
        w.var_i32(s.mirror).var_i32(s.rotation).string(s.data).f32(s.integrity).var_i64(s.seed);
        w.u8(u8::from(s.ignore_entities) | u8::from(s.show_air) << 1 | u8::from(s.show_bounding_box) << 2 | u8::from(s.strict) << 3);
    })
}

#[allow(clippy::too_many_arguments)]
pub fn set_jigsaw_block(pos: BlockPos, name: &str, target: &str, pool: &str, final_state: &str, joint: &str, selection_priority: i32, placement_priority: i32) -> Encoded {
    enc(sb::SET_JIGSAW_BLOCK, |w| {
        w.block_pos(pos).string(name).string(target).string(pool).string(final_state).string(joint).var_i32(selection_priority).var_i32(placement_priority);
    })
}

pub fn jigsaw_generate(pos: BlockPos, levels: i32, keep_jigsaws: bool) -> Encoded {
    enc(sb::JIGSAW_GENERATE, |w| {
        w.block_pos(pos).var_i32(levels).bool(keep_jigsaws);
    })
}

/// `mode`: 0 start, 1 log, 2 fail, 3 accept.
pub fn set_test_block(pos: BlockPos, mode: i32, message: &str) -> Encoded {
    enc(sb::SET_TEST_BLOCK, |w| {
        w.block_pos(pos).var_i32(mode).string(message);
    })
}

/// `action`: 0 init, 1 query, 2 set, 3 reset, 4 save, 5 export, 6 run.
#[allow(clippy::too_many_arguments)]
pub fn test_instance_block_action(pos: BlockPos, action: i32, id: Option<&str>, size: [i32; 3], rotation: i32, ignore_entities: bool, status: i32) -> Encoded {
    enc(sb::TEST_INSTANCE_BLOCK_ACTION, |w| {
        w.block_pos(pos).var_i32(action);
        w.bool(id.is_some());
        if let Some(id) = id {
            w.string(id);
        }
        w.var_i32(size[0]).var_i32(size[1]).var_i32(size[2]).var_i32(rotation).bool(ignore_entities).var_i32(status);
        // Errors are the server's to report.
        w.bool(false);
    })
}

pub fn set_game_rules(entries: &[(String, String)]) -> Encoded {
    enc(sb::SET_GAME_RULE, |w| {
        w.var_i32(entries.len() as i32);
        for (rule, value) in entries {
            w.string(rule).string(value);
        }
    })
}

/// `mode`: 0 survival, 1 creative, 2 adventure, 3 spectator.
pub fn change_game_mode(mode: i32) -> Encoded {
    enc(sb::CHANGE_GAME_MODE, |w| {
        w.var_i32(mode);
    })
}

/// `difficulty`: 0 peaceful, 1 easy, 2 normal, 3 hard.
pub fn change_difficulty(difficulty: i32) -> Encoded {
    enc(sb::CHANGE_DIFFICULTY, |w| {
        w.var_i32(difficulty);
    })
}

pub fn lock_difficulty(locked: bool) -> Encoded {
    enc(sb::LOCK_DIFFICULTY, |w| {
        w.bool(locked);
    })
}

/// Tells the server which advancement page is open, or that the screen closed.
pub fn seen_advancements(tab: Option<&str>) -> Encoded {
    enc(sb::SEEN_ADVANCEMENTS, |w| match tab {
        Some(tab) => {
            w.var_i32(0).string(tab);
        }
        None => {
            w.var_i32(1);
        }
    })
}

pub fn recipe_book_seen(recipe: i32) -> Encoded {
    enc(sb::RECIPE_BOOK_SEEN_RECIPE, |w| {
        w.var_i32(recipe);
    })
}

/// `book`: 0 crafting, 1 furnace, 2 blast furnace, 3 smoker.
pub fn recipe_book_settings(book: i32, open: bool, filtering: bool) -> Encoded {
    enc(sb::RECIPE_BOOK_CHANGE_SETTINGS, |w| {
        w.var_i32(book).bool(open).bool(filtering);
    })
}

/// Asks what's stored in a block, for copying it. The answer comes back under `transaction`.
pub fn block_entity_tag_query(transaction: i32, pos: BlockPos) -> Encoded {
    enc(sb::BLOCK_ENTITY_TAG_QUERY, |w| {
        w.var_i32(transaction).block_pos(pos);
    })
}

pub fn entity_tag_query(transaction: i32, entity: i32) -> Encoded {
    enc(sb::ENTITY_TAG_QUERY, |w| {
        w.var_i32(transaction).var_i32(entity);
    })
}

/// A spectator walks through a mob's eyes: the entity to follow, or none to leave it.
pub fn spectator_action(entity: Option<i32>) -> Encoded {
    enc(sb::SPECTATOR_ACTION, |w| {
        w.var_i32(entity.map_or(0, |e| e + 1));
    })
}

pub fn container_slot_state_changed(slot: i32, container: i32, enabled: bool) -> Encoded {
    enc(sb::CONTAINER_SLOT_STATE_CHANGED, |w| {
        w.var_i32(slot).var_i32(container).bool(enabled);
    })
}

pub fn bundle_item_selected(slot: i32, index: i32) -> Encoded {
    enc(sb::BUNDLE_ITEM_SELECTED, |w| {
        w.var_i32(slot).var_i32(index);
    })
}

pub fn resource_pack_response(uuid: Uuid, result: i32) -> Encoded {
    enc(sb::RESOURCE_PACK, |w| {
        w.uuid(uuid).var_i32(result);
    })
}

#[cfg(test)]
mod command_tree_tests {
    use super::*;

    fn node(kind: u8, name: &str, children: &[i32], signed: bool, greedy: bool) -> CommandNode {
        CommandNode { kind, children: children.to_vec(), redirect: None, name: name.into(), signed, greedy }
    }

    #[test]
    fn finds_what_needs_signing() {
        let tree = CommandTree {
            root: 0,
            nodes: vec![
                node(0, "", &[1, 3, 6], false, false),
                node(1, "say", &[2], false, false),
                node(2, "message", &[], true, true),
                node(1, "msg", &[4], false, false),
                node(2, "targets", &[5], false, false),
                node(2, "message", &[], true, true),
                node(1, "time", &[7], false, false),
                node(1, "set", &[8], false, false),
                node(2, "value", &[], false, false),
            ],
        };
        assert_eq!(tree.signed_arguments("say hello there"), vec![("message".to_string(), "hello there".to_string())]);
        assert_eq!(tree.signed_arguments("msg Bob hi you"), vec![("message".to_string(), "hi you".to_string())]);
        assert_eq!(tree.signed_arguments("msg @a[name=\"x y\"] hi"), vec![("message".to_string(), "hi".to_string())]);
        assert!(tree.signed_arguments("time set day").is_empty());
        assert!(tree.signed_arguments("nonsense word").is_empty());
        assert!(tree.signed_arguments("say").is_empty());
    }
}
