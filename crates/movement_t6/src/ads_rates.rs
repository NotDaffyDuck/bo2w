//! BO2's aim-rate and shock-walk scales: the BO2 callers' home.
//!
//! Quickdraw follows BO2's PM_UpdateAimDownSightLerp (facts
//! perks-movement-t6.md §2). The shellshock walk scale is still MW2's rule
//! (§3: BO2's float math there was lost).

use crate::bridge::Bo2Feel;
use playerstate_t6::game::AvatarState;

/// `playerAnimType` of the sniper anim set (weapon_t6 pm/ads.rs).
const ANIM_TYPE_SNIPER: i32 = 3;

/// `playerAnimType` of the sniper_rearclip anim set (weapon_t6 pm/ads.rs).
const ANIM_TYPE_SNIPER_REARCLIP: i32 = 17;

/// How much a shellshock slows walking.
/// TODO(fact): stand-in, MW2's `crate::bridge::shellshock_walk_scale`. BO2's
/// shock file has a scale and no on/off switch, but BO2's `shellshock`
/// native already puts that scale on `run_speed_modifier` and turns
/// `movement` off, so a BO2 shock gets 1.0 here and is not scaled twice.
#[must_use]
pub fn shellshock_walk_scale(movement: bool, movement_scale: f32) -> f32 {
    crate::bridge::shellshock_walk_scale(movement, movement_scale)
}

/// Quickdraw's aim-in and aim-out rates (per ms): with the perk, and a gun
/// whose `playerAnimType` is neither sniper set, both are divided by
/// `perk_weapAdsMultiplier` (the feel's multiplier).
///
/// BO2 asserts the multiplier is > 0; a multiplier <= 0 leaves the rates
/// alone here rather than dividing by it. BO2 also counts a perk the gun
/// itself carries, and a vehicle (eFlags 0x4000) then replaces the rate;
/// neither is in this stand-in yet.
#[must_use]
pub fn quickdraw_ads_rates(
    avatar: &AvatarState,
    scope_raise_speed: f32,
    scope_lower_speed: f32,
    player_motion_kind: i32,
    feel: Bo2Feel,
) -> (f32, f32) {
    let sniper = matches!(
        player_motion_kind,
        ANIM_TYPE_SNIPER | ANIM_TYPE_SNIPER_REARCLIP
    );
    if sniper
        || !avatar.has_perk_bit(feel.fast_ads_perk)
        || feel.fast_ads_multiplier <= 0.0
    {
        return (scope_raise_speed, scope_lower_speed);
    }
    (
        scope_raise_speed / feel.fast_ads_multiplier,
        scope_lower_speed / feel.fast_ads_multiplier,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quickdraw_player(feel: Bo2Feel) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.set_perk(feel.fast_ads_perk, true);
        avatar
    }

    fn feel() -> Bo2Feel {
        Bo2Feel {
            fast_ads_perk: playerstate_t6::perks::PERK_FASTADS,
            fast_ads_multiplier: 0.5,
            ..Bo2Feel::BO2_MP
        }
    }

    #[test]
    fn quickdraw_speeds_a_rifle() {
        let avatar = quickdraw_player(feel());
        assert_eq!(quickdraw_ads_rates(&avatar, 1.0, 2.0, 1, feel()), (2.0, 4.0));
    }

    #[test]
    fn quickdraw_skips_both_sniper_anim_sets() {
        let avatar = quickdraw_player(feel());
        for anim_type in [ANIM_TYPE_SNIPER, ANIM_TYPE_SNIPER_REARCLIP] {
            assert_eq!(
                quickdraw_ads_rates(&avatar, 1.0, 2.0, anim_type, feel()),
                (1.0, 2.0)
            );
        }
    }

    #[test]
    fn no_perk_no_change() {
        let avatar = AvatarState::ZERO;
        assert_eq!(quickdraw_ads_rates(&avatar, 1.0, 2.0, 1, feel()), (1.0, 2.0));
    }
}
