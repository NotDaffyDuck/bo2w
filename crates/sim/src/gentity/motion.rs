//! Starting states and trajectories for the entities the sim moves itself: script movers, missiles and dropped items.

use entity_t6::{
    EntityState, LerpMissile, Trajectory, et, evaluate_trajectory,
    trajectory::tr_type::{LINEAR_STOP, STATIONARY},
};

use crate::identities::ScriptModelId;

/// A script-spawned moving model: its replicated state, and the boxes its collision is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptMoverGentity {
    pub id: ScriptModelId,
    pub state: EntityState,
    pub box_mid: [f32; 3],
    pub box_half: [f32; 3],
    pub link_mid: [f32; 3],
    pub link_half: [f32; 3],
    pub nonsolid: bool,
    /// Bit set of the clients this model is shown to.
    pub shown_to: u64,
}

impl Default for ScriptMoverGentity {
    fn default() -> Self {
        Self {
            id: ScriptModelId::from_wire(0),
            state: EntityState::default(),
            box_mid: [0.0; 3],
            box_half: [0.0; 3],
            link_mid: [0.0; 3],
            link_half: [0.0; 3],
            nonsolid: false,
            shown_to: 0,
        }
    }
}

pub fn pos_from_entity_state(state: &EntityState) -> Trajectory {
    state.lerp.pos
}

pub fn apos_from_entity_state(state: &EntityState) -> Trajectory {
    state.lerp.apos
}

/// A trajectory that stays where it is.
fn parked_at(base: [f32; 3]) -> Trajectory {
    Trajectory { tr_type: STATIONARY, tr_base: base, ..Trajectory::default() }
}

pub fn init_script_mover_state(number: i32, origin: [f32; 3], angles: [f32; 3]) -> EntityState {
    let mut state = EntityState { number, e_type: et::SCRIPTMOVER, ..EntityState::default() };
    state.lerp.pos = parked_at(origin);
    state.lerp.apos = parked_at(angles);
    state
}

pub fn init_missile_state(number: i32, weapon: u32, pos: Trajectory, apos: Trajectory, launch_time: i32) -> EntityState {
    let mut state = EntityState {
        number,
        e_type: et::MISSILE,
        weapon: playerstate_t6::Weapon(weapon),
        ..EntityState::default()
    };
    state.lerp.e_flags = 0;
    state.lerp.pos = pos;
    state.lerp.apos = apos;
    state.lerp.u.set_missile(&LerpMissile { launch_time, ..LerpMissile::default() });
    state
}

pub fn init_item_state(number: i32, weapon: u32, pos: Trajectory, apos: Trajectory, owner_num: i32) -> EntityState {
    let mut state = EntityState {
        number,
        e_type: et::ITEM,
        weapon: playerstate_t6::Weapon(weapon),
        client_num: owner_num as u8,
        ..EntityState::default()
    };
    state.lerp.e_flags = 0;
    state.lerp.pos = pos;
    state.lerp.apos = apos;
    state
}

/// The trajectory that turns `current` at `speed` degrees a second for `total_time_seconds`, starting from wherever
/// `current` has got to by `level_time_ms`.
pub fn rotate_velocity_apos(
    current: &Trajectory,
    speed: [f32; 3],
    total_time_seconds: f32,
    level_time_ms: i32,
) -> Trajectory {
    let from = if current.tr_type == STATIONARY { current.tr_base } else { evaluate_trajectory(current, level_time_ms) };
    Trajectory {
        tr_type: LINEAR_STOP,
        tr_time: level_time_ms,
        tr_duration: (total_time_seconds * 1000.0) as i32,
        tr_base: from,
        tr_delta: speed,
    }
}

pub fn begin_script_mover_rotate_velocity(
    state: &mut EntityState,
    speed: [f32; 3],
    total_time_seconds: f32,
    level_time_ms: i32,
) {
    state.lerp.apos = rotate_velocity_apos(&state.lerp.apos, speed, total_time_seconds, level_time_ms);
}
