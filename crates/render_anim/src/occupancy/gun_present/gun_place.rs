//! Where the gun sits relative to the camera each frame: bob, sway, recoil,
//! stance offsets and the Black Ops II gun brain's own pose, composed into the
//! placement root's transform, plus the aim the HUD and the shooting side read.

use asset_game::WeaponChassisSummary;
use bevy::prelude::*;
use frame::EyeSubject;
use game_math::{euler_basis, vec3_norm};
use net::{FrameClock, HostViewClient, ShownWorldFrame, ViewweaponAim};
use playerstate_t6::game::{AvatarState, entity_flag_bits, ledge_climb_bits, movement_state_bits};
use render_scene::{
    HostVisualStage, FIRST_PERSON_WEAPON_SLOT, FIRST_PERSON_OFFHAND_SLOT, LevelScriptModelInstance,
    quat_from_viewmodel_basis,
};
use weapon_t6::kick::{ViewmodelKickReturn, ViewmodelKickPoseState};
use weapon_t6::placement::{
    Bo2GunBob, POSE_BUILD_STAGE_TOTAL, GunStrideSwayFeed,
    GunStrideWaveFeed, GunMotionPhysics, GunPoseBuildStage,
    GunPosePlayerFeed, GunPoseState, GunPostureRestShiftFeed,
    calculate_weapon_movement_bob_waveform, dual_wield_gun_offset,
    compose_gun_pose,
};
use weapon_t6::viewweapon::iron_sight_keeps_axis;
use weapon_t6::viewweapon::{keep_gun_pitch_yaw, gun_view_delta_to_world};

use super::GunMountAnchor;
use super::weapon::fpv_viewmodel_weapon;
use crate::motion::fpv_prepared::PreparedFpv;
use crate::occupancy::peer_body::OtherPlayerBody;
use crate::occupancy::t6_movement::{GunMoveInput, PMF_DIVE_ANY};
use crate::occupancy::other_view::shown_in_third_person;
use crate::occupancy::eye_kick::{
    GunOffset, CameraRecoilState, sum3, bo2_camera,
    iron_gun_turn, placement_to_camera_local,
};
use hud_t6::reticle_plan::calc_crosshair_position;
use hud_t6::fov::fov_half_tan;

/// One stance's gun-kick spring, from the four numbers a weapon lists for it.
fn kick_spring(speed_up_rate: f32, velocity_cap: f32, velocity_falloff: f32, resting_falloff: f32) -> ViewmodelKickReturn {
    ViewmodelKickReturn {
        resting_falloff,
        velocity_falloff,
        velocity_cap,
        speed_up_rate,
    }
}

/// What the Black Ops II gun brain is fed this frame, read off the player state.
fn t6_gun_input(
    pstate: &AvatarState,
    dt: f32,
    ground_speed: f32,
    perks: u64,
    mantle_frac: Option<f32>,
    state: &GunPoseState,
) -> GunMoveInput {
    let (flags, move_flags) = (pstate.e_flags, pstate.movement_state_bits);
    let (weapon_time, velocity, view_angs) = (pstate.weapon_timer, pstate.velocity, pstate.look_angles);
    let (speed, eye_height_goal) = (pstate.speed as f32, pstate.eye_height_goal);
    let fast_sprint_recovery = perks & (1 << weapon_t6::viewmodel::PERK_FASTSPRINTRECOVERY) != 0;
    let sprint_recovery_multiplier =
        weapon_t6::viewmodel::VmDvars::default().perk_sprint_recovery_multiplier;
    let pose = weapon_t6::pose::PoseInput {
        ducked: flags & entity_flag_bits::CROUCH_BIT != 0,
        prone: flags & entity_flag_bits::LYING_FLAT != 0,
        diving: move_flags & PMF_DIVE_ANY != 0,
        on_ladder: move_flags & movement_state_bits::LADDER != 0,
        gun_phase: pstate.weapon_state_t6,
        weaponstate_left: pstate.weapon_state_left_t6,
        gun_timer_ms: weapon_time,
        frac: pstate.weapon_raise_fraction,
        ground_velocity: ground_speed,
        speed,
        velocity,
        look_angles: view_angs,
        step_millis_elapsed: dt,
        fast_sprint_recovery,
        sprint_recovery_multiplier,
        mantle_frac,
    };
    let recoil = &state.gun_recoil;
    GunMoveInput {
        pose,
        frac: pstate.weapon_raise_fraction,
        e_bits: flags,
        movement_state_bits: move_flags,
        step_millis_elapsed: dt,
        sway_phase: pstate.sway_phase as i32,
        ground_velocity: ground_speed,
        waterlevel: pstate.waterlevel,
        eye_height_goal,
        recoil: [recoil.nod_offset, recoil.yaw_shift],
        damage_kick: state.hit_flinch_tilt,
    }
}

