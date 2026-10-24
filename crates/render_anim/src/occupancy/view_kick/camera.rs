//! Placing the world camera each frame, from whatever the view is made of right now:
//! a killcam, a remote missile, a vehicle, a death watch, or the player's own eyes.

use asset_game::WeaponBodyFacts;
use assets::PreparedWeapons;
use bevy::prelude::*;
use frame::{GameSettings, ViewSubject};
use hud_t6::fov::{
    AdsZoom, CG_FOV_DEFAULT, LINK_FLAG_FORCE_ADS_ZOOM, ViewFovInputs, vertical_fov_deg, view_fov, zoom_sensitivity,
};
use net::{CEntityRuntime, CEntitySlots, ClientActionInput, FrameClock, LocalPresentClient, LookState, PresentedSnapshot};
use playerstate_t6::game::{PM_TYPE_INTERMISSION, PM_TYPE_SPECTATOR, get_viewmodel_weapon_index};
use render_scene::{FlyCamera, FpvLens, SimCamera, WorldCameraPose, WorldScriptModelInstance, transform_from_iw_view};

use super::first_person::{FrameView, first_person_pose};
use super::state::{PendingViewHurt, SessionViewKick};
use crate::occupancy::camera_shake::CameraShakes;
use crate::occupancy::dyn_ent::DynEntPhysClip;
use crate::occupancy::killcam::KillcamCamera;
use crate::occupancy::remote_body::RemotePlayer;
use crate::occupancy::script_model::script_mover_pose_now;
use crate::occupancy::t6_body::LocalCorpseRoot;
use crate::occupancy::third_person::{
    VehicleSpring, death_watch_camera, presented_is_third_person, remote_missile_camera, vehicle_camera, vehicle_view,
};

/// Picks out the one camera that shows the player's own view.
pub(super) type EyeCameraFilter = (
    With<FlyCamera>,
    Without<RemotePlayer>,
    Without<WorldScriptModelInstance>,
);

/// The horizontal field of view the remote missile's camera looks through.
const MISSILE_CAM_FOV: f32 = 15.0;

/// BO2's fixed field of view for a view linked to the gun in his hands.
const LINKED_WEAPON_VIEW_FOV: f32 = 45.0;

/// BO2: a view linked to the gun in his hands (`playerlinkweaponviewtodelta`,
/// link flag 4; the Lodestar's drone view) is a fixed 45 degrees, and the
/// gun's own ADS zoom (the Lodestar's 10) as soon as he starts to aim: no
/// blend, no fov scale. None when the view is not linked to a gun.
pub(super) fn linked_weapon_view_fov(
    link_flags: u32,
    weapon: u32,
    f_weapon_pos_frac: f32,
    ads_zoom_fov: f32,
) -> Option<f32> {
    let linked = (link_flags & LINK_FLAG_FORCE_ADS_ZOOM) != 0 && weapon != 0;
    if !linked {
        return None;
    }
    let aiming_with_zoom = f_weapon_pos_frac > 0.0 && ads_zoom_fov > 0.0;
    Some(if aiming_with_zoom { ads_zoom_fov } else { LINKED_WEAPON_VIEW_FOV })
}

/// Sets how far the look controls turn for a field of view: zoomed in turns slower.
fn scale_look_to_fov(actions: Option<&mut ClientActionInput>, horiz_fov: f32) {
    if let Some(actions) = actions {
        actions.fov_scale = zoom_sensitivity(horiz_fov, CG_FOV_DEFAULT) * actions.shellshock_look_scale;
    }
}

/// Gives the first-person lens the vertical field of view that matches a horizontal one.
fn set_lens_fov(lenses: &mut Query<&mut Projection, With<FpvLens>>, horiz_fov: f32) {
    let vertical = vertical_fov_deg(horiz_fov).to_radians();
    for mut projection in lenses.iter_mut() {
        if let Projection::Perspective(lens) = &mut *projection {
            lens.fov = vertical;
        }
    }
}

/// Moves the eye camera to a pose.
fn place_eye(cameras: &mut Query<&mut Transform, EyeCameraFilter>, pose: WorldCameraPose) {
    let eye = transform_from_iw_view(pose);
    for mut at in cameras.iter_mut() {
        at.translation = eye.translation;
        at.rotation = eye.rotation;
    }
}

