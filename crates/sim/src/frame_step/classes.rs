//! Class choice: what a bot picks on its own, what a menu answer or a "select class" request
//! makes of it, and the check that a class only asks for things this match really has.

use crate::step_view::StepWorldView;
use crate::player_ledger::{EventRecipients, PERSONAL_CLASS_SLOTS, MatchOccurrence};
use crate::spawn_pick::{FORCED_SPAWN_SOURCE, SpawnTryOutcome};
use crate::sim_parts::{SeatNo, StepNo};
use crate::{LoadoutClassSpec, LoadoutClass, LoadoutRefusal, PersonalClass, SpawnPick};

/// How many attachments a class may put on one gun.
const MAX_ATTACHMENTS: usize = 2;

/// Lets a bot pick one of the classes the host configured for bots (in rotation), or the
/// default class of that index when there is none it may use.
pub(super) fn choose_bot_class(world: &mut StepWorldView, id: SeatNo, index: u8) {
    let offered = {
        let classes = &world.bootstrap_ref().bot_classes;
        (!classes.is_empty()).then(|| classes[index as usize % classes.len()].clone())
    };
    let chosen = offered.filter(|def| !def.locked && validate_class_content(world, def).is_ok());
    if let Some(def) = chosen {
        crate::gsc_host::choose_class(world.ecs(), id.0, &def);
    }
}

/// A "changeclass" menu answer of the form `customN` picks the player's Nth own class.
/// Returns whether the answer was one (so that nobody else should handle it), valid or not.
pub(super) fn answer_custom_class(world: &mut StepWorldView, id: SeatNo, menu: &str, response: &str) -> bool {
    if !menu.eq_ignore_ascii_case("changeclass") {
        return false;
    }
    let own_index = response
        .strip_prefix("custom")
        .and_then(|number| number.parse::<u32>().ok())
        .and_then(|number| number.checked_sub(1));
    let Some(own_index) = own_index else { return false };
    if let Some(def) = crate::gsc_host::personal_class(world.ecs(), id.0, LoadoutClass(own_index)) {
        if validate_class_content(world, &def).is_ok() {
            crate::gsc_host::choose_class(world.ecs(), id.0, &def);
        }
    }
    true
}

/// A client sends the class it built; it must match the revision the host knows and pass the
/// content check, and then it becomes the class for the next spawn.
pub(super) fn apply_select_class(
    level: &mut StepWorldView,
    step: StepNo,
    id: SeatNo,
    req_id: u32,
    picked: LoadoutClass,
    stamp: u32,
    loadout: PersonalClass,
) {
    let known = (picked.0 < PERSONAL_CLASS_SLOTS as u32)
        .then(|| loadout.definition(picked, stamp))
        .flatten();
    let verdict = match known {
        None => Err(LoadoutRefusal::UnknownOrStaleClass),
        Some(def) => validate_class_content(level, &def).map(|()| def),
    };

    let event = match verdict {
        Ok(def) => {
            crate::gsc_host::choose_class(level.ecs(), id.0, &def);
            MatchOccurrence::LoadoutTaken { request_id: req_id, class_id: picked, revision: stamp }
        }
        Err(harm_kind) => {
            diag::info!(
                Sim,
                "class select refused: client={} class={} rev={} cause={}",
                id.0,
                picked.0,
                stamp,
                harm_kind.as_str()
            );
            MatchOccurrence::LoadoutRefused { request_id: req_id, class_id: picked, revision: stamp, reason: harm_kind }
        }
    };
    level.log_occurrence(step, EventRecipients::Client(id), event);
}

/// Whether the weapons, perks and offhand items of a class exist in this match and are
/// allowed in the slots they were put in.
pub(super) fn validate_class_content(world: &StepWorldView, def: &LoadoutClassSpec) -> Result<(), LoadoutRefusal> {
    use LoadoutRefusal::{LockedContent, NoSuchGun};

    let offered = &world.bootstrap_ref().class_perk_slots;
    let perk_misplaced = def.perks.iter().copied().enumerate().any(|(slot, perk)| {
        perk != 0 && !offered.iter().any(|&(id, at)| id == perk && usize::from(at) == slot)
    });
    if perk_misplaced {
        return Err(LockedContent);
    }

    let known_weapons = world.weapon_stats_count();
    for gun in [def.primary, def.secondary].into_iter().filter(|&w| w != 0) {
        if gun as usize >= known_weapons {
            return Err(NoSuchGun);
        }
        let row = world.weapon_stats_entry(gun).ok_or(NoSuchGun)?;
        let too_many_attachments = world
            .weapon_setup(gun)
            .is_some_and(|setup| setup.attachments.len() > MAX_ATTACHMENTS);
        if !row.ready_for_use() || !world.weapon_runnable(gun) || too_many_attachments {
            return Err(LockedContent);
        }
    }

    for (slot, gun) in [def.lethal, def.tactical].into_iter().enumerate() {
        if gun == 0 {
            continue;
        }
        if gun as usize >= known_weapons {
            return Err(NoSuchGun);
        }
        let facts = world.side_slot_row(gun).ok_or(LockedContent)?;
        let right_slot = matches!((slot, facts.secondary_hand_category), (0, 1 | 4 | 5) | (1, 2 | 3));
        if !world.weapon_runnable(gun) || !right_slot {
            return Err(LockedContent);
        }
    }

    let has_attachment_picks = [&def.primary_attachments, &def.secondary_attachments]
        .into_iter()
        .flatten()
        .any(|&attachment| attachment != 0);
    if has_attachment_picks {
        return Err(LockedContent);
    }
    Ok(())
}

/// Writes one line to the log about how a forced spawn went.
pub(crate) fn log_forced_spawn(id: SeatNo, pick: SpawnPick, report: &SpawnTryOutcome) {
    let asked = match pick {
        SpawnPick::Seeded(seed) => format!("seed={seed}"),
        SpawnPick::At { origin, yaw } => format!(
            "at=[{:.1}, {:.1}, {:.1}] yaw={yaw:.1}",
            origin[0], origin[1], origin[2]
        ),
    };
    let fallback = report
        .refused
        .first()
        .filter(|(source, _)| *source == FORCED_SPAWN_SOURCE)
        .map(|(_, reason)| format!(" fallback={}", reason.as_str()))
        .unwrap_or_default();
    let tried = report.attempts;
    let Some(spawn) = &report.accepted else {
        diag::info!(
            Sim,
            "spawn: forced client={} {asked} refused tried={tried}{fallback}",
            id.0
        );
        return;
    };
    let source = if spawn.origin_row == FORCED_SPAWN_SOURCE {
        String::from("at")
    } else {
        spawn.origin_row.to_string()
    };
    diag::info!(
        Sim,
        "spawn: forced client={} {asked} origin=[{:.1}, {:.1}, {:.1}] yaw={:.1} source={source} tried={tried}{fallback}",
        id.0,
        spawn.dropped_origin[0],
        spawn.dropped_origin[1],
        spawn.dropped_origin[2],
        spawn.authored_angles[1]
    );
}
