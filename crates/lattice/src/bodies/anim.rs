//! How bodies move: each family of models, from the numbers in the game's own model code.

use super::St;
use lattice_render::parts::Rig;
use std::f32::consts::{FRAC_PI_2, PI};

const D: f32 = PI / 180.0;

fn cos(v: f32) -> f32 {
    v.cos()
}

fn sin(v: f32) -> f32 {
    v.sin()
}

pub fn look(r: &mut Rig, s: &St) {
    let head = r.p("head");
    head.rot.x = s.pitch * D;
    head.rot.y = s.yaw * D;
}

pub fn quadruped(r: &mut Rig, s: &St, amp: f32) {
    look(r, s);
    let w = s.wp * 0.6662;
    r.p("right_hind_leg").rot.x = cos(w) * amp * s.ws;
    r.p("left_hind_leg").rot.x = cos(w + PI) * amp * s.ws;
    r.p("right_front_leg").rot.x = cos(w + PI) * amp * s.ws;
    r.p("left_front_leg").rot.x = cos(w) * amp * s.ws;
}

pub fn bob_part(r: &mut Rig, name: &str, age: f32, sign: f32) {
    let p = r.p(name);
    p.rot.z += sign * (cos(age * 0.09) * 0.05 + 0.05);
    p.rot.x += sign * (sin(age * 0.067) * 0.05);
}

pub fn bob_arms(r: &mut Rig, age: f32) {
    bob_part(r, "right_arm", age, 1.0);
    bob_part(r, "left_arm", age, -1.0);
}

/// What an arm is doing, besides swinging as it walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArmPose {
    #[default]
    Empty,
    Item,
    Block,
    Bow,
    Crossbow,
    CrossbowCharge(u32),
    Throw,
    Spyglass,
    Horn,
    Brush,
}

pub fn pose_arm(r: &mut Rig, s: &St, pose: ArmPose, right: bool) {
    let (arm, other, side) = if right { ("right_arm", "left_arm", 1.0) } else { ("left_arm", "right_arm", -1.0) };
    let head = r.get("head").rot;
    match pose {
        ArmPose::Empty => r.p(arm).rot.y = 0.0,
        ArmPose::Item => {
            let a = r.p(arm);
            a.rot.x = a.rot.x * 0.5 - 0.31415927;
            a.rot.y = 0.0;
        }
        ArmPose::Block => {
            let a = r.p(arm);
            a.rot.x = a.rot.x * 0.5 - 0.9424779;
            a.rot.y = side * -0.5235988;
        }
        ArmPose::Throw => {
            let a = r.p(arm);
            a.rot.x = a.rot.x * 0.5 - PI;
            a.rot.y = 0.0;
        }
        ArmPose::Bow => {
            r.p(arm).rot.y = side * -0.1 + head.y;
            r.p(other).rot.y = side * 0.1 + head.y + side * 0.4;
            r.p(arm).rot.x = -FRAC_PI_2 + head.x;
            r.p(other).rot.x = -FRAC_PI_2 + head.x;
        }
        ArmPose::Crossbow => crossbow_hold(r, right),
        ArmPose::CrossbowCharge(ticks) => crossbow_charge(r, 25.0, ticks as f32, right),
        ArmPose::Spyglass => {
            let crouch = if s.crouching { 0.2617994 } else { 0.0 };
            let a = r.p(arm);
            a.rot.x = (head.x - 1.9198622 - crouch).clamp(-2.4, 3.3);
            a.rot.y = head.y - side * 0.2617994;
        }
        ArmPose::Horn => {
            let a = r.p(arm);
            a.rot.x = head.x.clamp(-1.2, 1.2) - 1.4835298;
            a.rot.y = head.y - side * 0.5235988;
        }
        ArmPose::Brush => {
            let a = r.p(arm);
            a.rot.x = a.rot.x * 0.5 - 0.62831855;
            a.rot.y = 0.0;
        }
    }
}

pub fn crossbow_hold(r: &mut Rig, right: bool) {
    let head = r.get("head").rot;
    let (a, b, sign) = if right { ("right_arm", "left_arm", -1.0) } else { ("left_arm", "right_arm", 1.0) };
    r.p(a).rot.y = sign * 0.3 + head.y;
    r.p(b).rot.y = -sign * 0.6 + head.y;
    r.p(a).rot.x = -FRAC_PI_2 + head.x + 0.1;
    r.p(b).rot.x = -1.5 + head.x;
}

pub fn crossbow_charge(r: &mut Rig, max: f32, used: f32, right: bool) {
    let (a, b, sign) = if right { ("right_arm", "left_arm", -1.0) } else { ("left_arm", "right_arm", 1.0) };
    r.p(a).rot.y = sign * 0.8;
    r.p(a).rot.x = -0.97079635;
    let x = r.get(a).rot.x;
    let t = used.clamp(0.0, max) / max;
    r.p(b).rot.y = (0.4 + (0.85 - 0.4) * t) * if right { 1.0 } else { -1.0 };
    r.p(b).rot.x = x + (-FRAC_PI_2 - x) * t;
}

fn out_quart(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(4)
}

/// The arm swing of a melee hit: body twists, arm comes down.
pub fn attack(r: &mut Rig, s: &St) {
    if s.swing <= 0.0 {
        return;
    }
    let left_arm_swings = s.swing_off_hand != s.left_handed;
    let mut twist = sin(s.swing.sqrt() * std::f32::consts::TAU) * 0.2;
    if left_arm_swings {
        twist = -twist;
    }
    r.p("body").rot.y = twist;
    let k = s.age_scale;
    {
        let a = r.p("right_arm");
        a.pos.z = sin(twist) * 5.0 * k;
        a.pos.x = -cos(twist) * 5.0 * k;
        a.rot.y += twist;
    }
    {
        let a = r.p("left_arm");
        a.pos.z = -sin(twist) * 5.0 * k;
        a.pos.x = cos(twist) * 5.0 * k;
        a.rot.y += twist;
        a.rot.x += twist;
    }
    let f = sin(out_quart(s.swing) * PI);
    let head_x = r.get("head").rot.x;
    let g = sin(s.swing * PI) * -(head_x - 0.7) * 0.75;
    let arm = r.p(if left_arm_swings { "left_arm" } else { "right_arm" });
    arm.rot.x -= f * 1.2 + g;
    arm.rot.y += twist * 2.0;
    arm.rot.z += sin(s.swing * PI) * -0.4;
}

fn quadratic_arm(t: f32) -> f32 {
    -65.0 * t + t * t
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + (b - a) * t
}

fn rot_lerp_rad(t: f32, a: f32, b: f32) -> f32 {
    let mut d = (b - a) % std::f32::consts::TAU;
    if d >= PI {
        d -= std::f32::consts::TAU;
    }
    if d < -PI {
        d += std::f32::consts::TAU;
    }
    a + t * d
}

