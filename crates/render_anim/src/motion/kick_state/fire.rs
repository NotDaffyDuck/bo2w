//! What one shot does to the kick state.

use weapon_t6::kick::{
    ShotKickPush, ShotKickPlayerFactors, shot_recoil_gain, shot_kick_viewmodel_bounds,
    shot_kick_camera_bounds, weapon_fire_recoil,
};

use super::params::GunKickTuning;
use super::state::ViewRecoilState;

/// Weapons count as fully aimed only when the blend is exactly one.
fn fully_aimed(aim_frac: f32) -> bool {
    aim_frac.to_bits() == 1.0f32.to_bits()
}

impl ViewRecoilState {
    /// Register a shot with the simple scaling inputs a caller usually has.
    ///
    /// `aim_frac` runs from 0 (hip) to 1 (aimed). `weapon_state_bits` and
    /// `recoil_scale` come from the weapon and player state, and
    /// `damping_spell_on` says the "reduced kick" window is open. The
    /// caller supplies no reduction percent of its own; the weapon's is used.
    pub fn start_shot_kick(
        &mut self,
        kick: &GunKickTuning,
        aim_frac: f32,
        weapon_state_bits: u32,
        kick_scale: i32,
        damping_spell_on: bool,
    ) {
        let scales = ShotKickPlayerFactors {
            kick_pct: kick_scale,
            weapon_state_bits,
            dampened_share: 0.0,
            damping_spell_on,
        };
        self.start_shot_kick_scaled(kick, aim_frac, scales);
    }

    /// Register a shot with the full set of scaling inputs.
    ///
    /// A fresh view push replaces whatever push was still running; the gun's
    /// swing speed is added on top of the swing already under way. When the
    /// reduced-kick window is open the weapon's own percent for the current
    /// stance overrides `scales.dampened_share`.
    pub fn start_shot_kick_scaled(
        &mut self,
        kick: &GunKickTuning,
        aim_frac: f32,
        mut scales: ShotKickPlayerFactors,
    ) {
        let aimed = fully_aimed(aim_frac);
        let (floor_stance, percent_stance) = match aimed {
            true => (
                kick.ads_view_kick_min_magnitude,
                kick.scoped_weapon_kick_damped_percent,
            ),
            false => (
                kick.hip_view_kick_min_magnitude,
                kick.hipfire_weapon_kick_damped_percent,
            ),
        };
        if scales.damping_spell_on {
            scales.dampened_share = percent_stance;
        }

        let (hip_view, ads_view) = (kick.hip_view_range(), kick.ads_view_range());
        let (hip_gun, ads_gun) = (kick.hip_gun_range(), kick.ads_gun_range());
        let gun_limits = shot_kick_viewmodel_bounds(aim_frac, hip_gun, ads_gun);
        let view_limits = shot_kick_camera_bounds(aim_frac, hip_view, ads_view);

        // Four rolls per shot, taken in a fixed order so the stream stays in step.
        let rolls: [f32; 4] = std::array::from_fn(|_| self.next_unit());
        let ShotKickPush {
            mut punch_spin_rate,
            viewmodel_velocity_step,
        } = weapon_fire_recoil(view_limits, gun_limits, scales, rolls);

        // bo2zm: a Black Ops II shot never kicks the view more gently than the
        // weapon's floor (scaled the same way the kick itself was).
        let floor = floor_stance * shot_recoil_gain(scales);
        stretch_to_floor(&mut punch_spin_rate, floor);

        self.punch_spin_rate = punch_spin_rate;
        for (speed, extra) in self.gun_swing_rate.iter_mut().zip(viewmodel_velocity_step) {
            *speed += extra;
        }
    }
}

/// Scale a push up so its pitch/yaw strength reaches `floor`.
///
/// Nothing happens when the floor is off, when there is no push at all (there
/// is no direction to stretch), or when the push is already strong enough.
/// Roll rides along with the same factor so the direction is kept.
fn stretch_to_floor(put: &mut [f32; 3], floor: f32) {
    let strength = put[0].hypot(put[1]);
    if floor <= 0.0 || strength <= 0.0 || strength >= floor {
        return;
    }
    put.iter_mut().for_each(|axis| *axis *= floor / strength);
}
