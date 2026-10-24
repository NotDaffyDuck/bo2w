//! The movement perk bits: the BO2 callers' home.
//!
//! BO2's `BG_HasPerk` tests bit `perk % 32` of `perks[perk / 32]`
//! ([`crate::PlayerState::has_perk_bit`]). The engine's `perks[0]` word is
//! not on BO2's numbers yet, so every bit here is a stand-in: MW2's value
//! from the legacy movement (feel.rs, sprint.rs). Each moves to its BO2 number
//! when the `perks` word does.

/// Fall perk (specialty_fallheight).
/// TODO(fact): BO2's perk number. Stand-in: MW2's `1 << 29`.
pub const PERK_FALLHEIGHT: u32 = 1 << 29;

/// Flak Jacket, Zombies' PhD Flopper (specialty_flakjacket).
/// TODO(fact): BO2's perk number. Stand-in: MW2's `1 << 31`.
pub const PERK_FLAKJACKET: u32 = 1 << 31;

/// Quickdraw (specialty_fastads). BO2's perk number is 15 (`perksEnum`,
/// weapon_t6 `env::perk::FASTADS`), bit `1 << 15` once `perks` is on BO2's
/// numbers. Stand-in until then: MW2's `1 << 30`.
pub const PERK_FASTADS: u32 = 1 << 30;

/// Stamin-Up, multiplayer's Extreme Conditioning (specialty_longersprint).
/// TODO(fact): BO2's perk number. Stand-in: MW2's `1 << 26`.
pub const PERK_LONGERSPRINT: u32 = 1 << 26;

/// Stalker (specialty_stalker): aiming does not slow the walk.
/// TODO(fact): BO2's perk number. Stand-in: MW2's `1 << 11`.
pub const PERK_STALKER: u32 = 1 << 11;

/// Move-faster (specialty_movefaster): multiplayer's Lightweight speed and
/// the Turned zombies' perk.
/// TODO(fact): BO2's perk number. Stand-in: MW2's `1 << 9`.
pub const PERK_MOVEFASTER: u32 = 1 << 9;

/// Marathon (specialty_marathon, and specialty_unlimitedsprint).
/// TODO(fact): BO2's perk numbers for both. Stand-in: MW2's `1 << 25`.
pub const PERK_MARATHON: u32 = 1 << 25;

// BO2's own perk numbers (`perksEnum`, facts perks-movement-t6.md §1), for
// [`crate::PlayerState::has_perk_bit`]. The masks above move onto these
// when the engine's `perks` words do.

/// specialty_fallheight.
pub const PERK_FALLHEIGHT_INDEX: u32 = 14;
/// specialty_fastads (Quickdraw).
pub const PERK_FASTADS_INDEX: u32 = 15;
/// specialty_fastmantle.
pub const PERK_FASTMANTLE_INDEX: u32 = 18;
/// specialty_flakjacket.
pub const PERK_FLAKJACKET_INDEX: u32 = 25;
/// specialty_longersprint.
pub const PERK_LONGERSPRINT_INDEX: u32 = 37;
/// specialty_movefaster.
pub const PERK_MOVEFASTER_INDEX: u32 = 40;
/// specialty_stalker.
pub const PERK_STALKER_INDEX: u32 = 59;
/// specialty_unlimitedsprint, which Marathon is in BO2.
pub const PERK_UNLIMITEDSPRINT_INDEX: u32 = 62;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_land_on_bo2s_words_and_bits() {
        let mut ps = crate::PlayerState::default();
        ps.set_perk(PERK_LONGERSPRINT_INDEX, true);
        assert_eq!(ps.perks[1], 1 << 5);
        assert!(ps.has_perk_bit(PERK_LONGERSPRINT_INDEX));
        assert!(!ps.has_perk_bit(PERK_FASTADS_INDEX));
        ps.set_perk(PERK_FASTADS_INDEX, true);
        assert_eq!(ps.perks[0], 1 << 15);
        ps.set_perk(PERK_UNLIMITEDSPRINT_INDEX, true);
        assert_eq!(ps.perks[1], (1 << 5) | (1 << 30));
    }
}
