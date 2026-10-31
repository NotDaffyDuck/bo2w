//! Where the player's own eyes are, and which way they look, this frame.
//!
//! The pose is the body's eye point plus the walk bob and the landing dip, turned by
//! the gun's kick, the hit flinch and the view bob. For a Black Ops II gun, part of
//! that turn is handed to the aim instead (queued for the next command) and so is
//! taken out of what the camera adds.

use assets::StagedWeapons;
use asset_game::WeaponChassisSummary;
use game_math::{lean_offset, euler_basis, vec3_norm};
use net::{FrameClock, ShownWorldFrame};
use playerstate_t6::game::{AvatarState, get_viewmodel_weapon_index};
use render_scene::WorldEyePlacement;
use weapon_t6::view_bob::{
    CAMERA_SWAY_LOWEST_DROP, CameraTiltSway, CameraTiltSwayFeed, CameraHeightSwayFeed, view_angle_bob,
    view_damage_angles, view_org_bob, view_walk_bob_angles, viewweapon_land_origin_z,
};

use super::feedback::{hit_aim_pitch, stamp_and_land_origin_z, stamp_damage_feedback};
use super::state::{QueuedHurtFlinch, CameraRecoilState, bo2_camera, retarget};
use crate::motion::kick_state::kicked_view_angles;
use crate::occupancy::t6_movement::ViewBobInput;

/// What the first-person pose reads from the frame.
pub(super) struct FrameView<'a> {
    pub ps: &'a AvatarState,
    pub presented: &'a ShownWorldFrame,
    pub clock: &'a FrameClock,
    pub weapons: Option<&'a StagedWeapons>,
    /// The hold-breath button is down.
    pub breath_key_down: bool,
    /// bo2zm: the bus he rides, if any.
    pub carry: Option<crate::occupancy::platform_carry::PlatformCarryPose>,
}

/// Steps the scope state for a scoped gun and returns the drift scale that hold breath gives.
///
/// Also records whether the gun is a Variable Zoom scope in use and whether a Hybrid Optic is ready, for
/// the input to read, and opens a Variable Zoom scope wide whenever he is not aiming.
fn scope_breath_scale(
    kick: &mut CameraRecoilState,
    facts: &WeaponChassisSummary,
    frac: f32,
    key_down: bool,
    frame_ms: i32,
) -> f32 {
    let scoped = facts.hud_crosshair_layer != 0;
    kick.vzoom_scoped = facts.variable_zoom_fovs[0] > 0.0 && scoped && frac >= 1.0;
    kick.optic_ready = facts.optic_switch && frac >= 1.0;
    if frac <= 0.0 {
        kick.vzoom_in = false;
    }
    if kick.freshly_primed_count > 0 {
        kick.hold_breath.fired(frac, scoped);
    }
    kick.hold_breath.step_on(
        facts.hold_breath_to_steady && scoped,
        frac,
        key_down,
        frame_ms as f32,
    )
}

/// The view angle bob for this frame: the full bob with a resolved gun, only the
/// hit flinch without one.
fn bob_for_frame(
    inputs: Option<CameraTiltSwayFeed>,
    org: CameraHeightSwayFeed,
    now: i32,
    kick: &CameraRecoilState,
) -> CameraTiltSway {
    match inputs {
        Some(full) => view_angle_bob(full),
        None => view_damage_angles(CameraTiltSwayFeed {
            hurt_kick_roll: kick.hurt_kick_roll,
            hurt_kick_pitch: kick.hurt_kick_pitch,
            hurt_stamp_ms: kick.hurt_stamp_ms,
            time: now,
            org,
            ..CameraTiltSwayFeed::default()
        }),
    }
}

