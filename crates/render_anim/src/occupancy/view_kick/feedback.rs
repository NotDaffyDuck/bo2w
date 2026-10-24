//! What the world does to the view: a landing dips it, a hit flinches it.

use playerstate_t6::game::ENTITYNUM_NONE;
use weapon_t6::placement::weapon_damage_kick_angles;
use weapon_t6::view_bob::{
    PERK_BULLETFLINCH, VIEW_DAMAGE_UNDIRECTED, VIEWWEAPON_LAND_SCALE, ViewAngleBobInputs, ViewDamageFeedback,
    ViewOrgBobInputs, crash_land_fall_height, crash_land_view_dip, damage_feedback_kick, damage_feedback_kick_bo2, land_origin_z,
    view_damage_angles,
};

use super::flags::shot_log_on;
use super::state::{PendingViewHurt, SessionViewKick, bo2_camera};

/// Tells a landing from the last frame's body state, starts its dip when the fall was
/// long enough, and returns the view's vertical offset this frame.
///
/// A landing is the frame the feet found ground after the last frame had none.
pub(super) fn stamp_and_land_origin_z(
    kick: &mut SessionViewKick,
    gravity: i32,
    origin: [f32; 3],
    velocity: [f32; 3],
    ground_entity: i32,
    cg_time: i32,
) -> f32 {
    let touched_down = kick.have_land_prev
        && kick.last_ground_entity == ENTITYNUM_NONE
        && ground_entity != ENTITYNUM_NONE;
    let fall = if touched_down {
        crash_land_fall_height(gravity, kick.last_origin[2], origin[2], kick.last_velocity[2])
    } else {
        None
    };
    if let Some(fall) = fall {
        let dip = crash_land_view_dip(fall, bo2_camera(kick));
        if dip > 0 {
            kick.land_change = -(dip as f32);
            kick.land_time = cg_time;
            kick.land_view_dip = dip;
            if let Some(factor) = kick.t6_land_factor
                && shot_log_on()
            {
                diag::info!(
                    Fpv,
                    "t6 land: fall {:.1} dip {} gun scale {:.3} frac {:.2}",
                    fall,
                    dip,
                    factor * VIEWWEAPON_LAND_SCALE,
                    kick.t6_land_frac
                );
            }
        }
    }
    kick.last_origin = origin;
    kick.last_velocity = velocity;
    kick.last_ground_entity = ground_entity;
    kick.have_land_prev = true;
    land_origin_z(cg_time.wrapping_sub(kick.land_time) as f32, kick.land_change)
}

/// Turns the player state's damage report into the flinch the view plays.
///
/// A new damage event with a direction wins. Failing that, one owed undirected hurt
/// (see [`PendingViewHurt`]) is paid out. For a Black Ops II gun the flinch can come
/// out as nothing, which leaves the last flinch's angles alone but still restarts it.
pub(super) fn stamp_damage_feedback(
    kick: &mut SessionViewKick,
    hurt: &mut PendingViewHurt,
    damage_event: u32,
    damage_yaw: u32,
    damage_pitch: u32,
    damage_count: i32,
    viewangles: [f32; 3],
    perks0: u32,
    cg_time: i32,
) {
    let bo2 = bo2_camera(kick);
    let toughness = (perks0 & PERK_BULLETFLINCH) != 0;
    let flinch_for = |yaw: u32, pitch: u32, count: i32| {
        if bo2 {
            damage_feedback_kick_bo2(yaw, pitch, count, viewangles, toughness)
        } else {
            Some(damage_feedback_kick(yaw, pitch, count, viewangles))
        }
    };
    let start_flinch = |kick: &mut SessionViewKick, flinch: Option<ViewDamageFeedback>| {
        if let Some(f) = flinch {
            kick.v_dmg_pitch = f.v_dmg_pitch;
            kick.v_dmg_roll = f.v_dmg_roll;
        }
        kick.damage_time = cg_time.max(1);
    };

    let fresh_event =
        kick.have_damage_prev && damage_event != kick.last_damage_event && damage_count != 0;
    if fresh_event {
        start_flinch(kick, flinch_for(damage_yaw, damage_pitch, damage_count));
    } else if hurt.0 > 0 {
        hurt.0 -= 1;
        start_flinch(kick, flinch_for(VIEW_DAMAGE_UNDIRECTED, VIEW_DAMAGE_UNDIRECTED, 1));
    }
    kick.last_damage_event = damage_event;
    kick.have_damage_prev = true;
}

/// bo2mp: the pitch a hit adds to Black Ops II's shot.
///
/// Its server builds the gun's aim from the view plus the camera's hit kick, always.
/// While the gun is aimed through iron sights (not a scope) the gun model's own hit
/// tilt comes on top (the client sends it; its yaw is dropped). Both are the same
/// curves the camera and the gun model already draw.
pub(super) fn hit_aim_pitch(
    time: i32,
    damage_time: i32,
    v_dmg_pitch: f32,
    v_dmg_roll: f32,
    frac: f32,
    overlay_reticle: i32,
    aim_down_sight: bool,
) -> f32 {
    let camera_part = view_damage_angles(ViewAngleBobInputs {
        org: ViewOrgBobInputs { weapon_pos_frac: frac, ..ViewOrgBobInputs::default() },
        overlay_reticle,
        time,
        damage_time,
        v_dmg_pitch,
        v_dmg_roll,
        ..ViewAngleBobInputs::default()
    })
    .pitch;
    let through_iron_sights = aim_down_sight && frac != 0.0 && overlay_reticle == 0;
    if !through_iron_sights {
        return camera_part;
    }
    camera_part + weapon_damage_kick_angles(time, damage_time, v_dmg_pitch, v_dmg_roll, frac, 0)[0]
}

/// bo2mp: how far (pitch, yaw) Black Ops II's iron-sight shot turns off the view.
///
/// The gun's own movement angles are turned into the view's frame with BO2's axes
/// (forward, left, up), and the view is taken away again
/// (CG_CalculateWeaponMovement_SaveAngles).
pub(crate) fn iron_gun_turn(gun_angles: [f32; 3], view: [f32; 3]) -> [f32; 2] {
    let in_view = game_math::matrix_multiply(
        game_math::angles_to_axis(gun_angles),
        game_math::angles_to_axis(view),
    );
    let [pitch, yaw, _roll] = game_math::axis_to_angles(in_view);
    let off = |angle: f32, base: f32| weapon_t6::recoil::angle_wrap180(angle - base);
    [off(pitch, view[0]), off(yaw, view[1])]
}
