//! Starting states and trajectories for the entities the sim moves itself: script movers, missiles and dropped items.

use entity_t6::{
    EntitySnapshot, LerpMissile, Trajectory, et, evaluate_trajectory,
    flight::path_kind::{STRAIGHT_THEN_STOP, AT_REST},
};

use crate::match_ids::ScriptedModel;

/// A script-spawned moving model: its replicated state, and the boxes its collision is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverSlot {
    pub id: ScriptedModel,
    pub state: EntitySnapshot,
    pub box_centre: [f32; 3],
    pub box_span: [f32; 3],
    pub link_centre: [f32; 3],
    pub link_span: [f32; 3],
    pub nonsolid: bool,
    /// Bit set of the clients this model is shown to.
    pub shown_to: u64,
    /// A bit per player it does not stop (hidden from him); its client gets
    /// it nonsolid. 0 for everything else.
    pub passes_for: u64,
}

impl Default for MoverSlot {
    fn default() -> Self {
        // Entity 0's id, a blank state, empty boxes, solid, shown to nobody.
        let empty = [0.0; 3];
        let (box_mid, box_half, link_mid, link_half) = (empty, empty, empty, empty);
        let (id, state) = (ScriptedModel::decode_net(0), EntitySnapshot::default());
        Self { id, state, box_centre: box_mid, box_span: box_half, link_centre: link_mid, link_span: link_half, nonsolid: false, shown_to: 0, passes_for: 0 }
    }
}

pub fn move_path_of(state: &EntitySnapshot) -> Trajectory {
    state.lerp.pos
}

pub fn turn_path_of(state: &EntitySnapshot) -> Trajectory {
    state.lerp.apos
}

/// A trajectory that stays where it is.
fn parked_at(base: [f32; 3]) -> Trajectory {
    Trajectory { tr_type: AT_REST, base_point: base, ..Trajectory::default() }
}

pub fn build_scripted_mover(number: i32, spot: [f32; 3], aim: [f32; 3]) -> EntitySnapshot {
    let mut state = EntitySnapshot { slot_index: number, e_type: et::SCRIPTMOVER, ..EntitySnapshot::default() };
    state.lerp.pos = parked_at(spot);
    state.lerp.apos = parked_at(aim);
    state
}

pub fn shell_state_at(number: i32, gun: u32, path: Trajectory, spin_path: Trajectory, spawn_moment: i32) -> EntitySnapshot {
    let mut state = EntitySnapshot {
        slot_index: number,
        e_type: et::MISSILE,
        weapon: playerstate_t6::Weapon(gun),
        ..EntitySnapshot::default()
    };
    state.lerp.e_flags = 0;
    state.lerp.pos = path;
    state.lerp.apos = spin_path;
    state.lerp.u.set_missile(&LerpMissile { spawn_moment, ..LerpMissile::default() });
    state
}

pub fn loot_state_at(number: i32, gun: u32, path: Trajectory, spin_path: Trajectory, owner_num: i32) -> EntitySnapshot {
    let mut state = EntitySnapshot {
        slot_index: number,
        e_type: et::ITEM,
        weapon: playerstate_t6::Weapon(gun),
        client_num: owner_num as u8,
        ..EntitySnapshot::default()
    };
    state.lerp.e_flags = 0;
    state.lerp.pos = path;
    state.lerp.apos = spin_path;
    state
}

/// The trajectory that turns `current` at `speed` degrees a second for `total_time_seconds`, starting from wherever
/// `current` has got to by `level_time_ms`.
pub fn spin_rate_pose(current: &Trajectory, pace: [f32; 3], spin_secs: f32, clock_ms: i32) -> Trajectory {
    let from = if current.tr_type == AT_REST { current.base_point } else { evaluate_trajectory(current, clock_ms) };
    let duration_ms = (spin_secs * 1000.0) as i32;
    Trajectory { tr_type: STRAIGHT_THEN_STOP, base_point: from, rate: pace, began_at: clock_ms, span_ms: duration_ms }
}

pub fn start_scripted_spin(state: &mut EntitySnapshot, pace: [f32; 3], spin_secs: f32, clock_ms: i32) {
    state.lerp.apos = spin_rate_pose(&state.lerp.apos, pace, spin_secs, clock_ms);
}
