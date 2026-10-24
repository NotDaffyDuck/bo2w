//! Where a script mover stands this frame, read from its centity.

use bevy::prelude::*;
use render_scene::WorldScriptModelInstance;

use super::state::ScriptModelDobjs;
use crate::occupancy::t6_body::SlowGuard;

/// The time at which movers are sampled: the client's clock once it runs,
/// else the snapshot's own, moved onto the trajectory timeline.
fn sample_time_ms(clock: Option<&net::FrameClock>, presented: &net::PresentedSnapshot) -> Option<i32> {
    let snapshot = presented.snapshot()?;
    let time = match clock {
        Some(clock) if clock.started() => clock.time(),
        _ => sim::level_time_ms(snapshot.tick),
    };
    Some(presented.trajectory_time_ms(time))
}

fn sample(runtime: &net::CEntityRuntime, time_ms: i32) -> ([f32; 3], [f32; 3]) {
    (
        entity_t6::evaluate_trajectory(&runtime.current.pos, time_ms),
        entity_t6::evaluate_trajectory(&runtime.current.apos, time_ms),
    )
}

/// The rotation of a model facing `[pitch, yaw, roll]` degrees.
fn rotation_from_angles([pitch, yaw, roll]: [f32; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::ZYX,
        yaw.to_radians(),
        pitch.to_radians(),
        roll.to_radians(),
    )
}

/// A script mover's place and angles this frame, sampled as
/// `apply_script_mover_centity_pose` places its model (a camera that rides
/// the model reads this, not last frame's Transform).
pub(crate) fn script_mover_pose_now(
    slots: Option<&net::CEntitySlots>,
    presented: &net::PresentedSnapshot,
    runtimes: &Query<&net::CEntityRuntime>,
    clock: &net::FrameClock,
    number: u16,
) -> Option<([f32; 3], [f32; 3])> {
    let time = sample_time_ms(Some(clock), presented)?;
    let runtime = runtimes.get(slots?.entity_for_number(number)?).ok()?;
    Some(sample(runtime, time))
}

/// Puts each mover's model where its centity is, and notes the angles for
/// the scene submission.
pub(super) fn apply_script_mover_centity_pose(
    slots: Option<Res<net::CEntitySlots>>,
    presented: Option<Res<net::PresentedSnapshot>>,
    runtimes: Query<&net::CEntityRuntime>,
    mut owners: Query<(&WorldScriptModelInstance, &mut Transform)>,
    mut persist: ResMut<ScriptModelDobjs>,
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
    for (instance, mut transform) in owners.iter_mut() {
        let Some(number) = instance.gentity_number else {
            continue;
        };
        let Some(runtime) = slots
            .entity_for_number(number)
            .and_then(|entity| runtimes.get(entity).ok())
        else {
            continue;
        };
        let (origin, angles) = sample(runtime, time);
        transform.rotation = rotation_from_angles(angles);
        transform.translation = Vec3::from_array(origin);
        persist
            .mover_angles
            .insert(instance.id.source_ordinal(), angles);
    }
}

#[cfg(test)]
mod tests {
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
