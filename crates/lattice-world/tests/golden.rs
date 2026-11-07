//! Replays movement recorded from the official client (tools/reference/run.py) against
//! Lattice's physics. Each recording holds the blocks of a scene, the keys held and the
//! player's exact state every tick. Lattice starts from the first recorded state and has
//! to stay within `TOLERANCE` for the rest of the recording.

use glam::DVec3;
use lattice_world::physics::{BlockAccess, Body, Keys, MoveCtx, MoveTags};
use serde::Deserialize;
use std::collections::HashMap;

/// Blocks per tick. Far below anything a player or server could notice.
const TOLERANCE: f64 = 1.0e-3;

#[derive(Deserialize)]
struct Segment {
    ticks: u32,
    keys: Vec<String>,
    yaw: Option<f32>,
    pitch: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
struct Row {
    pos: [f64; 3],
    vel: [f64; 3],
    yaw: f32,
    pitch: f32,
    ground: bool,
    wall: bool,
    water: bool,
    under: bool,
    lava: bool,
    swim: bool,
    sprint: bool,
    crouch: bool,
    fly: bool,
    pose: String,
}

#[derive(Deserialize)]
struct Scene {
    name: String,
    about: String,
    #[serde(default)]
    effects: HashMap<String, i32>,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    worn: HashMap<String, serde_json::Value>,
    blocks: Vec<[i32; 4]>,
    segments: Vec<Segment>,
    trace: Vec<Row>,
}

struct SceneWorld(HashMap<(i32, i32, i32), u32>);

impl BlockAccess for SceneWorld {
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        Some(*self.0.get(&(x, y, z)).unwrap_or(&0))
    }
}

fn keys_from(names: &[String]) -> Keys {
    let has = |n: &str| names.iter().any(|k| k == n);
    Keys { forward: has("forward"), back: has("back"), left: has("left"), right: has("right"), jump: has("jump"), shift: has("sneak"), sprint: has("sprint") }
}

