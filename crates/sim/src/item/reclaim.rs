//! Picking a thrown knife, throwing blade or sticky grenade back up from where it landed.

use entity_t6::EntityEventKind;
use entity_t6::trajectory::tr_type::STATIONARY;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};

use super::{announce_pickup, store_player};
use super::inventory::{Rounds, held_rounds, set_held_rounds};
use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick};
use crate::{ItemPickupRecord, ProjectileState};

/// What the player would hold for `projectile`'s weapon once they took it back, or `None` if it is not theirs to
/// take: they must carry the weapon, and it must be a retrievable knife, a blade, or one of their own sticky
/// throwables still waiting to go off.
pub(super) fn rounds_after_reclaim(
    world: &FrameWorld,
    walker: ClientId,
    ps: &PlayerState,
    projectile: &ProjectileState,
) -> Option<Rounds> {
    let weapon = projectile.weapon;
    if !ps.weapons.contains(&(weapon as i32)) {
        return None;
    }
    let facts = world.equipment_facts_for(weapon)?;
    let own_waiting_sticky = || {
        facts.is_offhand()
            && facts.stickiness != 0
            && !facts.timed_detonation
            && !facts.proj_impact_explode
            && projectile.detonate_at_ms.is_none()
            && projectile.owner == walker
            && world.client_meta(walker).is_some_and(|meta| meta.life_sequence == projectile.owner_life)
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
        let cap = world.combat_facts_for(weapon)?.max_ammo;
        (held.reserve < cap).then_some(Rounds { reserve: held.reserve + 1, ..held })
    } else {
        (held.right < facts.clip_size.max(1)).then_some(Rounds { right: held.right + 1, ..held })
    }
}

/// The player picks up the thrown weapon `number` that lies still; it goes back into their ammunition.
pub(super) fn reclaim(world: &mut FrameWorld, walker: ClientId, number: i32, tick: Tick) {
    let Some(projectile) = world.projectile_by_number(number) else {
        return;
    };
    let Some(mut ps) = world.player(walker).copied() else {
        return;
    };
    if projectile.pos.tr_type != STATIONARY {
        return;
    }
    let weapon = projectile.weapon;
    let Some(after) = rounds_after_reclaim(world, walker, &ps, &projectile) else {
        return;
    };
    let before = held_rounds(world, &ps, weapon);
    set_held_rounds(world, &mut ps, weapon, after);
    store_player(world, walker, ps);

    let meta = world.client_meta_mut(walker);
    meta.set_ammo(weapon, after.right, after.reserve);
    meta.mirror_held_ammo(ps.weapon);
    world.remove_projectile_by_number(number);
    world.free_dynamic_entity_number(number);

    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon,
        from_entnum: number,
        clip_r: after.right - before.right,
        clip_l: 0,
        stock: after.reserve - before.reserve,
        swapped_entnum: ENTITYNUM_NONE,
        picker_pm_type: ps.pm_type,
        ammo_only: true,
    });
    announce_pickup(world, tick, EntityEventKind::AMMO_PICKUP, walker, weapon, ps.origin);
    perf::pickup(ps.pm_type);
}
