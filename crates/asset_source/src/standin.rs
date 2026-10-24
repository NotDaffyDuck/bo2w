//! The Gravity Gun when no Half-Life 2 files are found (his 10-13 "if
//! that's not possible ... then let's use the Minecraft one"): a blocky,
//! pixel-textured gun made here, laid out as Half-Life 2's view model is
//! (the same bones, attachments, sequences and "active" claws), so it
//! animates, glows and takes the character's hands the same way. Nothing
//! in it is Valve's.

use glam::{Mat3, Mat4, Quat, Vec2, Vec3};

use crate::mdl::{Animation, Attachment, Bone, Mesh, Model, Pose, PoseParam, Sequence, Texture, Vertex};
use crate::vtf::Rgba;

/// The gun's axis (Source model space: X forward, Y left, Z up, inches),
/// where Half-Life 2's barrel runs.
const AXIS_Y: f32 = -12.5;
const AXIS_Z: f32 = -12.5;
/// The bones at rest: Base, Handle, Muzzle (Half-Life 2's places).
const BASE: Vec3 = Vec3::new(41.2, AXIS_Y, -10.5);
const HANDLE: Vec3 = Vec3::new(35.2, -1.0, -14.8);
const MUZZLE: Vec3 = Vec3::new(59.5, AXIS_Y, AXIS_Z);
/// The claws' hinges: this far out from the axis, at the collar's front.
const HINGE_X: f32 = 56.5;
const HINGE_OUT: f32 = 4.5;
/// The claws' turn outwards, shut and open (degrees).
const SHUT: f32 = 4.0;
const OPEN: f32 = 20.0;
const FPS: f32 = 30.0;

/// The materials, each a 16-pixel texture (1 pixel an inch).
const DARK: usize = 0;
const METAL: usize = 1;
const ACCENT: usize = 2;
const CORE: usize = 3;
const MATERIALS: [&str; 4] = ["dark", "metal", "accent", "core"];

/// A material's texture name for one gun.
fn material(upgraded: bool, m: usize) -> String {
    format!("standin_{}{}", if upgraded { "super_" } else { "" }, MATERIALS[m])
}

/// One claw's direction out from the axis (Y left, Z up).
fn claw_out(k: usize) -> Vec3 {
    let a = [90.0f32, 210.0, 330.0][k].to_radians();
    Vec3::new(0.0, a.cos(), a.sin())
}

/// A claw's frame at rest: X forward, Z out from the axis.
fn claw_frame(k: usize) -> Quat {
    let out = claw_out(k);
    Quat::from_mat3(&Mat3::from_cols(Vec3::X, out.cross(Vec3::X), out))
}

fn claw_pose(k: usize, open: f32) -> (Quat, Vec3) {
    let hinge = Vec3::new(HINGE_X, AXIS_Y, AXIS_Z) + claw_out(k) * HINGE_OUT;
    // About its own Y, tip outwards.
    (claw_frame(k) * Quat::from_rotation_y(-open.to_radians()), hinge - BASE)
}

/// The bones: 0 Base, 1 Handle, 2 Muzzle, 3..6 the claws (A top, B lower
/// left, C lower right).
const NAMES: [&str; 6] = ["Base", "Handle", "Muzzle", "Prong_A", "Prong_B", "Prong_C"];

/// A pose: the base moved by `shift` and turned by `turn` (about the
/// base), the claws `open` degrees out.
fn pose(shift: Vec3, turn: Quat, open: f32) -> Pose {
    let mut p: Pose = vec![
        (turn, BASE + shift),
        (Quat::IDENTITY, HANDLE - BASE),
        (Quat::IDENTITY, MUZZLE - BASE),
    ];
    p.extend((0..3).map(|k| claw_pose(k, open)));
    p
}

/// Each bone's model-space place at rest.
fn rest() -> Vec<Mat4> {
    let p = pose(Vec3::ZERO, Quat::IDENTITY, SHUT);
    let base = Mat4::from_rotation_translation(p[0].0, p[0].1);
    let mut out = vec![base];
    out.extend(p[1..].iter().map(|(q, t)| base * Mat4::from_rotation_translation(*q, *t)));
    out
}

