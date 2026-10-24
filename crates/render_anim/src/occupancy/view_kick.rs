use asset_game::{WeaponBodyFacts, WeaponKickFacts};
use assets::PreparedWeapons;
use bevy::prelude::*;
use frame::{LifeStarted, ViewSubject};
use hud_iw4::{
    CG_FOV_MIN_DEFAULT, CG_FOV_SCALE_DEFAULT, FovInputs, WeaponAdsOverlayFacts, calc_fov_from_ads,
    horizontal_to_vertical_fov_deg, shellshock_remaining_ms, zoom_sensitivity,
};
use math_iw4::{add_lean_to_position, angle_vectors};
use net::{
    AppliedEntityEventWalk, ClientActionInput, FrameClock, LocalPresentClient, PresentedSnapshot,
};
use playerstate_iw4::ENTITYNUM_NONE;
use weapon_iw4::{
    VIEW_DAMAGE_UNDIRECTED, VIEW_ORG_BOB_Z_MIN_OFS, ViewAngleBobInputs, ViewOrgBobInputs,
    crash_land_fall_height, crash_land_view_dip, get_viewmodel_weapon_index, land_origin_z,
    sway_shellshock_landing_scale, view_angle_bob, view_damage_angles, view_org_bob, viewweapon_land_origin_z,
};

use crate::anim::view_kick_state::{KickParams, ViewKickState, add_kick_to_viewangles};
use crate::anim::view_sway::ViewSwayState;
use crate::occupancy::remote_body::RemotePlayer;
use crate::occupancy::t6_movement::T6Movement;
use crate::occupancy::third_person::{
    death_watch_camera, presented_is_third_person, remote_missile_camera,
};
use render_scene::{FlyCamera, FpvLens, SimCamera, transform_from_iw_view};
use render_scene::{WorldCameraPose, WorldScriptModelInstance};

const MISSILE_CAM_FOV: f32 = 15.0;

fn kick_params(k: &WeaponKickFacts) -> KickParams {
    KickParams {
        f_ads_view_kick_center_speed: k.f_ads_view_kick_center_speed,
        f_hip_view_kick_center_speed: k.f_hip_view_kick_center_speed,
        gun_max_pitch: k.gun_max_pitch,
        gun_max_yaw: k.gun_max_yaw,
        ads_gun_kick_reduced_kick_percent: k.ads_gun_kick_reduced_kick_percent,
        ads_gun_kick_pitch_min: k.ads_gun_kick_pitch_min,
        ads_gun_kick_pitch_max: k.ads_gun_kick_pitch_max,
        ads_gun_kick_yaw_min: k.ads_gun_kick_yaw_min,
        ads_gun_kick_yaw_max: k.ads_gun_kick_yaw_max,
        ads_gun_kick_accel: k.ads_gun_kick_accel,
        ads_gun_kick_speed_max: k.ads_gun_kick_speed_max,
        ads_gun_kick_speed_decay: k.ads_gun_kick_speed_decay,
        ads_gun_kick_static_decay: k.ads_gun_kick_static_decay,
        ads_view_kick_pitch_min: k.ads_view_kick_pitch_min,
        ads_view_kick_pitch_max: k.ads_view_kick_pitch_max,
        ads_view_kick_yaw_min: k.ads_view_kick_yaw_min,
        ads_view_kick_yaw_max: k.ads_view_kick_yaw_max,
        hip_gun_kick_reduced_kick_percent: k.hip_gun_kick_reduced_kick_percent,
        hip_gun_kick_pitch_min: k.hip_gun_kick_pitch_min,
        hip_gun_kick_pitch_max: k.hip_gun_kick_pitch_max,
        hip_gun_kick_yaw_min: k.hip_gun_kick_yaw_min,
        hip_gun_kick_yaw_max: k.hip_gun_kick_yaw_max,
        hip_gun_kick_accel: k.hip_gun_kick_accel,
        hip_gun_kick_speed_max: k.hip_gun_kick_speed_max,
        hip_gun_kick_speed_decay: k.hip_gun_kick_speed_decay,
        hip_gun_kick_static_decay: k.hip_gun_kick_static_decay,
        hip_view_kick_pitch_min: k.hip_view_kick_pitch_min,
        hip_view_kick_pitch_max: k.hip_view_kick_pitch_max,
        hip_view_kick_yaw_min: k.hip_view_kick_yaw_min,
        hip_view_kick_yaw_max: k.hip_view_kick_yaw_max,
        hip_view_kick_min_magnitude: k.hip_view_kick_min_magnitude,
        ads_view_kick_min_magnitude: k.ads_view_kick_min_magnitude,
    }
}

