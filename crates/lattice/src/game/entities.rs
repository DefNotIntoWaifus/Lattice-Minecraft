//! Other entities: positions from the server, interpolated for drawing, plus their
//! synced data, equipment and passengers.

use glam::DVec3;
use lattice_proto::packets::play::Cb;
use lattice_proto::{DataValue, ItemStack};
use std::collections::HashMap;
use std::sync::OnceLock;
use uuid::Uuid;

pub struct Entity {
    pub kind: i32,
    pub uuid: Uuid,
    /// Authoritative position from the server.
    pub server_pos: DVec3,
    /// Smoothed position used for drawing.
    pub pos: DVec3,
    pub prev_pos: DVec3,
    pub velocity: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub prev_yaw: f32,
    pub prev_pitch: f32,
    /// Which way the body faces. Only the head's direction is sent, so this follows from movement.
    pub body_yaw: f32,
    pub prev_body_yaw: f32,
    pub head_yaw: f32,
    pub prev_head_yaw: f32,
    /// The object data sent when it was added (a falling block's state, say).
    pub object_data: i32,
    pub on_ground: bool,
    pub data: HashMap<u8, DataValue>,
    pub equipment: [Option<ItemStack>; 8],
    pub passengers: Vec<i32>,
    pub vehicle: Option<i32>,
    /// Where the server last said it's facing; the shown angles ease towards these.
    target_yaw: f32,
    target_pitch: f32,
    target_head: f32,
    turn_ticks: u8,
    head_ticks: u8,
    /// Where a player's cape is catching up from.
    pub cloak: DVec3,
    pub prev_cloak: DVec3,
    /// What it's tied to with a lead.
    pub leashed_to: Option<i32>,
    /// Latest values of its attributes that the server shares.
    pub attrs: HashMap<String, f64>,
    /// How far it moved while being steered from here, since the last tick.
    driven_moved: DVec3,
    /// Ticks since it was hurt, for the red flash.
    pub hurt_ticks: u32,
    /// Ticks since it started dying.
    pub death_ticks: u32,
    pub swing: super::swing::Swing,
    /// Distance walked, drives the leg animation.
    pub walk_position: f32,
    pub walk_speed: f32,
    pub prev_walk_speed: f32,
    pub age: u32,
    /// In the tab list as a spectator, so the crosshair passes through.
    pub spectator_hint: bool,
    /// Ticks spent using an item.
    pub use_ticks: u32,
    pub in_water: bool,
    /// A squid's tilt, degrees.
    pub body_pitch: f32,
    pub prev_body_pitch: f32,
    /// Fuse of a creeper.
    swell: i32,
    prev_swell: i32,
    /// Bounce of a slime.
    squish: f32,
    prev_squish: f32,
    was_on_ground: bool,
    /// Ticks left of an iron golem's, hoglin's or ravager's attack.
    attack_left: i32,
    lerp_ticks: u32,
}

/// Field names without the game's decorations, so `DATA_HEALTH_ID` and `HEALTH` both work.
fn field_key(name: &str) -> &str {
    let n = name.strip_prefix("DATA_").unwrap_or(name);
    n.strip_suffix("_ID").unwrap_or(n)
}

/// Field indices of entity classes, from the game's data definitions.
fn class_fields() -> &'static HashMap<String, HashMap<String, u8>> {
    static FIELDS: OnceLock<HashMap<String, HashMap<String, u8>>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        let data = lattice_data::game();
        let mut out: HashMap<String, HashMap<String, u8>> = HashMap::new();
        for name in data.entity_classes.keys() {
            let mut fields = HashMap::new();
            let mut at = Some(name.clone());
            while let Some(c) = at {
                let Some(class) = data.entity_classes.get(&c) else { break };
                for f in &class.fields {
                    fields.entry(field_key(&f.2).to_string()).or_insert(f.0);
                }
                at = class.parent.clone();
            }
            out.insert(name.clone(), fields);
        }
        out
    })
}

