//! BO2 Target Finder (ATTACHMENT_RANGEFINDER) sight screen: the UI3D window 5 picture the sight's
//! glass shows at full ADS. The two fixed quad lists over the background atlas, the colour change
//! latch, the range in feet, and the enemy diamonds placed through the viewmodel's tag_hud plane.
//! Who gets a diamond (enemy, visible by the overhead-name test, no perk 35) is the caller's call;
//! this module only places it.
//! Facts: Streaks notes facts/target-finder.md.

extern crate alloc;

/// UI3D window the screen draws into (R_UI3D_SetupTextureWindow 5, full 0..1 rect).
pub const UI3D_WINDOW: u8 = 5;
/// Texture space of the window.
pub const TEXTURE_SIZE: f32 = 512.0;
/// Top strip cleared black every frame (0, 0, 512, 85).
pub const CLEAR_STRIP: [f32; 4] = [0.0, 0.0, 512.0, 85.0];

/// Background art: mtl_t6_attach_optic_rangefinder_ui3d_background, a 256 x 128 px atlas.
pub const BACKGROUND_MATERIAL: &str = "mtl_t6_attach_optic_rangefinder_ui3d_background";
pub const ATLAS_W: f32 = 256.0;
pub const ATLAS_H: f32 = 128.0;

/// Draw only at fWeaponPosFrac >= 0.99, and not while EMP-jammed.
pub const DRAW_MIN_ADS_FRAC: f32 = 0.99;
/// The trace runs 8192 units along the view.
pub const TRACE_RANGE: f32 = 8192.0;
/// Range shown = int(dist * 0.083333) (dist / 12, in feet).
pub const UNITS_TO_FEET: f32 = 0.083_333;

/// A colour at full alpha.
const fn opaque(r: f32, g: f32, b: f32) -> [f32; 4] {
    [r, g, b, 1.0]
}

/// Dvar defaults.
pub const DEFAULT_COLOR: [f32; 4] = opaque(0.0, 1.0, 0.0);
pub const ACTIVE_COLOR: [f32; 4] = opaque(1.0, 0.0, 0.0);
pub const COLOR_CHANGE_DELAY_MS: i32 = 0;
pub const MIN_ENEMY_SPOTTING_DIST: f32 = -1.0;
pub const DIAMOND_INDICATOR: bool = true;
pub const DIAMOND_SIZE: f32 = 50.0;

/// Perks that hide a player from it.
pub const PERK_IMMUNE_RANGEFINDER: u32 = 35;
pub const PERK_NONAME: u32 = 42;

/// Loop sound while the colour change holds (not while EMP-jammed).
pub const LOOP_SOUND: &str = "fly_range_finder_lp";

/// Diamond art in the atlas, as UVs.
pub const DIAMOND_UV: [f32; 4] = [0.414_062_5, 0.273_437_5, 0.675_781_25, 0.796_875];
/// Corner-bracket art (bracket mode), as UVs.
pub const BRACKET_UV: [f32; 4] = [0.125, 0.75, 0.25, 1.0];

/// One quad of a list: texture-space rect, atlas source rect in px, rotation in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Element {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub src_x: f32,
    pub src_y: f32,
    pub src_w: f32,
    pub src_h: f32,
    pub turn_matrix: f32,
}

const fn el(x: f32, y: f32, w: f32, h: f32, sx: f32, sy: f32, sw: f32, sh: f32, turn_matrix: f32) -> Element {
    Element { x, y, w, h, src_x: sx, src_y: sy, src_w: sw, src_h: sh, turn_matrix }
}

/// normalColorElms: centre, label, 4 corners.
pub const NORMAL_ELEMENTS: [Element; 6] = [
    el(230.0, 230.0, 53.0, 53.0, 0.0, 0.0, 53.0, 53.0, 0.0),
    el(8.0, 85.0, 70.0, 25.0, 0.0, 53.0, 70.0, 25.0, 0.0),
    el(8.0, 98.0, 35.0, 35.0, 106.0, 0.0, 35.0, 35.0, 0.0),
    el(472.0, 98.0, 35.0, 35.0, 106.0, 0.0, 35.0, 35.0, 270.0),
    el(8.0, 408.0, 35.0, 35.0, 106.0, 0.0, 35.0, 35.0, 90.0),
    el(472.0, 408.0, 35.0, 35.0, 106.0, 0.0, 35.0, 35.0, 180.0),
];

