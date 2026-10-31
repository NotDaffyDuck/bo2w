//! BO2's `entityShared_t` and what `SV_LinkEntity`, `G_SetOrigin`,
//! `G_SetAngle` and `G_TempEntity` do to it: the packed `solid` box, the
//! angle snap, and the world bounds the clip map links.

use crate::entity_state::EntitySnapshot;
use crate::entity_type::event_entity_type;
use crate::flight::{Trajectory, path_kind};

/// BO2's `entityShared_t` (`gentity_t::r`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityShared {
    pub linked: u8,
    pub bmodel: u8,
    pub sv_flags: u8,
    pub inuse: u8,
    pub broadcast_time: i32,
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    pub content_mask: i32,
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    pub current_origin: [f32; 3],
    pub live_angles: [f32; 3],
    pub owner_num: u16,
    pub owner_info_index: u16,
    pub event_time: i32,
}

/// Contents that `SV_LinkEntity` packs as a standing box: width, depth
/// below the origin, height above it.
pub const SOLID_BOX_CONTENTS: i32 = 0x0200_8001;
/// Contents that `SV_LinkEntity` packs as a centred box (marked by bit 16).
pub const SOLID_CENTRED_CONTENTS: i32 = 0x0080_0000;
/// The `solid` of a brush model.
pub const SOLID_BMODEL: u32 = 0x00ff_ffff;

/// `svFlags` bit: the clip map links the actor locational box.
pub const SVF_ACTOR_LOCATIONAL: u8 = 2;
/// `svFlags` bit: the clip map links the model's animated bounds.
pub const SVF_DOBJ_BOUNDS: u8 = 4;

/// How `SV_LinkEntity` sizes the world bounds of a non-brush entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkShape {
    /// A sentient that is not a big dog and not a vehicle: its own box.
    Box,
    /// Everything else: a cube as wide as the box's radius, so it holds
    /// the box at any angle.
    Radius,
}

/// A float turned to int the way BO2's `(int)` cast does (toward zero).
fn trunc_i32(v: f32) -> i32 {
    v as i32
}

fn clamp_byte(v: i32) -> u32 {
    v.clamp(1, 0xff) as u32
}

/// `SV_LinkEntity`'s packed `solid`.
#[must_use]
pub fn packed_solid(bmodel: bool, content_mask: i32, box_min: [f32; 3], box_max: [f32; 3]) -> u32 {
    if bmodel {
        return SOLID_BMODEL;
    }
    if content_mask & SOLID_BOX_CONTENTS != 0 {
        let x = clamp_byte(trunc_i32(box_max[0]));
        let zd = clamp_byte(trunc_i32(1.0 - box_min[2]));
        let zu = clamp_byte(trunc_i32(box_max[2] + 32.0));
        let solid = (zu << 16) | (zd << 8) | x;
        if solid == SOLID_BMODEL { 1 } else { solid }
    } else if content_mask & SOLID_CENTRED_CONTENTS != 0 {
        let x = clamp_byte(trunc_i32(box_max[0]));
        let z = clamp_byte(trunc_i32((box_max[2] + box_min[2]) * 0.5 + 128.0));
        (1 << 16) | (z << 8) | x
    } else {
        0
    }
}

/// `SV_LinkEntity` snaps each angle within 0.001 degrees of a whole
/// number onto it.
#[must_use]
pub fn snap_angles(degrees: [f32; 3]) -> [f32; 3] {
    degrees.map(|deg| {
        let nearest = libm::rintf(deg + 9.313_226e-10) as i32 as f32;
        let off = nearest - deg;
        if off * off < 1.000_000_1e-6 { nearest } else { deg }
    })
}

/// `RadiusFromBounds`.
#[must_use]
pub fn radius_from_bounds(box_min: [f32; 3], box_max: [f32; 3]) -> f32 {
    let x = libm::fabsf(box_min[0]).max(libm::fabsf(box_max[0]));
    let y = libm::fabsf(box_min[1]).max(libm::fabsf(box_max[1]));
    let z = libm::fabsf(box_max[2]).max(libm::fabsf(box_min[2]));
    libm::sqrtf(x * x + y * y + z * z)
}

