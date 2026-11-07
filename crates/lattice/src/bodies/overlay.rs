//! What covers the view when the eyes are somewhere they shouldn't be: inside a block, under
//! water, or on fire. Drawn over everything else, in the same space as the held item.

use super::Ctx;
use crate::game::Session;
use glam::{Mat4, Vec3};
use lattice_data::flags;
use lattice_render::atlas::Sprite;
use lattice_render::mesher::Vertex;
use lattice_render::models::BlockModels;
use lattice_world::World;

fn quad(list: &mut Vec<Vertex>, m: Mat4, x: [f32; 2], y: [f32; 2], z: f32, u: [f32; 2], v: [f32; 2], color: [u8; 4]) {
    let full = [255, 255, 0, 0];
    for (px, py, tu, tv) in [(x[0], y[0], u[0], v[0]), (x[1], y[0], u[1], v[0]), (x[1], y[1], u[1], v[1]), (x[0], y[1], u[0], v[1])] {
        list.push(Vertex { pos: m.transform_point3(Vec3::new(px, py, z)).to_array(), uv: [tu, tv], color, light: full });
    }
}

/// The first solid block around the eyes, the way the game looks for one.
fn blocking_state(session: &Session, world: &World) -> Option<u32> {
    let b = &session.player.body;
    if b.no_physics || b.spectator {
        return None;
    }
    let data = lattice_data::game();
    let eye = b.pos.y + b.eye_height() as f64;
    let width = b.aabb().max.x - b.aabb().min.x;
    for i in 0..8 {
        let off = |bit: i32| ((i >> bit) % 2) as f64 - 0.5;
        let x = b.pos.x + off(0) * width * 0.8;
        let y = eye + off(1) * 0.1;
        let z = b.pos.z + off(2) * width * 0.8;
        let Some(s) = world.block(x.floor() as i32, y.floor() as i32, z.floor() as i32) else { continue };
        let info = data.state(s);
        if info.has(flags::SUFFOCATING) && !info.has(flags::INVISIBLE) {
            return Some(s);
        }
    }
    None
}

pub fn draw(ctx: &mut Ctx, session: &Session, world: &World, first_person: bool, look: (f32, f32), fire: Sprite, models: &BlockModels) {
    if session.player.body.pose == lattice_world::physics::Pose::Sleeping {
        return;
    }
    if let Some(state) = blocking_state(session, world) {
        let s = models.particle(state);
        quad(&mut ctx.out.overlay_blocks, Mat4::IDENTITY, [-1.0, 1.0], [-1.0, 1.0], -0.5, [s.u0, s.u1], [s.v0, s.v1], [0x19, 0x19, 0x19, 255]);
    }
    if !first_person || session.player.game_mode == 3 {
        return;
    }
    let b = &session.player.body;
    if b.eye_in_water {
        if let Some(s) = ctx.bodies.sheet.try_sprite("misc/underwater") {
            let eye = b.pos + glam::DVec3::new(0.0, b.eye_height() as f64, 0.0);
            let light = super::light_at(world, eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32);
            let bright = light[0].max(light[1]);
            let color = [bright, bright, bright, (0.1f32 * 255.0) as u8];
            // The picture repeats four times across the view and drifts as the head turns. It
            // can't wrap on the sheet, so each repeat is its own quad.
            let (u_off, v_off) = (-look.0 / 64.0, look.1 / 64.0);
            let list = &mut ctx.out.overlay;
            for ku in u_off.floor() as i32..=(u_off + 4.0).floor() as i32 {
                let (x0, x1) = ((u_off + 2.0 - ku as f32 - 1.0) / 2.0, (u_off + 2.0 - ku as f32) / 2.0);
                for kv in v_off.floor() as i32..=(v_off + 4.0).floor() as i32 {
                    let (y0, y1) = ((v_off + 2.0 - kv as f32 - 1.0) / 2.0, (v_off + 2.0 - kv as f32) / 2.0);
                    quad(list, Mat4::IDENTITY, [x0, x1], [y0, y1], -0.5, [s.u1, s.u0], [s.v1, s.v0], color);
                }
            }
        }
    }
    if session.self_flags & 1 != 0 {
        // Two sheets of flame, angled in from either side.
        let color = [255, 255, 255, 0xE5];
        for (x, angle) in [(0.24f32, -0.17453292f32), (-0.24, 0.17453292)] {
            let m = Mat4::from_translation(Vec3::new(x, -0.3, 0.0)) * Mat4::from_rotation_y(angle);
            quad(&mut ctx.out.overlay_blocks, m, [-0.5, 0.5], [-0.5, 0.5], -0.5, [fire.u1, fire.u0], [fire.v1, fire.v0], color);
        }
    }
}