/// Runs one recording. Returns the first disagreement, if any.
fn replay(scene: &Scene, tags: &MoveTags) -> Result<(), String> {
    let data = lattice_data::game();
    let world = SceneWorld(scene.blocks.iter().map(|b| ((b[0], b[1], b[2]), b[3] as u32)).collect());
    let ctx = MoveCtx {
        data,
        tags,
        fast_lava: false,
        auto_jump: false,
        sprint_window: 7,
        enough_food: true,
        leather_boots: scene.worn.contains_key("leather_boots"),
        can_glide: scene.worn.contains_key("can_glide"),
        item_use_speed: None,
        item_use_can_sprint: false,
        min_y: -64,
        border: None,
    };
    let first = &scene.trace[0];
    let mut b = Body::new(data);
    b.pos = DVec3::from(first.pos);
    b.vel = DVec3::from(first.vel);
    b.yaw = first.yaw;
    b.pitch = first.pitch;
    b.on_ground = first.ground;
    b.sprinting = first.sprint;
    b.in_water = first.water;
    b.in_lava = first.lava;
    b.under_water = first.under;
    b.eye_in_water = first.under;
    if let Some(a) = scene.effects.get("speed") {
        b.attrs.movement_speed *= 1.0 + 0.2 * (*a + 1) as f64;
    }
    b.effects.jump_boost = scene.effects.get("jump_boost").copied();
    b.effects.levitation = scene.effects.get("levitation").copied();
    b.effects.slow_falling = scene.effects.contains_key("slow_falling");
    b.effects.dolphins_grace = scene.effects.contains_key("dolphins_grace");
    if let Some(v) = scene.worn.get("water_movement_efficiency").and_then(|v| v.as_f64()) {
        b.attrs.water_movement_efficiency = v;
    }
    match scene.mode.as_str() {
        "creative" => b.may_fly = true,
        "spectator" => {
            b.may_fly = true;
            b.flying = true;
            b.spectator = true;
        }
        _ => {}
    }
    b.flying |= first.fly;
    b.pose = pose_from(&first.pose);

    let mut row = 1;
    for seg in &scene.segments {
        let keys = keys_from(&seg.keys);
        if let Some(y) = seg.yaw {
            b.yaw = y;
        }
        if let Some(p) = seg.pitch {
            b.pitch = p;
        }
        for _ in 0..seg.ticks {
            b.tick(keys, &world, &ctx);
            let want = &scene.trace[row];
            if row == 1 {
                // A single row can't show everything: being held by a cobweb, say, was
                // decided the tick before. One tick here fills that in; from the row
                // after this on, Lattice has to get there by itself.
                b.pos = DVec3::from(want.pos);
                b.vel = DVec3::from(want.vel);
                b.on_ground = want.ground;
                b.horizontal_collision = want.wall;
                b.sprinting = want.sprint;
                b.flying = want.fly;
                b.pose = pose_from(&want.pose);
                row += 1;
                continue;
            }
            let d = DVec3::from(want.pos) - b.pos;
            let dv = DVec3::from(want.vel) - b.vel;
            let bad_flag = [
                ("ground", b.on_ground, want.ground),
                ("wall", b.horizontal_collision, want.wall),
                ("water", b.in_water, want.water),
                ("lava", b.in_lava, want.lava),
                ("under", b.under_water, want.under),
                ("sprint", b.sprinting, want.sprint),
                ("crouch", b.crouching, want.crouch),
                ("fly", b.flying, want.fly),
            ]
            .into_iter()
            .find(|(_, a, b)| a != b);
            if d.abs().max_element() > TOLERANCE || dv.abs().max_element() > TOLERANCE || bad_flag.is_some() || pose_from(&want.pose) != b.pose {
                return Err(format!(
                    "{}: tick {row}/{}: {}\n  want pos {:?} vel {:?} ground {} wall {} water {} under {} swim {} sprint {} crouch {} pose {}\n  have pos {:?} vel {:?} ground {} wall {} water {} under {} swim {} sprint {} crouch {} pose {:?}\n  off by pos {:?} vel {:?} flag {:?}",
                    scene.name,
                    scene.trace.len() - 1,
                    scene.about,
                    want.pos, want.vel, want.ground, want.wall, want.water, want.under, want.swim, want.sprint, want.crouch, want.pose,
                    b.pos.to_array(), b.vel.to_array(), b.on_ground, b.horizontal_collision, b.in_water, b.under_water, b.is_swimming(), b.sprinting, b.crouching, b.pose,
                    d.to_array(), dv.to_array(), bad_flag,
                ));
            }
            row += 1;
        }
    }
    Ok(())
}

fn pose_from(name: &str) -> lattice_world::physics::Pose {
    use lattice_world::physics::Pose;
    match name {
        "CROUCHING" => Pose::Crouching,
        "SWIMMING" => Pose::Swimming,
        "FALL_FLYING" => Pose::FallFlying,
        "SPIN_ATTACK" => Pose::SpinAttack,
        "SLEEPING" => Pose::Sleeping,
        _ => Pose::Standing,
    }
}

#[test]
fn movement_matches_the_official_client() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
    let tags = MoveTags::defaults(lattice_data::game());
    let mut scenes: Vec<_> = std::fs::read_dir(dir).expect("recordings").filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    scenes.sort();
    assert!(!scenes.is_empty());
    let only = std::env::var("GOLDEN").unwrap_or_default();
    let (mut ok, mut failures) = (0, Vec::new());
    for path in scenes {
        let scene: Scene = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if !only.is_empty() && !scene.name.contains(&only) {
            continue;
        }
        match replay(&scene, &tags) {
            Ok(()) => ok += 1,
            Err(e) => failures.push(e),
        }
    }
    println!("{ok} scenes match, {} don't", failures.len());
    for f in &failures {
        println!("\n{f}");
    }
    assert!(failures.is_empty(), "{} of {} scenes differ from the official client", failures.len(), ok + failures.len());
}
