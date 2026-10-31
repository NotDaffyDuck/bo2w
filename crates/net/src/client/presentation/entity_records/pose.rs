//! An entity's runtime record and the steps that carry it from one snapshot to the next.

use entity_t6::{
    EntitySnapshot, Trajectory, et,
    evaluate_trajectory, flight,
};

/// The entity-flag bit that flips when an entity jumps to a new place instead of moving there.
pub const SNAP_MOVE_FLAG: i32 = 2;
/// The entity-flag bit marking a player as dead.
pub const DEAD_ENTITY_FLAG: i32 = playerstate_t6::game::entity_flag_bits::DEAD as i32;

/// Bit of [`ClientEntityPoseState::pose_now_flag`] set while the entity is in the snapshot just taken.
pub(super) const CURRENT_VALID_IN_NEXT: u8 = 0x1;

/// What an entity's flags and motion were at the pose time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendedPoseNow {
    pub flag_bits: u32,
    pub path: Trajectory,
    pub spin: Trajectory,
}

/// A handle to an effect attached to the entity. None is held yet; the type marks the place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntityEffectHandle {
    #[default]
    None,
}

/// A handle to the entity's drawable object. None is held yet; the type marks the place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntityModelHandle {
    #[default]
    None,
}

#[derive(bevy::prelude::Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientEntityPoseState {
    /// The state from the latest snapshot.
    pub upcoming_condition: EntitySnapshot,
    /// The state and time a remote player was last drawn from.
    pub shown_participant: Option<(EntitySnapshot, i32)>,
    pub current: BlendedPoseNow,
    pub pose_now_flag: u8,
    pub prior_occurrence_number: i32,
    /// The entity type the pose was sampled as.
    pub pose_entity_kind: u8,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// The time the pose was sampled at, milliseconds.
    pub pose_clock_millis: i32,
    /// The pose before the latest one, with its time, to blend from.
    pub prior_shown: Option<(BlendedPoseNow, i32)>,
    pub fx_handle: EntityEffectHandle,
    pub dobj_handle: EntityModelHandle,
}

impl ClientEntityPoseState {
    pub fn present_in_newer_frame(&self) -> bool {
        (self.pose_now_flag & CURRENT_VALID_IN_NEXT) == CURRENT_VALID_IN_NEXT
    }

    /// The point the entity's motion reaches at the pose time.
    pub fn installed_position(&self) -> [f32; 3] {
        evaluate_trajectory(&self.current.path, self.pose_clock_millis)
    }

    /// For a moving player or corpse, writes into `origin` and `angles` the pose that lies
    /// `for_instant_millis` along the way from the previous pose to the current one. Other entities, and
    /// ones that lack two interpolated poses, keep what was sampled.
    pub fn settle_pose(&mut self, for_instant_millis: i32) {
        let (before, before_ms) = match self.prior_shown {
            Some(pair) if self.present_in_newer_frame() => pair,
            _ => return,
        };
        let body = [et::PLAYER, et::PLAYER_CORPSE].contains(&i16::from(self.pose_entity_kind));
        let both_interpolated = [before.path.tr_type, self.current.path.tr_type]
            .iter()
            .all(|kind| *kind == flight::path_kind::BLEND);
        let window = self.pose_clock_millis - before_ms;
        if !(body && both_interpolated && window > 0) {
            return;
        }
        let along = ((for_instant_millis - before_ms) as f32 / window as f32).clamp(0.0, 1.0);
        let sample = |pos: &Trajectory, apos: &Trajectory, ms: i32| {
            (evaluate_trajectory(pos, ms), evaluate_trajectory(apos, ms))
        };
        let (from_pos, from_ang) = sample(&before.path, &before.spin, before_ms);
        let (to_pos, to_ang) = sample(&self.current.path, &self.current.spin, self.pose_clock_millis);
        self.origin = std::array::from_fn(|i| from_pos[i] + (to_pos[i] - from_pos[i]) * along);
        self.angles = std::array::from_fn(|i| {
            from_ang[i] + game_math::degree_gap(to_ang[i], from_ang[i]) * along
        });
    }

