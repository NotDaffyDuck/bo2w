//! Turns the server's player states and corpse slots into entity states, and reads back the pose
//! a remote player's animation is driven from.

use entity_t6::{
    EntitySnapshot, LerpEntityState, PlayerAnimState, Trajectory, et, flight,
};
use playerstate_t6::Weapon;
use playerstate_t6::game::{BodyAnims, MOVE_KIND_DEAD, AvatarState, entity_flag_bits};
use sim::{ClientId, PlayerCorpseSlot};

use super::pose::{ClientEntityPoseState, DEAD_ENTITY_FLAG};

/// A trajectory that follows whatever the snapshots say, resting at `base`.
fn stationary_path(rest: [f32; 3]) -> Trajectory {
    Trajectory {
        tr_type: flight::path_kind::BLEND,
        base_point: rest,
        ..Trajectory::default()
    }
}

fn anim_of(lower: i32, upper: i32) -> PlayerAnimState {
    PlayerAnimState {
        legs_clip: lower,
        torso_clip: upper,
        ..PlayerAnimState::default()
    }
}

/// Builds the entity state that `peer`'s player slot holds out of the server's player state.
pub fn avatar_entity_row(peer: ClientId, avatar: &AvatarState) -> EntitySnapshot {
    let slot = i32::try_from(peer.0).unwrap_or_default();
    let death_bit = if avatar.movement_mode >= MOVE_KIND_DEAD {
        DEAD_ENTITY_FLAG
    } else {
        0
    };
    let mut built = EntitySnapshot {
        slot_index: slot,
        e_type: et::PLAYER,
        client_num: slot as u8,
        ground_entity_num: avatar.standing_on as i16,
        event_sequence: avatar.predictable_event_sequence,
        events: avatar.predictable_events.map(|event| event as u8),
        event_parms: avatar.predictable_event_parms,
        weapon: Weapon(avatar.weapon),
        lerp: LerpEntityState {
            e_flags: avatar.e_flags as i32 | death_bit,
            pos: stationary_path(avatar.origin),
            apos: stationary_path(avatar.look_angles),
            ..LerpEntityState::default()
        },
        ..EntitySnapshot::default()
    };
    built
        .un2
        .set_player_anim(&anim_of(avatar.legs_anim, avatar.torso_anim));
    built
}

/// Builds the entity state a corpse slot holds out of the server's corpse record.
pub fn cadaver_slot_to_object_state(slot: &PlayerCorpseSlot) -> EntitySnapshot {
    let motion = Trajectory {
        tr_type: slot.tr_type,
        began_at: slot.tr_time,
        span_ms: slot.tr_duration,
        rate: slot.tr_delta,
        base_point: slot.tr_base,
    };
    let mut built = EntitySnapshot {
        slot_index: slot.entnum,
        e_type: et::PLAYER_CORPSE,
        client_num: slot.victim.0 as u8,
        weapon: Weapon(slot.weapon),
        ground_entity_num: slot.ground_entity_num as i16,
        lerp: LerpEntityState {
            e_flags: slot.e_flags as i32,
            pos: motion,
            apos: stationary_path(slot.viewangles),
            ..LerpEntityState::default()
        },
        ..EntitySnapshot::default()
    };
    built
        .un2
        .set_player_anim(&anim_of(slot.anim.legs, slot.anim.torso));
    built
}

/// The inputs a remote player's animation is worked out from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtherPlayerPoseReading {
    pub gait: BodyAnims,
    /// Where, and when, the player's speed is measured from.
    pub pacing_anchor_pos: [f32; 3],
    pub pacing_clock_millis: i32,
    pub weapon: u32,
    pub look_pitch_degrees: f32,
    pub lying: bool,
    pub kneeling: bool,
}

