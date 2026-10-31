//! Source studio models: the skeleton, skinned meshes and every animation
//! of an `.mdl` with its `.vvd` (vertices) and `.dx90.vtx` (triangles).
//! Units and axes are Source's: inches, X forward, Y left, Z up (the same
//! as Black Ops II's).

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

fn int_from_bytes(d: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn short_from_bytes(d: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

/// `count` records read one by one; the first unreadable one fails the table.
fn table<T>(count: usize, read: impl FnMut(usize) -> Option<T>) -> Option<Vec<T>> {
    (0..count).map(read).collect()
}

fn float_from_bytes(d: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn vec3_at(d: &[u8], at: usize) -> Option<Vec3> {
    Some(Vec3::new(float_from_bytes(d, at)?, float_from_bytes(d, at + 4)?, float_from_bytes(d, at + 8)?))
}

fn cstr(d: &[u8], at: usize) -> String {
    let Some(tail) = d.get(at..) else {
        return String::new();
    };
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).into_owned()
}

/// A name kept as an offset from the start of the struct that owns it.
fn offset_name(d: &[u8], owner: usize, slot: usize) -> String {
    match int_from_bytes(d, owner + slot) {
        Some(o) if o != 0 => cstr(d, (owner as i64 + i64::from(o)) as usize),
        _ => String::new(),
    }
}

/// A little-endian int read as a count or an offset.
fn index_at(d: &[u8], at: usize) -> Option<usize> {
    int_from_bytes(d, at).map(|v| v as usize)
}

/// A 3x4 matrix, rows of (x y z translation), as a `Mat4`.
fn mat3x4_at(d: &[u8], at: usize) -> Option<Mat4> {
    let mut r = [0f32; 12];
    for (i, v) in r.iter_mut().enumerate() {
        *v = float_from_bytes(d, at + i * 4)?;
    }
    Some(Mat4::from_cols(
        Vec4::new(r[0], r[4], r[8], 0.0),
        Vec4::new(r[1], r[5], r[9], 0.0),
        Vec4::new(r[2], r[6], r[10], 0.0),
        Vec4::new(r[3], r[7], r[11], 1.0),
    ))
}

/// Source's `AngleQuaternion` for a `RadianEuler` (x roll, y pitch, z yaw).
pub fn euler_quat(a: Vec3) -> Quat {
    let (sr, cr) = (a.x * 0.5).sin_cos();
    let (sp, codepoint) = (a.y * 0.5).sin_cos();
    let (sy, cy) = (a.z * 0.5).sin_cos();
    Quat::from_xyzw(
        sr * codepoint * cy - cr * sp * sy,
        cr * sp * cy + sr * codepoint * sy,
        cr * codepoint * sy - sr * sp * cy,
        cr * codepoint * cy + sr * sp * sy,
    )
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let man = f32::from(bits & 0x3ff);
    sign * if exp == 0 { man / 1024.0 * 2f32.powi(-14) } else { (1.0 + man / 1024.0) * 2f32.powi(exp - 15) }
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub parent_slot: Option<usize>,
    pub pos: Vec3,
    pub quat: Quat,
    pub turn_matrix: Vec3,
    pub pos_scale: Vec3,
    pub rot_scale: Vec3,
    /// Model space to the bone's own (the inverse bind pose).
    pub pose_to_bone: Mat4,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub pos: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub bones: [u8; 3],
    pub blend: [f32; 3],
}

/// Triangles that share a texture.
#[derive(Clone, Debug)]
pub struct Mesh {
    /// Index into `Model::textures`.
    pub texture: usize,
    pub vertices: Vec<Vertex>,
    pub tri_list: Vec<u32>,
}

impl Mesh {
    /// No triangles yet, drawn with texture `texture`.
    pub fn empty(texture: usize) -> Self {
        Self { texture, vertices: vec![], tri_list: vec![] }
    }
}

#[derive(Clone, Debug)]
pub struct Texture {
    /// The texture's name, and every folder it may be in
    /// (`materials/<folder><name>.vmt`).
    pub name: String,
    pub folders: Vec<String>,
}

/// One bone's local turn and place in one frame.
pub type Pose = Vec<(Quat, Vec3)>;

/// A bone that is not turned.
pub(crate) const NO_TURN: Quat = Quat::IDENTITY;
/// A vertex that follows its first bone alone.
pub(crate) const ONE_BONE: [f32; 3] = [1.0, 0.0, 0.0];

#[derive(Clone, Debug)]
pub struct Animation {
    pub name: String,
    pub fps: f32,
    pub repeats: bool,
    /// Added on top of another pose, not a whole pose.
    pub delta: bool,
    /// Every frame, every bone.
    pub frames: Vec<Pose>,
}

#[derive(Clone, Debug)]
pub struct Event {
    /// 0..1 through the sequence.
    pub cycle: f32,
    pub code: i32,
    pub name: String,
    pub options: String,
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub name: String,
    pub activity: String,
    pub repeats: bool,
    pub ease_on: f32,
    pub ease_off: f32,
    /// Its animations, a grid `groups[0]` wide and `groups[1]` high.
    pub anims: Vec<usize>,
    pub groups: [usize; 2],
    /// The pose parameter each grid axis follows, with its range.
    pub params: [Option<(usize, f32, f32)>; 2],
    pub events: Vec<Event>,
    /// `STUDIO_*` flags: 0x4 delta, 0x8 autoplay, 0x10 post.
    pub studio_bits: u32,
    /// Sequences laid over this one (`$addlayer`, `$blendlayer`).
    pub layers: Vec<Layer>,
    /// Each bone's share of this sequence when it is laid over another.
    pub weights: Vec<f32>,
}

/// One `mstudioautolayer_t`.
#[derive(Clone, Debug)]
pub struct Layer {
    pub sequence: usize,
    pub pose: Option<usize>,
    /// `STUDIO_AL_*`: 0x40 spline, 0x80 cross-fade, 0x200 no blend,
    /// 0x1000 local, 0x4000 by pose parameter.
    pub layer_bits: u32,
    pub start: f32,
    pub peak: f32,
    pub tail: f32,
    pub end: f32,
}

#[derive(Clone, Debug)]
pub struct PoseParam {
    pub name: String,
    pub start: f32,
    pub end: f32,
}

#[derive(Clone, Debug)]
pub struct Mount {
    pub name: String,
    pub joint: usize,
    pub local: Mat4,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub bones: Vec<Joint>,
    pub textures: Vec<Texture>,
    pub meshes: Vec<Mesh>,
    pub animations: Vec<Animation>,
    pub sequences: Vec<Sequence>,
    pub pose_params: Vec<PoseParam>,
    pub attachments: Vec<Mount>,
}

/// An RLE-packed run of values, read at `at_frame` (`ExtractAnimValue`).
fn rle_value(d: &[u8], run: usize, at_frame: usize) -> Option<f32> {
    let valid = usize::from(*d.get(run)?);
    let total = usize::from(*d.get(run + 1)?);
    if total == 0 {
        return Some(0.0);
    }
    match at_frame.checked_sub(total) {
        // Past this run: the next one starts after its stored values.
        Some(rest) => rle_value(d, run + (valid + 1) * 2, rest),
        // Inside it: the last stored value holds for the frames it has none for.
        None => short_from_bytes(d, run + valid.min(at_frame + 1) * 2).map(f32::from),
    }
}

/// The three tracks of a value pointer (0 = always zero).
fn tracks(d: &[u8], at: usize, frame: usize) -> Option<Vec3> {
    let mut v = [0f32; 3];
    for (j, out) in v.iter_mut().enumerate() {
        let o = short_from_bytes(d, at + j * 2)?;
        if o > 0 {
            *out = rle_value(d, at + o as usize, frame)?;
        }
    }
    Some(Vec3::from_array(v))
}

/// One frame of one section's bone records, over the rest pose.
fn decode_frame(d: &[u8], bones: &[Joint], mut at: usize, frame: usize, delta: bool, pose: &mut Pose) -> Option<()> {
    loop {
        let bone = usize::from(*d.get(at)?);
        let flags = *d.get(at + 1)?;
        let next = short_from_bytes(d, at + 2)?;
        let data = at + 4;
        if let Some(b) = bones.get(bone) {
            let (mut q, mut p) = if delta { (NO_TURN, Vec3::ZERO) } else { (b.quat, b.pos) };
            if flags & 0x02 != 0 {
                // Quaternion48.
                let x = (f32::from(u16_at(d, data)?) - 32768.0) / 32768.0;
                let y = (f32::from(u16_at(d, data + 2)?) - 32768.0) / 32768.0;
                let zw = u16_at(d, data + 4)?;
                let z = (f32::from(zw & 0x7fff) - 16384.0) / 16384.0;
                let mut w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
                if zw & 0x8000 != 0 {
                    w = -w;
                }
                q = Quat::from_xyzw(x, y, z, w);
            } else if flags & 0x20 != 0 {
                // Quaternion64.
                let bits = u64::from_le_bytes(d.get(data..data + 8)?.try_into().ok()?);
                let f = |s: u32| ((bits >> s) & 0x1f_ffff) as f32 - 1_048_576.0;
                let (x, y, z) = (f(0) / 1_048_576.5, f(21) / 1_048_576.5, f(42) / 1_048_576.5);
                let mut w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
                if bits >> 63 != 0 {
                    w = -w;
                }
                q = Quat::from_xyzw(x, y, z, w);
            } else if flags & 0x08 != 0 {
                let mut a = tracks(d, data, frame)? * b.rot_scale;
                if !delta {
                    a += b.turn_matrix;
                }
                q = euler_quat(a);
            }
            if flags & 0x01 != 0 {
                let off = data + if flags & 0x02 != 0 { 6 } else { 0 } + if flags & 0x20 != 0 { 8 } else { 0 };
                p = Vec3::new(half(u16_at(d, off)?), half(u16_at(d, off + 2)?), half(u16_at(d, off + 4)?));
            } else if flags & 0x04 != 0 {
                let off = data + if flags & 0x08 != 0 { 6 } else { 0 };
                let mut v = tracks(d, off, frame)? * b.pos_scale;
                if !delta {
                    v += b.pos;
                }
                p = v;
            }
            pose[bone] = (q, p);
        }
        if next == 0 {
            break Some(());
        }
        at = (at as i64 + i64::from(next)) as usize;
    }
}

/// Reads a model from its three files.
pub fn read(mdl: &[u8], vvd: &[u8], vtx: &[u8]) -> Option<Model> {
    let d = mdl;
    if d.get(0..4)? != b"IDST" {
        return None;
    }
    let version = int_from_bytes(d, 4)?;
    if !(44..=49).contains(&version) {
        return None;
    }
    let model_name = cstr(d, 12);
    // studiohdr_t, after the six vectors at 80.
    let h = 152 + 4;
    let count = |o: usize| -> Option<(usize, usize)> { Some((int_from_bytes(d, h + o)? as usize, int_from_bytes(d, h + o + 4)? as usize)) };
    let (skeleton_joint_total, bone_index) = count(0)?;
    let (num_anims, clip_ordinal) = count(24)?;
    let (num_seqs, seq_index) = count(32)?;
    let (num_textures, texture_index) = count(48)?;
    let (num_cd, cd_index) = count(56)?;
    let num_skinref = int_from_bytes(d, h + 64)? as usize;
    let skin_index = int_from_bytes(d, h + 72)? as usize;
    let (num_bodyparts, bodypart_index) = count(76)?;
    let (num_attach, attach_index) = count(84)?;
    let (num_pose, pose_index) = count(144)?;

    let mut bones = Vec::with_capacity(skeleton_joint_total);
    for i in 0..skeleton_joint_total {
        let b = bone_index + i * 216;
        let parent = int_from_bytes(d, b + 4)?;
        let joint = Joint {
            name: offset_name(d, b, 0),
            parent_slot: usize::try_from(parent).ok(),
            pos: vec3_at(d, b + 32)?,
            quat: Quat::from_xyzw(float_from_bytes(d, b + 44)?, float_from_bytes(d, b + 48)?, float_from_bytes(d, b + 52)?, float_from_bytes(d, b + 56)?),
            turn_matrix: vec3_at(d, b + 60)?,
            pos_scale: vec3_at(d, b + 72)?,
            rot_scale: vec3_at(d, b + 84)?,
            pose_to_bone: mat3x4_at(d, b + 96)?,
        };
        bones.push(joint);
    }

    let folders: Vec<String> = (0..num_cd)
        .map(|i| int_from_bytes(d, cd_index + i * 4).map(|o| crate::vpk::folded(&cstr(d, o as usize))))
        .collect::<Option<_>>()?;
    let textures: Vec<Texture> = (0..num_textures)
        .map(|i| Texture { name: crate::vpk::folded(&offset_name(d, texture_index + i * 64, 0)), folders: folders.clone() })
        .collect();
    // Skin family 0: a mesh's material to its texture.
    let skin: Vec<usize> = (0..num_skinref).map(|i| short_from_bytes(d, skin_index + i * 2).map_or(i, |t| t as usize)).collect();

    let pose_params = table(num_pose, |i| {
        let p = pose_index + i * 20;
        Some(PoseParam { name: offset_name(d, p, 0), start: float_from_bytes(d, p + 8)?, end: float_from_bytes(d, p + 12)? })
    })?;

    let mounts = table(num_attach, |i| {
        let a = attach_index + i * 92;
        Some(Mount { name: offset_name(d, a, 0), joint: int_from_bytes(d, a + 8)? as usize, local: mat3x4_at(d, a + 12)? })
    })?;

    let rest: Pose = bones.iter().map(|b| (b.quat, b.pos)).collect();
    let mut animations = Vec::with_capacity(num_anims);
    for i in 0..num_anims {
        let a = clip_ordinal + i * 100;
        let fps = float_from_bytes(d, a + 8)?;
        let flags = int_from_bytes(d, a + 12)?;
        let frames = int_from_bytes(d, a + 16)?.max(1) as usize;
        let block = int_from_bytes(d, a + 52)?;
        let index = int_from_bytes(d, a + 56)?;
        let section_index = int_from_bytes(d, a + 80)?;
        let section_frames = int_from_bytes(d, a + 84)?.max(0) as usize;
        let delta = flags & 0x0004 != 0;
        let base = if delta { vec![(Quat::IDENTITY, Vec3::ZERO); bones.len()] } else { rest.clone() };
        let mut out = Vec::with_capacity(frames);
        for f in 0..frames {
            let mut pose = base.clone();
            // Animation blocks live in other files; none here.
            if block == 0 {
                let (data, local) = if section_frames > 0 {
                    let s = (f / section_frames).min(frames.saturating_sub(1) / section_frames);
                    let rec = (a as i64 + i64::from(section_index)) as usize + s * 8;
                    let off = int_from_bytes(d, rec + 4)?;
                    ((a as i64 + i64::from(off)) as usize, f - s * section_frames)
                } else {
                    ((a as i64 + i64::from(index)) as usize, f)
                };
                if flags & 0x0020 == 0 {
                    decode_frame(d, &bones, data, local, delta, &mut pose)?;
                }
            }
            out.push(pose);
        }
        animations.push(Animation { name: offset_name(d, a, 4), fps, repeats: flags & 0x0001 != 0, delta, frames: out });
    }

    let mut sequences = Vec::with_capacity(num_seqs);
    for i in 0..num_seqs {
        let s = seq_index + i * 212;
        let groups = [int_from_bytes(d, s + 68)?.max(1) as usize, int_from_bytes(d, s + 72)?.max(1) as usize];
        let anim_list = s + int_from_bytes(d, s + 60)? as usize;
        let anims = (0..groups[0] * groups[1]).map(|k| short_from_bytes(d, anim_list + k * 2).map(|v| v as usize)).collect::<Option<Vec<_>>>()?;
        let mut params = [None, None];
        for (k, p) in params.iter_mut().enumerate() {
            let idx = int_from_bytes(d, s + 76 + k * 4)?;
            if idx >= 0 && groups[k] > 1 {
                *p = Some((idx as usize, float_from_bytes(d, s + 84 + k * 4)?, float_from_bytes(d, s + 92 + k * 4)?));
            }
        }
        let num_events = int_from_bytes(d, s + 24)? as usize;
        let event_index = s + int_from_bytes(d, s + 28)? as usize;
        let events = (0..num_events)
            .map(|k| {
                let e = event_index + k * 80;
                let ty = int_from_bytes(d, e + 8)?;
                Some(Event {
                    cycle: float_from_bytes(d, e)?,
                    code: int_from_bytes(d, e + 4)?,
                    name: if ty & (1 << 10) != 0 { offset_name(d, e, 76) } else { String::new() },
                    options: cstr(d, e + 12),
                })
            })
            .collect::<Option<Vec<Event>>>()?;
        let num_layers = int_from_bytes(d, s + 148)? as usize;
        let layer_index = s + int_from_bytes(d, s + 152)? as usize;
        let layers = (0..num_layers)
            .map(|k| {
                let l = layer_index + k * 24;
                Some(Layer {
                    sequence: short_from_bytes(d, l)? as usize,
                    pose: usize::try_from(short_from_bytes(d, l + 2)?).ok(),
                    layer_bits: int_from_bytes(d, l + 4)? as u32,
                    start: float_from_bytes(d, l + 8)?,
                    peak: float_from_bytes(d, l + 12)?,
                    tail: float_from_bytes(d, l + 16)?,
                    end: float_from_bytes(d, l + 20)?,
                })
            })
            .collect::<Option<Vec<Layer>>>()?;
        let weight_index = s + int_from_bytes(d, s + 156)? as usize;
        let weights = table(bones.len(), |b| float_from_bytes(d, weight_index + b * 4))?;
        sequences.push(Sequence {
            studio_bits: int_from_bytes(d, s + 12)? as u32,
            layers,
            weights,
            name: offset_name(d, s, 4),
            activity: offset_name(d, s, 8),
            repeats: int_from_bytes(d, s + 12)? & 1 != 0,
            ease_on: float_from_bytes(d, s + 104)?,
            ease_off: float_from_bytes(d, s + 108)?,
            anims,
            groups,
            params,
            events,
        });
    }

    // Vertices (LOD 0, after any fixups).
    if vvd.get(0..4)? != b"IDSV" {
        return None;
    }
    let num_fixups = index_at(vvd, 48)?;
    let fixup_start = int_from_bytes(vvd, 52)? as usize;
    let vertex_start = index_at(vvd, 56)?;
    let lod0 = int_from_bytes(vvd, 16)? as usize;
    let read_vertex = |i: usize| -> Option<Vertex> {
        let v = vertex_start + i * 48;
        let numbones = usize::from(*vvd.get(v + 15)?);
        let mut weights = [float_from_bytes(vvd, v)?, float_from_bytes(vvd, v + 4)?, float_from_bytes(vvd, v + 8)?];
        for w in weights.iter_mut().skip(numbones.max(1)) {
            *w = 0.0;
        }
        Some(Vertex {
            blend: weights,
            bones: [*vvd.get(v + 12)?, *vvd.get(v + 13)?, *vvd.get(v + 14)?],
            pos: vec3_at(vvd, v + 16)?,
            normal: vec3_at(vvd, v + 28)?,
            uv: Vec2::new(float_from_bytes(vvd, v + 40)?, float_from_bytes(vvd, v + 44)?),
        })
    };
    let mut all = Vec::with_capacity(lod0);
    if num_fixups == 0 {
        for i in 0..lod0 {
            all.push(read_vertex(i)?);
        }
    } else {
        for f in 0..num_fixups {
            let x = fixup_start + f * 12;
            if int_from_bytes(vvd, x)? >= 0 {
                let from = int_from_bytes(vvd, x + 4)? as usize;
                for i in 0..int_from_bytes(vvd, x + 8)? as usize {
                    all.push(read_vertex(from + i)?);
                }
            }
        }
    }

    // Triangles: body part 0's model 0 (the default body), LOD 0.
    let vtx_parts = int_from_bytes(vtx, 28)? as usize;
    let vtx_part_index = index_at(vtx, 32)?;
    let mut meshes: Vec<Mesh> = Vec::new();
    for bp in 0..num_bodyparts.min(vtx_parts) {
        let mbp = bodypart_index + bp * 16;
        let model_slot_total = int_from_bytes(d, mbp + 4)? as usize;
        if model_slot_total == 0 {
            continue;
        }
        let mm = (mbp as i64 + i64::from(int_from_bytes(d, mbp + 12)?)) as usize;
        let num_meshes = int_from_bytes(d, mm + 72)? as usize;
        let mesh_index = mm + int_from_bytes(d, mm + 76)? as usize;
        let model_first = int_from_bytes(d, mm + 84)? as usize / 48;

        let vbp = vtx_part_index + bp * 8;
        let vm = vbp + int_from_bytes(vtx, vbp + 4)? as usize;
        let vlod = vm + int_from_bytes(vtx, vm + 4)? as usize;
        let vlod_meshes = int_from_bytes(vtx, vlod)? as usize;
        let vmesh_index = vlod + int_from_bytes(vtx, vlod + 4)? as usize;
        for m in 0..num_meshes.min(vlod_meshes) {
            let sm = mesh_index + m * 116;
            let material = int_from_bytes(d, sm)? as usize;
            let mesh_first = int_from_bytes(d, sm + 12)? as usize;
            let mut mesh = Mesh::empty(*skin.get(material).unwrap_or(&material));
            let vmesh = vmesh_index + m * 9;
            let groups = int_from_bytes(vtx, vmesh)? as usize;
            let group_index = vmesh + int_from_bytes(vtx, vmesh + 4)? as usize;
            for g in 0..groups {
                let sg = group_index + g * 25;
                let nverts = int_from_bytes(vtx, sg)? as usize;
                let vert_at = sg + int_from_bytes(vtx, sg + 4)? as usize;
                let nidx = int_from_bytes(vtx, sg + 8)? as usize;
                let idx_at = sg + int_from_bytes(vtx, sg + 12)? as usize;
                let nstrips = int_from_bytes(vtx, sg + 16)? as usize;
                let strip_at = sg + int_from_bytes(vtx, sg + 20)? as usize;
                let base = mesh.vertices.len() as u32;
                for v in 0..nverts {
                    let orig = usize::from(u16_at(vtx, vert_at + v * 9 + 4)?);
                    mesh.vertices.push(*all.get(model_first + mesh_first + orig)?);
                }
                let index = |i: usize| -> Option<u32> { Some(base + u32::from(u16_at(vtx, idx_at + i * 2)?)) };
                for s in 0..nstrips {
                    let st = strip_at + s * 27;
                    let n = int_from_bytes(vtx, st)? as usize;
                    let first = int_from_bytes(vtx, st + 4)? as usize;
                    let flags = *vtx.get(st + 18)?;
                    if first + n > nidx {
                        return None;
                    }
                    if flags & 2 != 0 {
                        for t in 0..n.saturating_sub(2) {
                            let (a, b, c) = (index(first + t)?, index(first + t + 1)?, index(first + t + 2)?);
                            if a != b && b != c && a != c {
                                mesh.tri_list.extend(if t % 2 == 0 { [a, b, c] } else { [b, a, c] });
                            }
                        }
                    } else {
                        let run = table(n, |step| index(first + step))?;
                        mesh.tri_list.extend(run);
                    }
                }
            }
            if !mesh.tri_list.is_empty() {
                meshes.push(mesh);
            }
        }
    }

    Some(Model { name: model_name, bones, textures, meshes, animations, sequences, pose_params, attachments: mounts })
}

impl Model {
    /// A sequence by its name or activity.
    pub fn sequence(&self, wanted: &str) -> Option<usize> {
        self.sequences.iter().position(|s| s.name.eq_ignore_ascii_case(wanted) || s.activity.eq_ignore_ascii_case(wanted))
    }

    pub fn pose_param(&self, wanted: &str) -> Option<usize> {
        self.pose_params.iter().position(|p| p.name.eq_ignore_ascii_case(wanted))
    }

    /// An animation's pose `t` seconds in (held on its last frame, or
    /// wrapped if it loops), blended between frames.
    pub fn pose_at(&self, anim: usize, t: f32) -> Pose {
        let a = &self.animations[anim];
        let n = a.frames.len();
        if n <= 1 {
            return a.frames[0].clone();
        }
        let f = (t * a.fps).max(0.0);
        let f = if a.repeats { f % (n - 1) as f32 } else { f.min((n - 1) as f32) };
        let i = (f.floor() as usize).min(n - 2);
        let s = f - i as f32;
        a.frames[i].iter().zip(&a.frames[i + 1]).map(|(x, y)| (x.0.slerp(y.0, s), x.1.lerp(y.1, s))).collect()
    }

    /// A sequence's length in seconds (its first animation's).
    pub fn duration(&self, seq: usize) -> f32 {
        let a = &self.animations[self.sequences[seq].anims[0]];
        (a.frames.len().saturating_sub(1)) as f32 / a.fps.max(1.0)
    }

    /// A sequence's pose at cycle `t` seconds, its blend grid read at the
    /// pose parameters `params` (each in its own range).
    pub fn sequence_pose(&self, seq: usize, t: f32, params: &[f32]) -> Pose {
        const NO_AXIS: (usize, usize, f32) = (0, 0, 0.0);
        let s = &self.sequences[seq];
        let axis = |k: usize| -> (usize, usize, f32) {
            let Some((p, start, end)) = s.params[k] else { return NO_AXIS };
            let n = s.groups[k];
            let v = params.get(p).copied().unwrap_or(0.0);
            let u = if (end - start).abs() > 1e-6 { ((v - start) / (end - start)).clamp(0.0, 1.0) } else { 0.0 };
            let x = u * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 1);
            (i, (i + 1).min(n - 1), x - i as f32)
        };
        let (x0, x1, fx) = axis(0);
        let (y0, y1, fy) = axis(1);
        let w = s.groups[0];
        let at = |x: usize, y: usize| self.pose_at(s.anims[y * w + x], t);
        let blend = |from: Pose, to: Pose, amount: f32| -> Pose {
            if amount > 0.0 {
                from.iter().zip(&to).map(|(x, y)| (x.0.slerp(y.0, amount), x.1.lerp(y.1, amount))).collect()
            } else {
                from
            }
        };
        let row0 = blend(at(x0, y0), at(x1, y0), fx);
        if fy <= 0.0 {
            return row0;
        }
        blend(row0, blend(at(x0, y1), at(x1, y1), fx), fy)
    }

    /// Lays sequence `seq` at `cycle` (0..1) over `pose` by `amount`, as
    /// Source's `AccumulatePose` and `SlerpBones`: a delta sequence adds
    /// its rotation (before the bone's own, or after it if post) and its
    /// offset; any other blends towards its pose; each bone by the
    /// sequence's own share of it.
    pub fn accumulate(&self, pose: &mut Pose, seq: usize, cycle: f32, params: &[f32], amount: f32) {
        if let Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) = amount.partial_cmp(&0.0) {
            return;
        }
        let s = &self.sequences[seq];
        let layer = self.sequence_pose(seq, cycle * self.duration(seq), params);
        // A delta animation decodes from identity, so it adds even when
        // the sequence itself is not marked delta.
        let delta = s.studio_bits & 0x4 != 0 || s.anims.iter().any(|&a| self.animations[a].delta);
        let post = s.studio_bits & 0x10 != 0;
        for (i, (q2, p2)) in layer.into_iter().enumerate() {
            let Some(share) = Some(amount * s.weights.get(i).map_or(1.0, |w| *w)).filter(|v| *v > 0.0) else {
                continue;
            };
            let (q1, p1) = pose[i];
            pose[i] = if delta {
                let scaled = Quat::IDENTITY.slerp(q2, share);
                let q = if post { q1 * scaled } else { scaled * q1 };
                (q.normalize(), p1 + p2 * share)
            } else {
                let capped = share.min(1.0);
                (q1.slerp(q2, capped), p1.lerp(p2, capped))
            };
        }
    }

    /// A sequence's full pose: its own, its layers (`AddSequenceLayers`)
    /// and every autoplay sequence (`CalcAutoplaySequences`, cycling on
    /// `clock` seconds).
    pub fn pose(&self, seq: usize, t: f32, params: &[f32], clock: f32) -> Pose {
        let mut pose = self.sequence_pose(seq, t, params);
        let len = self.duration(seq).max(1e-3);
        let cycle = if self.sequences[seq].repeats { (t / len).fract() } else { (t / len).clamp(0.0, 1.0) };
        for layer in &self.sequences[seq].layers {
            if layer.layer_bits & 0x1000 != 0 || layer.sequence >= self.sequences.len() {
                continue;
            }
            let (layer_weight, layer_cycle) = if layer.start == layer.end {
                (1.0, cycle)
            } else {
                let by_pose = layer.layer_bits & 0x4000 != 0;
                let index = match (by_pose, layer.pose) {
                    (true, pose_slot) => pose_slot.and_then(|p| params.get(p).copied()).unwrap_or(0.0),
                    (false, _) => cycle,
                };
                if !(layer.start..layer.end).contains(&index) {
                    continue;
                }
                let ramp = if index < layer.peak && layer.start != layer.peak {
                    (index - layer.start) / (layer.peak - layer.start)
                } else if index > layer.tail && layer.end != layer.tail {
                    (layer.end - index) / (layer.end - layer.tail)
                } else {
                    1.0
                };
                let weight = if layer.layer_bits & 0x40 != 0 { ramp * ramp * (3.0 - 2.0 * ramp) } else { ramp };
                let local = if by_pose { cycle } else { (cycle - layer.start) / (layer.end - layer.start) };
                (weight, local)
            };
            self.accumulate(&mut pose, layer.sequence, layer_cycle, params, layer_weight);
        }
        for (i, s) in self.sequences.iter().enumerate() {
            if s.studio_bits & 0x8 != 0 && i != seq {
                let len = self.duration(i).max(1e-3);
                self.accumulate(&mut pose, i, (clock / len).fract(), params, 1.0);
            }
        }
        pose
    }

    /// Each bone's model-space matrix for a pose.
    pub fn joint_global_transform(&self, pose: &Pose) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let (q, p) = pose[i];
            let local = Mat4::from_rotation_translation(q, p);
            out.push(match b.parent_slot {
                Some(parent) if parent < i => out[parent] * local,
                _ => local,
            });
        }
        out
    }

    /// Skinning joint_transforms: bind pose to the pose's model space.
    pub fn skin(&self, pose: &Pose) -> Vec<Mat4> {
        self.joint_global_transform(pose).iter().zip(&self.bones).map(|(w, b)| *w * b.pose_to_bone).collect()
    }
}

#[cfg(test)]
mod anim_tests {
    use super::*;

    /// Run 1: valid 2, total 5, values 10 -20; run 2: valid 1, total 3, value 7; run 3: total 0.
    fn runs() -> Vec<u8> {
        let mut d = vec![2u8, 5];
        d.extend(10i16.to_le_bytes());
        d.extend((-20i16).to_le_bytes());
        d.extend([1u8, 3]);
        d.extend(7i16.to_le_bytes());
        d.extend([0u8, 0]);
        d
    }

    #[test]
    fn rle_track_values() {
        let d = runs();
        let at = |f: usize| rle_value(&d, 0, f);
        assert_eq!(at(0), Some(-20.0 + 30.0));
        assert_eq!(at(1), Some(-20.0));
        assert_eq!(at(4), Some(-20.0));
        assert_eq!(at(5), Some(7.0));
        assert_eq!(at(7), Some(7.0));
        assert_eq!(at(8), Some(0.0));
        assert_eq!(at(100), Some(0.0));
        assert_eq!(rle_value(&d[..3], 0, 1), None);
        assert_eq!(rle_value(&d, 40, 0), None);
    }
}
