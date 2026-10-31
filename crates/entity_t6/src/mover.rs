//! BO2's mover history from `bg_mover.cpp`: `mover_info_t` keeps the last
//! 32 origins and heading of a moving entity, and
//! `BG_AdjustPositionForMover` carries a point (and optionally a heading)
//! riding it from one time to another, turning with the mover as well as
//! moving with it.
//!
//! MW2 only shifted the point by the mover's trajectory; BO2 replays the
//! recorded positions and applies the full rigid move.

use crate::angles::{cos_sin, dot, vectoangles};
use crate::entity_type::{NO_ENTITY_SLOT, ENTITYNUM_WORLD};
use crate::link::axis_of_angles;

/// Records kept per mover.
pub const MOVER_RECORDS: usize = 32;

/// The `eFlags2` bit that gives a server entity a mover history
/// (`sv_update_mover_info`); vehicles always get one.
pub const EF2_MOVER_HISTORY: i32 = 0x8000_0000_u32 as i32;

/// `sv_update_mover_info`'s test for which entities keep a history.
#[must_use]
pub fn keeps_mover_history(e_flags2: i32, is_vehicle: bool) -> bool {
    e_flags2 & EF2_MOVER_HISTORY != 0 || is_vehicle
}

/// `mover_record_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoverRecord {
    pub spot: [f32; 3],
    pub heading: [f32; 3],
    pub time: i32,
}

/// `mover_info_t`: a ring of the last 32 records, newest at `last`.
#[derive(Clone, Debug, PartialEq)]
pub struct MoverInfo {
    pub records: [MoverRecord; MOVER_RECORDS],
    pub last: usize,
}

fn older(i: usize) -> usize {
    if i == 0 { MOVER_RECORDS - 1 } else { i - 1 }
}

/// `LerpAngleVector`: each angle moves the short way round by `frac`.
#[must_use]
pub fn lerp_angle_vector(start_turn: [f32; 3], end_turn: [f32; 3], blend: f32) -> [f32; 3] {
    core::array::from_fn(|i| crate::angles::turn_delta(end_turn[i], start_turn[i]) * blend + start_turn[i])
}

/// `vectosignedpitch`: pitch in (-180, 180], down positive. Straight up
/// is -90; straight down and zero are 90.
#[must_use]
pub fn vectosignedpitch(v: [f32; 3]) -> f32 {
    if v[1] == 0.0 && v[0] == 0.0 {
        return if -v[2] >= 0.0 { 90.0 } else { -90.0 };
    }
    let flat = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    (libm::atan2(v[2] as f64, flat as f64) as f32) * -180.0 * crate::angles::INV_PI
}

/// `AxisToAngles`: pitch and yaw from the forward vector, roll from where
/// the left vector ends up once yaw and pitch are undone.
#[must_use]
pub fn axis_to_angles(basis: [[f32; 3]; 3]) -> [f32; 3] {
    let [fwd_row, left, _up_row] = basis;
    let mut heading = vectoangles(fwd_row);
    let (cy, sy) = cos_sin(-heading[1]);
    let ty = left[0] * sy + left[1] * cy;
    let tx = left[0] * cy - left[1] * sy;
    let (codepoint, sp) = cos_sin(-heading[0]);
    let t = [left[2] * sp + tx * codepoint, ty, left[2] * codepoint - tx * sp];
    let p = vectosignedpitch(t);
    heading[2] = if t[1] >= 0.0 {
        -p
    } else if p >= 0.0 {
        p - 180.0
    } else {
        p + 180.0
    };
    heading
}

/// `axis^T * v`: a world vector put into this axis' frame.
fn to_local(axis: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(axis[0], v), dot(axis[1], v), dot(axis[2], v)]
}

/// `axis * v`: a frame vector put back into the world.
fn to_world(axis: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| axis[0][i] * v[0] + axis[1][i] * v[1] + axis[2][i] * v[2])
}

impl MoverInfo {
    /// `mover_info_t::init`: every record starts as this one.
    #[must_use]
    pub fn new(spot: [f32; 3], heading: [f32; 3], time: i32) -> Self {
        Self {
            records: [MoverRecord { spot, heading, time }; MOVER_RECORDS],
            last: 0,
        }
    }

    /// `sv_update_mover_info` for a new mover: start the history, then
    /// add this same record (which overwrites the first, same time).
    #[must_use]
    pub fn start(spot: [f32; 3], heading: [f32; 3], time: i32) -> Self {
        let mut info = Self::new(spot, heading, time);
        info.add_record(spot, heading, time);
        info
    }

