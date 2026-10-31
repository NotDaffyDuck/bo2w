#![cfg(test)]
//! Pinning tests for the view-kick helpers: what each small pure piece must keep doing
//! while the module is reorganised.

use super::breath::diff_track;
use super::camera::{set_scope_lens_fov, earthquake_pose};
use super::feedback::{stamp_and_land_origin_z, stamp_damage_feedback};
use super::params::kick_params;
use super::{
    GunOffset, HoldBreath, QueuedHurtFlinch, CameraRecoilState, sum3, bo2_camera,
    placement_to_camera_local,
};
use asset_game::{WeaponChassisSummary, WeaponRecoilSummary};
use bevy::prelude::*;
use hud_t6::fov::{AdsZoom, STOCK_VIEW_CONE_DEGREES, ViewFovInputs, vertical_fov_deg, view_fov, zoom_sensitivity};
use net::{PlayerControlSnapshot, ShownWorldFrame};
use playerstate_t6::game::NO_ENTITY_SLOT;
use render_scene::{FirstPersonLens, WorldEyePlacement};
use weapon_t6::view_bob::HIT_FLINCH_NO_DIRECTION;
use crate::occupancy::camera_shake::CameraShakes;
use bevy::ecs::system::RunSystemOnce;
use weapon_t6::view_bob::{
    hard_landing_drop, crash_land_view_dip, damage_feedback_kick, damage_feedback_kick_bo2,
    land_origin_z,
};

