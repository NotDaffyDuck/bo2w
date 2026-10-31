//! Elem movement and collision: 50 ms sub-steps, gravity, reflection.

use crate::flags::ELEM_DIES_ON_CONTACT;
use crate::flags::FX_ELEM_MODEL_PHYSICS;
use crate::flags::ELEM_HITS_WORLD;

pub const BOUNCE_SLICE_MS: i32 = 50;

pub const PARTICLE_HIT_FILTER: u32 = 0x811;

pub const PARTICLE_HIT_FILTER_PICKUPS: u32 = 0xc11;

/// Added to the trace mask when the elem uses item clip (flag 0x4000).
// BO2 unknown: which flag picks 0xc11 over 0x811 is not in the facts (see
// facts/QUESTIONS.md), so no mask chooser is given yet.
pub const FX_TRACE_MASK_BLOCK_SIGHT: u32 = 0x2000;

/// A hit normal with z above this is ground.
pub const FX_GROUND_NORMAL_Z: f32 = 0.7;

pub const FX_GRAVITY_SCALE: f32 = 800.0;

pub const FX_GRAVITY_CORRECTION: f32 = 0.8;

/// An impact effect spawns when the squared speed at the hit is above this.
pub const FX_IMPACT_MIN_SPEED_SQ: f32 = 1.0;

/// Velocity graphs are scaled by this.
pub const FX_VELOCITY_GRAPH_SCALE: f32 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleBounceSlice {
    pub window_begin_ms: i32,
    pub window_close_ms: i32,
}

/// Cuts [start, end) into 50 ms pieces; the last one is shorter.
#[derive(Clone, Debug)]
pub struct FxCollideSubsteps {
    cur: i32,
    end: i32,
}

impl Iterator for FxCollideSubsteps {
    type Item = ParticleBounceSlice;

    fn next(&mut self) -> Option<Self::Item> {
        (self.cur < self.end).then(|| {
            let upto = self.cur.saturating_add(BOUNCE_SLICE_MS).min(self.end);
            let from = core::mem::replace(&mut self.cur, upto);
            ParticleBounceSlice {
                window_begin_ms: from,
                window_close_ms: self.cur,
            }
        })
    }
}

#[inline]
pub fn collide_substeps(window_begin_ms: i32, window_close_ms: i32) -> FxCollideSubsteps {
    FxCollideSubsteps {
        cur: window_begin_ms,
        end: window_close_ms,
    }
}

/// Model-physics elems are moved by the physics engine, not here.
#[inline]
pub fn moved_by_fx(elem_flags: u32) -> bool {
    elem_flags & FX_ELEM_MODEL_PHYSICS == 0
}

#[inline]
pub fn uses_collision(elem_flags: u32) -> bool {
    elem_flags & ELEM_HITS_WORLD != 0
}

#[inline]
pub fn dies_on_touch(elem_flags: u32) -> bool {
    elem_flags & ELEM_DIES_ON_CONTACT != 0
}

#[inline]
pub fn is_ground(normal_z: f32) -> bool {
    normal_z > FX_GROUND_NORMAL_Z
}

#[inline]
pub fn spawns_impact(velocity: [f32; 3]) -> bool {
    dot(velocity, velocity) > FX_IMPACT_MIN_SPEED_SQ
}

/// v - 2 (v . n) n
#[inline]
pub fn reflect(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let twice_along = 2.0 * dot(v, n);
    core::array::from_fn(|k| v[k] - twice_along * n[k])
}

/// Gravity over dt seconds: z velocity falls by g dt and z position by
/// half g dt squared, with g = sampled gravity times 800.
#[inline]
pub fn gravity_step(sampled_gravity: f32, dt_sec: f32) -> (f32, f32) {
    let g = sampled_gravity * FX_GRAVITY_SCALE;
    (-g * dt_sec, -0.5 * g * dt_sec * dt_sec)
}

#[inline]
fn dot(lhs: [f32; 3], b: [f32; 3]) -> f32 {
    lhs.iter().zip(&b).fold(0.0, |sum, (x, y)| sum + x * y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substeps() {
        let s: Vec<_> = collide_substeps(0, 120).collect();
        assert_eq!(
            s,
            [
                ParticleBounceSlice {
                    window_begin_ms: 0,
                    window_close_ms: 50
                },
                ParticleBounceSlice {
                    window_begin_ms: 50,
                    window_close_ms: 100
                },
                ParticleBounceSlice {
                    window_begin_ms: 100,
                    window_close_ms: 120
                },
            ]
        );
        assert_eq!(collide_substeps(5, 5).count(), 0);
        assert_eq!(collide_substeps(0, 100).count(), 2);
    }

    #[test]
    fn flags_and_ground() {
        assert!(uses_collision(0x200));
        assert!(!uses_collision(0x100));
        assert!(dies_on_touch(0x400));
        assert!(!dies_on_touch(0x200));
        assert!(!moved_by_fx(0x800_0000));
        assert!(is_ground(0.71));
        assert!(!is_ground(0.7));
    }

    #[test]
    fn reflect_and_gravity() {
        assert_eq!(reflect([1.0, 0.0, -2.0], [0.0, 0.0, 1.0]), [1.0, 0.0, 2.0]);
        assert!(spawns_impact([1.0, 0.5, 0.0]));
        assert!(!spawns_impact([1.0, 0.0, 0.0]));
        assert_eq!(gravity_step(1.0, 0.5), (-400.0, -100.0));
    }
}
