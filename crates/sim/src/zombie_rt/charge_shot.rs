//! Black Ops II's charge shot (fire type 8): Origins' upgraded staffs. The
//! weapon brain charges and fires it (PM_UpdateChargeShot,
//! ShouldFireChargeShotWeapon, G_GetWeaponDefBasedOnNumberOfBullets); this
//! is the scripts' view of it: `self.chargeshotlevel` and the staff's charge
//! sounds (tomb_staff_charge).
//!
//! Ammo: Origins sets `tu14_bg_chargeShotExponentialAmmoPerChargeLevel`
//! (zm_tomb.gsc), so a level costs more than one round: a full charge
//! (level 3) takes 6 rounds, measured on all four staffs; level 2 takes 2
//! (unmeasured: 1, 2, 6 read as level factorial). `sync_dvars` hands the
//! script's setting to the brain.

use bevy_ecs::prelude::World;
use weapon_t6::player::wf;

use super::{Zm, frame, with_vm};
use crate::sim_parts::ClientId;

/// The script dvar Origins sets.
const EXPONENTIAL_AMMO: &str = "tu14_bg_chargeshotexponentialammoperchargelevel";

/// Each frame, before the weapon ticks read it: the scripts' charge-shot ammo setting.
pub(super) fn sync_dvars(world: &mut World) {
    let on = with_vm(world, |vm, _| {
        let hashed = format!("#{:08x}", gsc_t6::hash_name(EXPONENTIAL_AMMO));
        [EXPONENTIAL_AMMO, hashed.as_str()]
            .iter()
            .filter_map(|k| vm.dvars.get(*k))
            .any(|v| gsc_t6::math::parse_int(v) != 0)
    })
    .unwrap_or(false);
    if let Some(mut zm) = world.get_resource_mut::<Zm>()
        && zm.charge_ammo_exponential != on
    {
        zm.charge_ammo_exponential = on;
    }
}

/// The weapon charging and its level, while fire is held past the press.
pub(crate) fn charging(world: &mut World, id: ClientId) -> Option<(u32, i32)> {
    crate::gun_brain::script_read(&frame(world), id, |ps, _| {
        (ps.weapon_state_bits & wf::CHARGE_SHOT != 0 && ps.charge_shot_level > 0)
            .then_some((ps.weapon, ps.charge_shot_level))
    })
    .flatten()
}

/// `self.chargeshotlevel`: the level charging, or of the shot just fired (the brain keeps
/// it until the rechamber ends).
pub(crate) fn level(world: &mut World, id: ClientId) -> i32 {
    crate::gun_brain::script_read(&frame(world), id, |ps, _| ps.charge_shot_level).unwrap_or(0)
}