/// bo2zm: Black Ops II dual wield, hip fire only. With both guns firing
/// a shot kicks with the two guns' hip ranges added together; with only the
/// left gun firing it kicks with the left gun's own ranges.
fn dual_wield_kick(
    right: KickParams,
    left: &KickParams,
    right_firing: bool,
    left_firing: bool,
) -> KickParams {
    let mut k = right;
    if right_firing && left_firing {
        k.hip_view_kick_pitch_min += left.hip_view_kick_pitch_min;
        k.hip_view_kick_pitch_max += left.hip_view_kick_pitch_max;
        k.hip_view_kick_yaw_min += left.hip_view_kick_yaw_min;
        k.hip_view_kick_yaw_max += left.hip_view_kick_yaw_max;
        k.hip_gun_kick_pitch_min += left.hip_gun_kick_pitch_min;
        k.hip_gun_kick_pitch_max += left.hip_gun_kick_pitch_max;
        k.hip_gun_kick_yaw_min += left.hip_gun_kick_yaw_min;
        k.hip_gun_kick_yaw_max += left.hip_gun_kick_yaw_max;
    } else if left_firing {
        k.hip_view_kick_pitch_min = left.hip_view_kick_pitch_min;
        k.hip_view_kick_pitch_max = left.hip_view_kick_pitch_max;
        k.hip_view_kick_yaw_min = left.hip_view_kick_yaw_min;
        k.hip_view_kick_yaw_max = left.hip_view_kick_yaw_max;
        k.hip_gun_kick_pitch_min = left.hip_gun_kick_pitch_min;
        k.hip_gun_kick_pitch_max = left.hip_gun_kick_pitch_max;
        k.hip_gun_kick_yaw_min = left.hip_gun_kick_yaw_min;
        k.hip_gun_kick_yaw_max = left.hip_gun_kick_yaw_max;
    }
    k
}

#[cfg(test)]
mod shock_sway_tests {
    use super::shock_sway_scale;

    #[test]
    fn a_stunned_gun_sways_by_its_own_scale_and_eases_back() {
        let start = shock_sway_scale(4000, 4000, 2.0);
        let half = shock_sway_scale(2000, 4000, 2.0);
        let done = shock_sway_scale(0, 4000, 2.0);
        eprintln!("sway while stunned: start {start:.2} half {half:.2} done {done:.2}");
        assert_eq!(start, 2.0);
        assert!(half > 1.0 && half < 2.0);
        assert_eq!(done, 1.0);
        assert_eq!(shock_sway_scale(4000, 4000, 0.0), 1.0);
    }
}

#[cfg(test)]
mod dual_wield_kick_tests {
    use super::{KickParams, dual_wield_kick};

    fn gun(v: f32) -> KickParams {
        KickParams {
            hip_view_kick_pitch_min: v,
            hip_view_kick_pitch_max: 2.0 * v,
            hip_gun_kick_yaw_max: 3.0 * v,
            ads_view_kick_pitch_max: 9.0,
            ..KickParams::default()
        }
    }

    #[test]
    fn both_guns_firing_add_their_hip_kick() {
        let both = dual_wield_kick(gun(1.0), &gun(10.0), true, true);
        assert_eq!(both.hip_view_kick_pitch_min, 11.0);
        assert_eq!(both.hip_view_kick_pitch_max, 22.0);
        assert_eq!(both.hip_gun_kick_yaw_max, 33.0);
        // Aiming down the sights keeps the right gun's own kick.
        assert_eq!(both.ads_view_kick_pitch_max, 9.0);
        let left_only = dual_wield_kick(gun(1.0), &gun(10.0), false, true);
        assert_eq!(left_only.hip_view_kick_pitch_max, 20.0);
        let right_only = dual_wield_kick(gun(1.0), &gun(10.0), true, false);
        assert_eq!(right_only, gun(1.0));
    }
}

#[derive(Resource, Default)]
pub struct PendingViewHurt(pub u32);

#[derive(Resource, Default)]
pub struct SessionViewKick {
    pub state: ViewKickState,
    pub sway: ViewSwayState,

    pub placement_move_origin: [f32; 3],

    pub placement_move_angles: [f32; 3],

    pub weap_idle_time: i32,

    pub last_idle_factor: f32,

    pub view_last_idle_factor: f32,

    pub land_change: f32,

    pub land_time: i32,

    pub land_view_dip: i32,

    pub viewweapon_land_z: f32,

    pub viewweapon_land_view: [f32; 3],

    pub refdef_view_angles: [f32; 3],

    pub refdef_vieworg: [f32; 3],

    pub horiz_fov_deg: f32,

    pub killcam_focus_distance: Option<f32>,

    pub damage_time: i32,

    pub v_dmg_pitch: f32,

    pub v_dmg_roll: f32,
    last_weapon_id: u32,
    last_origin: [f32; 3],
    last_velocity: [f32; 3],
    last_ground_entity: i32,
    have_land_prev: bool,
    last_damage_event: u32,
    have_damage_prev: bool,

    pub seeded_this_frame: u32,

    last_weapon_pos_frac: f32,

    pub b_position_to_ads: bool,

    /// bo2zm: the held gun is Black Ops II's, whose view kick moves the
    /// aim itself (bullets follow the sights), not just the camera.
    pub aim_kick: bool,

