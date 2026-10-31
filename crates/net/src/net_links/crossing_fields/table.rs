//! The player-state field table: every field the wire carries, in wire order, and the four
//! operations that act on one field by its position in the table.
//!
//! The order is the wire format. Adding a field anywhere but the end breaks every peer.

use playerstate_t6::game::AvatarState;

use super::value::FieldValue;
use super::{PlayerStateWireField, SyncPolicy, FieldCheck};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

macro_rules! validation_of {
    (exact) => {
        FieldCheck::Exact
    };
    (adopt($reason:literal)) => {
        FieldCheck::InstallWithoutCompare($reason)
    };
}

/// Builds [`PLAYER_STATE_WIRE_FIELDS`] and one dispatcher per operation from a single list of fields.
/// A row is `path = exact;` (a prediction must match it) or `path = adopt("why");` (the server's
/// value is taken as it is).
macro_rules! player_wire_fields {
    ($($rust:ident $(. $sub:ident)* $([$idx:literal])? = $how:ident $(($reason:literal))?;)*) => {
        pub const PLAYER_STATE_WIRE_FIELDS: &[PlayerStateWireField] = &[
            $(PlayerStateWireField {
                code_identifier: concat!(stringify!($rust) $(, ".", stringify!($sub))* $(, "[", stringify!($idx), "]")?),
                sync_rule: SyncPolicy::Synced,
                check: validation_of!($how $(($reason))?),
            },)*
        ];

        pub(crate) fn value_changed(a: &AvatarState, b: &AvatarState, wanted: usize) -> bool {
            let mut walk = 0usize;
            $(
                if walk == wanted {
                    return a.$rust $(.$sub)* $([$idx])?.changed_from(&b.$rust $(.$sub)* $([$idx])?);
                }
                walk += 1;
            )*
            let _ = walk;
            false
        }

        pub(crate) fn value_gap(a: &AvatarState, b: &AvatarState, wanted: usize) -> f32 {
            let mut walk = 0usize;
            $(
                if walk == wanted {
                    return a.$rust $(.$sub)* $([$idx])?.distance(&b.$rust $(.$sub)* $([$idx])?);
                }
                walk += 1;
            )*
            let _ = walk;
            0.0
        }

        pub(crate) fn store_value(out: &mut ByteSinkWriter, avatar: &AvatarState, wanted: usize) {
            let mut walk = 0usize;
            $(
                if walk == wanted {
                    avatar.$rust $(.$sub)* $([$idx])?.put(out);
                    return;
                }
                walk += 1;
            )*
            let _ = walk;
        }

        pub(crate) fn extract_value(
            input: &mut ByteCursorReader<'_>,
            avatar: &mut AvatarState,
            wanted: usize,
        ) -> Result<(), PacketDecodeFault> {
            let mut walk = 0usize;
            $(
                if walk == wanted {
                    return avatar.$rust $(.$sub)* $([$idx])?.get(input);
                }
                walk += 1;
            )*
            let _ = walk;
            Err(PacketDecodeFault::UnlistedField(wanted))
        }
    };
}

