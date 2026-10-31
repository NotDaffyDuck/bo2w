//! What the world does to the view: a landing dips it, a hit flinches it.

use playerstate_t6::game::NO_ENTITY_SLOT;
use weapon_t6::placement::weapon_damage_kick_angles;
use weapon_t6::env::perk;
use weapon_t6::view_bob::{
    HIT_FLINCH_NO_DIRECTION, VIEWMODEL_TOUCHDOWN_FACTOR, CameraTiltSwayFeed, HitFlinchResponse,
    CameraHeightSwayFeed, hard_landing_drop, crash_land_view_dip, damage_feedback_kick, damage_feedback_kick_bo2, land_origin_z,
    view_damage_angles,
};

use super::flags::shot_log_on;
use super::state::{QueuedHurtFlinch, CameraRecoilState, bo2_camera};

/// Tells a landing from the last frame's body state, starts its dip when the fall was
/// long enough, and returns the view's vertical offset this frame.
///
/// A landing is the frame the feet found ground after the last frame had none.
pub(super) fn stamp_and_land_origin_z(
    kick: &mut CameraRecoilState,
    gravity_in: i32,
    origin_in: [f32; 3],
    velocity_in: [f32; 3],
    standing_on: i32,
    scene_clock_ms: i32,
) -> f32 {
    let touched_down = kick.land_seen
        && kick.prev_standing_on == NO_ENTITY_SLOT
        && standing_on != NO_ENTITY_SLOT;
    let fall = if touched_down {
        hard_landing_drop(gravity_in, kick.prev_origin[2], origin_in[2], kick.prev_velocity[2])
    } else {
        None
    };
    if let Some(fall) = fall {
        let dip = crash_land_view_dip(fall, bo2_camera(kick));
        if dip > 0 {
            kick.touchdown_dip = -(dip as f32);
            kick.touchdown_stamp = scene_clock_ms;
            kick.landing_dip = dip;
            if let Some(factor) = kick.t6_land_factor
                && shot_log_on()
            {
                diag::info!(
                    ViewModelPerspective,
                    "t6 land: fall {:.1} dip {} gun scale {:.3} frac {:.2}",
                    fall,
                    dip,
                    factor * VIEWMODEL_TOUCHDOWN_FACTOR,
                    kick.t6_land_frac
                );
            }
        }
    }
    kick.prev_origin = origin_in;
    kick.prev_velocity = velocity_in;
    kick.prev_standing_on = standing_on;
    kick.land_seen = true;
    land_origin_z(scene_clock_ms.wrapping_sub(kick.touchdown_stamp) as f32, kick.touchdown_dip)
}

/// Turns the player state's damage report into the flinch the view plays.
///
/// A new damage event with a direction wins. Failing that, one owed undirected hurt
/// (see [`QueuedHurtFlinch`]) is paid out. For a Black Ops II gun the flinch can come
/// out as nothing, which leaves the last flinch's angles alone but still restarts it.
pub(super) fn stamp_damage_feedback(
    kick: &mut CameraRecoilState,
    hurt: &mut QueuedHurtFlinch,
    damage_ev: u32,
    damage_yaw_deg: u32,
    damage_tilt: u32,
    damage_total: i32,
    view_angs: [f32; 3],
    perks: [u32; 2],
    scene_clock_ms: i32,
) {
    let bo2 = bo2_camera(kick);
    let toughness = perk::held(perks, perk::REDUCEBULLETFLINCH);
    let flinch_for = |yaw: u32, pitch: u32, count: i32| {
        if bo2 {
            damage_feedback_kick_bo2(yaw, pitch, count, view_angs, toughness)
        } else {
            Some(damage_feedback_kick(yaw, pitch, count, view_angs))
        }
    };
    let start_flinch = |kick: &mut CameraRecoilState, flinch: Option<HitFlinchResponse>| {
        if let Some(f) = flinch {
            kick.hurt_kick_pitch = f.hurt_kick_pitch;
            kick.hurt_kick_roll = f.hurt_kick_roll;
        }
        kick.hurt_stamp_ms = scene_clock_ms.max(1);
    };

    let fresh_event =
        kick.damage_seen && damage_ev != kick.prev_damage_event && damage_total != 0;
    if fresh_event {
        start_flinch(kick, flinch_for(damage_yaw_deg, damage_tilt, damage_total));
    } else if hurt.0 > 0 {
        hurt.0 -= 1;
        start_flinch(kick, flinch_for(HIT_FLINCH_NO_DIRECTION, HIT_FLINCH_NO_DIRECTION, 1));
    }
    kick.prev_damage_event = damage_ev;
    kick.damage_seen = true;
}

/// bo2mp: the pitch a hit adds to Black Ops II's shot.
///
/// Its server builds the gun's aim from the view plus the camera's hit kick, always.
/// While the gun is aimed through iron sights (not a scope) the gun model's own hit
/// tilt comes on top (the client sends it; its yaw is dropped). Both are the same
/// curves the camera and the gun model already draw.
pub(super) fn hit_aim_pitch(
    time: i32,
    hurt_stamp_ms: i32,
    hurt_kick_pitch: f32,
    hurt_kick_roll: f32,
    frac: f32,
    hud_crosshair_layer: i32,
    scoped_aiming: bool,
) -> f32 {
    let camera_part = view_damage_angles(CameraTiltSwayFeed {
        org: CameraHeightSwayFeed { gun_raise_share: frac, ..CameraHeightSwayFeed::default() },
        hud_crosshair_layer,
        time,
        hurt_stamp_ms,
        hurt_kick_pitch,
        hurt_kick_roll,
        ..CameraTiltSwayFeed::default()
    })
    .pitch;
    let through_iron_sights = scoped_aiming && frac != 0.0 && hud_crosshair_layer == 0;
    if !through_iron_sights {
        return camera_part;
    }
    camera_part + weapon_damage_kick_angles(time, hurt_stamp_ms, hurt_kick_pitch, hurt_kick_roll, frac, 0)[0]
}

/// bo2mp: how far (pitch, yaw) Black Ops II's iron-sight shot turns off the view.
///
/// The gun's own movement angles are turned into the view's frame with BO2's axes
/// (forward, left, up), and the view is taken away again
/// (CG_CalculateWeaponMovement_SaveAngles).
pub(crate) fn iron_gun_turn(weapon_aim_angles: [f32; 3], view: [f32; 3]) -> [f32; 2] {
    let in_view = game_math::axis_product(
        game_math::euler_to_rows(weapon_aim_angles),
        game_math::euler_to_rows(view),
    );
    let [pitch, yaw, _roll] = game_math::rows_to_euler(in_view);
    let off = |angle: f32, base: f32| weapon_t6::recoil::angle_wrap180(angle - base);
    [off(pitch, view[0]), off(yaw, view[1])]
}