/// The placement state a frame starts from, carried over in `kick`.
fn starting_state(kick_in: &CameraRecoilState, now: i32) -> GunPoseState {
    let [nod_offset, yaw_offset] = [kick_in.state.weapon_aim_angles[0], kick_in.state.weapon_aim_angles[1]];
    let [nod_rate, heading_turn_rate] = [kick_in.state.gun_swing_rate[0], kick_in.state.gun_swing_rate[1]];
    let recoil = ViewmodelKickPoseState {
        heading_turn_rate,
        yaw_shift: yaw_offset,
        nod_rate,
        nod_offset,
    };
    let CameraRecoilState {
        placement_shift: motion_shift,
        placement_turn: motion_tilt,
        gun_idle_clock,
        previous_idle_ratio,
        hurt_stamp_ms,
        hurt_kick_pitch,
        hurt_kick_roll,
        ..
    } = *kick_in;
    GunPoseState {
        hit_flinch_stamp: now,
        gun_recoil: recoil,
        sway_springs: kick_in.sway.springs(),
        motion_shift,
        motion_tilt,
        gun_idle_clock,
        previous_idle_ratio,
        hurt_stamp_ms,
        hurt_kick_pitch,
        hurt_kick_roll,
        ..Default::default()
    }
}

/// The bob waveform for this frame; a Black Ops II camera also gets the gun's
/// sprint and dive-to-prone bob.
fn bob_waveform(
    ps: &AvatarState,
    facts: &WeaponChassisSummary,
    ground_speed: f32,
    t6_camera: bool,
) -> weapon_t6::placement::GunStrideSwayState {
    let bo2_gun = t6_camera.then_some(Bo2GunBob {
        sprint: facts.bob.sprint_bob,
        dtp: facts.bob.dtp_bob,
    });
    let eye_height_goal = ps.eye_height_goal;
    calculate_weapon_movement_bob_waveform(GunStrideWaveFeed {
        bo2_gun,
        eye_height_goal,
        ground_velocity: ground_speed,
        sway_phase: ps.sway_phase as u32 as u8,
        movement_state_bits: ps.movement_state_bits,
        gun_raise_share: ps.weapon_raise_fraction,
        waterlevel: ps.waterlevel,
    })
}

/// The weapon's own overlay reticle, else 1 when its overlay is the HUD iris.
fn reticle_id(facts: &WeaponChassisSummary, hud_iris: bool) -> i32 {
    match facts.hud_crosshair_layer {
        0 => i32::from(hud_iris),
        own => own,
    }
}

/// How far the second hand's gun sits from the first, in view space.
fn second_hand_shift(facts: &WeaponChassisSummary) -> [f32; 3] {
    let side = [0.0, 1.0, 0.0];
    dual_wield_gun_offset(0, side, facts.twin_grip_viewmodel_shift)
}

/// The crosshair position for a gun aimed along `weapon_tilt_angle` / `weapon_swing_angle`, or
/// the screen centre when there is no window or no field of view yet.
fn crosshair_for(
    kick: &CameraRecoilState,
    window: Option<&Window>,
    aim_pitch: f32,
    aim_yaw: f32,
) -> [f32; 2] {
    let Some(window) = window.filter(|_| kick.horizontal_view_degrees > 0.0) else {
        return [0.0, 0.0];
    };
    let aspect = window.width() / window.height().max(1.0);
    let (tan_x, tan_y) = fov_half_tan(kick.horizontal_view_degrees, aspect);
    let (ahead, right, up) = euler_basis(kick.frame_look_angles);
    calc_crosshair_position(aim_pitch, aim_yaw, [ahead, right, up], tan_x, tan_y)
}

