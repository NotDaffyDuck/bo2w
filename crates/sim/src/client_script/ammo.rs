//! Magazines and reserve ammunition: reading, setting and refilling what a player carries for each weapon.

use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use weapon_t6::ammo_table::{
    ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, set_ammo_not_in_clip,
    set_clip_for_hand,
};
use weapon_t6::combat_facts::spawn_clip_stock;

/// Hand index of the right (or only) gun's magazine and of a dual-wield weapon's left one.
const RIGHT_HAND: usize = 0;
const LEFT_HAND: usize = 1;

/// The rounds in the magazine of one hand of `weapon`; 0 for a player or weapon that is not there.
fn magazine(world: &StepWorldView, id: SeatNo, gun: u32, hand: usize) -> i32 {
    let (Some(ps), Some(facts)) = (world.avatar(id), world.damage_profile_of(gun)) else {
        return 0;
    };
    get_clip_for_hand(&ps.magazine_rows, clip_table_key(facts.motion_clip_slot, gun), hand as u8)
}

/// Loads one hand's magazine, held to the weapon's clip size. Returns the count stored, or `None` for a weapon with
/// no combat data.
fn load_magazine(world: &mut StepWorldView, id: SeatNo, gun: u32, grip: usize, count: i32) -> Option<i32> {
    let facts = world.damage_profile_of(gun)?;
    let count = count.clamp(0, facts.magazine_capacity.max(0));
    let key = clip_table_key(facts.motion_clip_slot, gun);
    if let Some(ps) = world.occupant_mut(id) {
        let _ = set_clip_for_hand(&mut ps.magazine_rows, key, grip as u8, count);
    }
    Some(count)
}

pub(crate) fn ammo_clip(world: &StepWorldView, id: SeatNo, gun: u32) -> i32 {
    magazine(world, id, gun, RIGHT_HAND)
}

/// For a dual-wield weapon named by its left gun (scripts read and set the left clip through that name): the held
/// weapon whose left hand it is. Nothing when the player holds `weapon` itself.
pub(crate) fn dual_wield_right_of(world: &StepWorldView, id: SeatNo, weapon: u32) -> Option<u32> {
    let pstate = world.avatar(id)?;
    if weapon == 0 || pstate.weapons.contains(&(weapon as i32)) {
        return None;
    }
    let is_partner = |held: u32| {
        world
            .damage_profile_of(held)
            .is_some_and(|facts| facts.dual_wield_weapon == weapon)
    };
    pstate.weapons
        .iter()
        .filter(|&&slot| slot > 0)
        .map(|&slot| slot as u32)
        .find(|&held| is_partner(held))
}

/// A dual-wield weapon's left magazine.
pub(crate) fn left_clip(world: &StepWorldView, id: SeatNo, gun: u32) -> i32 {
    magazine(world, id, gun, LEFT_HAND)
}

/// Loads a dual-wield weapon's left magazine.
pub(crate) fn set_left_clip(world: &mut StepWorldView, id: SeatNo, gun: u32, rounds: i32) {
    load_magazine(world, id, gun, LEFT_HAND, rounds);
}

/// The reserve rounds: the ammunition not in the magazine.
pub(crate) fn rounds_held(world: &StepWorldView, id: SeatNo, gun: u32) -> i32 {
    let (Some(ps), Some(facts)) = (world.avatar(id), world.damage_profile_of(gun)) else {
        return 0;
    };
    get_ammo_not_in_clip(&ps.ammo, ammo_table_key(facts.ammo_type_slot, gun))
}

/// Loads the magazine (right hand) and keeps the client record of the weapon's ammo in step.
pub(crate) fn set_ammo_clip(world: &mut StepWorldView, id: SeatNo, gun: u32, rounds: i32) {
    let Some(loaded) = load_magazine(world, id, gun, RIGHT_HAND, rounds) else {
        return;
    };
    let stock = rounds_held(world, id, gun);
    world.client_meta_mut(id).store_ammo_counts(gun, loaded, stock);
}