/// targetingColorElms: the red art for the same 6, plus 4 edge marks.
pub const TARGETING_ELEMENTS: [Element; 10] = [
    el(230.0, 230.0, 53.0, 53.0, 53.0, 0.0, 53.0, 53.0, 0.0),
    el(8.0, 85.0, 70.0, 25.0, 0.0, 78.0, 70.0, 25.0, 0.0),
    el(8.0, 98.0, 35.0, 35.0, 141.0, 0.0, 35.0, 35.0, 0.0),
    el(472.0, 98.0, 35.0, 35.0, 141.0, 0.0, 35.0, 35.0, 270.0),
    el(8.0, 408.0, 35.0, 35.0, 141.0, 0.0, 35.0, 35.0, 90.0),
    el(472.0, 408.0, 35.0, 35.0, 141.0, 0.0, 35.0, 35.0, 180.0),
    el(240.0, 85.0, 33.0, 33.0, 70.0, 53.0, 33.0, 33.0, 0.0),
    el(8.0, 240.0, 33.0, 33.0, 70.0, 53.0, 33.0, 33.0, 90.0),
    el(472.0, 240.0, 33.0, 33.0, 70.0, 53.0, 33.0, 33.0, 270.0),
    el(240.0, 425.0, 33.0, 33.0, 70.0, 53.0, 33.0, 33.0, 180.0),
];

/// The list for this frame: targeting once the colour change fired.
#[must_use]
pub fn elements(targeting: bool) -> &'static [Element] {
    if targeting { &TARGETING_ELEMENTS } else { &NORMAL_ELEMENTS }
}

/// One textured corner: texture-space position and atlas UV. Vertex colour is always white.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadVertex {
    pub xy: [f32; 2],
    pub tex_at: [f32; 2],
}

/// The 4 corners of an element in draw order (x,y) (x+w,y) (x+w,y+h) (x,y+h). A rotation turns
/// the sampled atlas rect about its own centre; the screen rect stays axis-aligned.
#[must_use]
pub fn element_quad(e: &Element) -> [QuadVertex; 4] {
    let (u0, v0, du1, dv1, du3, dv3) = if e.turn_matrix == 0.0 {
        (e.src_x, e.src_y, e.src_w, 0.0, 0.0, e.src_h)
    } else {
        let (s, c) = libm::sincosf(e.turn_matrix * 0.017_453_292);
        let hw = e.src_w * 0.5;
        let hh = e.src_h * 0.5;
        (
            e.src_x + hw - hw * c + hh * s,
            hh + e.src_y - hw * s - hh * c,
            e.src_w * c,
            e.src_w * s,
            -(e.src_h * s),
            e.src_h * c,
        )
    };
    let (u0, v0) = (u0 * (1.0 / ATLAS_W), v0 * (1.0 / ATLAS_H));
    let (u1, v1) = (u0 + du1 * (1.0 / ATLAS_W), v0 + dv1 * (1.0 / ATLAS_H));
    let (du3, dv3) = (du3 * (1.0 / ATLAS_W), dv3 * (1.0 / ATLAS_H));
    [
        QuadVertex { xy: [e.x, e.y], tex_at: [u0, v0] },
        QuadVertex { xy: [e.x + e.w, e.y], tex_at: [u1, v1] },
        QuadVertex { xy: [e.x + e.w, e.y + e.h], tex_at: [u1 + du3, v1 + dv3] },
        QuadVertex { xy: [e.x, e.y + e.h], tex_at: [u0 + du3, v0 + dv3] },
    ]
}

/// The attachment is "on" (trace, latch, sound) only at exactly full ADS.
#[must_use]
pub fn active(has_rangefinder: bool, ads_frac: f32) -> bool {
    has_rangefinder && ads_frac == 1.0
}

/// The sight screen draws its lists and diamonds.
#[must_use]
pub fn draws_screen(has_rangefinder: bool, ads_frac: f32, emp_jammed: bool) -> bool {
    has_rangefinder && !emp_jammed && ads_frac >= DRAW_MIN_ADS_FRAC
}

/// Range readout in feet from a trace distance in units.
#[must_use]
pub fn range_feet(dist: f32) -> i32 {
    (dist * UNITS_TO_FEET) as i32
}

/// What the view trace hit, as the spotting test sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceTarget {
    pub is_client: bool,
    pub enemy: bool,
    pub has_noname: bool,
}

/// The trace spots a target: a client, I am not spectating, an enemy (FFA: anyone else), and it
/// lacks perk 42. `min_dist` < 0 = no minimum.
#[must_use]
pub fn spotted(hit: Option<TraceTarget>, spectator: bool, dist: f32, min_dist: f32) -> bool {
    let Some(t) = hit else { return false };
    t.is_client && !spectator && t.enemy && !t.has_noname && (min_dist < 0.0 || dist >= min_dist)
}

/// First-spotted time latch; the colour change fires once spotted for longer than the delay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpotLatch {
    pub start_ms: Option<i32>,
}

