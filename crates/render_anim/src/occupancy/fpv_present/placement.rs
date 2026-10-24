//! Where the gun sits relative to the camera each frame: bob, sway, recoil,
//! stance offsets and the Black Ops II gun brain's own pose, composed into the
//! placement root's transform, plus the aim the HUD and the shooting side read.

use bevy::prelude::*;
use frame::ViewSubject;
use game_math::{angle_vectors, vec3_length};
use net::{FrameClock, LocalPresentClient, PresentedSnapshot, ViewweaponAim};
use playerstate_t6::game::{PlayerState, eflags, mantle_flags, pm_flags};
use render_scene::{
    HostGfxScene, SCENE_VIEWMODEL_ENTNUM, SCENE_VIEWMODEL_LEFT_ENTNUM, WorldScriptModelInstance,
    scene_quat_from_viewmodel_axes,
};
use weapon_t6::kick::{GunKickSpring, GunRecoilPlacementState};
use weapon_t6::placement::{
    Bo2GunBob, PLACEMENT_ASSEMBLE_STEP_COUNT, StanceTransitionFadeGlobals, WeaponBobInputs,
    WeaponBobWaveformInputs, WeaponMovementKinematics, WeaponPlacementAssembleStep,
    WeaponPlacementPsInputs, WeaponPlacementState, WeaponStanceStaticOfsInputs,
    calculate_weapon_movement_bob_waveform, dual_wield_view_model_origin_add,
    weapon_placement_assemble,
};
use weapon_t6::viewweapon::{
    viewweapon_iron_ads_saves_composed_axis, viewweapon_save_gun_pitch_yaw,
    viewweapon_view_to_world_delta,
};

use super::FpvPlacementRoot;
use super::weapon::fpv_viewmodel_weapon;
use crate::anim::fpv_prepared::PreparedFpv;
use crate::occupancy::remote_body::RemotePlayer;
use crate::occupancy::t6_movement::{GunMoveInput, PMF_DIVE_ANY};
use crate::occupancy::third_person::presented_is_third_person;
use crate::occupancy::view_kick::{
    GunOffset, SessionViewKick, apply_cg_gun_offset_view, apply_viewweapon_land_view, bo2_camera,
    iron_gun_turn, iw_view_placement_to_bevy_camera_local,
};
use hud_t6::crosshair::calc_crosshair_position;
use hud_t6::fov::tan_half_fov;

/// The kick spring a gun uses for one stance, read from its facts.
fn kick_spring(accel: f32, speed_max: f32, speed_decay: f32, static_decay: f32) -> GunKickSpring {
    GunKickSpring {
        accel,
        speed_max,
        speed_decay,
        static_decay,
    }
}

/// Inputs for the Black Ops II gun brain, taken from the player state.
fn t6_gun_input(
    ps: &PlayerState,
    frametime: f32,
    xyspeed: f32,
    perks: u64,
    mantle_frac: Option<f32>,
    state: &WeaponPlacementState,
) -> GunMoveInput {
    GunMoveInput {
        frac: ps.f_weapon_pos_frac,
        e_flags: ps.e_flags,
        pm_flags: ps.pm_flags,
        frametime,
        bob_cycle: ps.bob_cycle as i32,
        xyspeed,
        waterlevel: ps.waterlevel,
        view_height_target: ps.view_height_target,
        pose: weapon_t6::pose::PoseInput {
            ducked: ps.e_flags & eflags::DUCK != 0,
            prone: ps.e_flags & eflags::PRONE != 0,
            diving: ps.pm_flags & PMF_DIVE_ANY != 0,
            on_ladder: ps.pm_flags & pm_flags::LADDER != 0,
            weaponstate: ps.weapon_state_t6,
            weaponstate_left: ps.weapon_state_left_t6,
            weapon_time: ps.weapon_time,
            frac: ps.f_weapon_pos_frac,
            xyspeed,
            speed: ps.speed as f32,
            velocity: ps.velocity,
            viewangles: ps.viewangles,
            frametime,
            fast_sprint_recovery: perks & (1 << weapon_t6::viewmodel::PERK_FASTSPRINTRECOVERY)
                != 0,
            sprint_recovery_multiplier: weapon_t6::viewmodel::VmDvars::default()
                .perk_sprint_recovery_multiplier,
            mantle_frac,
        },
        recoil: [state.gun_recoil.pitch_offset, state.gun_recoil.yaw_offset],
        damage_kick: state.damage_kick_angles,
    }
}

