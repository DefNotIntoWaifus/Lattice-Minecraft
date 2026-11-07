//! Steering what the player is sitting on. The client moves a vehicle it controls and tells
//! the server where it went; anything else it's on just carries it along.

use super::entities::Entity;
use crate::bodies::mobs::{BODY, SADDLE};
use glam::DVec3;
use lattice_data::GameData;
use lattice_proto::packets::play;
use lattice_world::physics::{mth_cos, mth_sin, Body, Boat, Keys, MoveCtx, RideInput};
use lattice_world::World;

const DEG: f32 = std::f32::consts::PI / 180.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Boat,
    Horse,
    Camel,
    Pig,
    Strider,
    HappyGhast,
    Nautilus,
}

impl Kind {
    fn charges_jump(self) -> bool {
        matches!(self, Kind::Horse | Kind::Camel | Kind::Nautilus)
    }
}

/// Which sort of driving this entity takes from a player in its first seat, if any.
pub fn controls(e: &Entity, holding: &dyn Fn(&str) -> bool) -> Option<Kind> {
    let name = e.type_name();
    if is_boat(name) {
        return Some(Kind::Boat);
    }
    let saddled = e.equipment.get(SADDLE).is_some_and(|s| s.is_some());
    let armored = e.equipment.get(BODY).is_some_and(|s| s.is_some());
    match name {
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "llama" | "trader_llama" if saddled => Some(Kind::Horse),
        "camel" | "camel_husk" if saddled => Some(Kind::Camel),
        "pig" if saddled && holding("carrot_on_a_stick") => Some(Kind::Pig),
        "strider" if saddled && holding("warped_fungus_on_a_stick") => Some(Kind::Strider),
        "happy_ghast" if armored && !e.bool_field("STAYS_STILL") => Some(Kind::HappyGhast),
        "nautilus" | "zombie_nautilus" if saddled => Some(Kind::Nautilus),
        _ => None,
    }
}

pub fn is_boat(name: &str) -> bool {
    name.ends_with("_boat") || name.ends_with("_raft") || name == "boat" || name == "raft"
}