    /// Adopts the motion and type of the latest state without sampling a new pose.
    pub fn blend_copy_crossfade(&mut self) {
        let latest = &self.upcoming_condition;
        self.pose_entity_kind = u8::try_from(latest.e_type).unwrap_or_default();
        self.current = lerp_from_entity_state(latest);
    }
}

fn lerp_from_entity_state(state: &EntitySnapshot) -> BlendedPoseNow {
    let lerp = &state.lerp;
    BlendedPoseNow {
        spin: lerp.apos,
        path: lerp.pos,
        flag_bits: lerp.e_flags as u32,
    }
}

/// The entity has left the snapshot: drops its pose and handles.
pub fn tear_down_object(runtime: &mut ClientEntityPoseState) {
    runtime.pose_now_flag &= !CURRENT_VALID_IN_NEXT;
    runtime.prior_shown = None;
    runtime.pose_entity_kind = 0;
    runtime.dobj_handle = EntityModelHandle::None;
    runtime.fx_handle = EntityEffectHandle::None;
}

/// Starts the entity over from `next`, sampled at `for_instant_millis`. With `fresh_slot` the slot now
/// holds a different entity, so its event history starts over too.
pub fn restart_body(
    runtime: &mut ClientEntityPoseState,
    incoming: EntitySnapshot,
    for_instant_millis: i32,
    fresh_slot: bool,
) {
    tear_down_object(runtime);
    runtime.current = lerp_from_entity_state(&incoming);
    runtime.pose_entity_kind = u8::try_from(incoming.e_type).unwrap_or_default();
    runtime.upcoming_condition = incoming;
    sample_pose(runtime, for_instant_millis);
    reset_event_cursor(runtime, &incoming, fresh_slot);
    runtime.pose_now_flag |= CURRENT_VALID_IN_NEXT;
}

fn sample_pose(runtime: &mut ClientEntityPoseState, for_instant_millis: i32) {
    let BlendedPoseNow { path: pos, spin: apos, .. } = runtime.current;
    runtime.pose_clock_millis = for_instant_millis;
    runtime.angles = evaluate_trajectory(&apos, for_instant_millis);
    runtime.origin = evaluate_trajectory(&pos, for_instant_millis);
}

/// Picks where the entity's event ring is read from next. Types 8 and 9 stay where they are;
/// types 0 and 4 start over only for a new entity; every other type (corpses too) skips the
/// events the state already carries.
fn reset_event_cursor(runtime: &mut ClientEntityPoseState, incoming: &EntitySnapshot, fresh_slot: bool) {
    let carried = i32::from(incoming.event_sequence);
    let target = match incoming.e_type {
        8 | 9 => None,
        0 | 4 => fresh_slot.then_some(0),
        _ => Some(carried),
    };
    if let Some(sequence) = target {
        runtime.prior_occurrence_number = sequence;
    }
}

/// Applies a new snapshot's state to an entity that is already known. A newcomer, a change of
/// type or a teleport starts it over; otherwise the pose taken last becomes the one to blend from
/// (provided time moved forward) and the new state is sampled.
pub fn refresh_known(runtime: &mut ClientEntityPoseState, incoming: EntitySnapshot, for_instant_millis: i32) {
    let newcomer = !runtime.present_in_newer_frame();
    let retyped = incoming.e_type != runtime.upcoming_condition.e_type;
    let jumped = ((incoming.lerp.e_flags ^ runtime.upcoming_condition.lerp.e_flags) & SNAP_MOVE_FLAG) != 0;
    if newcomer || retyped || jumped {
        return restart_body(runtime, incoming, for_instant_millis, newcomer || retyped);
    }
    runtime.prior_shown = match for_instant_millis.cmp(&runtime.pose_clock_millis) {
        std::cmp::Ordering::Greater => Some((runtime.current, runtime.pose_clock_millis)),
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => runtime.prior_shown,
    };
    runtime.current = lerp_from_entity_state(&incoming);
    runtime.upcoming_condition = incoming;
    sample_pose(runtime, for_instant_millis);
    runtime.pose_now_flag |= CURRENT_VALID_IN_NEXT;
}

