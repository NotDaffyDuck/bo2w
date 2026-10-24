//! Pinning tests for the view-kick helpers: what each small pure piece must keep doing
//! while the module is reorganised.

use super::breath::diff_track;
use super::camera::{apply_fpv_lens_fov, earthquake_pose};
use super::feedback::{stamp_and_land_origin_z, stamp_damage_feedback};
use super::params::kick_params;
use super::{
    GunOffset, HoldBreath, PendingViewHurt, SessionViewKick, apply_cg_gun_offset_view, apply_viewweapon_land_view, bo2_camera,
    iw_view_placement_to_bevy_camera_local,
};
use asset_game::{WeaponBodyFacts, WeaponKickFacts};
use bevy::prelude::*;
use hud_t6::fov::{AdsZoom, CG_FOV_DEFAULT, ViewFovInputs, vertical_fov_deg, view_fov, zoom_sensitivity};
use net::{ClientActionInput, PresentedSnapshot};
use playerstate_t6::game::ENTITYNUM_NONE;
use render_scene::{FpvLens, WorldCameraPose};
use weapon_t6::view_bob::VIEW_DAMAGE_UNDIRECTED;
use crate::occupancy::camera_shake::CameraShakes;
use bevy::ecs::system::RunSystemOnce;
use weapon_t6::view_bob::{
    crash_land_fall_height, crash_land_view_dip, damage_feedback_kick, damage_feedback_kick_bo2,
    land_origin_z,
};

#[test]
fn every_kick_fact_reaches_the_matching_kick_param() {
    macro_rules! carried {
        ($($field:ident),* $(,)?) => {{
            let mut n = 0.0f32;
            let facts = WeaponKickFacts {
                $($field: { n += 1.0; n },)*
                ..WeaponKickFacts::default()
            };
            let params = kick_params(&facts);
            let mut n = 0.0f32;
            $(
                n += 1.0;
                assert_eq!(params.$field, n, stringify!($field));
            )*
            n
        }};
    }
    let count = carried!(
        f_ads_view_kick_center_speed,
        f_hip_view_kick_center_speed,
        gun_max_pitch,
        gun_max_yaw,
        ads_gun_kick_reduced_kick_percent,
        ads_gun_kick_pitch_min,
        ads_gun_kick_pitch_max,
        ads_gun_kick_yaw_min,
        ads_gun_kick_yaw_max,
        ads_gun_kick_accel,
        ads_gun_kick_speed_max,
        ads_gun_kick_speed_decay,
        ads_gun_kick_static_decay,
        ads_view_kick_pitch_min,
        ads_view_kick_pitch_max,
        ads_view_kick_yaw_min,
        ads_view_kick_yaw_max,
        hip_gun_kick_reduced_kick_percent,
        hip_gun_kick_pitch_min,
        hip_gun_kick_pitch_max,
        hip_gun_kick_yaw_min,
        hip_gun_kick_yaw_max,
        hip_gun_kick_accel,
        hip_gun_kick_speed_max,
        hip_gun_kick_speed_decay,
        hip_gun_kick_static_decay,
        hip_view_kick_pitch_min,
        hip_view_kick_pitch_max,
        hip_view_kick_yaw_min,
        hip_view_kick_yaw_max,
        hip_view_kick_min_magnitude,
        ads_view_kick_min_magnitude,
    );
    assert_eq!(count, 32.0);
}

#[test]
fn bo2_camera_follows_the_aim_kick_flag() {
    let mut kick = SessionViewKick::default();
    assert!(!bo2_camera(&kick));
    kick.aim_kick = true;
    assert!(bo2_camera(&kick));
}

#[test]
fn offsets_add_up_per_axis() {
    assert_eq!(
        apply_cg_gun_offset_view([1.0, 2.0, 3.0], [0.5, -2.0, 10.0]),
        [1.5, 0.0, 13.0]
    );
    assert_eq!(
        apply_viewweapon_land_view([1.0, 2.0, 3.0], [-1.0, 0.25, 4.0]),
        [0.0, 2.25, 7.0]
    );
    let gun = GunOffset { x: 1.0, y: 2.0, z: 3.0 };
    assert_eq!(gun.xyz(), [1.0, 2.0, 3.0]);
}

