//! The shapes of mobs and block entities, read from the player's own game files by running
//! the game's model-building code (see classvm.rs). Results stay in memory.

use crate::classvm::{V, Vm, VmResult};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Cube {
    /// Texture position in pixels.
    pub tex: [f32; 2],
    pub origin: [f32; 3],
    pub size: [f32; 3],
    pub grow: [f32; 3],
    pub mirror: bool,
    pub tex_scale: [f32; 2],
    /// Which faces exist: down, up, north, south, west, east.
    pub faces: [bool; 6],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub cubes: Vec<Cube>,
    pub children: Vec<(String, Part)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub tex_size: [f32; 2],
    pub root: Part,
}

/// "zombie#main" to the model.
pub type Models = HashMap<String, Model>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Position,
    Rotation,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub time: f32,
    pub pre: [f32; 3],
    pub post: [f32; 3],
    pub smooth: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub part: String,
    pub target: Target,
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub length: f32,
    pub looping: bool,
    pub channels: Vec<Channel>,
}

/// "FrogAnimation.FROG_JUMP" to the animation.
pub type Animations = HashMap<String, Animation>;

fn float(v: &V) -> f32 {
    match v {
        V::Float(f) => *f,
        V::Double(d) => *d as f32,
        V::Int(i) => *i as f32,
        _ => 0.0,
    }
}

fn read_cube(vm: &Vm, c: &V) -> Cube {
    let tex = vm.get(c, "texCoord");
    let origin = vm.get(c, "origin");
    let dims = vm.get(c, "dimensions");
    let grow = vm.get(c, "grow");
    let scale = vm.get(c, "texScale");
    let mut faces = [false; 6];
    for d in vm.list_of(&vm.get(c, "visibleFaces")) {
        let name = vm.string_of(&d);
        let name = name.rsplit('.').next().unwrap_or("");
        let i = match name {
            "DOWN" => 0,
            "UP" => 1,
            "NORTH" => 2,
            "SOUTH" => 3,
            "WEST" => 4,
            "EAST" => 5,
            _ => continue,
        };
        faces[i] = true;
    }
    Cube {
        tex: [float(&vm.get(&tex, "u")), float(&vm.get(&tex, "v"))],
        origin: [float(&vm.get(&origin, "x")), float(&vm.get(&origin, "y")), float(&vm.get(&origin, "z"))],
        size: [float(&vm.get(&dims, "x")), float(&vm.get(&dims, "y")), float(&vm.get(&dims, "z"))],
        grow: [float(&vm.get(&grow, "growX")), float(&vm.get(&grow, "growY")), float(&vm.get(&grow, "growZ"))],
        mirror: vm.get(c, "mirror") == V::Int(1),
        tex_scale: [float(&vm.get(&scale, "u")), float(&vm.get(&scale, "v"))],
        faces,
    }
}

fn read_part(vm: &Vm, p: &V) -> Part {
    let pose = vm.get(p, "partPose");
    let f = |n: &str| float(&vm.get(&pose, n));
    let scale_or_one = |n: &str| {
        let v = vm.get(&pose, n);
        if v == V::Null { 1.0 } else { float(&v) }
    };
    let cubes = vm.list_of(&vm.get(p, "cubes")).iter().map(|c| read_cube(vm, c)).collect();
    let children = vm.map_of(&vm.get(p, "children")).iter().map(|(k, v)| (vm.string_of(k), read_part(vm, v))).collect();
    Part {
        offset: [f("x"), f("y"), f("z")],
        rotation: [f("xRot"), f("yRot"), f("zRot")],
        scale: [scale_or_one("xScale"), scale_or_one("yScale"), scale_or_one("zScale")],
        cubes,
        children,
    }
}

/// Runs the game's table of model definitions from its jar.
pub fn read_models(jar: &Path) -> VmResult<Models> {
    let mut vm = Vm::open(jar)?;
    let roots = vm.call("net/minecraft/client/model/geom/LayerDefinitions", "createRoots", "()Ljava/util/Map;", Vec::new())?;
    let mut out = HashMap::new();
    for (key, layer) in vm.map_of(&roots) {
        let model = vm.string_of(&vm.get(&key, "model"));
        let name = vm.string_of(&vm.get(&key, "layer"));
        let path = model.trim_start_matches("minecraft:").to_string();
        let material = vm.get(&layer, "material");
        let mesh = vm.get(&layer, "mesh");
        let root = vm.get(&mesh, "root");
        out.insert(
            format!("{path}#{name}"),
            Model { tex_size: [float(&vm.get(&material, "xTexSize")), float(&vm.get(&material, "yTexSize"))], root: read_part(&vm, &root) },
        );
    }
    Ok(out)
}

fn read_vec(vm: &Vm, v: &V) -> [f32; 3] {
    [float(&vm.get(v, "x")), float(&vm.get(v, "y")), float(&vm.get(v, "z"))]
}

/// Runs the game's tables of keyframe animations from its jar.
pub fn read_animations(jar: &Path) -> VmResult<Animations> {
    let mut vm = Vm::open(jar)?;
    let mut out = HashMap::new();
    for class in vm.classes_under("net/minecraft/client/animation/definitions/") {
        let short = class.rsplit('/').next().unwrap_or("").to_string();
        for (name, value) in vm.statics_of(&class)? {
            if value == V::Null || vm.get(&value, "boneAnimations") == V::Null {
                continue;
            }
            let mut channels = Vec::new();
            for (bone, list) in vm.map_of(&vm.get(&value, "boneAnimations")) {
                let part = vm.string_of(&bone);
                for ch in vm.list_of(&list) {
                    let target = vm.string_of(&vm.get(&ch, "target"));
                    let target = match target.rsplit('.').next().unwrap_or("") {
                        "POSITION" => Target::Position,
                        "SCALE" => Target::Scale,
                        _ => Target::Rotation,
                    };
                    let keys = vm
                        .list_of(&vm.get(&ch, "keyframes"))
                        .iter()
                        .map(|k| Key {
                            time: float(&vm.get(k, "timestamp")),
                            pre: read_vec(&vm, &vm.get(k, "preTarget")),
                            post: read_vec(&vm, &vm.get(k, "postTarget")),
                            smooth: vm.string_of(&vm.get(k, "interpolation")).ends_with("CATMULLROM"),
                        })
                        .collect();
                    channels.push(Channel { part: part.clone(), target, keys });
                }
            }
            out.insert(format!("{short}.{name}"), Animation { length: float(&vm.get(&value, "lengthInSeconds")), looping: vm.get(&value, "looping") == V::Int(1), channels });
        }
    }
    Ok(out)
}