#[cfg(test)]
mod tests {
    use entity_t6::LerpEntityState;

    use super::*;

    /// A player at `at` facing `facing`, moving by interpolation.
    fn on_foot(at: [f32; 3], facing: f32) -> EntitySnapshot {
        let moving = |tr_base| Trajectory {
            tr_type: flight::path_kind::BLEND,
            base_point: tr_base,
            ..Trajectory::default()
        };
        EntitySnapshot {
            e_type: et::PLAYER,
            lerp: LerpEntityState {
                pos: moving(at),
                apos: moving([0.0, facing, 0.0]),
                ..LerpEntityState::default()
            },
            ..EntitySnapshot::default()
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-3)
    }

    /// The first state resets a fresh runtime; every later one is applied on top of it.
    fn after_updates(updates: &[(EntitySnapshot, i32)]) -> ClientEntityPoseState {
        let mut rt = ClientEntityPoseState::default();
        let (first, rest) = updates.split_first().expect("at least one update");
        restart_body(&mut rt, first.0, first.1, true);
        for &(state, clock_stamp_ms) in rest {
            refresh_known(&mut rt, state, clock_stamp_ms);
        }
        rt
    }

    /// Where the event cursor ends up when a runtime that was at 99 is reset onto a state that
    /// carries sequence 5.
    fn cursor_after_reset(e_type: i16, e_flags: i32, fresh_slot: bool) -> i32 {
        let state = EntitySnapshot {
            e_type,
            event_sequence: 5,
            lerp: LerpEntityState {
                e_flags,
                ..LerpEntityState::default()
            },
            ..EntitySnapshot::default()
        };
        let mut rt = ClientEntityPoseState {
            prior_occurrence_number: 99,
            ..Default::default()
        };
        restart_body(&mut rt, state, 0, fresh_slot);
        rt.prior_occurrence_number
    }

    #[test]
    fn a_reset_entity_is_present_and_sampled_where_it_stands() {
        let mut rt = ClientEntityPoseState::default();
        assert!(!rt.present_in_newer_frame());
        let state = EntitySnapshot {
            event_sequence: 7,
            ..on_foot([1.0, 2.0, 3.0], 90.0)
        };
        restart_body(&mut rt, state, 500, true);
        assert!(rt.present_in_newer_frame());
        assert_eq!(rt.pose_clock_millis, 500);
        assert_eq!(rt.pose_entity_kind, 1);
        assert_eq!(rt.prior_occurrence_number, 7);
        assert!(close(rt.origin, [1.0, 2.0, 3.0]));
        assert!(close(rt.angles, [0.0, 90.0, 0.0]));
        assert!(close(rt.installed_position(), rt.origin));
    }

    #[test]
    fn a_departed_entity_loses_its_pose_but_not_its_last_state() {
        let mut rt = after_updates(&[(on_foot([4.0; 3], 0.0), 100)]);
        rt.prior_shown = Some((rt.current, 50));
        tear_down_object(&mut rt);
        assert_eq!(rt.upcoming_condition.e_type, et::PLAYER);
        assert!(!rt.present_in_newer_frame());
        assert!(rt.prior_shown.is_none());
        assert_eq!(rt.pose_entity_kind, 0);
    }

    #[test]
    fn the_event_cursor_follows_the_entity_type() {
        // Corpses skip what the state holds, even when the player was aiming: that flag shares
        // its bit with an old clone-tree marker.
        let aiming = playerstate_t6::flags::e_flags::AIM_BIT;
        let expected = [
            (8, 0, true, 99),
            (9, 0, false, 99),
            (0, 0, true, 0),
            (4, 0, false, 99),
            (2, aiming, false, 5),
            (2, 0, false, 5),
            (1, 0, true, 5),
        ];
        for (e_type, flags, fresh_slot, cursor) in expected {
            assert_eq!(
                cursor_after_reset(e_type, flags, fresh_slot),
                cursor,
                "type {e_type}"
            );
        }
    }