#[test]
fn a_view_placement_maps_forward_right_up_onto_the_camera_axes() {
    let t = iw_view_placement_to_bevy_camera_local([3.0, 5.0, 7.0], [0.0; 3]);
    assert_eq!(t.translation, Vec3::new(5.0, 7.0, -3.0));
    assert_eq!(t.scale, Vec3::ONE);
    let turned = iw_view_placement_to_bevy_camera_local([0.0; 3], [10.0, 20.0, 30.0]);
    let expected = crate::anim::fpv_pose::placement_angles_to_bevy_camera_quat([10.0, 20.0, 30.0]);
    assert_eq!(turned.rotation, expected);
}

#[test]
fn a_landing_after_a_fall_starts_a_dip_that_a_flat_step_does_not() {
    let mut kick = SessionViewKick::default();
    // First frame in the air: only remembered.
    let first = stamp_and_land_origin_z(
        &mut kick,
        800,
        [0.0, 0.0, 200.0],
        [0.0, 0.0, -900.0],
        ENTITYNUM_NONE,
        1000,
    );
    assert_eq!(first, land_origin_z(1000.0, 0.0));
    assert!(kick.have_land_prev);
    assert_eq!(kick.last_origin, [0.0, 0.0, 200.0]);
    assert_eq!(kick.last_velocity, [0.0, 0.0, -900.0]);
    assert_eq!(kick.last_ground_entity, ENTITYNUM_NONE);
    assert_eq!(kick.land_time, 0);
    // Touching ground after the fall.
    let landed_at = [0.0, 0.0, 0.0];
    let fall = crash_land_fall_height(800, 200.0, 0.0, -900.0);
    let dip = fall.map_or(0, |f| crash_land_view_dip(f, false));
    let z = stamp_and_land_origin_z(&mut kick, 800, landed_at, [0.0; 3], 5, 1100);
    if dip > 0 {
        assert_eq!(kick.land_change, -(dip as f32));
        assert_eq!(kick.land_time, 1100);
        assert_eq!(kick.land_view_dip, dip);
    } else {
        assert_eq!(kick.land_time, 0);
    }
    assert_eq!(z, land_origin_z((1100 - kick.land_time) as f32, kick.land_change));
    assert_eq!(kick.last_ground_entity, 5);
    // Still on the ground: nothing new starts.
    let before = (kick.land_change, kick.land_time, kick.land_view_dip);
    stamp_and_land_origin_z(&mut kick, 800, landed_at, [0.0; 3], 5, 1200);
    assert_eq!((kick.land_change, kick.land_time, kick.land_view_dip), before);
}

#[test]
fn the_landing_dip_is_the_hard_one_for_a_black_ops_ii_camera() {
    // A big fall: both cameras dip; the dip each gets is whatever the dip rule says for it.
    for bo2 in [false, true] {
        let mut kick = SessionViewKick::default();
        kick.aim_kick = bo2;
        stamp_and_land_origin_z(&mut kick, 800, [0.0, 0.0, 400.0], [0.0, 0.0, -1000.0], ENTITYNUM_NONE, 10);
        stamp_and_land_origin_z(&mut kick, 800, [0.0; 3], [0.0; 3], 3, 20);
        let want = crash_land_fall_height(800, 400.0, 0.0, -1000.0).map_or(0, |f| crash_land_view_dip(f, bo2));
        assert_eq!(kick.land_view_dip, want, "bo2 {bo2}");
        assert_eq!(kick.land_time, if want > 0 { 20 } else { 0 });
    }
}

#[test]
fn a_hit_stamps_the_flinch_once_per_new_damage_event() {
    let angles = [10.0, 20.0, 0.0];
    for bo2 in [false, true] {
        let mut kick = SessionViewKick::default();
        kick.aim_kick = bo2;
        let mut hurt = PendingViewHurt(0);
        // The first frame only remembers the event number.
        stamp_damage_feedback(&mut kick, &mut hurt, 3, 64, 1, 30, angles, 0, 500);
        assert_eq!(kick.damage_time, 0);
        assert!(kick.have_damage_prev);
        assert_eq!(kick.last_damage_event, 3);
        // The same event again: nothing.
        stamp_damage_feedback(&mut kick, &mut hurt, 3, 64, 1, 30, angles, 0, 600);
        assert_eq!(kick.damage_time, 0);
        // A new event: stamped at the time.
        stamp_damage_feedback(&mut kick, &mut hurt, 4, 64, 1, 30, angles, 0, 700);
        assert_eq!(kick.damage_time, 700);
        let want = if bo2 {
            damage_feedback_kick_bo2(64, 1, 30, angles, false).unwrap()
        } else {
            damage_feedback_kick(64, 1, 30, angles)
        };
        assert_eq!((kick.v_dmg_pitch, kick.v_dmg_roll), (want.v_dmg_pitch, want.v_dmg_roll));
        assert_eq!(kick.last_damage_event, 4);
        // A new event with no damage count does not stamp.
        stamp_damage_feedback(&mut kick, &mut hurt, 5, 64, 1, 0, angles, 0, 800);
        assert_eq!(kick.damage_time, 700);
    }
}

