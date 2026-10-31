//! Script movers: models the map scripts spawn, move and spin.

use entity_t6::Trajectory;
use entity_t6::flight::path_kind::{STRAIGHT, AT_REST};

use super::StepWorldView;
use super::rows::ScriptMoverRow;
use super::store::{find, find_mut, listed_numbers, remove_row, script_mover_by_id, spawn_row, visit_rows};
use crate::ent_slots::{EntitySlotShortage, MoverSlot, start_scripted_spin, build_scripted_mover};
use crate::match_ids::ScriptedModel;

/// How long a fan blade keeps turning once it is set going, in seconds.
const FAN_BLADE_SPIN_SECONDS: f32 = 20000.0;

/// A blade whose right vector lies mostly along x or y spins on pitch; any other blade spins on yaw.
const FAN_BLADE_AXIS_DOT: f32 = 0.9;

fn fan_blade_spin(right: [f32; 3], pace: f32) -> [f32; 3] {
    if right[0].abs() > FAN_BLADE_AXIS_DOT || right[1].abs() > FAN_BLADE_AXIS_DOT {
        [pace, 0.0, 0.0]
    } else {
        [0.0, pace, 0.0]
    }
}

/// Re-aims `track` at `target` as of `clock_stamp_ms`. When the track was last set exactly one tick ago it is given the
/// velocity that covers the gap in a tick, so a client evaluating between snapshots sees smooth motion rather than a
/// 20 Hz step; otherwise it is parked at `target`. `difference(to, from)` is the signed gap along one axis.
fn glide_to(track: &mut Trajectory, clock_stamp_ms: i32, aim_at: [f32; 3], difference: fn(f32, f32) -> f32) {
    let per_second = 1000.0 / crate::SIM_STEP_MILLIS as f32;
    let set_last_tick = track.began_at == clock_stamp_ms - crate::SIM_STEP_MILLIS as i32;
    let gap: [f32; 3] = if set_last_tick {
        core::array::from_fn(|axis| difference(aim_at[axis], track.base_point[axis]) * per_second)
    } else {
        [0.0; 3]
    };
    track.tr_type = if gap == [0.0; 3] { AT_REST } else { STRAIGHT };
    track.began_at = clock_stamp_ms;
    track.span_ms = 0;
    track.base_point = aim_at;
    track.rate = gap;
}

impl<'frame> StepWorldView<'frame> {
    pub(crate) fn script_mover_mut_by_number(&mut self, slot: i32) -> Option<&mut MoverSlot> {
        find_mut::<ScriptMoverRow>(self.ecs, slot)
    }

    /// Runs `edit` on the mover with this entity number; false when no such mover exists.
    fn edit_mover(&mut self, slot: i32, edit: impl FnOnce(&mut MoverSlot)) -> bool {
        self.script_mover_mut_by_number(slot).map(edit).is_some()
    }

    pub(crate) fn script_mover_by_number(&self, slot: i32) -> Option<MoverSlot> {
        find::<ScriptMoverRow>(self.ecs, slot).copied()
    }

    pub(crate) fn each_mover_row(&self, each: impl FnMut(&MoverSlot)) {
        visit_rows::<ScriptMoverRow>(self.ecs, each);
    }

    pub fn game_actor_number(&self, model: ScriptedModel) -> Option<i32> {
        script_mover_by_id(self.ecs, model).map(|found| found.state.slot_index)
    }

    /// Movers solid as a packed box rather than as a brush model: `(number, origin, packed box)`. The packed box is
    /// the entity state's `solid` field (half-width, depth below and height above plus 32, a byte each).
    pub(crate) fn packed_solid_movers(&self) -> Vec<(u16, [f32; 3], u32)> {
        let solid_model = entity_t6::script_mover::SOLID_BMODEL;
        listed_numbers::<ScriptMoverRow>(self.ecs)
            .into_iter()
            .filter_map(|number| {
                let mover = find::<ScriptMoverRow>(self.ecs, number)?;
                let solid = mover.state.solid;
                (solid != 0 && solid != solid_model).then_some((number as u16, mover.state.lerp.pos.base_point, solid as u32))
            })
            .collect()
    }

    /// The number of the mover already spawned for `id`, or a freshly spawned, parked one.
    pub fn create_scripted_mover(
        &mut self,
        id: ScriptedModel,
        spot: [f32; 3],
        aim: [f32; 3],
    ) -> Result<i32, EntitySlotShortage> {
        if let Some(existing) = self.game_actor_number(id) {
            return Ok(existing);
        }
        let slot = self.allocate_dynamic_entity(crate::ent_slots::EntityBehaviorClass::ScriptedMover)?.slot();
        let state = build_scripted_mover(slot, spot, aim);
        spawn_row::<ScriptMoverRow>(self.ecs, MoverSlot { id, state, ..Default::default() });
        Ok(slot)
    }

    /// Removes the mover at `number` and gives its entity number back. False when there is no such mover.
    pub(crate) fn remove_script_mover_by_number(&mut self, number: i32) -> bool {
        if remove_row::<ScriptMoverRow>(self.ecs, number).is_none() {
            return false;
        }
        self.free_dynamic_entity_number(number);
        true
    }

    pub fn start_scripted_spin(
        &mut self,
        id: ScriptedModel,
        pace: [f32; 3],
        spin_secs: f32,
        clock_ms: i32,
    ) -> bool {
        let Some(slot) = self.game_actor_number(id) else { return false };
        self.edit_mover(slot, |found| {
            start_scripted_spin(&mut found.state, pace, spin_secs, clock_ms);
        })
    }

    /// Sets every mover spinning at `speed` degrees a second about the axis its blades turn on; counts how many took.
    pub fn start_rate_spins(&mut self, speed: f32, level_time_ms: i32) -> usize {
        let mut spins = Vec::new();
        self.each_mover_row(|mover| {
            let right = game_math::euler_basis(mover.state.lerp.apos.base_point).1;
            spins.push((mover.id, fan_blade_spin(right, speed)));
        });
        spins
            .into_iter()
            .filter(|&(id, spin)| self.start_scripted_spin(id, spin, FAN_BLADE_SPIN_SECONDS, level_time_ms))
            .count()
    }

    pub fn resize_scripted_mover_box(&mut self, slot: i32, box_centre: [f32; 3], box_reach: [f32; 3]) -> bool {
        self.edit_mover(slot, |found| {
            found.box_centre = box_centre;
            found.box_span = box_reach;
        })
    }

    /// Parks the mover at `origin`, wherever it was heading.
    pub fn place_mover(&mut self, slot: i32, spot: [f32; 3]) -> bool {
        self.edit_mover(slot, |found| {
            let pos = &mut found.state.lerp.pos;
            pos.tr_type = AT_REST;
            pos.rate = [0.0; 3];
            pos.base_point = spot;
        })
    }

    /// Moves the mover to a pose for tick time `clock_stamp_ms`, with velocities that let clients smooth it between snapshots.
    pub fn set_script_mover_pose(&mut self, slot: i32, clock_stamp_ms: i32, spot: [f32; 3], aim: [f32; 3]) -> bool {
        self.edit_mover(slot, |found| {
            glide_to(&mut found.state.lerp.pos, clock_stamp_ms, spot, |to, from| to - from);
            glide_to(&mut found.state.lerp.apos, clock_stamp_ms, aim, game_math::degree_gap);
        })
    }
}
