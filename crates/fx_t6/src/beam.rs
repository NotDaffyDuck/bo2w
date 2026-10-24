//! BO2's beam list and beam mesh (FX_Beam_*), from facts/fx-beam-laser-t6.md
//! section 2. Item numbers below are that section's.

use crate::runtime::{vec3_length_sq, vec3_normalize};

/// Beams past this many in one frame are dropped (item 3). MW2 holds 48.
pub const FX_BEAM_ADD_CAP: usize = 0x60;

pub const FX_BEAM_MAX_SEGMENTS: u8 = 0xff;

/// Plain tracers multiply their length by 8 before the segment divide (item 4).
pub const FX_TRACER_SEGMENT_LENGTH_SCALE: f32 = 8.0;

/// The flat delta is "no beam" under this projected span squared (item 14).
pub const FX_BEAM_FLAT_DELTA_MIN_LEN_SQ: f32 = 2e-6;

// BO2 unknown: the flat-delta near plane and perspective constant (item 14);
// MW2's z-near 1 and K = 2047/2048 are kept.
pub const FX_INFINITE_PERSPECTIVE_K: f32 = 2047.0 / 2048.0;
pub const FX_CREATE_CLIP_ZNEAR: f32 = 1.0;

// BO2 unknown: the tracer path's near cut and first-person width cap are not
// in the facts; MW2's values are kept.
pub const FX_TRACER_MIN_DIST: f32 = 5.0;
pub const FX_TRACER_FIRST_PERSON_MAX_WIDTH: f32 = 4.0;

/// A draw-surf override of 0 is drawn with this one (item 15).
pub const FX_BEAM_DRAW_SURF_OVERRIDE_DEFAULT: u8 = 0x18;

/// The colour keys of a 5-colour beam change at these points along it (item 10).
pub const FX_BEAM_COLOR_BREAKS: [f32; 3] = [0.1, 0.5, 0.9];

/// Wiggle offsets by segment index mod 8 (item 8).
pub const FX_BEAM_WIGGLE: [[f32; 2]; 8] = [
    [0.0, 1.0],
    [0.71, 0.71],
    [1.0, 0.0],
    [0.71, -0.71],
    [0.0, -1.0],
    [-0.71, -0.71],
    [-1.0, 0.0],
    [-0.71, 0.71],
];

/// The beam's type byte and the union fields each type carries (items 1-2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxBeamKind {
    Tracer(FxTracerBeam),
    SmokeTracer(FxTracerBeam),
    Laser {
        end_radius: f32,
    },
    /// The flame's second quad mesh (item 15) is not built here.
    Flame {
        end_radius: f32,
    },
    DemoLine,
    Arc {
        end_radius: f32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxTracerBeam {
    pub wiggle_dist: f32,
    pub tex_repeats: f32,
    pub segment_len: f32,
}

impl FxBeamKind {
    #[must_use]
    pub const fn type_byte(self) -> u8 {
        match self {
            Self::Tracer(_) => 0,
            Self::SmokeTracer(_) => 1,
            Self::Laser { .. } => 2,
            Self::Flame { .. } => 3,
            Self::DemoLine => 4,
            Self::Arc { .. } => 5,
        }
    }

    /// 5 colour keys for the two tracer types, begin and end for the rest (item 9).
    #[must_use]
    pub const fn color_count(self) -> usize {
        match self {
            Self::Tracer(_) | Self::SmokeTracer(_) => 5,
            _ => 2,
        }
    }

    const fn tracer(self) -> Option<FxTracerBeam> {
        match self {
            Self::Tracer(t) | Self::SmokeTracer(t) => Some(t),
            _ => None,
        }
    }
}

/// One queued beam. `colors` are BO2's packed bytes, RGBA; only the first
/// `kind.color_count()` are read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxBeamTess {
    pub kind: FxBeamKind,
    pub depth_hack: bool,
    pub draw_surf_override: u8,
    pub begin: [f32; 3],
    pub end: [f32; 3],
    pub colors: [[u8; 4]; 5],
    pub radius: f32,
    /// Set by the caller (item 1): `tracer_segment_count` for tracers, 1 for lasers.
    pub segment_count: u8,
}