/// Computes the viewmodel's placement for this frame, stores it on the
/// placement root, publishes the gun aim, and tells the scene where each hand's
/// model is in the world.
#[allow(clippy::too_many_arguments, reason = "kept as written")]
pub fn place_viewmodel(
    clk: Res<FrameClock>,
    presented: Res<ShownWorldFrame>,
    own_client: Res<HostViewClient>,
    prepared: Res<PreparedFpv>,
    mut vk: ResMut<CameraRecoilState>,
    cg_gun_in: Res<GunOffset>,
    mut aim: ResMut<ViewweaponAim>,
    primary_window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut placement_root: Query<&mut Transform, (With<GunMountAnchor>, (Without<OtherPlayerBody>, Without<LevelScriptModelInstance>))>,
    view: Res<EyeSubject>,
    mut host_scene: ResMut<HostVisualStage>,
) {
    *aim = ViewweaponAim::default();
    let (Ok(mut root), Some(avatar)) = (placement_root.single_mut(), presented.player(own_client.0)) else {
        return;
    };
    let third_person = shown_in_third_person(&presented, own_client.0, view.replaying_final_death());
    if third_person {
        return;
    }
    let Some(table) = prepared.table() else {
        return;
    };
    let shown = fpv_viewmodel_weapon(avatar, table);
    let Some(info) = table.info_of(shown) else {
        return;
    };
    let step_millis_elapsed = clk.step_duration_seconds();

    let mut state = starting_state(&vk, clk.time());

    let hud_crosshair_layer = reticle_id(&info, table.hud_iris_overlay(shown));
    let ground_velocity = vec3_norm([avatar.velocity[0], avatar.velocity[1], 0.0]);
    let t6_camera = bo2_camera(&vk);
    let waveform = bob_waveform(avatar, &info, ground_velocity, t6_camera);
    let mut steps: [GunPoseBuildStage; POSE_BUILD_STAGE_TOTAL] =
        std::array::from_fn(|_| GunPoseBuildStage::Sway);
    let movement = info.movement;
    let pos_frac = avatar.weapon_raise_fraction;
    let hip = &info.kick;
    let (weapon_time, velocity, aim_angles) = (avatar.weapon_timer, avatar.velocity, avatar.look_angles);
    let (scoped_aiming, ads_aim_pitch) = (info.scoped_aiming, info.aimed_pitch);
    let goggles_equip_duration = info.goggles_equip_duration;
    let ps_inputs = GunPosePlayerFeed {
        ent_bits: avatar.e_flags,
        gun_raise_share: pos_frac,
        gun_timer_ms: weapon_time,
        scoped_aiming,
        hud_crosshair_layer,
        ..Default::default()
    };
    let move_kin = GunMotionPhysics {
        bo2_mantle: ledge_climb_bits::bo2_weapon_up(avatar),
        gun_phase: avatar.weapon_state_t6,
        gun_phase_alt: avatar.weapon_state_left_t6,
        movement_state_bits: avatar.movement_state_bits,
        speed: avatar.speed as f32,
        velocity,
        look_angles: aim_angles,
        ground_velocity,
        step_millis_elapsed,
    };
    let mut contrib_in = compose_gun_pose(
        &mut state,
        ps_inputs,
        GunPostureRestShiftFeed {
            ducked_ofs: info.ducked_ofs,
            prone_ofs: info.prone_ofs,
            aimed_pitch: ads_aim_pitch,
            goggles_equip_duration,
        },
        Default::default(),
        movement,
        move_kin,
        GunStrideSwayFeed {
            ads_bob_factor: info.ads_bob_factor,
        },
        info.idle,
        Option::from(waveform),
        kick_spring(
            hip.hipfire_weapon_kick_accel,
            hip.hipfire_weapon_kick_velocity_cap,
            hip.hipfire_weapon_kick_velocity_falloff,
            hip.hipfire_weapon_kick_resting_falloff,
        ),
        kick_spring(
            hip.scoped_weapon_kick_accel,
            hip.scoped_weapon_kick_velocity_cap,
            hip.scoped_weapon_kick_velocity_falloff,
            hip.scoped_weapon_kick_resting_falloff,
        ),
        hip.weapon_pitch_ceiling,
        hip.weapon_yaw_ceiling,
        0.0,
        &mut steps,
    );
    let GunPoseState {
        motion_shift,
        motion_tilt,
        gun_idle_clock,
        previous_idle_ratio,
        ..
    } = state;
    vk.placement_shift = motion_shift;
    vk.placement_turn = motion_tilt;
    vk.gun_idle_clock = gun_idle_clock;
    vk.previous_idle_ratio = previous_idle_ratio;

    // A Black Ops II gun is posed by its own rules for sway, bob and drift; the
    // generic assembly above still supplies the recoil spring and damage kick.
    let perks = presented
        .snapshot()
        .and_then(|s| s.meta.for_client(own_client.0))
        .map_or(0, |meta| meta.t6_perks);
    let climbing = avatar.movement_state_bits & movement_state_bits::MANTLE != 0 && ledge_climb_bits::bo2_weapon_up(avatar);
    let mantle_frac = climbing.then(|| ledge_climb_bits::bo2_progress(avatar)).flatten();
    if t6_camera {
        let input = t6_gun_input(avatar, step_millis_elapsed, ground_velocity, perks, mantle_frac, &state);
        if let Some((angles, origin)) = vk.t6.gun(&input) {
            (contrib_in.angles, contrib_in.origin) = (angles, origin);
        }
    }

    let offset_origin = sum3(contrib_in.origin, cg_gun_in.as_xyz());
    let landed = sum3(offset_origin, vk.gun_landing_dip);
    let dual = avatar.previous_gun_hand == 1;
    let origin: [f32; 3] = if dual {
        let shift = second_hand_shift(&info);
        [landed[0] + shift[0], landed[1] + shift[1], landed[2] + shift[2]]
    } else {
        landed
    };

    let ads = info.scoped_aiming;
    let from_combined_direction = iron_sight_keeps_axis(ads, pos_frac, hud_crosshair_layer);
    let look_rotation = vk.frame_look_angles;
    let saved =
        keep_gun_pitch_yaw(contrib_in.angles, look_rotation, ads, pos_frac, hud_crosshair_layer);
    let (weapon_tilt_angle, weapon_swing_angle) = (saved[0], saved[1]);
    // Through iron sights (no scope overlay, not fully lowered) the shot leaves
    // along the gun, so the aim takes the gun's own turn while the camera does
    // not. Off iron sights the aim gives it back.
    let iron_sighted = vk
        .t6
        .facts
        .is_some_and(|(sight, _)| sight.scoped_aiming && !sight.overlay);
    let through_iron = pos_frac != 0.0 && iron_sighted;
    let gun_aim = if vk.aim_kick && t6_camera && through_iron {
        iron_gun_turn(contrib_in.angles, look_rotation)
    } else {
        [0.0; 2]
    };
    vk.set_gun_aim(gun_aim);

    let [xhair_x, xhair_y] = crosshair_for(&vk, primary_window.single().ok(), weapon_tilt_angle, weapon_swing_angle);
    *aim = ViewweaponAim {
        live: true,
        weapon_tilt_angle,
        weapon_swing_angle,
        crosshair_x: xhair_x,
        crosshair_y: xhair_y,
        from_combined_direction,
    };

    let delta = gun_view_delta_to_world(origin, look_rotation);
    let eye = vk.frame_eye_position;
    let scene_anchor = [eye[0] + delta[0], eye[1] + delta[1], eye[2] + delta[2]];
    let world_quat = Some(quat_from_viewmodel_basis(contrib_in.angles, look_rotation));
    let hands: &[_] = if dual {
        &[FIRST_PERSON_WEAPON_SLOT, FIRST_PERSON_OFFHAND_SLOT]
    } else {
        &[FIRST_PERSON_WEAPON_SLOT]
    };
    for &entnum in hands {
        host_scene.scene.store_pose_origin_quat(entnum, scene_anchor, world_quat);
    }
    *root = placement_to_camera_local(origin, contrib_in.angles);
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn kick_spring_maps_each_number_to_its_field() {
        let spring = kick_spring(1.0, 2.0, 3.0, 4.0);
        assert_eq!(
            (spring.speed_up_rate, spring.velocity_cap, spring.velocity_falloff, spring.resting_falloff),
            (1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn crosshair_is_screen_centre_without_a_window() {
        let kick = CameraRecoilState::default();
        assert_eq!(crosshair_for(&kick, None, 1.0, 2.0), [0.0, 0.0]);
    }
}