#[test]
fn every_kick_fact_reaches_the_matching_kick_param() {
    macro_rules! carried {
        ($($field:ident),* $(,)?) => {{
            let mut n = 0.0f32;
            let facts = WeaponRecoilSummary {
                $($field: { n += 1.0; n },)*
                ..WeaponRecoilSummary::default()
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
        scoped_view_kick_recenter_speed,
        hipfire_view_kick_recenter_speed,
        weapon_pitch_ceiling,
        weapon_yaw_ceiling,
        scoped_weapon_kick_damped_percent,
        scoped_weapon_kick_pitch_bottom,
        scoped_weapon_kick_pitch_top,
        scoped_weapon_kick_yaw_bottom,
        scoped_weapon_kick_yaw_top,
        scoped_weapon_kick_accel,
        scoped_weapon_kick_velocity_cap,
        scoped_weapon_kick_velocity_falloff,
        scoped_weapon_kick_resting_falloff,
        scoped_camera_kick_pitch_bottom,
        scoped_camera_kick_pitch_top,
        scoped_camera_kick_yaw_bottom,
        scoped_camera_kick_yaw_top,
        hipfire_weapon_kick_damped_percent,
        hipfire_weapon_kick_pitch_bottom,
        hipfire_weapon_kick_pitch_top,
        hipfire_weapon_kick_yaw_bottom,
        hipfire_weapon_kick_yaw_top,
        hipfire_weapon_kick_accel,
        hipfire_weapon_kick_velocity_cap,
        hipfire_weapon_kick_velocity_falloff,
        hipfire_weapon_kick_resting_falloff,
        hipfire_camera_kick_pitch_bottom,
        hipfire_camera_kick_pitch_top,
        hipfire_camera_kick_yaw_bottom,
        hipfire_camera_kick_yaw_top,
        hip_view_kick_min_magnitude,
        ads_view_kick_min_magnitude,
    );
    assert_eq!(count, 32.0);
}

#[test]
fn bo2_camera_follows_the_aim_kick_flag() {
    let mut kick = CameraRecoilState::default();
    assert!(!bo2_camera(&kick));
    kick.aim_kick = true;
    assert!(bo2_camera(&kick));
}

#[test]
fn offsets_add_up_per_axis() {
    assert_eq!(
        sum3([1.0, 2.0, 3.0], [0.5, -2.0, 10.0]),
        [1.5, 0.0, 13.0]
    );
    assert_eq!(
        sum3([1.0, 2.0, 3.0], [-1.0, 0.25, 4.0]),
        [0.0, 2.25, 7.0]
    );
    let gun = GunOffset { x: 1.0, y: 2.0, z: 3.0 };
    assert_eq!(gun.as_xyz(), [1.0, 2.0, 3.0]);
}

#[test]
fn a_view_placement_maps_forward_right_up_onto_the_camera_axes() {
    let t = placement_to_camera_local([3.0, 5.0, 7.0], [0.0; 3]);
    assert_eq!(t.translation, Vec3::new(5.0, 7.0, -3.0));
    assert_eq!(t.scale, Vec3::ONE);
    let turned = placement_to_camera_local([0.0; 3], [10.0, 20.0, 30.0]);
    let expected = crate::motion::gun_pose::placement_angles_to_bevy_camera_quat([10.0, 20.0, 30.0]);
    assert_eq!(turned.rotation, expected);
}

#[test]
fn a_landing_after_a_fall_starts_a_dip_that_a_flat_step_does_not() {
    let mut kick = CameraRecoilState::default();
    // First frame in the air: only remembered.
    let first = stamp_and_land_origin_z(
        &mut kick,
        800,
        [0.0, 0.0, 200.0],
        [0.0, 0.0, -900.0],
        NO_ENTITY_SLOT,
        1000,
    );
    assert_eq!(first, land_origin_z(1000.0, 0.0));
    assert!(kick.land_seen);
    assert_eq!(kick.prev_origin, [0.0, 0.0, 200.0]);
    assert_eq!(kick.prev_velocity, [0.0, 0.0, -900.0]);
    assert_eq!(kick.prev_standing_on, NO_ENTITY_SLOT);
    assert_eq!(kick.touchdown_stamp, 0);
    // Touching ground after the fall.
    let landed_at = [0.0, 0.0, 0.0];
    let fall = hard_landing_drop(800, 200.0, 0.0, -900.0);
    let dip = fall.map_or(0, |f| crash_land_view_dip(f, false));
    let z = stamp_and_land_origin_z(&mut kick, 800, landed_at, [0.0; 3], 5, 1100);
    if dip > 0 {
        assert_eq!(kick.touchdown_dip, -(dip as f32));
        assert_eq!(kick.touchdown_stamp, 1100);
        assert_eq!(kick.landing_dip, dip);
    } else {
        assert_eq!(kick.touchdown_stamp, 0);
    }
    assert_eq!(z, land_origin_z((1100 - kick.touchdown_stamp) as f32, kick.touchdown_dip));
    assert_eq!(kick.prev_standing_on, 5);
    // Still on the ground: nothing new starts.
    let before = (kick.touchdown_dip, kick.touchdown_stamp, kick.landing_dip);
    stamp_and_land_origin_z(&mut kick, 800, landed_at, [0.0; 3], 5, 1200);
    assert_eq!((kick.touchdown_dip, kick.touchdown_stamp, kick.landing_dip), before);
}

#[test]
fn the_landing_dip_is_the_hard_one_for_a_black_ops_ii_camera() {
    // A big fall: both cameras dip; the dip each gets is whatever the dip rule says for it.
    for bo2 in [false, true] {
        let mut kick = CameraRecoilState::default();
        kick.aim_kick = bo2;
        stamp_and_land_origin_z(&mut kick, 800, [0.0, 0.0, 400.0], [0.0, 0.0, -1000.0], NO_ENTITY_SLOT, 10);
        stamp_and_land_origin_z(&mut kick, 800, [0.0; 3], [0.0; 3], 3, 20);
        let want = hard_landing_drop(800, 400.0, 0.0, -1000.0).map_or(0, |f| crash_land_view_dip(f, bo2));
        assert_eq!(kick.landing_dip, want, "bo2 {bo2}");
        assert_eq!(kick.touchdown_stamp, if want > 0 { 20 } else { 0 });
    }
}

#[test]
fn a_hit_stamps_the_flinch_once_per_new_damage_event() {
    let angles = [10.0, 20.0, 0.0];
    for bo2 in [false, true] {
        let mut kick = CameraRecoilState::default();
        kick.aim_kick = bo2;
        let mut hurt = QueuedHurtFlinch(0);
        // The first frame only remembers the event number.
        stamp_damage_feedback(&mut kick, &mut hurt, 3, 64, 1, 30, angles, [0; 2], 500);
        assert_eq!(kick.hurt_stamp_ms, 0);
        assert!(kick.damage_seen);
        assert_eq!(kick.prev_damage_event, 3);
        // The same event again: nothing.
        stamp_damage_feedback(&mut kick, &mut hurt, 3, 64, 1, 30, angles, [0; 2], 600);
        assert_eq!(kick.hurt_stamp_ms, 0);
        // A new event: stamped at the time.
        stamp_damage_feedback(&mut kick, &mut hurt, 4, 64, 1, 30, angles, [0; 2], 700);
        assert_eq!(kick.hurt_stamp_ms, 700);
        let want = if bo2 {
            damage_feedback_kick_bo2(64, 1, 30, angles, false).unwrap()
        } else {
            damage_feedback_kick(64, 1, 30, angles)
        };
        assert_eq!((kick.hurt_kick_pitch, kick.hurt_kick_roll), (want.hurt_kick_pitch, want.hurt_kick_roll));
        assert_eq!(kick.prev_damage_event, 4);
        // A new event with no damage count does not stamp.
        stamp_damage_feedback(&mut kick, &mut hurt, 5, 64, 1, 0, angles, [0; 2], 800);
        assert_eq!(kick.hurt_stamp_ms, 700);
    }
}

#[test]
fn a_pending_hurt_flinches_without_a_direction_and_is_used_up() {
    let angles = [0.0; 3];
    let mut kick = CameraRecoilState::default();
    let mut hurt = QueuedHurtFlinch(2);
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, [0; 2], 0);
    assert_eq!(hurt.0, 1);
    assert_eq!(kick.hurt_stamp_ms, 1, "time 0 is stamped as 1");
    let want = damage_feedback_kick(HIT_FLINCH_NO_DIRECTION, HIT_FLINCH_NO_DIRECTION, 1, angles);
    assert_eq!((kick.hurt_kick_pitch, kick.hurt_kick_roll), (want.hurt_kick_pitch, want.hurt_kick_roll));
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, [0; 2], 900);
    assert_eq!(hurt.0, 0);
    assert_eq!(kick.hurt_stamp_ms, 900);
    stamp_damage_feedback(&mut kick, &mut hurt, 0, 0, 0, 0, angles, [0; 2], 1000);
    assert_eq!(kick.hurt_stamp_ms, 900);
}

#[test]
fn a_directed_hit_wins_over_a_pending_hurt() {
    let mut kick = CameraRecoilState::default();
    let mut hurt = QueuedHurtFlinch(0);
    stamp_damage_feedback(&mut kick, &mut hurt, 1, 0, 0, 10, [0.0; 3], [0; 2], 100);
    hurt.0 = 1;
    stamp_damage_feedback(&mut kick, &mut hurt, 2, 0, 0, 10, [0.0; 3], [0; 2], 200);
    assert_eq!(hurt.0, 1, "the pending hurt waits while a real hit is stamped");
    assert_eq!(kick.hurt_stamp_ms, 200);
}

#[test]
fn the_bo2_flinch_can_keep_the_last_angles() {
    // Toughness (bullet flinch perk) at a small hit: the BO2 rule may give no punch at all;
    // the angles then stay and only the time moves.
    let perk = [1u32 << weapon_t6::env::perk::REDUCEBULLETFLINCH, 0];
    let mut kick = CameraRecoilState::default();
    kick.aim_kick = true;
    kick.hurt_kick_pitch = 1.25;
    kick.hurt_kick_roll = -0.5;
    let mut hurt = QueuedHurtFlinch(0);
    stamp_damage_feedback(&mut kick, &mut hurt, 1, 0, 0, 1, [0.0; 3], perk, 10);
    stamp_damage_feedback(&mut kick, &mut hurt, 2, 0, 0, 1, [0.0; 3], perk, 20);
    assert_eq!(kick.hurt_stamp_ms, 20);
    match damage_feedback_kick_bo2(0, 0, 1, [0.0; 3], true) {
        Some(p) => assert_eq!((kick.hurt_kick_pitch, kick.hurt_kick_roll), (p.hurt_kick_pitch, p.hurt_kick_roll)),
        None => assert_eq!((kick.hurt_kick_pitch, kick.hurt_kick_roll), (1.25, -0.5)),
    }
}

#[test]
fn clearing_for_a_new_life_empties_every_remembered_value() {
    let mut kick = CameraRecoilState::default();
    kick.placement_shift = [1.0; 3];
    kick.placement_turn = [1.0; 3];
    kick.gun_idle_clock = 5;
    kick.previous_idle_ratio = 1.0;
    kick.view_prev_sway_ratio = 1.0;
    kick.touchdown_dip = -3.0;
    kick.touchdown_stamp = 9;
    kick.landing_dip = 3;
    kick.land_z_viewweapon = 2.0;
    kick.gun_landing_dip = [1.0; 3];
    kick.hurt_stamp_ms = 7;
    kick.hurt_kick_pitch = 1.0;
    kick.hurt_kick_roll = 1.0;
    kick.prev_origin = [1.0; 3];
    kick.prev_velocity = [1.0; 3];
    kick.prev_standing_on = 4;
    kick.land_seen = true;
    kick.prev_damage_event = 8;
    kick.damage_seen = true;
    kick.freshly_primed_count = 2;
    kick.last_weapon_pos_frac = 0.5;
    kick.snap_to_sights = false;
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
    kick.horizontal_view_degrees = 65.0;
    kick.aim_kick = true;
    kick.last_weapon_id = 12;
    kick.reset_on_respawn();
    let mut want = CameraRecoilState::default();
    want.snap_to_sights = true;
    assert_eq!(kick.placement_shift, [0.0; 3]);
    assert_eq!(kick.placement_turn, [0.0; 3]);
    assert_eq!(kick.gun_idle_clock, 0);
    assert_eq!((kick.previous_idle_ratio, kick.view_prev_sway_ratio), (0.0, 0.0));
    assert_eq!((kick.touchdown_dip, kick.touchdown_stamp, kick.landing_dip), (0.0, 0, 0));
    assert_eq!(kick.land_z_viewweapon, 0.0);
    assert_eq!(kick.gun_landing_dip, [0.0; 3]);
    assert_eq!((kick.hurt_stamp_ms, kick.hurt_kick_pitch, kick.hurt_kick_roll), (0, 0.0, 0.0));
    assert_eq!(kick.prev_origin, [0.0; 3]);
    assert_eq!(kick.prev_velocity, [0.0; 3]);
    assert_eq!(kick.prev_standing_on, 0);
    assert!(!kick.land_seen && !kick.damage_seen);
    assert_eq!(kick.prev_damage_event, 0);
    assert_eq!(kick.freshly_primed_count, 0);
    assert_eq!(kick.last_weapon_pos_frac, 0.0);
    assert!(kick.snap_to_sights);
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
    assert_eq!(kick.horizontal_view_degrees, 65.0);
    assert!(kick.aim_kick);
    assert_eq!(kick.last_weapon_id, 12);
}

#[test]
fn the_gun_turn_the_aim_holds_is_queued_as_a_change() {
    let mut kick = CameraRecoilState::default();
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
    let mut kick = CameraRecoilState::default();
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
    let shown = ShownWorldFrame::default();
    let pose_in = WorldEyePlacement { origin: [10.0, 20.0, 30.0], turn: [1.0, 2.0, 3.0] };
    let mut calm = CameraShakes::default();
    let same = earthquake_pose(pose_in, &shown, 1000, &mut calm);
    assert_eq!(same.origin, pose_in.origin);
    assert_eq!(same.turn, pose_in.turn);
    // With a shake running the pose matches the shake applied by hand.
    let mut a = CameraShakes::default();
    a.start(2.0, 5000, [0.0; 3], 0.0, 1000, pose_in.turn);
    let mut b = a.clone();
    let shaken = earthquake_pose(pose_in, &shown, 1200, &mut a);
    let mut by_hand = pose_in.turn;
    b.apply(&mut by_hand, 1200, pose_in.origin);
    assert_eq!(shaken.turn, by_hand);
    assert_eq!(shaken.origin, pose_in.origin);
    assert_ne!(shaken.turn, pose_in.turn);
}

fn lens_world() -> World {
    let mut world = World::new();
    world.spawn((FirstPersonLens, Projection::Perspective(PerspectiveProjection::default())));
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
    for (movement_mode, frac, to_ads) in [(0, 0.0, true), (0, 1.0, true), (0, 0.5, false), (6, 0.0, true)] {
        let mut world = lens_world();
        let mut actions = PlayerControlSnapshot::default();
        actions.stun_aim_dampening = 0.5;
        let facts = WeaponChassisSummary {
            payload_linked: true,
            scoped_aiming: true,
            aim_magnified_view: 40.0,
            aim_in_share: 0.3,
            aim_out_share: 0.7,
            ..WeaponChassisSummary::default()
        };
        let base = 80.0;
        let got = world
            .run_system_once(move |mut lenses: Query<&mut Projection, With<FirstPersonLens>>| {
                let mut actions = actions.clone();
                let h = set_scope_lens_fov(&mut lenses, base, movement_mode, 0, 0, frac, 7, Some(facts), to_ads, Some(&mut actions));
                (h, actions.view_angle_multiplier)
            })
            .unwrap();
        let ads_target = 40.0;
        let want_in = view_fov(&ViewFovInputs {
            view_fov: base,
            default_view: movement_mode == playerstate_t6::game::MOVE_MODE_INTERLUDE,
            attach_bits: 0,
            ent_bits: 0,
            has_weapon: true,
            zoom: AdsZoom { fov: [ads_target; 3], zoom_in_frac: 0.3, zoom_out_frac: 0.7 },
            pos_frac: frac,
            to_ads,
            ..ViewFovInputs::default()
        });
        assert_eq!(got.0, Some(want_in), "pm {movement_mode} frac {frac}");
        assert_eq!(got.1, zoom_sensitivity(want_in, STOCK_VIEW_CONE_DEGREES) * 0.5);
        assert_eq!(lens_fov(&mut world), vertical_fov_deg(want_in).to_radians());
    }
}

#[test]
fn a_gun_without_resolved_facts_does_not_zoom_and_no_weapon_is_a_default_view() {
    let mut world = lens_world();
    let unresolved = WeaponChassisSummary { payload_linked: false, scoped_aiming: true, aim_magnified_view: 20.0, ..WeaponChassisSummary::default() };
    let got = world
        .run_system_once(move |mut lenses: Query<&mut Projection, With<FirstPersonLens>>| {
            set_scope_lens_fov(&mut lenses, 75.0, 0, 0, 0, 1.0, 7, Some(unresolved), true, None)
        })
        .unwrap();
    let plain = world
        .run_system_once(move |mut lenses: Query<&mut Projection, With<FirstPersonLens>>| {
            set_scope_lens_fov(&mut lenses, 75.0, 0, 0, 0, 1.0, 7, None, true, None)
        })
        .unwrap();
    assert_eq!(got, plain);
    assert_eq!(lens_fov(&mut world), vertical_fov_deg(plain.unwrap()).to_radians());
}