impl FxBeamTess {
    /// Only lasers and arcs widen along the beam (item 7).
    // TODO(fact): item 7 also names the flame's union end radius; whether a
    // flame widens is not settled, so it keeps one width here.
    #[must_use]
    pub const fn end_radius(&self) -> f32 {
        match self.kind {
            FxBeamKind::Laser { end_radius } | FxBeamKind::Arc { end_radius } => end_radius,
            _ => self.radius,
        }
    }

    #[must_use]
    pub const fn draw_surf_override(&self) -> u8 {
        if self.draw_surf_override == 0 {
            FX_BEAM_DRAW_SURF_OVERRIDE_DEFAULT
        } else {
            self.draw_surf_override
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxBeamVert {
    pub xyz: [f32; 3],
    pub uv: [f32; 2],
    /// Already packed for the vertex: RGB divided by 4 except on smoke tracers.
    pub color: [u8; 4],
}

#[inline]
#[must_use]
pub const fn beam_vert_count(seg_count: usize) -> usize {
    2 * seg_count + 4
}

#[inline]
#[must_use]
pub const fn beam_index_count(seg_count: usize) -> usize {
    3 * (2 * seg_count + 2)
}

#[inline]
#[must_use]
pub fn beam_add_allows(live: usize) -> bool {
    live < FX_BEAM_ADD_CAP
}

/// The tracer segment count (item 4): length over the screw distance,
/// truncated, plain tracers at 8x length; under 2 is 1, 255 and up is 255.
#[must_use]
pub fn tracer_segment_count(smoke: bool, length: f32, screw_dist: f32) -> u8 {
    // BO2 unknown: a zero screw distance; MW2's one segment is kept.
    if screw_dist < 1e-5 {
        return 1;
    }
    let length = if smoke {
        length
    } else {
        length * FX_TRACER_SEGMENT_LENGTH_SCALE
    };
    let n = (length / screw_dist) as i32;
    if n < 2 {
        1
    } else {
        n.min(i32::from(FX_BEAM_MAX_SEGMENTS)) as u8
    }
}

/// A 0..1 colour as a beam key byte.
// BO2 unknown: how the callers pack their colours; MW2's round is kept.
#[inline]
#[must_use]
pub fn beam_color_key(color: [f32; 4]) -> [u8; 4] {
    color.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// The blended key bytes at `t` along the beam (item 10), before the divide.
#[must_use]
pub fn beam_sample_color(kind: FxBeamKind, colors: &[[u8; 4]; 5], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    if kind.color_count() == 2 {
        return blend_key(colors[0], colors[1], t);
    }
    let [b0, b1, b2] = FX_BEAM_COLOR_BREAKS;
    let (i, lo, hi) = if t < b0 {
        (0, 0.0, b0)
    } else if t < b1 {
        (1, b0, b1)
    } else if t < b2 {
        (2, b1, b2)
    } else {
        (3, b2, 1.0)
    };
    blend_key(colors[i], colors[i + 1], (t - lo) / (hi - lo))
}

/// Linear on bytes, truncated, clamped to 0..255 (item 11).
fn blend_key(a: [u8; 4], b: [u8; 4], f: f32) -> [u8; 4] {
    core::array::from_fn(|c| {
        let (a, b) = (f32::from(a[c]), f32::from(b[c]));
        ((a + (b - a) * f) as i32).clamp(0, 255) as u8
    })
}

/// The vertex colour: RGB shifted right 2 on every type but smoke tracer;
/// alpha is never divided (item 11).
#[inline]
#[must_use]
pub fn beam_vertex_color(kind: FxBeamKind, key: [u8; 4]) -> [u8; 4] {
    if matches!(kind, FxBeamKind::SmokeTracer(_)) {
        key
    } else {
        [key[0] >> 2, key[1] >> 2, key[2] >> 2, key[3]]
    }
}

/// The two body UVs of the vertex pair at `t` (item 12).
#[must_use]
pub fn beam_body_uvs(kind: FxBeamKind, t: f32) -> [[f32; 2]; 2] {
    match kind {
        FxBeamKind::Laser { .. } | FxBeamKind::DemoLine => [[1.0, 0.0], [0.0, 1.0]],
        // BO2 unknown: the tracer's u start value; u runs from 0 over the repeats.
        FxBeamKind::Tracer(tr) => {
            let u = t * tr.tex_repeats;
            [[u, 0.0], [u, 1.0]]
        }
        // BO2 unknown: the smoke tracer's tiled run, and the flame and arc
        // mappings, were not read exactly; MW2's run is kept.
        FxBeamKind::SmokeTracer(_) | FxBeamKind::Flame { .. } | FxBeamKind::Arc { .. } => {
            [[t, 0.0], [t, 1.0]]
        }
    }
}

// BO2 unknown: the cap-vertex UVs per type (item 12); MW2's are kept.
const BEAM_BEGIN_CAP_UV: [f32; 2] = [0.0, 0.5];
const BEAM_END_CAP_UV: [f32; 2] = [1.0, 0.5];

#[inline]
#[must_use]
pub fn mat4_mul_vec4(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    core::array::from_fn(|r| m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2] + m[12 + r] * v[3])
}

#[must_use]
pub fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        let c = mat4_mul_vec4(
            a,
            [b[4 * col], b[4 * col + 1], b[4 * col + 2], b[4 * col + 3]],
        );
        out[4 * col..4 * col + 4].copy_from_slice(&c);
    }
    out
}

#[must_use]
pub fn infinite_perspective_matrix(tan_half_x: f32, tan_half_y: f32, z_near: f32) -> [f32; 16] {
    let k = FX_INFINITE_PERSPECTIVE_K;
    let mut m = [0.0; 16];
    m[0] = k / tan_half_x;
    m[5] = k / tan_half_y;
    m[10] = k;
    m[11] = 1.0;
    m[14] = -z_near * k;
    m
}

#[must_use]
pub fn matrix_for_viewer(origin: [f32; 3], axis: [[f32; 3]; 3]) -> [f32; 16] {
    let right_n = scale(axis[1], -1.0);
    let (up, fwd) = (axis[2], axis[0]);
    [
        right_n[0],
        up[0],
        fwd[0],
        0.0,
        right_n[1],
        up[1],
        fwd[1],
        0.0,
        right_n[2],
        up[2],
        fwd[2],
        0.0,
        -dot(origin, right_n),
        -dot(origin, up),
        -dot(origin, fwd),
        1.0,
    ]
}

#[must_use]
pub fn create_clip_matrix(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    tan_half_x: f32,
    tan_half_y: f32,
) -> [f32; 16] {
    let proj = infinite_perspective_matrix(tan_half_x, tan_half_y, FX_CREATE_CLIP_ZNEAR);
    mat4_mul(&proj, &matrix_for_viewer(origin, axis))
}

/// Clips the homogeneous pair to the side of `plane` where it is >= 0;
/// false when both ends are outside.
pub fn beam_clip_against_plane(begin: &mut [f32; 4], end: &mut [f32; 4], plane: [f32; 4]) -> bool {
    let d = |p: &[f32; 4]| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3] * p[3];
    let (d_begin, d_end) = (d(begin), d(end));
    let (begin_in, end_in) = (d_begin >= 0.0, d_end >= 0.0);
    if !begin_in && !end_in {
        return false;
    }
    let denom = d_end - d_begin;
    let t = if denom != 0.0 { 1.0 / denom } else { f32::MAX };
    let hit: [f32; 4] = core::array::from_fn(|i| (begin[i] - end[i]) * t * d_end + end[i]);
    if !end_in {
        *end = hit;
    }
    if !begin_in {
        *begin = hit;
    }
    true
}