/// The crosshair position for a gun aimed along `gun_pitch` / `gun_yaw`, or
/// the screen centre when there is no window or no field of view yet.
fn crosshair_for(
    kick: &SessionViewKick,
    window: Option<&Window>,
    gun_pitch: f32,
    gun_yaw: f32,
) -> [f32; 2] {
    let Some(window) = window.filter(|_| kick.horiz_fov_deg > 0.0) else {
        return [0.0, 0.0];
    };
    let aspect = window.width() / window.height().max(1.0);
    let (tan_x, tan_y) = tan_half_fov(kick.horiz_fov_deg, aspect);
    let (forward, right, up) = angle_vectors(kick.refdef_view_angles);
    calc_crosshair_position(gun_pitch, gun_yaw, [forward, right, up], tan_x, tan_y)
}

/// Computes the viewmodel's placement for this frame, stores it on the
/// placement root, publishes the gun aim, and tells the scene where each hand's
/// model is in the world.
#[allow(clippy::too_many_arguments)]
pub fn apply_fpv_placement(
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    prepared: Res<PreparedFpv>,
    mut kick: ResMut<SessionViewKick>,
    cg_gun: Res<GunOffset>,
    mut aim: ResMut<ViewweaponAim>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut roots: Query<
        &mut Transform,
        (
            With<FpvPlacementRoot>,
            Without<RemotePlayer>,
            Without<WorldScriptModelInstance>,
        ),
    >,
    view: Res<ViewSubject>,
    mut gfx_scene: ResMut<HostGfxScene>,
) {
    *aim = ViewweaponAim::default();
    let Ok(mut root) = roots.single_mut() else {
        return;
    };
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    if presented_is_third_person(&presented, local.0, view.in_killcam()) {
        return;
    }
    let Some(table) = prepared.table() else {
        return;
    };
    let shown = fpv_viewmodel_weapon(ps, table);
    let Some(facts) = table.facts_of(shown) else {
        return;
    };
    let frametime = clock.frametime_secs();

    let mut state = WeaponPlacementState {
        sway_springs: kick.sway.springs(),
        gun_recoil: GunRecoilPlacementState {
            pitch_offset: kick.state.gun_angles[0],
            pitch_speed: kick.state.gun_speed[0],
            yaw_offset: kick.state.gun_angles[1],
            yaw_speed: kick.state.gun_speed[1],
        },
        movement_origin: kick.placement_move_origin,
        movement_angles: kick.placement_move_angles,
        weap_idle_time: kick.weap_idle_time,
        last_idle_factor: kick.last_idle_factor,
        damage_kick_time: clock.time(),
        damage_time: kick.damage_time,
        v_dmg_pitch: kick.v_dmg_pitch,
        v_dmg_roll: kick.v_dmg_roll,
        ..Default::default()
    };

    // The weapon's own overlay reticle, else 1 when its overlay is the HUD iris.
    let overlay_reticle = if facts.overlay_reticle != 0 {
        facts.overlay_reticle
    } else {
        i32::from(table.overlay_is_hud_iris(shown))
    };
    let xyspeed = vec3_length([ps.velocity[0], ps.velocity[1], 0.0]);
    let waveform = calculate_weapon_movement_bob_waveform(WeaponBobWaveformInputs {
        bob_cycle: (ps.bob_cycle as u32 & 0xff) as u8,
        xyspeed,
        view_height_target: ps.view_height_target,
        pm_flags: ps.pm_flags,
        weapon_pos_frac: ps.f_weapon_pos_frac,
        bo2_gun: bo2_camera(&kick).then_some(Bo2GunBob {
            sprint: facts.bob.sprint_bob,
            dtp: facts.bob.dtp_bob,
        }),
        waterlevel: ps.waterlevel,
    });
    let mut steps = [WeaponPlacementAssembleStep::Sway; PLACEMENT_ASSEMBLE_STEP_COUNT];
    let mut contrib = weapon_placement_assemble(
        &mut state,
        WeaponPlacementPsInputs {
            e_flags: ps.e_flags,
            weapon_pos_frac: ps.f_weapon_pos_frac,
            weapon_time: ps.weapon_time,
            aim_down_sight: facts.aim_down_sight,
            overlay_reticle,
            weapon_transition_active: false,
            lean_fraction: 0.0,
        },
        WeaponStanceStaticOfsInputs {
            ducked_ofs: facts.ducked_ofs,
            prone_ofs: facts.prone_ofs,
            ads_aim_pitch: facts.ads_aim_pitch,
            night_vision_wear_time: facts.night_vision_wear_time,
        },
        StanceTransitionFadeGlobals::default(),
        facts.movement,
        WeaponMovementKinematics {
            xyspeed,
            speed: ps.speed as f32,
            velocity: ps.velocity,
            viewangles: ps.viewangles,
            weaponstate: ps.weapon_state_t6,
            weaponstate_secondary: ps.weapon_state_left_t6,
            pm_flags: ps.pm_flags,
            frametime,
            bo2_mantle: mantle_flags::bo2_weapon_up(ps),
        },
        WeaponBobInputs {
            ads_bob_factor: facts.ads_bob_factor,
        },
        facts.idle,
        Some(waveform),
        kick_spring(
            facts.kick.hip_gun_kick_accel,
            facts.kick.hip_gun_kick_speed_max,
            facts.kick.hip_gun_kick_speed_decay,
            facts.kick.hip_gun_kick_static_decay,
        ),
        kick_spring(
            facts.kick.ads_gun_kick_accel,
            facts.kick.ads_gun_kick_speed_max,
            facts.kick.ads_gun_kick_speed_decay,
            facts.kick.ads_gun_kick_static_decay,
        ),
        facts.kick.gun_max_pitch,
        facts.kick.gun_max_yaw,
        0.0,
        &mut steps,
    );
    kick.placement_move_origin = state.movement_origin;
    kick.placement_move_angles = state.movement_angles;
    kick.weap_idle_time = state.weap_idle_time;
    kick.last_idle_factor = state.last_idle_factor;

    // A Black Ops II gun is posed by its own rules for sway, bob and drift; the
    // generic assembly above still supplies the recoil spring and damage kick.
    let perks = presented
        .snapshot()
        .and_then(|snap| snap.meta.for_client(local.0))
        .map_or(0, |meta| meta.t6_perks);
    let mantle_frac = (ps.pm_flags & pm_flags::MANTLE != 0 && mantle_flags::bo2_weapon_up(ps))
        .then(|| mantle_flags::bo2_progress(ps))
        .flatten();
    if bo2_camera(&kick) {
        let input = t6_gun_input(ps, frametime, xyspeed, perks, mantle_frac, &state);
        if let Some((angles, origin)) = kick.t6.gun(&input) {
            contrib.angles = angles;
            contrib.origin = origin;
        }
    }

    let mut origin = apply_viewweapon_land_view(
        apply_cg_gun_offset_view(contrib.origin, cg_gun.xyz()),
        kick.viewweapon_land_view,
    );
    let dual = ps.last_weapon_hand == 1;
    if dual {
        let shift = dual_wield_view_model_origin_add(
            0,
            [0.0, 1.0, 0.0],
            facts.dual_wield_view_model_offset,
        );
        for (axis, delta) in origin.iter_mut().zip(shift) {
            *axis += delta;
        }
    }

    let from_composed_axis = viewweapon_iron_ads_saves_composed_axis(
        facts.aim_down_sight,
        ps.f_weapon_pos_frac,
        overlay_reticle,
    );
    let [gun_pitch, gun_yaw] = viewweapon_save_gun_pitch_yaw(
        contrib.angles,
        kick.refdef_view_angles,
        facts.aim_down_sight,
        ps.f_weapon_pos_frac,
        overlay_reticle,
    );
    // Through iron sights (no scope overlay, not fully lowered) the shot leaves
    // along the gun, so the aim takes the gun's own turn while the camera does
    // not. Off iron sights the aim gives it back.
    let through_iron = ps.f_weapon_pos_frac != 0.0
        && kick
            .t6
            .facts
            .is_some_and(|(sight, _)| sight.aim_down_sight && !sight.overlay);
    let gun_aim = if kick.aim_kick && bo2_camera(&kick) && through_iron {
        iron_gun_turn(contrib.angles, kick.refdef_view_angles)
    } else {
        [0.0; 2]
    };
    kick.set_gun_aim(gun_aim);

    let [xhair_x, xhair_y] = crosshair_for(&kick, windows.single().ok(), gun_pitch, gun_yaw);
    *aim = ViewweaponAim {
        live: true,
        gun_pitch,
        gun_yaw,
        xhair_x,
        xhair_y,
        from_composed_axis,
    };

    let delta = viewweapon_view_to_world_delta(origin, kick.refdef_view_angles);
    let world_origin = std::array::from_fn(|i| kick.refdef_vieworg[i] + delta[i]);
    let world_quat = scene_quat_from_viewmodel_axes(contrib.angles, kick.refdef_view_angles);
    let scene = &mut gfx_scene.scene;
    scene.store_pose_origin_quat(SCENE_VIEWMODEL_ENTNUM, world_origin, Some(world_quat));
    if dual {
        scene.store_pose_origin_quat(SCENE_VIEWMODEL_LEFT_ENTNUM, world_origin, Some(world_quat));
    }
    *root = iw_view_placement_to_bevy_camera_local(origin, contrib.angles);
}
