//! Magazines and reserve ammunition: reading, setting and refilling what a player carries for each weapon.

use crate::frame::FrameWorld;
use crate::world::ClientId;
use weapon_t6::ammo_table::{
    ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, set_ammo_not_in_clip,
    set_clip_for_hand,
};
use weapon_t6::combat_facts::spawn_clip_stock;

/// Hand index of the right (or only) gun's magazine and of a dual-wield weapon's left one.
const RIGHT_HAND: usize = 0;
const LEFT_HAND: usize = 1;

/// The rounds in the magazine of one hand of `weapon`; 0 for a player or weapon that is not there.
fn magazine(world: &FrameWorld, id: ClientId, weapon: u32, hand: usize) -> i32 {
    let (Some(ps), Some(facts)) = (world.player(id), world.combat_facts_for(weapon)) else {
        return 0;
    };
    get_clip_for_hand(&ps.ammoclip, clip_table_key(facts.clip_index, weapon), hand as u8)
}

/// Loads one hand's magazine, held to the weapon's clip size. Returns the count stored, or `None` for a weapon with
/// no combat data.
fn load_magazine(world: &mut FrameWorld, id: ClientId, weapon: u32, hand: usize, count: i32) -> Option<i32> {
    let facts = world.combat_facts_for(weapon)?;
    let count = count.clamp(0, facts.clip_size.max(0));
    let key = clip_table_key(facts.clip_index, weapon);
    if let Some(ps) = world.player_mut(id) {
        let _ = set_clip_for_hand(&mut ps.ammoclip, key, hand as u8, count);
    }
    Some(count)
}

pub(crate) fn ammo_clip(world: &FrameWorld, id: ClientId, weapon: u32) -> i32 {
    magazine(world, id, weapon, RIGHT_HAND)
}

/// For a dual-wield weapon named by its left gun (scripts read and set the left clip through that name): the held
/// weapon whose left hand it is. Nothing when the player holds `weapon` itself.
pub(crate) fn dual_wield_right_of(world: &FrameWorld, id: ClientId, weapon: u32) -> Option<u32> {
    let ps = world.player(id)?;
    if weapon == 0 || ps.weapons.contains(&(weapon as i32)) {
        return None;
    }
    let is_partner = |held: u32| {
        world
            .combat_facts_for(held)
            .is_some_and(|facts| facts.dual_wield_weapon == weapon)
    };
    ps.weapons
        .iter()
        .filter(|&&slot| slot > 0)
        .map(|&slot| slot as u32)
        .find(|&held| is_partner(held))
}

/// A dual-wield weapon's left magazine.
pub(crate) fn left_clip(world: &FrameWorld, id: ClientId, weapon: u32) -> i32 {
    magazine(world, id, weapon, LEFT_HAND)
}

/// Loads a dual-wield weapon's left magazine.
pub(crate) fn set_left_clip(world: &mut FrameWorld, id: ClientId, weapon: u32, count: i32) {
    load_magazine(world, id, weapon, LEFT_HAND, count);
}

/// The reserve rounds: the ammunition not in the magazine.
pub(crate) fn ammo_stock(world: &FrameWorld, id: ClientId, weapon: u32) -> i32 {
    let (Some(ps), Some(facts)) = (world.player(id), world.combat_facts_for(weapon)) else {
        return 0;
    };
    get_ammo_not_in_clip(&ps.ammo, ammo_table_key(facts.ammo_index, weapon))
}

/// Loads the magazine (right hand) and keeps the client record of the weapon's ammo in step.
pub(crate) fn set_ammo_clip(world: &mut FrameWorld, id: ClientId, weapon: u32, count: i32) {
    let Some(loaded) = load_magazine(world, id, weapon, RIGHT_HAND, count) else {
        return;
    };
    let stock = ammo_stock(world, id, weapon);
    world.client_meta_mut(id).set_ammo(weapon, loaded, stock);
}

/// Sets the reserve rounds, held to the most the weapon carries.
pub(crate) fn set_ammo_stock(world: &mut FrameWorld, id: ClientId, weapon: u32, count: i32) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    let count = count.clamp(0, facts.max_ammo.max(0));
    let key = ammo_table_key(facts.ammo_index, weapon);
    if let Some(ps) = world.player_mut(id) {
        let _ = set_ammo_not_in_clip(&mut ps.ammo, key, count);
    }
    let clip = ammo_clip(world, id, weapon);
    world.client_meta_mut(id).set_ammo(weapon, clip, count);
}

/// Fills a weapon to the most it carries. A grenade or mine has no reserve: its magazine is filled instead.
pub(crate) fn give_max_ammo(world: &mut FrameWorld, id: ClientId, weapon: u32) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    match world.equipment_facts_for(weapon).filter(|eq| eq.is_offhand()) {
        Some(eq) => set_ammo_clip(world, id, weapon, eq.clip_size.max(eq.start_ammo)),
        None => set_ammo_stock(world, id, weapon, facts.max_ammo),
    }
}

