//! The script-facing player: what a script can do to, and ask of, one client (spawn and kill it, hand it weapons
//! and ammunition, set its perks, hit it, restrict its commands).
//!
//! Each file is one concern; this module re-exports their entry points under the names the rest of the simulation
//! already calls.

mod ammo;
mod arsenal;
mod controls;
mod hits;
mod lifecycle;
mod perks;

pub(crate) use ammo::{
    ammo_clip, ammo_fraction, ammo_stock, dual_wield_right_of, give_max_ammo, give_start_ammo, left_clip,
    set_ammo_clip, set_ammo_stock, set_left_clip,
};
pub(crate) use arsenal::{
    WeaponList, give_weapon, has_weapon, holds_weapon, set_spawn_weapon, switch_to_weapon, take_all_weapons,
    take_weapon, weapon_name, weapon_named, weapons, weapons_with_alt_modes,
};
pub(crate) use controls::{buttons, constrain_cmd};
pub(crate) use hits::{
    Finish, Hit, IDFLAGS_NO_ARMOR, IDFLAGS_NO_KNOCKBACK, IDFLAGS_PENETRATION, IDFLAGS_RADIUS, damage, debug_damage,
    finish_damage, knockback, means,
};
pub(crate) use lifecycle::{
    clone_corpse, kill, move_to_forced_spawn, obituary, revive, spawn, spectator_lifecycle,
};
pub(crate) use perks::{clear_perks, set_perk, stance};
