//! Where the player's own eyes are, and which way they look, this frame.
//!
//! The pose is the body's eye point plus the walk bob and the landing dip, turned by
//! the gun's kick, the hit flinch and the view bob. For a Black Ops II gun, part of
//! that turn is handed to the aim instead (queued for the next command) and so is
//! taken out of what the camera adds.

use assets::PreparedWeapons;
use asset_game::WeaponBodyFacts;
use game_math::{add_lean_to_position, angle_vectors, vec3_length};
use net::{FrameClock, PresentedSnapshot};
use playerstate_t6::game::{PlayerState, get_viewmodel_weapon_index};
use render_scene::WorldCameraPose;
use weapon_t6::view_bob::{
    VIEW_ORG_BOB_Z_MIN_OFS, ViewAngleBob, ViewAngleBobInputs, ViewOrgBobInputs, view_angle_bob,
    view_damage_angles, view_org_bob, view_walk_bob_angles, viewweapon_land_origin_z,
};

use super::feedback::{hit_aim_pitch, stamp_and_land_origin_z, stamp_damage_feedback};
use super::state::{PendingViewHurt, SessionViewKick, bo2_camera, retarget};
use crate::anim::view_kick_state::add_kick_to_viewangles;
use crate::occupancy::t6_movement::ViewBobInput;

/// What the first-person pose reads from the frame.
pub(super) struct FrameView<'a> {
    pub ps: &'a PlayerState,
    pub presented: &'a PresentedSnapshot,
    pub clock: &'a FrameClock,
    pub weapons: Option<&'a PreparedWeapons>,
    /// The hold-breath button is down.
    pub breath_key_down: bool,
}

/// Steps the scope state for a scoped gun and returns the drift scale that hold breath gives.
///
/// Also records whether the gun is a Variable Zoom scope in use and whether a Hybrid Optic is ready, for
/// the input to read, and opens a Variable Zoom scope wide whenever he is not aiming.
fn scope_breath_scale(
    kick: &mut SessionViewKick,
    facts: &WeaponBodyFacts,
    frac: f32,
    key_down: bool,
    frame_ms: i32,
) -> f32 {
    let scoped = facts.overlay_reticle != 0;
    kick.vzoom_scoped = facts.variable_zoom_fovs[0] > 0.0 && scoped && frac >= 1.0;
    kick.optic_ready = facts.optic_switch && frac >= 1.0;
    if frac <= 0.0 {
        kick.vzoom_in = false;
    }
    if kick.seeded_this_frame > 0 {
        kick.hold_breath.fired(frac, scoped);
    }
    kick.hold_breath.advance(
        facts.hold_breath_to_steady && scoped,
        frac,
        key_down,
        frame_ms as f32,
    )
}

/// The view angle bob for this frame: the full bob with a resolved gun, only the
/// hit flinch without one.
fn bob_for_frame(
    inputs: Option<ViewAngleBobInputs>,
    org: ViewOrgBobInputs,
    now: i32,
    kick: &SessionViewKick,
) -> ViewAngleBob {
    match inputs {
        Some(full) => view_angle_bob(full),
        None => view_damage_angles(ViewAngleBobInputs {
            org,
            time: now,
            damage_time: kick.damage_time,
            v_dmg_pitch: kick.v_dmg_pitch,
            v_dmg_roll: kick.v_dmg_roll,
            ..ViewAngleBobInputs::default()
        }),
    }
}

