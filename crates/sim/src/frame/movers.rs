//! Script movers: models the map scripts spawn, move and spin.

use entity_t6::Trajectory;
use entity_t6::trajectory::tr_type::{LINEAR, STATIONARY};

use super::FrameWorld;
use super::rows::ScriptMoverRow;
use super::store::{find, find_mut, listed_numbers, remove_row, script_mover_by_id, spawn_row, visit_rows};
use crate::gentity::{EntityAllocError, ScriptMoverGentity, begin_script_mover_rotate_velocity, init_script_mover_state};
use crate::identities::ScriptModelId;

/// How long a fan blade keeps turning once it is set going, in seconds.
const FAN_BLADE_SPIN_SECONDS: f32 = 20000.0;

/// A blade whose right vector lies mostly along x or y spins on pitch; any other blade spins on yaw.
const FAN_BLADE_AXIS_DOT: f32 = 0.9;

fn fan_blade_spin(right: [f32; 3], speed: f32) -> [f32; 3] {
    if right[0].abs() > FAN_BLADE_AXIS_DOT || right[1].abs() > FAN_BLADE_AXIS_DOT {
        [speed, 0.0, 0.0]
    } else {
        [0.0, speed, 0.0]
    }
}

/// Re-aims `track` at `target` as of `time_ms`. When the track was last set exactly one tick ago it is given the
/// velocity that covers the gap in a tick, so a client evaluating between snapshots sees smooth motion rather than a
/// 20 Hz step; otherwise it is parked at `target`. `difference(to, from)` is the signed gap along one axis.
fn glide_to(track: &mut Trajectory, time_ms: i32, target: [f32; 3], difference: fn(f32, f32) -> f32) {
    let per_second = 1000.0 / crate::MATCH_TICK_MS as f32;
    let set_last_tick = track.tr_time == time_ms - crate::MATCH_TICK_MS as i32;
    let delta: [f32; 3] = if set_last_tick {
        core::array::from_fn(|axis| difference(target[axis], track.tr_base[axis]) * per_second)
    } else {
        [0.0; 3]
    };
    track.tr_type = if delta == [0.0; 3] { STATIONARY } else { LINEAR };
    track.tr_time = time_ms;
    track.tr_duration = 0;
    track.tr_base = target;
    track.tr_delta = delta;
}

impl FrameWorld<'_> {
    pub(crate) fn script_mover_mut_by_number(&mut self, number: i32) -> Option<&mut ScriptMoverGentity> {
        find_mut::<ScriptMoverRow>(self.ecs, number)
    }

    pub(crate) fn script_mover_by_number(&self, number: i32) -> Option<ScriptMoverGentity> {
        find::<ScriptMoverRow>(self.ecs, number).copied()
    }

    pub(crate) fn visit_script_movers(&self, visit: impl FnMut(&ScriptMoverGentity)) {
        visit_rows::<ScriptMoverRow>(self.ecs, visit);
    }

    pub fn gentity_number(&self, id: ScriptModelId) -> Option<i32> {
        script_mover_by_id(self.ecs, id).map(|mover| mover.state.number)
    }

    /// Movers solid as a packed box rather than as a brush model: `(number, origin, packed box)`. The packed box is
    /// the entity state's `solid` field (half-width, depth below and height above plus 32, a byte each).
    pub(crate) fn packed_solid_movers(&self) -> Vec<(u16, [f32; 3], u32)> {
        let brush_model = entity_t6::script_mover::SOLID_BMODEL;
        listed_numbers::<ScriptMoverRow>(self.ecs)
            .into_iter()
            .filter_map(|number| {
                let mover = find::<ScriptMoverRow>(self.ecs, number)?;
                let solid = mover.state.solid;
                (solid != 0 && solid != brush_model).then_some((number as u16, mover.state.lerp.pos.tr_base, solid as u32))
            })
            .collect()
    }

    /// The number of the mover already spawned for `id`, or a freshly spawned, parked one.
    pub fn spawn_script_mover(
        &mut self,
        id: ScriptModelId,
        origin: [f32; 3],
        angles: [f32; 3],
    ) -> Result<i32, EntityAllocError> {
        if let Some(number) = self.gentity_number(id) {
            return Ok(number);
        }
        let number = self.allocate_dynamic_entity(crate::gentity::EntityRunKind::ScriptMover)?.number();
        let state = init_script_mover_state(number, origin, angles);
        spawn_row::<ScriptMoverRow>(self.ecs, ScriptMoverGentity { id, state, ..Default::default() });
        Ok(number)
    }

    /// Removes the mover at `number` and gives its entity number back. False when there is no such mover.
    pub(crate) fn remove_script_mover_by_number(&mut self, number: i32) -> bool {
        if remove_row::<ScriptMoverRow>(self.ecs, number).is_none() {
            return false;
        }
        self.free_dynamic_entity_number(number);
        true
    }

    pub fn begin_script_mover_rotate_velocity(
        &mut self,
        id: ScriptModelId,
        speed: [f32; 3],
        total_time_seconds: f32,
        level_time_ms: i32,
    ) -> bool {
        let Some(number) = self.gentity_number(id) else {
            return false;
        };
        let Some(mover) = self.script_mover_mut_by_number(number) else {
            return false;
        };
        begin_script_mover_rotate_velocity(&mut mover.state, speed, total_time_seconds, level_time_ms);
        true
    }

    /// Sets every mover spinning at `speed` degrees a second about the axis its blades turn on; counts how many took.
    pub fn begin_script_movers_rotate_velocity_supplied(&mut self, speed: f32, level_time_ms: i32) -> usize {
        let mut spins = Vec::new();
        self.visit_script_movers(|mover| {
            let right = game_math::angle_vectors(mover.state.lerp.apos.tr_base).1;
            spins.push((mover.id, fan_blade_spin(right, speed)));
        });
        spins
            .into_iter()
            .filter(|&(id, spin)| self.begin_script_mover_rotate_velocity(id, spin, FAN_BLADE_SPIN_SECONDS, level_time_ms))
            .count()
    }

    pub fn set_script_mover_r_box(&mut self, number: i32, box_mid: [f32; 3], box_half: [f32; 3]) -> bool {
        let Some(mover) = self.script_mover_mut_by_number(number) else {
            return false;
        };
        mover.box_mid = box_mid;
        mover.box_half = box_half;
        true
    }

    /// Parks the mover at `origin`, wherever it was heading.
    pub fn set_script_mover_origin(&mut self, number: i32, origin: [f32; 3]) -> bool {
        let Some(mover) = self.script_mover_mut_by_number(number) else {
            return false;
        };
        let pos = &mut mover.state.lerp.pos;
        pos.tr_type = STATIONARY;
        pos.tr_delta = [0.0; 3];
        pos.tr_base = origin;
        true
    }

    /// Moves the mover to a pose for tick time `time_ms`, with velocities that let clients smooth it between snapshots.
    pub fn set_script_mover_pose(&mut self, number: i32, time_ms: i32, origin: [f32; 3], angles: [f32; 3]) -> bool {
        let Some(mover) = self.script_mover_mut_by_number(number) else {
            return false;
        };
        glide_to(&mut mover.state.lerp.pos, time_ms, origin, |to, from| to - from);
        glide_to(&mut mover.state.lerp.apos, time_ms, angles, game_math::angle_subtract);
        true
    }
}