/// Works out the field of view for the view and sets it on the lens.
///
/// Returns the horizontal field of view used. The look controls are scaled to it
/// when the caller passes the action input.
pub(super) fn apply_fpv_lens_fov(
    lenses: &mut Query<&mut Projection, With<FpvLens>>,
    base_fov: f32,
    pm_type: i32,
    link_flags: u32,
    e_flags: u32,
    f_weapon_pos_frac: f32,
    viewmodel: u32,
    facts: Option<WeaponBodyFacts>,
    b_position_to_ads: bool,
    actions: Option<&mut ClientActionInput>,
) -> Option<f32> {
    let gun = facts.filter(|f| f.body_resolved).unwrap_or_default();
    let zoomed = gun.aim_down_sight && gun.ads_zoom_fov > 0.0;
    let ads_fov = if zoomed { gun.ads_zoom_fov } else { base_fov };
    let horiz = view_fov(&ViewFovInputs {
        cg_fov: base_fov,
        default_view: pm_type == PM_TYPE_INTERMISSION,
        link_flags,
        e_flags,
        has_weapon: viewmodel != 0,
        zoom: AdsZoom {
            fov: [ads_fov; 3],
            zoom_in_frac: gun.ads_zoom_in_frac,
            zoom_out_frac: gun.ads_zoom_out_frac,
        },
        pos_frac: f_weapon_pos_frac,
        to_ads: b_position_to_ads,
        ..ViewFovInputs::default()
    });
    scale_look_to_fov(actions, horiz);
    set_lens_fov(lenses, horiz);
    Some(horiz)
}

/// The view shaken by the server's earthquakes, then (bo2mp) by Black Ops
/// II's camera shakes.
pub(super) fn earthquake_pose(
    mut pose: WorldCameraPose,
    presented: &PresentedSnapshot,
    now_ms: i32,
    shakes: &mut CameraShakes,
) -> WorldCameraPose {
    if let Some(snapshot) = presented.snapshot() {
        let eye = pose.origin;
        for quake in &snapshot.meta.objectives.earthquakes {
            let shake = quake.angle_offset(eye, now_ms);
            for (angle, delta) in pose.angles.iter_mut().zip(shake) {
                *angle += delta;
            }
        }
    }
    shakes.apply(&mut pose.angles, now_ms, pose.origin);
    pose
}