/// The screen-flat direction of the beam, clipped to the near and far
/// planes; None under the minimum span (FX_GenerateBeam_GetFlatDelta).
#[must_use]
pub fn generate_beam_get_flat_delta(
    clip: &[f32; 16],
    inv_clip: &[f32; 16],
    begin: [f32; 3],
    end: [f32; 3],
) -> Option<[f32; 3]> {
    let mut b = mat4_mul_vec4(clip, [begin[0], begin[1], begin[2], 1.0]);
    let mut e = mat4_mul_vec4(clip, [end[0], end[1], end[2], 1.0]);
    if !beam_clip_against_plane(&mut b, &mut e, [0.0, 0.0, 1.0, 0.0])
        || !beam_clip_against_plane(&mut b, &mut e, [0.0, 0.0, -1.0, 1.0])
    {
        return None;
    }
    if b[3] == 0.0 || e[3] == 0.0 {
        return None;
    }
    let dx = e[0] / e[3] - b[0] / b[3];
    let dy = e[1] / e[3] - b[1] / b[3];
    let world = mat4_mul_vec4(inv_clip, [dx, dy, 0.0, 0.0]);
    let len_sq = world.iter().map(|c| c * c).sum::<f32>();
    if len_sq < FX_BEAM_FLAT_DELTA_MIN_LEN_SQ {
        return None;
    }
    let inv = 1.0 / libm::sqrtf(len_sq);
    Some([world[0] * inv, world[1] * inv, world[2] * inv])
}