    /// bo2zm: how much of the kick (pitch, yaw) is already in the aim.
    aim_applied: [f32; 2],

    /// bo2mp gun brain: BO2's own kick for a gun the T6 facts know (the shot's kick, the
    /// camera kick settling, the gun's recoil spring); `state` then shows this copy.
    t6_kick: weapon_t6::recoil::KickState,
    /// The random seed BO2's client draws the kick from.
    t6_seed: u32,
    /// The trigger has stayed down since the last shot: the gun has not left firing since it
    /// (a shot with it set builds up the ADS recoil reduction; the first shot of a burst has none).
    t6_held: bool,
    /// The gun's landing scale over the iw4 one (a rangefinder aiming dips the gun less).
    t6_land_factor: Option<f32>,
    /// The aim fraction that landing scale was read at (for the IW4L_SHOT_LOG land line).
    t6_land_frac: f32,
    /// BO2's gun sway, bob and drift, and the scope's drift on the camera (window 12).
    pub t6: T6Movement,
}

impl SessionViewKick {
    pub fn clear_for_new_life(&mut self) {
        self.state.reset();
        self.sway.reset();
        self.placement_move_origin = [0.0; 3];
        self.placement_move_angles = [0.0; 3];
        self.weap_idle_time = 0;
        self.last_idle_factor = 0.0;
        self.view_last_idle_factor = 0.0;
        self.land_change = 0.0;
        self.land_time = 0;
        self.land_view_dip = 0;
        self.viewweapon_land_z = 0.0;
        self.viewweapon_land_view = [0.0; 3];
        self.damage_time = 0;
        self.v_dmg_pitch = 0.0;
        self.v_dmg_roll = 0.0;
        self.last_origin = [0.0; 3];
        self.last_velocity = [0.0; 3];
        self.last_ground_entity = 0;
        self.have_land_prev = false;
        self.last_damage_event = 0;
        self.have_damage_prev = false;
        self.seeded_this_frame = 0;
        self.last_weapon_pos_frac = 0.0;
        self.b_position_to_ads = true;
        self.aim_applied = [0.0; 2];
        self.t6.clear();
    }
}

pub fn reset_view_kick_on_life_started(
    local: Res<LocalPresentClient>,
    mut started: MessageReader<LifeStarted>,
    mut kick: ResMut<SessionViewKick>,
    mut hurt: ResMut<PendingViewHurt>,
) {
    for ev in started.read() {
        if ev.client == local.0.0 {
            kick.clear_for_new_life();
            hurt.0 = 0;
        }
    }
}

