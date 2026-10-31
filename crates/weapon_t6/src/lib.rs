//! BO2 (T6) weapon brain, rebuilt in our own Rust from facts about BO2's behaviour.
//!
//! - [`state`], [`anim`], [`events`]: BO2's weapon states, viewmodel anims and events.
//! - [`player`]: BO2's per-player weapon data (both hands), [`env`] and [`cmd`]: what a tick reads.
//! - [`facts`]: per-weapon facts from the fastfiles.
//! - [`tick`]: PM_Weapon in BO2's order; [`timing`]: the timer step; [`base`]: shared helpers.
//! - [`pm`]: one file of hooks per mechanic window; [`switch`]: weapon switching (window 2).
//! - [`fire`], [`reload`]: windows 3 and 4's standalone timelines, wired in through `pm`.
//! - [`hitloc`]: BO2's 21 hit locations and the damage multiplier each one gets.
//! - [`damage`], [`penetration`], [`surface`]: bullet damage falloff, penetration and surface names.
//! - [`trace`]: the frame trace and scripted input runner.
//! - [`pickup`]: BO2's dropped-gun rules (throw down, ammo leech, pickup).
//! - [`vehicle_heat`]: a vehicle gun's heat (the VTOL Warship's rocket reload).
//! - [`asset_slots`]: the catalog's viewmodel anim slots and which one a BO2 anim plays.
//! - [`combat_facts`], [`ammo_table`], [`kick`], [`sway`], [`placement`], [`view_bob`],
//!   [`viewweapon`], [`ads_allow`], [`ads_overlay`], [`melee_trace`], [`dual_mag`], [`fire_type`]:
//!   the gun code moved in from the old MW2 crate (TODO(fact) where it still reads MW2 numbers,
//!   [`ps_flags`]).

pub mod ads_allow;
pub mod ads_overlay;
pub mod aim;
pub mod ammo_table;
pub mod anim;
pub mod asset_slots;
pub mod base;
pub mod class_presets;
pub mod cmd;
pub mod combat_facts;
pub mod damage;
pub mod dual_mag;
pub mod env;
pub mod events;
pub mod facts;
pub mod fire;
pub mod fire_type;
pub mod hitloc;
pub mod kick;
pub mod melee_trace;
pub mod penetration;
pub mod pickup;
pub mod placement;
pub mod player;
pub mod pm;
pub mod pose;
pub mod ps_flags;
pub mod recoil;
pub mod reload;
pub mod script;
pub mod state;
pub mod surface;
pub mod sway;
pub mod switch;
pub mod third;
pub mod tick;
pub mod timing;
pub mod trace;
pub mod vehicle_heat;
pub mod view_bob;
pub mod viewmodel;
pub mod viewweapon;
mod gun_anim;