/// The first-person eye pose for this frame, before any camera shake. Updates the
/// flinch, the landing and the aim carry-overs on the way.
pub(super) fn first_person_pose(
    frame: &FrameView,
    kick: &mut CameraRecoilState,
    hurt: &mut QueuedHurtFlinch,
) -> WorldEyePlacement {
    let FrameView { ps: pstate, presented, clock, weapons, breath_key_down, carry } = *frame;
    let now = clock.time();
    let frac = pstate.weapon_raise_fraction;
    let viewmodel = get_viewmodel_weapon_index(pstate);
    let gun_facts = weapons.and_then(|w| w.0.summary_for(viewmodel));
    let resolved = gun_facts.as_ref().filter(|f| f.payload_linked);

    // bo2zm: riding the bus, his view keeps his place in the bus as drawn (the server's 20 Hz
    // carry steps are not a prediction miss to ease).
    let (base, eye_shift, yaw_lead) = match carry {
        Some(pose) => (pose.carry_point(pstate.origin), [0.0; 3], pose.yaw_lead()),
        None => (pstate.origin, presented.eye_displacement(), 0.0),
    };
    let ground_speed = vec3_norm([pstate.velocity[0], pstate.velocity[1], 0.0]);
    let org_inputs = CameraHeightSwayFeed {
        sway_phase: (pstate.sway_phase as u32 & 0xff) as u8,
        ground_velocity: ground_speed,
        eye_height_goal: pstate.eye_height_goal,
        movement_state_bits: pstate.movement_state_bits,
        gun_raise_share: frac,
        perks: pstate.perks,
        bo2: bo2_camera(kick),
        waterlevel: pstate.waterlevel,
    };
    let (hit_event, hit_yaw, hit_pitch, hit_count) =
        (pstate.damage_event, pstate.damage_yaw, pstate.damage_pitch, pstate.hurt_amount);
    stamp_damage_feedback(kick, hurt, hit_event, hit_yaw, hit_pitch, hit_count, pstate.look_angles, pstate.perks, now);

    // The bob with a resolved gun in hand reads that gun's own bob factors.
    let gun_bob_inputs = resolved.map(|info| {
        let breath = scope_breath_scale(kick, info, frac, breath_key_down, clock.step_millis_elapsed());
        CameraTiltSwayFeed {
            org: org_inputs,
            ent_bits: pstate.e_flags,
            hud_crosshair_layer: info.hud_crosshair_layer,
            ads_bob_factor: info.ads_bob_factor,
            ads_view_bob_mult: info.ads_view_bob_mult,
            time: now,
            hurt_stamp_ms: kick.hurt_stamp_ms,
            hurt_kick_pitch: kick.hurt_kick_pitch,
            hurt_kick_roll: kick.hurt_kick_roll,
            scoped_aiming: info.scoped_aiming,
            idle: info.idle,
            step_millis_elapsed: clock.step_duration_seconds(),
            steady_aim_factor: breath,
            gun_idle_clock: kick.gun_idle_clock,
            view_prev_sway_ratio: kick.view_prev_sway_ratio,
        }
    });
    let mut bob = bob_for_frame(gun_bob_inputs, org_inputs, now, kick);
    kick.gun_idle_clock = bob.gun_idle_clock;
    kick.view_prev_sway_ratio = bob.view_prev_sway_ratio;

    // A gun BO2's facts know brings its own scope drift and walk bob in place of the legacy ones.
    let own_bob = bo2_camera(kick) && kick.t6.facts.is_some();
    if own_bob {
        let drift = kick.t6.camera(frac, pstate.e_flags, pstate.movement_state_bits, clock.step_duration_seconds(), kick.hold_breath.sway_factor);
        bob.pitch += drift[0] - bob.camera_rest_nod;
        bob.yaw += drift[1] - bob.camera_rest_turn;
        bob.camera_rest_nod = drift[0];
        bob.camera_rest_turn = drift[1];
    }
    let legacy_walk = gun_bob_inputs.map_or([0.0; 3], view_walk_bob_angles);
    let walk = if own_bob {
        let own = kick.t6.view_bob(&ViewBobInput {
            frac,
            flags_e: pstate.e_flags,
            movement_state_bits: pstate.movement_state_bits,
            sway_phase: pstate.sway_phase as i32,
            ground_velocity: ground_speed,
            eye_height_goal: pstate.eye_height_goal,
        });
        bob.pitch += own[0] - legacy_walk[0];
        bob.yaw += own[1] - legacy_walk[1];
        bob.roll += own[2] - legacy_walk[2];
        own
    } else {
        legacy_walk
    };

    // For a Black Ops II gun the scope drift, the walk bob, the hit flinch and the
    // iron-sight gun turn all live in the aim (queued for the next command), so the
    // camera stops adding them.
    if kick.aim_kick {
        let drift = [bob.camera_rest_nod, bob.camera_rest_turn];
        retarget(&mut kick.idle_aim_applied, &mut kick.idle_aim_pending, drift);
        (bob.pitch, bob.yaw) = (bob.pitch - drift[0], bob.yaw - drift[1]);
    }
    let walk_in_aim = match kick.aim_kick {
        true => [walk[0], walk[1]],
        false => [0.0; 2],
    };
    retarget(&mut kick.bob_aim_applied, &mut kick.bob_aim_pending, walk_in_aim);
    let [weapon_tilt_angle, weapon_swing_angle] = kick.gun_aim_applied;
    bob.pitch = bob.pitch - walk_in_aim[0] - weapon_tilt_angle;
    bob.yaw = bob.yaw - walk_in_aim[1] - weapon_swing_angle;

    // The flinch goes into the aim for a Black Ops II gun (the server turns the shot by it,
    // bg_useClientDamageKick); an older gun keeps it camera-only, and the aim gives back its share.
    let flinch_in_aim = match kick.aim_kick {
        false => 0.0,
        true => {
            let (scope, iron) = resolved.map_or((0, false), |f| (f.hud_crosshair_layer, f.scoped_aiming));
            // A gun BO2's facts know has its own hit tilt inside the iron-sight gun turn.
            let tilt_in_gun_turn = iron && !own_bob;
            hit_aim_pitch(now, kick.hurt_stamp_ms, kick.hurt_kick_pitch, kick.hurt_kick_roll, frac, scope, tilt_in_gun_turn)
        }
    };
    kick.hit_aim_pending = kick.hit_aim_pending + (flinch_in_aim - kick.hit_aim_applied);
    kick.hit_aim_applied = flinch_in_aim;
    bob.pitch = bob.pitch - flinch_in_aim;

    // A Black Ops II gun's kick is in the aim already; only its roll is the camera's own.
    let mut camera_kick = kick.state.recoil_tilt;
    if kick.aim_kick {
        camera_kick[..2].fill(0.0);
    }
    let [kp, ky, kr] = kicked_view_angles(pstate.look_angles, camera_kick);
    let angles = [kp + bob.pitch, ky + bob.yaw + yaw_lead, kr + bob.roll];

    // The eye: the body's, lowered over a stair step (bo2zm), swayed by the walk bob and
    // dipped by a landing, then leaned.
    let mut eye: [f32; 3] = core::array::from_fn(|axis_in| {
        let level = base[axis_in] + eye_shift[axis_in];
        match axis_in {
            2 => level + pstate.eye_height_now,
            _ => level,
        }
    });
    if matches!(pstate.movement_mode, 0 | 2 | 3) {
        eye[2] -= presented.view_z_drop();
    }
    let sway = view_org_bob(org_inputs);
    eye[2] += sway.vertical;
    let (fwd, rgt, upv) = euler_basis(angles);
    for axis_in in 0..3 {
        eye[axis_in] += sway.horizontal * rgt[axis_in];
    }
    eye[2] += stamp_and_land_origin_z(kick, pstate.gravity, pstate.origin, pstate.velocity, pstate.standing_on, now);
    let since_landing = now.wrapping_sub(kick.touchdown_stamp);
    kick.land_z_viewweapon =
        viewweapon_land_origin_z(since_landing, kick.touchdown_dip) * kick.t6_land_factor.unwrap_or(1.0);
    let land_z = kick.land_z_viewweapon;
    kick.gun_landing_dip = [fwd, rgt, upv].map(|basis| land_z * basis[2]);
    let [lean_x, lean_y, lean_z] = lean_offset(eye, pstate.look_angles[1], pstate.tilt_sideways, 16.0, 20.0);
    let lowest_z = base[2] + eye_shift[2] + CAMERA_SWAY_LOWEST_DROP;
    WorldEyePlacement { turn: angles, origin: [lean_x, lean_y, lean_z.max(lowest_z)] }
}