pub fn tick_session_view_kick(
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    walk: Option<Res<AppliedEntityEventWalk>>,
    t6_facts: Option<Res<sim::t6_viewmodel::T6GunFactsRes>>,
    mut kick: ResMut<SessionViewKick>,
    mut look: Option<ResMut<net::LookState>>,
) {
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    let viewmodel = get_viewmodel_weapon_index(ps);
    if viewmodel != kick.last_weapon_id {
        kick.state.reset();
        kick.t6_kick = weapon_t6::recoil::KickState::default();
        kick.t6_held = false;
        // The aim keeps the kick it has; only new kick moves it.
        kick.aim_applied = [0.0; 2];
        kick.sway.reset();
        kick.placement_move_origin = [0.0; 3];
        kick.placement_move_angles = [0.0; 3];
        kick.weap_idle_time = 0;
        kick.last_idle_factor = 0.0;
        kick.view_last_idle_factor = 0.0;
        kick.seeded_this_frame = 0;
        kick.last_weapon_id = viewmodel;
        kick.last_weapon_pos_frac = 0.0;
        kick.b_position_to_ads = true;
        kick.t6.weapon_changed();
    }
    kick.t6.facts = None;

    let Some(reg) = weapons.as_ref() else {
        return;
    };
    let Some(facts) = reg.0.facts_of(viewmodel) else {
        return;
    };
    if !facts.body_resolved {
        return;
    }

    let dt = clock.frametime_secs();
    kick.seeded_this_frame = 0;

    let n = walk.map(|w| w.local_fire).unwrap_or(0);
    // bo2mp gun brain: a gun BO2's facts know kicks by BO2's own rules.
    let t6 = t6_facts
        .as_ref()
        .filter(|t| !t.0.name(viewmodel).is_empty())
        .map(|t| (t.0.get(viewmodel).spread, reg.0.dual_wield_weapon_of(viewmodel), t));
    kick.t6_land_frac = ps.f_weapon_pos_frac;
    kick.t6_land_factor = t6.as_ref().map(|(s, _, _)| {
        weapon_t6::recoil::gun_landing_scale(s, ps.f_weapon_pos_frac) / weapon_iw4::VIEWWEAPON_LAND_SCALE
    });
    if let Some((spread, left, table)) = t6.as_ref() {
        tick_t6_kick(&mut kick, ps, spread, *left, table, n, clock.time(), clock.frametime());
    } else if n > 0 {
        let reduce_window_active = ps.weapon_restrict_kick_time > 0;
        let mut params = kick_params(&facts.kick);
        let left = reg.0.dual_wield_weapon_of(viewmodel);
        if bo2_camera(&kick)
            && left != 0
            && let Some(left_facts) = reg.0.facts_of(left)
        {
            let firing = weapon_iw4::WeaponState::Firing as i32;
            params = dual_wield_kick(
                params,
                &kick_params(&left_facts.kick),
                ps.weaponstate_primary == firing,
                ps.weaponstate_secondary == firing,
            );
        }
        for _ in 0..n {
            kick.state.seed_fire(
                &params,
                ps.f_weapon_pos_frac,
                ps.weap_flags,
                ps.recoil_scale,
                reduce_window_active,
            );
        }
        kick.seeded_this_frame = n;
        // bo2zm: IW4L_SHOT_LOG=1: where the camera points against the aim.
        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *LOG.get_or_init(|| std::env::var_os("IW4L_SHOT_LOG").is_some()) {
            diag::info!(
                Fpv,
                "shot kick: camera off aim by [{:.2}, {:.2}] ads {:.2}",
                kick.state.kick_angles[0],
                kick.state.kick_angles[1],
                ps.f_weapon_pos_frac
            );
        }
    }

    let dt_ms = clock.frametime();
    let weapon_index = if viewmodel == 0 { 0 } else { 1 };
    let params = kick_params(&facts.kick);
    if t6.is_none() {
        kick.state
            .advance(&params, weapon_index, ps.f_weapon_pos_frac, dt_ms);
    }
    // bo2zm: a Black Ops II gun's kick moves the aim: queue this frame's
    // change for the next command, so the bullets go where the sights are.
    kick.aim_kick = reg.0.namespace_of(viewmodel) == Some(asset_core::AssetNamespace::T6);
    if kick.aim_kick
        && let Some(look) = look.as_mut()
    {
        for axis in 0..2 {
            let delta = kick.state.kick_angles[axis] - kick.aim_applied[axis];
            look.kick_pending[axis] += delta;
            kick.aim_applied[axis] = kick.state.kick_angles[axis];
        }
    }

    // bo2mp gun brain: BO2's own sway for a gun the T6 facts know (the iw4 sway below still runs
    // for its other readers).
    if bo2_camera(&kick)
        && let Some((_, _, table)) = t6.as_ref()
    {
        let f = table.0.get(viewmodel);
        kick.t6.facts = Some((f.spread, f.ads));
        let shock_left = if ps.shellshock_duration > 0 {
            ps.shellshock_time.wrapping_add(ps.shellshock_duration).wrapping_sub(clock.time()).max(0)
        } else {
            0
        };
        kick.t6.tick_sway(viewmodel, ps.f_weapon_pos_frac, ps.viewangles, dt, clock.frametime(), shock_left);
    }

    let overlay_active = facts.overlay_reticle != 0;
    if bo2_camera(&kick) && facts.aim_down_sight && overlay_active && ps.f_weapon_pos_frac > 0.0 {
        kick.sway.follow_view(ps.viewangles);
        return;
    }
    let shock_sway = if bo2_camera(&kick) {
        shock_sway_scale(
            shellshock_remaining_ms(clock.time(), ps.shellshock_time, ps.shellshock_duration),
            ps.shellshock_duration,
            facts.sway.sway_shell_shock_scale,
        )
    } else {
        1.0
    };
    kick.sway.advance(
        facts.sway.hip_params(),
        facts.sway.ads_params(),
        ps.viewangles,
        ps.f_weapon_pos_frac,
        facts.aim_down_sight,
        overlay_active,
        shock_sway,
        dt,
    );
}

