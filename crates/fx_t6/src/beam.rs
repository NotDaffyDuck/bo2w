//! BO2's beam list and beam mesh (FX_Beam_*), from facts/fx-beam-laser-t6.md
//! section 2. Item numbers below are that section's.

use crate::runtime::vec3_normalize;

/// Beams past this many in one frame are dropped (item 3). MW2 holds 48.
pub const RAY_QUEUE_LIMIT: usize = 0x60;

pub const RAY_PIECE_LIMIT: u8 = 0xff;

/// Plain tracers multiply their length by 8 before the segment divide (item 4).
pub const FX_TRACER_SEGMENT_LENGTH_SCALE: f32 = 8.0;

/// The flat delta is "no beam" under this projected span squared (item 14).
pub const RAY_LEVEL_STEP_SHORTEST_SQ: f32 = 2e-6;

// BO2 unknown: the flat-delta near plane and perspective constant (item 14);
// MW2's z-near 1 and K = 2047/2048 are kept.
pub const ENDLESS_DEPTH_SQUEEZE: f32 = 2047.0 / 2048.0;
pub const BIRTH_FRONT_CLIP_DEPTH: f32 = 1.0;

// BO2 unknown: the tracer path's near cut and first-person width cap are not
// in the facts; MW2's values are kept.
pub const BULLET_STREAK_SHORTEST: f32 = 5.0;
pub const OWN_BULLET_STREAK_WIDEST: f32 = 4.0;

/// A draw-surf override of 0 is drawn with this one (item 15).
pub const FX_BEAM_DRAW_SURF_OVERRIDE_DEFAULT: u8 = 0x18;

/// The colour keys of a 5-colour beam change at these points along it (item 10).
pub const FX_BEAM_COLOR_BREAKS: [f32; 3] = [0.1, 0.5, 0.9];

/// The diagonal step of the wiggle compass, kept at the stored two-place value.
const JITTER_DIAGONAL: f32 = 0.71;

/// Wiggle offsets by segment index mod 8 (item 8): a compass walked clockwise from straight up.
pub const RAY_JITTER: [[f32; 2]; 8] = {
    let d = JITTER_DIAGONAL;
    [[0.0, 1.0], [d, d], [1.0, 0.0], [d, -d], [0.0, -1.0], [-d, -d], [-1.0, 0.0], [-d, d]]
};

