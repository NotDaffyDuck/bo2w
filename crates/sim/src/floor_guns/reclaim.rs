//! Picking a thrown knife, throwing blade or sticky grenade back up from where it landed.

use entity_t6::EventCode;
use entity_t6::flight::path_kind::AT_REST;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState};

use super::{announce_pickup, store_player};
use super::inventory::{Rounds, held_rounds, set_held_rounds};
use crate::step_view::StepWorldView;
use crate::sim_parts::{SeatNo, StepNo};
use crate::{GrabbedItemNote, FlyingProjectile};

/// What the player would hold for `thrown`'s weapon once they took it back, or `None` if it is not theirs to
/// take: they must carry the weapon, and it must be a retrievable knife, a blade, or one of their own sticky
/// throwables still waiting to go off.
pub(super) fn rounds_after_reclaim(
    world: &StepWorldView,
    taker: SeatNo,
    ps: &AvatarState,
    thrown: &FlyingProjectile,
) -> Option<Rounds> {
    let weapon = thrown.gun;
    let carried = ps.weapons.contains(&(weapon as i32));
    if !carried {
        return None;
    }
    let facts = world.gadget_stats_lookup(weapon)?;
    let own_waiting_sticky = || {
        facts.is_secondary_gear()
            && facts.stickiness != 0
            && !facts.timed_detonation
            && !facts.shell_detonate_on_hit
            && thrown.detonate_at_ms.is_none()
            && thrown.owner == taker
            && world.client_meta(taker).is_some_and(|meta| meta.life_sequence == thrown.owner_respawn_count)
    };
    if !facts.is_retrievable_knife() && !own_waiting_sticky() {
        return None;
    }

    let held = held_rounds(world, ps, weapon);
    if facts.ballistic_blade {
        // A blade not in hand with an empty magazine comes back into the magazine; otherwise it joins the reserve.
        if ps.weapon != weapon && held.right == 0 {
            return Some(Rounds { right: 1, ..held });
        }
        let cap = world.damage_profile_of(weapon)?.reserve_cap;
        (held.reserve < cap).then_some(Rounds { reserve: held.reserve + 1, ..held })
    } else {
        (held.right < facts.magazine_capacity.max(1)).then_some(Rounds { right: held.right + 1, ..held })
    }
}

/// The player picks up the thrown weapon `number` that lies still; it goes back into their ammunition.
pub(super) fn reclaim(world: &mut StepWorldView, walker: SeatNo, slot_no: i32, tick: StepNo) {
    let Some(projectile) = world.projectile_in_slot(slot_no) else {
        return;
    };
    let Some(mut ps) = world.avatar(walker).copied() else {
        return;
    };
    if projectile.at.tr_type != AT_REST {
        return;
    }
    let weapon = projectile.gun;
    let Some(after) = rounds_after_reclaim(world, walker, &ps, &projectile) else {
        return;
    };
    let before = held_rounds(world, &ps, weapon);
    set_held_rounds(world, &mut ps, weapon, after);
    store_player(world, walker, ps);

    let record = world.client_meta_mut(walker);
    record.store_ammo_counts(weapon, after.right, after.reserve);
    record.copy_carried_ammo(ps.weapon);
    world.drop_projectile_slot(slot_no);
    world.free_dynamic_entity_number(slot_no);

    let picker = walker.0 as i32;
    let nothing_left_behind = NO_ENTITY_SLOT;
    let entry = GrabbedItemNote {
        from_entnum: slot_no,
        clip_r: after.right - before.right,
        clip_l: 0,
        stock: after.reserve - before.reserve,
        picker_pm_type: ps.movement_mode,
        swapped_entnum: nothing_left_behind,
        weapon,
        picker,
        ammo_only: true,
    };
    world.item_pickups_mut().push(entry);
    announce_pickup(world, tick, EventCode::AMMO_PICKUP, walker, weapon, ps.origin);
    perf::pickup(ps.movement_mode);
}