/// Sets the reserve rounds, held to the most the weapon carries.
pub(crate) fn set_ammo_stock(world: &mut StepWorldView, id: SeatNo, gun: u32, rounds: i32) {
    let Some(facts) = world.damage_profile_of(gun) else {
        return;
    };
    let rounds = rounds.clamp(0, facts.reserve_cap.max(0));
    let key = ammo_table_key(facts.ammo_type_slot, gun);
    if let Some(ps) = world.occupant_mut(id) {
        let _ = set_ammo_not_in_clip(&mut ps.ammo, key, rounds);
    }
    let clip = ammo_clip(world, id, gun);
    world.client_meta_mut(id).store_ammo_counts(gun, clip, rounds);
}

/// Fills a weapon to the most it carries. A grenade or mine has no reserve: its magazine is filled instead.
pub(crate) fn give_max_ammo(world: &mut StepWorldView, id: SeatNo, gun: u32) {
    let Some(facts) = world.damage_profile_of(gun) else {
        return;
    };
    match world.gadget_stats_lookup(gun).filter(|eq| eq.is_secondary_gear()) {
        Some(eq) => set_ammo_clip(world, id, gun, eq.magazine_capacity.max(eq.opening_ammo)),
        None => set_ammo_stock(world, id, gun, facts.reserve_cap),
    }
}

/// How full a weapon is: its magazine and reserve over the most it carries, or with `start` over what it spawns with
/// (`getfractionmaxammo` / `getfractionstartammo`). A grenade or mine is counted by its magazine alone.
pub(crate) fn ammo_fraction(world: &StepWorldView, id: SeatNo, gun: u32, start: bool) -> f32 {
    let Some(facts) = world.damage_profile_of(gun) else {
        return 0.0;
    };
    let have = ammo_clip(world, id, gun) + rounds_held(world, id, gun);
    let offhand = world.gadget_stats_lookup(gun).filter(|eq| eq.is_secondary_gear());
    let capacity = match (offhand, start) {
        (Some(eq), true) => eq.starting_magazine_count(),
        (Some(eq), false) => eq.magazine_capacity.max(eq.opening_ammo),
        (None, true) => {
            let (clip, _, stock) = spawn_clip_stock(&facts, 0);
            clip + stock
        }
        (None, false) => facts.reserve_cap + facts.magazine_capacity,
    };
    if capacity <= 0 {
        0.0
    } else {
        have as f32 / capacity as f32
    }
}

/// `giveStartAmmo`, only for a weapon the player holds: the gun is topped up to its start ammunition (never cut down
/// to it), its magazine is filled from the reserve, and its alternate weapon (a launcher attachment) gets the same.
pub(crate) fn give_start_ammo(world: &mut StepWorldView, seat: SeatNo, gun: u32) {
    let holds = gun != 0
        && world
            .avatar(seat)
            .is_some_and(|ps| ps.weapons.contains(&(gun as i32)));
    if !holds {
        return;
    }
    start_ammo_one(world, seat, gun);
    let single_handed = world.damage_profile_of(gun).filter(|facts| facts.dual_wield_weapon == 0);
    let alt = single_handed.map_or(0, |facts| facts.alternate_weapon);
    if alt != 0 && alt != gun {
        start_ammo_one(world, seat, alt);
    }
}

fn start_ammo_one(world: &mut StepWorldView, seat: SeatNo, gun: u32) {
    let Some(facts) = world.damage_profile_of(gun) else {
        return;
    };
    if let Some(eq) = world.gadget_stats_lookup(gun).filter(|eq| eq.is_secondary_gear()) {
        let loaded = ammo_clip(world, seat, gun);
        set_ammo_clip(world, seat, gun, loaded.max(eq.starting_magazine_count()));
        return;
    }
    // Other held guns drawing on the same ammunition pool count their magazines, less their own start ammunition,
    // against it.
    let pool = ammo_table_key(facts.ammo_type_slot, gun);
    let held: Vec<u32> = world
        .avatar(seat)
        .map(|pstate| {
            pstate.weapons
                .iter()
                .filter(|&&slot| slot > 0)
                .map(|&slot| slot as u32)
                .collect()
        })
        .unwrap_or_default();
    let mut others = 0;
    for other in held.into_iter().filter(|&w| w != gun) {
        let Some(other_facts) = world.damage_profile_of(other) else {
            continue;
        };
        if ammo_table_key(other_facts.ammo_type_slot, other) == pool {
            others += ammo_clip(world, seat, other) - other_facts.spawn_rounds.max(0);
        }
    }
    let dual = facts.dual_wield_weapon != 0;
    let left = if dual { left_clip(world, seat, gun) } else { 0 };
    let (right, left, stock) = topped_up(
        StartAmmo {
            start: facts.spawn_rounds.max(0),
            magazine: facts.magazine_capacity.max(0),
            max_stock: facts.reserve_cap.max(0),
            dual,
        },
        Carried {
            right: ammo_clip(world, seat, gun),
            left,
            stock: rounds_held(world, seat, gun),
            others,
        },
    );
    if dual {
        set_left_clip(world, seat, gun, left);
    }
    set_ammo_stock(world, seat, gun, stock);
    set_ammo_clip(world, seat, gun, right);
}

