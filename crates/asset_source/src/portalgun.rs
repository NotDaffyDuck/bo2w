//! bo2mc 10-14: Portal's portal gun as a blocky Minecraft-style view model
//! (his "The render you made of the grav gun in blocky form is way better"
//! and "there's literally blocky ones that you can copy from"). The cubes
//! and the 32x32 sheet below are copied from the Portal Gun mod by MeowMC
//! (MIT License, Copyright (c) 2021 MeowMC; via
//! https://github.com/iPortalTeam/PortalGun, `portalgun.geo.json` and
//! `portalgun.png`; the licence text is in `LICENSE-portalgun-MIT.txt`
//! beside this file). Laid out as the Gravity Gun's view models are (Source
//! model space: X forward, Y left, Z up, inches; bones Base, Handle,
//! Muzzle; idle, draw and fire), so it takes the character's hands and
//! moves the same way. Added here: the blocky grip the hand holds. Nothing
//! in it is Valve's.

use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};

use crate::mdl::{Animation, Attachment, Bone, Mesh, Model, Pose, Sequence, Texture, Vertex};
use crate::vtf::Rgba;

/// The barrel's axis, where the Gravity Gun's runs (its hand fits there).
const AXIS_Y: f32 = -12.5;
const AXIS_Z: f32 = -12.5;
const BASE: Vec3 = Vec3::new(41.2, AXIS_Y, -10.5);
/// The Gravity Gun's handle bone: the character's grip is fitted to it.
const HANDLE: Vec3 = Vec3::new(35.2, -1.0, -14.8);
const MUZZLE: Vec3 = Vec3::new(57.0, AXIS_Y, AXIS_Z);
const FPS: f32 = 30.0;
/// Where it sits in view, moved from the Gravity Gun's place (inches out,
/// left and up) and turned nose-in (degrees): its side, prongs and glowing
/// tip show beside the crosshair, as Portal's do, above the hotbar, not
/// just the back of its shell. The hand rides the handle bone, so it follows.
const SIT: Vec3 = Vec3::new(16.0, -1.0, 2.0);
const SIT_TURN: f32 = 15.0;
/// Inches per pixel of the mod's model: it is about 13 pixels long, the
/// Gravity Gun's length at this size.
const SCALE: f32 = 3.0;
/// Where the mod's model sits in Source inches: its pixel 0 along the
/// barrel, and its barrel's height in pixels (on our axis).
const ZERO_X: f32 = 41.5;
const GEO_AXIS: f32 = 5.0;

/// The materials: the mod's sheet, and the faces that glow the colour of
/// the last portal shot (`CORE`: its blue texture is the model's, the
/// orange one is swapped in by the view).
pub const SKIN: usize = 0;
pub const CORE: usize = 1;
pub const CORE_ORANGE: usize = 2;
const MATERIALS: [&str; 3] = ["skin", "core_blue", "core_orange"];

/// A material's texture name for one gun.
pub fn material(upgraded: bool, m: usize) -> String {
    format!("portalgun_{}{}", if upgraded { "super_" } else { "" }, MATERIALS[m])
}

/// A sheet pixel's colour and whether it glows (the Super Portal Gun,
/// Pack-a-Punched: dark violet where the black was, gold where the slate
/// was); `orange`: the core's blue turned to the orange portal's.
fn pixel(c: u8, upgraded: bool, orange: bool) -> Option<([u8; 3], bool)> {
    Some(match (c, upgraded) {
        (b'K', false) => ([5, 5, 5], false),
        (b'K', true) => ([46, 28, 64], false),
        (b'W', _) => ([241, 241, 241], false),
        (b'w', _) => ([231, 231, 231], false),
        (b'l', _) => ([218, 218, 218], false),
        (b'S', false) => ([83, 97, 116], false),
        (b'S', true) => ([236, 186, 46], false),
        (b'P', _) => ([180, 212, 225], false),
        (b'Y', _) => ([248, 221, 114], true),
        (b'G', _) => ([67, 232, 141], true),
        (b'R', _) => ([244, 134, 134], true),
        (b'C' | b'B', _) if orange => (if upgraded { [255, 206, 130] } else { [255, 160, 50] }, true),
        (b'C', false) => ([81, 235, 244], true),
        (b'B', false) => ([91, 188, 244], true),
        (b'C' | b'B', true) => ([170, 220, 255], true),
        _ => return None,
    })
}

/// Whether a sheet pixel is the core's (blue or cyan).
fn is_core(c: u8) -> bool {
    matches!(c, b'C' | b'B')
}

/// The sheet's pixel at (u, v) in pixels.
fn sheet(u: f32, v: f32) -> u8 {
    let (x, y) = ((u.floor() as usize).min(31), (v.floor() as usize).min(31));
    SHEET[y].as_bytes()[x]
}