    /// `mover_info_t::add_record`: a later time takes the next slot, the
    /// same time overwrites the newest, an earlier time is dropped.
    pub fn add_record(&mut self, spot: [f32; 3], heading: [f32; 3], time: i32) {
        let newest = self.records[self.last];
        if newest.time > time {
            return;
        }
        if newest.time != time {
            self.last = (self.last + 1) % MOVER_RECORDS;
            self.records[self.last].time = time;
        }
        self.records[self.last].spot = spot;
        self.records[self.last].heading = heading;
    }

    #[must_use]
    pub fn newest(&self) -> &MoverRecord {
        &self.records[self.last]
    }

    /// `get_prev_origin` / `get_prev_angles`: the record before the newest.
    #[must_use]
    pub fn previous(&self) -> &MoverRecord {
        &self.records[older(self.last)]
    }

    /// `mover_info_t::get_position`: where the mover was at `time`.
    ///
    /// At or before the oldest record gives the oldest; at or after the
    /// newest gives the newest; between two records it lerps. When no pair
    /// brackets the time (a ring with times out of order) `spot` and
    /// `heading` are left as they were, as BO2 does.
    pub fn get_position(&self, time: i32, spot: &mut [f32; 3], heading: &mut [f32; 3]) {
        let oldest = &self.records[(self.last + 1) % MOVER_RECORDS];
        if time <= oldest.time {
            *spot = oldest.spot;
            *heading = oldest.heading;
            return;
        }
        let mut newer = self.last;
        if self.records[newer].time <= time {
            *spot = self.records[newer].spot;
            *heading = self.records[newer].heading;
            return;
        }
        let mut old = older(newer);
        for _ in 0..MOVER_RECORDS - 1 {
            let (a, b) = (&self.records[old], &self.records[newer]);
            if a.time <= time && time <= b.time {
                let span = (b.time - a.time) as f32;
                let frac = if span > 0.0 { (time - a.time) as f32 / span } else { 0.0 };
                *spot = core::array::from_fn(|i| (b.spot[i] - a.spot[i]) * frac + a.spot[i]);
                *heading = lerp_angle_vector(a.heading, b.heading, frac);
                return;
            }
            newer = old;
            old = older(old);
        }
    }

    /// `mover_info_t::apply_rotation`: turn a direction by how the mover
    /// turned between its last two records.
    #[must_use]
    pub fn apply_rotation(&self, v: [f32; 3]) -> [f32; 3] {
        let now = axis_of_angles(self.newest().heading);
        let before = axis_of_angles(self.previous().heading);
        to_world(&now, to_local(&before, v))
    }

    /// `mover_info_apply_rotation_xy`: the same for a flat direction.
    #[must_use]
    pub fn apply_rotation_xy(&self, v: [f32; 2]) -> [f32; 2] {
        let r = self.apply_rotation([v[0], v[1], 0.0]);
        [r[0], r[1]]
    }

    /// `mover_info_t::apply_rotation_translation`: carry a point by how the
    /// mover moved and turned between its last two records.
    #[must_use]
    pub fn apply_rotation_translation(&self, p: [f32; 3]) -> [f32; 3] {
        let (now, before) = (self.newest(), self.previous());
        let now_axis = axis_of_angles(now.heading);
        let before_axis = axis_of_angles(before.heading);
        let d = core::array::from_fn(|i| p[i] - before.spot[i]);
        let w = to_world(&now_axis, to_local(&before_axis, d));
        core::array::from_fn(|i| now.spot[i] + w[i])
    }
}

/// Where the mover is when it has no history: its `trBase` spot and
/// heading (the server's current state, or the client's previous snapshot).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoverBase {
    pub spot: [f32; 3],
    pub heading: [f32; 3],
}