/// The first-person eye pose for this frame, before any camera shake. Updates the
/// flinch, the landing and the aim carry-overs on the way.
pub(super) fn first_person_pose(
    frame: &FrameView,
    kick: &mut SessionViewKick,
    hurt: &mut PendingViewHurt,
) -> WorldCameraPose {
    let FrameView { ps, presented, clock, weapons, breath_key_down } = *frame;
    let now = clock.time();
    let frac = ps.f_weapon_pos_frac;
    let viewmodel = get_viewmodel_weapon_index(ps);
    let gun_facts = weapons.and_then(|w| w.0.facts_of(viewmodel));
    let resolved = gun_facts.as_ref().filter(|f| f.body_resolved);

    let offset = presented.view_offset();
    let ground_speed = vec3_length([ps.velocity[0], ps.velocity[1], 0.0]);
    let org = ViewOrgBobInputs {
        bob_cycle: (ps.bob_cycle as u32 & 0xff) as u8,
        xyspeed: ground_speed,
        view_height_target: ps.view_height_target,
        pm_flags: ps.pm_flags,
        weapon_pos_frac: frac,
        perks0: ps.perks[0],
        bo2: bo2_camera(kick),
        waterlevel: ps.waterlevel,
    };
    stamp_damage_feedback(
        kick,
        hurt,
        ps.damage_event,
        ps.damage_yaw,
        ps.damage_pitch,
        ps.damage_count,
        ps.viewangles,
        ps.perks[0],
        now,
    );

    // The bob with a resolved gun in hand reads that gun's own bob factors.
    let gun_bob_inputs = resolved.map(|facts| {
        let breath = scope_breath_scale(kick, facts, frac, breath_key_down, clock.frametime());
        ViewAngleBobInputs {
            org,
            e_flags: ps.e_flags,
            overlay_reticle: facts.overlay_reticle,
            ads_bob_factor: facts.ads_bob_factor,
            ads_view_bob_mult: facts.ads_view_bob_mult,
            time: now,
            damage_time: kick.damage_time,
            v_dmg_pitch: kick.v_dmg_pitch,
            v_dmg_roll: kick.v_dmg_roll,
            aim_down_sight: facts.aim_down_sight,
            idle: facts.idle,
            frametime: clock.frametime_secs(),
            hold_breath_scale: breath,
            weap_idle_time: kick.weap_idle_time,
            view_last_idle_factor: kick.view_last_idle_factor,
        }
    });
    let mut bob = bob_for_frame(gun_bob_inputs, org, now, kick);
    kick.weap_idle_time = bob.weap_idle_time;
    kick.view_last_idle_factor = bob.view_last_idle_factor;

    // A gun BO2's facts know brings its own scope drift and walk bob in place of the legacy ones.
    let own_bob = bo2_camera(kick) && kick.t6.facts.is_some();
    if own_bob {
        let drift = kick.t6.camera(frac, ps.e_flags, ps.pm_flags, clock.frametime_secs(), kick.hold_breath.scale);
        bob.pitch += drift[0] - bob.cam_idle_pitch;
        bob.yaw += drift[1] - bob.cam_idle_yaw;
        bob.cam_idle_pitch = drift[0];
        bob.cam_idle_yaw = drift[1];
    }
    let legacy_walk = gun_bob_inputs.map_or([0.0; 3], view_walk_bob_angles);
    let walk = if own_bob {
        let own = kick.t6.view_bob(&ViewBobInput {
            frac,
            e_flags: ps.e_flags,
            pm_flags: ps.pm_flags,
            bob_cycle: ps.bob_cycle as i32,
            xyspeed: ground_speed,
            view_height_target: ps.view_height_target,
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
        let drift = [bob.cam_idle_pitch, bob.cam_idle_yaw];
        retarget(&mut kick.idle_aim_applied, &mut kick.idle_aim_pending, drift);
        bob.pitch -= drift[0];
        bob.yaw -= drift[1];
    }
    let walk_in_aim = if kick.aim_kick { [walk[0], walk[1]] } else { [0.0; 2] };
    retarget(&mut kick.bob_aim_applied, &mut kick.bob_aim_pending, walk_in_aim);
    bob.pitch -= walk_in_aim[0];
    bob.yaw -= walk_in_aim[1];
    bob.pitch -= kick.gun_aim_applied[0];
    bob.yaw -= kick.gun_aim_applied[1];

    // The flinch goes into the aim for a Black Ops II gun (the server turns the shot by it,
    // bg_useClientDamageKick); an older gun keeps it camera-only, and the aim gives back its share.
    let flinch_in_aim = if kick.aim_kick {
        let (scope, iron) = resolved.map_or((0, false), |f| (f.overlay_reticle, f.aim_down_sight));
        // A gun BO2's facts know has its own hit tilt inside the iron-sight gun turn.
        hit_aim_pitch(now, kick.damage_time, kick.v_dmg_pitch, kick.v_dmg_roll, frac, scope, iron && !own_bob)
    } else {
        0.0
    };
    kick.hit_aim_pending += flinch_in_aim - kick.hit_aim_applied;
    kick.hit_aim_applied = flinch_in_aim;
    bob.pitch -= flinch_in_aim;

    // A Black Ops II gun's kick is in the aim already; only its roll is the camera's own.
    let camera_kick = if kick.aim_kick {
        [0.0, 0.0, kick.state.kick_angles[2]]
    } else {
        kick.state.kick_angles
    };
    let kicked = add_kick_to_viewangles(ps.viewangles, camera_kick);
    let angles = [kicked[0] + bob.pitch, kicked[1] + bob.yaw, kicked[2] + bob.roll];

    // The eye: the body's, lowered over a stair step (bo2zm), swayed by the walk bob and
    // dipped by a landing, then leaned.
    let mut origin = [
        ps.origin[0] + offset[0],
        ps.origin[1] + offset[1],
        ps.origin[2] + offset[2] + ps.view_height_current,
    ];
    if matches!(ps.pm_type, 0 | 2 | 3) {
        origin[2] -= presented.view_z_drop();
    }
    let sway = view_org_bob(org);
    origin[2] += sway.vertical;
    let (forward, right, up) = angle_vectors(angles);
    origin[0] += sway.horizontal * right[0];
    origin[1] += sway.horizontal * right[1];
    origin[2] += sway.horizontal * right[2];
    origin[2] += stamp_and_land_origin_z(kick, ps.gravity, ps.origin, ps.velocity, ps.ground_entity_num, now);
    let since_landing = now.wrapping_sub(kick.land_time);
    kick.viewweapon_land_z =
        viewweapon_land_origin_z(since_landing, kick.land_change) * kick.t6_land_factor.unwrap_or(1.0);
    kick.viewweapon_land_view = [
        kick.viewweapon_land_z * forward[2],
        kick.viewweapon_land_z * right[2],
        kick.viewweapon_land_z * up[2],
    ];
    origin = add_lean_to_position(origin, ps.viewangles[1], ps.leanf, 16.0, 20.0);
    origin[2] = origin[2].max(ps.origin[2] + offset[2] + VIEW_ORG_BOB_Z_MIN_OFS);
    WorldCameraPose { origin, angles }
}