player_wire_fields! {
    last_cmd_ms = exact;
    movement_mode = exact;
    move_timer = exact;
    movement_state_bits = exact;
    secondary_state_bits = exact;
    attach_bits = exact;
    sway_phase = exact;
    origin = exact;
    velocity = exact;
    cook_remaining = exact;
    returned_grenade_owner = exact;
    gravity = exact;
    tilt_sideways = exact;
    speed = exact;
    view_offset_angles = exact;
    standing_on = exact;
    ladder_normal_dir = exact;
    hop_start_ms = exact;
    takeoff_height = exact;
    dive_end_time = exact;
    foliage_sound_time = exact;
    slide_time = exact;
    mover_entity_num = exact;
    mover_timestamp = exact;
    weapon_state_t6 = exact;
    weapon_state_left_t6 = exact;
    weap_anim_t6 = exact;
    weap_anim_left_t6 = exact;
    waterlevel = exact;
    prone_direction = exact;
    prone_direction_pitch = exact;
    prone_torso_pitch = exact;
    f_torso_pitch = exact;
    f_waist_pitch = exact;
    lower_body_clock = exact;
    legs_anim = exact;
    upper_body_clock = exact;
    torso_anim = exact;
    hurt_clock = exact;
    hurt_length = exact;
    flinch_yaw_anim = exact;
    body_index = exact;
    move_heading = exact;
    e_flags = exact;
    predictable_event_sequence = exact;
    predictable_events[0] = exact;
    predictable_events[1] = exact;
    predictable_events[2] = exact;
    predictable_events[3] = exact;
    predictable_event_parms[0] = exact;
    predictable_event_parms[1] = exact;
    predictable_event_parms[2] = exact;
    predictable_event_parms[3] = exact;
    predictable_event_sequence_old = exact;
    look_angles = exact;
    eye_height_goal = exact;
    eye_height_now = exact;
    eye_blend_clock = exact;
    eye_blend_goal = exact;
    eye_lower_blend = exact;
    damage_event = adopt(
        "authority damage feedback (`damage_event` is a counter the server bumps). Same argument as health"
    );
    damage_yaw = adopt(
        "direction of an incoming hit the client never saw fired; authority-only damage feedback"
    );
    damage_pitch = adopt(
        "direction of an incoming hit the client never saw fired; authority-only damage feedback"
    );
    hurt_amount = adopt(
        "magnitude of an incoming hit the client never saw fired; authority-only damage feedback"
    );
    health = adopt(
        "resolved from other clients' commands, which a predicting client does not have. A difference here is missing information, not a prediction error, and counting it as one would make every firefight look like a desync"
    );
    hit_point_ceiling = exact;
    link_weapon_angles = exact;
    hint_kind = adopt("authority-selected interaction target");
    hint_text = adopt("authority-selected interaction target");
    hint_entity = adopt("authority-selected interaction target");
    cursor_hint_dual_wield = adopt("authority-selected interaction target");
    sprint_state.sprint_needs_release = exact;
    sprint_state.sprint_wait = exact;
    sprint_state.sprint_begin_ms = exact;
    sprint_state.sprint_stop_ms = exact;
    sprint_state.sprint_launch_cap = exact;
    run_speed_modifier = exact;
    mantle_state.yaw = exact;
    mantle_state.timer = exact;
    mantle_state.trans_index = exact;
    mantle_state.flags = exact;
    weapon_anim_id = exact;
    weapon_timer = exact;
    weapon_ready_wait = exact;
    gun_lockout_kick_stamp = exact;
    weaponstate_primary = exact;
    weap_hand_flags = exact;
    shots_fired = exact;
    weap_anim_secondary = exact;
    weapon_time_secondary = exact;
    weapon_delay_secondary = exact;
    weapon_restrict_kick_time_secondary = exact;
    weaponstate_secondary = exact;
    weap_hand_flags_secondary = exact;
    weapon_shot_count_secondary = exact;
    weapons = exact;
    weapon_data = exact;
    secondary_hand_slot = exact;
    secondary_gear_main = exact;
    secondary_gear_alt = exact;
    weapon = exact;
    weapon_primary = exact;
    weapon_state_bits = exact;
    weapon_raise_fraction = exact;
    sight_spread_factor = exact;
    scope_in_wait = exact;
    spread_forced_value = exact;
    spread_forced_phase = exact;
    previous_gun_hand = exact;
    ammo = exact;
    magazine_rows = exact;
    lunge_heading = exact;
    lunge_reach = exact;
    lunge_clock = exact;
    perks = exact;
    loadout_perks = exact;
    dpad_kind = exact;
    action_slot_param = exact;
    inventory_weapon = exact;
    stun_effect_id = exact;
    stun_start = exact;
    stun_length = exact;
    objectives = exact;
    time_slice = exact;
    death_replay_target = exact;
    death_replay_focus = exact;
    death_replay_player = exact;
    recoil_scale = exact;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position_of(name: &str) -> usize {
        PLAYER_STATE_WIRE_FIELDS
            .iter()
            .position(|row| row.code_identifier == name)
            .unwrap_or_else(|| panic!("no field named {name}"))
    }

    #[test]
    fn nested_and_indexed_fields_are_named_by_their_path() {
        position_of("origin");
        position_of("sprint_state.sprint_wait");
        position_of("mantle_state.yaw");
        position_of("predictable_events[3]");
        position_of("predictable_event_parms[0]");
    }

    #[test]
    fn field_names_are_unique() {
        let mut names: Vec<_> = PLAYER_STATE_WIRE_FIELDS.iter().map(|row| row.code_identifier).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }

    #[test]
    fn the_first_and_last_fields_hold_their_wire_positions() {
        assert_eq!(PLAYER_STATE_WIRE_FIELDS.first().map(|row| row.code_identifier), Some("last_cmd_ms"));
        assert_eq!(PLAYER_STATE_WIRE_FIELDS.last().map(|row| row.code_identifier), Some("recoil_scale"));
    }

    #[test]
    fn each_operation_acts_on_the_field_at_its_index() {
        let mut a = AvatarState::ZERO;
        let b = AvatarState::ZERO;
        a.mantle_state.timer = 40;
        let index = position_of("mantle_state.timer");
        assert!(value_changed(&a, &b, index));
        assert_eq!(value_gap(&a, &b, index), 40.0);
        assert!(!value_changed(&a, &b, position_of("mantle_state.flags")));

        let mut out = ByteSinkWriter::new();
        store_value(&mut out, &a, index);
        let bytes = out.into_bytes();
        let mut back = AvatarState::ZERO;
        extract_value(&mut ByteCursorReader::new(&bytes), &mut back, index).unwrap();
        assert_eq!(back.mantle_state.timer, 40);
    }

    #[test]
    fn an_index_past_the_table_reads_as_unknown_and_changes_nothing() {
        let a = AvatarState::ZERO;
        let mut b = AvatarState::ZERO;
        let past = PLAYER_STATE_WIRE_FIELDS.len();
        assert!(!value_changed(&a, &b, past));
        assert_eq!(value_gap(&a, &b, past), 0.0);
        let mut out = ByteSinkWriter::new();
        store_value(&mut out, &a, past);
        assert!(out.into_bytes().is_empty());
        assert_eq!(
            extract_value(&mut ByteCursorReader::new(&[0; 8]), &mut b, past),
            Err(PacketDecodeFault::UnlistedField(past))
        );
    }

    #[test]
    fn every_field_survives_the_wire() {
        let mut ps = AvatarState::ZERO;
        ps.last_cmd_ms = 123_456;
        ps.origin = [1.5, -2.5, 3.5];
        ps.predictable_events = [1, 2, 3, 4];
        ps.weapons[7] = 99;
        ps.ammo[5] = 6;
        ps.objectives[0x37f] = 8;
        ps.loadout_perks[7] = 0xdead_beef;
        ps.predictable_event_sequence = -5;
        let mut out = ByteSinkWriter::new();
        for index in 0..PLAYER_STATE_WIRE_FIELDS.len() {
            store_value(&mut out, &ps, index);
        }
        let bytes = out.into_bytes();
        let mut reader = ByteCursorReader::new(&bytes);
        let mut back = AvatarState::ZERO;
        for index in 0..PLAYER_STATE_WIRE_FIELDS.len() {
            extract_value(&mut reader, &mut back, index).unwrap();
        }
        for index in 0..PLAYER_STATE_WIRE_FIELDS.len() {
            assert!(
                !value_changed(&ps, &back, index),
                "{} did not survive",
                PLAYER_STATE_WIRE_FIELDS[index].code_identifier
            );
        }
    }
}
