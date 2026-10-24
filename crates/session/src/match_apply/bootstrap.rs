//! Starting the match in the simulation: where players can spawn, what the
//! classes on offer are, and which of them the loaded weapons can't back.

use std::sync::Arc;

use asset_game::WeaponRegistry;
use asset_world::SpawnPoint;
use frame::HostClassLoadouts;
use gamemode_t6::GameModeKind;
use net::AuthorityInputGate;
use render_frontend::prepare::scene::camera::SimCamera;

use super::refusal::InstallRefusal;
use crate::loadout::{AuthoritativeClassProjection, ClassRow, project_class};

/// The classes the host set up, or the showcase ones when it set none up.
fn bootstrap_class_rows(host: Option<&HostClassLoadouts>) -> Vec<ClassRow> {
    let fallback = HostClassLoadouts::default();
    let host = host
        .filter(|host| !host.slots.is_empty())
        .unwrap_or(&fallback);
    host.slots.iter().map(ClassRow::from).collect()
}

/// What the loaded weapons let each class be.
pub(super) struct Roster {
    classes: Vec<sim::ClassDef>,
    bot_classes: Vec<sim::ClassDef>,
    /// Why each class is locked, in class order; `None` for a usable one.
    pub lock_reasons: Vec<Option<String>>,
}

pub(super) fn roster(
    host: Option<&HostClassLoadouts>,
    weapons: &WeaponRegistry,
    combat: &[weapon_t6::combat_facts::WeaponCombatFacts],
    equipment: &[sim::EquipmentRuntimeFacts],
) -> Roster {
    let project = |index: usize, row: &ClassRow| -> AuthoritativeClassProjection {
        project_class(index as u32, row, weapons, combat, equipment)
    };
    let projected: Vec<_> = bootstrap_class_rows(host)
        .iter()
        .enumerate()
        .map(|(index, row)| project(index, row))
        .collect();
    let lock_reasons = projected
        .iter()
        .map(|row| row.lock_reason.clone())
        .collect();
    let classes = projected.into_iter().map(|row| row.def).collect();
    let bot_classes = frame::showcase_classes()
        .iter()
        .enumerate()
        .map(|(index, preset)| project(index, &ClassRow::from(&preset.into())).def)
        .collect();
    Roster {
        classes,
        bot_classes,
        lock_reasons,
    }
}

/// A place the map says the camera rests while players pick a class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct IntermissionView {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// What the match needs to be started.
pub(super) struct Start<'a> {
    pub kind: GameModeKind,
    pub spawns: &'a [SpawnPoint],
    pub view: Option<IntermissionView>,
    pub airstrike_height: Option<f32>,
    /// The score limit a test asked for, as typed.
    pub score_limit: Option<&'a str>,
}

pub(super) struct Started {
    /// What the launch report says the player is waiting on.
    pub gap: &'static str,
    pub lock_reasons: Vec<Option<String>>,
}

fn authored_spawn(spawn: &SpawnPoint) -> sim::AuthoredSpawnPoint {
    sim::AuthoredSpawnPoint {
        classname: spawn.classname.clone(),
        origin: spawn.origin,
        angles: spawn.angles,
        script_linkto: spawn.script_linkto.clone(),
        script_destructable_area: spawn.script_destructable_area.clone(),
    }
}

fn intermission_spawn(view: IntermissionView) -> sim::AuthoredSpawnPoint {
    sim::AuthoredSpawnPoint {
        classname: gamemode_t6::MP_GLOBAL_INTERMISSION.to_owned(),
        origin: view.origin,
        angles: view.angles,
        script_linkto: String::new(),
        script_destructable_area: String::new(),
    }
}

/// The score a match ends at: the test's if it gave a number, else the game
/// type's own.
fn score_limit(kind: GameModeKind, asked: Option<&str>) -> i32 {
    asked
        .and_then(|text| text.parse().ok())
        .unwrap_or_else(|| kind.default_score_limit())
}

fn waiting_on(has_view: bool) -> &'static str {
    if has_view {
        "player awaits SelectClass; mp_global_intermission holds the camera until Equip ack"
    } else {
        "player awaits SelectClass; Equip ack enables sim camera (no authored intermission)"
    }
}

