//! Placing the world camera every frame, from whichever kind of view applies
//! right now: killcam, remote missile, vehicle, death watch, or the player's
//! own eyes.

use asset_game::WeaponChassisSummary;
use assets::StagedWeapons;
use bevy::prelude::*;
use frame::{PlayerPrefsBundle, EyeSubject};
use hud_t6::fov::{AdsZoom, STOCK_VIEW_CONE_DEGREES, LINK_FLAG_FORCE_ADS_ZOOM, ViewFovInputs};
use hud_t6::fov::{vertical_fov_deg, view_fov, zoom_sensitivity};
use net::{ClientEntityPoseState, EntityTableCells, PlayerControlSnapshot, FrameClock, HostViewClient};
use net::{ViewAimAccumulator, ShownWorldFrame};
use playerstate_t6::game::{MOVE_MODE_INTERLUDE, MOVE_MODE_OBSERVER, get_viewmodel_weapon_index};
use render_scene::{FreeRoamCamera, FirstPersonLens, GameplayCamera, WorldEyePlacement};
use render_scene::{LevelScriptModelInstance, transform_from_game_view};

use super::first_person::{FrameView, first_person_pose};
use super::state::{QueuedHurtFlinch, CameraRecoilState};
use crate::occupancy::camera_shake::CameraShakes;
use crate::occupancy::shard_piece::PropPhysicsShape;
use crate::occupancy::death_replay::KillcamCamera;
use crate::occupancy::peer_body::OtherPlayerBody;
use crate::occupancy::script_prop::script_mover_pose_now;
use crate::occupancy::t6_body::LocalCorpseRoot;
use crate::occupancy::other_view::{VehicleSpring, victim_follow_view, shown_in_third_person};
use crate::occupancy::other_view::{remote_missile_camera, vehicle_camera, vehicle_view};

/// Horizontal field of view through the remote missile's camera.
const MISSILE_CAM_FOV: f32 = 15.0;

/// Fixed field of view of a view that is linked to the held gun.
const LINKED_WEAPON_VIEW_FOV: f32 = 45.0;

/// BO2: a view linked to the gun being held (`playerlinkweaponviewtodelta`,
/// link flag 4, as on the Lodestar's drone) is a flat 45 degrees. Once aiming
/// begins it switches straight to the gun's own ADS zoom (10 on the Lodestar),
/// with no blend and no fov scale. Gives None when no gun is linked.
pub(super) fn linked_weapon_view_fov(
    attach_bits: u32,
    weapon_idx: u32,
    weapon_raise_fraction: f32,
    aim_magnified_view: f32,
) -> Option<f32> {
    if attach_bits & LINK_FLAG_FORCE_ADS_ZOOM == 0 || weapon_idx == 0 {
        return None;
    }
    match weapon_raise_fraction > 0.0 && aim_magnified_view > 0.0 {
        true => Some(aim_magnified_view),
        false => Some(LINKED_WEAPON_VIEW_FOV),
    }
}

/// Scales how far the look controls turn to a field of view; zoomed in is slower.
fn scale_look_to_fov(input: Option<&mut PlayerControlSnapshot>, horiz_fov: f32) {
    let Some(input) = input else {
        return;
    };
    let base = zoom_sensitivity(horiz_fov, STOCK_VIEW_CONE_DEGREES);
    input.view_angle_multiplier = base * input.stun_aim_dampening;
}

/// Sets the first-person lens to the vertical fov matching a horizontal one.
fn set_lens_fov(lenses: &mut Query<&mut Projection, With<FirstPersonLens>>, horiz_fov: f32) {
    let vertical_in = vertical_fov_deg(horiz_fov).to_radians();
    lenses.iter_mut().for_each(|mut projection| {
        if let Projection::Perspective(lens) = &mut *projection {
            lens.fov = vertical_in;
        }
    });
}

/// Puts the eye camera at a pose.
fn place_eye(cameras: &mut Query<&mut Transform, (Without<LevelScriptModelInstance>, Without<OtherPlayerBody>, With<FreeRoamCamera>)>, pose: WorldEyePlacement) {
    let target = transform_from_game_view(pose);
    for mut current in cameras.iter_mut() {
        (current.translation, current.rotation) = (target.translation, target.rotation);
    }
}