    #[test]
    fn an_update_keeps_the_pose_it_replaces_while_time_moves_on() {
        let mut rt = after_updates(&[
            (on_foot([0.0; 3], 10.0), 100),
            (on_foot([10.0, 0.0, 0.0], 30.0), 200),
        ]);
        let (older, older_ms) = rt.prior_shown.expect("a pose to blend from");
        assert!(close(older.path.base_point, [0.0; 3]));
        assert_eq!([older_ms, rt.pose_clock_millis], [100, 200]);

        // The same time keeps the remembered pose; an earlier time drops it.
        refresh_known(&mut rt, on_foot([20.0, 0.0, 0.0], 30.0), 200);
        assert!(rt.prior_shown.is_some());
        refresh_known(&mut rt, on_foot([30.0, 0.0, 0.0], 30.0), 150);
        assert!(rt.prior_shown.is_none());
    }

    #[test]
    fn a_teleport_or_a_new_type_restarts_the_entity() {
        let mut rt = after_updates(&[
            (on_foot([0.0; 3], 0.0), 100),
            (on_foot([1.0, 0.0, 0.0], 0.0), 150),
        ]);
        assert!(rt.prior_shown.is_some());

        let mut jump = on_foot([500.0, 0.0, 0.0], 0.0);
        jump.lerp.e_flags ^= SNAP_MOVE_FLAG;
        refresh_known(&mut rt, jump, 200);
        assert!(rt.prior_shown.is_none());
        assert!(close(rt.origin, [500.0, 0.0, 0.0]));

        let retyped = EntitySnapshot { e_type: 3, ..jump };
        refresh_known(&mut rt, retyped, 250);
        assert_eq!(rt.pose_entity_kind, 3);
        assert!(rt.prior_shown.is_none());
    }

    #[test]
    fn presenting_blends_inside_the_window_and_clamps_outside() {
        let mut rt = after_updates(&[
            (on_foot([0.0; 3], 10.0), 100),
            (on_foot([10.0, 0.0, 0.0], 30.0), 200),
        ]);
        // (time, where the origin should be, where the angles should be if that is checked)
        let probes = [
            (900, [10.0, 0.0, 0.0], None),
            (150, [5.0, 0.0, 0.0], Some([0.0, 20.0, 0.0])),
            (0, [0.0; 3], None),
        ];
        for (time, origin, angles) in probes {
            rt.settle_pose(time);
            assert!(close(rt.origin, origin), "origin at {time}");
            if let Some(angles) = angles {
                assert!(close(rt.angles, angles), "angles at {time}");
            }
        }
    }

    #[test]
    fn presenting_leaves_other_entities_as_sampled() {
        let blended = after_updates(&[
            (on_foot([0.0; 3], 0.0), 100),
            (on_foot([10.0, 0.0, 0.0], 0.0), 200),
        ]);
        let sampled = blended.origin;

        let other_kind = ClientEntityPoseState {
            pose_entity_kind: 3,
            ..blended
        };
        let gone = ClientEntityPoseState {
            pose_now_flag: blended.pose_now_flag & !CURRENT_VALID_IN_NEXT,
            ..blended
        };
        let single = after_updates(&[(on_foot([8.0; 3], 0.0), 100)]);
        for (mut rt, held) in [(other_kind, sampled), (gone, sampled), (single, [8.0; 3])] {
            rt.settle_pose(150);
            assert!(close(rt.origin, held));
        }
    }

    #[test]
    fn copying_the_lerp_takes_motion_and_type_but_not_presence() {
        let mut latest = on_foot([6.0; 3], 0.0);
        latest.lerp.e_flags = 4;
        let mut rt = ClientEntityPoseState {
            upcoming_condition: latest,
            ..Default::default()
        };
        rt.blend_copy_crossfade();
        assert!(!rt.present_in_newer_frame());
        assert_eq!((rt.pose_entity_kind, rt.current.flag_bits), (1, 4));
        assert!(close(rt.current.path.base_point, [6.0; 3]));
    }
}
