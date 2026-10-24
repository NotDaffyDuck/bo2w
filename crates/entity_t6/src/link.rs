//! BO2's `entityShared_t` and what `SV_LinkEntity`, `G_SetOrigin`,
//! `G_SetAngle` and `G_TempEntity` do to it: the packed `solid` box, the
//! angle snap, and the world bounds the clip map links.

use crate::entity_state::EntityState;
use crate::entity_type::event_entity_type;
use crate::trajectory::{Trajectory, tr_type};

/// BO2's `entityShared_t` (`gentity_t::r`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityShared {
    pub linked: u8,
    pub bmodel: u8,
    pub sv_flags: u8,
    pub inuse: u8,
    pub broadcast_time: i32,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub contents: i32,
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    pub current_origin: [f32; 3],
    pub current_angles: [f32; 3],
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
pub fn packed_solid(bmodel: bool, contents: i32, mins: [f32; 3], maxs: [f32; 3]) -> u32 {
    if bmodel {
        return SOLID_BMODEL;
    }
    if contents & SOLID_BOX_CONTENTS != 0 {
        let x = clamp_byte(trunc_i32(maxs[0]));
        let zd = clamp_byte(trunc_i32(1.0 - mins[2]));
        let zu = clamp_byte(trunc_i32(maxs[2] + 32.0));
        let solid = (zu << 16) | (zd << 8) | x;
        if solid == SOLID_BMODEL { 1 } else { solid }
    } else if contents & SOLID_CENTRED_CONTENTS != 0 {
        let x = clamp_byte(trunc_i32(maxs[0]));
        let z = clamp_byte(trunc_i32((maxs[2] + mins[2]) * 0.5 + 128.0));
        (1 << 16) | (z << 8) | x
    } else {
        0
    }
}

/// `SV_LinkEntity` snaps each angle within 0.001 degrees of a whole
/// number onto it.
#[must_use]
pub fn snap_angles(angles: [f32; 3]) -> [f32; 3] {
    let mut out = angles;
    for a in &mut out {
        let whole = libm::rintf(*a + 9.313_226e-10) as i32 as f32;
        let d = whole - *a;
        if d * d < 1.000_000_1e-6 {
            *a = whole;
        }
    }
    out
}

/// `RadiusFromBounds`.
#[must_use]
pub fn radius_from_bounds(mins: [f32; 3], maxs: [f32; 3]) -> f32 {
    let x = libm::fabsf(mins[0]).max(libm::fabsf(maxs[0]));
    let y = libm::fabsf(mins[1]).max(libm::fabsf(maxs[1]));
    let z = libm::fabsf(maxs[2]).max(libm::fabsf(mins[2]));
    libm::sqrtf(x * x + y * y + z * z)
}

