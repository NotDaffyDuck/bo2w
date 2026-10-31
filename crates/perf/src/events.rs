//! One call per notable event, carrying the facts a trace would label it
//! with. Nothing records them, so each call is empty.

#![allow(clippy::too_many_arguments)]

/// One player's movement and weapon state after a server tick.
#[inline]
pub fn avatar_step_mark(
    _client_id: u32,
    _is_bot: bool,
    _time_ms: i32,
    _level_time_ms: i32,
    _origin: [f32; 3],
    _velocity_z: f32,
    _yaw: f32,
    _pitch: f32,
    _jump_time: i32,
    _buttons: u32,
    _weapon_state: i32,
    _ammo_clip: i32,
    _walking: Option<i32>,
    _legs_anim: i32,
    _torso_anim: i32,
    _lifecycle: &'static str,
) {
}

/// A player died; `suicide` is 1 when they killed themselves.
#[inline]
pub fn death(_victim: u32, _attacker: Option<u32>, _suicide: u8, _tick: u32) {}

/// A weapon fired a projectile.
#[inline]
pub fn projectile(_weapon: u32) {}

/// How the local player's shown view lines up with the server's: which
/// snapshot was chosen, the two origins, and the client clock's lag.
#[inline]
pub fn feel(
    _time_ms: i32,
    _lifecycle: Option<&str>,
    _seat_applied: Option<i32>,
    _seat_lookup_tick: Option<i32>,
    _present_choice: Option<&str>,
    _snapshot_delta_ms: Option<i32>,
    _authority_origin: Option<[f32; 3]>,
    _adopted_origin: Option<[f32; 3]>,
    _clock_debt_ms: Option<f32>,
    _ingress_queue_depth: Option<i64>,
    _authority_command_time: Option<i32>,
    _predicted_command_time: Option<i32>,
) {
}

/// A body left on the ground and the pose it was drawn in.
#[inline]
pub fn corpse(
    _occupied: i64,
    _origin_z: Option<f32>,
    _pose_type: Option<i64>,
    _legs_anim: Option<&str>,
) {
}

/// An item lying in the world.
#[inline]
pub fn item(_entity_type: i32, _clip: Option<i32>, _scavenger: Option<i32>, _gap: Option<&str>) {}

/// Someone picked an item up.
#[inline]
pub fn pickup(_picker_move_type: i32) {}

/// Where another player was drawn this frame, against their snapshot.
#[inline]
pub fn remote(
    _client: u32,
    _time_ms: i32,
    _pose_type: i32,
    _origin: [f32; 3],
    _snapshot_origin: Option<[f32; 3]>,
    _outcome: Option<&str>,
) {
}

/// A model was drawn without its lighting.
#[inline]
pub fn illumination_fault_mark() {}

/// What the renderer planned for one owner's model this frame.
#[inline]
pub fn draw_thread_layout_mark(
    _frame: u64,
    _world_generation: Option<u64>,
    _owner_kind: &str,
    _owner: u32,
    _model: Option<&str>,
    _outcome: &str,
    _object: Option<u16>,
    _camera_origin: Option<[f32; 3]>,
    _camera_hidden: Option<bool>,
    _lighting: Option<u32>,
    _surfaces: u32,
) {
}

/// The running match was torn down.
#[inline]
pub fn match_torn(_reason: &str) {}

/// A match was set up on `zone`.
#[inline]
pub fn match_ready_mark(_zone: &str, _has_world: i64) {}

/// The world is not shown yet; what it still waits for.
#[inline]
pub fn level_hold(_spawned: i64, _gpu_plan: i64, _glass: i64) {}

/// The world is up.
#[inline]
pub fn level_ready(_spawned: i64) {}

/// The simulation is held; what it still waits for.
#[inline]
pub fn sim_hold(_running: i64, _movers: i64, _loopback_pending: i64) {}

/// Ambient sound is held until it has booted.
#[inline]
pub fn ambient_hold(_booted: i64) {}

/// Ambient sound started for `zone`.
#[inline]
pub fn ambient_boot(_zone: &str, _alias: Option<&str>) {}

/// One step of swapping to another map.
#[inline]
pub fn swap(_id: u64, _phase: &str, _target: &str) {}

/// Theater playback state.
#[inline]
pub fn theater(_present: i64, _quit_on_end: Option<i64>, _runs_authority: Option<i64>) {}

/// The client game view is held; which snapshot it would show.
#[inline]
pub fn view_freeze_mark(_present_choice: &str, _has_player_state: i64) {}