#[test]
fn a_pending_hurt_flinches_without_a_direction_and_is_used_up() {
    let angles = [0.0; 3];
    let mut kick = SessionViewKick::default();
    let mut hurt = PendingViewHurt(2);
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, 0, 0);
    assert_eq!(hurt.0, 1);
    assert_eq!(kick.damage_time, 1, "time 0 is stamped as 1");
    let want = damage_feedback_kick(VIEW_DAMAGE_UNDIRECTED, VIEW_DAMAGE_UNDIRECTED, 1, angles);
    assert_eq!((kick.v_dmg_pitch, kick.v_dmg_roll), (want.v_dmg_pitch, want.v_dmg_roll));
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, 0, 900);
    assert_eq!(hurt.0, 0);
    assert_eq!(kick.damage_time, 900);
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, 0, 1000);
    assert_eq!(kick.damage_time, 900);
}

#[test]
fn a_directed_hit_wins_over_a_pending_hurt() {
    let mut kick = SessionViewKick::default();
    let mut hurt = PendingViewHurt(0);
    stamp_damage_feedback(&mut kick, &mut hurt, 1, 0, 0, 10, [0.0; 3], 0, 100);
    hurt.0 = 1;
    stamp_damage_feedback(&mut kick, &mut hurt, 2, 0, 0, 10, [0.0; 3], 0, 200);
    assert_eq!(hurt.0, 1, "the pending hurt waits while a real hit is stamped");
    assert_eq!(kick.damage_time, 200);
}

#[test]
fn the_bo2_flinch_can_keep_the_last_angles() {
    // Toughness (bullet flinch perk) at a small hit: the BO2 rule may give no punch at all;
    // the angles then stay and only the time moves.
    let perk = weapon_t6::view_bob::PERK_BULLETFLINCH;
    let mut kick = SessionViewKick::default();
    kick.aim_kick = true;
    kick.v_dmg_pitch = 1.25;
    kick.v_dmg_roll = -0.5;
    let mut hurt = PendingViewHurt(0);
    stamp_damage_feedback(&mut kick, &mut hurt, 1, 0, 0, 1, [0.0; 3], perk, 10);
    stamp_damage_feedback(&mut kick, &mut hurt, 2, 0, 0, 1, [0.0; 3], perk, 20);
    assert_eq!(kick.damage_time, 20);
    match damage_feedback_kick_bo2(0, 0, 1, [0.0; 3], true) {
        Some(p) => assert_eq!((kick.v_dmg_pitch, kick.v_dmg_roll), (p.v_dmg_pitch, p.v_dmg_roll)),
        None => assert_eq!((kick.v_dmg_pitch, kick.v_dmg_roll), (1.25, -0.5)),
    }
}