/// bo2mp gun brain: one client frame of BO2's kick: this frame's shots (in CG_FireWeapon), the
/// right gun's shot counter (in CG_AddPlayerWeaponFx), then the camera kick settling
/// (CG_KickAngles) and the gun's recoil spring. The result is copied into `kick.state`, which
/// the camera, the aim and the gun placement already read.
#[allow(clippy::too_many_arguments)]
fn tick_t6_kick(
    kick: &mut SessionViewKick,
    ps: &playerstate_iw4::PlayerState,
    spread: &weapon_t6::pm::spread::SpreadFacts,
    left: u32,
    table: &sim::t6_viewmodel::T6GunFactsRes,
    shots: u32,
    now: i32,
    frame_ms: i32,
) {
    use weapon_t6::recoil::{
        CenterInput, FireRecoilInput, RecoilDvars, fire_recoil, gun_spring, settle_view_kick,
        view_kick_center_speed,
    };
    // The client copy of the player carries the old state numbers: one "firing" for every
    // BO2 firing state, which is what the counter and the dual-wield ranges ask.
    let iw4_firing = weapon_iw4::WeaponState::Firing as i32;
    let as_t6 = |s: i32| {
        if s == iw4_firing {
            weapon_t6::state::FIRING
        } else {
            weapon_t6::state::READY
        }
    };
    let (right_state, left_state) = (as_t6(ps.weaponstate_primary), as_t6(ps.weaponstate_secondary));
    let dual_facts = (left != 0).then(|| table.0.get(left).spread);
    let dvars = RecoilDvars::default();
    for _ in 0..shots {
        let held = kick.t6_held;
        let input = FireRecoilInput {
            facts: spread,
            dual: dual_facts.as_ref(),
            frac: ps.f_weapon_pos_frac,
            restrict_kick_time: ps.weapon_restrict_kick_time,
            held,
            now,
            charge_level: None,
            gas: false,
            weaponstate: right_state,
            weaponstate_left: left_state,
        };
        let mut seed = kick.t6_seed;
        fire_recoil(&mut kick.t6_kick, &input, &mut seed, &dvars);
        kick.t6_seed = seed;
        kick.t6_held = true;
        // IW4L_SHOT_LOG=1: each shot's BO2 kick.
        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *LOG.get_or_init(|| std::env::var_os("IW4L_SHOT_LOG").is_some()) {
            let k = &kick.t6_kick;
            diag::info!(
                Fpv,
                "t6 kick: ads {:.2} held {} reduction {:.3} view avel [{:.2}, {:.2}, {:.2}] gun speed [{:.2}, {:.2}] restrict {}",
                ps.f_weapon_pos_frac,
                held as u8,
                k.ads_recoil_reduction,
                k.view_avel[0],
                k.view_avel[1],
                k.view_avel[2],
                k.gun_speed[0],
                k.gun_speed[1],
                ps.weapon_restrict_kick_time
            );
        }
    }
    kick.seeded_this_frame = shots;
    if ps.weaponstate_primary != iw4_firing && ps.weaponstate_secondary != iw4_firing {
        kick.t6_held = false;
    }
    let stance = if ps.pm_flags & playerstate_iw4::pm_flags::PRONE != 0 {
        1
    } else if ps.pm_flags & playerstate_iw4::pm_flags::CROUCH != 0 {
        2
    } else {
        0
    };
    let center = view_kick_center_speed(
        &CenterInput {
            facts: Some(spread),
            dual: dual_facts.as_ref(),
            frac: ps.f_weapon_pos_frac,
            charge_level: None,
            weaponstate: right_state,
            weaponstate_left: left_state,
            stance,
        },
        &dvars,
    );
    if frame_ms > 0 {
        settle_view_kick(&mut kick.t6_kick, frame_ms, center);
        gun_spring(&mut kick.t6_kick, spread, ps.f_weapon_pos_frac, frame_ms as f32 * 0.001);
    }
    kick.state.kick_angles = kick.t6_kick.view_angles;
    kick.state.kick_avel = kick.t6_kick.view_avel;
    kick.state.gun_angles = kick.t6_kick.gun_angles;
    kick.state.gun_speed = kick.t6_kick.gun_speed;
}

/// bo2zm: while stunned or shocked, a Black Ops II gun sways by its own
/// swayShellShockScale, easing back to normal as the shock wears off.
fn shock_sway_scale(remaining_ms: i32, duration_ms: i32, gun_scale: f32) -> f32 {
    if gun_scale <= 0.0 {
        return 1.0;
    }
    sway_shellshock_landing_scale(remaining_ms, duration_ms, gun_scale)
}

