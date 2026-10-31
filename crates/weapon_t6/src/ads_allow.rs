//! Whether the player may aim down the sights right now.

use crate::ammo_table::{CLIP_LEDGER_LEN, get_clip_for_hand};
use crate::ps_flags::{ent_bit, secondary_state_bits, movement_state_bits, weapon_state_bits};
use crate::state;

/// The e_flags bits that put the player on a turret or in a vehicle.
const EFLAGS_ON_TURRET_OR_VEHICLE: u32 = ent_bit::TURRET_MASK | ent_bit::IN_VEHICLE;
/// The left-hand weapon is the last one used.
const LAST_HAND_LEFT: i32 = 1;
/// movement_mode values that never aim.
const PM_TYPES_WITHOUT_ADS: [i32; 6] = [2, 3, 5, 6, 10, 11];

/// The player-state fields BO2's PM_IsAdsAllowed reads.
///
/// TODO(fact): `e_flags`, `secondary_state_bits`, `movement_state_bits` and `weapon_state_bits` still carry
/// the MW2 player-state bits ([`crate::ps_flags`]); the caller copies them over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdsAllowPlayer {
    pub previous_gun_hand: i32,
    pub ent_bits: u32,
    pub movement_mode: i32,
    pub secondary_state_bits: u32,
    pub movement_state_bits: u32,
    pub weapon_state_bits: u32,
    pub weapon_state_t6: i32,
}

/// The weapon file fields that decide aiming.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopePermitGunInfo {
    pub scoped_aiming: bool,
    /// An empty clip lowers the sights.
    pub block_aim_if_clip_dry: bool,
    pub motion_clip_slot: i32,
    /// bo2mp: Hybrid Optic: the sight swap keeps the player aimed.
    pub optic_switch: bool,
}

/// A weapon state that keeps the sights down.
fn state_blocks_ads(ws: i32, optic_switch: bool) -> bool {
    // A throw or hold of a grenade or equipment (a detonator press aims on).
    if state::is_secondary_gear(ws) {
        return ws != state::OFFHAND_DETONATE;
    }
    // bo2mp: the Hybrid Optic's swap (alternate raise/drop) stays in ADS.
    let optic_swap = ws == state::RAISING_ALTSWITCH || ws == state::DROPPING_ALTSWITCH;
    if optic_swap && optic_switch {
        return false;
    }
    (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&ws)
        || [
            state::SWIM_IN,
            state::SWIM_OUT,
            state::NIGHTVISION_WEAR,
            state::NIGHTVISION_REMOVE,
        ]
        .contains(&ws)
}

/// BO2's PM_IsAdsAllowed: a knife swing keeps the sights up, and climbing
/// over a ledge drops them.
pub fn is_ads_allowed_bo2(
    ps: &AdsAllowPlayer,
    weap: &ScopePermitGunInfo,
    magazine_rows: &[u8; CLIP_LEDGER_LEN],
    side_of_clip: u8,
) -> bool {
    let left_hand_off_turret =
        ps.previous_gun_hand == LAST_HAND_LEFT && ps.ent_bits & EFLAGS_ON_TURRET_OR_VEHICLE == 0;
    let blocked = left_hand_off_turret
        || PM_TYPES_WITHOUT_ADS.contains(&ps.movement_mode)
        || ps.secondary_state_bits & secondary_state_bits::PLAYER == 0
        || !weap.scoped_aiming
        || state_blocks_ads(ps.weapon_state_t6, weap.optic_switch)
        || ps.movement_state_bits & movement_state_bits::MANTLE != 0
        || ps.weapon_state_bits & weapon_state_bits::AIM_DISABLED_BIT != 0
        || weap.block_aim_if_clip_dry
            && get_clip_for_hand(magazine_rows, weap.motion_clip_slot, side_of_clip) == 0;
    !blocked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aiming_player() -> AdsAllowPlayer {
        AdsAllowPlayer {
            secondary_state_bits: crate::ps_flags::secondary_state_bits::PLAYER,
            ..AdsAllowPlayer::default()
        }
    }

    fn rifle() -> ScopePermitGunInfo {
        ScopePermitGunInfo {
            scoped_aiming: true,
            ..ScopePermitGunInfo::default()
        }
    }

    /// BO2's PM_IsAdsAllowed has no knife states in its list: the sights
    /// stay up through a knife swing.
    #[test]
    fn bo2_keeps_aim_through_a_knife_swing() {
        let clips = [0u8; CLIP_LEDGER_LEN];
        let mut ps = aiming_player();
        for state in [
            crate::state::MELEE_INIT,
            crate::state::MELEE_FIRE,
            crate::state::MELEE_END,
        ] {
            ps.weapon_state_t6 = state;
            assert!(is_ads_allowed_bo2(&ps, &rifle(), &clips, 0));
        }
    }

    /// BO2 blocks aiming while climbing over a ledge.
    #[test]
    fn bo2_blocks_aim_while_mantling() {
        let clips = [0u8; CLIP_LEDGER_LEN];
        let mut ps = aiming_player();
        ps.movement_state_bits |= crate::ps_flags::movement_state_bits::MANTLE;
        assert!(!is_ads_allowed_bo2(&ps, &rifle(), &clips, 0));
        ps.movement_state_bits = 0;
        assert!(is_ads_allowed_bo2(&ps, &rifle(), &clips, 0));
    }

    #[test]
    fn the_hybrid_optic_swap_stays_aimed_and_a_raise_does_not() {
        let clips = [0u8; CLIP_LEDGER_LEN];
        let mut ps = aiming_player();
        ps.weapon_state_t6 = state::RAISING_ALTSWITCH;
        assert!(!is_ads_allowed_bo2(&ps, &rifle(), &clips, 0));
        let optic = ScopePermitGunInfo {
            optic_switch: true,
            ..rifle()
        };
        assert!(is_ads_allowed_bo2(&ps, &optic, &clips, 0));
        ps.weapon_state_t6 = state::RAISING;
        assert!(!is_ads_allowed_bo2(&ps, &optic, &clips, 0));
    }

    #[test]
    fn an_empty_clip_lowers_sights_only_when_the_gun_says_so() {
        let mut clips = [0u8; CLIP_LEDGER_LEN];
        let ps = aiming_player();
        let picky = ScopePermitGunInfo {
            block_aim_if_clip_dry: true,
            motion_clip_slot: 3,
            ..rifle()
        };
        assert!(!is_ads_allowed_bo2(&ps, &picky, &clips, 0));
        crate::ammo_table::set_clip_for_hand(&mut clips, 3, 0, 1);
        assert!(is_ads_allowed_bo2(&ps, &picky, &clips, 0));
        assert!(is_ads_allowed_bo2(
            &ps,
            &rifle(),
            &[0u8; CLIP_LEDGER_LEN],
            0
        ));
    }
}