/// `AnglesToAxis`: forward, left, up.
#[must_use]
pub fn axis_of_angles(turn: [f32; 3]) -> [[f32; 3]; 3] {
    let [pitch, yaw, roll] = turn;
    let (cos_y, sin_y) = crate::angles::cos_sin(yaw);
    let (cos_p, sin_p) = crate::angles::cos_sin(pitch);
    let (cos_r, sin_r) = crate::angles::cos_sin(roll);
    let forward = [cos_p * cos_y, cos_p * sin_y, -sin_p];
    let left = [sin_r * sin_p * cos_y - cos_r * sin_y, sin_r * sin_p * sin_y + cos_r * cos_y, sin_r * cos_p];
    let up = [cos_r * sin_p * cos_y + sin_r * sin_y, cos_r * sin_p * sin_y - sin_r * cos_y, cos_r * cos_p];
    [forward, left, up]
}

/// `phys_calc_world_aabb`: the world box around a box with this centre
/// and half size, turned by `axis` and moved to `origin`.
#[must_use]
pub fn world_aabb(
    centre: [f32; 3],
    half: [f32; 3],
    turn_axis: [[f32; 3]; 3],
    at: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for k in 0..3 {
        let c = at[k] + turn_axis[0][k] * centre[0] + turn_axis[1][k] * centre[1] + turn_axis[2][k] * centre[2];
        let e = half[0] * libm::fabsf(turn_axis[0][k])
            + half[1] * libm::fabsf(turn_axis[1][k])
            + half[2] * libm::fabsf(turn_axis[2][k]);
        lo[k] = c - e;
        hi[k] = c + e;
    }
    (lo, hi)
}

impl EntityShared {
    /// The bounds part of `SV_LinkEntity`: snap the angles, size
    /// `absmin`/`absmax` (a brush model turns its box by its angles), grow
    /// them by 1 on every side, and mark the entity linked. Returns the
    /// packed `solid` for `entityState_s::solid`.
    ///
    /// The clip map link itself, and the model or actor bounds that
    /// [`SVF_DOBJ_BOUNDS`] / [`SVF_ACTOR_LOCATIONAL`] hand it instead, are
    /// the clip map's.
    pub fn link(&mut self, shape: LinkShape) -> u32 {
        let solid = packed_solid(self.bmodel != 0, self.content_mask, self.box_min, self.box_max);
        self.live_angles = snap_angles(self.live_angles);
        let o = self.current_origin;
        let (mut lo, mut hi) = if self.bmodel != 0 {
            let mut centre = [0.0; 3];
            let mut half = [0.0; 3];
            for k in 0..3 {
                half[k] = (self.box_max[k] - self.box_min[k]) * 0.5;
                centre[k] = (self.box_min[k] + self.box_max[k]) * 0.5;
            }
            world_aabb(centre, half, axis_of_angles(self.live_angles), o)
        } else {
            match shape {
                LinkShape::Box => (
                    [o[0] + self.box_min[0], o[1] + self.box_min[1], o[2] + self.box_min[2]],
                    [o[0] + self.box_max[0], o[1] + self.box_max[1], o[2] + self.box_max[2]],
                ),
                LinkShape::Radius => {
                    let r = radius_from_bounds(self.box_min, self.box_max);
                    ([o[0] - r, o[1] - r, o[2] - r], [o[0] + r, o[1] + r, o[2] + r])
                }
            }
        };
        for k in 0..3 {
            lo[k] -= 1.0;
            hi[k] += 1.0;
        }
        self.absmin = lo;
        self.absmax = hi;
        self.linked = 1;
        solid
    }
}

fn stationary(at: [f32; 3]) -> Trajectory {
    Trajectory {
        tr_type: path_kind::AT_REST,
        began_at: 0,
        span_ms: 0,
        base_point: at,
        rate: [0.0; 3],
    }
}

/// `G_SetOrigin`: stand the entity still at `origin`.
pub fn set_origin(es: &mut EntitySnapshot, r: &mut EntityShared, spot: [f32; 3]) {
    es.lerp.pos = stationary(spot);
    r.current_origin = spot;
}

/// `G_SetAngle`: hold the entity still at `angles`.
pub fn set_angle(es: &mut EntitySnapshot, r: &mut EntityShared, heading: [f32; 3]) {
    es.lerp.apos = stationary(heading);
    r.live_angles = heading;
}

/// What `G_TempEntity` sets on the entity it spawns for event `ev` at
/// `origin` (cut to whole units), before it links it. The caller sets
/// `classname` to `tempEntity` and `freeAfterEvent`.
pub fn init_temp_entity(
    es: &mut EntitySnapshot,
    r: &mut EntityShared,
    spot: [f32; 3],
    ev: u8,
    match_clock_ms: i32,
) {
    es.e_type = event_entity_type(ev);
    r.event_time = match_clock_ms;
    let whole = [
        trunc_i32(spot[0]) as f32,
        trunc_i32(spot[1]) as f32,
        trunc_i32(spot[2]) as f32,
    ];
    set_origin(es, r, whole);
}