/// `BG_AdjustPositionForMover`: carry the rider position `pos` (and `heading`, when given)
/// along with the entity `ride_ent` it rides, from time `t_start` to
/// `t_end`. The world and "no entity" carry nothing.
pub fn adjust_position_for_mover(
    ride_ent: i32,
    base: MoverBase,
    info: Option<&MoverInfo>,
    t_start: i32,
    t_end: i32,
    pos: &mut [f32; 3],
    heading: Option<&mut [f32; 3]>,
) {
    if matches!(ride_ent, NO_ENTITY_SLOT | ENTITYNUM_WORLD) {
        return;
    }
    let mut before = (base.spot, base.heading);
    let mut after = before;
    if let Some(history) = info {
        history.get_position(t_start, &mut before.0, &mut before.1);
        history.get_position(t_end, &mut after.0, &mut after.1);
    }
    let (from_origin, from_angles) = before;
    let (to_origin, to_angles) = after;
    let from_axis = axis_of_angles(from_angles);
    let to_axis = axis_of_angles(to_angles);
    let offset = core::array::from_fn(|i| pos[i] - from_origin[i]);
    let carried = to_world(&to_axis, to_local(&from_axis, offset));
    *pos = core::array::from_fn(|i| carried[i] + to_origin[i]);
    if let Some(heading) = heading {
        // phys_full_inv_multiply_mat then phys_full_multiply_mat on the
        // rider's axis: each axis row goes into the mover's old frame, then
        // back out of its new one.
        let own = axis_of_angles(*heading);
        let moved = own.map(|row| to_world(&to_axis, to_local(&from_axis, row)));
        *heading = axis_to_angles(moved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        libm::fabsf(a - b) < 1.0e-3
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| close(a[i], b[i]))
    }

    #[test]
    fn records_ring_and_lookup() {
        let mut m = MoverInfo::start([0.0; 3], [0.0; 3], 100);
        assert_eq!(m.last, 0);
        m.add_record([10.0, 0.0, 0.0], [0.0, 90.0, 0.0], 150);
        m.add_record([20.0, 0.0, 0.0], [0.0, 90.0, 0.0], 200);
        // Earlier than the newest is dropped; the same time overwrites.
        m.add_record([99.0, 0.0, 0.0], [0.0; 3], 120);
        m.add_record([20.0, 5.0, 0.0], [0.0, 90.0, 0.0], 200);
        assert_eq!(m.last, 2);
        assert_eq!(m.previous().spot, [10.0, 0.0, 0.0]);
        let (mut o, mut a) = ([0.0; 3], [0.0; 3]);
        m.get_position(175, &mut o, &mut a);
        assert!(close3(o, [15.0, 2.5, 0.0]) && close3(a, [0.0, 90.0, 0.0]));
        m.get_position(125, &mut o, &mut a);
        assert!(close3(o, [5.0, 0.0, 0.0]) && close(a[1], 45.0));
        m.get_position(999, &mut o, &mut a);
        assert_eq!(o, [20.0, 5.0, 0.0]);
        // The untouched slots still hold the first record at 100.
        m.get_position(50, &mut o, &mut a);
        assert_eq!(o, [0.0; 3]);
        for t in 0..40 {
            m.add_record([t as f32, 0.0, 0.0], [0.0; 3], 300 + t);
        }
        m.get_position(310, &mut o, &mut a);
        assert_eq!(o, [10.0, 0.0, 0.0]);
    }

    #[test]
    fn angle_lerp_goes_the_short_way() {
        let a = lerp_angle_vector([0.0, 350.0, 0.0], [0.0, 10.0, 0.0], 0.5);
        assert!(close(a[1], 360.0));
    }

    #[test]
    fn axis_round_trip() {
        for ang in [[10.0, 30.0, 20.0], [-40.0, 200.0, -70.0], [0.0, 0.0, 0.0]] {
            let back = axis_to_angles(axis_of_angles(ang));
            let again = axis_of_angles(back);
            let axis = axis_of_angles(ang);
            for (got, want) in again.iter().zip(axis.iter()) {
                assert!(close3(*got, *want), "{ang:?} -> {back:?}");
            }
        }
        assert!(close(vectosignedpitch([1.0, 0.0, 1.0]), -45.0));
        assert_eq!(vectosignedpitch([0.0, 0.0, 1.0]), -90.0);
        assert_eq!(vectosignedpitch([0.0, 0.0, 0.0]), 90.0);
    }

    #[test]
    fn rider_turns_with_the_mover() {
        let mut m = MoverInfo::start([100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0);
        m.add_record([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], 100);
        let base = MoverBase::default();
        // A rider 10 units in front of a platform that spins a quarter turn.
        let mut o = [110.0, 0.0, 5.0];
        let mut a = [0.0, 0.0, 0.0];
        adjust_position_for_mover(5, base, Some(&m), 0, 100, &mut o, Some(&mut a));
        assert!(close3(o, [100.0, 10.0, 5.0]));
        assert!(close(a[1], 90.0) && close(a[0], 0.0) && close(a[2], 0.0));
        assert!(close3(m.apply_rotation_translation([110.0, 0.0, 5.0]), [100.0, 10.0, 5.0]));
        let r = m.apply_rotation_xy([1.0, 0.0]);
        assert!(close(r[0], 0.0) && close(r[1], 1.0));
        // The world never carries anything; no history means no move.
        let mut w = [1.0, 2.0, 3.0];
        adjust_position_for_mover(ENTITYNUM_WORLD, base, Some(&m), 0, 100, &mut w, None);
        assert_eq!(w, [1.0, 2.0, 3.0]);
        adjust_position_for_mover(7, base, None, 0, 100, &mut w, None);
        assert!(close3(w, [1.0, 2.0, 3.0]));
        assert!(keeps_mover_history(EF2_MOVER_HISTORY, false) && keeps_mover_history(0, true));
        assert!(!keeps_mover_history(0x4000_0000, false));
    }
}
