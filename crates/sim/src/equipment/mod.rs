//! Thrown, launched and planted equipment (grenades, rockets, knives, claymores and the like): how each starts, how it
//! flies, what it does on touching something and when it goes off.
//!
//! One `flight::think_projectile` call per missile per tick drives the rest: `scene` answers what it can meet, `contact`
//! what each kind of contact does, `bounce`/`roll`/`stick` how it moves afterward, and `blast` what the tick costs.

mod blast;
mod bounce;
mod contact;
mod events;
mod facts;
mod flight;
mod launch;
mod predict;
mod rider;
mod roll;
mod scene;
mod state;
mod stick;
mod vec3;

#[cfg(test)]
mod tests;

pub use facts::EquipmentRuntimeFacts;
pub use rider::StuckRider;
pub use state::{ProjectileState, ROCKET_CLEANUP_MS, projectile_birth_ms};

pub(crate) use flight::think_projectile;
pub(crate) use launch::{drop_grenade_on_death, spawn_grenade_projectile, spawn_offhand_projectile};
pub(crate) use predict::predict_projectile;
pub(crate) use state::{GrenadeLaunchKind, WeaponNote, order_script_detonation, set_missile_coasting};