/// A bone of the mod's model: its parent, the point it turns about and its
/// turn (degrees about X, Y, Z), in the mod's own numbers.
struct GeoBone {
    parent: Option<usize>,
    pivot: [f32; 3],
    turn: [f32; 3],
}

/// A cube of the mod's model, in its own numbers (Bedrock geometry:
/// pixels, Y up, the barrel along +Z): its bone, corner, size, turn about
/// `pivot`, and each face's pixels on the sheet (u, v, width, height;
/// north, east, south, west, up, down).
struct Cube {
    bone: usize,
    from: [f32; 3],
    size: [f32; 3],
    turn: [f32; 3],
    pivot: [f32; 3],
    faces: [[f32; 4]; 6],
}

/// Bedrock files store X mirrored (and the X and Y turns with it); this
/// undoes that, as Blockbench does on reading one.
fn mirrored(v: [f32; 3]) -> Vec3 {
    Vec3::new(-v[0], v[1], v[2])
}

fn turned(pivot: [f32; 3], turn: [f32; 3]) -> Mat4 {
    let p = mirrored(pivot);
    let r = Mat4::from_euler(EulerRot::ZYX, turn[2].to_radians(), (-turn[1]).to_radians(), (-turn[0]).to_radians());
    Mat4::from_translation(p) * r * Mat4::from_translation(-p)
}

fn bone_place(b: usize) -> Mat4 {
    let bone = &BONES[b];
    let own = turned(bone.pivot, bone.turn);
    bone.parent.map_or(own, |p| bone_place(p) * own)
}

/// The mod's pixels (X left, Y up, Z forward) into Source inches.
fn to_source(p: Vec3) -> Vec3 {
    Vec3::new(ZERO_X + p.z * SCALE, AXIS_Y + p.x * SCALE, AXIS_Z + (p.y - GEO_AXIS) * SCALE)
}

/// Builds boxes into meshes, one per material.
struct Builder {
    meshes: Vec<Mesh>,
}

