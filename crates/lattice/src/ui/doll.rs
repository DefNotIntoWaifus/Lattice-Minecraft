//! The player standing in their inventory, turning to look at the pointer. A plain
//! set of boxes textured from their skin, in the skin format's standard layout.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Painter, Pos2, Rect, Shape, TextureId, pos2};
use glam::{Mat3, Vec3};

struct Part {
    /// Where it hangs from, in skin pixels from the feet; y is up, z towards you.
    pivot: Vec3,
    /// Its middle, relative to that.
    centre: Vec3,
    size: Vec3,
    uv: (f32, f32),
    grow: f32,
    head: bool,
}

/// One face of a box, in order around it from the top-left corner as seen from outside.
fn face(a: Vec3, b: Vec3, c: Vec3, d: Vec3, uv: (f32, f32), wh: (f32, f32)) -> ([Vec3; 4], [(f32, f32); 4]) {
    let (u, v) = uv;
    let (w, h) = wh;
    ([a, b, c, d], [(u, v), (u + w, v), (u + w, v + h), (u, v + h)])
}

fn parts(slim: bool) -> Vec<Part> {
    let arm = if slim { 3.0 } else { 4.0 };
    let reach = 4.0 + arm / 2.0;
    let box_of = |pivot: Vec3, centre: Vec3, size: Vec3, uv: (f32, f32), grow: f32, head: bool| Part { pivot, centre, size, uv, grow, head };
    let neck = Vec3::new(0.0, 24.0, 0.0);
    let (right, left) = (Vec3::new(-reach, 22.0, 0.0), Vec3::new(reach, 22.0, 0.0));
    let (hip_r, hip_l) = (Vec3::new(-2.0, 12.0, 0.0), Vec3::new(2.0, 12.0, 0.0));
    let (limb, arm_size, trunk) = (Vec3::new(4.0, 12.0, 4.0), Vec3::new(arm, 12.0, 4.0), Vec3::new(8.0, 12.0, 4.0));
    let hang = Vec3::new(0.0, -6.0, 0.0);
    let arm_hang = Vec3::new(0.0, -4.0, 0.0);
    vec![
        box_of(hip_r, hang, limb, (0.0, 16.0), 0.0, false),
        box_of(hip_r, hang, limb, (0.0, 32.0), 0.25, false),
        box_of(hip_l, hang, limb, (16.0, 48.0), 0.0, false),
        box_of(hip_l, hang, limb, (0.0, 48.0), 0.25, false),
        box_of(neck, Vec3::new(0.0, -6.0, 0.0), trunk, (16.0, 16.0), 0.0, false),
        box_of(neck, Vec3::new(0.0, -6.0, 0.0), trunk, (16.0, 32.0), 0.25, false),
        box_of(right, arm_hang, arm_size, (40.0, 16.0), 0.0, false),
        box_of(right, arm_hang, arm_size, (40.0, 32.0), 0.25, false),
        box_of(left, arm_hang, arm_size, (32.0, 48.0), 0.0, false),
        box_of(left, arm_hang, arm_size, (48.0, 48.0), 0.25, false),
        box_of(neck, Vec3::new(0.0, 4.0, 0.0), Vec3::splat(8.0), (0.0, 0.0), 0.0, true),
        box_of(neck, Vec3::new(0.0, 4.0, 0.0), Vec3::splat(8.0), (32.0, 0.0), 0.5, true),
    ]
}

/// Draws the figure filling `area`, facing `pointer`.
pub fn draw(p: &Painter, area: Rect, texture: TextureId, slim: bool, pointer: Option<Pos2>, worn: &[Worn]) {
    let scale = (area.height() / 34.0).min(area.width() / 18.0);
    let middle = pos2(area.center().x, area.bottom() - (area.height() - 32.0 * scale) / 2.0);
    let chest = pos2(area.center().x, middle.y - 24.0 * scale);
    let (mx, my) = pointer.map_or((0.0, 0.0), |q| (q.x - chest.x, q.y - chest.y));
    let body_turn = (mx / 150.0).atan() * 0.45;
    let head_turn = (mx / 90.0).atan() * 0.9;
    // A head can't look further down than a neck lets it.
    let head_tilt = ((my / 120.0).atan() * 0.5).clamp(-0.45, 0.45);
    let body = Mat3::from_rotation_y(body_turn);
    let head = Mat3::from_rotation_y(head_turn) * Mat3::from_rotation_x(head_tilt);

    // Whatever's furthest away goes first; each box then only shows faces towards you.
    let mut order: Vec<(f32, usize, Part)> = parts(slim)
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let turn = if part.head { head } else { Mat3::IDENTITY };
            let z = (body * (part.pivot + turn * part.centre)).z;
            (z, i, part)
        })
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));

    let view = View { body, head, chest, scale };
    let mut mesh = Mesh::with_texture(texture);
    for (_, _, part) in &order {
        add_part(&mut mesh, part, &view, 64.0, Color32::WHITE);
    }
    p.add(Shape::mesh(mesh));
    // What's worn goes round the body, in the same pose.
    for piece in worn {
        let mut mesh = Mesh::with_texture(piece.texture);
        for part in armour_parts(piece.kind, slim) {
            add_part(&mut mesh, &part, &view, 32.0, piece.tint);
        }
        p.add(Shape::mesh(mesh));
    }
}

struct View {
    body: Mat3,
    head: Mat3,
    chest: Pos2,
    scale: f32,
}

/// Which part of the body a piece of armour goes on.
#[derive(Clone, Copy, PartialEq)]
pub enum ArmourKind {
    Head,
    Chest,
    Legs,
    Feet,
}