pub fn sync_camera_from_presented(
    mut killcam: Local<super::killcam::KillcamCamera>,
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    sim_cam: Res<SimCamera>,
    settings: Res<frame::GameSettings>,
    mut kick: ResMut<SessionViewKick>,
    mut hurt: ResMut<PendingViewHurt>,
    weapons: Option<Res<PreparedWeapons>>,
    mut actions: Option<ResMut<ClientActionInput>>,
    mut q: Query<
        &mut Transform,
        (
            With<FlyCamera>,
            Without<RemotePlayer>,
            Without<WorldScriptModelInstance>,
        ),
    >,
    mut lenses: Query<&mut Projection, With<FpvLens>>,
    view: Res<ViewSubject>,
    death_cam_clip: Res<crate::occupancy::dyn_ent::DynEntPhysClip>,
    carry: Res<crate::occupancy::platform_carry::PlatformCarry>,
) {
    kick.killcam_focus_distance = None;
    if !sim_cam.enabled {
        return;
    }
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    let viewmodel = get_viewmodel_weapon_index(ps);
    if let Some((pose, fov, focus_distance)) = killcam.update(
        &presented,
        local.0,
        clock.time(),
        view.in_killcam(),
        weapons.as_deref(),
        death_cam_clip.0.as_deref(),
    ) {
        let pose = earthquake_pose(pose, &presented, clock.time());
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        for mut projection in &mut lenses {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = horizontal_to_vertical_fov_deg(fov).to_radians();
            }
        }
        kick.horiz_fov_deg = fov;
        kick.refdef_vieworg = pose.origin;
        kick.refdef_view_angles = pose.angles;
        kick.killcam_focus_distance = focus_distance;
        return;
    }
    if let Some(pose) = remote_missile_camera(&presented, local.0, clock.time()) {
        let pose = earthquake_pose(pose, &presented, clock.time());
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        let horiz = MISSILE_CAM_FOV;
        if let Some(actions) = actions.as_deref_mut() {
            actions.fov_scale = zoom_sensitivity(horiz) * actions.shellshock_look_scale;
        }
        let vertical = horizontal_to_vertical_fov_deg(horiz).to_radians();
        for mut projection in lenses.iter_mut() {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = vertical;
            }
        }
        kick.horiz_fov_deg = horiz;
        return;
    }
    if presented_is_third_person(&presented, local.0, view.in_killcam()) {
        let Some(pose) = death_watch_camera(&presented, local.0, death_cam_clip.0.as_deref())
        else {
            return;
        };
        let pose = earthquake_pose(pose, &presented, clock.time());
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        apply_fpv_lens_fov(
            &mut lenses,
            settings.fov,
            ps.pm_type,
            ps.link_flags,
            ps.e_flags,
            0.0,
            viewmodel,
            weapons.as_ref().and_then(|w| w.0.facts_of(viewmodel)),
            false,
            actions.as_deref_mut(),
        );
        return;
    }
    // bo2zm: riding the bus, his view keeps his place in the bus as drawn
    // (the server's 20 Hz carry steps are not a prediction miss to ease).
    let (base, offset, yaw_lead) = match carry.0 {
        Some(pose) => (pose.carry_point(ps.origin), [0.0; 3], pose.yaw_lead()),
        None => (ps.origin, presented.view_offset(), 0.0),
    };
    let xyspeed = {
        let vx = ps.velocity[0];
        let vy = ps.velocity[1];
        math_iw4::vec3_length([vx, vy, 0.0])
    };
    let org = ViewOrgBobInputs {
        bob_cycle: (ps.bob_cycle as u32 & 0xff) as u8,
        xyspeed,
        view_height_target: ps.view_height_target,
        pm_flags: ps.pm_flags,
        weapon_pos_frac: ps.f_weapon_pos_frac,
        perks0: ps.perks[0],
        bo2: bo2_camera(&kick),
    };
    stamp_damage_feedback(
        &mut kick,
        &mut hurt,
        ps.damage_event,
        ps.damage_yaw,
        ps.damage_pitch,
        ps.damage_count,
        ps.viewangles,
        ps.perks[0],
        clock.time(),
    );
    let bob_angles = match weapons.as_ref().and_then(|w| w.0.facts_of(viewmodel)) {
        Some(facts) if facts.body_resolved => view_angle_bob(ViewAngleBobInputs {
            org,
            e_flags: ps.e_flags,
            overlay_reticle: facts.overlay_reticle,
            ads_bob_factor: facts.ads_bob_factor,
            ads_view_bob_mult: facts.ads_view_bob_mult,
            time: clock.time(),
            damage_time: kick.damage_time,
            v_dmg_pitch: kick.v_dmg_pitch,
            v_dmg_roll: kick.v_dmg_roll,
            aim_down_sight: facts.aim_down_sight,
            idle: facts.idle,
            frametime: clock.frametime_secs(),
            hold_breath_scale: 1.0,
            weap_idle_time: kick.weap_idle_time,
            view_last_idle_factor: kick.view_last_idle_factor,
        }),
        _ => view_damage_angles(ViewAngleBobInputs {
            org,
            time: clock.time(),
            damage_time: kick.damage_time,
            v_dmg_pitch: kick.v_dmg_pitch,
            v_dmg_roll: kick.v_dmg_roll,
            ..ViewAngleBobInputs::default()
        }),
    };
    kick.weap_idle_time = bob_angles.weap_idle_time;
    kick.view_last_idle_factor = bob_angles.view_last_idle_factor;
    // bo2mp gun brain: a T6 gun's scope drift by BO2's rules in place of the iw4 one (no
    // hold breath in Zombies: scale 1).
    let mut bob_angles = bob_angles;
    if bo2_camera(&kick) && kick.t6.facts.is_some() {
        let idle = kick.t6.camera(
            ps.f_weapon_pos_frac,
            ps.e_flags,
            ps.pm_flags,
            clock.frametime_secs(),
            1.0,
        );
        bob_angles.pitch += idle[0] - bob_angles.cam_idle_pitch;
        bob_angles.yaw += idle[1] - bob_angles.cam_idle_yaw;
        bob_angles.cam_idle_pitch = idle[0];
        bob_angles.cam_idle_yaw = idle[1];
    }
    // bo2zm: a Black Ops II gun's kick is in the aim already; only its
    // roll is the camera's own.
    let camera_kick = if kick.aim_kick {
        [0.0, 0.0, kick.state.kick_angles[2]]
    } else {
        kick.state.kick_angles
    };
    let kick_angles = add_kick_to_viewangles(ps.viewangles, camera_kick);
    let angles = [
        kick_angles[0] + bob_angles.pitch,
        kick_angles[1] + bob_angles.yaw + yaw_lead,
        kick_angles[2] + bob_angles.roll,
    ];
    kick.refdef_view_angles = angles;
    let mut origin = [
        base[0] + offset[0],
        base[1] + offset[1],
        base[2] + offset[2] + ps.view_height_current,
    ];
    // bo2zm: Black Ops II eases the camera over a stair step.
    if matches!(ps.pm_type, 0 | 2 | 3) {
        origin[2] -= presented.view_z_drop();
    }
    let bob = view_org_bob(org);
    origin[2] += bob.vertical;

    let (fwd, right, up) = angle_vectors(angles);
    origin[0] += bob.horizontal * right[0];
    origin[1] += bob.horizontal * right[1];
    origin[2] += bob.horizontal * right[2];
    let land_ofs = stamp_and_land_origin_z(
        &mut kick,
        ps.gravity,
        ps.origin,
        ps.velocity,
        ps.ground_entity_num,
        clock.time(),
    );
    origin[2] += land_ofs;
    let delta_ms = clock.time().wrapping_sub(kick.land_time);
    kick.viewweapon_land_z = viewweapon_land_origin_z(delta_ms, kick.land_change)
        * kick.t6_land_factor.unwrap_or(1.0);
    kick.viewweapon_land_view = [
        kick.viewweapon_land_z * fwd[2],
        kick.viewweapon_land_z * right[2],
        kick.viewweapon_land_z * up[2],
    ];
    origin = add_lean_to_position(origin, ps.viewangles[1], ps.leanf, 16.0, 20.0);
    let min_z = base[2] + offset[2] + VIEW_ORG_BOB_Z_MIN_OFS;
    if origin[2] < min_z {
        origin[2] = min_z;
    }
    let pose = earthquake_pose(WorldCameraPose { origin, angles }, &presented, clock.time());
    kick.refdef_vieworg = pose.origin;
    kick.refdef_view_angles = pose.angles;
    crate::occupancy::platform_carry::camera_jitter(
        clock.time(),
        pose.origin,
        carry.0.is_some(),
    );
    let eye = transform_from_iw_view(pose);
    for mut transform in &mut q {
        transform.translation = eye.translation;
        transform.rotation = eye.rotation;
    }

    if ps.f_weapon_pos_frac > kick.last_weapon_pos_frac {
        kick.b_position_to_ads = true;
    } else if ps.f_weapon_pos_frac < kick.last_weapon_pos_frac {
        kick.b_position_to_ads = false;
    }
    kick.last_weapon_pos_frac = ps.f_weapon_pos_frac;
    if let Some(horiz) = apply_fpv_lens_fov(
        &mut lenses,
        settings.fov,
        ps.pm_type,
        ps.link_flags,
        ps.e_flags,
        ps.f_weapon_pos_frac,
        viewmodel,
        weapons.as_ref().and_then(|w| w.0.facts_of(viewmodel)),
        kick.b_position_to_ads,
        actions.as_deref_mut(),
    ) {
        kick.horiz_fov_deg = horiz;
    }
}