/// The beam's type byte and the union fields each type carries (items 1-2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxBeamKind {
    Tracer(FxTracerBeam),
    SmokeTracer(FxTracerBeam),
    Laser {
        finish_reach: f32,
    },
    /// The flame's second quad mesh (item 15) is not built here.
    Flame {
        finish_reach: f32,
    },
    DemoLine,
    Arc {
        finish_reach: f32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxTracerBeam {
    pub sway_reach: f32,
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
    pub const fn tint_key_total(self) -> usize {
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
/// `kind.tint_key_total()` are read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectBeamStitch {
    pub kind: FxBeamKind,
    pub near_layer_draw: bool,
    pub draw_surf_override: u8,
    pub begin: [f32; 3],
    pub end: [f32; 3],
    pub colors: [[u8; 4]; 5],
    pub radius: f32,
    /// Set by the caller (item 1): `tracer_segment_count` for tracers, 1 for lasers.
    pub piece_tally: u8,
}

impl EffectBeamStitch {
    /// Only lasers and arcs widen along the beam (item 7).
    // TODO(fact): item 7 also names the flame's union end radius; whether a
    // flame widens is not settled, so it keeps one width here.
    #[must_use]
    pub const fn finish_reach(&self) -> f32 {
        match self.kind {
            FxBeamKind::Laser { finish_reach } | FxBeamKind::Arc { finish_reach } => finish_reach,
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
pub struct RayCorner {
    pub spot: [f32; 3],
    pub sheet_uv: [f32; 2],
    /// Already packed for the vertex: RGB divided by 4 except on smoke tracers.
    pub color: [u8; 4],
}

/// Vertices of a beam with `ribbon_piece_total` steps: one cap at each end plus a
/// left/right pair at every step boundary.
#[inline]
#[must_use]
pub const fn beam_vert_count(ribbon_piece_total: usize) -> usize {
    (ribbon_piece_total + 2) * 2
}

/// Three indices per triangle: one triangle per cap, two per segment.
#[inline]
#[must_use]
pub const fn beam_index_count(ribbon_piece_total: usize) -> usize {
    (ribbon_piece_total * 2 + 2) * 3
}

#[inline]
#[must_use]
pub fn beam_add_allows(live: usize) -> bool {
    live < RAY_QUEUE_LIMIT
}

/// The tracer segment count (item 4): length over the screw distance,
/// truncated, plain tracers at 8x length; under 2 is 1, 255 and up is 255.
#[must_use]
pub fn tracer_segment_count(smoke: bool, length: f32, spiral_pitch_distance: f32) -> u8 {
    // BO2 unknown: a zero screw distance; MW2's one segment is kept.
    let degenerate = spiral_pitch_distance < 1e-5;
    let scale = if smoke { 1.0 } else { FX_TRACER_SEGMENT_LENGTH_SCALE };
    let raw = if degenerate { 1 } else { (length * scale / spiral_pitch_distance) as i32 };
    raw.clamp(1, i32::from(RAY_PIECE_LIMIT)) as u8
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
    if kind.tint_key_total() == 2 {
        return blend_key(colors[0], colors[1], t);
    }
    // The key pair is picked by how many break points `t` has passed.
    let [b0, b1, b2] = FX_BEAM_COLOR_BREAKS;
    let stops = [0.0, b0, b1, b2, 1.0];
    let pair = FX_BEAM_COLOR_BREAKS.iter().filter(|&&b| t >= b).count();
    let span = stops[pair + 1] - stops[pair];
    blend_key(colors[pair], colors[pair + 1], (t - stops[pair]) / span)
}

/// Linear on bytes, truncated, clamped to 0..255 (item 11).
fn blend_key(a: [u8; 4], b: [u8; 4], f: f32) -> [u8; 4] {
    core::array::from_fn(|ch| {
        let (lo, hi) = (f32::from(a[ch]), f32::from(b[ch]));
        ((lo + (hi - lo) * f) as i32).clamp(0, 255) as u8
    })
}

/// The vertex colour: RGB shifted right 2 on every type but smoke tracer;
/// alpha is never divided (item 11).
#[inline]
#[must_use]
pub fn beam_vertex_color(kind: FxBeamKind, key: [u8; 4]) -> [u8; 4] {
    let shift = if matches!(kind, FxBeamKind::SmokeTracer(_)) { 0 } else { 2 };
    let [r, g, b, a] = key;
    [r >> shift, g >> shift, b >> shift, a]
}

/// The two body UVs of the vertex pair at `k` (item 12).
#[must_use]
pub fn beam_body_uvs(kind: FxBeamKind, k: f32) -> [[f32; 2]; 2] {
    // The u coordinate for the types that run along the beam.
    let along = |u: f32| [[u, 0.0], [u, 1.0]];
    match kind {
        FxBeamKind::Laser { .. } | FxBeamKind::DemoLine => [[1.0, 0.0], [0.0, 1.0]],
        // BO2 unknown: the tracer's u start value; u runs from 0 over the repeats.
        FxBeamKind::Tracer(tr) => along(k * tr.tex_repeats),
        // BO2 unknown: the smoke tracer's tiled run, and the flame and arc
        // mappings, were not read exactly; MW2's run is kept.
        FxBeamKind::SmokeTracer(_) | FxBeamKind::Flame { .. } | FxBeamKind::Arc { .. } => {
            along(k)
        }
    }
}

// BO2 unknown: the cap-vertex UVs per type (item 12); MW2's are kept.
const BEAM_BEGIN_CAP_UV: [f32; 2] = [0.0, 0.5];
const BEAM_END_CAP_UV: [f32; 2] = [1.0, 0.5];

/// Column-major `m * v`: the columns of `m` weighted by the entries of `val`.
#[inline]
#[must_use]
pub fn mat4_mul_vec4(m: &[f32; 16], val: [f32; 4]) -> [f32; 4] {
    m.chunks_exact(4)
        .zip(val)
        .fold([0.0; 4], |acc, (column, weight)| {
            [
                acc[0] + column[0] * weight,
                acc[1] + column[1] * weight,
                acc[2] + column[2] * weight,
                acc[3] + column[3] * weight,
            ]
        })
}

#[must_use]
pub fn infinite_perspective_matrix(horizontal_cone_slope: f32, vertical_cone_slope: f32, front_clip_depth: f32) -> [f32; 16] {
    // Column-major: a scaled diagonal, w taken from z, and the near-plane
    // shift in the fourth column.
    core::array::from_fn(|slot| match slot {
        0 => ENDLESS_DEPTH_SQUEEZE / horizontal_cone_slope,
        5 => ENDLESS_DEPTH_SQUEEZE / vertical_cone_slope,
        10 => ENDLESS_DEPTH_SQUEEZE,
        11 => 1.0,
        14 => -front_clip_depth * ENDLESS_DEPTH_SQUEEZE,
        _ => 0.0,
    })
}

/// World to view: the three basis rows become the first three columns
/// (transposed), and the translation is the origin projected on each.
#[must_use]
pub fn matrix_for_viewer(origin: [f32; 3], turn: [[f32; 3]; 3]) -> [f32; 16] {
    let rows = [V3(turn[1]) * -1.0, V3(turn[2]), V3(turn[0])];
    let origin = V3(origin);
    let mut m = [0.0f32; 16];
    for (j, column) in m.chunks_exact_mut(4).enumerate() {
        let entry = |r: V3| if j < 3 { r.0[j] } else { -origin.dot(r) };
        let w = if j == 3 { 1.0 } else { 0.0 };
        column.copy_from_slice(&[entry(rows[0]), entry(rows[1]), entry(rows[2]), w]);
    }
    m
}

#[must_use]
pub fn create_clip_matrix(
    spot: [f32; 3],
    turn: [[f32; 3]; 3],
    horizontal_cone_slope: f32,
    vertical_cone_slope: f32,
) -> [f32; 16] {
    let view = matrix_for_viewer(spot, turn);
    let projection = infinite_perspective_matrix(horizontal_cone_slope, vertical_cone_slope, BIRTH_FRONT_CLIP_DEPTH);
    // The product projection * view, one view column at a time.
    let mut clip = [0.0f32; 16];
    for (dst, column) in clip.chunks_exact_mut(4).zip(view.chunks_exact(4)) {
        dst.copy_from_slice(&mat4_mul_vec4(&projection, [column[0], column[1], column[2], column[3]]));
    }
    clip
}

/// Keeps the part of the homogeneous pair on the side of `wall` where the
/// plane equation is >= 0, moving whichever end is outside onto the plane;
/// false when both ends are outside.
pub fn beam_clip_against_plane(begin: &mut [f32; 4], to_pt: &mut [f32; 4], wall: [f32; 4]) -> bool {
    let signed = |p: &[f32; 4]| p.iter().zip(wall).map(|(c, w)| c * w).sum::<f32>();
    let (d_begin, d_end) = (signed(begin), signed(to_pt));
    let (from, to) = (*to_pt, *begin);
    // Where the pair crosses the plane, measured from the end point.
    let crossing = || -> [f32; 4] {
        let inv_gap = 1.0 / (d_end - d_begin);
        core::array::from_fn(|i| (to[i] - from[i]) * inv_gap * d_end + from[i])
    };
    match (d_begin >= 0.0, d_end >= 0.0) {
        (false, false) => false,
        (true, true) => true,
        (false, true) => {
            *begin = crossing();
            true
        }
        (true, false) => {
            *to_pt = crossing();
            true
        }
    }
}

/// The screen-flat direction of the beam, clipped to the near and far
/// planes; None under the minimum span (FX_GenerateBeam_GetFlatDelta).
#[must_use]
pub fn generate_beam_get_flat_delta(
    clip_m: &[f32; 16],
    clip_inv: &[f32; 16],
    from_pt: [f32; 3],
    to_pt: [f32; 3],
) -> Option<[f32; 3]> {
    let to_clip = |p: [f32; 3]| mat4_mul_vec4(clip_m, [p[0], p[1], p[2], 1.0]);
    let (mut b, mut e) = (to_clip(from_pt), to_clip(to_pt));
    for plane in [[0.0, 0.0, 1.0, 0.0], [0.0, 0.0, -1.0, 1.0]] {
        if !beam_clip_against_plane(&mut b, &mut e, plane) {
            return None;
        }
    }
    if [b[3], e[3]].contains(&0.0) {
        return None;
    }
    // The screen-space step between the two ends, taken back to world space.
    let step = [e[0] / e[3] - b[0] / b[3], e[1] / e[3] - b[1] / b[3], 0.0, 0.0];
    let world = mat4_mul_vec4(clip_inv, step);
    match world.iter().map(|c| c * c).sum::<f32>() {
        short if short < RAY_LEVEL_STEP_SHORTEST_SQ => None,
        span_sq => Some((V3([world[0], world[1], world[2]]) * (1.0 / libm::sqrtf(span_sq))).0),
    }
}

/// The flat and sideways directions without a clip matrix: the beam
/// direction less its part along the view.
#[must_use]
pub fn beam_flat_basis(
    from_pt: [f32; 3],
    end: [f32; 3],
    eye_at: [f32; 3],
    camera_ahead: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let (from_pt, end) = (V3(from_pt), V3(end));
    let span = end - from_pt;
    if span.magnitude_squared() < 1e-10 {
        return None;
    }
    let dir = span * (1.0 / libm::sqrtf(span.magnitude_squared()));
    // The view direction: as given, else toward the beam's midpoint.
    let usable = |v: V3| (v.magnitude_squared() >= 1e-6).then_some(v);
    let toward_mid = (from_pt + end) * 0.5 - V3(eye_at);
    let fwd = V3(vec3_normalize(camera_ahead));
    let fwd = usable(fwd).or_else(|| usable(V3(vec3_normalize(toward_mid.0))))?;
    // Out of the view direction, then any axis that is not parallel to it.
    let in_screen = dir - fwd * dir.dot(fwd);
    let flat = [in_screen, fwd.cross(V3([0.0, 1.0, 0.0])), fwd.cross(V3([1.0, 0.0, 0.0]))]
        .into_iter()
        .find(|v| v.magnitude_squared() >= 1e-8)
        .unwrap_or(fwd.cross(V3([1.0, 0.0, 0.0])));
    let flat = V3(vec3_normalize(flat.0));
    let perp = [flat.cross(fwd), dir.cross(fwd)]
        .into_iter()
        .find(|v| v.magnitude_squared() >= 1e-8)?;
    Some((flat.0, vec3_normalize(perp.0)))
}

/// One beam's mesh (FX_Beam_GenerateVerts): a cap vertex, a pair a width
/// apart at each segment step, a cap vertex. Returns the vertex and index
/// counts, or None when the buffers are short or the beam has no flat delta.
pub fn beam_generate_verts(
    strip: &EffectBeamStitch,
    eye_at: [f32; 3],
    camera_ahead: [f32; 3],
    to_clip: Option<(&[f32; 16], &[f32; 16])>,
    vertex_points: &mut [RayCorner],
    tri_ids: &mut [u16],
) -> Option<(usize, usize)> {
    let steps = usize::from(strip.piece_tally.max(1));
    let (vert_total, index_total) = (beam_vert_count(steps), beam_index_count(steps));
    if vertex_points.len() < vert_total || tri_ids.len() < index_total {
        return None;
    }
    let basis = if let Some((clip_m, inv_m)) = to_clip {
        generate_beam_get_flat_delta(clip_m, inv_m, strip.begin, strip.end)
            .and_then(|f| {
                // Sideways: across both the view and the flat direction.
                let side = V3(camera_ahead).cross(V3(f));
                (side.magnitude_squared() != 0.0).then(|| (f, vec3_normalize(side.0)))
            })
    } else {
        beam_flat_basis(strip.begin, strip.end, eye_at, camera_ahead)
    };
    let (flat, perp) = basis.map(|(f, p)| (V3(f), V3(p)))?;
    let (begin, end) = (V3(strip.begin), V3(strip.end));
    let kind = strip.kind;
    let tint = |t: f32| beam_vertex_color(kind, beam_sample_color(kind, &strip.colors, t));
    let wiggle = kind.tracer().map_or(0.0, |t| t.sway_reach);
    let far_radius = strip.finish_reach();
    let cap = |at: V3, uv: [f32; 2], t: f32| RayCorner { spot: at.0, sheet_uv: uv, color: tint(t) };

    vertex_points[0] = cap(begin - flat * strip.radius, BEAM_BEGIN_CAP_UV, 0.0);
    let body = &mut vertex_points[1..vert_total - 1];
    for (step, pair) in body.chunks_exact_mut(2).enumerate() {
        let t = step as f32 / steps as f32;
        let [across, up] = RAY_JITTER[step % RAY_JITTER.len()];
        let centre = begin + (end - begin) * t + perp * (across * wiggle) + flat * (up * wiggle);
        let half_width = strip.radius + (far_radius - strip.radius) * t;
        let texcoord_set = beam_body_uvs(kind, t);
        let color = tint(t);
        let sides = [centre - perp * half_width, centre + perp * half_width];
        for ((vert, side), uv) in pair.iter_mut().zip(sides).zip(texcoord_set) {
            *vert = RayCorner { spot: side.0, sheet_uv: uv, color };
        }
    }
    vertex_points[vert_total - 1] = cap(end + flat * far_radius, BEAM_END_CAP_UV, 1.0);
    Some((vert_total, write_beam_indices(steps, tri_ids)))
}

/// Triangle list for the strip: a start triangle, a quad per step (two
/// triangles), and an end triangle. Returns how many indices it wrote.
/// The caller sizes `dst_ids` with `beam_index_count`.
fn write_beam_indices(steps: usize, dst_ids: &mut [u16]) -> usize {
    let tail = steps as u16 * 2;
    let quads = (0..steps as u16).flat_map(|s| {
        let left = s * 2 + 1;
        [left + 1, left + 3, left, left, left + 3, left + 2]
    });
    let list = [0, 2, 1].into_iter().chain(quads).chain([tail + 1, tail + 2, tail + 3]);
    dst_ids.iter_mut().zip(list).map(|(slot, index)| *slot = index).count()
}

/// A private 3-vector with the arithmetic the mesh code needs.
#[derive(Clone, Copy, Debug)]
struct V3([f32; 3]);

impl core::ops::Add for V3 {
    type Output = V3;
    fn add(self, rhs: V3) -> V3 {
        V3(core::array::from_fn(|i| self.0[i] + rhs.0[i]))
    }
}

impl core::ops::Sub for V3 {
    type Output = V3;
    fn sub(self, rhs: V3) -> V3 {
        V3(core::array::from_fn(|i| self.0[i] - rhs.0[i]))
    }
}

impl core::ops::Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, factor: f32) -> V3 {
        V3(self.0.map(|c| c * factor))
    }
}

impl V3 {
    fn dot(self, other: V3) -> f32 {
        self.0.iter().zip(other.0).map(|(a, b)| a * b).sum()
    }

    fn magnitude_squared(self) -> f32 {
        self.dot(self)
    }

    fn cross(self, other: V3) -> V3 {
        let (a, b) = (self.0, other.0);
        V3([
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE5: [[u8; 4]; 5] = [[255; 4]; 5];

    /// A 100-unit white laser along +x, one segment, 0.35 wide at the start.
    fn laser(finish_reach: f32) -> EffectBeamStitch {
        let mut end = [0.0; 3];
        end[0] = 100.0;
        EffectBeamStitch {
            kind: FxBeamKind::Laser { finish_reach },
            near_layer_draw: false,
            draw_surf_override: 0,
            begin: [0.0; 3],
            end,
            colors: WHITE5,
            radius: 0.35,
            piece_tally: 1,
        }
    }

    fn blank_verts<const N: usize>() -> [RayCorner; N] {
        [RayCorner { spot: [0.0; 3], sheet_uv: [0.0; 2], color: [0; 4] }; N]
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
        assert_eq!(laser(2.0).finish_reach(), 2.0);
        let mut b = laser(2.0);
        b.kind = FxBeamKind::Flame { finish_reach: 2.0 };
        assert_eq!(b.finish_reach(), 0.35);
        b.kind = FxBeamKind::Arc { finish_reach: 3.0 };
        assert_eq!(b.finish_reach(), 3.0);
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
        let mut vertex_points = blank_verts::<6>();
        let mut idx = [0u16; 12];
        let looking_north = ([50.0, -100.0, 0.0], [0.0, 1.0, 0.0]);
        let got = beam_generate_verts(&laser(1.0), looking_north.0, looking_north.1, None, &mut vertex_points, &mut idx);
        assert_eq!(got, Some((6, 12)));
        assert_eq!((vertex_points[1].sheet_uv, vertex_points[2].sheet_uv), ([1.0, 0.0], [0.0, 1.0]));
        let width = |a: &RayCorner, b: &RayCorner| libm::sqrtf((V3(a.spot) - V3(b.spot)).magnitude_squared());
        assert!((width(&vertex_points[1], &vertex_points[2]) - 0.7).abs() < 1e-5);
        assert!((width(&vertex_points[3], &vertex_points[4]) - 2.0).abs() < 1e-5);
        assert_eq!(vertex_points[1].color, [63, 63, 63, 255]);
        assert_eq!(&idx[..6], &[0, 2, 1, 2, 4, 1]);
        assert_eq!(&idx[9..], &[3, 4, 5]);
    }

    #[test]
    fn short_buffers_build_nothing() {
        let b = laser(1.0);
        let mut vertex_points = blank_verts::<5>();
        let mut idx = [0u16; 12];
        assert!(
            beam_generate_verts(&b, [0.0; 3], [0.0, 1.0, 0.0], None, &mut vertex_points, &mut idx)
                .is_none()
        );
    }

    #[test]
    fn beam_cap_is_96() {
        assert!(beam_add_allows(95));
        assert!(!beam_add_allows(96));
    }

    #[test]
    fn clipping_moves_only_the_outside_end_onto_the_plane() {
        let plane = [1.0, 0.0, 0.0, 0.0];
        let (mut begin, mut end) = ([-2.0, 5.0, 0.0, 1.0], [2.0, 5.0, 0.0, 1.0]);
        assert!(beam_clip_against_plane(&mut begin, &mut end, plane));
        assert_eq!((begin, end), ([0.0, 5.0, 0.0, 1.0], [2.0, 5.0, 0.0, 1.0]));
        let (mut a, mut b) = ([-2.0, 0.0, 0.0, 1.0], [-1.0, 0.0, 0.0, 1.0]);
        assert!(!beam_clip_against_plane(&mut a, &mut b, plane));
        assert_eq!((a[0], b[0]), (-2.0, -1.0));
    }

    #[test]
    fn viewer_matrix_puts_the_eye_at_the_origin() {
        let axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let m = matrix_for_viewer([3.0, 4.0, 5.0], axis);
        let eye = mat4_mul_vec4(&m, [3.0, 4.0, 5.0, 1.0]);
        assert_eq!(eye, [0.0, 0.0, 0.0, 1.0]);
        // One unit ahead of the eye lands on the view's depth axis.
        let ahead = mat4_mul_vec4(&m, [4.0, 4.0, 5.0, 1.0]);
        assert_eq!(ahead, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn flat_basis_is_perpendicular_to_the_view() {
        let (flat, perp) =
            beam_flat_basis([0.0, 0.0, 0.0], [10.0, 0.0, 5.0], [0.0, -50.0, 0.0], [0.0, 1.0, 0.0])
                .expect("a beam across the view");
        assert!(V3(flat).dot(V3([0.0, 1.0, 0.0])).abs() < 1e-6);
        assert!(V3(flat).dot(V3(perp)).abs() < 1e-6);
        assert!(beam_flat_basis([1.0; 3], [1.0; 3], [0.0; 3], [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn index_list_walks_the_strip() {
        // Filled with a marker so an unwritten slot would show.
        let mut out = [u16::MAX; 18];
        assert_eq!(write_beam_indices(2, &mut out), beam_index_count(2));
        assert_eq!(
            out,
            [0, 2, 1, 2, 4, 1, 1, 4, 3, 4, 6, 3, 3, 6, 5, 5, 6, 7]
        );
    }
}