/// Zombies' caps on temp entities in one server frame (`G_TempEntity`):
/// bullet hits, dynamic entity hits, and two events
/// `sv_restrictedTempEnts` blocks. Over a cap, BO2 hands back a dud
/// entity that nobody sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZombieTempEntCaps {
    pub bullet_hits: u32,
    pub debris_pieces: u32,
}

impl ZombieTempEntCaps {
    pub const MAX_BULLET_HITS: u32 = 5;
    pub const MAX_DYN_ENTS: u32 = 4;

    /// Count `ev` and say whether it may spawn.
    pub fn allow(&mut self, ev: u8, restricted_temp_ents: bool) -> bool {
        match ev {
            crate::event::ROUND_HIT => {
                if self.bullet_hits >= Self::MAX_BULLET_HITS {
                    return false;
                }
                self.bullet_hits += 1;
                true
            }
            crate::event::ROUND_HIT_BODY_SMALL | crate::event::ROUND_HIT_BODY_LARGE => {
                !restricted_temp_ents
            }
            crate::event::CREATE_DYNENT => {
                if self.debris_pieces >= Self::MAX_DYN_ENTS {
                    return false;
                }
                self.debris_pieces += 1;
                true
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_packs_like_bo2() {
        let box_min = [-15.0, -15.0, 0.0];
        let box_max = [15.0, 15.0, 72.0];
        assert_eq!(packed_solid(false, 0x0200_0000, box_min, box_max), (104 << 16) | (1 << 8) | 15);
        assert_eq!(packed_solid(false, SOLID_CENTRED_CONTENTS, box_min, box_max), (1 << 16) | (164 << 8) | 15);
        assert_eq!(packed_solid(false, 0x10, box_min, box_max), 0);
        assert_eq!(packed_solid(true, 0, box_min, box_max), SOLID_BMODEL);
        let big = [300.0, 300.0, 300.0];
        assert_eq!(packed_solid(false, 1, [-300.0; 3], big), 1);
    }

    #[test]
    fn angles_snap_only_when_close() {
        assert_eq!(snap_angles([89.9995, 10.2, -45.0004]), [90.0, 10.2, -45.0]);
    }

    #[test]
    fn link_grows_bounds_by_one() {
        let mut r = EntityShared {
            current_origin: [100.0, 0.0, 0.0],
            box_max: [15.0, 15.0, 72.0],
            ..Default::default()
        };
        r.box_min = [-15.0, -15.0, 0.0];
        r.link(LinkShape::Box);
        assert_eq!(r.absmin, [84.0, -16.0, -1.0]);
        assert_eq!(r.absmax, [116.0, 16.0, 73.0]);
        assert_eq!(r.linked, 1);
        r.box_min = [-3.0, -4.0, 0.0];
        r.box_max = [3.0, 4.0, 12.0];
        r.link(LinkShape::Radius);
        assert_eq!(r.absmax, [114.0, 14.0, 14.0]);
    }

    #[test]
    fn brush_bounds_turn_with_angles() {
        let mut r = EntityShared {
            bmodel: 1,
            box_min: [0.0, -1.0, 0.0],
            box_max: [10.0, 1.0, 2.0],
            live_angles: [0.0, 90.0, 0.0],
            ..Default::default()
        };
        r.link(LinkShape::Radius);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(r.absmin, [-2.0, -1.0, -1.0]), "{:?}", r.absmin);
        assert!(near(r.absmax, [2.0, 11.0, 3.0]), "{:?}", r.absmax);
    }

    #[test]
    fn temp_entity_sits_on_whole_units() {
        let mut es = EntitySnapshot::default();
        let mut r = EntityShared::default();
        init_temp_entity(&mut es, &mut r, [10.7, -3.9, 5.0], crate::event::OBITUARY, 500);
        assert_eq!(es.lerp.pos.base_point, [10.0, -3.0, 5.0]);
        assert_eq!(r.current_origin, [10.0, -3.0, 5.0]);
        assert_eq!(es.e_type, 21 + crate::event::OBITUARY as i16);
        assert_eq!(r.event_time, 500);
    }

    #[test]
    fn zombie_caps() {
        let mut caps = ZombieTempEntCaps::default();
        let n = (0..9).filter(|_| caps.allow(crate::event::ROUND_HIT, false)).count();
        assert_eq!(n, 5);
        assert!(!caps.allow(crate::event::ROUND_HIT_BODY_SMALL, true));
        assert!(caps.allow(crate::event::ROUND_HIT_BODY_LARGE, false));
    }
}