fn apply_fpv_lens_fov(
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
    let facts = facts.filter(|f| f.body_resolved).unwrap_or_default();
    let overlay = WeaponAdsOverlayFacts {
        ads_zoom_in_frac: facts.ads_zoom_in_frac,
        ads_zoom_out_frac: facts.ads_zoom_out_frac,
        overlay_reticle: facts.overlay_reticle,
        ..WeaponAdsOverlayFacts::default()
    };
    let ads_target = if facts.ads_zoom_fov > 0.0 {
        facts.ads_zoom_fov
    } else {
        base_fov
    };
    let inputs = FovInputs {
        cg_fov: base_fov,
        pm_type,
        link_flags,
        e_flags,
        weapon_index_nonzero: viewmodel != 0,
        aim_down_sight: facts.aim_down_sight && facts.ads_zoom_fov > 0.0,
        ads_zoom_fov: ads_target,
        overlay_zoom: 0.0,
        fov_scale: CG_FOV_SCALE_DEFAULT,
        fov_min: CG_FOV_MIN_DEFAULT,
    };
    let (horiz, _) = calc_fov_from_ads(&inputs, f_weapon_pos_frac, b_position_to_ads, &overlay);
    let zoom_sensitivity = zoom_sensitivity(horiz);
    if let Some(actions) = actions {
        actions.fov_scale = zoom_sensitivity * actions.shellshock_look_scale;
    }
    let vertical = horizontal_to_vertical_fov_deg(horiz).to_radians();
    for mut projection in lenses.iter_mut() {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = vertical;
        }
    }
    Some(horiz)
}