impl Entity {
    pub fn info(&self) -> Option<&'static lattice_data::EntityInfo> {
        lattice_data::game().entity(self.kind)
    }

    pub fn type_name(&self) -> &'static str {
        self.info().map_or("unknown", |i| i.name.as_str())
    }

    /// A synced value by the game's field name, such as `DATA_BABY_ID`.
    pub fn field(&self, name: &str) -> Option<&DataValue> {
        let class = self.info()?.class.as_deref()?;
        let index = class_fields().get(class)?.get(field_key(name))?;
        self.data.get(index)
    }

    pub fn bool_field(&self, name: &str) -> bool {
        matches!(self.field(name), Some(DataValue::Bool(true)))
    }

    pub fn int_field(&self, name: &str) -> Option<i32> {
        match self.field(name)? {
            DataValue::Int(v) | DataValue::Variant(v) | DataValue::Enum(v) | DataValue::Pose(v) | DataValue::Direction(v) | DataValue::BlockState(v) => Some(*v),
            DataValue::Byte(v) => Some(*v as i32),
            _ => None,
        }
    }

    pub fn float_field(&self, name: &str) -> Option<f32> {
        match self.field(name)? {
            DataValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Bits of the shared flags byte: 0 on fire, 1 sneaking, 3 sprinting, 4 swimming, 5 invisible, 6 glowing, 7 gliding.
    pub fn flag(&self, bit: u8) -> bool {
        matches!(self.data.get(&0), Some(DataValue::Byte(v)) if v & (1 << bit) != 0)
    }

    /// How swollen a creeper is, 0..1.
    pub fn swell_at(&self, partial: f32) -> f32 {
        (self.prev_swell as f32 + (self.swell - self.prev_swell) as f32 * partial) / 28.0
    }

    pub fn squish_at(&self, partial: f32) -> f32 {
        self.prev_squish + (self.squish - self.prev_squish) * partial
    }

    /// Ticks left of a melee attack's animation.
    pub fn attack_ticks(&self, partial: f32) -> f32 {
        if self.attack_left > 0 { self.attack_left as f32 - partial } else { 0.0 }
    }

    fn turn_to(&mut self, yaw: f32, pitch: f32) {
        self.target_yaw = yaw;
        self.target_pitch = pitch;
        self.turn_ticks = 3;
    }

    pub fn is_baby(&self) -> bool {
        self.bool_field("DATA_BABY_ID")
    }

    pub fn long_field(&self, name: &str) -> Option<i64> {
        match self.field(name)? {
            DataValue::Long(v) => Some(*v),
            _ => None,
        }
    }

    /// One of the animals, which sit differently in a boat.
    pub fn is_animal(&self) -> bool {
        let Some(mut class) = self.info().and_then(|i| i.class.clone()) else { return false };
        let classes = &lattice_data::game().entity_classes;
        loop {
            if class == "Animal" {
                return true;
            }
            match classes.get(&class).and_then(|c| c.parent.clone()) {
                Some(p) => class = p,
                None => return false,
            }
        }
    }

    pub fn is_dead(&self) -> bool {
        self.float_field("DATA_HEALTH_ID").is_some_and(|h| h <= 0.0)
    }

    /// The game's pose names by index.
    pub fn pose(&self) -> i32 {
        match self.data.get(&6) {
            Some(DataValue::Pose(p)) => *p,
            _ => 0,
        }
    }

    /// Width and height of its box right now.
    pub fn dimensions(&self) -> (f64, f64) {
        let Some(info) = self.info() else { return (0.6, 1.8) };
        let (mut w, mut h) = (info.width as f64, info.height as f64);
        let name = info.name.as_str();
        match name {
            "player" => match self.pose() {
                1 | 3 | 4 => (w, h) = (0.6, 0.6), // gliding, swimming, spin attack
                5 => (w, h) = (0.6, 1.5),         // crouching
                2 => (w, h) = (0.2, 0.2),         // sleeping
                _ => {}
            },
            "slime" | "magma_cube" => {
                let size = self.int_field("ID_SIZE").unwrap_or(1) as f64;
                let s = 0.255 * size;
                (w, h) = (w * s / 0.52, h * s / 0.52);
            }
            "armor_stand" => {
                let flags = self.int_field("DATA_CLIENT_FLAGS").unwrap_or(0);
                if flags & 16 != 0 {
                    (w, h) = (0.0, 0.0);
                } else if flags & 1 != 0 {
                    (w, h) = (w * 0.5, h * 0.5);
                }
            }
            "interaction" => {
                (w, h) = (self.float_field("DATA_WIDTH_ID").unwrap_or(1.0) as f64, self.float_field("DATA_HEIGHT_ID").unwrap_or(1.0) as f64);
            }
            _ => {
                if self.is_baby() {
                    (w, h) = (w * 0.5, h * 0.5);
                }
            }
        }
        (w, h)
    }

    /// Can the crosshair land on it?
    pub fn is_pickable(&self) -> bool {
        if self.is_dead() {
            return false;
        }
        if self.type_name() == "player" && self.spectator_hint {
            return false;
        }
        !matches!(
            self.type_name(),
            "item"
                | "experience_orb"
                | "arrow"
                | "spectral_arrow"
                | "trident"
                | "area_effect_cloud"
                | "lightning_bolt"
                | "marker"
                | "block_display"
                | "item_display"
                | "text_display"
                | "firework_rocket"
                | "llama_spit"
                | "evoker_fangs"
                | "eye_of_ender"
                | "fishing_bobber"
                | "experience_bottle"
                | "egg"
                | "snowball"
                | "ender_pearl"
                | "splash_potion"
                | "lingering_potion"
                | "wither_skull"
                | "shulker_bullet"
                | "dragon_fireball"
                | "wind_charge"
                | "breeze_wind_charge"
                | "falling_block"
                | "ominous_item_spawner"
        )
    }
}

/// A direction's unit vector, numbered down, up, north, south, west, east.
pub fn direction_step(d: i32) -> DVec3 {
    match d {
        0 => DVec3::NEG_Y,
        1 => DVec3::Y,
        2 => DVec3::NEG_Z,
        3 => DVec3::Z,
        4 => DVec3::NEG_X,
        _ => DVec3::X,
    }
}

fn wrap(mut d: f32) -> f32 {
    d %= 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

#[derive(Default)]
pub struct Entities {
    /// What's tied to what can be heard of before the thing itself: kept till it arrives.
    pub map: HashMap<i32, Entity>,
    /// The vehicle this client is steering, whose position it decides itself.
    pub driven: Option<i32>,
    /// Things that happened to entities that show as particles: id and what.
    pub events: Vec<(i32, i32)>,
}

impl Entities {
    pub fn clear(&mut self) {
        self.map.clear();
    }

    fn moved_to(&mut self, id: i32, target: DVec3, ticks: u32) {
        if let Some(e) = self.map.get_mut(&id) {
            e.server_pos = target;
            e.lerp_ticks = ticks.max(1);
        }
    }

    /// Applies an entity packet. Returns it if it wasn't about entities.
    pub fn handle(&mut self, p: Cb, self_id: i32) {
        match p {
            Cb::AddEntity(a) if a.id != self_id => {
                let mut pos = DVec3::from(a.pos);
                // Things hung on walls arrive as the block they're in, with their facing as data.
                let name = lattice_data::game().entity(a.kind).map_or("", |i| i.name.as_str());
                if matches!(name, "painting" | "item_frame" | "glow_item_frame") {
                    let step = direction_step(a.data);
                    pos = pos.floor() + DVec3::splat(0.5) - step * 0.46875;
                } else if name == "leash_knot" {
                    pos = pos.floor() + DVec3::new(0.5, 0.375, 0.5);
                }
                self.map.insert(
                    a.id,
                    Entity {
                        kind: a.kind,
                        uuid: a.uuid,
                        server_pos: pos,
                        pos,
                        prev_pos: pos,
                        velocity: DVec3::from(a.velocity),
                        yaw: a.yaw,
                        pitch: a.pitch,
                        prev_yaw: a.yaw,
                        prev_pitch: a.pitch,
                        body_yaw: a.head_yaw,
                        prev_body_yaw: a.head_yaw,
                        head_yaw: a.head_yaw,
                        prev_head_yaw: a.head_yaw,
                        object_data: a.data,
                        on_ground: false,
                        data: HashMap::new(),
                        equipment: Default::default(),
                        passengers: Vec::new(),
                        vehicle: None,
                        target_yaw: a.yaw,
                        target_pitch: a.pitch,
                        target_head: a.head_yaw,
                        turn_ticks: 0,
                        head_ticks: 0,
                        cloak: DVec3::from(a.pos),
                        prev_cloak: DVec3::from(a.pos),
                        leashed_to: None,
                        attrs: HashMap::new(),
                        driven_moved: DVec3::ZERO,
                        hurt_ticks: u32::MAX,
                        death_ticks: 0,
                        swing: Default::default(),
                        walk_position: 0.0,
                        walk_speed: 0.0,
                        prev_walk_speed: 0.0,
                        age: 0,
                        lerp_ticks: 0,
                        spectator_hint: false,
                        use_ticks: 0,
                        in_water: false,
                        body_pitch: 0.0,
                        prev_body_pitch: 0.0,
                        swell: 0,
                        prev_swell: 0,
                        squish: 0.0,
                        prev_squish: 0.0,
                        was_on_ground: false,
                        attack_left: 0,
                    },
                );
            }
            Cb::RemoveEntities(ids) => {
                for id in ids {
                    self.map.remove(&id);
                }
            }
            // The vehicle being steered here goes where this client says; word of where it
            // used to be only arrives late.
            Cb::MoveEntity { id, .. } | Cb::MoveEntityRot { id, .. } | Cb::EntityPositionSync { id, .. } | Cb::TeleportEntity { id, .. } if self.driven == Some(id) => {}
            Cb::UpdateAttributes { entity, attributes } => {
                if let Some(e) = self.map.get_mut(&entity) {
                    let names = lattice_data::game().registry("attribute");
                    for a in attributes {
                        if let Some(name) = names.get(a.id as usize) {
                            e.attrs.insert(name.clone(), a.value());
                        }
                    }
                }
            }
            Cb::MoveEntity { id, steps, rotation, on_ground } => {
                let Some(e) = self.map.get_mut(&id) else { return };
                let mut target = e.server_pos;
                let mut ticks = 3;
                for s in &steps {
                    target += DVec3::from(s.delta);
                    ticks = ticks.max(s.tick_offset as u32 + 1);
                }
                if let Some((yaw, pitch)) = rotation {
                    e.turn_to(yaw, pitch);
                }
                e.on_ground = on_ground;
                self.moved_to(id, target, ticks);
            }
            Cb::MoveEntityRot { id, yaw, pitch, on_ground } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.turn_to(yaw, pitch);
                    e.on_ground = on_ground;
                }
            }
            Cb::EntityPositionSync { id, path, yaw, pitch, on_ground } => {
                if let Some(&(last, offset)) = path.last() {
                    if let Some(e) = self.map.get_mut(&id) {
                        e.turn_to(yaw, pitch);
                        e.on_ground = on_ground;
                    }
                    self.moved_to(id, DVec3::from(last), offset as u32 + 3);
                }
            }
            Cb::TeleportEntity { id, pos, yaw, pitch, flags, on_ground, .. } => {
                if let Some(e) = self.map.get_mut(&id) {
                    let rel = |b: i32| flags & b != 0;
                    let p = DVec3::new(
                        if rel(1) { e.server_pos.x + pos[0] } else { pos[0] },
                        if rel(2) { e.server_pos.y + pos[1] } else { pos[1] },
                        if rel(4) { e.server_pos.z + pos[2] } else { pos[2] },
                    );
                    let (new_yaw, new_pitch) = (if rel(8) { e.target_yaw + yaw } else { yaw }, if rel(16) { e.target_pitch + pitch } else { pitch });
                    e.turn_to(new_yaw, new_pitch);
                    e.server_pos = p;
                    e.on_ground = on_ground;
                    e.lerp_ticks = 1;
                }
            }
            Cb::RotateHead { id, head_yaw } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.target_head = head_yaw;
                    e.head_ticks = 3;
                }
            }
            Cb::SetEntityMotion { id, velocity } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.velocity = DVec3::from(velocity);
                }
            }
            Cb::SetEntityData { id, values } => {
                if let Some(e) = self.map.get_mut(&id) {
                    for (index, value) in values {
                        e.data.insert(index, value);
                    }
                }
            }
            Cb::SetEquipment { id, slots } => {
                if let Some(e) = self.map.get_mut(&id) {
                    for (slot, item) in slots {
                        if let Some(s) = e.equipment.get_mut(slot as usize) {
                            *s = item;
                        }
                    }
                }
            }
            Cb::SetPassengers { vehicle, passengers } => {
                if let Some(e) = self.map.get_mut(&vehicle) {
                    for old in std::mem::take(&mut e.passengers) {
                        if let Some(p) = self.map.get_mut(&old) {
                            p.vehicle = None;
                        }
                    }
                }
                for &p in &passengers {
                    if let Some(e) = self.map.get_mut(&p) {
                        e.vehicle = Some(vehicle);
                    }
                }
                if let Some(e) = self.map.get_mut(&vehicle) {
                    e.passengers = passengers;
                }
            }
            Cb::SetEntityLink { source, dest } => {
                if let Some(e) = self.map.get_mut(&source) {
                    e.leashed_to = (dest != 0).then_some(dest);
                }
            }
            Cb::Animate { id, action } => {
                if let Some(e) = self.map.get_mut(&id) {
                    match action {
                        0 | 3 => {
                            e.swing.start(action == 3, super::swing::NORMAL_TICKS);
                        }
                        1 => e.hurt_ticks = 0,
                        4 | 5 => self.events.push((id, -(action as i32))),
                        _ => {}
                    }
                }
            }
            Cb::SwingAnimation { id, hand, duration, .. } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.swing.start(hand == 1, duration.max(1) as u32);
                }
            }
            Cb::HurtAnimation { id, .. } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.hurt_ticks = 0;
                }
            }
            Cb::DamageEvent { id, .. } => {
                if let Some(e) = self.map.get_mut(&id) {
                    e.hurt_ticks = 0;
                }
            }
            Cb::EntityEvent { id, event } => {
                if let Some(e) = self.map.get_mut(&id) {
                    match (event, e.type_name()) {
                        // A melee attack starting: golems, hoglins and ravagers swing their whole bodies.
                        (4, "iron_golem" | "ravager") => e.attack_left = 10,
                        (4, "hoglin" | "zoglin") => e.attack_left = 10,
                        _ => {}
                    }
                }
                self.events.push((id, event as i32));
            }
            _ => {}
        }
    }

    /// A stand-in with nothing known about it but what it is, for drawing a spawner's mob.
    pub fn display(kind: i32) -> Entity {
        let mut list = Entities::default();
        list.handle(
            Cb::AddEntity(lattice_proto::packets::play::AddEntity { id: 1, uuid: Uuid::nil(), kind, pos: [0.0; 3], velocity: [0.0; 3], pitch: 0.0, yaw: 0.0, head_yaw: 0.0, data: 0 }),
            0,
        );
        list.map.remove(&1).expect("just added")
    }

    /// Puts the vehicle being steered where it now is.
    pub fn drive(&mut self, id: i32, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool) {
        if let Some(e) = self.map.get_mut(&id) {
            e.driven_moved = pos - e.pos;
            e.pos = pos;
            e.server_pos = pos;
            e.lerp_ticks = 0;
            e.yaw = yaw;
            e.pitch = pitch;
            e.target_yaw = yaw;
            e.target_pitch = pitch;
            e.turn_ticks = 0;
            e.head_ticks = 0;
            e.head_yaw = yaw;
            e.target_head = yaw;
            e.body_yaw = yaw;
            e.on_ground = on_ground;
        }
    }

    pub fn tick(&mut self, world: Option<&lattice_world::World>) {
        let data = lattice_data::game();
        for e in self.map.values_mut() {
            if let Some(w) = world {
                let p = e.pos;
                e.in_water = w.block(p.x.floor() as i32, (p.y + 0.1).floor() as i32, p.z.floor() as i32).is_some_and(|s| data.state(s).fluid == 1);
            }
            e.prev_pos = e.pos;
            e.prev_yaw = e.yaw;
            e.prev_pitch = e.pitch;
            e.prev_head_yaw = e.head_yaw;
            e.prev_walk_speed = e.walk_speed;
            e.prev_body_yaw = e.body_yaw;
            // Turning eases over a few ticks, as moving does.
            if e.turn_ticks > 0 {
                let k = e.turn_ticks as f32;
                e.yaw += wrap(e.target_yaw - e.yaw) / k;
                e.pitch += (e.target_pitch - e.pitch) / k;
                e.turn_ticks -= 1;
            }
            if e.head_ticks > 0 {
                e.head_yaw += wrap(e.target_head - e.head_yaw) / e.head_ticks as f32;
                e.head_ticks -= 1;
            }
            if e.lerp_ticks > 0 {
                e.pos += (e.server_pos - e.pos) / e.lerp_ticks as f64;
                e.lerp_ticks -= 1;
            } else {
                e.pos = e.server_pos;
            }
            // A cape trails behind, and doesn't try to follow a long way.
            e.prev_cloak = e.cloak;
            let trail = e.pos - e.cloak;
            if trail.abs().max_element() > 10.0 {
                e.cloak = e.pos;
                e.prev_cloak = e.pos;
            } else {
                e.cloak += trail * 0.25;
            }
            let moved = if e.driven_moved != DVec3::ZERO { std::mem::take(&mut e.driven_moved) } else { e.pos - e.prev_pos };
            let d = ((moved.x * moved.x + moved.z * moved.z).sqrt() * 4.0).min(1.0) as f32;
            e.walk_speed += (d - e.walk_speed) * 0.4;
            e.walk_position += e.walk_speed;
            e.age += 1;
            // The body turns toward where it's walking, and never strays far from the head.
            let square = (moved.x * moved.x + moved.z * moved.z) as f32;
            let mut target = e.body_yaw;
            if square > 0.0025000002 {
                let facing = (moved.z as f32).atan2(moved.x as f32).to_degrees() - 90.0;
                let off = (wrap(e.yaw) - facing).abs();
                target = if 95.0 < off && off < 265.0 { facing - 180.0 } else { facing };
            }
            if e.swing.active() {
                target = e.yaw;
            }
            e.body_yaw += wrap(target - e.body_yaw) * 0.3;
            let off = wrap(e.head_yaw - e.body_yaw);
            if off.abs() > 50.0 {
                e.body_yaw += off - 50.0 * off.signum();
            }
            // Squids tip over on land, and point where they swim in water.
            e.prev_body_pitch = e.body_pitch;
            if matches!(e.type_name(), "squid" | "glow_squid") {
                if e.in_water {
                    let horizontal = (moved.x * moved.x + moved.z * moved.z).sqrt() as f32;
                    let target = -horizontal.atan2(moved.y as f32).to_degrees();
                    e.body_pitch += (target - e.body_pitch) * 0.1;
                } else {
                    e.body_pitch += (-90.0 - e.body_pitch) * 0.02;
                }
            }
            // Using an item, by the flags the server keeps up to date.
            if e.int_field("LIVING_ENTITY_FLAGS").unwrap_or(0) & 1 != 0 {
                e.use_ticks += 1;
            } else {
                e.use_ticks = 0;
            }
            // A creeper's fuse moves by the direction the server gives it.
            e.prev_swell = e.swell;
            if let Some(dir) = e.int_field("SWELL_DIR") {
                e.swell = (e.swell + dir).clamp(0, 30);
            }
            // A slime squashes on landing, then stretches back.
            e.prev_squish = e.squish;
            if e.on_ground && !e.was_on_ground {
                e.squish = -0.5;
            } else if !e.on_ground && e.was_on_ground {
                e.squish = 1.0;
            }
            e.squish *= 0.6;
            e.was_on_ground = e.on_ground;
            if e.attack_left > 0 {
                e.attack_left -= 1;
            }
            e.hurt_ticks = e.hurt_ticks.saturating_add(1);
            if e.is_dead() {
                e.death_ticks += 1;
            }
            e.swing.tick();
        }
        // Riders move with their vehicle.
        let riders: Vec<(i32, i32)> = self.map.iter().filter_map(|(id, e)| e.vehicle.map(|v| (*id, v))).collect();
        for (rider, vehicle) in riders {
            let Some(v) = self.map.get(&vehicle) else { continue };
            let index = v.passengers.iter().position(|p| *p == rider).unwrap_or(0);
            let Some(r) = self.map.get(&rider) else { continue };
            let at = r.info().map_or([0.0; 3], |i| i.rides_at);
            let base = super::riding::seat(v, v.yaw, index, v.passengers.len(), r.is_animal(), at);
            if let Some(r) = self.map.get_mut(&rider) {
                r.pos = base;
                r.server_pos = base;
            }
        }
    }
}