/// A gun's ammunition limits.
struct StartAmmo {
    start: i32,
    magazine: i32,
    max_stock: i32,
    /// Both hands hold a magazine.
    dual: bool,
}

/// What a player carries for a gun. `others` is the other guns' magazines on the same pool less their start ammunition.
struct Carried {
    right: i32,
    left: i32,
    stock: i32,
    others: i32,
}

/// Moves up to a magazine's room from the pool into it.
fn refill(clip: i32, magazine_capacity: i32, pool: &mut i32) -> i32 {
    let moved = (magazine_capacity - clip).max(0).min((*pool).max(0));
    *pool -= moved;
    clip + moved
}

/// G_GetNeededStartAmmo, Add_Ammo and Fill_Clip on one gun: whatever it lacks of its start ammunition is added to
/// the reserve (up to the most it holds), then the magazines fill from the reserve. Returns the right magazine, the
/// left magazine and the reserve.
fn topped_up(limits: StartAmmo, have: Carried) -> (i32, i32, i32) {
    let lacking = limits.start - (have.right + have.left + have.stock + have.others);
    let mut pool = have.stock;
    if lacking >= 1 {
        pool = pool.saturating_add(lacking).min(limits.max_stock);
    }
    let right = refill(have.right, limits.magazine, &mut pool);
    let left = if limits.dual {
        refill(have.left, limits.magazine, &mut pool)
    } else {
        have.left
    };
    (right, left, pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tops up a gun with `start` / `magazine_capacity` / `max_stock` that carries (`right`, `left`, `stock`, `others`).
    fn top_up(
        (start, magazine_capacity, max_stock, dual): (i32, i32, i32, bool),
        (right, left, stock, others): (i32, i32, i32, i32),
    ) -> (i32, i32, i32) {
        topped_up(
            StartAmmo { start, magazine: magazine_capacity, max_stock, dual },
            Carried { right, left, stock, others },
        )
    }

    #[test]
    fn a_short_gun_is_topped_up_and_its_magazine_filled() {
        // Start ammunition 120 is 30 + 90; he carries 4 and 20.
        assert_eq!(top_up((120, 30, 270, false), (4, 0, 20, 0)), (30, 0, 90));
    }

    #[test]
    fn a_gun_over_its_start_ammunition_is_not_cut_down() {
        assert_eq!(top_up((120, 30, 270, false), (10, 0, 200, 0)), (30, 0, 180));
    }

    #[test]
    fn a_dual_wield_gun_fills_both_magazines() {
        assert_eq!(top_up((60, 15, 120, true), (0, 0, 0, 0)), (15, 15, 30));
    }

    #[test]
    fn the_reserve_is_never_topped_past_the_most_it_holds() {
        assert_eq!(top_up((500, 30, 270, false), (0, 0, 0, 0)), (30, 0, 240));
    }

    #[test]
    fn guns_on_the_same_pool_share_the_start_ammunition() {
        // Another gun on the pool holds 20 over its own start ammunition, so this one is owed 20 fewer.
        assert_eq!(top_up((120, 30, 270, false), (0, 0, 70, 0)), (30, 0, 90));
        assert_eq!(top_up((120, 30, 270, false), (0, 0, 70, 20)), (30, 0, 70));
    }

    #[test]
    fn a_magazine_takes_only_what_the_pool_has() {
        let mut pool = 5;
        assert_eq!(refill(10, 30, &mut pool), 15);
        assert_eq!(pool, 0);
        let mut pool = 100;
        assert_eq!(refill(40, 30, &mut pool), 40);
        assert_eq!(pool, 100);
    }
}