/// Builds cuboids into meshes, one per material.
struct Builder {
    meshes: Vec<Mesh>,
}

impl Builder {
    /// A box from `lo` to `hi` in bone `bone`'s own space (placed by
    /// `place`), its faces textured 1 pixel an inch.
    fn cuboid(&mut self, place: Mat4, bone: u8, m: usize, lo: Vec3, hi: Vec3) {
        let mesh = &mut self.meshes[m];
        // Each face: its normal, and its two in-face axes (u, v).
        let faces = [
            (Vec3::X, Vec3::NEG_Y, Vec3::NEG_Z),
            (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::NEG_X, Vec3::NEG_Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::X, Vec3::NEG_Y),
        ];
        let centre = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        for (n, u, v) in faces {
            let base = mesh.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let local = centre + (n + u * su + v * sv) * half;
                // Pixels run along the face's own axes, from the box's corner.
                let uv = Vec2::new((local - lo).dot(u.abs()), (local - lo).dot(v.abs())) / 16.0;
                mesh.vertices.push(Vertex {
                    pos: place.transform_point3(local),
                    normal: place.transform_vector3(n).normalize(),
                    uv,
                    bones: [bone, 0, 0],
                    weights: [1.0, 0.0, 0.0],
                });
            }
            // Wound so the face's front is its normal's side (the guest
            // pass draws both sides; this keeps it right if that changes).
            if u.cross(v).dot(n) > 0.0 {
                mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            } else {
                mesh.indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }
}

/// A box given by its model-space corners at rest, on bone 0 (the base).
fn body(b: &mut Builder, m: usize, lo: [f32; 3], hi: [f32; 3]) {
    b.cuboid(Mat4::IDENTITY, 0, m, Vec3::from_array(lo), Vec3::from_array(hi));
}

fn meshes() -> Vec<Mesh> {
    let rest = rest();
    let mut b = Builder {
        meshes: (0..MATERIALS.len()).map(|m| Mesh { texture: m, vertices: Vec::new(), indices: Vec::new() }).collect(),
    };
    let (y, z) = (AXIS_Y, AXIS_Z);
    // The body: a block round the axis, a dark back cap, a metal collar
    // at the front with the core between the claws.
    body(&mut b, METAL, [30.0, y - 5.5, z - 4.5], [52.0, y + 5.5, z + 4.5]);
    body(&mut b, DARK, [26.0, y - 4.5, z - 3.5], [30.0, y + 4.5, z + 3.5]);
    body(&mut b, DARK, [52.0, y - 6.5, z - 5.5], [56.5, y + 6.5, z + 5.5]);
    body(&mut b, ACCENT, [53.0, y - 6.8, z - 2.0], [56.0, y + 6.8, z + 2.0]);
    body(&mut b, CORE, [56.5, y - 2.5, z - 2.5], [59.0, y + 2.5, z + 2.5]);
    // The glowing stripes along its sides and the top rail.
    body(&mut b, ACCENT, [33.0, y - 5.8, z - 1.0], [50.0, y + 5.8, z + 1.0]);
    body(&mut b, DARK, [34.0, y - 2.0, z + 4.5], [50.0, y + 2.0, z + 6.0]);
    body(&mut b, ACCENT, [36.0, y - 1.0, z + 6.0], [48.0, y + 1.0, z + 6.6]);
    // The side handle the hand holds (where Half-Life 2's was), on a strut.
    body(&mut b, DARK, [33.0, y + 5.5, -16.5], [37.5, -3.5, -13.0]);
    body(&mut b, DARK, [27.0, -3.5, -17.0], [39.0, 1.0, -13.0]);
    // The three claws, each two blocks bending in at the tip.
    for k in 0..3 {
        let bone = 3 + k as u8;
        let place = rest[3 + k];
        b.cuboid(place, bone, DARK, Vec3::new(-1.0, -1.5, -1.0), Vec3::new(3.0, 1.5, 1.5));
        b.cuboid(place, bone, METAL, Vec3::new(2.0, -0.75, -0.25), Vec3::new(11.0, 0.75, 1.25));
        b.cuboid(place, bone, ACCENT, Vec3::new(10.0, -1.0, -2.5), Vec3::new(13.5, 1.0, 0.5));
    }
    b.meshes
}

/// A sequence of two animations (claws shut, open) blended by "active",
/// each `seconds` long, its base at `shift(t)` turned `turn(t)`.
fn sequence(
    model: &mut Model,
    name: &str,
    seconds: f32,
    looping: bool,
    motion: impl Fn(f32) -> (Vec3, Quat),
) {
    let frames = ((seconds * FPS).round() as usize).max(1) + 1;
    let mut anims = Vec::new();
    for (label, open) in [("shut", SHUT), ("open", OPEN)] {
        let frames = (0..frames)
            .map(|i| {
                let (shift, turn) = motion(i as f32 / FPS);
                pose(shift, turn, open)
            })
            .collect();
        anims.push(model.animations.len());
        model.animations.push(Animation { name: format!("{name}_{label}"), fps: FPS, looping, delta: false, frames });
    }
    model.sequences.push(Sequence {
        name: name.to_owned(),
        activity: String::new(),
        looping,
        fade_in: 0.2,
        fade_out: 0.2,
        anims,
        groups: [2, 1],
        params: [Some((0, 0.0, 1.0)), None],
        events: Vec::new(),
        flags: 0,
        layers: Vec::new(),
        weights: vec![1.0; NAMES.len()],
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

/// The stand-in view model (`upgraded`: the Super Gravity Gun).
pub fn model(upgraded: bool) -> Model {
    let rest = rest();
    let bones = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let p = pose(Vec3::ZERO, Quat::IDENTITY, SHUT)[i];
            Bone {
                name: (*name).to_owned(),
                parent: (i > 0).then_some(0),
                pos: p.1,
                quat: p.0,
                rot: Vec3::ZERO,
                pos_scale: Vec3::ZERO,
                rot_scale: Vec3::ZERO,
                pose_to_bone: rest[i].inverse(),
            }
        })
        .collect();
    let at = |bone: usize, local: Vec3| Attachment {
        name: String::new(),
        bone,
        local: Mat4::from_translation(local),
    };
    let named = |name: &str, a: Attachment| Attachment { name: name.to_owned(), ..a };
    let mut model = Model {
        name: if upgraded { "standin_superphyscannon" } else { "standin_physcannon" }.to_owned(),
        bones,
        textures: (0..MATERIALS.len()).map(|m| Texture { name: material(upgraded, m), folders: Vec::new() }).collect(),
        meshes: meshes(),
        animations: Vec::new(),
        sequences: Vec::new(),
        pose_params: vec![PoseParam { name: "active".to_owned(), start: 0.0, end: 1.0 }],
        attachments: vec![
            named("muzzle", at(2, Vec3::ZERO)),
            named("fork1b", at(3, Vec3::new(1.0, 0.0, 0.5))),
            named("fork1m", at(3, Vec3::new(6.0, 0.0, 1.0))),
            named("fork1t", at(3, Vec3::new(12.5, 0.0, -1.5))),
            named("fork2b", at(4, Vec3::new(1.0, 0.0, 0.5))),
            named("fork2m", at(4, Vec3::new(6.0, 0.0, 1.0))),
            named("fork2t", at(4, Vec3::new(12.5, 0.0, -1.5))),
        ],
    };
    let tau = std::f32::consts::TAU;
    // A slow breath; the super gun's quicker, as Half-Life 2's.
    let breath = if upgraded { 3.6 } else { 5.0 };
    sequence(&mut model, "idle", breath, true, |t| {
        let s = (t / breath * tau).sin();
        (Vec3::new(0.0, 0.0, 0.25 * s), pitch(0.6 * s))
    });
    // Straining while it holds something.
    sequence(&mut model, "hold_idle", 1.0, true, |t| {
        let (a, b) = ((t * tau * 5.0).sin(), (t * tau * 3.0).sin());
        (Vec3::new(0.15 * b, 0.12 * a, 0.2 * a), pitch(0.5 * b))
    });
    // Raised from below, nose down.
    sequence(&mut model, "draw", 0.5, false, |t| {
        let s = 1.0 - ease_out(t / 0.5);
        (Vec3::new(-4.0 * s, 0.0, -14.0 * s), pitch(-35.0 * s))
    });
    // The kick of a launch: back and nose up, then home.
    let kick = |back: f32, up: f32, seconds: f32| {
        move |t: f32| {
            let peak = 0.05;
            let s = if t < peak { t / peak } else { 1.0 - ease_out((t - peak) / (seconds - peak)) };
            (Vec3::new(-back * s, 0.0, 0.3 * back * s), pitch(up * s))
        }
    };
    sequence(&mut model, "fire", 0.45, false, kick(3.5, 8.0, 0.45));
    sequence(&mut model, "altfire", 0.3, false, kick(1.5, 3.0, 0.3));
    model
}

/// A small, steady hash for the pixel art's speckle.
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^ (h >> 16)
}

fn rgb(c: [u8; 3], shift: i32, glow: u8) -> [u8; 4] {
    let f = |v: u8| (i32::from(v) + shift).clamp(0, 255) as u8;
    [f(c[0]), f(c[1]), f(c[2]), glow]
}

/// A stand-in material's 16x16 texture (alpha: the glow, as Half-Life 2's
/// `$selfillum`).
pub fn texture(name: &str) -> Option<Rgba> {
    let upgraded = name.contains("super_");
    let m = MATERIALS.iter().position(|m| name.ends_with(m))?;
    // Half-Life 2's colours: gunmetal and orange; the super gun's white
    // and blue.
    let (colour, glow, seed) = match (m, upgraded) {
        (DARK, false) => ([58, 60, 66], 0, 1),
        (METAL, false) => ([150, 152, 158], 0, 2),
        (ACCENT, false) => ([236, 124, 22], 150, 3),
        (CORE, false) => ([255, 196, 92], 255, 4),
        (DARK, true) => ([150, 156, 168], 0, 5),
        (METAL, true) => ([222, 226, 234], 0, 6),
        (ACCENT, true) => ([70, 160, 255], 170, 7),
        _ => ([176, 232, 255], 255, 8),
    };
    let mut pixels = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let h = hash(x, y, seed);
            // Speckle, a darker rim every 16 pixels, and rivets on metal.
            let mut shift = (h % 21) as i32 - 10;
            if x == 0 || y == 0 {
                shift -= 18;
            }
            if x == 15 || y == 15 {
                shift += 10;
            }
            if m == METAL && (x == 3 || x == 12) && (y == 3 || y == 12) {
                shift -= 45;
            }
            if m == CORE {
                // Hotter in the middle.
                let d = (x as f32 - 7.5).abs().max((y as f32 - 7.5).abs());
                shift += 30 - (d * 6.0) as i32;
            }
            pixels.extend(rgb(colour, shift, glow));
        }
    }
    Some(Rgba { width: 16, height: 16, pixels })
}

