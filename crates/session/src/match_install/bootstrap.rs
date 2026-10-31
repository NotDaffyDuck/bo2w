//! Starting the match in the simulation: where players can spawn, what the
//! classes on offer are, and which of them the loaded weapons can't back.

use std::sync::Arc;

use asset_game::WeaponLedger;
use asset_world::RespawnMarker;
use frame::ServerKitRoster;
use gamemode_t6::RulesetKind;
use net::HostInputValve;
use render_frontend::prepare::scene::camera::GameplayCamera;

use super::refusal::InstallRefusal;
use crate::loadout::{
    HostLoadoutMirror, ClassRow, class_perk_slots, fill_default_perks, project_loadout,
};

/// The classes the host set up, or the showcase ones when it set none up.
fn bootstrap_class_rows(
    chosen: Option<&ServerKitRoster>,
    registry: &WeaponLedger,
) -> Vec<ClassRow> {
    let showcase = ServerKitRoster::default();
    let source = match chosen {
        Some(own) if !own.slots.is_empty() => own,
        _ => &showcase,
    };
    let mut rows = Vec::with_capacity(source.slots.len());
    for original in &source.slots {
        let mut copy = original.clone();
        fill_default_perks(&mut copy, registry);
        rows.push(ClassRow::from(&copy));
    }
    rows
}

/// What the loaded weapons let each class be.
pub(super) struct Roster {
    player_defs: Vec<sim::LoadoutClassSpec>,
    bot_defs: Vec<sim::LoadoutClassSpec>,
    /// Which slot each perk of the stats tables goes in.
    perk_slots: Vec<(u32, u8)>,
    /// Why each class is locked, in class order; `None` for a usable one.
    pub locked_because: Vec<Option<String>>,
}

pub(super) fn roster(
    host: Option<&ServerKitRoster>,
    weapons: &WeaponLedger,
    combat: &[weapon_t6::combat_facts::GunBattleProfile],
    gear: &[sim::GadgetLiveStats],
) -> Roster {
    let project = |position: usize, row: &ClassRow| -> HostLoadoutMirror {
        project_loadout(position as u32, row, weapons, combat, gear)
    };
    let mut projected: Vec<HostLoadoutMirror> = Vec::new();
    for (position, row) in bootstrap_class_rows(host, weapons).iter().enumerate() {
        projected.push(project(position, row));
    }
    let locked_because: Vec<Option<String>> = projected
        .iter()
        .map(|entry| entry.frozen_because.clone())
        .collect();
    let player_defs = projected.into_iter().map(|entry| entry.def).collect();
    let mut bot_defs = Vec::new();
    for (position, preset) in frame::showcase_classes().iter().enumerate() {
        let mut slot: frame::ServerKitSlot = preset.into();
        fill_default_perks(&mut slot, weapons);
        bot_defs.push(project(position, &ClassRow::from(&slot)).def);
    }
    Roster {
        player_defs,
        bot_defs,
        perk_slots: class_perk_slots(weapons),
        locked_because,
    }
}

/// A place the map says the camera rests while players pick a class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BetweenRoundsCamera {
    pub spot: [f32; 3],
    pub turn: [f32; 3],
}

/// What the match needs to be started.
pub(super) struct Start<'a> {
    pub kind: RulesetKind,
    pub spawns: &'a [RespawnMarker],
    pub view: Option<BetweenRoundsCamera>,
    pub airstrike_height: Option<f32>,
    /// The score limit a test asked for, as typed.
    pub score_limit: Option<&'a str>,
}

pub(super) struct Started {
    /// What the launch report says the player is waiting on.
    pub gap: &'static str,
    pub locked_because: Vec<Option<String>>,
}

fn authored_spawn(marker: &RespawnMarker) -> sim::MapSpawnSpot {
    let (at, turn) = (marker.origin, marker.angles);
    sim::MapSpawnSpot {
        kind_name: marker.spawn_class.clone(),
        origin: at,
        turn,
        script_tied: marker.attached_to.clone(),
        script_destructable_area: marker.script_destructable_area.clone(),
    }
}

fn intermission_spawn(rest: BetweenRoundsCamera) -> sim::MapSpawnSpot {
    sim::MapSpawnSpot {
        kind_name: String::from(gamemode_t6::INTERLUDE_STATE_KEY),
        origin: rest.spot,
        turn: rest.turn,
        script_tied: String::default(),
        script_destructable_area: String::new(),
    }
}

/// The score a match ends at: the test's if it gave a number, else the game
/// type's own.
fn score_limit(mode: RulesetKind, asked: Option<&str>) -> i32 {
    match asked.and_then(|typed| typed.parse().ok()) {
        Some(number) => number,
        None => mode.default_score_limit(),
    }
}

fn waiting_on(has_view: bool) -> &'static str {
    match has_view {
        true => "player awaits SelectClass; mp_global_intermission holds the camera until Equip ack",
        false => {
            "player awaits SelectClass; Equip ack enables sim camera (no authored intermission)"
        }
    }
}