#[must_use]
pub fn beam_perp_from_flat(view_axis: [f32; 3], flat: [f32; 3]) -> Option<[f32; 3]> {
    let p = cross(view_axis, flat);
    (vec3_length_sq(p) != 0.0).then(|| vec3_normalize(p))
}

/// The flat and sideways directions without a clip matrix: the beam
/// direction less its part along the view.
#[must_use]
pub fn beam_flat_basis(
    begin: [f32; 3],
    end: [f32; 3],
    view_pos: [f32; 3],
    view_fwd: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let delta = sub(end, begin);
    let len_sq = vec3_length_sq(delta);
    if len_sq < 1e-10 {
        return None;
    }
    let beam_dir = scale(delta, 1.0 / libm::sqrtf(len_sq));
    let mut fwd = vec3_normalize(view_fwd);
    if vec3_length_sq(fwd) < 1e-6 {
        fwd = vec3_normalize(sub(scale(add(begin, end), 0.5), view_pos));
    }
    if vec3_length_sq(fwd) < 1e-6 {
        return None;
    }
    let mut flat = sub(beam_dir, scale(fwd, dot(beam_dir, fwd)));
    if vec3_length_sq(flat) < 1e-8 {
        flat = cross(fwd, [0.0, 1.0, 0.0]);
        if vec3_length_sq(flat) < 1e-8 {
            flat = cross(fwd, [1.0, 0.0, 0.0]);
        }
    }
    let flat = vec3_normalize(flat);
    let mut perp = cross(flat, fwd);
    if vec3_length_sq(perp) < 1e-8 {
        perp = cross(beam_dir, fwd);
    }
    if vec3_length_sq(perp) < 1e-8 {
        return None;
    }
    Some((flat, vec3_normalize(perp)))
}