impl Builder {
    /// One face: its corners (Source inches) top-left, top-right,
    /// bottom-right, bottom-left as seen from outside, and its pixels on
    /// the sheet. Four corners per face, in that order (the box picture
    /// reads a face's colour from them).
    fn face(&mut self, corners: [Vec3; 4], uv: [f32; 4]) {
        let [u, v, w, h] = uv;
        let m = if is_core(sheet(u + w * 0.5, v + h * 0.5)) { CORE } else { SKIN };
        let mesh = &mut self.meshes[m];
        let normal = (corners[1] - corners[0]).cross(corners[3] - corners[0]).normalize_or_zero();
        let base = mesh.vertices.len() as u32;
        for (corner, (su, sv)) in corners.into_iter().zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]) {
            mesh.vertices.push(Vertex {
                pos: corner,
                normal: -normal,
                uv: Vec2::new(u + w * su, v + h * sv) / 32.0,
                bones: [0, 0, 0],
                weights: [1.0, 0.0, 0.0],
            });
        }
        // Wound so the face's front is outside (the guest pass draws both
        // sides; this keeps it right if that changes).
        mesh.indices.extend([base, base + 3, base + 2, base, base + 2, base + 1]);
    }

    /// A box from `lo` to `hi` (the mod's pixels, already unmirrored),
    /// placed by `place`, its faces' pixels (north, east, south, west, up,
    /// down: the mod's order).
    fn cube(&mut self, place: Mat4, lo: Vec3, hi: Vec3, faces: [[f32; 4]; 6]) {
        let c = |x: f32, y: f32, z: f32| to_source(place.transform_point3(Vec3::new(x, y, z)));
        let (x0, y0, z0, x1, y1, z1) = (lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
        let [north, east, south, west, up, down] = faces;
        self.face([c(x1, y1, z0), c(x0, y1, z0), c(x0, y0, z0), c(x1, y0, z0)], north);
        self.face([c(x1, y1, z1), c(x1, y1, z0), c(x1, y0, z0), c(x1, y0, z1)], east);
        self.face([c(x0, y1, z1), c(x1, y1, z1), c(x1, y0, z1), c(x0, y0, z1)], south);
        self.face([c(x0, y1, z0), c(x0, y1, z1), c(x0, y0, z1), c(x0, y0, z0)], west);
        self.face([c(x0, y1, z0), c(x1, y1, z0), c(x1, y1, z1), c(x0, y1, z1)], up);
        self.face([c(x0, y0, z1), c(x1, y0, z1), c(x1, y0, z0), c(x0, y0, z0)], down);
    }

    /// A box straight in Source inches, every face one sheet pixel.
    fn block(&mut self, lo: Vec3, hi: Vec3, pixel: [f32; 2]) {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let uv = [pixel[0], pixel[1], 1.0, 1.0];
        // Front (+X), back, left (+Y), right, top, bottom.
        let (x0, y0, z0, x1, y1, z1) = (lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
        self.face([c(x1, y0, z1), c(x1, y1, z1), c(x1, y1, z0), c(x1, y0, z0)], uv);
        self.face([c(x0, y1, z1), c(x0, y0, z1), c(x0, y0, z0), c(x0, y1, z0)], uv);
        self.face([c(x1, y1, z1), c(x0, y1, z1), c(x0, y1, z0), c(x1, y1, z0)], uv);
        self.face([c(x0, y0, z1), c(x1, y0, z1), c(x1, y0, z0), c(x0, y0, z0)], uv);
        self.face([c(x1, y1, z1), c(x1, y0, z1), c(x0, y0, z1), c(x0, y1, z1)], uv);
        self.face([c(x1, y0, z0), c(x1, y1, z0), c(x0, y1, z0), c(x0, y0, z0)], uv);
    }
}

fn meshes() -> Vec<Mesh> {
    let mut b = Builder {
        meshes: (0..CORE_ORANGE).map(|m| Mesh { texture: m, vertices: Vec::new(), indices: Vec::new() }).collect(),
    };
    for cube in &CUBES {
        // Unmirrored: the corner with the lowest X is the far side's.
        let lo = Vec3::new(-(cube.from[0] + cube.size[0]), cube.from[1], cube.from[2]);
        let hi = lo + Vec3::from_array(cube.size);
        b.cube(bone_place(cube.bone) * turned(cube.pivot, cube.turn), lo, hi, cube.faces);
    }
    // The grip the hand holds (where the Gravity Gun's handle is), on a
    // strut out of the shell's side, a trigger before it: black blocks,
    // the trigger slate.
    let black = [1.0, 12.0];
    b.block(Vec3::new(34.0, -8.5, -15.8), Vec3::new(36.0, -2.5, -13.6), black);
    b.block(Vec3::new(29.0, -2.9, -16.9), Vec3::new(40.0, 0.5, -13.5), black);
    b.block(Vec3::new(40.0, -1.9, -17.6), Vec3::new(41.4, -0.5, -15.6), [23.0, 2.0]);
    b.meshes
}

/// `SIT` and `SIT_TURN`, or test aid BO2MP_PORTALGUN_SIT="out,left,up,turn"
/// (trying places without a rebuild).
fn sit() -> (Vec3, f32) {
    let aid = std::env::var("BO2MP_PORTALGUN_SIT").ok().and_then(|v| {
        let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (n.len() == 4).then(|| (Vec3::new(n[0], n[1], n[2]), n[3]))
    });
    aid.unwrap_or((SIT, SIT_TURN))
}

/// A pose: the base moved by `shift` and turned by `turn`.
fn pose(shift: Vec3, turn: Quat) -> Pose {
    vec![(turn, BASE + shift), (Quat::IDENTITY, HANDLE - BASE), (Quat::IDENTITY, MUZZLE - BASE)]
}

fn sequence(model: &mut Model, name: &str, seconds: f32, looping: bool, motion: impl Fn(f32) -> (Vec3, Quat)) {
    let frames = ((seconds * FPS).round() as usize).max(1) + 1;
    let frames = (0..frames)
        .map(|i| {
            let (shift, turn) = motion(i as f32 / FPS);
            let (sit, sit_turn) = sit();
            pose(shift + sit, Quat::from_rotation_z(sit_turn.to_radians()) * turn)
        })
        .collect();
    let anim = model.animations.len();
    model.animations.push(Animation { name: name.to_owned(), fps: FPS, looping, delta: false, frames });
    model.sequences.push(Sequence {
        name: name.to_owned(),
        activity: String::new(),
        looping,
        fade_in: 0.2,
        fade_out: 0.2,
        anims: vec![anim],
        groups: [1, 1],
        params: [None, None],
        events: Vec::new(),
        flags: 0,
        layers: Vec::new(),
        weights: vec![1.0; 3],
    });
}

/// Nose up (degrees, about the base).
fn pitch(degrees: f32) -> Quat {
    Quat::from_rotation_y(-degrees.to_radians())
}

fn ease_out(s: f32) -> f32 {
    let s = s.clamp(0.0, 1.0);
    1.0 - (1.0 - s) * (1.0 - s)
}

/// The view model (`upgraded`: the Super Portal Gun).
pub fn model(upgraded: bool) -> Model {
    let rest = pose(Vec3::ZERO, Quat::IDENTITY);
    let base = Mat4::from_rotation_translation(rest[0].0, rest[0].1);
    let world: Vec<Mat4> = std::iter::once(base)
        .chain(rest[1..].iter().map(|(q, t)| base * Mat4::from_rotation_translation(*q, *t)))
        .collect();
    let bones = ["Base", "Handle", "Muzzle"]
        .iter()
        .enumerate()
        .map(|(i, name)| Bone {
            name: (*name).to_owned(),
            parent: (i > 0).then_some(0),
            pos: rest[i].1,
            quat: rest[i].0,
            rot: Vec3::ZERO,
            pos_scale: Vec3::ZERO,
            rot_scale: Vec3::ZERO,
            pose_to_bone: world[i].inverse(),
        })
        .collect();
    let mut model = Model {
        name: if upgraded { "portalgun_super" } else { "portalgun" }.to_owned(),
        bones,
        // The orange core is a texture too (the view swaps it in).
        textures: (0..MATERIALS.len()).map(|m| Texture { name: material(upgraded, m), folders: Vec::new() }).collect(),
        meshes: meshes(),
        animations: Vec::new(),
        sequences: Vec::new(),
        pose_params: Vec::new(),
        attachments: vec![Attachment { name: "muzzle".to_owned(), bone: 2, local: Mat4::IDENTITY }],
    };
    let tau = std::f32::consts::TAU;
    let breath = if upgraded { 3.4 } else { 4.6 };
    sequence(&mut model, "idle", breath, true, |t| {
        let s = (t / breath * tau).sin();
        (Vec3::new(0.0, 0.0, 0.22 * s), pitch(0.5 * s))
    });
    // Raised from below, nose down.
    sequence(&mut model, "draw", 0.5, false, |t| {
        let s = 1.0 - ease_out(t / 0.5);
        (Vec3::new(-4.0 * s, 0.0, -14.0 * s), pitch(-35.0 * s))
    });
    // A shot: a quick kick back and nose up, then home.
    sequence(&mut model, "fire", 0.4, false, |t| {
        let peak = 0.04;
        let s = if t < peak { t / peak } else { 1.0 - ease_out((t - peak) / (0.4 - peak)) };
        (Vec3::new(-2.6 * s, 0.0, 0.7 * s), pitch(5.0 * s))
    });
    model
}

/// A material's texture: the mod's sheet (alpha: the glow, as the Gravity
/// Gun's `$selfillum`), the core's with its blue turned orange.
pub fn texture(name: &str) -> Option<Rgba> {
    let upgraded = name.contains("super_");
    let m = MATERIALS.iter().position(|m| name.ends_with(m))?;
    let mut pixels = Vec::with_capacity(32 * 32 * 4);
    for row in SHEET {
        for c in row.bytes() {
            pixels.extend(match pixel(c, upgraded, m == CORE_ORANGE) {
                Some((rgb, glow)) => [rgb[0], rgb[1], rgb[2], if glow { 255 } else { 0 }],
                None => [0, 0, 0, 0],
            });
        }
    }
    Some(Rgba { width: 32, height: 32, pixels })
}

/// The mod's sheet, a letter a pixel (see `pixel`; `.` is unused).
const SHEET: [&str; 32] = [
    "WlWWWWWWKKKKKKKKKWWWWKKWKw......",
    "WlWwWWWwKKKKKKKKKWWWWKKWKW......",
    "WCWwwwwwKKKKKKKKKKKKWlWSKw......",
    "WlWWWWWWWWWKKKKKKKKKKKKSKw......",
    "WlWWWWWWWWWKKCCCKKKKKKKKKW......",
    "KKKWWWWWWWWKKKKKKKGGGKKKK.......",
    "KKKWWWWWWWWKKCCWKKKKKKKYK.......",
    "KKKWlWWWWWWKKCCWKKKKWKKBK.......",
    "WWWWWKWWWWWKKCCWWWKKwKBBK.......",
    "wWWWwWWWwwwKKKKWwWKKwKKKK.......",
    "wwwwwWWWwwwKKKKWwwwwKKKKS.......",
    "KKKKKWWWwwwKKKKKKwWwwKKKK.......",
    "KKKKKWWWwwwKKKKKKwwwwKKKK.......",
    "KKKKKKKKCCCKKKWWWWWKWKKKK.......",
    "KKKKKKKKKKKKKKKKWWKKWKwwK.......",
    "KKKKKKKKWWWWWWKKWWKKWKKBK.......",
    "KKKWWKKYYKKKKWKKKKKWWWWWK.......",
    "WWWWWKKYYKKKKWWYKKKKKKKKK.......",
    "WWWWWWWWWWWWwWWRRRKwWwKKK.......",
    "WWWWWWWwwKKKKWWKKKKKYYKKK.......",
    "WWWWWKKKKKKKKKWKWWKKWlWKK.......",
    "KKKKKKKKKKKKKKKKwWKKSSwwK.......",
    "KKKKKKKKKKKBKKKKwWKKKKKKK.......",
    "KKKKKKKKKKKBKKWWWSWKKKKWK.......",
    "KKKKKKKKKKKPSKWWWSWKKPSWK.......",
    "KKSKKKKKKKKKKKKKKKSKKKKKKW......",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
];
/// The mod's bones (`everything`, `head`, the three claws, the shells).
const BONES: [GeoBone; 7] = [
    // everything
    GeoBone { parent: None, pivot: [-0.00333, 3.00667, -1.335], turn: [0.0, 0.0, 0.0] },
    // head
    GeoBone { parent: Some(0), pivot: [-1.6, 5.0, 3.4], turn: [0.0, 0.0, 0.0] },
    // claw1
    GeoBone { parent: Some(0), pivot: [0.0, 6.3, 1.5], turn: [0.0, 0.0, 0.0] },
    // claw2
    GeoBone { parent: Some(0), pivot: [-0.4, 4.9, 3.7], turn: [0.0, 0.0, -122.5] },
    // claw3
    GeoBone { parent: Some(0), pivot: [0.4, 4.9, 3.7], turn: [0.0, 0.0, 122.5] },
    // frontshell
    GeoBone { parent: Some(0), pivot: [0.0, 3.3, 0.0], turn: [0.0, 0.0, 0.0] },
    // backshell
    GeoBone { parent: Some(0), pivot: [0.0, 0.0, 1.0], turn: [0.0, 0.0, 0.0] },
];
/// The mod's cubes, each on its bone.
const CUBES: [Cube; 37] = [
    Cube { bone: 0, from: [-1.0, 4.0, -1.7], size: [2.0, 2.0, 3.0], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[5.0, 16.0, 2.0, 2.0], [8.0, 13.0, 3.0, 2.0], [7.0, 16.0, 2.0, 2.0], [13.0, 4.0, 3.0, 2.0], [13.0, 6.0, 2.0, 3.0], [14.0, 17.0, 2.0, -3.0]] },
    Cube { bone: 0, from: [-1.2, 4.0, -1.7], size: [0.2, 1.0, 2.7], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[5.0, 8.0, 1.0, 1.0], [8.0, 2.0, 3.0, 1.0], [15.0, 17.0, 1.0, 1.0], [15.0, 18.0, 3.0, 1.0], [18.0, 18.0, 1.0, 3.0], [18.0, 5.0, 1.0, -3.0]] },
    Cube { bone: 0, from: [1.0, 4.0, -1.7], size: [0.3, 1.0, 2.7], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[23.0, 4.0, 1.0, 1.0], [18.0, 5.0, 3.0, 1.0], [23.0, 6.0, 1.0, 1.0], [18.0, 6.0, 3.0, 1.0], [18.0, 7.0, 1.0, 3.0], [18.0, 17.0, 1.0, -3.0]] },
    Cube { bone: 0, from: [-0.2, 4.0, -5.95], size: [0.4, 0.4, 0.4], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[23.0, 7.0, 1.0, 1.0], [23.0, 9.0, 1.0, 1.0], [23.0, 10.0, 1.0, 1.0], [23.0, 11.0, 1.0, 1.0], [23.0, 12.0, 1.0, 1.0], [23.0, 14.0, 1.0, -1.0]] },
    Cube { bone: 0, from: [-0.2, 5.7, -5.95], size: [0.4, 0.4, 0.4], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[23.0, 15.0, 1.0, 1.0], [23.0, 17.0, 1.0, 1.0], [23.0, 18.0, 1.0, 1.0], [23.0, 19.0, 1.0, 1.0], [23.0, 20.0, 1.0, 1.0], [23.0, 23.0, 1.0, -1.0]] },
    Cube { bone: 0, from: [-0.2, 4.0, -6.35], size: [0.4, 2.1, 0.4], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[20.0, 3.0, 1.0, 2.0], [0.0, 21.0, 1.0, 2.0], [1.0, 21.0, 1.0, 2.0], [2.0, 21.0, 1.0, 2.0], [3.0, 24.0, 1.0, 1.0], [4.0, 25.0, 1.0, -1.0]] },
    Cube { bone: 0, from: [-1.25, 3.75, 1.0], size: [2.5, 2.5, 1.0], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[0.0, 5.0, 3.0, 3.0], [9.0, 19.0, 1.0, 3.0], [5.0, 13.0, 3.0, 3.0], [10.0, 19.0, 1.0, 3.0], [18.0, 17.0, 3.0, 1.0], [15.0, 20.0, 3.0, -1.0]] },
    Cube { bone: 0, from: [-1.05, 5.85, 0.5], size: [2.1, 0.4, 0.5], turn: [27.5, 0.0, 0.0], pivot: [0.05, 6.25, 1.0], faces: [[20.0, 10.0, 2.0, 1.0], [9.0, 24.0, 1.0, 1.0], [20.0, 19.0, 2.0, 1.0], [10.0, 24.0, 1.0, 1.0], [3.0, 21.0, 2.0, 1.0], [20.0, 22.0, 2.0, -1.0]] },
    Cube { bone: 0, from: [-1.4, 3.8, -5.55], size: [2.6, 2.2, 3.65], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[14.0, 11.0, 3.0, 2.0], [11.0, 9.0, 4.0, 2.0], [0.0, 15.0, 3.0, 2.0], [13.0, 0.0, 4.0, 2.0], [0.0, 11.0, 3.0, 4.0], [11.0, 15.0, 3.0, -4.0]] },
    Cube { bone: 1, from: [-0.85, 4.25, 3.65], size: [0.4, 1.6, 0.5], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[14.0, 21.0, 1.0, 2.0], [18.0, 21.0, 1.0, 2.0], [21.0, 0.0, 1.0, 2.0], [21.0, 3.0, 1.0, 2.0], [11.0, 24.0, 1.0, 1.0], [12.0, 25.0, 1.0, -1.0]] },
    Cube { bone: 1, from: [0.35, 6.25, 3.65], size: [0.4, 1.7, 0.5], turn: [0.0, 0.0, 90.0], pivot: [-0.4, 6.7, 3.4], faces: [[21.0, 5.0, 1.0, 2.0], [21.0, 7.0, 1.0, 2.0], [21.0, 11.0, 1.0, 2.0], [21.0, 13.0, 1.0, 2.0], [19.0, 24.0, 1.0, 1.0], [20.0, 25.0, 1.0, -1.0]] },
    Cube { bone: 1, from: [0.45, 4.25, 3.65], size: [0.4, 1.6, 0.5], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[3.0, 22.0, 1.0, 2.0], [4.0, 22.0, 1.0, 2.0], [9.0, 22.0, 1.0, 2.0], [10.0, 22.0, 1.0, 2.0], [21.0, 24.0, 1.0, 1.0], [22.0, 25.0, 1.0, -1.0]] },
    Cube { bone: 1, from: [-1.3, 2.95, 3.65], size: [0.4, 1.7, 0.5], turn: [0.0, 0.0, 90.0], pivot: [-0.3, 3.5, 3.4], faces: [[11.0, 22.0, 1.0, 2.0], [12.0, 22.0, 1.0, 2.0], [19.0, 22.0, 1.0, 2.0], [20.0, 22.0, 1.0, 2.0], [24.0, 24.0, 1.0, 1.0], [24.0, 1.0, 1.0, -1.0]] },
    Cube { bone: 1, from: [-0.8, 4.1, -1.4], size: [1.6, 1.8, 5.3], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[9.0, 16.0, 2.0, 2.0], [8.0, 0.0, 5.0, 2.0], [11.0, 16.0, 2.0, 2.0], [11.0, 2.0, 5.0, 2.0], [3.0, 11.0, 2.0, 5.0], [11.0, 9.0, 2.0, -5.0]] },
    Cube { bone: 1, from: [-0.95, 4.1, 2.55], size: [1.9, 1.85, 0.9], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[16.0, 16.0, 2.0, 2.0], [21.0, 22.0, 1.0, 2.0], [16.0, 2.0, 2.0, 2.0], [22.0, 22.0, 1.0, 2.0], [21.0, 9.0, 2.0, 1.0], [21.0, 16.0, 2.0, -1.0]] },
    Cube { bone: 1, from: [-1.15, 3.85, 1.6], size: [2.3, 2.3, 0.7], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[16.0, 4.0, 2.0, 2.0], [22.0, 0.0, 1.0, 2.0], [16.0, 6.0, 2.0, 2.0], [22.0, 3.0, 1.0, 2.0], [21.0, 17.0, 2.0, 1.0], [22.0, 6.0, 2.0, -1.0]] },
    Cube { bone: 2, from: [-0.1, 6.95545, 4.74665], size: [0.2, 1.4, 0.2], turn: [54.0, 0.0, 0.0], pivot: [0.0, 8.2, 4.8], faces: [[24.0, 1.0, 1.0, 1.0], [24.0, 2.0, 1.0, 1.0], [24.0, 3.0, 1.0, 1.0], [24.0, 4.0, 1.0, 1.0], [24.0, 5.0, 1.0, 1.0], [24.0, 7.0, 1.0, -1.0]] },
    Cube { bone: 2, from: [-0.1, 7.6, 1.6], size: [0.2, 3.5, 0.2], turn: [-80.0, 0.0, 0.0], pivot: [0.0, 7.7, 1.7], faces: [[11.0, 19.0, 1.0, 3.0], [12.0, 19.0, 1.0, 3.0], [19.0, 19.0, 1.0, 3.0], [19.0, 2.0, 1.0, 3.0], [24.0, 7.0, 1.0, 1.0], [24.0, 9.0, 1.0, -1.0]] },
    Cube { bone: 2, from: [-0.1, 6.4, 1.4], size: [0.2, 1.6, 0.2], turn: [-15.0, 0.0, 0.0], pivot: [0.0, 6.5, 1.5], faces: [[22.0, 6.0, 1.0, 2.0], [22.0, 10.0, 1.0, 2.0], [22.0, 12.0, 1.0, 2.0], [22.0, 18.0, 1.0, 2.0], [24.0, 9.0, 1.0, 1.0], [24.0, 11.0, 1.0, -1.0]] },
    Cube { bone: 2, from: [-0.4, 6.0, 1.1], size: [0.8, 0.4, 0.8], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[24.0, 11.0, 1.0, 1.0], [24.0, 12.0, 1.0, 1.0], [24.0, 13.0, 1.0, 1.0], [24.0, 14.0, 1.0, 1.0], [24.0, 15.0, 1.0, 1.0], [24.0, 17.0, 1.0, -1.0]] },
    Cube { bone: 3, from: [-0.5, 5.82431, 4.37412], size: [0.2, 1.4, 0.2], turn: [54.0, 0.0, 0.0], pivot: [-0.4, 7.9, 3.7], faces: [[24.0, 17.0, 1.0, 1.0], [24.0, 18.0, 1.0, 1.0], [24.0, 19.0, 1.0, 1.0], [24.0, 20.0, 1.0, 1.0], [24.0, 21.0, 1.0, 1.0], [24.0, 23.0, 1.0, -1.0]] },
    Cube { bone: 3, from: [-0.5, 7.4, 1.6], size: [0.2, 3.5, 0.2], turn: [-80.0, 0.0, 0.0], pivot: [-0.4, 7.5, 1.7], faces: [[19.0, 7.0, 1.0, 3.0], [19.0, 13.0, 1.0, 3.0], [5.0, 20.0, 1.0, 3.0], [6.0, 20.0, 1.0, 3.0], [24.0, 23.0, 1.0, 1.0], [0.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 3, from: [-0.5, 6.2, 1.4], size: [0.2, 1.6, 0.2], turn: [-15.0, 0.0, 0.0], pivot: [-0.4, 6.3, 1.5], faces: [[0.0, 23.0, 1.0, 2.0], [1.0, 23.0, 1.0, 2.0], [2.0, 23.0, 1.0, 2.0], [5.0, 23.0, 1.0, 2.0], [1.0, 25.0, 1.0, 1.0], [2.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 3, from: [-0.9, 5.7, 1.1], size: [0.8, 0.4, 0.8], turn: [0.0, 0.0, 32.5], pivot: [0.1, 5.8, 0.0], faces: [[3.0, 25.0, 1.0, 1.0], [4.0, 25.0, 1.0, 1.0], [5.0, 25.0, 1.0, 1.0], [6.0, 25.0, 1.0, 1.0], [7.0, 25.0, 1.0, 1.0], [8.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 4, from: [0.3, 5.82431, 4.37412], size: [0.2, 1.4, 0.2], turn: [54.0, 0.0, 0.0], pivot: [0.4, 7.9, 3.7], faces: [[9.0, 25.0, 1.0, 1.0], [10.0, 25.0, 1.0, 1.0], [11.0, 25.0, 1.0, 1.0], [12.0, 25.0, 1.0, 1.0], [13.0, 25.0, 1.0, 1.0], [14.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 4, from: [0.3, 7.4, 1.6], size: [0.2, 3.5, 0.2], turn: [-80.0, 0.0, 0.0], pivot: [0.4, 7.5, 1.7], faces: [[7.0, 20.0, 1.0, 3.0], [8.0, 20.0, 1.0, 3.0], [13.0, 20.0, 1.0, 3.0], [15.0, 20.0, 1.0, 3.0], [15.0, 25.0, 1.0, 1.0], [16.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 4, from: [0.3, 6.2, 1.4], size: [0.2, 1.6, 0.2], turn: [-15.0, 0.0, 0.0], pivot: [0.4, 6.3, 1.5], faces: [[6.0, 23.0, 1.0, 2.0], [7.0, 23.0, 1.0, 2.0], [8.0, 23.0, 1.0, 2.0], [13.0, 23.0, 1.0, 2.0], [17.0, 25.0, 1.0, 1.0], [18.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 4, from: [0.3, 6.2, 1.1], size: [0.8, 0.4, 0.8], turn: [0.0, 0.0, -32.5], pivot: [0.9, 5.8, 0.0], faces: [[19.0, 25.0, 1.0, 1.0], [20.0, 25.0, 1.0, 1.0], [21.0, 25.0, 1.0, 1.0], [22.0, 25.0, 1.0, 1.0], [23.0, 25.0, 1.0, 1.0], [24.0, 26.0, 1.0, -1.0]] },
    Cube { bone: 5, from: [-1.5, 5.0, 1.0], size: [0.2, 0.9, 1.9], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[14.0, 23.0, 1.0, 2.0], [16.0, 8.0, 2.0, 1.0], [15.0, 23.0, 1.0, 2.0], [16.0, 14.0, 2.0, 2.0], [16.0, 23.0, 1.0, 2.0], [17.0, 25.0, 1.0, -2.0]] },
    Cube { bone: 5, from: [-1.5, 3.8, -0.1], size: [0.2, 1.2, 3.9], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[25.0, 25.0, 1.0, 1.0], [16.0, 10.0, 4.0, 1.0], [25.0, 0.0, 1.0, 1.0], [17.0, 0.0, 4.0, 1.0], [0.0, 17.0, 1.0, 4.0], [1.0, 21.0, 1.0, -4.0]] },
    Cube { bone: 5, from: [1.3, 3.8, -0.1], size: [0.2, 1.2, 3.9], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[25.0, 1.0, 1.0, 1.0], [17.0, 1.0, 4.0, 1.0], [25.0, 2.0, 1.0, 1.0], [17.0, 11.0, 4.0, 1.0], [2.0, 17.0, 1.0, 4.0], [14.0, 21.0, 1.0, -4.0]] },
    Cube { bone: 5, from: [1.3, 5.0, 1.0], size: [0.2, 0.9, 1.9], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[18.0, 23.0, 1.0, 2.0], [5.0, 18.0, 2.0, 2.0], [23.0, 23.0, 1.0, 2.0], [7.0, 18.0, 2.0, 1.0], [23.0, 0.0, 1.0, 2.0], [23.0, 4.0, 1.0, -2.0]] },
    Cube { bone: 5, from: [-1.5, 3.6, 0.0], size: [3.0, 0.2, 3.8], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[19.0, 16.0, 3.0, 1.0], [17.0, 12.0, 4.0, 1.0], [19.0, 18.0, 3.0, 1.0], [9.0, 18.0, 4.0, 1.0], [5.0, 9.0, 3.0, 4.0], [8.0, 13.0, 3.0, -4.0]] },
    Cube { bone: 5, from: [-1.1, 3.6, 3.8], size: [2.2, 0.2, 0.7], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[22.0, 8.0, 2.0, 1.0], [25.0, 3.0, 1.0, 1.0], [22.0, 14.0, 2.0, 1.0], [25.0, 4.0, 1.0, 1.0], [22.0, 16.0, 2.0, 1.0], [22.0, 22.0, 2.0, -1.0]] },
    Cube { bone: 6, from: [-1.5, 6.0, -6.45], size: [3.0, 0.3, 4.95], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[20.0, 20.0, 3.0, 1.0], [14.0, 13.0, 5.0, 1.0], [20.0, 2.0, 3.0, 1.0], [8.0, 15.0, 5.0, 1.0], [0.0, 0.0, 3.0, 5.0], [3.0, 8.0, 3.0, -5.0]] },
    Cube { bone: 6, from: [-1.5, 3.6, -6.45], size: [0.3, 2.7, 4.95], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[16.0, 20.0, 1.0, 3.0], [3.0, 0.0, 5.0, 3.0], [17.0, 20.0, 1.0, 3.0], [6.0, 6.0, 5.0, 3.0], [13.0, 15.0, 1.0, 5.0], [15.0, 11.0, 1.0, -5.0]] },
    Cube { bone: 6, from: [1.2, 3.6, -6.45], size: [0.3, 2.7, 4.95], turn: [0.0, 0.0, 0.0], pivot: [0.0, 0.0, 0.0], faces: [[20.0, 7.0, 1.0, 3.0], [6.0, 3.0, 5.0, 3.0], [20.0, 13.0, 1.0, 3.0], [0.0, 8.0, 5.0, 3.0], [3.0, 16.0, 1.0, 5.0], [4.0, 21.0, 1.0, -5.0]] },
];
