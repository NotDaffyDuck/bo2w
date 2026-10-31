//! Golden bytes for the snapshot wire: each test encodes a fixed sample and
//! checks the bytes, and the decoded value's Debug text, against the files in
//! `testdata/meta_wire/`. A missing file is written from the current encoder,
//! so the first run records them and every later build (and any rewrite of
//! this module) must reproduce them exactly.

use std::path::PathBuf;

use playerstate_t6::game::BodyAnims;
use sim::{
    AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaSectorSnapshot, ClassId,
    ClassRejectReason, PlayerIntent, ClientId, PlayerSessionStage, ClientSnapshotMeta,
    ConfigurationChangeRejectReason, HarmOrigin, DebrisRecord, DestructibleLoopSound,
    DroppedItemAmmo, EntityEventPayload, EntityEventRecord, EntityKernelOccupiedSnapshot,
    EntityKernelSlotSnapshot, EntityKernelSnapshot, EntHandle, EntityRelations, EntityBehaviorClass,
    EventRecipients, EventRecord, EventSequence, GiveRejectReason, ItemPickupRecord, LifeSequence,
    LoadoutSpec, MatchOverCause, GameRoundStage, PelletFxRecord, PlayerCorpsePool, RngDebugMeta,
    ScriptModelId, MatchOccurrence, SnapshotMeta, SpawnPick, Tick, WorldObjectSnapshot,
};

use super::*;
use crate::net_links::byte_codec::{ByteCursorReader, ByteSinkWriter};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("meta_wire")
}

/// Records `name` on the first run; afterwards the bytes and the text must match.
fn check_golden(name: &str, raw: &[u8], text: &str) {
    let dir = fixture_dir();
    let bin = dir.join(format!("{name}.bin"));
    let txt = dir.join(format!("{name}.debug.txt"));
    if !bin.exists() || !txt.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&bin, raw).unwrap();
        std::fs::write(&txt, text).unwrap();
        return;
    }
    assert_eq!(std::fs::read(&bin).unwrap(), raw, "{name}.bin");
    let recorded = std::fs::read_to_string(&txt).unwrap().replace("\r\n", "\n");
    assert_eq!(recorded, text, "{name}.debug.txt");
}

/// A zero-padded text field of `N` bytes.
fn field<const N: usize>(text: &str) -> [u8; N] {
    let mut wr = [0u8; N];
    wr[..text.len()].copy_from_slice(text.as_bytes());
    wr
}

#[test]
fn every_client_action_keeps_its_bytes() {
    let actions = [
        PlayerIntent::JoinMatch { request_id: 1 },
        PlayerIntent::LeaveMatch { request_id: 2 },
        PlayerIntent::PickLoadout {
            request_id: 3,
            class_id: ClassId(7),
            revision: 41,
            loadout: sim::PersonalClass {
                weapons: [101, 202, 303, 404],
                perks: [11, 22, 33],
            },
        },
        PlayerIntent::HandOutGun { request_id: 4, gun: 0x0102_0304 },
        PlayerIntent::ChangeWeaponConfiguration { request_id: 5, from: 17, to: 18 },
        PlayerIntent::ForceDeath { request_id: 6 },
        PlayerIntent::SpawnClient { request_id: 7 },
        PlayerIntent::ForceSpawn {
            request_id: 8,
            pick: SpawnPick::Seeded(0x1122_3344_5566_7788),
        },
        PlayerIntent::ForceSpawn {
            request_id: 9,
            pick: SpawnPick::At { origin: [1.5, -2.25, 3.125], yaw: 270.5 },
        },
        PlayerIntent::SpawnIntermission { request_id: 10 },
        PlayerIntent::StageMatch { request_id: 11, phase: GameRoundStage::Warmup },
        PlayerIntent::StageMatch { request_id: 12, phase: GameRoundStage::Playing },
        PlayerIntent::StageMatch { request_id: 13, phase: GameRoundStage::Intermission },
        PlayerIntent::StageMatch { request_id: 14, phase: GameRoundStage::PostGame },
        PlayerIntent::Move {
            request_id: 15,
            origin: [-512.0, 1024.5, 64.25],
            angles: [-10.0, 359.5, 0.125],
        },
        PlayerIntent::SpinMover { request_id: 16, speed: -45.75 },
        PlayerIntent::TestHarm { request_id: 17, amount: -250 },
        PlayerIntent::SetName { request_id: 18, name: field("Player One") },
        PlayerIntent::UseCopycat { request_id: 19 },
        PlayerIntent::ActionSlot { request_id: 20, slot: 3 },
        PlayerIntent::ChooseDefaultClass { request_id: 21, index: 4 },
        PlayerIntent::MenuResponse {
            request_id: 22,
            menu: field("changeclass"),
            response: field("custom1"),
        },
        PlayerIntent::GiveKillstreak { request_id: 23, name: field("killstreak_rcbomb") },
    ];
    let actions: Vec<(ClientId, PlayerIntent)> =
        actions.into_iter().enumerate().map(|(i, action)| (ClientId(i as u32 * 3), action)).collect();

    let mut wr = ByteSinkWriter::new();
    write_intent_list(&mut wr, &actions);
    let bytes = wr.into_bytes();

    let mut rd = ByteCursorReader::new(&bytes);
    let back = read_intent_list(&mut rd).unwrap();
    assert!(rd.is_empty());
    let mut again = ByteSinkWriter::new();
    write_intent_list(&mut again, &back);
    assert_eq!(again.into_bytes(), bytes);

    check_golden("actions", &bytes, &format!("{back:#?}"));
}