/// One beam's mesh (FX_Beam_GenerateVerts): a cap vertex, a pair a width
/// apart at each segment step, a cap vertex. Returns the vertex and index
/// counts, or None when the buffers are short or the beam has no flat delta.
pub fn beam_generate_verts(
    beam: &FxBeamTess,
    view_pos: [f32; 3],
    view_fwd: [f32; 3],
    clip: Option<(&[f32; 16], &[f32; 16])>,
    verts: &mut [FxBeamVert],
    indices: &mut [u16],
) -> Option<(usize, usize)> {
    let seg_count = usize::from(beam.segment_count.max(1));
    let n_verts = beam_vert_count(seg_count);
    let n_idx = beam_index_count(seg_count);
    if verts.len() < n_verts || indices.len() < n_idx {
        return None;
    }
    let (flat, perp) = match clip {
        Some((clip_m, inv_m)) => {
            let flat = generate_beam_get_flat_delta(clip_m, inv_m, beam.begin, beam.end)?;
            (flat, beam_perp_from_flat(view_fwd, flat)?)
        }
        None => beam_flat_basis(beam.begin, beam.end, view_pos, view_fwd)?,
    };
    let kind = beam.kind;
    let color_at = |t: f32| beam_vertex_color(kind, beam_sample_color(kind, &beam.colors, t));
    let wiggle_dist = kind.tracer().map_or(0.0, |t| t.wiggle_dist);
    let end_radius = beam.end_radius();

    verts[0] = FxBeamVert {
        xyz: sub(beam.begin, scale(flat, beam.radius)),
        uv: BEAM_BEGIN_CAP_UV,
        color: color_at(0.0),
    };
    for seg in 0..=seg_count {
        let t = seg as f32 / seg_count as f32;
        let [wx, wy] = FX_BEAM_WIGGLE[seg % 8].map(|w| w * wiggle_dist);
        let base = add(
            lerp(beam.begin, beam.end, t),
            add(scale(perp, wx), scale(flat, wy)),
        );
        let radius = beam.radius + (end_radius - beam.radius) * t;
        let color = color_at(t);
        let [uv_a, uv_b] = beam_body_uvs(kind, t);
        verts[1 + 2 * seg] = FxBeamVert {
            xyz: sub(base, scale(perp, radius)),
            uv: uv_a,
            color,
        };
        verts[2 + 2 * seg] = FxBeamVert {
            xyz: add(base, scale(perp, radius)),
            uv: uv_b,
            color,
        };
    }
    verts[n_verts - 1] = FxBeamVert {
        xyz: add(beam.end, scale(flat, end_radius)),
        uv: BEAM_END_CAP_UV,
        color: color_at(1.0),
    };
    Some((n_verts, write_beam_indices(seg_count, indices)))
}

fn write_beam_indices(seg_count: usize, out: &mut [u16]) -> usize {
    out[..3].copy_from_slice(&[0, 2, 1]);
    let mut i = 3;
    for seg in 0..seg_count {
        let o = (2 * seg) as u16;
        out[i..i + 6].copy_from_slice(&[o + 2, o + 4, o + 1, o + 1, o + 4, o + 3]);
        i += 6;
    }
    let last = (2 * seg_count) as u16;
    out[i..i + 3].copy_from_slice(&[last + 1, last + 2, last + 3]);
    i + 3
}