/// Hand the simulation its spawns and classes, and park the player's camera
/// and input until the class is chosen.
pub(super) fn start_match(
    sim: &mut sim::SimWorld,
    start: Start,
    roster: Roster,
    camera: &mut SimCamera,
    gate: &mut AuthorityInputGate,
) -> Result<Started, InstallRefusal> {
    let Start {
        kind,
        spawns,
        view,
        airstrike_height,
        score_limit: asked,
    } = start;
    let Roster {
        classes,
        bot_classes,
        lock_reasons,
    } = roster;
    let class_count = classes.len();
    let result = sim.bootstrap(sim::MatchBootstrap {
        spawns: spawns.iter().map(authored_spawn).collect(),
        classes,
        bot_classes,
        seed: 0,
        kind,
        score_limit: score_limit(kind, asked),
        time_limit_ms: kind.default_time_limit_ms(),
        allow_debug_actions: true,
        intermission_view: view.map(intermission_spawn),
        airstrike_height,
    });
    if let Err(err) = result {
        diag::info!(Sim, "bootstrap: {err}");
        return Err(InstallRefusal::new(format!(
            "MatchBootstrap refused: {err}"
        )));
    }
    camera.enabled = false;
    gate.local_cmds_enabled = false;
    camera.freeze_fly = view.is_some();
    let locked = lock_reasons.iter().flatten().count();
    diag::info!(
        Sim,
        "spawn: match bootstrapped with {} dm spawns ({locked} / {class_count} classes Locked by coverage)",
        spawns.len(),
    );
    Ok(Started {
        gap: waiting_on(view.is_some()),
        lock_reasons,
    })
}

/// What the simulation traces a player's body and head against, for one team's
/// kit. A kit missing from the catalog traces against nothing.
pub(super) fn player_kit_collision(
    bodies: &asset_model::BodyMeshCatalog,
    axis: bool,
) -> sim::PlayerKitCollision {
    let kit = bodies.kits().kit(axis);
    let body_key = kit.map_or_else(String::new, |kit| kit.body.clone());
    let head_key = kit
        .and_then(|kit| kit.head.as_deref())
        .unwrap_or("")
        .to_owned();
    let capability = |key: &str| {
        if key.is_empty() {
            return None;
        }
        let entry = bodies.get(key)?;
        entry.skel.retained_capability().map(Arc::new)
    };
    sim::PlayerKitCollision {
        body: capability(&body_key),
        head: capability(&head_key),
        body_key,
        head_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_host_the_showcase_classes_are_offered() {
        let rows = bootstrap_class_rows(None);
        assert_eq!(rows.len(), HostClassLoadouts::default().slots.len());
        assert!(!rows.is_empty());
    }

    #[test]
    fn a_host_with_no_classes_gets_the_showcase_ones() {
        let host = HostClassLoadouts { slots: Vec::new() };
        assert_eq!(
            bootstrap_class_rows(Some(&host)).len(),
            bootstrap_class_rows(None).len()
        );
    }

    #[test]
    fn the_hosts_own_classes_are_offered_when_it_has_some() {
        let mut host = HostClassLoadouts::default();
        host.slots.truncate(2);
        assert_eq!(bootstrap_class_rows(Some(&host)).len(), 2);
    }

    #[test]
    fn a_test_can_ask_for_a_score_limit() {
        let kind = GameModeKind::TeamDeathmatch;
        assert_eq!(score_limit(kind, Some("17")), 17);
        assert_eq!(score_limit(kind, Some("many")), kind.default_score_limit());
        assert_eq!(score_limit(kind, None), kind.default_score_limit());
    }

    #[test]
    fn the_intermission_view_becomes_a_spawn_of_its_own_class() {
        let spawn = intermission_spawn(IntermissionView {
            origin: [1.0, 2.0, 3.0],
            angles: [0.0, 90.0, 0.0],
        });
        assert_eq!(spawn.classname, gamemode_t6::MP_GLOBAL_INTERMISSION);
        assert_eq!(spawn.origin, [1.0, 2.0, 3.0]);
        assert_eq!(spawn.angles, [0.0, 90.0, 0.0]);
        assert!(spawn.script_linkto.is_empty());
    }

    #[test]
    fn a_spawn_keeps_its_script_links() {
        let spawn = authored_spawn(&SpawnPoint {
            classname: "mp_tdm_spawn".into(),
            origin: [4.0, 5.0, 6.0],
            angles: [0.0, 180.0, 0.0],
            script_linkto: "a".into(),
            script_destructable_area: "b".into(),
        });
        assert_eq!(spawn.classname, "mp_tdm_spawn");
        assert_eq!(spawn.script_linkto, "a");
        assert_eq!(spawn.script_destructable_area, "b");
    }

    #[test]
    fn the_player_waits_on_the_camera_only_when_the_map_has_a_view() {
        assert!(waiting_on(true).contains("mp_global_intermission"));
        assert!(waiting_on(false).contains("no authored intermission"));
    }

    #[test]
    fn an_empty_catalog_has_no_kit_to_trace() {
        let kit = player_kit_collision(&asset_model::BodyMeshCatalog::default(), false);
        assert!(kit.body_key.is_empty() && kit.head_key.is_empty());
        assert!(kit.body.is_none() && kit.head.is_none());
    }
}
