//! What one shot does to the kick state.

use weapon_t6::kick::{
    FireRecoilImpulse, FireRecoilPsScales, fire_recoil_amplitude_scale, fire_recoil_gun_range,
    fire_recoil_view_range, weapon_fire_recoil,
};

use super::params::KickParams;
use super::state::ViewKickState;

/// Weapons count as fully aimed only when the blend is exactly one.
fn fully_aimed(aim_frac: f32) -> bool {
    aim_frac == 1.0
}

impl ViewKickState {
    /// Register a shot with the simple scaling inputs a caller usually has.
    ///
    /// `aim_frac` runs from 0 (hip) to 1 (aimed). `weap_flags` and
    /// `recoil_scale` come from the weapon and player state, and
    /// `reduce_window_active` says the "reduced kick" window is open. The
    /// caller supplies no reduction percent of its own; the weapon's is used.
    pub fn seed_fire(
        &mut self,
        kick: &KickParams,
        aim_frac: f32,
        weap_flags: u32,
        recoil_scale: i32,
        reduce_window_active: bool,
    ) {
        let scales = FireRecoilPsScales {
            reduce_window_active,
            reduced_percent: 0.0,
            weap_flags,
            recoil_scale,
        };
        self.seed_fire_scaled(kick, aim_frac, scales);
    }

    /// Register a shot with the full set of scaling inputs.
    ///
    /// A fresh view push replaces whatever push was still running; the gun's
    /// swing speed is added on top of the swing already under way. When the
    /// reduced-kick window is open the weapon's own percent for the current
    /// stance overrides `scales.reduced_percent`.
    pub fn seed_fire_scaled(
        &mut self,
        kick: &KickParams,
        aim_frac: f32,
        mut scales: FireRecoilPsScales,
    ) {
        let aimed = fully_aimed(aim_frac);

        if scales.reduce_window_active {
            scales.reduced_percent = if aimed {
                kick.ads_gun_kick_reduced_kick_percent
            } else {
                kick.hip_gun_kick_reduced_kick_percent
            };
        }

        let view_limits =
            fire_recoil_view_range(aim_frac, kick.hip_view_range(), kick.ads_view_range());
        let gun_limits = fire_recoil_gun_range(aim_frac, kick.hip_gun_range(), kick.ads_gun_range());

        // Four rolls per shot, taken in a fixed order so the stream stays in step.
        let rolls: [f32; 4] = std::array::from_fn(|_| self.next_unit());
        let FireRecoilImpulse {
            mut kick_avel,
            gun_speed_delta,
        } = weapon_fire_recoil(view_limits, gun_limits, scales, rolls);

        // bo2zm: a Black Ops II shot never kicks the view more gently than the
        // weapon's floor (scaled the same way the kick itself was).
        let floor = if aimed {
            kick.ads_view_kick_min_magnitude
        } else {
            kick.hip_view_kick_min_magnitude
        } * fire_recoil_amplitude_scale(scales);
        stretch_to_floor(&mut kick_avel, floor);

        self.kick_avel = kick_avel;
        for (speed, extra) in self.gun_speed.iter_mut().zip(gun_speed_delta) {
            *speed += extra;
        }
    }
}

/// Scale a push up so its pitch/yaw strength reaches `floor`.
///
/// Nothing happens when the floor is off, when there is no push at all (there
/// is no direction to stretch), or when the push is already strong enough.
/// Roll rides along with the same factor so the direction is kept.
fn stretch_to_floor(push: &mut [f32; 3], floor: f32) {
    let strength = (push[0] * push[0] + push[1] * push[1]).sqrt();
    let needs_stretch = floor > 0.0 && strength > 0.0 && strength < floor;
    if !needs_stretch {
        return;
    }
    let factor = floor / strength;
    for axis in push.iter_mut() {
        *axis *= factor;
    }
}
