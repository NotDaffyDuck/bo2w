//! The climb animations the mantle reads its timing and root motion from.

mod bind;

pub use bind::LedgeClimbAnimSet;
use movement_t6::bridge::CLIMB_CLIP_SLOTS;

/// Black Ops II's climb tree, in the order its exe lists it. The climbs up and the high and mid vaults use the names
/// the older engine had; the low vault is the player's own `player_mantle_over_low` (BO2's `mp_mantle_over_low` has no
/// root motion). BO2 has no second, fast tree.
pub const MANTLE_XANIM_NAMES_T6: [&str; CLIMB_CLIP_SLOTS] = [
    "mp_mantle_root",
    "mp_mantle_up_57",
    "mp_mantle_up_51",
    "mp_mantle_up_45",
    "mp_mantle_up_39",
    "mp_mantle_up_33",
    "mp_mantle_up_27",
    "mp_mantle_up_21",
    "mp_mantle_over_high",
    "mp_mantle_over_mid",
    "player_mantle_over_low",
];

/// `perk_mantleReduction`: with `specialty_fastmantle` the climb-up part of the same animations runs shortened to this
/// share of its time (the vault over keeps its length). BO2's own value: 40%.
pub const PERK_MANTLE_REDUCTION_DEFAULT: f32 = 0.4;

/// bo2mp's value: the climb up with Dexterity takes 80% of its time, not BO2's 40%.
/// Play-test, 10-20, with Dexterity: "The pull-up speed is still way too fast ... It needs to be like half speed
/// of what it is now."
pub const PERK_MANTLE_REDUCTION_BO2MP: f32 = 0.8;