#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add(a, scale(sub(b, a), t))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE5: [[u8; 4]; 5] = [[255; 4]; 5];

    fn laser(end_radius: f32) -> FxBeamTess {
        FxBeamTess {
            kind: FxBeamKind::Laser { end_radius },
            depth_hack: false,
            draw_surf_override: 0,
            begin: [0.0, 0.0, 0.0],
            end: [100.0, 0.0, 0.0],
            colors: WHITE5,
            radius: 0.35,
            segment_count: 1,
        }
    }

    #[test]
    fn tracer_segments_truncate_and_plain_is_eight_times() {
        assert_eq!(tracer_segment_count(true, 99.0, 10.0), 9);
        assert_eq!(tracer_segment_count(false, 10.0, 10.0), 8);
        assert_eq!(tracer_segment_count(true, 19.0, 10.0), 1);
        assert_eq!(tracer_segment_count(false, 1000.0, 1.0), 255);
        assert_eq!(tracer_segment_count(true, 50.0, 0.0), 1);
    }

    #[test]
    fn five_keys_break_at_tenth_half_and_nine_tenths() {
        let mut keys = [[0u8; 4]; 5];
        for (i, k) in keys.iter_mut().enumerate() {
            *k = [40 * i as u8; 4];
        }
        let kind = FxBeamKind::Tracer(FxTracerBeam::default());
        assert_eq!(beam_sample_color(kind, &keys, 0.05)[0], 20);
        assert_eq!(beam_sample_color(kind, &keys, 0.1)[0], 40);
        assert_eq!(beam_sample_color(kind, &keys, 0.3)[0], 60);
        assert_eq!(beam_sample_color(kind, &keys, 0.9)[0], 120);
        assert_eq!(beam_sample_color(kind, &keys, 1.0)[0], 160);
    }

    #[test]
    fn two_key_types_blend_begin_to_end() {
        let mut keys = [[0u8; 4]; 5];
        keys[1] = [200; 4];
        keys[4] = [9; 4];
        let kind = FxBeamKind::DemoLine;
        assert_eq!(beam_sample_color(kind, &keys, 0.5)[0], 100);
        assert_eq!(beam_sample_color(kind, &keys, 1.0)[0], 200);
    }

    #[test]
    fn rgb_is_quartered_except_smoke_tracer() {
        let key = [255, 128, 8, 200];
        assert_eq!(
            beam_vertex_color(FxBeamKind::DemoLine, key),
            [63, 32, 2, 200]
        );
        let smoke = FxBeamKind::SmokeTracer(FxTracerBeam::default());
        assert_eq!(beam_vertex_color(smoke, key), key);
    }

    #[test]
    fn only_laser_and_arc_widen() {
        assert_eq!(laser(2.0).end_radius(), 2.0);
        let mut b = laser(2.0);
        b.kind = FxBeamKind::Flame { end_radius: 2.0 };
        assert_eq!(b.end_radius(), 0.35);
        b.kind = FxBeamKind::Arc { end_radius: 3.0 };
        assert_eq!(b.end_radius(), 3.0);
    }

    #[test]
    fn override_zero_draws_as_0x18() {
        let mut b = laser(1.0);
        assert_eq!(b.draw_surf_override(), 0x18);
        b.draw_surf_override = 0xf;
        assert_eq!(b.draw_surf_override(), 0xf);
    }

    #[test]
    fn laser_mesh_uses_fixed_uvs_and_widens() {
        let b = laser(1.0);
        let mut verts = [FxBeamVert {
            xyz: [0.0; 3],
            uv: [0.0; 2],
            color: [0; 4],
        }; 6];
        let mut idx = [0u16; 12];
        let got = beam_generate_verts(
            &b,
            [50.0, -100.0, 0.0],
            [0.0, 1.0, 0.0],
            None,
            &mut verts,
            &mut idx,
        );
        assert_eq!(got, Some((6, 12)));
        assert_eq!((verts[1].uv, verts[2].uv), ([1.0, 0.0], [0.0, 1.0]));
        let width = |a: &FxBeamVert, b: &FxBeamVert| libm::sqrtf(vec3_length_sq(sub(a.xyz, b.xyz)));
        assert!((width(&verts[1], &verts[2]) - 0.7).abs() < 1e-5);
        assert!((width(&verts[3], &verts[4]) - 2.0).abs() < 1e-5);
        assert_eq!(verts[1].color, [63, 63, 63, 255]);
        assert_eq!(&idx[..6], &[0, 2, 1, 2, 4, 1]);
        assert_eq!(&idx[9..], &[3, 4, 5]);
    }

    #[test]
    fn short_buffers_build_nothing() {
        let b = laser(1.0);
        let mut verts = [FxBeamVert {
            xyz: [0.0; 3],
            uv: [0.0; 2],
            color: [0; 4],
        }; 5];
        let mut idx = [0u16; 12];
        assert!(
            beam_generate_verts(&b, [0.0; 3], [0.0, 1.0, 0.0], None, &mut verts, &mut idx)
                .is_none()
        );
    }

    #[test]
    fn beam_cap_is_96() {
        assert!(beam_add_allows(95));
        assert!(!beam_add_allows(96));
    }
}