/// Places the world camera from the presented snapshot every frame.
pub fn sync_camera_from_presented(
    mut killcam: Local<KillcamCamera>,
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    sim_cam: Res<SimCamera>,
    settings: Res<GameSettings>,
    mut kick: ResMut<SessionViewKick>,
    mut hurt: ResMut<PendingViewHurt>,
    weapons: Option<Res<PreparedWeapons>>,
    mut actions: Option<ResMut<ClientActionInput>>,
    mut eye_cameras: Query<&mut Transform, EyeCameraFilter>,
    mut lenses: Query<&mut Projection, With<FpvLens>>,
    view: Res<ViewSubject>,
    (clip, mut spring, slots, runtimes, look, mut old_corpse, mut shakes): (
        Res<DynEntPhysClip>,
        Local<VehicleSpring>,
        Option<Res<CEntitySlots>>,
        Query<&CEntityRuntime>,
        Option<Res<LookState>>,
        Local<Option<i32>>,
        ResMut<CameraShakes>,
    ),
    corpse_root: Option<Res<LocalCorpseRoot>>,
    script_models: Query<(&WorldScriptModelInstance, &Transform), Without<FlyCamera>>,
) {
    kick.killcam_focus_distance = None;
    if !sim_cam.enabled {
        return;
    }
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    let now = clock.time();
    let viewmodel = get_viewmodel_weapon_index(ps);
    let held_facts = |gun: u32| weapons.as_deref().and_then(|w| w.0.facts_of(gun));

    // A killcam replays the killer's view.
    if let Some((pose, fov, focus_distance)) = killcam.update(
        &presented,
        local.0,
        now,
        view.in_killcam(),
        weapons.as_deref(),
        clip.0.as_deref(),
    ) {
        let pose = earthquake_pose(pose, &presented, now, &mut shakes);
        place_eye(&mut eye_cameras, pose);
        set_lens_fov(&mut lenses, fov);
        kick.horiz_fov_deg = fov;
        kick.refdef_vieworg = pose.origin;
        kick.refdef_view_angles = pose.angles;
        kick.killcam_focus_distance = focus_distance;
        kick.note_death_cam_source(now, "killcam", ps.pm_type);
        return;
    }

    // Looking through a remote missile.
    if let Some(pose) = remote_missile_camera(&presented, local.0, now) {
        let pose = earthquake_pose(pose, &presented, now, &mut shakes);
        place_eye(&mut eye_cameras, pose);
        scale_look_to_fov(actions.as_deref_mut(), MISSILE_CAM_FOV);
        set_lens_fov(&mut lenses, MISSILE_CAM_FOV);
        kick.horiz_fov_deg = MISSILE_CAM_FOV;
        return;
    }

    // bo2mp: riding a scorestreak vehicle - his view rides its model.
    if let Some(vehicle) = vehicle_view(&presented, local.0)
        && let Some((owner, drawn_at)) = script_models
            .iter()
            .find(|(model, _)| model.id.source_ordinal() == vehicle.model)
    {
        // The vehicle where it is drawn this frame (its Transform is placed
        // after the camera, so it holds last frame's place).
        let mut at = *drawn_at;
        let live = owner.gentity_number.and_then(|number| {
            script_mover_pose_now(slots.as_deref(), &presented, &runtimes, &clock, number)
        });
        if let Some((origin, [pitch, yaw, roll])) = live {
            at.translation = Vec3::from_array(origin);
            at.rotation = Quat::from_euler(EulerRot::ZYX, yaw.to_radians(), pitch.to_radians(), roll.to_radians());
        }
        let pushed = if vehicle.first_person {
            0.0
        } else {
            spring.step(&vehicle, at.translation, at.rotation, clock.frametime_secs())
        };
        let pose = vehicle_camera(
            &presented,
            local.0,
            vehicle,
            at.translation.to_array(),
            at.rotation,
            clip.0.as_deref(),
            pushed,
            look.as_deref().map(|l| l.angles),
        );
        let pose = earthquake_pose(pose, &presented, now, &mut shakes);
        place_eye(&mut eye_cameras, pose);
        // Its own field of view (the RC-XD's 90); a view linked to the gun in
        // his hands (the Lodestar's) zooms with it; else his.
        let zoom_fov = held_facts(ps.weapon)
            .filter(|f| f.body_resolved)
            .map_or(0.0, |f| f.ads_zoom_fov);
        let linked = linked_weapon_view_fov(ps.link_flags, ps.weapon, ps.f_weapon_pos_frac, zoom_fov);
        let fov = if vehicle.fov > 0.0 { Some(vehicle.fov) } else { linked };
        if let Some(fov) = fov {
            if linked.is_some() {
                scale_look_to_fov(actions.as_deref_mut(), fov);
            }
            set_lens_fov(&mut lenses, fov);
            kick.horiz_fov_deg = fov;
        }
        return;
    }

    // His body from this death: one that showed while he was still alive
    // is an earlier death's (the new body shows a frame or two after he
    // dies, and the camera sat on the old one across the map: row 33).
    let corpse = corpse_root
        .as_deref()
        .and_then(|r| r.0)
        .filter(|(_, since)| old_corpse.is_none_or(|old| *since > old))
        .map(|(root, _)| root);
    if presented_is_third_person(&presented, local.0, view.in_killcam()) {
        let Some(pose) = death_watch_camera(&presented, local.0, clip.0.as_deref(), corpse) else {
            // No collision to pull the camera back against: it holds.
            kick.note_death_cam_source(now, "frozen", ps.pm_type);
            return;
        };
        let on_body = ps.pm_type >= 10 && corpse.is_some();
        kick.note_death_cam_source(now, if on_body { "corpse" } else { "eye" }, ps.pm_type);
        let pose = earthquake_pose(pose, &presented, now, &mut shakes);
        place_eye(&mut eye_cameras, pose);
        apply_fpv_lens_fov(
            &mut lenses,
            settings.fov,
            ps.pm_type,
            ps.link_flags,
            ps.e_flags,
            0.0,
            viewmodel,
            held_facts(viewmodel),
            false,
            actions.as_deref_mut(),
        );
        return;
    }

    // His own eyes.
    if ps.pm_type < PM_TYPE_SPECTATOR {
        *old_corpse = corpse_root.as_deref().and_then(|r| r.0).map(|(_, since)| since);
    }
    let source = if ps.pm_type == PM_TYPE_SPECTATOR { "spectator" } else { "alive" };
    kick.note_death_cam_source(now, source, ps.pm_type);

    let frame = FrameView {
        ps,
        presented: &presented,
        clock: &clock,
        weapons: weapons.as_deref(),
        breath_key_down: actions.as_deref().is_some_and(|a| a.client.kb.holdbreath.active),
    };
    let pose = first_person_pose(&frame, &mut kick, &mut hurt);
    let pose = earthquake_pose(pose, &presented, now, &mut shakes);
    kick.refdef_vieworg = pose.origin;
    kick.refdef_view_angles = pose.angles;
    place_eye(&mut eye_cameras, pose);

    let frac = ps.f_weapon_pos_frac;
    if frac > kick.last_weapon_pos_frac {
        kick.b_position_to_ads = true;
    } else if frac < kick.last_weapon_pos_frac {
        kick.b_position_to_ads = false;
    }
    kick.last_weapon_pos_frac = frac;
    // bo2mp: a Variable Zoom scope zoomed in takes its second zoom.
    let zoomed_facts = held_facts(viewmodel).map(|mut facts| {
        if kick.vzoom_in && facts.variable_zoom_fovs[1] > 0.0 {
            facts.ads_zoom_fov = facts.variable_zoom_fovs[1];
        }
        facts
    });
    if let Some(horiz) = apply_fpv_lens_fov(
        &mut lenses,
        settings.fov,
        ps.pm_type,
        ps.link_flags,
        ps.e_flags,
        frac,
        viewmodel,
        zoomed_facts,
        kick.b_position_to_ads,
        actions.as_deref_mut(),
    ) {
        kick.horiz_fov_deg = horiz;
    }
}