fn swim(r: &mut Rig, s: &St) {
    let amount = s.swimming;
    if !s.gliding {
        let h = r.p("head");
        h.rot.x = rot_lerp_rad(amount, h.rot.x, -0.7853982);
    }
    let w = s.wp % 26.0;
    let (left, right) = (amount, amount);
    if w < 14.0 {
        let q = 1.8707964 * quadratic_arm(w) / quadratic_arm(14.0);
        let a = r.p("left_arm");
        a.rot.x = rot_lerp_rad(left, a.rot.x, 0.0);
        a.rot.y = rot_lerp_rad(left, a.rot.y, PI);
        a.rot.z = rot_lerp_rad(left, a.rot.z, PI + q);
        let a = r.p("right_arm");
        a.rot.x = lerp(right, a.rot.x, 0.0);
        a.rot.y = lerp(right, a.rot.y, PI);
        a.rot.z = lerp(right, a.rot.z, PI - q);
    } else if w < 22.0 {
        let t = (w - 14.0) / 8.0;
        let a = r.p("left_arm");
        a.rot.x = rot_lerp_rad(left, a.rot.x, FRAC_PI_2 * t);
        a.rot.y = rot_lerp_rad(left, a.rot.y, PI);
        a.rot.z = rot_lerp_rad(left, a.rot.z, 5.012389 - 1.8707964 * t);
        let a = r.p("right_arm");
        a.rot.x = lerp(right, a.rot.x, FRAC_PI_2 * t);
        a.rot.y = lerp(right, a.rot.y, PI);
        a.rot.z = lerp(right, a.rot.z, 1.2707963 + 1.8707964 * t);
    } else {
        let t = (w - 22.0) / 4.0;
        let a = r.p("left_arm");
        a.rot.x = rot_lerp_rad(left, a.rot.x, FRAC_PI_2 - FRAC_PI_2 * t);
        a.rot.y = rot_lerp_rad(left, a.rot.y, PI);
        a.rot.z = rot_lerp_rad(left, a.rot.z, PI);
        let a = r.p("right_arm");
        a.rot.x = lerp(right, a.rot.x, FRAC_PI_2 - FRAC_PI_2 * t);
        a.rot.y = lerp(right, a.rot.y, PI);
        a.rot.z = lerp(right, a.rot.z, PI);
    }
    let l = r.p("left_leg");
    l.rot.x = lerp(amount, l.rot.x, 0.3 * cos(s.wp * 0.33333334 + PI));
    let l = r.p("right_leg");
    l.rot.x = lerp(amount, l.rot.x, 0.3 * cos(s.wp * 0.33333334));
}

/// People and everything shaped like them.
pub fn humanoid(r: &mut Rig, s: &St, right: ArmPose, left: ArmPose, zombie: Option<bool>) {
    look(r, s);
    if s.gliding {
        r.p("head").rot.x = -0.7853982;
    }
    let w = s.wp * 0.6662;
    r.p("right_arm").rot.x = cos(w + PI) * 2.0 * s.ws * 0.5;
    r.p("left_arm").rot.x = cos(w) * 2.0 * s.ws * 0.5;
    {
        let l = r.p("right_leg");
        l.rot.x = cos(w) * 1.4 * s.ws;
        l.rot.y = 0.005;
        l.rot.z = 0.005;
    }
    {
        let l = r.p("left_leg");
        l.rot.x = cos(w + PI) * 1.4 * s.ws;
        l.rot.y = -0.005;
        l.rot.z = -0.005;
    }
    if s.riding {
        r.p("right_arm").rot.x += -0.62831855;
        r.p("left_arm").rot.x += -0.62831855;
        let l = r.p("right_leg");
        l.rot.x = -1.4137167;
        l.rot.y = 0.31415927;
        l.rot.z = 0.07853982;
        let l = r.p("left_leg");
        l.rot.x = -1.4137167;
        l.rot.y = -0.31415927;
        l.rot.z = -0.07853982;
    }
    // The dominant arm's pose goes first, and two-handed poses take both arms.
    let two_handed = |p: ArmPose| matches!(p, ArmPose::Bow | ArmPose::Crossbow | ArmPose::CrossbowCharge(_));
    if !s.left_handed {
        pose_arm(r, s, right, true);
        if !two_handed(right) {
            pose_arm(r, s, left, false);
        }
    } else {
        pose_arm(r, s, left, false);
        if !two_handed(left) {
            pose_arm(r, s, right, true);
        }
    }
    match zombie {
        Some(holding) => zombie_arms(r, s, holding),
        None => attack(r, s),
    }
    if s.crouching {
        r.p("body").rot.x = 0.5;
        r.p("right_arm").rot.x += 0.4;
        r.p("left_arm").rot.x += 0.4;
        r.p("right_leg").pos.z += 4.0;
        r.p("left_leg").pos.z += 4.0;
        r.p("head").pos.y += 4.2;
        r.p("body").pos.y += 3.2;
        r.p("left_arm").pos.y += 3.2;
        r.p("right_arm").pos.y += 3.2;
    }
    if right != ArmPose::Spyglass {
        bob_part(r, "right_arm", s.age, 1.0);
    }
    if left != ArmPose::Spyglass {
        bob_part(r, "left_arm", s.age, -1.0);
    }
    if s.swimming > 0.0 {
        swim(r, s);
    }
}

/// Undead reach forward when they're after someone.
pub fn zombie_arms(r: &mut Rig, s: &St, holding: bool) {
    let raised = !s.baby || !holding;
    let base = if raised { -PI / if s.aggressive { 1.5 } else { 2.25 } } else { 0.0 };
    let f = if raised { 1.0 } else { -1.0 } * sin(s.swing * PI);
    let g = sin((1.0 - (1.0 - s.swing) * (1.0 - s.swing)) * PI);
    let x = base + f * 1.2 - g * 0.4;
    let y = 0.1 - f * 0.6;
    let a = r.p("right_arm");
    a.rot.x = x;
    a.rot.y = if raised { -y } else { y };
    a.rot.z = 0.0;
    let a = r.p("left_arm");
    a.rot.x = x;
    a.rot.y = if raised { y } else { -y };
    a.rot.z = 0.0;
}

pub fn skeleton_melee(r: &mut Rig, s: &St) {
    let f = sin(s.swing * PI);
    let g = sin((1.0 - (1.0 - s.swing) * (1.0 - s.swing)) * PI);
    let a = r.p("right_arm");
    a.rot.z = 0.0;
    a.rot.y = -(0.1 - f * 0.6);
    a.rot.x = -FRAC_PI_2 - (f * 1.2 - g * 0.4);
    let a = r.p("left_arm");
    a.rot.z = 0.0;
    a.rot.y = 0.1 - f * 0.6;
    a.rot.x = -FRAC_PI_2 - (f * 1.2 - g * 0.4);
    bob_arms(r, s.age);
}

pub fn swing_weapon_down(r: &mut Rig, s: &St) {
    let f = sin(s.swing * PI);
    let g = sin((1.0 - (1.0 - s.swing) * (1.0 - s.swing)) * PI);
    r.p("right_arm").rot.z = 0.0;
    r.p("left_arm").rot.z = 0.0;
    r.p("right_arm").rot.y = 0.15707964;
    r.p("left_arm").rot.y = -0.15707964;
    if !s.left_handed {
        r.p("right_arm").rot.x = -1.8849558 + cos(s.age * 0.09) * 0.15 + (f * 2.2 - g * 0.4);
        r.p("left_arm").rot.x = cos(s.age * 0.19) * 0.5 + (f * 1.2 - g * 0.4);
    } else {
        r.p("right_arm").rot.x = cos(s.age * 0.19) * 0.5 + (f * 1.2 - g * 0.4);
        r.p("left_arm").rot.x = -1.8849558 + cos(s.age * 0.09) * 0.15 + (f * 2.2 - g * 0.4);
    }
    bob_arms(r, s.age);
}

fn triangle_wave(t: f32, period: f32) -> f32 {
    (((t % period) - period * 0.5).abs() - period * 0.25) / (period * 0.25)
}