/// How full a weapon is: its magazine and reserve over the most it carries, or with `start` over what it spawns with
/// (`getfractionmaxammo` / `getfractionstartammo`). A grenade or mine is counted by its magazine alone.
pub(crate) fn ammo_fraction(world: &FrameWorld, id: ClientId, weapon: u32, start: bool) -> f32 {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return 0.0;
    };
    let have = ammo_clip(world, id, weapon) + ammo_stock(world, id, weapon);
    let offhand = world.equipment_facts_for(weapon).filter(|eq| eq.is_offhand());
    let capacity = match (offhand, start) {
        (Some(eq), true) => eq.spawn_clip_count(),
        (Some(eq), false) => eq.clip_size.max(eq.start_ammo),
        (None, true) => {
            let (clip, _, stock) = spawn_clip_stock(&facts, 0);
            clip + stock
        }
        (None, false) => facts.max_ammo + facts.clip_size,
    };
    if capacity <= 0 {
        0.0
    } else {
        have as f32 / capacity as f32
    }
}

/// `giveStartAmmo`, only for a weapon the player holds: the gun is topped up to its start ammunition (never cut down
/// to it), its magazine is filled from the reserve, and its alternate weapon (a launcher attachment) gets the same.
pub(crate) fn give_start_ammo(world: &mut FrameWorld, id: ClientId, weapon: u32) {
    let holds = weapon != 0
        && world
            .player(id)
            .is_some_and(|ps| ps.weapons.contains(&(weapon as i32)));
    if !holds {
        return;
    }
    start_ammo_one(world, id, weapon);
    let alt = world
        .combat_facts_for(weapon)
        .filter(|facts| facts.dual_wield_weapon == 0)
        .map_or(0, |facts| facts.alternate_weapon);
    if alt != 0 && alt != weapon {
        start_ammo_one(world, id, alt);
    }
}

fn start_ammo_one(world: &mut FrameWorld, id: ClientId, weapon: u32) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    if let Some(eq) = world.equipment_facts_for(weapon).filter(|eq| eq.is_offhand()) {
        let loaded = ammo_clip(world, id, weapon);
        set_ammo_clip(world, id, weapon, loaded.max(eq.spawn_clip_count()));
        return;
    }
    // Other held guns drawing on the same ammunition pool count their magazines, less their own start ammunition,
    // against it.
    let pool = ammo_table_key(facts.ammo_index, weapon);
    let held: Vec<u32> = world
        .player(id)
        .map(|ps| {
            ps.weapons
                .iter()
                .filter(|&&slot| slot > 0)
                .map(|&slot| slot as u32)
                .collect()
        })
        .unwrap_or_default();
    let mut others = 0;
    for other in held.into_iter().filter(|&w| w != weapon) {
        let Some(other_facts) = world.combat_facts_for(other) else {
            continue;
        };
        if ammo_table_key(other_facts.ammo_index, other) == pool {
            others += ammo_clip(world, id, other) - other_facts.start_ammo.max(0);
        }
    }
    let dual = facts.dual_wield_weapon != 0;
    let left = if dual { left_clip(world, id, weapon) } else { 0 };
    let (right, left, stock) = topped_up(
        StartAmmo {
            start: facts.start_ammo.max(0),
            clip_size: facts.clip_size.max(0),
            max_stock: facts.max_ammo.max(0),
            dual,
        },
        Carried {
            right: ammo_clip(world, id, weapon),
            left,
            stock: ammo_stock(world, id, weapon),
            others,
        },
    );
    if dual {
        set_left_clip(world, id, weapon, left);
    }
    set_ammo_stock(world, id, weapon, stock);
    set_ammo_clip(world, id, weapon, right);
}

/// A gun's ammunition limits.
struct StartAmmo {
    start: i32,
    clip_size: i32,
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
fn refill(clip: i32, clip_size: i32, pool: &mut i32) -> i32 {
    let moved = (clip_size - clip).max(0).min((*pool).max(0));
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
    let right = refill(have.right, limits.clip_size, &mut pool);
    let left = if limits.dual {
        refill(have.left, limits.clip_size, &mut pool)
    } else {
        have.left
    };
    (right, left, pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tops up a gun with `start` / `clip_size` / `max_stock` that carries (`right`, `left`, `stock`, `others`).
    fn top_up(
        (start, clip_size, max_stock, dual): (i32, i32, i32, bool),
        (right, left, stock, others): (i32, i32, i32, i32),
    ) -> (i32, i32, i32) {
        topped_up(
            StartAmmo { start, clip_size, max_stock, dual },
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