/// Samples a remote player: from the state last drawn when there is one, otherwise from the
/// newest state at the moment its pose was taken.
pub fn other_player_pose(runtime: &ClientEntityPoseState) -> OtherPlayerPoseReading {
    let drawn = runtime
        .shown_participant
        .as_ref()
        .map(|(shown, stamp)| (shown, shown.lerp.pos.base_point, *stamp));
    let (state, pacing_anchor_pos, pacing_clock_millis) = drawn.unwrap_or((
        &runtime.upcoming_condition,
        runtime.installed_position(),
        runtime.pose_clock_millis,
    ));
    let has = |bit: u32| state.lerp.e_flags & bit as i32 != 0;
    let PlayerAnimState {
        legs_clip: lower,
        torso_clip: upper,
        ..
    } = state.un2.player_anim();
    OtherPlayerPoseReading {
        gait: BodyAnims {
            legs: lower,
            torso: upper,
        },
        pacing_anchor_pos,
        pacing_clock_millis,
        weapon: state.weapon.0,
        look_pitch_degrees: runtime.angles[0],
        lying: has(entity_flag_bits::LYING_FLAT as u32),
        kneeling: has(entity_flag_bits::CROUCH_BIT as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::super::pose::restart_body;
    use super::*;

    #[test]
    fn player_state_conversion_lands_on_the_clients_own_slot() {
        let avatar = AvatarState {
            origin: [10.0, 20.0, 30.0],
            look_angles: [5.0, 90.0, 0.0],
            weapon: 12,
            e_flags: entity_flag_bits::LYING_FLAT as _,
            ..AvatarState::ZERO
        };
        let built = avatar_entity_row(ClientId(5), &avatar);
        assert_eq!(
            (built.slot_index, built.client_num, built.e_type),
            (5, 5, et::PLAYER)
        );
        assert_eq!(built.lerp.pos, stationary_path([10.0, 20.0, 30.0]));
        assert_eq!(built.lerp.apos, stationary_path([5.0, 90.0, 0.0]));
        assert_eq!(built.weapon, Weapon(12));
        assert!(built.lerp.e_flags & DEAD_ENTITY_FLAG == 0);
        assert!(built.lerp.e_flags & entity_flag_bits::LYING_FLAT as i32 != 0);
    }

    #[test]
    fn a_dead_pm_type_sets_the_dead_flag() {
        let avatar = AvatarState {
            movement_mode: MOVE_KIND_DEAD,
            ..AvatarState::ZERO
        };
        let built = avatar_entity_row(ClientId(0), &avatar);
        assert!(built.lerp.e_flags & DEAD_ENTITY_FLAG != 0);
    }

    fn standing(legs: i32, torso: i32) -> EntitySnapshot {
        let mut built = EntitySnapshot {
            e_type: et::PLAYER,
            weapon: Weapon(9),
            ..EntitySnapshot::default()
        };
        built.lerp.pos = stationary_path([1.0, 2.0, 3.0]);
        built.un2.set_player_anim(&anim_of(legs, torso));
        built
    }

    fn reset_at(state: EntitySnapshot, clock_stamp_ms: i32) -> ClientEntityPoseState {
        let mut rt = ClientEntityPoseState::default();
        restart_body(&mut rt, state, clock_stamp_ms, true);
        rt
    }

    #[test]
    fn with_nothing_drawn_the_newest_state_is_sampled() {
        let mut state = standing(3, 4);
        state.lerp.e_flags = entity_flag_bits::CROUCH_BIT as i32;
        let mut rt = reset_at(state, 640);
        rt.angles[0] = 12.5;
        let got = other_player_pose(&rt);
        assert_eq!((got.pacing_anchor_pos, got.pacing_clock_millis), ([1.0, 2.0, 3.0], 640));
        assert_eq!((got.weapon, got.look_pitch_degrees), (9, 12.5));
        assert_eq!((got.gait.legs, got.gait.torso), (3, 4));
        assert_eq!((got.kneeling, got.lying), (true, false));
    }

    #[test]
    fn a_drawn_state_wins_over_the_newest_one() {
        let mut rt = reset_at(standing(3, 4), 640);
        let mut earlier = standing(7, 8);
        earlier.lerp.pos.base_point = [9.0; 3];
        earlier.lerp.e_flags = entity_flag_bits::LYING_FLAT as i32;
        rt.shown_participant = Some((earlier, 600));
        let got = other_player_pose(&rt);
        assert_eq!((got.pacing_anchor_pos, got.pacing_clock_millis), ([9.0; 3], 600));
        assert_eq!((got.gait.legs, got.gait.torso), (7, 8));
        assert_eq!((got.lying, got.kneeling), (true, false));
    }

    #[test]
    fn corpse_conversion_copies_the_motion_and_the_victim() {
        let slot = PlayerCorpseSlot {
            entnum: 1100,
            victim: ClientId(6),
            tr_type: 2,
            tr_time: 30,
            tr_duration: 40,
            tr_delta: [1.0, 2.0, 3.0],
            tr_base: [4.0, 5.0, 6.0],
            viewangles: [0.0, 45.0, 0.0],
            weapon: 7,
            ..PlayerCorpseSlot::default()
        };
        let built = cadaver_slot_to_object_state(&slot);
        assert_eq!(
            (built.slot_index, built.e_type, built.client_num),
            (1100, et::PLAYER_CORPSE, 6)
        );
        assert_eq!(built.lerp.pos.rate, [1.0, 2.0, 3.0]);
        assert_eq!(built.lerp.pos.base_point, [4.0, 5.0, 6.0]);
        assert_eq!(built.lerp.apos, stationary_path([0.0, 45.0, 0.0]));
        assert_eq!(built.weapon, Weapon(7));
    }
}