#[test]
fn every_sim_event_keeps_its_bytes() {
    let mut events = vec![
        MatchOccurrence::LoadoutTaken { request_id: 1, class_id: ClassId(2), revision: 3 },
    ];
    for (i, reason) in [
        ClassRejectReason::UnknownOrStaleClass,
        ClassRejectReason::NoSpawnAvailable,
        ClassRejectReason::LockedContent,
        ClassRejectReason::NoSuchGun,
    ]
    .into_iter()
    .enumerate()
    {
        let i = i as u32;
        events.push(MatchOccurrence::LoadoutRefused {
            request_id: 10 + i,
            class_id: ClassId(20 + i),
            revision: 30 + i,
            reason,
        });
    }
    events.push(MatchOccurrence::HandoutTaken { request_id: 40, gun: 41 });
    for (i, reason) in [
        GiveRejectReason::NotAlive,
        GiveRejectReason::InvalidWeapon,
        GiveRejectReason::NoSuchGun,
        GiveRejectReason::UnsupportedWeapon,
        GiveRejectReason::EmptyCombatProfile,
    ]
    .into_iter()
    .enumerate()
    {
        let i = i as u32;
        events.push(MatchOccurrence::HandoutRefused { request_id: 50 + i, gun: 60 + i, reason });
    }
    events.push(MatchOccurrence::ConfigurationChangeAccepted { request_id: 70, from: 71, to: 72 });
    for (i, reason) in [
        ConfigurationChangeRejectReason::NotAlive,
        ConfigurationChangeRejectReason::StaleSource,
        ConfigurationChangeRejectReason::InvalidTarget,
        ConfigurationChangeRejectReason::DifferentFamily,
        ConfigurationChangeRejectReason::Busy,
        ConfigurationChangeRejectReason::NoInventorySlot,
        ConfigurationChangeRejectReason::AmmoTableFull,
        ConfigurationChangeRejectReason::SharedAmmoConflict,
    ]
    .into_iter()
    .enumerate()
    {
        let i = i as u32;
        events.push(MatchOccurrence::ConfigurationChangeRejected {
            request_id: 80 + i,
            from: 90 + i,
            to: 100 + i,
            reason,
        });
    }
    events.push(MatchOccurrence::Spawned {
        class_id: ClassId(5),
        spawn_slot: 17,
        life_sequence: LifeSequence(9),
    });
    for (i, source) in [
        HarmOrigin::Shot(sim::ShotId(1001)),
        HarmOrigin::Projectile(sim::ProjectileId(2002)),
        HarmOrigin::Radius(ScriptModelId::decode_net(3003)),
        HarmOrigin::Melee,
    ]
    .into_iter()
    .enumerate()
    {
        let i = i as u32;
        events.push(MatchOccurrence::Died {
            victim: ClientId(i),
            life_sequence: LifeSequence(110 + i),
            attacker: Some(ClientId(10 + i)),
            attacker_life: Some(LifeSequence(120 + i)),
            source: Some(source),
            gun: 130 + i,
            cam_ent_since: -1 - i as i32,
        });
    }
    events.push(MatchOccurrence::Died {
        victim: ClientId(17),
        life_sequence: LifeSequence(3),
        attacker: None,
        attacker_life: None,
        source: None,
        gun: 0,
        cam_ent_since: 90_000,
    });
    events.push(MatchOccurrence::TallyMoved { client: ClientId(4), score: 1250, kills: 12, times_died: -1 });
    events.push(MatchOccurrence::MatchEnded { reason: MatchOverCause::PointsReached });
    events.push(MatchOccurrence::MatchEnded { reason: MatchOverCause::TimeLimit });
    events.push(MatchOccurrence::TriggerLet);
    events.push(MatchOccurrence::WeaponSwitchRequested { weapon: 0xdead_beef });

    let mut wr = ByteSinkWriter::new();
    for ev in &events {
        pack_occurrence(&mut wr, ev);
    }
    let bytes = wr.into_bytes();

    let mut rd = ByteCursorReader::new(&bytes);
    let mut back = Vec::new();
    while !rd.is_empty() {
        back.push(read_occurrence(&mut rd).unwrap());
    }
    let mut again = ByteSinkWriter::new();
    for event in &back {
        pack_occurrence(&mut again, event);
    }
    assert_eq!(again.into_bytes(), bytes);

    check_golden("events", &bytes, &format!("{back:#?}"));
}

