//! What reaches the second PC: each part of the snapshot message is written, read back and
//! compared, and malformed bytes are refused the way the protocol says.

use gamemode_t6::{RulesetKind, WarmupStage};
use movement_t6::bridge::Bo2Feel;
use sim::{
    ClientId, ClientSnapshotMeta, DebrisRecord, EntityEventPayload, EntityEventRecord,
    EventRecipients, EventSequence, GameRoundStage, SnapshotMeta, T6ViewInputs, Tick,
};

use super::codec::Wire;
use super::feel::{put_feel, take_feel};
use super::*;
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

fn bytes_of<T: Wire>(value: &T) -> Vec<u8> {
    let mut wr = ByteSinkWriter::new();
    value.put(&mut wr);
    wr.into_bytes()
}

fn read_back<T: Wire>(bytes: &[u8]) -> Result<T, PacketDecodeFault> {
    T::take(&mut ByteCursorReader::new(bytes))
}

/// A snapshot message for `meta`, with the world-object bytes the host would send beside it.
fn snapshot_bytes(meta: &SnapshotMeta) -> Vec<u8> {
    let world = pack_scene_prop_resync(
        &mut WorldPropStreamEncoder::default(),
        Tick(1),
        &meta.world_objects,
    );
    let mut wr = ByteSinkWriter::new();
    encode_snapshot_meta(&mut wr, meta, &world);
    wr.into_bytes()
}

fn read_snapshot(bytes: &[u8]) -> Result<SnapshotMeta, PacketDecodeFault> {
    let mut decoder = WorldPropStreamDecoder::default();
    parse_state_header(&mut ByteCursorReader::new(bytes), &mut decoder).map(|(meta, _)| meta)
}

fn through_the_wire(meta: &SnapshotMeta) -> SnapshotMeta {
    read_snapshot(&snapshot_bytes(meta)).unwrap()
}

#[test]
fn every_event_kind_of_the_game_reaches_the_client() {
    for event in [
        entity_t6::EventCode::BOLT_IMPACT,
        entity_t6::EventCode::MISSILE_REMOTE_BOOST,
        entity_t6::EventCode::EARTHQUAKE,
        entity_t6::EventCode::PLAY_RUMBLE_ON_POS,
        entity_t6::EventCode::STOP_ALL_RUMBLES,
        entity_t6::EventCode::SENSOR_LASER,
        entity_t6::EventCode::FOOTSTEP_CROUCHRUN,
        entity_t6::EventCode::SLIDE_START,
        entity_t6::EventCode::SLIDE_STOP,
    ] {
        let record = EntityEventRecord {
            sequence: EventSequence(3),
            tick: Tick(9),
            audience: EventRecipients::All,
            event,
            payload: EntityEventPayload {
                number: 40,
                weapon: 12,
                surf_type: 7,
                ..EntityEventPayload::default()
            },
        };
        let back: EntityEventRecord = read_back(&bytes_of(&record)).unwrap();
        assert_eq!(back, record);
    }
}

#[test]
fn an_event_kind_past_the_game_table_is_refused() {
    let record = EntityEventRecord {
        sequence: EventSequence(1),
        tick: Tick(1),
        audience: EventRecipients::All,
        event: entity_t6::EventCode(239),
        payload: EntityEventPayload::default(),
    };
    assert!(read_back::<EntityEventRecord>(&bytes_of(&record)).is_err());
}

#[test]
fn each_movement_lockout_switch_reaches_the_client_alone() {
    for which in 0..6 {
        let mut meta = ClientSnapshotMeta::default();
        let controls = &mut meta.controls;
        match which {
            0 => controls.prone_disabled = true,
            1 => controls.crouch_disabled = true,
            2 => controls.stand_disabled = true,
            3 => controls.sprint_disabled = true,
            4 => controls.melee_disabled = true,
            _ => controls.ads_disabled = true,
        }
        controls.stunned = true;
        controls.slick = true;
        controls.switch_to = 7;
        let back: ClientSnapshotMeta = read_back(&bytes_of(&meta)).unwrap();
        assert_eq!(back.controls, meta.controls, "switch {which}");
    }
}

#[test]
fn the_weapon_view_inputs_reach_the_client_or_stay_absent() {
    let view = T6ViewInputs {
        weap_anim: 0x400 | 37,
        weap_anim_left: 83,
        weaponstate: 11,
        weaponstate_left: 2,
        weapon_time: 1300,
        weapon_time_left: -16,
        intro_shots_fired: 3,
        ever_had: 0b101,
        needs_rechamber: 0b100,
    };
    for t6_view in [None, Some(view)] {
        let meta = ClientSnapshotMeta {
            t6_view,
            t6_perks: 1 << 22 | 1 << 21 | 1 << 16,
            ..ClientSnapshotMeta::default()
        };
        let back: ClientSnapshotMeta = read_back(&bytes_of(&meta)).unwrap();
        assert_eq!(back.t6_view, t6_view);
        assert_eq!(back.t6_perks, meta.t6_perks);
    }
}

/// Every field a different value, so a missed or swapped one shows.
fn distinct_feel(switches: u8) -> Bo2Feel {
    let on = |bit: u8| switches & (1 << bit) != 0;
    Bo2Feel {
        sprint_strafe_speed_scale: 1.5,
        jump_slowdown: on(1),
        fall_damage_min_height: 2.5,
        fall_damage_max_height: 3.5,
        zombies: on(2),
        gravity: 4,
        fall_damage_perk: 5,
        flak_jacket_perk: 6,
        fast_ads_perk: 7,
        fast_ads_multiplier: 8.5,
        longer_sprint_perk: 9,
        sprint_multiplier: 10.5,
        move_faster_perk: 11,
        speed_multiplier: 12.5,
        g_speed: 13.5,
        dive: on(3),
        sprint_cycle_scale: 14.5,
        ducked_sprint_cycle_scale: 15.5,
        dtp_cycle_scale: 16.5,
        omni: on(4),
        mantle_weapon_height: 17.5,
        mantle_weapon_anim_height: 18.5,
        leap_height: 19.5,
    }
}

