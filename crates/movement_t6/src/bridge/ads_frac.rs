//! How far the sights are raised: the aim-down-sights fraction, 0 (hip) to 1.

use playerstate_t6::consts::weapon_state;
use playerstate_t6::game::{PlayerState, pm_flags};

/// What the weapon and the server settings say about the sights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdsFracContext {
    /// The weapon can aim at all; without it the fraction stays at 0.
    pub aim_down_sight: bool,
    /// Fraction gained per millisecond while raising the sights.
    pub ads_in_rate: f32,
    /// Fraction lost per millisecond while lowering them.
    pub ads_out_rate: f32,
    /// The sights stay up through the rechamber after a shot.
    pub rechamber_while_ads: bool,
    /// The sights only come up to fire, not to hold.
    pub ads_fire_only: bool,
    /// A reload lowers the sights (the weapon cannot reload while aiming).
    pub reload_lowers_ads: bool,
    /// The sights come back up when this much of the reload is left.
    pub position_reload_trans_ms: i32,
    /// The weapon reloads a shell at a time.
    pub segmented_reload: bool,
    /// bo2mp: the weapon brain moves the sights itself; leave the fraction.
    pub weapon_brain_owns: bool,
}

impl Default for AdsFracContext {
    fn default() -> Self {
        Self {
            aim_down_sight: true,
            ads_in_rate: 1.0 / 200.0,
            ads_out_rate: 1.0 / 200.0,
            rechamber_while_ads: true,
            ads_fire_only: false,
            reload_lowers_ads: false,
            position_reload_trans_ms: 0,
            segmented_reload: false,
            weapon_brain_owns: false,
        }
    }
}

/// Move the sights one frame of `msec` toward where the player wants them.
pub fn update_ads_frac(ps: &mut PlayerState, msec: i32, context: AdsFracContext) {
    if context.weapon_brain_owns {
        return;
    }
    if !context.aim_down_sight {
        ps.f_weapon_pos_frac = 0.0;
        ps.ads_delay_time = 0;
        return;
    }

    let step_ms = msec.max(0) as f32;
    let frac = ps.f_weapon_pos_frac;
    ps.f_weapon_pos_frac = if sights_wanted_up(ps, context) {
        if frac >= 1.0 || context.ads_in_rate <= 0.0 {
            1.0
        } else {
            (frac + context.ads_in_rate * step_ms).clamp(0.0, 1.0)
        }
    } else if frac <= 0.0 || context.ads_out_rate <= 0.0 {
        0.0
    } else {
        (frac - context.ads_out_rate * step_ms).clamp(0.0, 1.0)
    };
    ps.ads_delay_time = 0;
}

/// The aim button, overridden by what the weapon is doing: a rechamber or a
/// reload can lower the sights, and a shot on a fire-only weapon raises them.
fn sights_wanted_up(ps: &PlayerState, context: AdsFracContext) -> bool {
    let state = ps.weaponstate_primary;

    let firing = (ps.weapon_delay != 0 && state == weapon_state::FIRING)
        || (ps.weapon_delay_secondary != 0 && ps.weaponstate_secondary == weapon_state::FIRING);
    if context.ads_fire_only && firing {
        return true;
    }

    let mut wanted = ps.pm_flags & pm_flags::ADS_INTENT != 0;
    if !context.rechamber_while_ads && state == weapon_state::RECHAMBER_START {
        wanted = false;
    }
    if context.reload_lowers_ads && reload_keeps_sights_down(ps, context) {
        wanted = false;
    }
    wanted
}