/// A map with twenty panes: one cracked, one shattered.
fn glass_world() -> WorldObjectSnapshot {
    let mut t6_glass: Vec<glass_t6::PaneUpdate> = (0..20u16)
        .map(|pane| glass_t6::PaneUpdate { pane, state: glass_t6::PaneState::Pristine, hit: None })
        .collect();
    t6_glass[18].state = glass_t6::PaneState::Cracked;
    t6_glass[19].state = glass_t6::PaneState::Shattered;
    t6_glass[19].hit = Some(([12.5, -40.0, 96.25], [0.0, 0.6, -0.8]));
    WorldObjectSnapshot {
        as_of_ms: 10_050,
        map_round_epoch: 3,
        t6_glass,
        destructible_loop_sounds: vec![
            DestructibleLoopSound {
                owner: ScriptModelId::decode_net(77),
                alias_index: 4,
                origin: [100.0, 200.5, -8.0],
                volume: 200,
                pitch: 90,
            },
            DestructibleLoopSound {
                owner: ScriptModelId::decode_net(78),
                alias_index: 9,
                origin: [-1.0, 0.0, 1.0],
                volume: 255,
                pitch: 128,
            },
        ],
    }
}

#[test]
fn world_object_sync_keeps_its_bytes_through_full_quiet_and_delta_ticks() {
    let mut world = glass_world();
    let mut steps = Vec::new();
    // First tick after a reset: the full world.
    steps.push((201, world.clone()));
    // Nothing but time moved: only the clock goes out.
    world.as_of_ms += 50;
    steps.push((202, world.clone()));
    // One pane cracks: a glass delta.
    world.as_of_ms += 50;
    world.t6_glass[4].state = glass_t6::PaneState::Cracked;
    steps.push((203, world.clone()));
    // A loop sound stops: the sounds again, no glass news.
    world.as_of_ms += 50;
    world.destructible_loop_sounds.pop();
    steps.push((204, world.clone()));
    // A pane shatters, carrying where it was hit.
    world.as_of_ms += 50;
    world.t6_glass[7].state = glass_t6::PaneState::Shattered;
    world.t6_glass[7].hit = Some(([1.0, 2.0, 3.0], [0.0, 0.0, -1.0]));
    steps.push((205, world.clone()));
    // The periodic full resend.
    world.as_of_ms += 50;
    world.map_round_epoch += 1;
    steps.push((400, world.clone()));

    let mut encoder = WorldPropStreamEncoder::default();
    encoder.start_over();
    let mut decoder = WorldPropStreamDecoder::default();
    let mut bytes = Vec::new();
    let mut text = String::new();
    for (tick, current) in &steps {
        let wire = pack_scene_prop_resync(&mut encoder, Tick(*tick), current);
        let back = parse_scene_prop_resync(&mut decoder, &wire).unwrap();
        bytes.extend_from_slice(&(wire.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&wire);
        text.push_str(&format!("tick {tick}: {back:#?}\n"));
    }

    check_golden("world_sync", &bytes, &text);
}

/// Encodes `meta`, decodes it again, and records both the bytes and the text.
/// The decoded value is not compared with the sample (a few fields never
/// travel); instead it must encode to the very same bytes.
fn meta_round_trip(name: &str, meta: &SnapshotMeta) {
    let wire = pack_scene_prop_resync(
        &mut WorldPropStreamEncoder::default(),
        Tick(1),
        &meta.world_objects,
    );
    let mut wr = ByteSinkWriter::new();
    let sizes = pack_state_header_parts(&mut wr, meta, &wire);
    let bytes = wr.into_bytes();

    let mut rd = ByteCursorReader::new(&bytes);
    let (back, back_wire) =
        parse_state_header(&mut rd, &mut WorldPropStreamDecoder::default()).unwrap();
    assert!(rd.is_empty());
    let mut again = ByteSinkWriter::new();
    pack_state_header_parts(&mut again, &back, &back_wire);
    assert_eq!(again.into_bytes(), bytes);

    check_golden(name, &bytes, &format!("{sizes:#?}\n{back:#?}"));
}

fn hud_pair() -> [hud_t6::overlay_piece::OverlayPiece; 2] {
    let mut bank = [hud_t6::overlay_piece::OverlayPiece::default(); 2];
    bank[0].x = 320.5;
    bank[0].y = -12.25;
    bank[0].z = 1.0;
    bank[0].text_size = 1.5;
    bank[0].tint_rgba = 0xff80_40ff;
    bank[0].time = 9000;
    bank[0].lasts = 2500;
    bank[0].val = 42.0;
    bank[0].sort = 3.0;
    bank[0].caption = 7;
    bank[0].words = 8;
    bank[0].piece_kind = 1;
    bank[0].font = 2;
    bank[0].anchor = 5;
    bank[0].skin_no = 11;
    bank[0].sound_no = 4;
    bank[1].x = -64.0;
    bank[1].y = 480.0;
    bank[1].width = 200;
    bank[1].height = 24;
    bank[1].tint_rgba = 0x1020_3040;
    bank[1].bits = 0b1010;
    bank[1].aim_ent = 31;
    bank
}

/// A client with every optional part present and almost no field at its default.
fn busy_client() -> ClientSnapshotMeta {
    let bank = hud_pair();
    use hud_t6::flashbang;
    let shock = flashbang::ShellshockParms {
        screen_blend: flashbang::ShellshockScreenBlend {
            blurred_fade_ms: 300,
            blurred_effect_ms: 410,
            flash_white_fade_ms: 120,
            flash_shot_fade_ms: 65,
            kind: flashbang::ShockViewType::Flashed,
        },
        view: flashbang::ShellshockView { fade_for_ms: 250, kick_rate: 4.5, kick_radius: 8.0 },
        sound: flashbang::ShellshockSound {
            touches: true,
            loop_alias: "shock_loop".into(),
            loop_silent_alias: "shock_loop_quiet".into(),
            end_alias: "shock_end".into(),
            end_abort_alias: "shock_abort".into(),
            fade_in_ms: 30,
            fade_out_ms: 40,
            loop_fade_ms: 50,
            loop_end_delay_ms: 70,
            room_type: "generic".into(),
            dry_level: 0.5,
            wet_level: 0.25,
            mod_end_delay_ms: 90,
            snapshot: "snd_shock".into(),
        },
        look_control: flashbang::ShellshockLookControl {
            touches: true,
            fade_for_ms: 80,
            touchiness: 0.5,
            pitch_cap: 20.0,
            yaw_cap: 30.0,
        },
        movement_scale: 0.75,
        animation: true,
        vision_set: flashbang::ShellshockVisionSet { name: "flashed".into(), in_ms: 100, out_ms: 900 },
    };

    let vision = |name: &str, duration_ms: i32, set_ms: i32| {
        Some(sim::VisionChange { name: name.into(), lasts_ms: duration_ms, set_ms })
    };
    ClientSnapshotMeta {
        controls: sim::ScriptControls {
            stuck: true,
            jump_disabled: true,
            paired: true,
            prone_disabled: true,
            stand_disabled: true,
            melee_disabled: true,
            switch_to: 0x0a0b,
            ..sim::ScriptControls::default()
        },
        weapon_lock: sim::WeaponLock {
            gun: 321,
            life_no: 4,
            bits: 0b101,
            target: [1.0, 2.0, 3.0],
            aim: Some(sim::MissileTarget::Entity {
                entity: EntHandle::from_parts(30, 2).unwrap(),
                offset: [0.5, 0.25, -0.5],
            }),
            acquire_started_at: 8800,
        },
        killcam_hud: Some(sim::KillcamHud { final_kill: true }),
        lifecycle: PlayerSessionStage::Alive,
        loadout: Some(LoadoutSpec {
            class_id: ClassId(3),
            revision: 12,
            primary: 101,
            secondary: 202,
            primary_attachments: [1, 2, 3, 4],
            secondary_attachments: [5, 6, 7, 8],
            lethal: 9,
            tactical: 10,
            perks: [11, 12, 13],
        }),
        life_sequence: LifeSequence(5),
        item_ready_ms: 1500,
        item_use_entity: Some(EntHandle::from_parts(31, 1).unwrap()),
        mag_rounds: 17,
        ammo_stock: 120,
        score: 4500,
        kills: 31,
        deaths: 7,
        kill_streak: 5,
        radar: sim::RadarMode::Fast,
        remote_missile: Some(sim::SteeredMissile {
            projectile: sim::ProjectileId(77),
            ent_no: 45,
            turn: [10.0, 20.0, 30.0],
            armed: true,
            boosted: false,
            attack: true,
            coasting: false,
            unlink_at_ms: Some(1234),
            t6: true,
            holding: false,
        }),
        rounds_per_gun: vec![(101, 12, 90), (202, 8, 24)],
        taped_mag_spent: vec![101],
        weapon_shot_count: 3,
        burst_latch: true,
        burst_latch_secondary: false,
        rechamber_pending: true,
        rechamber_pending_secondary: false,
        died_at_step: Some(777),
        look_at_killer_yaw: -90,
        name: field("Player One"),
        hud_archival: bank.to_vec(),
        hud_current: vec![bank[1]],
        ffa_team: Some(2),
        client_state_team: 1,
        rank: 55,
        prestige: 3,
        card_icon: 1001,
        card_title: 1002,
        card_plate: 1003,
        client_dvars: vec![("cg_fov".into(), "80".into()), ("name".into(), "Player One".into())],
        shellshock: Some(shock),
        view_effects: sim::ViewEffects {
            naked_vision: vision("naked", 400, 10),
            thermal_vision: vision("thermal", 500, 20),
            missile_vision: vision("missile", 600, 30),
            night_vision: vision("night", 700, 40),
            pain_vision: vision("pain", 800, 50),
            depth_of_field: sim::ScriptDepthOfField {
                near_from: 1.0,
                near_to: 2.0,
                far_from: 300.0,
                far_to: 900.0,
                near_smear: 4.0,
                far_smear: 0.5,
            },
            blur: Some(sim::ScriptBlur { from: 0.0, to: 6.5, set_ms: 5, lasts_ms: 1500 }),
        },
        menu_commands: vec![
            sim::MenuOrder { serial: 1, kind: sim::MenuCommandKind::Show("class".into()) },
            sim::MenuOrder { serial: 2, kind: sim::MenuCommandKind::ClosePopup },
            sim::MenuOrder { serial: 3, kind: sim::MenuCommandKind::CloseInGame },
        ],
        location_selection: Some(sim::LocationSelection {
            material: "compass_map_default".into(),
            choose_direction: true,
            reach: 192.0,
        }),
        t6_view: Some(sim::T6ViewInputs {
            weap_anim: 5,
            weap_anim_left: 6,
            weaponstate: 7,
            weaponstate_left: 8,
            weapon_time: 250,
            weapon_time_left: 260,
            intro_shots_fired: 2,
            ever_had: 0x0f0f,
            needs_rechamber: 0x0003,
        }),
        t6_perks: 0x0123_4567_89ab_cdef,
    }
}

#[test]
fn match_header_and_every_kind_of_client_keep_their_bytes() {
    let mut chasing = ClientSnapshotMeta {
        lifecycle: PlayerSessionStage::Dead,
        killcam_hud: Some(sim::KillcamHud { final_kill: false }),
        radar: sim::RadarMode::Constant,
        ..ClientSnapshotMeta::default()
    };
    chasing.weapon_lock.aim = Some(sim::MissileTarget::Player {
        client: ClientId(1),
        life: LifeSequence(3),
        offset: [0.0, 0.0, 12.0],
    });
    let mut pointing =
        ClientSnapshotMeta { lifecycle: PlayerSessionStage::Spectating, ..Default::default() };
    pointing.weapon_lock.aim = Some(sim::MissileTarget::Point([100.0, -200.0, 50.5]));
    pointing.radar = sim::RadarMode::Normal;

    let mut clients =
        vec![(ClientId(0), busy_client()), (ClientId(1), chasing), (ClientId(7), pointing)];
    for (id, lifecycle) in [
        (2, PlayerSessionStage::Connecting),
        (3, PlayerSessionStage::PickingLoadout),
        (4, PlayerSessionStage::AwaitingFirstLife),
        (5, PlayerSessionStage::AwaitingNextLife),
        (6, PlayerSessionStage::Intermission),
    ] {
        clients.push((ClientId(id), ClientSnapshotMeta { lifecycle, ..Default::default() }));
    }
    let meta = SnapshotMeta {
        phase: GameRoundStage::Playing,
        match_clock_ms: 123_456,
        prematch: gamemode_t6::WarmupStage::Starting { spent_ms: 2500 },
        score_limit: 75,
        time_limit_ms: 600_000,
        kind: gamemode_t6::RulesetKind::Domination,
        clients,
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_clients", &meta);

    let waiting = SnapshotMeta {
        phase: GameRoundStage::Warmup,
        prematch: gamemode_t6::WarmupStage::Waiting { spent_ms: 40 },
        kind: gamemode_t6::RulesetKind::TeamDeathmatch,
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_waiting", &waiting);
    let done = SnapshotMeta {
        phase: GameRoundStage::PostGame,
        prematch: gamemode_t6::WarmupStage::Done,
        kind: gamemode_t6::RulesetKind::Hardpoint,
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_done", &done);
}

#[test]
fn journal_events_effects_and_rules_keep_their_bytes() {
    let record = |sequence: u32, audience: EventRecipients, event: MatchOccurrence| EventRecord {
        sequence: EventSequence(sequence),
        tick: Tick(100 + sequence),
        audience,
        event,
    };
    let journal = vec![
        record(1, EventRecipients::All, MatchOccurrence::TriggerLet),
        record(
            2,
            EventRecipients::AllExcept(ClientId(3)),
            MatchOccurrence::TallyMoved { client: ClientId(1), score: 100, kills: 2, times_died: 0 },
        ),
        record(
            3,
            EventRecipients::Client(ClientId(2)),
            MatchOccurrence::Spawned {
                class_id: ClassId(1),
                spawn_slot: 4,
                life_sequence: LifeSequence(2),
            },
        ),
        record(
            4,
            EventRecipients::Clients(vec![ClientId(0), ClientId(5), ClientId(9)]),
            MatchOccurrence::WeaponSwitchRequested { weapon: 77 },
        ),
    ];
    let payload = EntityEventPayload {
        number: 40,
        other_entity_num: 41,
        attacker_entity_num: 3,
        event_parm: 9,
        weapon: 12,
        correlation: 555,
        pellet: 4,
        hand: 1,
        origin: [1.0, 2.0, 3.0],
        origin2: [4.0, 5.0, 6.0],
        direction: [0.0, 0.0, 1.0],
        surf_type: 7,
        surface_flags: 0x20,
        wall_pass_bits: 2,
    };
    let entity_events = [
        entity_t6::EventCode::BOLT_IMPACT,
        entity_t6::EventCode::MISSILE_REMOTE_BOOST,
        entity_t6::EventCode::EARTHQUAKE,
        entity_t6::EventCode::PLAY_RUMBLE_ON_POS,
        entity_t6::EventCode::STOP_ALL_RUMBLES,
        entity_t6::EventCode::SENSOR_LASER,
        entity_t6::EventCode::FOOTSTEP_CROUCHRUN,
        entity_t6::EventCode::SLIDE_START,
        entity_t6::EventCode::SLIDE_STOP,
        entity_t6::EventCode(4),
        entity_t6::EventCode(11),
        entity_t6::EventCode(18),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, event)| EntityEventRecord {
        sequence: EventSequence(10 + i as u32),
        tick: Tick(200 + i as u32),
        audience: if i % 2 == 0 {
            EventRecipients::All
        } else {
            EventRecipients::Client(ClientId(i as u32))
        },
        event,
        payload: EntityEventPayload { number: payload.number + i as i32, ..payload },
    })
    .collect();
    let pellet_fx = vec![
        PelletFxRecord {
            attacker: 3,
            weapon: 99,
            correlation: 7,
            pellet: 2,
            hand: 1,
            start: [0.0, 0.0, 60.0],
            end: [300.0, 20.0, 55.0],
            normal: [-1.0, 0.0, 0.0],
            surf_type: 5,
            surface_flags: 0x40,
            body_hit_bits: 1,
        },
        PelletFxRecord::default(),
    ];
    let debris = vec![DebrisRecord {
        mesh: "xmodel_glass_shard".into(),
        origin: [10.0, 20.0, 30.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0, 5.0, 100.0],
        mass: 2.0,
        bounce: 0.3,
        friction: 0.5,
        bullet_force_scale: 1.0,
        explosive_force_scale: 2.0,
    }];
    let oriented_blockers = vec![sim::OrientedBlocker {
        ent_no: 12,
        origin: [64.0, -32.0, 8.0],
        frame: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        lo: [-8.0, -16.0, -4.0],
        hi: [8.0, 16.0, 4.0],
        face_bits: 0x40,
    }];
    let feel = movement_t6::bridge::Bo2Feel {
        sprint_strafe_speed_scale: 1.5,
        jump_slowdown: true,
        fall_damage_min_height: 2.5,
        fall_damage_max_height: 3.5,
        zombies: false,
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
        dive: true,
        sprint_cycle_scale: 14.5,
        ducked_sprint_cycle_scale: 15.5,
        dtp_cycle_scale: 16.5,
        omni: false,
        mantle_weapon_height: 17.5,
        mantle_weapon_anim_height: 18.5,
        leap_height: 19.5,
    };
    let meta = SnapshotMeta {
        journal,
        entity_events,
        pellet_fx,
        debris,
        oriented_blockers,
        feel: Some(feel),
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_events", &meta);
}

#[test]
fn name_tables_strings_and_rng_keep_their_bytes() {
    let table = |names: &[&str]| -> Vec<(u8, String)> {
        names.iter().enumerate().map(|(i, name)| (i as u8 * 3, name.to_string())).collect()
    };
    let meta = SnapshotMeta {
        sound_aliases: table(&["snd_pickup", "snd_reload_ar", "snd_glass_break"]),
        effect_names: table(&["fx_muzzle_flash", "fx_blood_hit"]),
        hud_materials: table(&["hud_icon_kills", "hud_icon_deaths", "white"]),
        hud_strings: vec![(3, "Capture the point".into()), (20, "Enemy team wins".into())],
        rng: RngDebugMeta {
            root_seed: 0x0123_4567_89ab_cdef,
            scheme: 2,
            spawn_draws: 11,
            combat_draws: 222,
            bot_draws: 3333,
        },
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_aliases", &meta);
}

#[test]
fn model_skeleton_states_keep_their_bytes() {
    use xmodel_runtime::{
        RigLayout, RigMeshEntry, RigPoseRecord, HiddenParts, PartSet,
        GraphSlot, NodeClock, GraphNodeRecord, AnimGraphRole, GraphCapture,
    };
    let node = |parent: Option<u16>,
                kind: AnimGraphRole,
                clip: Option<&str>,
                parts: Option<PartSet>,
                weight: f32| GraphNodeRecord {
        parent: parent.map(GraphSlot),
        kind,
        clip: clip.map(str::to_string),
        parts,
        state: NodeClock {
            time: 0.25,
            prev_t: 0.125,
            laps: 3,
            prev_laps: -2,
            blend_left: 1.0,
            target_w: 0.5,
            weight,
            rate: 1.5,
        },
    };
    let rich = RigPoseRecord {
        composition: RigLayout {
            revision: 3,
            models: vec![
                RigMeshEntry {
                    model: "veh_body".into(),
                    host_index: None,
                    mount_tag: None,
                    no_hits: false,
                    tag_offset: None,
                },
                RigMeshEntry {
                    model: "veh_turret".into(),
                    host_index: Some(0),
                    mount_tag: Some("tag_turret".into()),
                    no_hits: true,
                    tag_offset: None,
                },
            ],
        },
        pose_epoch: 9,
        tree: Some(GraphCapture {
            shape_epoch: 2,
            clock_epoch: 5,
            nodes: vec![
                node(None, AnimGraphRole::Blend, None, None, 1.0),
                node(
                    Some(0),
                    AnimGraphRole::Leaf,
                    Some("idle"),
                    Some(PartSet::from_bit_chunks([0b101, 0, 0, 0, 0b11])),
                    0.75,
                ),
                node(Some(0), AnimGraphRole::Additive, Some("recoil"), None, 0.25),
            ],
        }),
        wanted_mask: Some(PartSet::from_bit_chunks([1, 2, 3, 4, 5])),
        hidden_mask: HiddenParts::from_bit_chunks([0, 0, 8, 0, 0x10]),
        vehicle_steer: Some(0.25),
        turret_aim: Some([10.0, -5.0]),
        ragdoll: None,
        eye_luminance: None,
    };
    let plain = RigPoseRecord {
        composition: RigLayout {
            revision: 1,
            models: vec![RigMeshEntry {
                model: "crate_small".into(),
                host_index: None,
                mount_tag: None,
                no_hits: false,
                tag_offset: None,
            }],
        },
        pose_epoch: 0,
        tree: None,
        wanted_mask: None,
        hidden_mask: HiddenParts::from_bit_chunks([0; 5]),
        vehicle_steer: None,
        turret_aim: None,
        ragdoll: None,
        eye_luminance: None,
    };
    let owner = |id: u32| sim::ServerModelSource::ScriptModel(ScriptModelId::decode_net(id));
    let meta = SnapshotMeta {
        entity_dobjs: vec![(owner(77), rich), (owner(78), plain)],
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_dobjs", &meta);
}

fn sample_entity(number: i32, e_type: i16) -> entity_t6::EntitySnapshot {
    let mut es = entity_t6::EntitySnapshot::default();
    es.slot_index = number;
    es.e_type = e_type;
    es.lerp.e_flags = 0x0410;
    es.lerp.pos = entity_t6::Trajectory {
        tr_type: 2,
        began_at: 5000,
        span_ms: 750,
        base_point: [100.0, 200.0, 30.5],
        rate: [0.0, 8.0, -4.0],
    };
    es.lerp.apos = entity_t6::Trajectory {
        tr_type: 1,
        began_at: 6000,
        span_ms: 0,
        base_point: [0.0, 90.0, 0.0],
        rate: [0.0, 0.0, 0.0],
    };
    es.index = 12;
    es.weapon = playerstate_t6::Weapon(321);
    es.solid = 0xffffff;
    es.lerp.u.set_missile(&entity_t6::LerpMissile {
        spawn_moment: 4800,
        ..entity_t6::LerpMissile::default()
    });
    es.client_num = 6;
    es.un2.set_player_anim(&entity_t6::PlayerAnimState {
        legs_clip: 21,
        torso_clip: 34,
        ..entity_t6::PlayerAnimState::default()
    });
    es.ground_entity_num = 1022;
    es.event_sequence = 9;
    es.events = [1, 2, 3, 4];
    es.event_parms = [10, 20, 30, 40];
    es
}

#[test]
fn entities_kernel_corpses_and_items_keep_their_bytes() {
    let mut corpses = PlayerCorpsePool::default();
    corpses.spawn_ring = 2;
    {
        let slot = &mut corpses.slots[1];
        slot.occupied = true;
        slot.entnum = 60;
        slot.victim = ClientId(3);
        slot.origin = [10.0, 20.0, 30.0];
        slot.viewangles = [0.0, 180.0, 0.0];
        slot.anim = BodyAnims { legs: 7, torso: 9 };
        slot.view_height_current = 11.5;
        slot.weapon = 88;
        slot.e_flags = 0x400;
        slot.tr_type = 2;
        slot.tr_time = 7000;
        slot.tr_duration = 1500;
        slot.tr_delta = [1.0, 2.0, 3.0];
        slot.tr_base = [4.0, 5.0, 6.0];
        slot.falling = true;
        slot.ground_entity_num = 1022;
    }

    let owner = EntHandle::from_parts(26, 2).unwrap();
    let occupied = |kind, relations| {
        Some(EntityKernelOccupiedSnapshot {
            kind,
            linked: true,
            relations,
            next_think_ms: Some(9100),
            transient_event_time_ms: None,
        })
    };
    let bare = EntityRelations {
        owner: None,
        parent: None,
        ground: None,
        parent_tag: -1,
        parent_link_axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        parent_link_origin: [0.0; 3],
    };
    let entity_kernel = EntityKernelSnapshot {
        slots: vec![
            EntityKernelSlotSnapshot {
                generation: 2,
                occupied: occupied(EntityBehaviorClass::ScriptedMover, bare),
                freed_at_ms: 0,
            },
            EntityKernelSlotSnapshot { generation: 3, occupied: None, freed_at_ms: 900 },
            EntityKernelSlotSnapshot {
                generation: 1,
                occupied: Some(EntityKernelOccupiedSnapshot {
                    kind: EntityBehaviorClass::Missile,
                    linked: false,
                    relations: EntityRelations {
                        owner: Some(owner),
                        parent_tag: 5,
                        parent_link_origin: [1.0, 2.0, 3.0],
                        ..bare
                    },
                    next_think_ms: None,
                    transient_event_time_ms: Some(8800),
                }),
                freed_at_ms: 0,
            },
            EntityKernelSlotSnapshot { generation: 1, occupied: None, freed_at_ms: 400 },
        ],
        high_water: 30,
        free_fifo: vec![29, 27],
        level_time_ms: 12_345,
        frame_serial: 678,
    };

    let area_entities = AreaEntityWorldSnapshot {
        world_mins: [-2048.0, -2048.0, -512.0],
        world_maxs: [2048.0, 2048.0, 512.0],
        free_prefix: vec![5, 3],
        contiguous_free_head: 6,
        sectors: vec![
            AreaSectorSnapshot {
                index: 1,
                contents_entities: 0x3,
                entities: 8,
                dist: 0.0,
                axis: 0,
                parent: 0,
                child: [2, 4],
            },
            AreaSectorSnapshot {
                index: 2,
                contents_entities: 0,
                entities: 0,
                dist: 256.0,
                axis: 1,
                parent: 1,
                child: [0, 0],
            },
            AreaSectorSnapshot {
                index: 4,
                contents_entities: 0x1,
                entities: 4,
                dist: -128.0,
                axis: 2,
                parent: 1,
                child: [0, 0],
            },
        ],
        entities: vec![
            AreaEntityLinkSnapshot {
                entity_num: 7,
                world_sector: 1,
                next_entity: 0,
                linkcontents: 0x3,
                linkmin: [0.0, 0.0, 0.0],
                linkmax: [16.0, 16.0, 16.0],
                contents: 0x1,
            },
            AreaEntityLinkSnapshot {
                entity_num: 3,
                world_sector: 4,
                next_entity: 0,
                linkcontents: 0x1,
                linkmin: [-8.0, -8.0, -8.0],
                linkmax: [8.0, 8.0, 8.0],
                contents: 0x1,
            },
        ],
    };

    let mover = |id: u32, nonsolid: bool| sim::MoverSlot {
        id: ScriptModelId::decode_net(id),
        state: sample_entity(70 + id as i32, 6),
        nonsolid,
        ..Default::default()
    };
    let meta = SnapshotMeta {
        entities: vec![sample_entity(40, 1), sample_entity(41, 8), entity_t6::EntitySnapshot::default()],
        script_movers: vec![mover(1, false), mover(2, true)],
        entity_kernel,
        corpses,
        item_ammo: vec![
            DroppedItemAmmo {
                entnum: 50,
                clip_r: 12,
                clip_l: 6,
                stock: 90,
                stock2: 30,
                scavenger: 1,
                owner_clear_ms: 4500,
            },
            DroppedItemAmmo::default(),
        ],
        item_pickups: vec![
            ItemPickupRecord {
                picker: 2,
                weapon: 101,
                from_entnum: 50,
                clip_r: 12,
                clip_l: 6,
                stock: 90,
                swapped_entnum: 51,
                picker_pm_type: 1,
                ammo_only: false,
            },
            ItemPickupRecord { picker: 4, ammo_only: true, ..ItemPickupRecord::default() },
        ],
        area_entities: Some(area_entities),
        ..SnapshotMeta::default()
    };
    meta_round_trip("meta_world", &meta);
}

#[test]
fn compass_fog_effects_and_quakes_keep_their_bytes() {
    let vision = |name: &str, duration_ms: i32, set_ms: i32| {
        Some(sim::VisionChange { name: name.into(), lasts_ms: duration_ms, set_ms })
    };
    let fog = sim::ScriptFog {
        from: sim::ScriptFogParams {
            from_dist: 0.0,
            half_dist: 1000.0,
            tint_rgb: [0.5, 0.5, 0.5],
            thickest: 0.75,
            sun: None,
        },
        to: sim::ScriptFogParams {
            from_dist: 100.0,
            half_dist: 2500.0,
            tint_rgb: [0.25, 0.5, 1.0],
            thickest: 1.0,
            sun: Some(sim::ScriptSunFog {
                tint_rgb: [1.0, 0.75, 0.5],
                sun_way: [0.0, 0.6, 0.8],
                from_deg: 10.0,
                to_deg: 60.0,
                scale: 1.5,
            }),
        },
        began_ms: 5000,
        lasts_ms: 2000,
    };
    let objectives = sim::MatchBoard {
        scores: [0, 120, 95],
        compass: vec![
            sim::CompassMark {
                index: 2,
                state: sim::ObjectiveState::Active,
                origin: [100.0, -100.0, 8.0],
                team: gamemode_t6::Team::Allies,
                icon: "compass_waypoint_capture".into(),
            },
            sim::CompassMark {
                index: 5,
                state: sim::ObjectiveState::Invisible,
                origin: [0.0, 0.0, 0.0],
                team: gamemode_t6::Team::Axis,
                icon: String::new(),
            },
        ],
        vehicles: vec![sim::CompassRide {
            origin: [50.0, 60.0, 70.0],
            turn_deg: 135.0,
            owner: 3,
            team: 2,
            icons: ["compass_uav".into(), "compass_uav_red".into()],
            size: [16, 24],
        }],
        server_info: vec![("g_gametype".into(), "tdm".into()), ("mapname".into(), "mp_raid".into())],
        game_end_time: 900_000,
        scripted_effects: true,
        effects: vec![
            sim::PlacedEffect {
                id: 1,
                effect: 4,
                origin: [1.0, 2.0, 3.0],
                forward: [1.0, 0.0, 0.0],
                up_dir: [0.0, 0.0, 1.0],
                start_ms: Some(0),
                repeat_ms: 500,
                cull_distance: 4096.0,
            },
            sim::PlacedEffect { id: 2, effect: 9, start_ms: None, ..Default::default() },
        ],
        fog: Some(fog),
        earthquakes: vec![sim::ScriptEarthquake {
            id: 3,
            origin: [10.0, 20.0, 30.0],
            scale: 0.4,
            reach: 800.0,
            began_ms: 6000,
            lasts_ms: 1500,
        }],
        naked_vision: vision("mpintro", 0, 0),
        thermal_vision: vision("thermal", 300, 10),
        missile_vision: vision("missile_pitch_black", 500, 20),
        night_vision: vision("nightvision", 700, 30),
        pain_vision: vision("pain", 900, 40),
        ..sim::MatchBoard::default()
    };
    let meta = SnapshotMeta { objectives, ..SnapshotMeta::default() };
    meta_round_trip("meta_objectives", &meta);
}