/// `AnglesToAxis`: forward, left, up.
#[must_use]
pub fn angles_to_axis(angles: [f32; 3]) -> [[f32; 3]; 3] {
    let sc = |deg: f32| {
        let r = (deg * 0.017_453_292) as f64;
        (libm::sin(r) as f32, libm::cos(r) as f32)
    };
    let (sy, cy) = sc(angles[1]);
    let (sp, cp) = sc(angles[0]);
    let (sr, cr) = sc(angles[2]);
    [
        [cp * cy, cp * sy, -sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
}

/// `phys_calc_world_aabb`: the world box around a box with this centre
/// and half size, turned by `axis` and moved to `origin`.
#[must_use]
pub fn world_aabb(
    centre: [f32; 3],
    half: [f32; 3],
    axis: [[f32; 3]; 3],
    origin: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for k in 0..3 {
        let c = origin[k] + axis[0][k] * centre[0] + axis[1][k] * centre[1] + axis[2][k] * centre[2];
        let e = half[0] * libm::fabsf(axis[0][k])
            + half[1] * libm::fabsf(axis[1][k])
            + half[2] * libm::fabsf(axis[2][k]);
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
        let solid = packed_solid(self.bmodel != 0, self.contents, self.mins, self.maxs);
        self.current_angles = snap_angles(self.current_angles);
        let o = self.current_origin;
        let (mut lo, mut hi) = if self.bmodel != 0 {
            let mut centre = [0.0; 3];
            let mut half = [0.0; 3];
            for k in 0..3 {
                half[k] = (self.maxs[k] - self.mins[k]) * 0.5;
                centre[k] = (self.mins[k] + self.maxs[k]) * 0.5;
            }
            world_aabb(centre, half, angles_to_axis(self.current_angles), o)
        } else {
            match shape {
                LinkShape::Box => (
                    [o[0] + self.mins[0], o[1] + self.mins[1], o[2] + self.mins[2]],
                    [o[0] + self.maxs[0], o[1] + self.maxs[1], o[2] + self.maxs[2]],
                ),
                LinkShape::Radius => {
                    let r = radius_from_bounds(self.mins, self.maxs);
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
        tr_type: tr_type::STATIONARY,
        tr_time: 0,
        tr_duration: 0,
        tr_base: at,
        tr_delta: [0.0; 3],
    }
}

/// `G_SetOrigin`: stand the entity still at `origin`.
pub fn set_origin(es: &mut EntityState, r: &mut EntityShared, origin: [f32; 3]) {
    es.lerp.pos = stationary(origin);
    r.current_origin = origin;
}

/// `G_SetAngle`: hold the entity still at `angles`.
pub fn set_angle(es: &mut EntityState, r: &mut EntityShared, angles: [f32; 3]) {
    es.lerp.apos = stationary(angles);
    r.current_angles = angles;
}

/// What `G_TempEntity` sets on the entity it spawns for event `ev` at
/// `origin` (cut to whole units), before it links it. The caller sets
/// `classname` to `tempEntity` and `freeAfterEvent`.
pub fn init_temp_entity(
    es: &mut EntityState,
    r: &mut EntityShared,
    origin: [f32; 3],
    ev: u8,
    level_time: i32,
) {
    es.e_type = event_entity_type(ev);
    r.event_time = level_time;
    let whole = [
        trunc_i32(origin[0]) as f32,
        trunc_i32(origin[1]) as f32,
        trunc_i32(origin[2]) as f32,
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
    pub dyn_ents: u32,
}

impl ZombieTempEntCaps {
    pub const MAX_BULLET_HITS: u32 = 5;
    pub const MAX_DYN_ENTS: u32 = 4;

    /// Count `ev` and say whether it may spawn.
    pub fn allow(&mut self, ev: u8, restricted_temp_ents: bool) -> bool {
        match ev {
            crate::event::BULLET_HIT => {
                if self.bullet_hits >= Self::MAX_BULLET_HITS {
                    return false;
                }
                self.bullet_hits += 1;
                true
            }
            crate::event::BULLET_HIT_CLIENT_SMALL | crate::event::BULLET_HIT_CLIENT_LARGE => {
                !restricted_temp_ents
            }
            crate::event::CREATE_DYNENT => {
                if self.dyn_ents >= Self::MAX_DYN_ENTS {
                    return false;
                }
                self.dyn_ents += 1;
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
        let mins = [-15.0, -15.0, 0.0];
        let maxs = [15.0, 15.0, 72.0];
        assert_eq!(packed_solid(false, 0x0200_0000, mins, maxs), (104 << 16) | (1 << 8) | 15);
        assert_eq!(packed_solid(false, SOLID_CENTRED_CONTENTS, mins, maxs), (1 << 16) | (164 << 8) | 15);
        assert_eq!(packed_solid(false, 0x10, mins, maxs), 0);
        assert_eq!(packed_solid(true, 0, mins, maxs), SOLID_BMODEL);
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
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 72.0],
            current_origin: [100.0, 0.0, 0.0],
            ..Default::default()
        };
        r.link(LinkShape::Box);
        assert_eq!(r.absmin, [84.0, -16.0, -1.0]);
        assert_eq!(r.absmax, [116.0, 16.0, 73.0]);
        assert_eq!(r.linked, 1);
        r.mins = [-3.0, -4.0, 0.0];
        r.maxs = [3.0, 4.0, 12.0];
        r.link(LinkShape::Radius);
        assert_eq!(r.absmax, [114.0, 14.0, 14.0]);
    }

    #[test]
    fn brush_bounds_turn_with_angles() {
        let mut r = EntityShared {
            bmodel: 1,
            mins: [0.0, -1.0, 0.0],
            maxs: [10.0, 1.0, 2.0],
            current_angles: [0.0, 90.0, 0.0],
            ..Default::default()
        };
        r.link(LinkShape::Radius);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(r.absmin, [-2.0, -1.0, -1.0]), "{:?}", r.absmin);
        assert!(near(r.absmax, [2.0, 11.0, 3.0]), "{:?}", r.absmax);
    }

    #[test]
    fn temp_entity_sits_on_whole_units() {
        let mut es = EntityState::default();
        let mut r = EntityShared::default();
        init_temp_entity(&mut es, &mut r, [10.7, -3.9, 5.0], crate::event::OBITUARY, 500);
        assert_eq!(es.lerp.pos.tr_base, [10.0, -3.0, 5.0]);
        assert_eq!(r.current_origin, [10.0, -3.0, 5.0]);
        assert_eq!(es.e_type, 21 + crate::event::OBITUARY as i16);
        assert_eq!(r.event_time, 500);
    }

    #[test]
    fn zombie_caps() {
        let mut caps = ZombieTempEntCaps::default();
        let n = (0..9).filter(|_| caps.allow(crate::event::BULLET_HIT, false)).count();
        assert_eq!(n, 5);
        assert!(!caps.allow(crate::event::BULLET_HIT_CLIENT_SMALL, true));
        assert!(caps.allow(crate::event::BULLET_HIT_CLIENT_LARGE, false));
    }
}