impl SpotLatch {
    /// Feed one frame; returns whether the targeting list + loop sound are on.
    pub fn feed(&mut self, seen: bool, now: i32, hold_ms: i32) -> bool {
        if !seen {
            self.start_ms = None;
            return false;
        }
        let start = *self.start_ms.get_or_insert(now);
        now - start > hold_ms
    }

}

/// Stance height added to the target's origin for its diamond.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Stand,
    Crouch,
    Prone,
}

#[must_use]
pub fn stance_height(stance: Stance) -> f32 {
    match stance {
        Stance::Stand => 45.0,
        Stance::Crouch => 30.0,
        Stance::Prone => 11.0,
    }
}

/// eFlags bits read for the stance (4 crouch, 8 prone).
#[must_use]
pub fn stance_from_eflags(eflags: u32) -> Stance {
    if eflags & 8 != 0 {
        Stance::Prone
    } else if eflags & 4 != 0 {
        Stance::Crouch
    } else {
        Stance::Stand
    }
}

/// tag_hud bone's world frame: origin and axis rows (forward, left, up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TagHud {
    pub spot: [f32; 3],
    pub frame: [[f32; 3]; 3],
}

impl TagHud {
    fn local(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.spot[0], p[1] - self.spot[1], p[2] - self.spot[2]];
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        [dot(self.frame[0]), dot(self.frame[1]), dot(self.frame[2])]
    }
}

/// Where a world point lands on the screen texture: the eye-to-point ray, in tag_hud space, meets
/// the bone's x = 0 plane; y and z map to texture px (centre 256). None when the ray misses the
/// plane between the eye and the point.
#[must_use]
pub fn project(tag: &TagHud, eye: [f32; 3], spot: [f32; 3]) -> Option<(f32, f32)> {
    let e = tag.local(eye);
    let d = tag.local(spot);
    let den = e[0] - d[0];
    if libm::fabsf(den) <= 0.0001 {
        return None;
    }
    let t = e[0] / den;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let y = (d[1] - e[1]) * t + e[1];
    let z = (d[2] - e[2]) * t + e[2];
    Some(((1.0 - 2.0 * y) * 0.5 * TEXTURE_SIZE, (1.0 - 2.0 * z) * 0.5 * TEXTURE_SIZE))
}

/// The diamond's texture rect (x, y, w, h), centred on the projected point.
#[must_use]
pub fn diamond_rect(tag: &TagHud, eye: [f32; 3], target_origin: [f32; 3], stance: Stance, size: f32) -> Option<[f32; 4]> {
    let p = [target_origin[0], target_origin[1], target_origin[2] + stance_height(stance)];
    let (cx, cy) = project(tag, eye, p)?;
    let half = size * 0.5;
    Some([cx - half, cy - half, size, size])
}

/// One quad of a frame for the renderer: 4 corners in texture px with atlas UVs. `textured` false
/// = the black clear strip (no material).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameQuad {
    pub corners: [QuadVertex; 4],
    pub textured: bool,
}

fn rect_quad(r: [f32; 4], uv: [f32; 4]) -> FrameQuad {
    let [x, y, w, h] = r;
    FrameQuad {
        corners: [
            QuadVertex { xy: [x, y], tex_at: [uv[0], uv[1]] },
            QuadVertex { xy: [x + w, y], tex_at: [uv[2], uv[1]] },
            QuadVertex { xy: [x + w, y + h], tex_at: [uv[2], uv[3]] },
            QuadVertex { xy: [x, y + h], tex_at: [uv[0], uv[3]] },
        ],
        textured: true,
    }
}