/// One piece being worn: its picture, where it goes, and its dye.
pub struct Worn {
    pub texture: TextureId,
    pub kind: ArmourKind,
    pub tint: Color32,
}

/// Armour's boxes, in the standard layout of armour pictures.
fn armour_parts(kind: ArmourKind, slim: bool) -> Vec<Part> {
    let arm = if slim { 3.0 } else { 4.0 };
    let reach = 4.0 + arm / 2.0;
    let neck = Vec3::new(0.0, 24.0, 0.0);
    let hang = Vec3::new(0.0, -6.0, 0.0);
    let trunk = Vec3::new(8.0, 12.0, 4.0);
    let limb = Vec3::new(4.0, 12.0, 4.0);
    let (hip_r, hip_l) = (Vec3::new(-2.0, 12.0, 0.0), Vec3::new(2.0, 12.0, 0.0));
    let part = |pivot: Vec3, centre: Vec3, size: Vec3, uv: (f32, f32), grow: f32, head: bool| Part { pivot, centre, size, uv, grow, head };
    match kind {
        ArmourKind::Head => vec![part(neck, Vec3::new(0.0, 4.0, 0.0), Vec3::splat(8.0), (0.0, 0.0), 1.0, true)],
        ArmourKind::Chest => vec![
            part(neck, hang, trunk, (16.0, 16.0), 1.0, false),
            part(Vec3::new(-reach, 22.0, 0.0), Vec3::new(0.0, -4.0, 0.0), limb, (40.0, 16.0), 1.0, false),
            part(Vec3::new(reach, 22.0, 0.0), Vec3::new(0.0, -4.0, 0.0), limb, (40.0, 16.0), 1.0, false),
        ],
        ArmourKind::Legs => vec![part(neck, hang, trunk, (16.0, 16.0), 0.5, false), part(hip_r, hang, limb, (0.0, 16.0), 0.5, false), part(hip_l, hang, limb, (0.0, 16.0), 0.5, false)],
        ArmourKind::Feet => vec![part(hip_r, hang, limb, (0.0, 16.0), 1.0, false), part(hip_l, hang, limb, (0.0, 16.0), 1.0, false)],
    }
}

/// Adds the faces of one box that look towards the viewer. `sheet_h` is the picture's height.
fn add_part(mesh: &mut Mesh, part: &Part, view: &View, sheet_h: f32, tint: Color32) {
    let (body, head, chest, scale) = (view.body, view.head, view.chest, view.scale);
    {
        let turn = if part.head { body * head } else { body };
        let at = body * part.pivot;
        let half = (part.size + Vec3::splat(part.grow * 2.0)) / 2.0;
        let c = part.centre;
        let corner = |x: f32, y: f32, z: f32| c + Vec3::new(x * half.x, y * half.y, z * half.z);
        let (w, h, d) = (part.size.x, part.size.y, part.size.z);
        let (u, v) = part.uv;
        let faces = [
            // front
            face(corner(-1.0, 1.0, 1.0), corner(1.0, 1.0, 1.0), corner(1.0, -1.0, 1.0), corner(-1.0, -1.0, 1.0), (u + d, v + d), (w, h)),
            // back
            face(corner(1.0, 1.0, -1.0), corner(-1.0, 1.0, -1.0), corner(-1.0, -1.0, -1.0), corner(1.0, -1.0, -1.0), (u + d + w + d, v + d), (w, h)),
            // their right, on your left
            face(corner(-1.0, 1.0, -1.0), corner(-1.0, 1.0, 1.0), corner(-1.0, -1.0, 1.0), corner(-1.0, -1.0, -1.0), (u, v + d), (d, h)),
            // their left, on your right
            face(corner(1.0, 1.0, 1.0), corner(1.0, 1.0, -1.0), corner(1.0, -1.0, -1.0), corner(1.0, -1.0, 1.0), (u + d + w, v + d), (d, h)),
            // top
            face(corner(-1.0, 1.0, -1.0), corner(1.0, 1.0, -1.0), corner(1.0, 1.0, 1.0), corner(-1.0, 1.0, 1.0), (u + d, v), (w, d)),
            // bottom
            face(corner(-1.0, -1.0, 1.0), corner(1.0, -1.0, 1.0), corner(1.0, -1.0, -1.0), corner(-1.0, -1.0, -1.0), (u + d + w, v), (w, d)),
        ];
        for (corners, uvs) in faces {
            let seen: Vec<Vec3> = corners.iter().map(|q| at + turn * *q).collect();
            // Corners run clockwise as seen from outside, which makes this point outward.
            let across = (seen[3] - seen[0]).cross(seen[1] - seen[0]);
            if across.z <= 0.0 {
                continue;
            }
            let shade = (0.62 + 0.38 * (across.normalize().dot(Vec3::new(0.25, 0.55, 0.8).normalize())).max(0.0) * 1.15).min(1.0);
            let light = (shade * 255.0) as u32;
            let dim = |c: u8| (c as u32 * light / 255) as u8;
            let tint = Color32::from_rgba_unmultiplied(dim(tint.r()), dim(tint.g()), dim(tint.b()), tint.a());
            let first = mesh.vertices.len() as u32;
            for (q, (tu, tv)) in seen.iter().zip(uvs) {
                let on_screen = pos2(chest.x + q.x * scale, chest.y - (q.y - 24.0) * scale);
                mesh.vertices.push(Vertex { pos: on_screen, uv: pos2(tu / 64.0, tv / sheet_h), color: tint });
            }
            mesh.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
    }
}
