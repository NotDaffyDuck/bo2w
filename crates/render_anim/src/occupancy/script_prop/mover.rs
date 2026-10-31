//! Where a script mover stands this frame, read from its centity.

use bevy::prelude::*;
use render_scene::LevelScriptModelInstance;

use super::state::ScriptPropSkeletons;
use crate::occupancy::t6_body::SlowGuard;

/// The time at which movers are sampled: the client's clock once it runs,
/// else the snapshot's own, moved onto the trajectory timeline.
fn sample_time_ms(
    clock: Option<&net::FrameClock>,
    presented: &net::ShownWorldFrame,
) -> Option<i32> {
    let tick = presented.snapshot()?.tick;
    let running = clock.filter(|clock| clock.started());
    let now = running.map_or_else(|| sim::level_time_ms(tick), net::FrameClock::time);
    Some(presented.trajectory_time_ms(now))
}

fn sample(runtime: &net::ClientEntityPoseState, clock_stamp_ms: i32) -> ([f32; 3], [f32; 3]) {
    let at = |trajectory| entity_t6::evaluate_trajectory(trajectory, clock_stamp_ms);
    let current = &runtime.current;
    (at(&current.path), at(&current.spin))
}

/// The rotation of a model facing `[pitch, yaw, roll]` degrees.
fn rotation_from_angles([pitch, yaw, roll]: [f32; 3]) -> Quat {
    let [yaw, pitch, roll] = [yaw, pitch, roll].map(f32::to_radians);
    Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll)
}

/// A script mover's place and angles this frame, sampled as
/// `apply_script_mover_centity_pose` places its model (a camera that rides
/// the model reads this, not last frame's Transform).
pub(crate) fn script_mover_pose_now(
    slots: Option<&net::EntityTableCells>,
    presented: &net::ShownWorldFrame,
    pose_states: &Query<&net::ClientEntityPoseState>,
    clock: &net::FrameClock,
    num: u16,
) -> Option<([f32; 3], [f32; 3])> {
    let entity = slots?.object_at_index(num)?;
    let time = sample_time_ms(Some(clock), presented)?;
    pose_states
        .get(entity)
        .ok()
        .map(|runtime| sample(runtime, time))
}

/// Puts each mover's model where its centity is, and notes the angles for
/// the scene submission.
pub(super) fn apply_scripted_mover_centity_pose(
    slots: Option<Res<net::EntityTableCells>>,
    presented: Option<Res<net::ShownWorldFrame>>,
    pose_states: Query<&net::ClientEntityPoseState>,
    mut owners_in: Query<(&mut Transform, &LevelScriptModelInstance)>,
    mut persist: ResMut<ScriptPropSkeletons>,
    clock: Option<Res<net::FrameClock>>,
) {
    let _slow = SlowGuard::new("apply_script_mover_centity_pose");
    persist.mover_angles.clear();
    let (Some(slots), Some(presented)) = (slots, presented) else {
        return;
    };
    let Some(time) = sample_time_ms(clock.as_deref(), &presented) else {
        return;
    };
    for (mut xform, instance) in &mut owners_in {
        let found = instance.game_actor_number.and_then(|number| {
            let entity = slots.object_at_index(number)?;
            pose_states.get(entity).ok()
        });
        let Some(runtime) = found else {
            continue;
        };
        let (spot, angles) = sample(runtime, time);
        xform.translation = Vec3::from_array(spot);
        xform.rotation = rotation_from_angles(angles);
        let ordinal = instance.id.origin_sequence_number();
        persist.mover_angles.insert(ordinal, angles);
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn yaw_turns_a_model_about_up() {
        let facing = rotation_from_angles([0.0, 90.0, 0.0]) * Vec3::X;
        assert!((facing - Vec3::Y).length() < 1e-5, "{facing:?}");
    }

    #[test]
    fn pitch_tips_the_nose_and_roll_leaves_it() {
        let nose = rotation_from_angles([90.0, 0.0, 0.0]) * Vec3::X;
        assert!((nose - Vec3::NEG_Z).length() < 1e-5, "{nose:?}");
        let rolled = rotation_from_angles([0.0, 0.0, 90.0]) * Vec3::X;
        assert!((rolled - Vec3::X).length() < 1e-5, "{rolled:?}");
    }

    #[test]
    fn no_angles_is_no_turn() {
        assert!(rotation_from_angles([0.0; 3]).abs_diff_eq(Quat::IDENTITY, 1e-6));
    }
}