pub fn iron_golem(r: &mut Rig, s: &St, attack_ticks: f32) {
    look(r, s);
    if attack_ticks > 0.0 {
        r.p("right_arm").rot.x = -2.0 + 1.5 * triangle_wave(attack_ticks, 10.0);
        r.p("left_arm").rot.x = -2.0 + 1.5 * triangle_wave(attack_ticks, 10.0);
    } else {
        r.p("right_arm").rot.x = (-0.2 + 1.5 * triangle_wave(s.wp, 13.0)) * s.ws;
        r.p("left_arm").rot.x = (-0.2 - 1.5 * triangle_wave(s.wp, 13.0)) * s.ws;
    }
    let t = triangle_wave(s.wp, 13.0);
    r.p("right_leg").rot.x = -1.5 * t * s.ws;
    r.p("left_leg").rot.x = 1.5 * t * s.ws;
    r.p("right_leg").rot.y = 0.0;
    r.p("left_leg").rot.y = 0.0;
}

pub fn spider(r: &mut Rig, s: &St) {
    look(r, s);
    let w = s.wp * 0.6662;
    let a = -(cos(w * 2.0) * 0.4) * s.ws;
    let b = -(cos(w * 2.0 + PI) * 0.4) * s.ws;
    let c = -(cos(w * 2.0 + FRAC_PI_2) * 0.4) * s.ws;
    let d = -(cos(w * 2.0 + 4.712389) * 0.4) * s.ws;
    let e = (sin(w) * 0.4).abs() * s.ws;
    let f = (sin(w + PI) * 0.4).abs() * s.ws;
    let g = (sin(w + FRAC_PI_2) * 0.4).abs() * s.ws;
    let h = (sin(w + 4.712389) * 0.4).abs() * s.ws;
    for (right, left, y, z) in [
        ("right_hind_leg", "left_hind_leg", a, e),
        ("right_middle_hind_leg", "left_middle_hind_leg", b, f),
        ("right_middle_front_leg", "left_middle_front_leg", c, g),
        ("right_front_leg", "left_front_leg", d, h),
    ] {
        r.p(right).rot.y += y;
        r.p(left).rot.y -= y;
        r.p(right).rot.z += z;
        r.p(left).rot.z -= z;
    }
}

pub fn chicken(r: &mut Rig, s: &St, flap: f32) {
    look(r, s);
    let w = s.wp * 0.6662;
    r.p("right_leg").rot.x = cos(w) * 1.4 * s.ws;
    r.p("left_leg").rot.x = cos(w + PI) * 1.4 * s.ws;
    r.p("right_wing").rot.z = flap;
    r.p("left_wing").rot.z = -flap;
}

pub fn villager_like(r: &mut Rig, s: &St, unhappy: bool) {
    look(r, s);
    if unhappy {
        let h = r.p("head");
        h.rot.z = 0.3 * sin(0.45 * s.age);
        h.rot.x = 0.4;
    } else {
        r.p("head").rot.z = 0.0;
    }
    let w = s.wp * 0.6662;
    r.p("right_leg").rot.x = cos(w) * 1.4 * s.ws * 0.5;
    r.p("left_leg").rot.x = cos(w + PI) * 1.4 * s.ws * 0.5;
    r.p("right_leg").rot.y = 0.0;
    r.p("left_leg").rot.y = 0.0;
}

pub fn witch(r: &mut Rig, s: &St, holding: bool) {
    villager_like(r, s, false);
    let t = 0.01 * (s.id.rem_euclid(10)) as f32;
    let n = r.p("nose");
    n.rot.x = sin(s.age * t) * 4.5 * D;
    n.rot.z = cos(s.age * t) * 2.5 * D;
    if holding {
        n.pos = glam::Vec3::new(0.0, 1.0, -1.5);
        n.rot.x = -0.9;
    }
}