/// The sights stay down through a reload and come back once only
/// `position_reload_trans_ms` of it is left. A shell-at-a-time reload keeps
/// them down for the start and every shell, and lets them up near the end.
fn reload_keeps_sights_down(ps: &PlayerState, context: AdsFracContext) -> bool {
    let early = ps.weapon_time > context.position_reload_trans_ms;
    match ps.weaponstate_primary {
        weapon_state::RECHAMBERING => context.segmented_reload || early,
        weapon_state::RELOADING_RIGHT | weapon_state::RELOADING_LEFT | weapon_state::RELOADING => {
            context.segmented_reload
        }
        weapon_state::RELOADING_INTERUPT => context.segmented_reload && early,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aiming(state: i32, weapon_time: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags |= pm_flags::ADS_INTENT;
        ps.f_weapon_pos_frac = 1.0;
        ps.weaponstate_primary = state;
        ps.weapon_time = weapon_time;
        ps
    }

    fn reload_rules(reload_lowers_ads: bool, segmented_reload: bool) -> AdsFracContext {
        AdsFracContext {
            reload_lowers_ads,
            position_reload_trans_ms: 300,
            segmented_reload,
            ..AdsFracContext::default()
        }
    }

    #[test]
    fn the_sights_rise_and_fall_at_their_own_rates() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags |= pm_flags::ADS_INTENT;
        update_ads_frac(&mut ps, 50, AdsFracContext::default());
        assert!((ps.f_weapon_pos_frac - 0.25).abs() < 1e-6);
        ps.pm_flags &= !pm_flags::ADS_INTENT;
        update_ads_frac(&mut ps, 100, AdsFracContext::default());
        // 100 ms at 1/200 per ms is more than the 0.25 there was to lose.
        assert_eq!(ps.f_weapon_pos_frac, 0.0);
    }

    #[test]
    fn a_zero_rate_snaps_the_sights_all_the_way() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags |= pm_flags::ADS_INTENT;
        update_ads_frac(
            &mut ps,
            1,
            AdsFracContext {
                ads_in_rate: 0.0,
                ..AdsFracContext::default()
            },
        );
        assert_eq!(ps.f_weapon_pos_frac, 1.0);
    }

    #[test]
    fn a_weapon_that_cannot_aim_stays_at_the_hip() {
        let mut ps = aiming(weapon_state::READY, 0);
        update_ads_frac(
            &mut ps,
            50,
            AdsFracContext {
                aim_down_sight: false,
                ..AdsFracContext::default()
            },
        );
        assert_eq!(ps.f_weapon_pos_frac, 0.0);
    }

    #[test]
    fn the_weapon_brain_keeps_its_own_fraction() {
        let mut ps = aiming(weapon_state::READY, 0);
        ps.f_weapon_pos_frac = 0.3;
        update_ads_frac(
            &mut ps,
            50,
            AdsFracContext {
                weapon_brain_owns: true,
                ..AdsFracContext::default()
            },
        );
        assert_eq!(ps.f_weapon_pos_frac, 0.3);
    }

    #[test]
    fn a_reload_lowers_the_sights_then_raises_them_near_the_end() {
        let mut ps = aiming(weapon_state::RECHAMBERING, 1500);
        update_ads_frac(&mut ps, 50, reload_rules(true, false));
        assert!(
            ps.f_weapon_pos_frac < 1.0,
            "early in the reload the sights come down"
        );

        let mut ps = aiming(weapon_state::RECHAMBERING, 300);
        ps.f_weapon_pos_frac = 0.5;
        update_ads_frac(&mut ps, 50, reload_rules(true, false));
        assert!(
            ps.f_weapon_pos_frac > 0.5,
            "with the last bit left they come back up"
        );
    }

    #[test]
    fn a_gun_that_aims_while_reloading_keeps_the_sights() {
        let mut ps = aiming(weapon_state::RECHAMBERING, 1500);
        update_ads_frac(&mut ps, 50, reload_rules(false, false));
        assert_eq!(ps.f_weapon_pos_frac, 1.0);
    }

    #[test]
    fn a_shell_at_a_time_reload_keeps_the_sights_down_until_the_end() {
        for state in [
            weapon_state::RECHAMBERING,
            weapon_state::RELOADING_RIGHT,
            weapon_state::RELOADING_LEFT,
            weapon_state::RELOADING,
        ] {
            let mut ps = aiming(state, 100);
            update_ads_frac(&mut ps, 50, reload_rules(true, true));
            assert!(ps.f_weapon_pos_frac < 1.0, "state {state}");
        }
        let mut ps = aiming(weapon_state::RELOADING_INTERUPT, 600);
        update_ads_frac(&mut ps, 50, reload_rules(true, true));
        assert!(ps.f_weapon_pos_frac < 1.0, "reload end, early");
        let mut ps = aiming(weapon_state::RELOADING_INTERUPT, 200);
        update_ads_frac(&mut ps, 50, reload_rules(true, true));
        assert_eq!(ps.f_weapon_pos_frac, 1.0, "reload end, last bit");
    }

    #[test]
    fn a_knife_swing_keeps_the_sights_up() {
        let mut ps = aiming(weapon_state::MELEE_FIRE, 200);
        update_ads_frac(&mut ps, 50, reload_rules(false, false));
        assert_eq!(ps.f_weapon_pos_frac, 1.0);
    }

    #[test]
    fn a_rechamber_drops_the_sights_only_when_the_weapon_cannot_hold_them() {
        let mut ps = aiming(weapon_state::RECHAMBER_START, 200);
        update_ads_frac(
            &mut ps,
            50,
            AdsFracContext {
                rechamber_while_ads: false,
                ..AdsFracContext::default()
            },
        );
        assert!(ps.f_weapon_pos_frac < 1.0);
        let mut ps = aiming(weapon_state::RECHAMBER_START, 200);
        update_ads_frac(&mut ps, 50, AdsFracContext::default());
        assert_eq!(ps.f_weapon_pos_frac, 1.0);
    }

    #[test]
    fn a_fire_only_weapon_raises_the_sights_to_shoot() {
        let mut ps = PlayerState::ZERO;
        ps.weaponstate_primary = weapon_state::FIRING;
        ps.weapon_delay = 10;
        update_ads_frac(
            &mut ps,
            50,
            AdsFracContext {
                ads_fire_only: true,
                ..AdsFracContext::default()
            },
        );
        assert!(ps.f_weapon_pos_frac > 0.0);
    }
}