/// Everything drawn into window 5 this frame, in draw order: the clear strip, the element list,
/// then one diamond per rect from [`diamond_rect`]. All textured quads use [`BACKGROUND_MATERIAL`]
/// with a white vertex colour.
#[must_use]
pub fn frame_quads(targeting: bool, diamonds: &[[f32; 4]]) -> alloc::vec::Vec<FrameQuad> {
    let mut out = alloc::vec::Vec::with_capacity(1 + 10 + diamonds.len());
    let mut strip = rect_quad(CLEAR_STRIP, [0.0; 4]);
    strip.textured = false;
    out.push(strip);
    out.extend(elements(targeting).iter().map(|e| FrameQuad { corners: element_quad(e), textured: true }));
    out.extend(diamonds.iter().map(|r| rect_quad(*r, DIAMOND_UV)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        libm::fabsf(a - b) < 1e-4
    }

    #[test]
    fn lists_have_bo2_counts_and_art() {
        assert_eq!(elements(false).len(), 6);
        assert_eq!(elements(true).len(), 10);
        assert_eq!(NORMAL_ELEMENTS[0].src_x, 0.0);
        assert_eq!(TARGETING_ELEMENTS[0].src_x, 53.0);
        assert_eq!(TARGETING_ELEMENTS[1].src_y, 78.0);
    }

    #[test]
    fn unrotated_quad_maps_atlas_px() {
        let q = element_quad(&NORMAL_ELEMENTS[1]);
        assert_eq!(q[0].xy, [8.0, 85.0]);
        assert_eq!(q[2].xy, [78.0, 110.0]);
        assert!(close(q[0].tex_at[0], 0.0) && close(q[0].tex_at[1], 53.0 / 128.0));
        assert!(close(q[2].tex_at[0], 70.0 / 256.0) && close(q[2].tex_at[1], 78.0 / 128.0));
    }

    #[test]
    fn rotated_180_flips_the_source() {
        let q = element_quad(&NORMAL_ELEMENTS[5]);
        // Corner 0 samples the far corner of the 106,0 35x35 rect.
        assert!(close(q[0].tex_at[0], 141.0 / 256.0) && close(q[0].tex_at[1], 35.0 / 128.0));
        assert!(close(q[2].tex_at[0], 106.0 / 256.0) && close(q[2].tex_at[1], 0.0));
    }

    #[test]
    fn rotated_90_turns_the_source() {
        let q = element_quad(&NORMAL_ELEMENTS[4]);
        assert!(close(q[0].tex_at[0], 141.0 / 256.0) && close(q[0].tex_at[1], 0.0));
        assert!(close(q[1].tex_at[0], 141.0 / 256.0) && close(q[1].tex_at[1], 35.0 / 128.0));
    }

    #[test]
    fn gates_and_range() {
        assert!(active(true, 1.0));
        assert!(!active(true, 0.995));
        assert!(draws_screen(true, 0.99, false));
        assert!(!draws_screen(true, 0.98, false));
        assert!(!draws_screen(true, 1.0, true));
        assert_eq!(range_feet(1200.0), 99);
        assert_eq!(range_feet(1212.0), 100);
    }

    #[test]
    fn spotting_and_latch() {
        let enemy = TraceTarget { is_client: true, enemy: true, has_noname: false };
        assert!(spotted(Some(enemy), false, 500.0, MIN_ENEMY_SPOTTING_DIST));
        assert!(!spotted(Some(TraceTarget { has_noname: true, ..enemy }), false, 500.0, -1.0));
        assert!(!spotted(Some(TraceTarget { enemy: false, ..enemy }), false, 500.0, -1.0));
        assert!(!spotted(Some(enemy), true, 500.0, -1.0));
        assert!(!spotted(None, false, 500.0, -1.0));
        let mut l = SpotLatch::default();
        assert!(!l.feed(true, 1000, 0));
        assert!(l.feed(true, 1001, 0));
        assert!(!l.feed(false, 1002, 0));
        assert!(!l.feed(true, 1003, 100));
        assert!(!l.feed(true, 1103, 100));
        assert!(l.feed(true, 1104, 100));
    }

    #[test]
    fn stance_heights() {
        assert_eq!(stance_height(stance_from_eflags(0)), 45.0);
        assert_eq!(stance_height(stance_from_eflags(4)), 30.0);
        assert_eq!(stance_height(stance_from_eflags(8 | 4)), 11.0);
    }

    #[test]
    fn straight_ahead_lands_mid_texture() {
        // tag_hud 2 units ahead of the eye, axes = world; target far down +x.
        let level = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let tag = TagHud { spot: [2.0, 0.0, 0.0], frame: level };
        let p = project(&tag, [0.0, 0.0, 0.0], [500.0, 0.0, 0.0]).unwrap();
        assert!(close(p.0, 256.0) && close(p.1, 256.0));
        let r = diamond_rect(&tag, [0.0, 0.0, -45.0], [500.0, 0.0, -45.0], Stance::Stand, DIAMOND_SIZE).unwrap();
        assert!(r[2] == 50.0 && r[0] < 256.0 && r[0] + 50.0 > 256.0);
        // A point behind the eye never crosses the plane.
        assert!(project(&tag, [0.0, 0.0, 0.0], [-500.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn frame_order_strip_list_diamonds() {
        let f = frame_quads(true, &[[10.0, 20.0, 50.0, 50.0]]);
        assert_eq!(f.len(), 1 + 10 + 1);
        assert!(!f[0].textured && f[0].corners[2].xy == [512.0, 85.0]);
        let d = f[11].corners;
        assert_eq!(d[0].xy, [10.0, 20.0]);
        assert_eq!(d[2].tex_at, [DIAMOND_UV[2], DIAMOND_UV[3]]);
        assert_eq!(frame_quads(false, &[]).len(), 7);
    }
}