fn stamp_and_land_origin_z(
    kick: &mut SessionViewKick,
    gravity: i32,
    origin: [f32; 3],
    velocity: [f32; 3],
    ground_entity: i32,
    cg_time: i32,
) -> f32 {
    if kick.have_land_prev
        && kick.last_ground_entity == ENTITYNUM_NONE
        && ground_entity != ENTITYNUM_NONE
    {
        if let Some(fall) = crash_land_fall_height(
            gravity,
            kick.last_origin[2],
            origin[2],
            kick.last_velocity[2],
        ) {
            let dip = crash_land_view_dip(fall, bo2_camera(kick));
            if dip > 0 {
                kick.land_change = -(dip as f32);
                kick.land_time = cg_time;
                kick.land_view_dip = dip;
                // IW4L_SHOT_LOG=1: each T6 gun landing (the gun dips by the dip times its scale).
                static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if let Some(factor) = kick.t6_land_factor
                    && *LOG.get_or_init(|| std::env::var_os("IW4L_SHOT_LOG").is_some())
                {
                    diag::info!(
                        Fpv,
                        "t6 land: fall {:.1} dip {} gun scale {:.3} frac {:.2}",
                        fall,
                        dip,
                        factor * weapon_iw4::VIEWWEAPON_LAND_SCALE,
                        kick.t6_land_frac
                    );
                }
            }
        }
    }
    kick.last_origin = origin;
    kick.last_velocity = velocity;
    kick.last_ground_entity = ground_entity;
    kick.have_land_prev = true;
    let delta = cg_time.wrapping_sub(kick.land_time) as f32;
    land_origin_z(delta, kick.land_change)
}

fn stamp_damage_feedback(
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
    // bo2zm: Black Ops II's own flinch (0.175 a percent of health, a
    // quarter with Toughness); None keeps the last flinch's angles.
    let bo2 = bo2_camera(kick);
    let bullet_flinch = (perks0 & weapon_iw4::PERK_BULLETFLINCH) != 0;
    let punch = |yaw: u32, pitch: u32, count: i32| {
        if bo2 {
            weapon_iw4::damage_feedback_kick_bo2(yaw, pitch, count, viewangles, bullet_flinch)
        } else {
            Some(weapon_iw4::damage_feedback_kick(yaw, pitch, count, viewangles))
        }
    };
    let mut stamped = false;
    if kick.have_damage_prev && damage_event != kick.last_damage_event && damage_count != 0 {
        if let Some(punch) = punch(damage_yaw, damage_pitch, damage_count) {
            kick.v_dmg_pitch = punch.v_dmg_pitch;
            kick.v_dmg_roll = punch.v_dmg_roll;
        }
        kick.damage_time = cg_time.max(1);
        stamped = true;
    }
    if !stamped && hurt.0 > 0 {
        hurt.0 -= 1;
        if let Some(punch) = punch(VIEW_DAMAGE_UNDIRECTED, VIEW_DAMAGE_UNDIRECTED, 1) {
            kick.v_dmg_pitch = punch.v_dmg_pitch;
            kick.v_dmg_roll = punch.v_dmg_roll;
        }
        kick.damage_time = cg_time.max(1);
    }
    kick.last_damage_event = damage_event;
    kick.have_damage_prev = true;
}

/// bo2zm: Black Ops II camera rules while holding a Black Ops II gun.
/// BO2_FEEL=mw2 keeps the old camera, to compare the two.
pub(crate) fn bo2_camera(kick: &SessionViewKick) -> bool {
    static MW2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let mw2 =
        *MW2.get_or_init(|| std::env::var("BO2_FEEL").is_ok_and(|v| v.eq_ignore_ascii_case("mw2")));
    kick.aim_kick && !mw2
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct GunOffset {
    pub x: f32,

    pub y: f32,

    pub z: f32,
}

impl GunOffset {
    pub fn xyz(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

pub(crate) fn apply_cg_gun_offset_view(origin: [f32; 3], gun: [f32; 3]) -> [f32; 3] {
    [origin[0] + gun[0], origin[1] + gun[1], origin[2] + gun[2]]
}

pub(crate) fn apply_viewweapon_land_view(origin: [f32; 3], land_view: [f32; 3]) -> [f32; 3] {
    [
        origin[0] + land_view[0],
        origin[1] + land_view[1],
        origin[2] + land_view[2],
    ]
}

pub(crate) fn iw_view_placement_to_bevy_camera_local(
    origin: [f32; 3],
    angles_deg: [f32; 3],
) -> Transform {
    let [fwd, right, up] = origin;
    let translation = Vec3::new(right, up, -fwd);
    Transform {
        translation,
        rotation: crate::anim::fpv_pose::placement_angles_to_bevy_camera_quat(angles_deg),
        ..Default::default()
    }
}

fn earthquake_pose(
    mut pose: WorldCameraPose,
    presented: &PresentedSnapshot,
    now_ms: i32,
) -> WorldCameraPose {
    if let Some(snapshot) = presented.snapshot() {
        let eye = pose.origin;
        for quake in &snapshot.meta.objectives.earthquakes {
            let offset = quake.angle_offset(eye, now_ms);
            for (angle, delta) in pose.angles.iter_mut().zip(offset) {
                *angle += delta;
            }
        }
    }
    pose
}