/// Hand the simulation its spawns and classes, and park the player's camera
/// and input until the class is chosen.
pub(super) fn start_match(
    running: &mut sim::SimWorld,
    start: Start,
    roster: Roster,
    camera: &mut GameplayCamera,
    gate: &mut HostInputValve,
) -> Result<Started, InstallRefusal> {
    let Start {
        kind,
        spawns,
        view,
        airstrike_height,
        score_limit: asked,
    } = start;
    let Roster {
        player_defs,
        bot_defs,
        perk_slots,
        locked_because,
    } = roster;
    let class_count = player_defs.len();
    let outcome = running.bootstrap(sim::MatchStartSetup {
        spawns: spawns.iter().map(authored_spawn).collect(),
        classes: player_defs,
        bot_classes: bot_defs,
        class_perk_slots: perk_slots,
        root: 0,
        kind,
        win_score: score_limit(kind, asked),
        round_ms: kind.default_time_limit_ms(),
        permit_cheat_requests: true,
        scoreboard_eye: view.map(intermission_spawn),
        airstrike_height,
    });
    if let Err(refused) = outcome {
        diag::info!(Sim, "bootstrap: {refused}");
        return Err(InstallRefusal::new(format!(
            "MatchBootstrap refused: {refused}"
        )));
    }
    camera.enabled = false;
    camera.hold_free_camera = view.is_some();
    gate.local_cmds_enabled = false;
    let locked = locked_because.iter().flatten().count();
    let dm_spawns = spawns.len();
    diag::info!(
        Sim,
        "spawn: match bootstrapped with {dm_spawns} dm spawns \
         ({locked} / {class_count} classes Locked by coverage)"
    );
    Ok(Started {
        gap: waiting_on(view.is_some()),
        locked_because,
    })
}

/// What the simulation traces a player's body and head against, for one team's
/// kit. A kit missing from the catalog traces against nothing.
pub(super) fn player_kit_collision(
    bodies: &asset_model::CharacterMeshRoster,
    axis: bool,
) -> sim::PlayerOutfitHitbox {
    let team_kit = bodies.kits().kit(axis);
    let torso_model_key = team_kit.map_or_else(String::new, |found| found.torso.clone());
    let face_model_key = team_kit
        .and_then(|found| found.skull.as_deref())
        .map(str::to_owned)
        .unwrap_or_default();
    let capability = |key: &str| {
        if key.is_empty() {
            return None;
        }
        let mesh = bodies.by_label(key)?;
        mesh.bone_rig.retained_capability().map(Arc::new)
    };
    sim::PlayerOutfitHitbox {
        body: capability(&torso_model_key),
        head: capability(&face_model_key),
        torso_model_key,
        face_model_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_host_the_showcase_classes_are_offered() {
        weapon_t6::class_presets::register();
        let offered = bootstrap_class_rows(None, &WeaponLedger::default());
        assert_eq!(offered.len(), ServerKitRoster::default().slots.len());
        assert!(!offered.is_empty());
    }

    #[test]
    fn a_host_with_no_classes_gets_the_showcase_ones() {
        weapon_t6::class_presets::register();
        let empty_host = ServerKitRoster { slots: Vec::new() };
        let ledger = WeaponLedger::default();
        let from_host = bootstrap_class_rows(Some(&empty_host), &ledger).len();
        assert_eq!(from_host, bootstrap_class_rows(None, &ledger).len());
    }

    #[test]
    fn the_hosts_own_classes_are_offered_when_it_has_some() {
        weapon_t6::class_presets::register();
        let mut own = ServerKitRoster::default();
        own.slots.truncate(2);
        let offered = bootstrap_class_rows(Some(&own), &WeaponLedger::default());
        assert_eq!(offered.len(), 2);
    }

    #[test]
    fn a_test_can_ask_for_a_score_limit() {
        let mode = RulesetKind::TeamDeathmatch;
        assert_eq!(score_limit(mode, Some("17")), 17);
        assert_eq!(score_limit(mode, Some("many")), mode.default_score_limit());
        assert_eq!(score_limit(mode, None), mode.default_score_limit());
    }

    #[test]
    fn the_intermission_view_becomes_a_spawn_of_its_own_class() {
        let spawn = intermission_spawn(BetweenRoundsCamera {
            spot: [1.0, 2.0, 3.0],
            turn: [0.0, 90.0, 0.0],
        });
        assert_eq!(spawn.kind_name, gamemode_t6::INTERLUDE_STATE_KEY);
        assert_eq!(spawn.origin, [1.0, 2.0, 3.0]);
        assert_eq!(spawn.turn, [0.0, 90.0, 0.0]);
        assert!(spawn.script_tied.is_empty());
    }

    #[test]
    fn a_spawn_keeps_its_script_links() {
        let spawn = authored_spawn(&RespawnMarker {
            spawn_class: "mp_tdm_spawn".into(),
            origin: [4.0, 5.0, 6.0],
            angles: [0.0, 180.0, 0.0],
            attached_to: "a".into(),
            script_destructable_area: "b".into(),
        });
        assert_eq!(spawn.kind_name, "mp_tdm_spawn");
        assert_eq!(spawn.script_tied, "a");
        assert_eq!(spawn.script_destructable_area, "b");
    }

    #[test]
    fn the_player_waits_on_the_camera_only_when_the_map_has_a_view() {
        assert!(waiting_on(true).contains("mp_global_intermission"));
        assert!(waiting_on(false).contains("no authored intermission"));
    }

    #[test]
    fn an_empty_catalog_has_no_kit_to_trace() {
        let kit = player_kit_collision(&asset_model::CharacterMeshRoster::default(), false);
        assert!(kit.torso_model_key.is_empty() && kit.face_model_key.is_empty());
        assert!(kit.body.is_none() && kit.head.is_none());
    }
}