#[test]
fn clearing_for_a_new_life_empties_every_remembered_value() {
    let mut kick = SessionViewKick::default();
    kick.placement_move_origin = [1.0; 3];
    kick.placement_move_angles = [1.0; 3];
    kick.weap_idle_time = 5;
    kick.last_idle_factor = 1.0;
    kick.view_last_idle_factor = 1.0;
    kick.land_change = -3.0;
    kick.land_time = 9;
    kick.land_view_dip = 3;
    kick.viewweapon_land_z = 2.0;
    kick.viewweapon_land_view = [1.0; 3];
    kick.damage_time = 7;
    kick.v_dmg_pitch = 1.0;
    kick.v_dmg_roll = 1.0;
    kick.last_origin = [1.0; 3];
    kick.last_velocity = [1.0; 3];
    kick.last_ground_entity = 4;
    kick.have_land_prev = true;
    kick.last_damage_event = 8;
    kick.have_damage_prev = true;
    kick.seeded_this_frame = 2;
    kick.last_weapon_pos_frac = 0.5;
    kick.b_position_to_ads = false;
    kick.aim_applied = [1.0; 2];
    kick.idle_aim_applied = [1.0; 2];
    kick.idle_aim_pending = [1.0; 2];
    kick.hit_aim_applied = 1.0;
    kick.hit_aim_pending = 1.0;
    kick.bob_aim_applied = [1.0; 2];
    kick.bob_aim_pending = [1.0; 2];
    kick.gun_aim_applied = [1.0; 2];
    kick.gun_aim_pending = [1.0; 2];
    kick.gun_aim_set = true;
    kick.hold_breath.fired(1.0, true);
    kick.vzoom_in = true;
    kick.vzoom_scoped = true;
    kick.optic_ready = true;
    // Things a new life keeps.
    kick.horiz_fov_deg = 65.0;
    kick.aim_kick = true;
    kick.last_weapon_id = 12;
    kick.clear_for_new_life();
    let mut want = SessionViewKick::default();
    want.b_position_to_ads = true;
    assert_eq!(kick.placement_move_origin, [0.0; 3]);
    assert_eq!(kick.placement_move_angles, [0.0; 3]);
    assert_eq!(kick.weap_idle_time, 0);
    assert_eq!((kick.last_idle_factor, kick.view_last_idle_factor), (0.0, 0.0));
    assert_eq!((kick.land_change, kick.land_time, kick.land_view_dip), (0.0, 0, 0));
    assert_eq!(kick.viewweapon_land_z, 0.0);
    assert_eq!(kick.viewweapon_land_view, [0.0; 3]);
    assert_eq!((kick.damage_time, kick.v_dmg_pitch, kick.v_dmg_roll), (0, 0.0, 0.0));
    assert_eq!(kick.last_origin, [0.0; 3]);
    assert_eq!(kick.last_velocity, [0.0; 3]);
    assert_eq!(kick.last_ground_entity, 0);
    assert!(!kick.have_land_prev && !kick.have_damage_prev);
    assert_eq!(kick.last_damage_event, 0);
    assert_eq!(kick.seeded_this_frame, 0);
    assert_eq!(kick.last_weapon_pos_frac, 0.0);
    assert!(kick.b_position_to_ads);
    assert_eq!(kick.aim_applied, [0.0; 2]);
    assert_eq!(kick.idle_aim_applied, [0.0; 2]);
    assert_eq!(kick.idle_aim_pending, [0.0; 2]);
    assert_eq!((kick.hit_aim_applied, kick.hit_aim_pending), (0.0, 0.0));
    assert_eq!(kick.bob_aim_applied, [0.0; 2]);
    assert_eq!(kick.bob_aim_pending, [0.0; 2]);
    assert_eq!(kick.gun_aim_applied, [0.0; 2]);
    assert_eq!(kick.gun_aim_pending, [0.0; 2]);
    assert!(!kick.gun_aim_set);
    assert_eq!(kick.hold_breath, HoldBreath::default());
    assert!(!kick.vzoom_in && !kick.vzoom_scoped && !kick.optic_ready);
    assert_eq!(want.hold_breath, kick.hold_breath);
    // Kept across a life.
    assert_eq!(kick.horiz_fov_deg, 65.0);
    assert!(kick.aim_kick);
    assert_eq!(kick.last_weapon_id, 12);
}

#[test]
fn the_gun_turn_the_aim_holds_is_queued_as_a_change() {
    let mut kick = SessionViewKick::default();
    kick.set_gun_aim([2.0, -1.0]);
    assert_eq!(kick.gun_aim_applied, [2.0, -1.0]);
    assert_eq!(kick.gun_aim_pending, [2.0, -1.0]);
    assert!(kick.gun_aim_set);
    kick.set_gun_aim([0.5, -1.0]);
    assert_eq!(kick.gun_aim_applied, [0.5, -1.0]);
    // The pending part is the sum of the changes: 2.0 then -1.5 on pitch, -1.0 then 0 on yaw.
    assert_eq!(kick.gun_aim_pending, [0.5, -1.0]);
}

#[test]
fn the_aim_movement_adds_drift_hit_bob_and_gun_turn() {
    let mut kick = SessionViewKick::default();
    kick.idle_aim_applied = [1.0, 10.0];
    kick.hit_aim_applied = 2.0;
    kick.bob_aim_applied = [4.0, 20.0];
    kick.gun_aim_applied = [8.0, 40.0];
    // The hit has pitch only.
    assert_eq!(kick.aim_movement(), [15.0, 70.0]);
}

#[test]
fn the_breath_scale_tracks_its_target_without_overshooting() {
    assert_eq!(diff_track(0.0, 1.0, 1.0, 0.1), 0.9);
    // A step past the target lands on it.
    assert_eq!(diff_track(0.0, 1.0, 20.0, 0.1), 0.0);
    // A gap under the dead zone snaps.
    assert_eq!(diff_track(1.0, 1.0005, 1.0, 0.1), 1.0);
    assert_eq!(diff_track(4.5, 1.0, 6.0, 0.05), 1.0 + 6.0 * 3.5 * 0.05);
}

