//! Class choice: what a bot picks on its own, what a menu answer or a "select class" request
//! makes of it, and the check that a class only asks for things this match really has.

use crate::frame::FrameWorld;
use crate::match_state::{EventAudience, PERSONAL_CLASS_SLOTS, SimEvent, perk_slot_from_class_catalog};
use crate::spawn::{FORCED_SPAWN_SOURCE, SpawnAttemptReport};
use crate::world::{ClientId, Tick};
use crate::{ClassDef, ClassId, ClassRejectReason, PersonalClass, SpawnPick};

/// The perk in the first slot that lets a class carry a second attachment.
const PERK_SECOND_ATTACHMENT: u32 = 12;

/// Lets a bot pick one of the classes the host configured for bots (in rotation), or the
/// default class of that index when there is none it may use.
pub(super) fn choose_bot_class(world: &mut FrameWorld, id: ClientId, index: u8) {
    let t5 = crate::script::is_t5(world.ecs());
    let offered = {
        let classes = &world.bootstrap_ref().bot_classes;
        (!classes.is_empty() && !t5).then(|| classes[index as usize % classes.len()].clone())
    };
    let chosen = offered.filter(|def| !def.locked && validate_class_content(world, def).is_ok());
    match chosen {
        Some(def) => crate::script::choose_class(world.ecs(), id.0, &def),
        None => crate::script::choose_default_class(world.ecs(), id.0, index),
    }
}

/// A "changeclass" menu answer of the form `customN` picks the player's Nth own class.
/// Returns whether the answer was one (so that nobody else should handle it), valid or not.
pub(super) fn answer_custom_class(world: &mut FrameWorld, id: ClientId, menu: &str, response: &str) -> bool {
    if !menu.eq_ignore_ascii_case("changeclass") {
        return false;
    }
    let slot = response
        .strip_prefix("custom")
        .and_then(|number| number.parse::<u32>().ok())
        .and_then(|number| number.checked_sub(1));
    let Some(slot) = slot else {
        return false;
    };
    if let Some(def) = crate::script::personal_class(world.ecs(), id.0, ClassId(slot)) {
        if validate_class_content(world, &def).is_ok() {
            crate::script::choose_class(world.ecs(), id.0, &def);
        }
    }
    true
}

/// A client sends the class it built; it must match the revision the host knows and pass the
/// content check, and then it becomes the class for the next spawn.
pub(super) fn apply_select_class(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    class_id: ClassId,
    revision: u32,
    loadout: PersonalClass,
) {
    let known = (class_id.0 < PERSONAL_CLASS_SLOTS as u32)
        .then(|| loadout.definition(class_id, revision))
        .flatten();
    let verdict = match known {
        None => Err(ClassRejectReason::UnknownOrStaleClass),
        Some(def) => validate_class_content(world, &def).map(|()| def),
    };

    let event = match verdict {
        Ok(def) => {
            crate::script::answer_join(world.ecs(), id.0);
            crate::script::choose_class(world.ecs(), id.0, &def);
            SimEvent::ClassAccepted { request_id, class_id, revision }
        }
        Err(reason) => {
            diag::info!(
                Sim,
                "class select: rejected client={} id={} rev={} reason={}",
                id.0,
                class_id.0,
                revision,
                reason.as_str()
            );
            SimEvent::ClassRejected { request_id, class_id, revision, reason }
        }
    };
    world.push_event(tick, EventAudience::Client(id), event);
}

/// Whether the weapons, perks and offhand items of a class exist in this match and are
/// allowed in the slots they were put in.
pub(super) fn validate_class_content(world: &FrameWorld, def: &ClassDef) -> Result<(), ClassRejectReason> {
    use ClassRejectReason::{LockedContent, UnknownWeaponId};

    let perk_misplaced = def
        .perks
        .iter()
        .copied()
        .enumerate()
        .any(|(slot, perk)| perk != 0 && perk_slot_from_class_catalog(perk) != Some(slot));
    if perk_misplaced {
        return Err(LockedContent);
    }

    let table_len = world.weapon_combat_len();
    let max_attachments = if def.perks[0] == PERK_SECOND_ATTACHMENT { 2 } else { 1 };
    for weapon in [def.primary, def.secondary].into_iter().filter(|&w| w != 0) {
        if weapon as usize >= table_len {
            return Err(UnknownWeaponId);
        }
        let row = world.weapon_combat_row(weapon).ok_or(UnknownWeaponId)?;
        let too_many_attachments = world
            .weapon_setup(weapon)
            .is_some_and(|setup| setup.attachments.len() > max_attachments);
        if !row.is_usable() || !world.weapon_runnable(weapon) || too_many_attachments {
            return Err(LockedContent);
        }
    }

    for (slot, weapon) in [def.lethal, def.tactical].into_iter().enumerate() {
        if weapon == 0 {
            continue;
        }
        if weapon as usize >= table_len {
            return Err(UnknownWeaponId);
        }
        let facts = world.offhand_loadout_row(weapon).ok_or(LockedContent)?;
        let right_slot = matches!((slot, facts.offhand_class), (0, 1 | 4 | 5) | (1, 2 | 3));
        if !world.weapon_runnable(weapon) || !right_slot {
            return Err(LockedContent);
        }
    }

    let has_attachment_picks = def
        .primary_attachments
        .iter()
        .chain(&def.secondary_attachments)
        .any(|&attachment| attachment != 0);
    if has_attachment_picks {
        return Err(LockedContent);
    }
    Ok(())
}

/// Writes one line to the log about how a forced spawn went.
pub(crate) fn log_forced_spawn(id: ClientId, pick: SpawnPick, report: &SpawnAttemptReport) {
    let asked = match pick {
        SpawnPick::Seeded(seed) => format!("seed={seed}"),
        SpawnPick::At { origin, yaw } => format!(
            "at=[{:.1}, {:.1}, {:.1}] yaw={yaw:.1}",
            origin[0], origin[1], origin[2]
        ),
    };
    let fallback = report
        .rejected
        .first()
        .filter(|(source, _)| *source == FORCED_SPAWN_SOURCE)
        .map(|(_, reason)| format!(" fallback={}", reason.as_str()))
        .unwrap_or_default();
    let Some(spawn) = &report.accepted else {
        diag::info!(
            Sim,
            "spawn: forced client={} {asked} refused tried={}{fallback}",
            id.0,
            report.tried
        );
        return;
    };
    let source = if spawn.source_index == FORCED_SPAWN_SOURCE {
        String::from("at")
    } else {
        spawn.source_index.to_string()
    };
    diag::info!(
        Sim,
        "spawn: forced client={} {asked} origin=[{:.1}, {:.1}, {:.1}] yaw={:.1} source={source} tried={}{fallback}",
        id.0,
        spawn.traced_origin[0],
        spawn.traced_origin[1],
        spawn.traced_origin[2],
        spawn.raw_angles[1],
        report.tried
    );
}