/// Settles the field of view for this view and applies it to the lens.
///
/// Gives back the horizontal fov that was used. When the action input is
/// passed, the look controls are scaled to match.
pub(super) fn set_scope_lens_fov(
    lens_q: &mut Query<&mut Projection, With<FirstPersonLens>>,
    base_fov: f32,
    movement_mode: i32,
    attach_bits: u32,
    e_bits: u32,
    weapon_raise_fraction: f32,
    viewmodel_in: u32,
    facts: Option<WeaponChassisSummary>,
    snap_to_sights: bool,
    actions: Option<&mut PlayerControlSnapshot>,
) -> Option<f32> {
    let held = facts.filter(|f| f.payload_linked).unwrap_or_default();
    let scoped = held.scoped_aiming && held.aim_magnified_view > 0.0;
    let ads_fov = match scoped {
        true => held.aim_magnified_view,
        false => base_fov,
    };
    let zoom = AdsZoom {
        fov: [ads_fov; 3],
        zoom_in_frac: held.aim_in_share,
        zoom_out_frac: held.aim_out_share,
    };
    let lateral_drift = view_fov(&ViewFovInputs {
        zoom,
        view_fov: base_fov,
        default_view: movement_mode == MOVE_MODE_INTERLUDE,
        attach_bits,
        ent_bits: e_bits,
        has_weapon: viewmodel_in != 0,
        pos_frac: weapon_raise_fraction,
        to_ads: snap_to_sights,
        ..ViewFovInputs::default()
    });
    scale_look_to_fov(actions, lateral_drift);
    set_lens_fov(lens_q, lateral_drift);
    Some(lateral_drift)
}

/// The view with the server's earthquakes applied, then bo2mp's own Black Ops
/// II camera shakes.
pub(super) fn earthquake_pose(
    mut pose: WorldEyePlacement,
    presented: &ShownWorldFrame,
    clock_millis: i32,
    shakes: &mut CameraShakes,
) -> WorldEyePlacement {
    let quakes = presented
        .snapshot()
        .map(|snap| snap.meta.objectives.earthquakes.as_slice())
        .unwrap_or_default();
    for quake in quakes {
        let offsets = quake.angle_offset(pose.origin, clock_millis);
        pose.turn.iter_mut().zip(offsets).for_each(|(angle, delta)| *angle += delta);
    }
    shakes.apply(&mut pose.turn, clock_millis, pose.origin);
    pose
}

/// Keeps track of whether the weapon is moving into or out of ADS.
fn track_ads_direction(state: &mut CameraRecoilState, frac: f32) {
    let previous = core::mem::replace(&mut state.last_weapon_pos_frac, frac);
    if frac != previous {
        state.snap_to_sights = frac > previous;
    }
}

