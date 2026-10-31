//! How far the sights are raised: the aim-down-sights fraction, 0 (hip) to 1.

use playerstate_t6::consts::weapon_state;
use playerstate_t6::game::{AvatarState, movement_state_bits};

/// What the weapon and the server settings say about the sights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeBlendArgs {
    /// The weapon can aim at all; without it the fraction stays at 0.
    pub scoped_aiming: bool,
    /// Fraction gained per millisecond while raising the sights.
    pub scope_raise_speed: f32,
    /// Fraction lost per millisecond while lowering them.
    pub scope_lower_speed: f32,
    /// The sights stay up through the rechamber after a shot.
    pub rack_bolt_when_scoped: bool,
    /// The sights only come up to fire, not to hold.
    pub fire_only_when_scoped: bool,
    /// A reload lowers the sights (the weapon cannot reload while aiming).
    pub reload_lowers_ads: bool,
    /// The sights come back up when this much of the reload is left.
    pub position_reload_trans_ms: i32,
    /// The weapon reloads a shell at a time.
    pub staged_reload: bool,
    /// bo2mp: the weapon brain moves the sights itself; leave the fraction.
    pub weapon_brain_owns: bool,
}

impl Default for ScopeBlendArgs {
    fn default() -> Self {
        Self {
            scoped_aiming: true,
            scope_raise_speed: 1.0 / 200.0,
            scope_lower_speed: 1.0 / 200.0,
            rack_bolt_when_scoped: true,
            fire_only_when_scoped: false,
            reload_lowers_ads: false,
            position_reload_trans_ms: 0,
            staged_reload: false,
            weapon_brain_owns: false,
        }
    }
}

/// Move the sights one frame of `msec` toward where the player wants them.
pub fn update_ads_frac(avatar: &mut AvatarState, msec: i32, frame_ctx: ScopeBlendArgs) {
    if frame_ctx.weapon_brain_owns {
        return;
    }
    if !frame_ctx.scoped_aiming {
        avatar.weapon_raise_fraction = 0.0;
        avatar.scope_in_wait = 0;
        return;
    }

    let step_ms = msec.max(0) as f32;
    let frac = avatar.weapon_raise_fraction;
    avatar.weapon_raise_fraction = if sights_wanted_up(avatar, frame_ctx) {
        if frac >= 1.0 || frame_ctx.scope_raise_speed <= 0.0 {
            1.0
        } else {
            (frac + frame_ctx.scope_raise_speed * step_ms).clamp(0.0, 1.0)
        }
    } else if frac <= 0.0 || frame_ctx.scope_lower_speed <= 0.0 {
        0.0
    } else {
        (frac - frame_ctx.scope_lower_speed * step_ms).clamp(0.0, 1.0)
    };
    avatar.scope_in_wait = 0;
}

/// The aim button, overridden by what the weapon is doing: a rechamber or a
/// reload can lower the sights, and a shot on a fire-only weapon raises them.
fn sights_wanted_up(avatar: &AvatarState, frame_ctx: ScopeBlendArgs) -> bool {
    let state = avatar.weaponstate_primary;

    let firing = (avatar.weapon_ready_wait != 0 && state == weapon_state::FIRING)
        || (avatar.weapon_delay_secondary != 0 && avatar.weaponstate_secondary == weapon_state::FIRING);
    if frame_ctx.fire_only_when_scoped && firing {
        return true;
    }

    let mut wanted = avatar.movement_state_bits & movement_state_bits::ADS_INTENT != 0;
    if !frame_ctx.rack_bolt_when_scoped && state == weapon_state::RECHAMBER_START {
        wanted = false;
    }
    if frame_ctx.reload_lowers_ads && reload_keeps_sights_down(avatar, frame_ctx) {
        wanted = false;
    }
    wanted
}