/// A glow sprite in pixels, for the sprite with this path (Half-Life 2's
/// `SPRITES` names): orange, warm white or blue, a round blocky glow.
pub fn sprite(path: &str) -> Rgba {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (colour, rings, star) = match name {
        n if n.starts_with("orangecore") => ([255, 150, 40], 6.0, false),
        n if n.starts_with("orangeflare") => ([255, 170, 60], 5.0, true),
        n if n.starts_with("glow04") => ([255, 210, 150], 7.0, false),
        n if n.starts_with("fluttercore") => ([150, 210, 255], 6.0, false),
        n if n.contains("flare") => ([120, 190, 255], 5.0, true),
        _ => ([110, 180, 255], 6.0, false),
    };
    let side = 16u32;
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let d = (dx * dx + dy * dy).sqrt();
            // Stepped rings, Minecraft's particles' look.
            let mut a = (1.0 - (d / 8.0 * rings).floor() / rings).clamp(0.0, 1.0);
            a *= a;
            if star && (dx.abs() < 1.0 || dy.abs() < 1.0) {
                a = a.max((1.0 - d / 8.0).clamp(0.0, 1.0));
            }
            let hot = (1.0 - d / 3.0).clamp(0.0, 1.0);
            let c = colour.map(|c: u8| (f32::from(c) + (255.0 - f32::from(c)) * hot).round() as u8);
            pixels.extend([c[0], c[1], c[2], (a * 255.0).round() as u8]);
        }
    }
    Rgba { width: side, height: side, pixels }
}