#[test]
fn a_shake_turns_the_view_after_the_quakes_and_nothing_shaking_leaves_it() {
    let presented = PresentedSnapshot::default();
    let pose = WorldCameraPose { origin: [10.0, 20.0, 30.0], angles: [1.0, 2.0, 3.0] };
    let mut calm = CameraShakes::default();
    let same = earthquake_pose(pose, &presented, 1000, &mut calm);
    assert_eq!(same.origin, pose.origin);
    assert_eq!(same.angles, pose.angles);
    // With a shake running the pose matches the shake applied by hand.
    let mut a = CameraShakes::default();
    a.start(2.0, 5000, [0.0; 3], 0.0, 1000, pose.angles);
    let mut b = a.clone();
    let shaken = earthquake_pose(pose, &presented, 1200, &mut a);
    let mut by_hand = pose.angles;
    b.apply(&mut by_hand, 1200, pose.origin);
    assert_eq!(shaken.angles, by_hand);
    assert_eq!(shaken.origin, pose.origin);
    assert_ne!(shaken.angles, pose.angles);
}

fn lens_world() -> World {
    let mut world = World::new();
    world.spawn((FpvLens, Projection::Perspective(PerspectiveProjection::default())));
    world
}

fn lens_fov(world: &mut World) -> f32 {
    let mut q = world.query::<&Projection>();
    match q.single(world).unwrap() {
        Projection::Perspective(p) => p.fov,
        _ => panic!("not a perspective lens"),
    }
}

#[test]
fn the_lens_takes_the_view_fov_and_the_look_scale_follows_it() {
    for (pm_type, frac, to_ads) in [(0, 0.0, true), (0, 1.0, true), (0, 0.5, false), (6, 0.0, true)] {
        let mut world = lens_world();
        let mut actions = ClientActionInput::default();
        actions.shellshock_look_scale = 0.5;
        let facts = WeaponBodyFacts {
            body_resolved: true,
            aim_down_sight: true,
            ads_zoom_fov: 40.0,
            ads_zoom_in_frac: 0.3,
            ads_zoom_out_frac: 0.7,
            ..WeaponBodyFacts::default()
        };
        let base = 80.0;
        let got = world
            .run_system_once(move |mut lenses: Query<&mut Projection, With<FpvLens>>| {
                let mut actions = actions.clone();
                let h = apply_fpv_lens_fov(&mut lenses, base, pm_type, 0, 0, frac, 7, Some(facts), to_ads, Some(&mut actions));
                (h, actions.fov_scale)
            })
            .unwrap();
        let ads_target = 40.0;
        let want = view_fov(&ViewFovInputs {
            cg_fov: base,
            default_view: pm_type == playerstate_t6::game::PM_TYPE_INTERMISSION,
            link_flags: 0,
            e_flags: 0,
            has_weapon: true,
            zoom: AdsZoom { fov: [ads_target; 3], zoom_in_frac: 0.3, zoom_out_frac: 0.7 },
            pos_frac: frac,
            to_ads,
            ..ViewFovInputs::default()
        });
        assert_eq!(got.0, Some(want), "pm {pm_type} frac {frac}");
        assert_eq!(got.1, zoom_sensitivity(want, CG_FOV_DEFAULT) * 0.5);
        assert_eq!(lens_fov(&mut world), vertical_fov_deg(want).to_radians());
    }
}

#[test]
fn a_gun_without_resolved_facts_does_not_zoom_and_no_weapon_is_a_default_view() {
    let mut world = lens_world();
    let unresolved = WeaponBodyFacts { body_resolved: false, aim_down_sight: true, ads_zoom_fov: 20.0, ..WeaponBodyFacts::default() };
    let got = world
        .run_system_once(move |mut lenses: Query<&mut Projection, With<FpvLens>>| {
            apply_fpv_lens_fov(&mut lenses, 75.0, 0, 0, 0, 1.0, 7, Some(unresolved), true, None)
        })
        .unwrap();
    let plain = world
        .run_system_once(move |mut lenses: Query<&mut Projection, With<FpvLens>>| {
            apply_fpv_lens_fov(&mut lenses, 75.0, 0, 0, 0, 1.0, 7, None, true, None)
        })
        .unwrap();
    assert_eq!(got, plain);
    assert_eq!(lens_fov(&mut world), vertical_fov_deg(plain.unwrap()).to_radians());
}