pub fn illager(r: &mut Rig, s: &St, pose: IllagerPose) {
    look(r, s);
    if s.riding {
        for (arm, leg, y, z) in [("right_arm", "right_leg", 0.31415927, 0.07853982), ("left_arm", "left_leg", -0.31415927, -0.07853982)] {
            let a = r.p(arm);
            a.rot = glam::Vec3::new(-0.62831855, 0.0, 0.0);
            let l = r.p(leg);
            l.rot = glam::Vec3::new(-1.4137167, y, z);
        }
    } else {
        let w = s.wp * 0.6662;
        r.p("right_arm").rot = glam::Vec3::new(cos(w + PI) * 2.0 * s.ws * 0.5, 0.0, 0.0);
        r.p("left_arm").rot = glam::Vec3::new(cos(w) * 2.0 * s.ws * 0.5, 0.0, 0.0);
        r.p("right_leg").rot = glam::Vec3::new(cos(w) * 1.4 * s.ws * 0.5, 0.0, 0.0);
        r.p("left_leg").rot = glam::Vec3::new(cos(w + PI) * 1.4 * s.ws * 0.5, 0.0, 0.0);
    }
    let head = r.get("head").rot;
    match pose {
        IllagerPose::AttackBare => {
            zombie_arms(r, s, false);
            bob_arms(r, s.age);
        }
        IllagerPose::Attack => swing_weapon_down(r, s),
        IllagerPose::Spell => {
            for (arm, x, z) in [("right_arm", -5.0, 2.3561945), ("left_arm", 5.0, -2.3561945)] {
                let a = r.p(arm);
                a.pos.z = 0.0;
                a.pos.x = x;
                a.rot = glam::Vec3::new(cos(s.age * 0.6662) * 0.25, 0.0, z);
            }
        }
        IllagerPose::Bow => {
            r.p("right_arm").rot.y = -0.1 + head.y;
            r.p("right_arm").rot.x = -FRAC_PI_2 + head.x;
            r.p("left_arm").rot.x = -0.9424779 + head.x;
            r.p("left_arm").rot.y = head.y - 0.4;
            r.p("left_arm").rot.z = FRAC_PI_2;
        }
        IllagerPose::CrossbowHold => crossbow_hold(r, true),
        IllagerPose::CrossbowCharge(t) => crossbow_charge(r, 25.0, t, true),
        IllagerPose::Celebrate => {
            let a = r.p("right_arm");
            a.pos.z = 0.0;
            a.pos.x = -5.0;
            a.rot = glam::Vec3::new(cos(s.age * 0.6662) * 0.05, 0.0, 2.670354);
            let a = r.p("left_arm");
            a.pos.z = 0.0;
            a.pos.x = 5.0;
            a.rot = glam::Vec3::new(cos(s.age * 0.6662) * 0.05, 0.0, -2.3561945);
        }
        IllagerPose::Crossed => {}
    }
    let crossed = pose == IllagerPose::Crossed;
    r.p("arms").visible = crossed;
    r.p("left_arm").visible = !crossed;
    r.p("right_arm").visible = !crossed;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IllagerPose {
    Crossed,
    AttackBare,
    Attack,
    Spell,
    Bow,
    CrossbowHold,
    CrossbowCharge(f32),
    Celebrate,
}

pub fn feline(r: &mut Rig, s: &St, crouching: bool, sprinting: bool, sitting: bool) {
    let k = s.age_scale;
    if crouching {
        r.p("body").pos.y += k;
        r.p("head").pos.y += 2.0 * k;
        r.p("tail1").pos.y += k;
        r.p("tail2").pos.y -= 4.0 * k;
        r.p("tail2").pos.z += 2.0 * k;
        r.p("tail1").rot.x = FRAC_PI_2;
        r.p("tail2").rot.x = FRAC_PI_2;
    } else if sprinting {
        let y = r.get("tail1").pos.y;
        r.p("tail2").pos.y = y;
        r.p("tail2").pos.z += 2.0 * k;
        r.p("tail1").rot.x = FRAC_PI_2;
        r.p("tail2").rot.x = FRAC_PI_2;
    }
    look(r, s);
    if !sitting {
        r.p("body").rot.x = FRAC_PI_2;
        let w = s.wp * 0.6662;
        if sprinting {
            r.p("left_hind_leg").rot.x = cos(w) * s.ws;
            r.p("right_hind_leg").rot.x = cos(w + 0.3) * s.ws;
            r.p("left_front_leg").rot.x = cos(w + PI + 0.3) * s.ws;
            r.p("right_front_leg").rot.x = cos(w + PI) * s.ws;
            r.p("tail2").rot.x = 1.7278761 + 0.31415927 * cos(s.wp) * s.ws;
        } else {
            r.p("left_hind_leg").rot.x = cos(w) * s.ws;
            r.p("right_hind_leg").rot.x = cos(w + PI) * s.ws;
            r.p("left_front_leg").rot.x = cos(w + PI) * s.ws;
            r.p("right_front_leg").rot.x = cos(w) * s.ws;
            let amp = if crouching { 0.47123894 } else { 0.7853982 };
            r.p("tail2").rot.x = 1.7278761 + amp * cos(s.wp) * s.ws;
        }
    } else {
        r.p("body").rot.x = 0.7853982;
        r.p("body").pos.y -= 4.0 * k;
        r.p("body").pos.z += 5.0 * k;
        r.p("head").pos.y -= 3.3 * k;
        r.p("head").pos.z += k;
        r.p("tail1").pos.y += 8.0 * k;
        r.p("tail1").pos.z -= 2.0 * k;
        r.p("tail2").pos.y += 2.0 * k;
        r.p("tail2").pos.z -= 0.8 * k;
        r.p("tail1").rot.x = 1.7278761;
        r.p("tail2").rot.x = 2.670354;
        for leg in ["left_front_leg", "right_front_leg"] {
            let l = r.p(leg);
            l.rot.x = -0.15707964;
            l.pos.y += 2.0 * k;
            l.pos.z -= 2.0 * k;
        }
        for leg in ["left_hind_leg", "right_hind_leg"] {
            let l = r.p(leg);
            l.rot.x = -FRAC_PI_2;
            l.pos.y += 3.0 * k;
            l.pos.z -= 4.0 * k;
        }
    }
}

pub fn wolf(r: &mut Rig, s: &St, angry: bool, sitting: bool, tail_angle: f32) {
    let w = s.wp * 0.6662;
    r.p("tail").rot.y = if angry { 0.0 } else { cos(w) * 1.4 * s.ws };
    if sitting {
        let k = s.age_scale;
        r.p("body").pos.y += 4.0 * k;
        r.p("body").pos.z -= 2.0 * k;
        r.p("body").rot.x = 0.7853982;
        r.p("tail").pos.y += 9.0 * k;
        r.p("tail").pos.z -= 2.0 * k;
        for leg in ["right_hind_leg", "left_hind_leg"] {
            let l = r.p(leg);
            l.pos.y += 6.7 * k;
            l.pos.z -= 5.0 * k;
            l.rot.x = 4.712389;
        }
        r.p("right_front_leg").rot.x = 5.811947;
        r.p("right_front_leg").pos.x += 0.01 * k;
        r.p("right_front_leg").pos.y += k;
        r.p("left_front_leg").rot.x = 5.811947;
        r.p("left_front_leg").pos.x -= 0.01 * k;
        r.p("left_front_leg").pos.y += k;
        // The neck ruff of grown wolves sits up too.
        if r.has("upper_body") {
            let u = r.p("upper_body");
            u.pos.y += 2.0;
            u.rot.x = 1.2566371;
            u.rot.y = 0.0;
        }
    } else {
        r.p("right_hind_leg").rot.x = cos(w) * 1.4 * s.ws;
        r.p("left_hind_leg").rot.x = cos(w + PI) * 1.4 * s.ws;
        r.p("right_front_leg").rot.x = cos(w + PI) * 1.4 * s.ws;
        r.p("left_front_leg").rot.x = cos(w) * 1.4 * s.ws;
    }
    look(r, s);
    r.p("tail").rot.x = tail_angle;
}

pub fn fox(r: &mut Rig, s: &St, crouching: bool, sleeping: bool, sitting: bool) {
    let w = s.wp * 0.6662;
    r.p("right_hind_leg").rot.x = cos(w) * 1.4 * s.ws;
    r.p("left_hind_leg").rot.x = cos(w + PI) * 1.4 * s.ws;
    r.p("right_front_leg").rot.x = cos(w + PI) * 1.4 * s.ws;
    r.p("left_front_leg").rot.x = cos(w) * 1.4 * s.ws;
    if crouching {
        r.p("body").pos.y += 1.0;
    } else if sleeping {
        r.p("body").rot.z = -FRAC_PI_2;
        r.p("body").pos.y += 5.0;
        r.p("tail").rot.x = -2.6179938;
        r.p("head").pos.x += 2.0;
        r.p("head").pos.y += 2.99;
        r.p("head").rot.y = -2.0943952;
        r.p("head").rot.z = 0.0;
    } else if sitting {
        r.p("body").rot.x = 0.5235988;
        r.p("body").pos.y -= 7.0;
        r.p("body").pos.z += 3.0;
        r.p("tail").rot.x = 0.7853982;
        r.p("tail").pos.z -= 1.0;
        r.p("head").pos.y -= 6.5;
        r.p("head").pos.z += 2.75;
        r.p("right_front_leg").rot.x = -0.2617994;
        r.p("left_front_leg").rot.x = -0.2617994;
        for leg in ["right_hind_leg", "left_hind_leg"] {
            let l = r.p(leg);
            l.rot.x = -1.3089969;
            l.pos.y += 4.0;
            l.pos.z -= 0.25;
        }
    }
    if sleeping {
        let h = r.p("head");
        h.rot.x = 0.0;
        h.rot.y = -2.0943952;
        h.rot.z = cos(s.age * 0.027) / 22.0;
    } else if !crouching {
        look(r, s);
    }
}

pub fn equine(r: &mut Rig, s: &St, eat: f32, stand: f32, tail_swish: bool) {
    let yaw = s.yaw.clamp(-20.0, 20.0);
    let mut pitch = s.pitch * D;
    if s.ws > 0.2 {
        pitch += cos(s.wp * 0.8) * 0.15 * s.ws;
    }
    let inv = 1.0 - stand;
    let k = if s.in_water { 0.2 } else { 1.0 };
    let c = cos(k * s.wp * 0.6662 + PI);
    let front = c * 0.8 * s.ws;
    let neck = 1.0 - stand.max(eat);
    {
        let h = r.p("head_parts");
        h.rot.x = stand * (0.2617994 + pitch) + eat * (2.1816616 + sin(s.age) * 0.05) + neck * (0.5235988 + pitch);
        h.rot.y = stand * yaw * D + neck * (yaw * D);
        h.pos.y += (if eat > 0.0 { 7.0 * eat } else { 0.0 }) + (1.0 - eat) * (-8.0 * stand);
        h.pos.z = h.pos.z + (-4.0 - h.pos.z) * stand;
    }
    let body = r.p("body");
    body.rot.x = stand * -0.7853982 + inv * body.rot.x;
    r.p("left_front_leg").pos.y -= 12.0 * stand;
    r.p("left_front_leg").pos.z += 4.0 * stand;
    let (y, z) = (r.get("left_front_leg").pos.y, r.get("left_front_leg").pos.z);
    r.p("right_front_leg").pos.y = y;
    r.p("right_front_leg").pos.z = z;
    let hind = 0.2617994 * stand;
    let wave = cos(s.age * 0.6 + PI);
    let lift = -1.0471976;
    r.p("left_hind_leg").rot.x = hind - c * 0.5 * s.ws * inv;
    r.p("right_hind_leg").rot.x = hind + c * 0.5 * s.ws * inv;
    r.p("left_front_leg").rot.x = (lift + wave) * stand + front * inv;
    r.p("right_front_leg").rot.x = (lift - wave) * stand - front * inv;
    let t = r.p("tail");
    t.rot.x = 0.5235988 + s.ws * 0.75;
    t.pos.y += s.ws * s.age_scale;
    t.pos.z += s.ws * 2.0 * s.age_scale;
    t.rot.y = if tail_swish { cos(s.age * 0.7) } else { 0.0 };
}

pub fn llama(r: &mut Rig, s: &St, chest: bool) {
    quadruped(r, s, 1.4);
    r.p("right_chest").visible = chest;
    r.p("left_chest").visible = chest;
}

pub fn blaze(r: &mut Rig, s: &St) {
    let mut a = s.age * PI * -0.1;
    for i in 0..4 {
        let p = r.p(PART[i]);
        p.pos.y = -2.0 + cos((i as f32 * 2.0 + s.age) * 0.25);
        p.pos.x = cos(a) * 9.0;
        p.pos.z = sin(a) * 9.0;
        a += FRAC_PI_2;
    }
    let mut a = 0.7853982 + s.age * PI * 0.03;
    for i in 4..8 {
        let p = r.p(PART[i]);
        p.pos.y = 2.0 + cos((i as f32 * 2.0 + s.age) * 0.25);
        p.pos.x = cos(a) * 7.0;
        p.pos.z = sin(a) * 7.0;
        a += FRAC_PI_2;
    }
    let mut a = 0.47123894 + s.age * PI * -0.05;
    for i in 8..12 {
        let p = r.p(PART[i]);
        p.pos.y = 11.0 + cos((i as f32 * 1.5 + s.age) * 0.5);
        p.pos.x = cos(a) * 5.0;
        p.pos.z = sin(a) * 5.0;
        a += FRAC_PI_2;
    }
    look(r, s);
}

const PART: [&str; 12] = ["part0", "part1", "part2", "part3", "part4", "part5", "part6", "part7", "part8", "part9", "part10", "part11"];
const TENTACLE: [&str; 9] = ["tentacle0", "tentacle1", "tentacle2", "tentacle3", "tentacle4", "tentacle5", "tentacle6", "tentacle7", "tentacle8"];
const SEGMENT: [&str; 7] = ["segment0", "segment1", "segment2", "segment3", "segment4", "segment5", "segment6"];
const SPIKE: [&str; 12] = ["spike0", "spike1", "spike2", "spike3", "spike4", "spike5", "spike6", "spike7", "spike8", "spike9", "spike10", "spike11"];

pub fn ghast(r: &mut Rig, s: &St) {
    for (i, t) in TENTACLE.iter().enumerate() {
        r.p(t).rot.x = 0.2 * sin(s.age * 0.3 + i as f32) + 0.4;
    }
}

pub fn squid(r: &mut Rig, angle: f32) {
    for t in &TENTACLE[..8] {
        r.p(t).rot.x = angle;
    }
}

pub fn silverfish(r: &mut Rig, s: &St) {
    for (i, name) in SEGMENT.iter().enumerate() {
        let phase = s.age * 0.9 + i as f32 * 0.15 * PI;
        let a = (i as i32 - 2).abs() as f32;
        let p = r.p(name);
        p.rot.y = cos(phase) * PI * 0.05 * (1.0 + a);
        p.pos.x = sin(phase) * PI * 0.2 * a;
    }
    let (two, four, one) = (r.get("segment2"), r.get("segment4"), r.get("segment1"));
    r.p("layer0").rot.y = two.rot.y;
    r.p("layer1").rot.y = four.rot.y;
    r.p("layer1").pos.x = four.pos.x;
    r.p("layer2").rot.y = one.rot.y;
    r.p("layer2").pos.x = one.pos.x;
}

pub fn endermite(r: &mut Rig, s: &St) {
    for (i, name) in SEGMENT[..4].iter().enumerate() {
        let phase = s.age * 0.9 + i as f32 * 0.15 * PI;
        let a = (i as i32 - 2).abs() as f32;
        let p = r.p(name);
        p.rot.y = cos(phase) * PI * 0.01 * (1.0 + a);
        p.pos.x = sin(phase) * PI * 0.1 * a;
    }
}

pub fn snow_golem(r: &mut Rig, s: &St) {
    look(r, s);
    let y = s.yaw * D * 0.25;
    r.p("upper_body").rot.y = y;
    let (sn, cs) = (sin(y), cos(y));
    let a = r.p("left_arm");
    a.rot.y = y;
    a.pos.x = cs * 5.0;
    a.pos.z = -sn * 5.0;
    let a = r.p("right_arm");
    a.rot.y = y + PI;
    a.pos.x = -cs * 5.0;
    a.pos.z = sn * 5.0;
}

pub fn guardian(r: &mut Rig, s: &St, spikes_out: f32, tail: f32) {
    look(r, s);
    const X: [f32; 12] = [0.0, 0.0, 8.0, -8.0, -8.0, 8.0, 8.0, -8.0, 0.0, 0.0, 8.0, -8.0];
    const Y: [f32; 12] = [-8.0, -8.0, -8.0, -8.0, 0.0, 0.0, 0.0, 0.0, 8.0, 8.0, 8.0, 8.0];
    const Z: [f32; 12] = [8.0, -8.0, 0.0, 0.0, -8.0, -8.0, 8.0, 8.0, 8.0, -8.0, 0.0, 0.0];
    let f = (1.0 - spikes_out) * 0.55;
    for (i, name) in SPIKE.iter().enumerate() {
        let k = 1.0 + cos(s.age * 1.5 + i as f32) * 0.01 - f;
        r.p(name).pos = glam::Vec3::new(X[i] * k, 16.0 + Y[i] * k, Z[i] * k);
    }
    r.p("eye").visible = true;
    let v = sin(tail) * PI;
    r.p("tail0").rot.y = v * 0.05;
    r.p("tail1").rot.y = v * 0.1;
    r.p("tail2").rot.y = v * 0.15;
}

pub fn bee(r: &mut Rig, s: &St, on_ground: bool, angry: bool, stinger: bool) {
    r.p("stinger").visible = stinger;
    if !on_ground {
        let a = s.age * 120.32113 * D;
        let w = r.p("right_wing");
        w.rot.y = 0.0;
        w.rot.z = cos(a) * PI * 0.15;
        let (x, y, z) = (w.rot.x, w.rot.y, w.rot.z);
        let w = r.p("left_wing");
        w.rot.x = x;
        w.rot.y = y;
        w.rot.z = -z;
        r.p("front_legs").rot.x = 0.7853982;
        r.p("middle_legs").rot.x = 0.7853982;
        r.p("back_legs").rot.x = 0.7853982;
    }
    if !angry && !on_ground {
        let v = cos(s.age * 0.18);
        r.p("bone").rot.x = 0.1 + v * PI * 0.025;
        r.p("bone").pos.y -= cos(s.age * 0.18) * 0.9;
        r.p("front_legs").rot.x = -v * PI * 0.1 + 0.3926991;
        r.p("back_legs").rot.x = -v * PI * 0.05 + 0.7853982;
    }
}

pub fn phantom(r: &mut Rig, flap: f32) {
    let a = flap * 7.448451 * D;
    let z = cos(a) * 16.0 * D;
    r.p("left_wing_base").rot.z = z;
    r.p("left_wing_tip").rot.z = z;
    r.p("right_wing_base").rot.z = -z;
    r.p("right_wing_tip").rot.z = -z;
    let tail = -(5.0 + cos(a * 2.0) * 5.0) * D;
    r.p("tail_base").rot.x = tail;
    r.p("tail_tip").rot.x = tail;
}

pub fn dolphin(r: &mut Rig, s: &St, moving: bool) {
    let b = r.p("body");
    b.rot.x = s.pitch * D;
    b.rot.y = s.yaw * D;
    if moving {
        b.rot.x += -0.05 - 0.05 * cos(s.age * 0.3);
        r.p("tail").rot.x = -0.1 * cos(s.age * 0.3);
        r.p("tail_fin").rot.x = -0.2 * cos(s.age * 0.3);
    }
}

pub fn fish_tail(r: &mut Rig, s: &St, part: &str, amp: f32, speed: f32) {
    let k = if s.in_water { 1.0 } else { 1.5 };
    r.p(part).rot.y = -k * amp * sin(speed * s.age);
}

pub fn turtle(r: &mut Rig, s: &St, on_land: bool, laying: bool, has_egg: bool) {
    r.p("egg_belly").visible = has_egg;
    if has_egg {
        r.p("root").pos.y -= 1.0;
    }
    if on_land {
        let (a, b) = if laying { (4.0, 2.0) } else { (1.0, 1.0) };
        let t = s.wp * 5.0;
        let f = cos(a * t);
        let h = cos(t);
        r.p("right_front_leg").rot.y = -f * 8.0 * s.ws * b;
        r.p("left_front_leg").rot.y = f * 8.0 * s.ws * b;
        r.p("right_hind_leg").rot.y = -h * 3.0 * s.ws;
        r.p("left_hind_leg").rot.y = h * 3.0 * s.ws;
    } else {
        let v = cos(s.wp * 0.6662 * 0.6) * 0.5 * s.ws;
        r.p("right_hind_leg").rot.x = v;
        r.p("left_hind_leg").rot.x = -v;
        r.p("right_front_leg").rot.z = -v;
        r.p("left_front_leg").rot.z = v;
    }
}

pub fn strider(r: &mut Rig, s: &St, ridden: bool) {
    let ws = s.ws.min(0.25);
    let b = r.p("body");
    if ridden {
        b.rot.x = 0.0;
        b.rot.y = 0.0;
    } else {
        b.rot.x = s.pitch * D;
        b.rot.y = s.yaw * D;
    }
    b.rot.z = 0.1 * sin(s.wp * 1.5) * 4.0 * ws;
    let w = s.wp * 1.5 * 0.5;
    r.p("left_leg").rot.x = sin(w) * 2.0 * ws;
    r.p("right_leg").rot.x = sin(w + PI) * 2.0 * ws;
    r.p("left_leg").rot.z = 0.17453292 * cos(w) * ws;
    r.p("right_leg").rot.z = 0.17453292 * cos(w + PI) * ws;
    let wave = cos(s.wp * 1.5 + PI) * ws;
    let sway = |k: f32, f: f32| wave * k + f;
    let (a, m, l) = (sway(0.6, 0.1 * sin(s.age * 0.4)), sway(1.2, 0.1 * sin(s.age * 0.2)), sway(1.3, 0.05 * sin(s.age * -0.4)));
    if s.baby {
        r.p("body").pos.y = 17.25 - cos(s.wp * 1.5) * 2.0 * ws;
        r.p("left_leg").pos.y = 20.0 + 2.0 * sin(w + PI) * 2.0 * ws;
        r.p("right_leg").pos.y = 20.0 + 2.0 * sin(w) * 2.0 * ws;
        r.p("bristle0").rot.x += a;
        r.p("bristle1").rot.x += m;
        r.p("bristle2").rot.x += l;
    } else {
        r.p("right_bottom_bristle").rot.z = -1.2217305 + l;
        r.p("right_middle_bristle").rot.z = -1.134464 + m;
        r.p("right_top_bristle").rot.z = -0.87266463 + a;
        r.p("left_top_bristle").rot.z = 0.87266463 - a;
        r.p("left_middle_bristle").rot.z = 1.134464 - m;
        r.p("left_bottom_bristle").rot.z = 1.2217305 - l;
        r.p("body").pos.y = 2.0 - 2.0 * cos(s.wp * 1.5) * 2.0 * ws;
        r.p("left_leg").pos.y = 8.0 + 2.0 * sin(w + PI) * 2.0 * ws;
        r.p("right_leg").pos.y = 8.0 + 2.0 * sin(w) * 2.0 * ws;
    }
}

/// A parrot sitting on someone's shoulder.
pub fn parrot_on_shoulder(r: &mut Rig) {
    r.p("left_wing").rot.z = -0.0873;
    r.p("right_wing").rot.z = 0.0873;
}

/// A parrot told to sit.
pub fn parrot_sitting(r: &mut Rig, s: &St) {
    look(r, s);
    for part in ["head", "tail", "body", "left_wing", "right_wing", "left_leg", "right_leg"] {
        r.p(part).pos.y += 1.9;
    }
    r.p("tail").rot.x += 0.5235988;
    r.p("left_wing").rot.z = -0.0873;
    r.p("right_wing").rot.z = 0.0873;
    r.p("left_leg").rot.x += 1.5707964;
    r.p("right_leg").rot.x += 1.5707964;
}

pub fn parrot(r: &mut Rig, s: &St, flying: bool, flap: f32) {
    look(r, s);
    if flying {
        r.p("left_leg").rot.x += 0.6981317;
        r.p("right_leg").rot.x += 0.6981317;
    } else {
        let w = s.wp * 0.6662;
        r.p("left_leg").rot.x += cos(w) * 1.4 * s.ws;
        r.p("right_leg").rot.x += cos(w + PI) * 1.4 * s.ws;
    }
    let v = flap * 0.3;
    r.p("head").pos.y += v;
    r.p("tail").rot.x += cos(s.wp * 0.6662) * 0.3 * s.ws;
    r.p("tail").pos.y += v;
    r.p("body").pos.y += v;
    r.p("left_wing").rot.z = -0.0873 - flap;
    r.p("left_wing").pos.y += v;
    r.p("right_wing").rot.z = 0.0873 + flap;
    r.p("right_wing").pos.y += v;
    r.p("left_leg").pos.y += v;
    r.p("right_leg").pos.y += v;
}

pub fn wither(r: &mut Rig, s: &St, heads: [(f32, f32); 2]) {
    let v = cos(s.age * 0.1);
    let rib = (0.065 + 0.05 * v) * PI;
    r.p("ribcage").rot.x = rib;
    let t = r.p("tail");
    t.pos = glam::Vec3::new(-2.0, 6.9 + cos(rib) * 10.0, -0.5 + sin(rib) * 10.0);
    t.rot.x = (0.265 + 0.1 * v) * PI;
    let c = r.p("center_head");
    c.rot.y = s.yaw * D;
    c.rot.x = s.pitch * D;
    r.p("right_head").rot.y = (heads[0].0 - s.body) * D;
    r.p("right_head").rot.x = heads[0].1 * D;
    r.p("left_head").rot.y = (heads[1].0 - s.body) * D;
    r.p("left_head").rot.x = heads[1].1 * D;
}

pub fn warden(r: &mut Rig, s: &St, tendril: f32) {
    {
        let h = r.p("head");
        h.rot.x = s.pitch * D;
        h.rot.y = s.yaw * D;
    }
    let walk = (3.0 * s.ws).min(0.5);
    let w = s.wp * 0.8662;
    let (c, sn) = (cos(w), sin(w));
    let small = walk.min(0.35);
    r.p("head").rot.z += 0.3 * sn * walk;
    r.p("head").rot.x += 1.2 * cos(w + FRAC_PI_2) * small;
    r.p("body").rot.z = 0.1 * sn * walk;
    r.p("body").rot.x = c * small;
    r.p("left_leg").rot.x = c * walk;
    r.p("right_leg").rot.x = cos(w + PI) * walk;
    r.p("left_arm").rot.x = -(0.8 * c * walk);
    r.p("left_arm").rot.z = 0.0;
    r.p("right_arm").rot.x = -(0.8 * sn * walk);
    r.p("right_arm").rot.z = 0.0;
    let t = s.age * 0.1;
    let (c, sn) = (cos(t), sin(t));
    r.p("head").rot.z += 0.06 * c;
    r.p("head").rot.x += 0.06 * sn;
    r.p("body").rot.z += 0.025 * sn;
    r.p("body").rot.x += 0.025 * c;
    let v = tendril * ((s.age * 2.25).cos() * PI * 0.1);
    r.p("left_tendril").rot.x = v;
    r.p("right_tendril").rot.x = -v;
}

pub fn panda(r: &mut Rig, s: &St, sit: f32, roll: f32, back: f32) {
    quadruped(r, s, 1.4);
    if sit > 0.0 {
        let b = r.p("body");
        b.rot.x = rot_lerp_rad(sit, b.rot.x, 1.7407963);
        let h = r.p("head");
        h.rot.x = rot_lerp_rad(sit, h.rot.x, FRAC_PI_2);
        r.p("right_front_leg").rot.z = -0.27079642;
        r.p("left_front_leg").rot.z = 0.27079642;
        r.p("right_hind_leg").rot.z = 0.5707964;
        r.p("left_hind_leg").rot.z = -0.5707964;
    }
    if back > 0.0 {
        r.p("right_hind_leg").rot.x = -0.6 * sin(s.age * 0.15);
        r.p("left_hind_leg").rot.x = 0.6 * sin(s.age * 0.15);
        r.p("right_front_leg").rot.x = 0.3 * sin(s.age * 0.25);
        r.p("left_front_leg").rot.x = -0.3 * sin(s.age * 0.25);
        let h = r.p("head");
        h.rot.x = rot_lerp_rad(back, h.rot.x, FRAC_PI_2);
    }
    if roll > 0.0 {
        let h = r.p("head");
        h.rot.x = rot_lerp_rad(roll, h.rot.x, 2.0561945);
        r.p("right_hind_leg").rot.x = -0.5 * sin(s.age * 0.5);
        r.p("left_hind_leg").rot.x = 0.5 * sin(s.age * 0.5);
        r.p("right_front_leg").rot.x = 0.5 * sin(s.age * 0.5);
        r.p("left_front_leg").rot.x = -0.5 * sin(s.age * 0.5);
    }
}

pub fn polar_bear(r: &mut Rig, s: &St, stand: f32) {
    quadruped(r, s, 1.4);
    let v = stand * stand;
    let k = s.age_scale;
    r.p("body").rot.x -= v * PI * 0.35;
    r.p("body").pos.y += v * k * 2.0;
    r.p("right_front_leg").pos.y -= v * k * 20.0;
    r.p("right_front_leg").pos.z += v * k * 4.0;
    r.p("right_front_leg").rot.x -= v * PI * 0.45;
    let (y, z) = (r.get("right_front_leg").pos.y, r.get("right_front_leg").pos.z);
    r.p("left_front_leg").pos.y = y;
    r.p("left_front_leg").pos.z = z;
    r.p("left_front_leg").rot.x -= v * PI * 0.45;
    r.p("head").pos.y -= v * 24.0;
    r.p("head").pos.z += v * 13.0;
    r.p("head").rot.x += v * PI * 0.15;
}

pub fn hoglin(r: &mut Rig, s: &St, attack_ticks: f32) {
    r.p("right_ear").rot.z = -0.6981317 - s.ws * sin(s.wp);
    r.p("left_ear").rot.z = 0.6981317 + s.ws * sin(s.wp);
    r.p("head").rot.y = s.yaw * D;
    // The head stays low, and swings up to butt.
    let t = 1.0 - (10.0 - 2.0 * attack_ticks).abs() / 10.0;
    r.p("head").rot.x = 0.87266463 + (-0.34906584 - 0.87266463) * t;
    r.p("right_front_leg").rot.x = cos(s.wp) * 1.2 * s.ws;
    r.p("left_front_leg").rot.x = cos(s.wp + PI) * 1.2 * s.ws;
    let (a, b) = (r.get("left_front_leg").rot.x, r.get("right_front_leg").rot.x);
    r.p("right_hind_leg").rot.x = a;
    r.p("left_hind_leg").rot.x = b;
}

pub fn ravager(r: &mut Rig, s: &St, attack_ticks: f32, stunned: f32, roar: f32) {
    let neck_x = r.get("neck").rot.x;
    if attack_ticks > 0.0 {
        let v = (1.0 + triangle_wave(attack_ticks, 10.0)) * 0.5;
        let reach = v * v * v * 12.0;
        let n = r.p("neck");
        n.pos.z = -6.5 + reach;
        n.pos.y = -7.0 - reach * sin(neck_x);
        let m = r.p("mouth");
        m.rot.x = if attack_ticks > 5.0 { sin((-4.0 + attack_ticks) / 4.0) * PI * 0.4 } else { 0.15707964 * sin(PI * attack_ticks / 10.0) };
    } else {
        let n = r.p("neck");
        n.pos.x = 0.0;
        n.pos.y = -7.0 + sin(neck_x);
        n.pos.z = 5.5;
        let is_stunned = stunned > 0.0;
        n.rot.x = if is_stunned { 0.21991149 } else { 0.0 };
        r.p("mouth").rot.x = PI * if is_stunned { 0.05 } else { 0.01 };
        if is_stunned {
            r.p("neck").pos.x = ((stunned as f64 / 40.0 * 10.0).sin() * 3.0) as f32;
        } else if roar > 0.0 {
            r.p("mouth").rot.x = FRAC_PI_2 * sin(roar * PI * 0.25);
        }
    }
    look(r, s);
    let v = 0.4 * s.ws;
    let w = s.wp * 0.6662;
    r.p("right_hind_leg").rot.x = cos(w) * v;
    r.p("left_hind_leg").rot.x = cos(w + PI) * v;
    r.p("right_front_leg").rot.x = cos(w + PI) * v;
    r.p("left_front_leg").rot.x = cos(w) * v;
}

pub fn shulker(r: &mut Rig, s: &St, peek: f32, head_yaw: f32) {
    let v = (0.5 + peek) * PI;
    let w = -1.0 + sin(v);
    let wobble = if v > PI { sin(s.age * 0.1) * 0.7 } else { 0.0 };
    let l = r.p("lid");
    l.pos = glam::Vec3::new(0.0, 16.0 + sin(v) * 8.0 + wobble, 0.0);
    l.rot.y = if peek > 0.3 { w * w * w * w * PI * 0.125 } else { 0.0 };
    let h = r.p("head");
    h.rot.x = s.pitch * D;
    h.rot.y = (head_yaw - 180.0 - s.body) * D;
}

pub fn vex(r: &mut Rig, s: &St, charging: bool) {
    look(r, s);
    let v = cos(s.age * 5.5 * D) * 0.1;
    r.p("right_arm").rot.z = 0.62831855 + v;
    r.p("left_arm").rot.z = -(0.62831855 + v);
    r.p("body").rot.x = if charging { 0.0 } else { 0.15707964 };
    let wing = 1.0995574 + cos(s.age * 45.836624 * D) * D * 16.2;
    let w = r.p("left_wing");
    w.rot = glam::Vec3::new(0.47123888, wing, -0.47123888);
    let w = r.p("right_wing");
    w.rot = glam::Vec3::new(0.47123888, -wing, 0.47123888);
}

pub fn allay(r: &mut Rig, s: &St, holding: f32) {
    let flap = s.age * 20.0 * D + s.wp;
    let wing = cos(flap) * PI * 0.15 + s.ws;
    let bob = s.age * 9.0 * D;
    let forward = (s.ws / 0.3).min(1.0);
    let idle = 1.0 - forward;
    look(r, s);
    let w = r.p("right_wing");
    w.rot.x = 0.43633232 * idle;
    w.rot.y = -0.7853982 + wing;
    let w = r.p("left_wing");
    w.rot.x = 0.43633232 * idle;
    w.rot.y = 0.7853982 - wing;
    r.p("body").rot.x = forward * 0.7853982;
    let arm = holding * (-1.0471976 + (-1.134464 - -1.0471976) * forward);
    r.p("root").pos.y += bob.cos() * 0.25 * idle;
    r.p("right_arm").rot.x = arm;
    r.p("left_arm").rot.x = arm;
    let k = idle * (1.0 - holding);
    let z = 0.43633232 - cos(bob + 4.712389) * PI * 0.075 * k;
    r.p("left_arm").rot.z = -z;
    r.p("right_arm").rot.z = z;
    r.p("right_arm").rot.y = 0.27925268 * holding;
    r.p("left_arm").rot.y = -0.27925268 * holding;
}

/// Grown axolotls: the blend of swimming, hovering, crawling and lying still.
pub fn axolotl(r: &mut Rig, s: &St, moving: f32, in_water: f32, on_ground: f32, dead: f32) {
    let still = 1.0 - moving;
    let mirror = 1.0 - on_ground.min(moving);
    r.p("body").rot.y += s.yaw * D;
    let (swim_w, hover, crawl, rest) = (moving.min(in_water), still.min(in_water), moving.min(on_ground), still.min(on_ground));
    if swim_w > 0.0 {
        let t = s.age * 0.33;
        let (sn, cs) = (sin(t), cos(t));
        let v = 0.13 * sn;
        r.p("body").rot.x += (s.pitch * D + v) * swim_w;
        r.p("head").rot.x -= v * 1.8 * swim_w;
        r.p("body").pos.y -= 0.45 * cs * swim_w;
        r.p("top_gills").rot.x += (-0.5 * sn - 0.8) * swim_w;
        let g = (0.3 * sn + 0.9) * swim_w;
        r.p("left_gills").rot.y += g;
        r.p("right_gills").rot.y -= g;
        r.p("tail").rot.y += 0.3 * cos(t * 0.9) * swim_w;
        let l = r.p("left_hind_leg");
        l.rot.x += 1.8849558 * swim_w;
        l.rot.y += -0.4 * sn * swim_w;
        l.rot.z += FRAC_PI_2 * swim_w;
        let l = r.p("left_front_leg");
        l.rot.x += 1.8849558 * swim_w;
        l.rot.y += (-0.2 * cs - 0.1) * swim_w;
        l.rot.z += FRAC_PI_2 * swim_w;
    }
    if hover > 0.0 {
        let t = s.age * 0.075;
        let cs = cos(t);
        let sn = sin(t) * 0.15;
        let v = (-0.15 + 0.075 * cs) * hover;
        r.p("body").rot.x += v;
        r.p("body").pos.y -= sn * hover;
        r.p("head").rot.x -= v;
        r.p("top_gills").rot.x += 0.2 * cs * hover;
        let g = (-0.3 * cs - 0.19) * hover;
        r.p("left_gills").rot.y += g;
        r.p("right_gills").rot.y -= g;
        let l = r.p("left_hind_leg");
        l.rot.x += (2.3561945 - cs * 0.11) * hover;
        l.rot.y += 0.47123894 * hover;
        l.rot.z += 1.7278761 * hover;
        let l = r.p("left_front_leg");
        l.rot.x += (0.7853982 - cs * 0.2) * hover;
        l.rot.y += 2.042035 * hover;
        r.p("tail").rot.y += 0.5 * cs * hover;
    }
    if crawl > 0.0 {
        let t = s.age * 0.11;
        let cs = cos(t);
        let v = (cs * cs - 2.0 * cs) / 5.0;
        let u = 0.7 * cs;
        let w = 0.09 * cs * crawl;
        r.p("head").rot.y += w;
        r.p("tail").rot.y += w;
        let g = (0.6 - 0.08 * (cs * cs + 2.0 * sin(t))) * crawl;
        r.p("top_gills").rot.x += g;
        r.p("left_gills").rot.y -= g;
        r.p("right_gills").rot.y += g;
        let (a, b) = (0.9424779 * crawl, 1.0995574 * crawl);
        let l = r.p("left_hind_leg");
        l.rot.x += a;
        l.rot.y += (1.5 - v) * crawl;
        l.rot.z += -0.1 * crawl;
        let l = r.p("left_front_leg");
        l.rot.x += b;
        l.rot.y += (FRAC_PI_2 - u) * crawl;
        let l = r.p("right_hind_leg");
        l.rot.x += a;
        l.rot.y += (-1.0 - v) * crawl;
        let l = r.p("right_front_leg");
        l.rot.x += b;
        l.rot.y += (-FRAC_PI_2 - u) * crawl;
    }
    if rest > 0.0 {
        let t = s.age * 0.09;
        let (sn, cs) = (sin(t), cos(t));
        let a = sn * sn - 2.0 * sn;
        let b = cs * cs - 3.0 * sn;
        r.p("head").rot.x += -0.09 * a * rest;
        r.p("head").rot.z += -0.2 * rest;
        r.p("tail").rot.y += (-0.1 + 0.1 * a) * rest;
        let g = (0.6 + 0.05 * b) * rest;
        r.p("top_gills").rot.x += g;
        r.p("left_gills").rot.y -= g;
        r.p("right_gills").rot.y += g;
        let l = r.p("left_hind_leg");
        l.rot.x += 1.1 * rest;
        l.rot.y += rest;
        let l = r.p("left_front_leg");
        l.rot.x += 0.8 * rest;
        l.rot.y += 2.3 * rest;
        l.rot.z -= 0.5 * rest;
    }
    if dead > 0.0 {
        let l = r.p("left_hind_leg");
        l.rot.x += 1.4137167 * dead;
        l.rot.y += 1.0995574 * dead;
        l.rot.z += 0.7853982 * dead;
        let l = r.p("left_front_leg");
        l.rot.x += 0.7853982 * dead;
        l.rot.y += 2.042035 * dead;
        r.p("body").rot.x += -0.15 * dead;
        r.p("body").rot.z += 0.35 * dead;
    }
    if mirror > 1e-5 {
        let (h, f) = (r.get("left_hind_leg").rot, r.get("left_front_leg").rot);
        let l = r.p("right_hind_leg");
        l.rot.x += h.x * mirror;
        l.rot.y += -h.y * mirror;
        l.rot.z += -h.z * mirror;
        let l = r.p("right_front_leg");
        l.rot.x += f.x * mirror;
        l.rot.y += -f.y * mirror;
        l.rot.z += -f.z * mirror;
    }
}

pub fn piglin_ears(r: &mut Rig, s: &St, default_degrees: f32) {
    let base = default_degrees * D;
    let t = s.age * 0.1 + s.wp * 0.5;
    let v = 0.08 + s.ws * 0.4;
    r.p("left_ear").rot.z = -base - cos(t * 1.2) * v;
    r.p("right_ear").rot.z = base + cos(t) * v;
}