/// Whether the inventory key asks the server for this vehicle's own screen instead.
pub fn has_own_screen(name: &str) -> bool {
    matches!(name, "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "llama" | "trader_llama" | "camel" | "camel_husk" | "chest_minecart" | "hopper_minecart" | "nautilus" | "zombie_nautilus")
        || name.ends_with("_chest_boat")
        || name.ends_with("_chest_raft")
}

/// Where a rider's feet go. `index` is the seat, `count` how many are riding, `rider` the
/// rider's own sitting point.
pub fn seat(vehicle: &Entity, yaw: f32, index: usize, count: usize, rider_is_animal: bool, rider: [f64; 3]) -> DVec3 {
    let name = vehicle.type_name();
    let (_, height) = vehicle.dimensions();
    let info = vehicle.info();
    let scale = if vehicle.is_baby() { 0.5 } else { 1.0 };
    let point: [f64; 3] = if is_boat(name) {
        let mut along = 0.0f32;
        if count > 1 {
            along = if index == 0 { 0.2 } else { -0.6 };
            if rider_is_animal {
                along += 0.2;
            }
        }
        let up = if name.contains("raft") { height as f32 * 0.8888889 } else { height as f32 / 3.0 };
        [0.0, up as f64, along as f64]
    } else if matches!(name, "camel" | "camel_husk") {
        let sitting = vehicle.int_field("LAST_POSE_CHANGE_TICK").is_some_and(|t| t < 0);
        let mut along = 0.5f32;
        if count > 1 {
            if index != 0 {
                along = -0.7;
            }
            if rider_is_animal {
                along += 0.2;
            }
        }
        let anchor = if vehicle.is_baby() { 0.09375 } else { 0.375 };
        let up = height - anchor + if sitting { 0.2 * scale as f64 } else { 0.0 };
        [0.0, up, (along * scale) as f64]
    } else {
        let list = info.map_or(&[][..], |i| i.seats.as_slice());
        let p = list.get(index.min(list.len().saturating_sub(1))).copied().unwrap_or([0.0, height, 0.0]);
        [p[0] * scale as f64, p[1] * scale as f64, p[2] * scale as f64]
    };
    let (s, c) = ((-yaw * DEG).sin() as f64, (-yaw * DEG).cos() as f64);
    let at = DVec3::new(point[0] * c + point[2] * s, point[1], point[2] * c - point[0] * s);
    vehicle.pos + at - DVec3::from(rider)
}

enum Drive {
    Boat(Box<Boat>),
    Mount(Box<Body>),
}

/// The vehicle being steered.
pub struct Ride {
    pub vehicle: i32,
    pub kind: Kind,
    drive: Drive,
    /// Charge of the jump key, 0 to 1.
    pub jump_scale: f32,
    jump_ticks: i32,
    was_jumping: bool,
    pending_jump: f32,
    pub cooldown: i32,
    boost_time: i32,
    boost_total: i32,
    boost_seen: i32,
    /// Degrees the vehicle turned, which the rider turns with.
    pub turned: f32,
    last_sent: Option<(DVec3, f32, f32)>,
    reminder: i32,
}

pub struct RideOutcome {
    pub pos: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl Ride {
    pub fn new(vehicle: i32, kind: Kind, e: &Entity, data: &GameData) -> Ride {
        let (w, h) = e.dimensions();
        let drive = if kind == Kind::Boat {
            Drive::Boat(Box::new(Boat::new(data, w as f32, h as f32, e.pos, e.yaw)))
        } else {
            let mut b = Body::for_vehicle(data, w as f32, h as f32, e.pos, e.yaw);
            b.attrs.step_height = attr(e, "step_height", 0.6);
            b.attrs.gravity = attr(e, "gravity", 0.08);
            b.attrs.water_movement_efficiency = 0.0;
            b.attrs.movement_efficiency = 0.0;
            b.vel = e.velocity;
            Drive::Mount(Box::new(b))
        };
        Ride {
            vehicle,
            kind,
            drive,
            jump_scale: 0.0,
            jump_ticks: 0,
            was_jumping: false,
            pending_jump: 0.0,
            cooldown: 0,
            boost_time: 0,
            boost_total: 0,
            boost_seen: e.int_field("BOOST_TIME").unwrap_or(0),
            turned: 0.0,
            last_sent: None,
            reminder: 0,
        }
    }

    pub fn can_charge_jump(&self) -> bool {
        self.kind.charges_jump()
    }

    pub fn body(&self) -> &Body {
        match &self.drive {
            Drive::Boat(b) => &b.body,
            Drive::Mount(b) => b,
        }
    }

    fn body_mut(&mut self) -> &mut Body {
        match &mut self.drive {
            Drive::Boat(b) => &mut b.body,
            Drive::Mount(b) => b,
        }
    }

    /// The server put the vehicle somewhere else.
    pub fn snap_to(&mut self, pos: DVec3, yaw: f32) {
        let b = self.body_mut();
        b.pos = pos;
        b.yaw = yaw;
        b.vel = DVec3::ZERO;
    }

    pub fn paddles(&self) -> Option<(bool, bool)> {
        match &self.drive {
            Drive::Boat(b) => Some((b.paddle_left, b.paddle_right)),
            Drive::Mount(_) => None,
        }
    }

    fn boost_factor(&self) -> f32 {
        if self.boost_total > 0 && self.boost_time <= self.boost_total {
            1.0 + 1.15 * mth_sin((self.boost_time as f32 / self.boost_total as f32 * std::f32::consts::PI) as f64)
        } else {
            1.0
        }
    }

    /// One tick of steering. Returns where the vehicle is now.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(&mut self, e: &Entity, rider_yaw: f32, rider_pitch: f32, keys: Keys, axes: (f32, f32), world: &World, ctx: &MoveCtx, game_time: i64, conn: &lattice_net::Connection, rider_id: i32) -> RideOutcome {
        let before_yaw = self.body().yaw;
        match self.kind {
            Kind::Boat => {
                if let Drive::Boat(b) = &mut self.drive {
                    b.tick(keys.left, keys.right, keys.forward, keys.back, world, ctx);
                }
            }
            kind => self.drive_mount(kind, e, rider_yaw, rider_pitch, keys, axes, world, ctx, game_time, conn, rider_id),
        }
        self.turned = if self.kind == Kind::Boat { self.body().yaw - before_yaw } else { 0.0 };
        let b = self.body();
        RideOutcome { pos: b.pos, yaw: b.yaw, pitch: b.pitch, on_ground: b.on_ground }
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_mount(&mut self, kind: Kind, e: &Entity, rider_yaw: f32, rider_pitch: f32, keys: Keys, axes: (f32, f32), world: &World, ctx: &MoveCtx, game_time: i64, conn: &lattice_net::Connection, rider_id: i32) {
        // The server starts a boost by writing how long it lasts.
        let boost = e.int_field("BOOST_TIME").unwrap_or(0);
        if boost != self.boost_seen {
            self.boost_seen = boost;
            self.boost_time = 0;
            self.boost_total = boost;
        } else if self.boost_total > 0 && self.boost_time <= self.boost_total {
            self.boost_time += 1;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        let refuses = kind == Kind::Camel && camel_refuses(e, game_time);
        let speed_attr = attr(e, "movement_speed", 0.1);
        let strength = attr(e, "jump_strength", 0.42);
        let can_jump = kind.charges_jump() && !refuses;

        // Holding the jump key charges up; letting go leaps.
        if can_jump && self.cooldown == 0 {
            if self.jump_ticks < 0 {
                self.jump_ticks += 1;
                if self.jump_ticks == 0 {
                    self.jump_scale = 0.0;
                }
            }
            if self.was_jumping && !keys.jump {
                self.jump_ticks = -10;
                let amount = (self.jump_scale * 100.0).floor() as i32;
                let on_ground = self.body().on_ground;
                if kind != Kind::Camel || on_ground {
                    self.pending_jump = if amount >= 90 { 1.0 } else { 0.4 + 0.4 * amount.max(0) as f32 / 90.0 };
                }
                conn.send(play::player_command_data(rider_id, play::command::START_RIDING_JUMP, amount));
            } else if !self.was_jumping && keys.jump {
                self.jump_ticks = 0;
                self.jump_scale = 0.0;
            } else if self.was_jumping {
                self.jump_ticks += 1;
                self.jump_scale = if self.jump_ticks < 10 { self.jump_ticks as f32 * 0.1 } else { 0.8 + 2.0 / (self.jump_ticks - 9) as f32 * 0.1 };
            }
        } else {
            self.jump_scale = 0.0;
        }
        self.was_jumping = keys.jump;

        let boost_factor = self.boost_factor();
        let pending = std::mem::take(&mut self.pending_jump);
        let cooldown_clear = self.cooldown == 0;
        let (xxa, zza) = axes;
        let in_water = self.body().in_water;
        let b = self.body_mut();
        let mut input = RideInput::default();
        match kind {
            Kind::Pig | Kind::Strider => {
                b.yaw = rider_yaw;
                b.pitch = rider_pitch * 0.5;
                input.zza = 1.0;
                input.speed = if kind == Kind::Pig {
                    (speed_attr * 0.225) as f32 * boost_factor
                } else {
                    let k = if e.bool_field("SUFFOCATING") { 0.35 } else { 0.55 };
                    (speed_attr * k) as f32 * boost_factor
                };
                input.lava_walker = kind == Kind::Strider;
            }
            Kind::Horse | Kind::Camel => {
                let standing = kind == Kind::Horse && e.int_field("ID_FLAGS").is_some_and(|f| f & 32 != 0);
                if refuses || (b.on_ground && pending == 0.0 && standing) {
                    input.xxa = 0.0;
                    input.zza = 0.0;
                } else {
                    input.xxa = xxa * 0.5;
                    input.zza = if zza <= 0.0 { zza * 0.25 } else { zza };
                }
                if !refuses {
                    b.yaw = rider_yaw;
                    b.pitch = rider_pitch * 0.5;
                }
                input.speed = if kind == Kind::Camel {
                    speed_attr as f32 + if keys.sprint && cooldown_clear { 0.1 } else { 0.0 }
                } else {
                    speed_attr as f32
                };
                if b.on_ground && pending > 0.0 {
                    if kind == Kind::Horse {
                        b.vel.y = b.jump_power(strength, pending, world, ctx);
                        if input.zza > 0.0 {
                            let (s, c) = (mth_sin((b.yaw * DEG) as f64), mth_cos((b.yaw * DEG) as f64));
                            b.vel += DVec3::new((-0.4 * s * pending) as f64, 0.0, (0.4 * c * pending) as f64);
                        }
                    } else {
                        let power = b.jump_power(strength, 1.0, world, ctx);
                        let (s, c) = (mth_sin((b.yaw * DEG) as f64) as f64, mth_cos((b.yaw * DEG) as f64) as f64);
                        let along = (22.2222 * pending) as f64 * speed_attr * b.speed_factor(world, ctx);
                        b.vel += DVec3::new(-s * along, (1.4285 * pending) as f64 * power, c * along);
                    }
                }
            }
            Kind::HappyGhast => {
                let (mut y, mut z) = (0.0f32, 0.0f32);
                if zza != 0.0 {
                    let (mut f, mut u) = (mth_cos((rider_pitch * DEG) as f64), -mth_sin((rider_pitch * DEG) as f64));
                    if zza < 0.0 {
                        f *= -0.5;
                        u *= -0.5;
                    }
                    z = f;
                    y = u;
                }
                if keys.jump {
                    y += 0.5;
                }
                let flying = attr(e, "flying_speed", 0.05);
                let k = (3.9000000953674316 * flying) as f32;
                input.xxa = xxa * k;
                input.yya = y * k;
                input.zza = z * k;
                input.speed = speed_attr as f32;
                input.flying = Some(flying as f32 * 5.0 / 3.0);
                let turn = wrap(rider_yaw - b.yaw) * 0.08;
                b.yaw += turn;
                b.pitch = rider_pitch * 0.5;
            }
            Kind::Nautilus => {
                let (mut y, mut z) = (0.0f32, 0.0f32);
                if zza != 0.0 {
                    let (mut f, mut u) = (mth_cos((rider_pitch * DEG) as f64), -mth_sin((rider_pitch * DEG) as f64));
                    if zza < 0.0 {
                        f *= -0.5;
                        u *= -0.5;
                    }
                    z = f;
                    y = u;
                }
                input.xxa = xxa;
                input.yya = y;
                input.zza = z;
                input.speed = if in_water { 0.0325 * speed_attr as f32 } else { 0.02 * speed_attr as f32 };
                input.swims = Some(0.9);
                let turn = wrap(rider_yaw - b.yaw) * 0.5;
                b.yaw += turn;
                b.pitch = rider_pitch * 0.5;
                if pending > 0.0 {
                    let (yr, pr) = (rider_yaw * DEG, rider_pitch * DEG);
                    let look = DVec3::new((-yr.sin() * pr.cos()) as f64, -pr.sin() as f64, (yr.cos() * pr.cos()) as f64);
                    let k = if in_water { 1.2 } else { 0.5 };
                    b.vel += look * (k * pending) as f64 * speed_attr * b.speed_factor(world, ctx);
                }
            }
            Kind::Boat => {}
        }
        b.ride_tick(input, world, ctx);
        if pending > 0.0 {
            self.cooldown = match kind {
                Kind::Camel => 55,
                Kind::Nautilus => 40,
                _ => 0,
            };
        }
    }

    /// Reports where the vehicle is, each tick as the game does.
    pub fn report(&mut self, conn: &lattice_net::Connection) {
        let b = self.body();
        let at = (b.pos, b.yaw, b.pitch);
        let on_ground = b.on_ground;
        conn.send(play::move_vehicle(at.0.to_array(), at.1, at.2, on_ground));
        self.last_sent = Some(at);
        self.reminder += 1;
        if let Some((left, right)) = self.paddles() {
            conn.send(play::paddle_boat(left, right));
        }
    }
}

fn camel_refuses(e: &Entity, game_time: i64) -> bool {
    let Some(last) = e.long_field("LAST_POSE_CHANGE_TICK") else { return false };
    let sitting = last < 0;
    let since = game_time - last.abs();
    sitting || since < 52
}

/// The value of one of an entity's movement numbers, as last heard or by default.
pub fn attr(e: &Entity, name: &str, fallback: f64) -> f64 {
    e.attrs.get(name).copied().or_else(|| e.info().and_then(|i| i.attributes.get(name).copied())).unwrap_or(fallback)
}

pub fn wrap(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}