/// Places the world camera from the presented snapshot every frame.
pub fn sync_camera_from_shown(
    mut killcam_view: Local<KillcamCamera>,
    clock: Res<FrameClock>,
    presented_in: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    sim_cam: Res<GameplayCamera>,
    settings: Res<PlayerPrefsBundle>,
    mut session_kick: ResMut<CameraRecoilState>,
    mut hurt_queue: ResMut<QueuedHurtFlinch>,
    prepared_in: Option<Res<StagedWeapons>>,
    mut look_input: Option<ResMut<PlayerControlSnapshot>>,
    mut eye_cameras: Query<&mut Transform, (Without<LevelScriptModelInstance>, Without<OtherPlayerBody>, With<FreeRoamCamera>)>,
    mut lens_q: Query<&mut Projection, With<FirstPersonLens>>,
    view: Res<EyeSubject>,
    (clip, mut spring, slots, pose_states, look, mut old_corpse, mut shakes, carry): (
        Res<PropPhysicsShape>,
        Local<VehicleSpring>,
        Option<Res<EntityTableCells>>,
        Query<&ClientEntityPoseState>,
        Option<Res<ViewAimAccumulator>>,
        Local<Option<i32>>,
        ResMut<CameraShakes>,
        Res<crate::occupancy::platform_carry::PlatformCarry>,
    ),
    corpse_root: Option<Res<LocalCorpseRoot>>,
    script_models: Query<(&Transform, &LevelScriptModelInstance), Without<FreeRoamCamera>>,
) {
    session_kick.killcam_focus_distance = None;
    let Some(pstate) = presented_in.player(local.0).filter(|_| sim_cam.enabled) else {
        return;
    };
    let now = clock.time();
    let viewmodel = get_viewmodel_weapon_index(pstate);
    let gun_facts = |gun: u32| prepared_in.as_deref().and_then(|p| p.0.summary_for(gun));
    let shaken = |pose_in: WorldEyePlacement, shakes: &mut CameraShakes| {
        earthquake_pose(pose_in, &presented_in, now, shakes)
    };
    // The lens and the look controls for the player's own fov, which only
    // differs between views in the fraction, the direction and the gun.
    let fit_own_lens = |lens_q: &mut Query<&mut Projection, With<FirstPersonLens>>,
                        input: Option<&mut PlayerControlSnapshot>,
                        frac: f32,
                        to_ads: bool,
                        facts: Option<WeaponChassisSummary>| {
        set_scope_lens_fov(
            lens_q,
            settings.view_angle,
            pstate.movement_mode,
            pstate.attach_bits,
            pstate.e_flags,
            frac,
            viewmodel,
            facts,
            to_ads,
            input,
        )
    };

    // A killcam replays the killer's view.
    let replay = killcam_view.chase_frame(
        &presented_in,
        local.0,
        now,
        view.replaying_final_death(),
        prepared_in.as_deref(),
        clip.0.as_deref(),
    );
    if let Some((pose_in, fov, focus_distance)) = replay {
        let pose_in = shaken(pose_in, &mut shakes);
        place_eye(&mut eye_cameras, pose_in);
        set_lens_fov(&mut lens_q, fov);
        session_kick.horizontal_view_degrees = fov;
        (session_kick.frame_eye_position, session_kick.frame_look_angles) = (pose_in.origin, pose_in.turn);
        session_kick.killcam_focus_distance = focus_distance;
        session_kick.note_death_cam_source(now, "killcam", pstate.movement_mode);
        return;
    }

    // Looking through a remote missile.
    if let Some(pose_in) = remote_missile_camera(&presented_in, local.0, now) {
        let pose_in = shaken(pose_in, &mut shakes);
        place_eye(&mut eye_cameras, pose_in);
        scale_look_to_fov(look_input.as_deref_mut(), MISSILE_CAM_FOV);
        set_lens_fov(&mut lens_q, MISSILE_CAM_FOV);
        session_kick.horizontal_view_degrees = MISSILE_CAM_FOV;
        return;
    }

    // bo2mp: riding a scorestreak vehicle, the view follows its model.
    if let Some(vehicle) = vehicle_view(&presented_in, local.0)
        && let Some((drawn_at, owner)) = script_models
            .iter()
            .find(|(_, model)| model.id.origin_sequence_number() == vehicle.xmodel)
    {
        // Use the vehicle's place as drawn this frame. Its Transform is set
        // after the camera, so it still holds last frame's place.
        let live_pose = owner.game_actor_number.and_then(|number| {
            script_mover_pose_now(slots.as_deref(), &presented_in, &pose_states, &clock, number)
        });
        let at = match live_pose {
            Some((pos, angs)) => {
                let [pitch, yaw, roll] = angs.map(f32::to_radians);
                Transform {
                    translation: Vec3::from_array(pos),
                    rotation: Quat::from_rotation_z(yaw)
                        * Quat::from_rotation_y(pitch)
                        * Quat::from_rotation_x(roll),
                    ..*drawn_at
                }
            }
            None => *drawn_at,
        };
        let pushed = match vehicle.first_person {
            true => 0.0,
            false => spring.step(&vehicle, at.translation, at.rotation, clock.step_duration_seconds()),
        };
        let pose_in = vehicle_camera(
            &presented_in,
            local.0,
            vehicle,
            at.translation.to_array(),
            at.rotation,
            clip.0.as_deref(),
            pushed,
            look.as_deref().map(|l| l.angles),
        );
        let pose_in = shaken(pose_in, &mut shakes);
        place_eye(&mut eye_cameras, pose_in);
        // The vehicle has a field of view of its own (90 on the RC-XD). A view
        // linked to the held gun, as on the Lodestar, zooms along with it.
        // Otherwise the player's setting applies.
        let zoom_fov = gun_facts(pstate.weapon)
            .filter(|f| f.payload_linked)
            .map(|f| f.aim_magnified_view).unwrap_or(0.0);
        let linked = linked_weapon_view_fov(pstate.attach_bits, pstate.weapon, pstate.weapon_raise_fraction, zoom_fov);
        let own_fov = Some(vehicle.seat_fov).filter(|fov| *fov > 0.0);
        if let Some(fov) = own_fov.or(linked) {
            if linked.is_some() {
                scale_look_to_fov(look_input.as_deref_mut(), fov);
            }
            set_lens_fov(&mut lens_q, fov);
            session_kick.horizontal_view_degrees = fov;
        }
        return;
    }

    // The body left by this death. One that was already shown while the
    // player was alive belongs to an earlier death: the new body only shows a
    // frame or two after death, and the camera once sat on the old one far
    // across the map.
    let newest_body = corpse_root.as_deref().and_then(|r| r.0);
    let corpse = newest_body
        .filter(|(_k, since)| old_corpse.is_none_or(|old| *since > old))
        .map(|(root, _)| root);
    let watching_death = shown_in_third_person(&presented_in, local.0, view.replaying_final_death());
    if watching_death {
        let Some(pose_in) = victim_follow_view(&presented_in, local.0, clip.0.as_deref(), corpse) else {
            // Nothing to pull the camera back against, so it stays put.
            session_kick.note_death_cam_source(now, "frozen", pstate.movement_mode);
            return;
        };
        let source = if pstate.movement_mode >= 10 && corpse.is_some() { "corpse" } else { "eye" };
        session_kick.note_death_cam_source(now, source, pstate.movement_mode);
        let pose_in = shaken(pose_in, &mut shakes);
        place_eye(&mut eye_cameras, pose_in);
        fit_own_lens(&mut lens_q, look_input.as_deref_mut(), 0.0, false, gun_facts(viewmodel));
        return;
    }

    // The player's own eyes.
    if pstate.movement_mode < MOVE_MODE_OBSERVER {
        *old_corpse = newest_body.map(|(_, since)| since);
    }
    let source = if pstate.movement_mode == MOVE_MODE_OBSERVER { "spectator" } else { "alive" };
    session_kick.note_death_cam_source(now, source, pstate.movement_mode);

    let holding_breath = look_input.as_deref().is_some_and(|a| a.client.pressed_key_latch.holdbreath.active);
    let own_view = FrameView {
        ps: pstate,
        presented: &presented_in,
        clock: &clock,
        weapons: prepared_in.as_deref(),
        breath_key_down: holding_breath,
        carry: carry.0,
    };
    let pose_in = first_person_pose(&own_view, &mut session_kick, &mut hurt_queue);
    let pose_in = shaken(pose_in, &mut shakes);
    (session_kick.frame_eye_position, session_kick.frame_look_angles) = (pose_in.origin, pose_in.turn);
    crate::occupancy::platform_carry::camera_jitter(clock.time(), pose_in.origin, carry.0.is_some());
    place_eye(&mut eye_cameras, pose_in);

    track_ads_direction(&mut session_kick, pstate.weapon_raise_fraction);
    // bo2mp: with a Variable Zoom scope zoomed in, the second zoom applies.
    let second_zoom = session_kick.vzoom_in;
    let facts = gun_facts(viewmodel).map(|mut facts| {
        if second_zoom && facts.variable_zoom_fovs[1] > 0.0 {
            facts.aim_magnified_view = facts.variable_zoom_fovs[1];
        }
        facts
    });
    let to_ads = session_kick.snap_to_sights;
    let lateral_drift = fit_own_lens(&mut lens_q, look_input.as_deref_mut(), pstate.weapon_raise_fraction, to_ads, facts);
    session_kick.horizontal_view_degrees = lateral_drift.unwrap_or(session_kick.horizontal_view_degrees);
}