#[test]
fn the_movement_rules_reach_the_client_whole() {
    for switches in [0u8, 0b1_1111, 0b0_0001, 0b0_0010, 0b0_0100, 0b0_1000, 0b1_0000] {
        let meta = SnapshotMeta { feel: Some(distinct_feel(switches)), ..SnapshotMeta::default() };
        assert_eq!(through_the_wire(&meta).feel, meta.feel, "switches {switches:#b}");
    }
}

#[test]
fn no_movement_rules_stay_none() {
    let mut wr = ByteSinkWriter::new();
    put_feel(&mut wr, &None);
    let bytes = wr.into_bytes();
    assert_eq!(bytes, [0]);
    assert_eq!(take_feel(&mut ByteCursorReader::new(&bytes)).unwrap(), None);
}

#[test]
fn a_broken_off_piece_reaches_the_client_whole() {
    let meta = SnapshotMeta {
        debris: vec![DebrisRecord {
            mesh: "nt_nuked_female_05_head".into(),
            origin: [916.5, 421.9, 4.0],
            rotation: [0.0, 0.0, 0.70710677, 0.70710677],
            velocity: [-400.0, 0.5, 30.0],
            mass: 0.002,
            bounce: 0.3,
            friction: 0.6,
            bullet_force_scale: 1.0,
            explosive_force_scale: 0.1,
        }],
        ..SnapshotMeta::default()
    };
    assert_eq!(through_the_wire(&meta).debris, meta.debris);
}

#[test]
fn the_match_header_reaches_the_client() {
    let meta = SnapshotMeta {
        phase: GameRoundStage::Playing,
        match_clock_ms: 123_456,
        prematch: WarmupStage::Starting { spent_ms: 4_321 },
        score_limit: 75,
        time_limit_ms: 600_000,
        kind: RulesetKind::SearchAndDestroy,
        clients: vec![(ClientId(2), ClientSnapshotMeta { score: 900, ..Default::default() })],
        ..SnapshotMeta::default()
    };
    let back = through_the_wire(&meta);
    assert_eq!(back.phase, meta.phase);
    assert_eq!(back.match_clock_ms, meta.match_clock_ms);
    assert_eq!(back.prematch, meta.prematch);
    assert_eq!(back.score_limit, meta.score_limit);
    assert_eq!(back.time_limit_ms, meta.time_limit_ms);
    assert_eq!(back.kind, meta.kind);
    assert_eq!(back.clients, meta.clients);
}

#[test]
fn a_finished_prematch_comes_back_with_no_time_on_it() {
    let meta = SnapshotMeta { prematch: WarmupStage::Done, ..SnapshotMeta::default() };
    assert_eq!(through_the_wire(&meta).prematch, WarmupStage::Done);
}

#[test]
fn an_unknown_prematch_tag_is_refused() {
    let mut bytes = snapshot_bytes(&SnapshotMeta::default());
    // One byte of phase and four of elapsed time come first.
    bytes[5] = 9;
    assert_eq!(read_snapshot(&bytes).unwrap_err(), PacketDecodeFault::GarbledRequest("unknown prematch tag"));
}

#[test]
fn an_unknown_game_mode_tag_is_refused() {
    let mut bytes = snapshot_bytes(&SnapshotMeta::default());
    // Phase 1, elapsed 4, prematch tag 1 and time 4, score limit 4, time limit 4.
    bytes[18] = 0xFF;
    assert_eq!(
        read_snapshot(&bytes).unwrap_err(),
        PacketDecodeFault::GarbledRequest("unknown GameModeKind tag")
    );
}

#[test]
fn a_snapshot_cut_short_is_refused_at_every_length() {
    let whole = snapshot_bytes(&SnapshotMeta::default());
    for kept in 0..whole.len() {
        assert!(read_snapshot(&whole[..kept]).is_err(), "kept {kept} of {}", whole.len());
    }
}

#[test]
fn the_section_sizes_add_up_to_what_was_written() {
    let meta = SnapshotMeta {
        feel: Some(distinct_feel(0b1_1111)),
        score_limit: 50,
        ..SnapshotMeta::default()
    };
    let mut wr = ByteSinkWriter::new();
    let sizes = pack_state_header_parts(&mut wr, &meta, &[1, 2, 3]);
    assert_eq!(sizes.sum_bytes(), wr.len());
    assert_eq!(sizes.world_len, 2 + 3);
    let mut plain = ByteSinkWriter::new();
    encode_snapshot_meta(&mut plain, &meta, &[1, 2, 3]);
    assert_eq!(plain.into_bytes(), wr.into_bytes());
}

#[test]
fn the_world_bytes_are_handed_back_as_they_came() {
    let meta = SnapshotMeta::default();
    let world = pack_scene_prop_resync(
        &mut WorldPropStreamEncoder::default(),
        Tick(1),
        &meta.world_objects,
    );
    let mut wr = ByteSinkWriter::new();
    encode_snapshot_meta(&mut wr, &meta, &world);
    let bytes = wr.into_bytes();
    let (_, passed_on) =
        parse_state_header(&mut ByteCursorReader::new(&bytes), &mut WorldPropStreamDecoder::default())
            .unwrap();
    assert_eq!(passed_on, world);
}