/// The sights stay down through a reload and come back once only
/// `position_reload_trans_ms` of it is left. A shell-at-a-time reload keeps
/// them down for the start and every shell, and lets them up near the end.
fn reload_keeps_sights_down(avatar: &AvatarState, frame_ctx: ScopeBlendArgs) -> bool {
    let early = avatar.weapon_timer > frame_ctx.position_reload_trans_ms;
    match avatar.weaponstate_primary {
        weapon_state::RECHAMBERING => frame_ctx.staged_reload || early,
        weapon_state::RELOADING_RIGHT | weapon_state::RELOADING_LEFT | weapon_state::RELOADING => {
            frame_ctx.staged_reload
        }
        weapon_state::RELOADING_INTERUPT => frame_ctx.staged_reload && early,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aiming(state: i32, weapon_time: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        avatar.weapon_raise_fraction = 1.0;
        avatar.weaponstate_primary = state;
        avatar.weapon_timer = weapon_time;
        avatar
    }

    fn reload_rules(reload_lowers_ads: bool, staged_reload: bool) -> ScopeBlendArgs {
        ScopeBlendArgs {
            reload_lowers_ads,
            position_reload_trans_ms: 300,
            staged_reload,
            ..ScopeBlendArgs::default()
        }
    }

    #[test]
    fn the_sights_rise_and_fall_at_their_own_rates() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        update_ads_frac(&mut avatar, 50, ScopeBlendArgs::default());
        assert!((avatar.weapon_raise_fraction - 0.25).abs() < 1e-6);
        avatar.movement_state_bits &= !movement_state_bits::ADS_INTENT;
        update_ads_frac(&mut avatar, 100, ScopeBlendArgs::default());
        // 100 ms at 1/200 per ms is more than the 0.25 there was to lose.
        assert_eq!(avatar.weapon_raise_fraction, 0.0);
    }

    #[test]
    fn a_zero_rate_snaps_the_sights_all_the_way() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits |= movement_state_bits::ADS_INTENT;
        update_ads_frac(
            &mut avatar,
            1,
            ScopeBlendArgs {
                scope_raise_speed: 0.0,
                ..ScopeBlendArgs::default()
            },
        );
        assert_eq!(avatar.weapon_raise_fraction, 1.0);
    }

    #[test]
    fn a_weapon_that_cannot_aim_stays_at_the_hip() {
        let mut avatar = aiming(weapon_state::READY, 0);
        update_ads_frac(
            &mut avatar,
            50,
            ScopeBlendArgs {
                scoped_aiming: false,
                ..ScopeBlendArgs::default()
            },
        );
        assert_eq!(avatar.weapon_raise_fraction, 0.0);
    }

    #[test]
    fn the_weapon_brain_keeps_its_own_fraction() {
        let mut avatar = aiming(weapon_state::READY, 0);
        avatar.weapon_raise_fraction = 0.3;
        update_ads_frac(
            &mut avatar,
            50,
            ScopeBlendArgs {
                weapon_brain_owns: true,
                ..ScopeBlendArgs::default()
            },
        );
        assert_eq!(avatar.weapon_raise_fraction, 0.3);
    }

    #[test]
    fn a_reload_lowers_the_sights_then_raises_them_near_the_end() {
        let mut avatar = aiming(weapon_state::RECHAMBERING, 1500);
        update_ads_frac(&mut avatar, 50, reload_rules(true, false));
        assert!(
            avatar.weapon_raise_fraction < 1.0,
            "early in the reload the sights come down"
        );

        let mut avatar = aiming(weapon_state::RECHAMBERING, 300);
        avatar.weapon_raise_fraction = 0.5;
        update_ads_frac(&mut avatar, 50, reload_rules(true, false));
        assert!(
            avatar.weapon_raise_fraction > 0.5,
            "with the last bit left they come back up"
        );
    }

    #[test]
    fn a_gun_that_aims_while_reloading_keeps_the_sights() {
        let mut avatar = aiming(weapon_state::RECHAMBERING, 1500);
        update_ads_frac(&mut avatar, 50, reload_rules(false, false));
        assert_eq!(avatar.weapon_raise_fraction, 1.0);
    }

    #[test]
    fn a_shell_at_a_time_reload_keeps_the_sights_down_until_the_end() {
        for state in [
            weapon_state::RECHAMBERING,
            weapon_state::RELOADING_RIGHT,
            weapon_state::RELOADING_LEFT,
            weapon_state::RELOADING,
        ] {
            let mut avatar = aiming(state, 100);
            update_ads_frac(&mut avatar, 50, reload_rules(true, true));
            assert!(avatar.weapon_raise_fraction < 1.0, "state {state}");
        }
        let mut avatar = aiming(weapon_state::RELOADING_INTERUPT, 600);
        update_ads_frac(&mut avatar, 50, reload_rules(true, true));
        assert!(avatar.weapon_raise_fraction < 1.0, "reload end, early");
        let mut avatar = aiming(weapon_state::RELOADING_INTERUPT, 200);
        update_ads_frac(&mut avatar, 50, reload_rules(true, true));
        assert_eq!(avatar.weapon_raise_fraction, 1.0, "reload end, last bit");
    }

    #[test]
    fn a_knife_swing_keeps_the_sights_up() {
        let mut avatar = aiming(weapon_state::MELEE_FIRE, 200);
        update_ads_frac(&mut avatar, 50, reload_rules(false, false));
        assert_eq!(avatar.weapon_raise_fraction, 1.0);
    }

    #[test]
    fn a_rechamber_drops_the_sights_only_when_the_weapon_cannot_hold_them() {
        let mut avatar = aiming(weapon_state::RECHAMBER_START, 200);
        update_ads_frac(
            &mut avatar,
            50,
            ScopeBlendArgs {
                rack_bolt_when_scoped: false,
                ..ScopeBlendArgs::default()
            },
        );
        assert!(avatar.weapon_raise_fraction < 1.0);
        let mut avatar = aiming(weapon_state::RECHAMBER_START, 200);
        update_ads_frac(&mut avatar, 50, ScopeBlendArgs::default());
        assert_eq!(avatar.weapon_raise_fraction, 1.0);
    }

    #[test]
    fn a_fire_only_weapon_raises_the_sights_to_shoot() {
        let mut avatar = AvatarState::ZERO;
        avatar.weaponstate_primary = weapon_state::FIRING;
        avatar.weapon_ready_wait = 10;
        update_ads_frac(
            &mut avatar,
            50,
            ScopeBlendArgs {
                fire_only_when_scoped: true,
                ..ScopeBlendArgs::default()
            },
        );
        assert!(avatar.weapon_raise_fraction > 0.0);
    }
}
