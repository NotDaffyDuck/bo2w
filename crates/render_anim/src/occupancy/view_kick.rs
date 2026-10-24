use asset_game::{WeaponBodyFacts, WeaponKickFacts};
use assets::PreparedWeapons;
use bevy::prelude::*;
use frame::{LifeStarted, ViewSubject};
use hud_t6::flashbang::shellshock_remaining_ms;
use hud_t6::fov::{
    AdsZoom, CG_FOV_DEFAULT, LINK_FLAG_FORCE_ADS_ZOOM, ViewFovInputs, vertical_fov_deg, view_fov,
    zoom_sensitivity,
};
use game_math::{add_lean_to_position, angle_vectors};
use net::{
    AppliedEntityEventWalk, ClientActionInput, FrameClock, LocalPresentClient, PresentedSnapshot,
};
use playerstate_t6::game::ENTITYNUM_NONE;
use weapon_t6::view_bob::{VIEW_DAMAGE_UNDIRECTED, VIEW_ORG_BOB_Z_MIN_OFS, ViewAngleBobInputs, ViewOrgBobInputs, crash_land_fall_height, crash_land_view_dip, land_origin_z, view_angle_bob, view_damage_angles, view_org_bob, view_walk_bob_angles, viewweapon_land_origin_z};
use playerstate_t6::game::get_viewmodel_weapon_index;
use weapon_t6::sway::sway_shellshock_landing_scale;
use weapon_t6::placement::weapon_damage_kick_angles;

use crate::anim::view_kick_state::{KickParams, ViewKickState, add_kick_to_viewangles};
use crate::anim::view_sway::ViewSwayState;
use crate::occupancy::remote_body::RemotePlayer;
use crate::occupancy::t6_movement::{T6Movement, ViewBobInput};
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

    /// bo2mp: a scoped Black Ops II gun's drift (its ADS idle) moves the
    /// aim as well, so the bullet goes to the scope's crosshair: how much
    /// of it is already in the aim, and what the next command still owes.
    idle_aim_applied: [f32; 2],
    idle_aim_pending: [f32; 2],

    /// bo2mp: a hit moves the aim the way Black Ops II's server moves the
    /// shot (`hit_aim_pitch`): how much pitch is in the aim, and what the
    /// next command still owes. Kept over a weapon switch, so the aim gives
    /// back exactly what the hit put in.
    hit_aim_applied: f32,
    hit_aim_pending: f32,

    /// bo2mp: Black Ops II's server turns the shot by the view's walk bob
    /// (the scope's bob and the ADS bob), so the aim takes it as it takes
    /// the drift: how much (pitch, yaw) is in the aim, and what the next
    /// command still owes. Kept over a weapon switch, like the hit.
    bob_aim_applied: [f32; 2],
    bob_aim_pending: [f32; 2],

    /// bo2mp: through iron sights Black Ops II's server shoots along the
    /// gun (the view turned by the gun's own movement): how much of the
    /// gun's turn (pitch, yaw) is in the aim, what the next command still
    /// owes, and whether the gun placement measured it this frame (when it
    /// did not, the aim gives it back).
    gun_aim_applied: [f32; 2],
    gun_aim_pending: [f32; 2],
    gun_aim_set: bool,

    /// bo2mp: Sprint / Hold Breath while scoped.
    pub hold_breath: HoldBreath,

    /// bo2mp: a Variable Zoom scope zoomed in (melee while scoped toggles
    /// it; it opens wide every time he aims).
    pub vzoom_in: bool,
    /// The melee presses already taken (`LookState::zoom_presses`).
    vzoom_presses: u32,
    /// Scoped on a Variable Zoom gun, for the input to read.
    vzoom_scoped: bool,
    /// Aiming through a Hybrid Optic, for the input to read.
    optic_ready: bool,

    /// bo2mp: BO2MP_DEATHCAMLOG - what the view was made from last frame.
    death_cam_source: Option<&'static str>,

    /// bo2mp gun brain: BO2's own kick for a gun the T6 facts know (the shot's kick, the
    /// camera kick settling, the gun's recoil spring); `state` then shows this copy.
    t6_kick: weapon_t6::recoil::KickState,
    /// The random seed BO2's client draws the kick from.
    t6_seed: u32,
    /// The trigger has stayed down since the last shot: the gun has not left firing since it
    /// (a shot with it set builds up the ADS recoil reduction; the first shot of a burst has none).
    t6_held: bool,
    /// The gun's landing scale over the legacy one (a rangefinder aiming dips the gun less).
    t6_land_factor: Option<f32>,
    /// The aim fraction that landing scale was read at (for the BO2MP_SHOT_LOG land line).
    t6_land_frac: f32,
    /// BO2's gun sway, bob and drift, and the scope's drift on the camera (window 12).
    pub t6: T6Movement,
}

/// bo2mp: hold breath on a scoped sniper (Black Ops II's
/// `bHoldBreathToSteady` guns), by the rule of Black Ops' pmove
/// (`PM_UpdateHoldBreath`): one timer rises while the breath is held and
/// falls otherwise; the hold can only start when the timer is at zero; when
/// it passes the hold time the hold ends and the timer jumps to hold + gasp.
/// The drift scale follows its target by `DiffTrack` (a fraction of the gap
/// per second): 0 while held, otherwise 1 plus the gasp scale's excess times
/// how full the timer is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldBreath {
    /// The drift scale (1 normal, 0 held, above 1 gasping).
    pub scale: f32,
    /// `holdBreathTimer`, milliseconds.
    timer_ms: i32,
    /// The hold flag (`weapFlags & 4`).
    holding: bool,
}

impl Default for HoldBreath {
    fn default() -> Self {
        Self {
            scale: 1.0,
            timer_ms: 0,
            holding: false,
        }
    }
}

/// `player_breath_hold_time`: milliseconds of steady aim per breath.
pub const BREATH_HOLD_TIME_MS: i32 = 4500;
/// `player_breath_gasp_time`.
pub const BREATH_GASP_TIME_MS: i32 = 1000;
/// `player_breath_gasp_scale`: the drift at the top of the gasp.
pub const BREATH_GASP_SCALE: f32 = 4.5;
/// `player_breath_hold_lerp`: per second, while steadying.
pub const BREATH_HOLD_LERP: f32 = 1.0;
/// `player_breath_gasp_lerp`: per second, otherwise.
pub const BREATH_GASP_LERP: f32 = 6.0;

/// `DiffTrack`: a step of `rate * gap` per second, never past the target.
fn diff_track(target: f32, current: f32, rate: f32, dt_secs: f32) -> f32 {
    let gap = target - current;
    let step = rate * gap * dt_secs;
    if gap.abs() <= 0.001 || step.abs() > gap.abs() {
        target
    } else {
        current + step
    }
}

impl HoldBreath {
    /// One frame (`PM_UpdateHoldBreath`): `eligible` (a hold-breath scope),
    /// `pos_frac` (the aim fraction), `held` (the button), `dt_ms`. Returns
    /// the drift scale.
    pub fn advance(&mut self, eligible: bool, pos_frac: f32, held: bool, dt_ms: f32) -> f32 {
        let msec = dt_ms as i32;
        if eligible && pos_frac >= 1.0 && held {
            if self.timer_ms == 0 {
                self.holding = true;
            }
        } else {
            self.holding = false;
        }
        if self.holding {
            self.timer_ms += msec;
        } else {
            self.timer_ms -= msec;
        }
        self.timer_ms = self.timer_ms.max(0);
        if self.holding && self.timer_ms > BREATH_HOLD_TIME_MS {
            self.timer_ms = BREATH_GASP_TIME_MS + BREATH_HOLD_TIME_MS;
            self.holding = false;
        }
        let (target, rate) = if self.holding {
            (0.0, BREATH_HOLD_LERP)
        } else {
            let full = (BREATH_GASP_TIME_MS + BREATH_HOLD_TIME_MS) as f32;
            (
                (BREATH_GASP_SCALE - 1.0) * (self.timer_ms as f32 / full) + 1.0,
                BREATH_GASP_LERP,
            )
        };
        self.scale = diff_track(
            (target - 1.0) * pos_frac + 1.0,
            self.scale,
            rate,
            dt_ms / 1000.0,
        );
        self.scale
    }

    /// A shot at full aim (`PM_HoldBreathFire`): the hold ends (the fire
    /// delay, 0, adds nothing to the timer).
    pub fn fired(&mut self, pos_frac: f32, overlay_reticle: bool) {
        if pos_frac >= 1.0 && overlay_reticle {
            self.holding = false;
        }
    }
}

impl SessionViewKick {
    /// bo2mp: BO2MP_DEATHCAMLOG=1 writes one line each time the view changes
    /// what it is made from (alive, eye = his own eyes while dead, corpse =
    /// his body, spectator, killcam), with the time, so a glitch after a
    /// death can be matched to the moment the source changed.
    fn note_death_cam_source(&mut self, time_ms: i32, source: &'static str, pm_type: i32) {
        if self.death_cam_source == Some(source) {
            return;
        }
        if death_cam_log_on() {
            diag::info!(
                Fpv,
                "deathcam: t={} source={} was={} pm_type={}",
                time_ms,
                source,
                self.death_cam_source.unwrap_or("none"),
                pm_type
            );
        }
        self.death_cam_source = Some(source);
    }

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
        self.idle_aim_applied = [0.0; 2];
        self.idle_aim_pending = [0.0; 2];
        self.hit_aim_applied = 0.0;
        self.hit_aim_pending = 0.0;
        self.bob_aim_applied = [0.0; 2];
        self.bob_aim_pending = [0.0; 2];
        self.gun_aim_applied = [0.0; 2];
        self.gun_aim_pending = [0.0; 2];
        self.gun_aim_set = false;
        self.hold_breath = HoldBreath::default();
        self.vzoom_in = false;
        self.vzoom_scoped = false;
        self.optic_ready = false;
        self.t6.clear();
    }

    /// bo2mp: the gun's turn the aim should hold now (zero off iron
    /// sights); the change goes to the next command.
    pub(crate) fn set_gun_aim(&mut self, target: [f32; 2]) {
        for axis in 0..2 {
            self.gun_aim_pending[axis] += target[axis] - self.gun_aim_applied[axis];
            self.gun_aim_applied[axis] = target[axis];
        }
        self.gun_aim_set = true;
    }

    /// bo2mp: the part of the view angles that is the aim's own movement
    /// (the drift, the hit, the bob, the gun's turn). Black Ops II's sway
    /// follows the view and its kick only, so it must not chase these.
    fn aim_movement(&self) -> [f32; 2] {
        [
            self.idle_aim_applied[0]
                + self.hit_aim_applied
                + self.bob_aim_applied[0]
                + self.gun_aim_applied[0],
            self.idle_aim_applied[1] + self.bob_aim_applied[1] + self.gun_aim_applied[1],
        ]
    }
}

fn death_cam_log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_DEATHCAMLOG").is_ok_and(|v| v == "1"))
}

/// One frame's camera move over this many units is logged as a jump.
const DEATH_CAM_JUMP_UNITS: f32 = 100.0;

/// What `log_death_cam_jumps` saw since he died.
#[derive(Default)]
pub struct DeathCamWatch {
    last: Option<(Vec3, &'static str)>,
    frames: u32,
    jumps: u32,
    max_move: f32,
}

/// bo2mp: BO2MP_DEATHCAMLOG=1 also follows the camera every frame from his death to his
/// respawn: a move of over 100 units in one frame is logged with what the view was made
/// from before and after it, and each death ends with a count line (row 33: the death
/// camera flying across the map and back).
pub fn log_death_cam_jumps(
    clock: Res<FrameClock>,
    kick: Res<SessionViewKick>,
    q: Query<
        &Transform,
        (
            With<FlyCamera>,
            Without<RemotePlayer>,
            Without<WorldScriptModelInstance>,
        ),
    >,
    mut watch: Local<DeathCamWatch>,
) {
    if !death_cam_log_on() {
        return;
    }
    let Some(at) = q.iter().next().map(|t| t.translation) else {
        return;
    };
    let source = kick.death_cam_source.unwrap_or("none");
    let was = watch.last.map_or("none", |(_, s)| s);
    if source == "alive" && was == "alive" {
        watch.last = Some((at, source));
        return;
    }
    if let Some((before, _)) = watch.last {
        let moved = before.distance(at);
        if source != "alive" {
            watch.frames += 1;
            watch.max_move = watch.max_move.max(moved);
        }
        if moved > DEATH_CAM_JUMP_UNITS {
            watch.jumps += 1;
            diag::info!(
                Fpv,
                "deathcam jump: t={} moved={:.0} source={} was={} from=({:.0} {:.0} {:.0}) to=({:.0} {:.0} {:.0})",
                clock.time(),
                moved,
                source,
                was,
                before.x,
                before.y,
                before.z,
                at.x,
                at.y,
                at.z
            );
        }
    }
    if source == "alive" && was != "alive" && watch.frames > 0 {
        diag::info!(
            Fpv,
            "deathcam life: t={} frames={} jumps={} max_move={:.0}",
            clock.time(),
            watch.frames,
            watch.jumps,
            watch.max_move
        );
        watch.frames = 0;
        watch.jumps = 0;
        watch.max_move = 0.0;
    }
    watch.last = Some((at, source));
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
        kick.idle_aim_applied = [0.0; 2];
        kick.idle_aim_pending = [0.0; 2];
        kick.hold_breath = HoldBreath::default();
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
    // bo2mp: a hit's pitch, the walk bob and the iron-sight gun turn,
    // measured by the camera and the gun last frame. Sent before anything
    // can return early and after a switch to an older gun, so the aim gives
    // back exactly what they put in. A gun turn the placement did not
    // measure last frame is gone.
    if !kick.gun_aim_set {
        kick.set_gun_aim([0.0; 2]);
    }
    kick.gun_aim_set = false;
    if let Some(look) = look.as_mut() {
        look.kick_pending[0] +=
            kick.hit_aim_pending + kick.bob_aim_pending[0] + kick.gun_aim_pending[0];
        look.kick_pending[1] += kick.bob_aim_pending[1] + kick.gun_aim_pending[1];
    }
    kick.hit_aim_pending = 0.0;
    kick.bob_aim_pending = [0.0; 2];
    kick.gun_aim_pending = [0.0; 2];
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
        weapon_t6::recoil::gun_landing_scale(s, ps.f_weapon_pos_frac) / weapon_t6::view_bob::VIEWWEAPON_LAND_SCALE
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
            params = dual_wield_kick(
                params,
                &kick_params(&left_facts.kick),
                weapon_t6::state::is_any_firing(ps.weapon_state_t6),
                weapon_t6::state::is_any_firing(ps.weapon_state_left_t6),
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
        // bo2zm: BO2MP_SHOT_LOG=1: where the camera points against the aim.
        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *LOG.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some()) {
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
            // bo2mp: the scope's drift, measured by the camera last frame.
            look.kick_pending[axis] += kick.idle_aim_pending[axis];
            kick.idle_aim_pending[axis] = 0.0;
        }
    }
    // bo2mp: Variable Zoom: melee while scoped changes the zoom.
    if let Some(look) = look.as_mut() {
        look.melee_is_zoom = kick.vzoom_scoped;
        look.breath_is_switch = kick.optic_ready;
        if look.zoom_presses != kick.vzoom_presses {
            if kick.vzoom_scoped {
                kick.vzoom_in = !kick.vzoom_in;
            }
            kick.vzoom_presses = look.zoom_presses;
        }
    }

    // bo2mp: the sway follows the view and its kick, not the drift, hit, bob
    // and gun turn the aim carries for the server's shot.
    let sway_view = if kick.aim_kick {
        let m = kick.aim_movement();
        [ps.viewangles[0] - m[0], ps.viewangles[1] - m[1], ps.viewangles[2]]
    } else {
        ps.viewangles
    };
    // bo2mp gun brain: BO2's own sway for a gun the T6 facts know (the legacy sway below still runs
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
        kick.t6.tick_sway(viewmodel, ps.f_weapon_pos_frac, sway_view, dt, clock.frametime(), shock_left);
    }

    let overlay_active = facts.overlay_reticle != 0;
    if bo2_camera(&kick) && facts.aim_down_sight && overlay_active && ps.f_weapon_pos_frac > 0.0 {
        kick.sway.follow_view(sway_view);
        return;
    }
    let shock_sway = if bo2_camera(&kick) {
        shock_sway_scale(
            shellshock_remaining_ms(clock.time(), ps.shellshock_time, ps.shellshock_duration)
                .filter(|r| *r >= 1)
                .unwrap_or(0),
            ps.shellshock_duration,
            facts.sway.sway_shell_shock_scale,
        )
    } else {
        1.0
    };
    kick.sway.advance(
        facts.sway.hip_params(),
        facts.sway.ads_params(),
        sway_view,
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
    ps: &playerstate_t6::game::PlayerState,
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
    // One "firing" for every BO2 firing state, which is what the counter and the
    // dual-wield ranges ask.
    let firing = weapon_t6::state::is_any_firing;
    let as_t6 = |s: i32| {
        if firing(s) {
            weapon_t6::state::FIRING
        } else {
            weapon_t6::state::READY
        }
    };
    let (right_state, left_state) = (as_t6(ps.weapon_state_t6), as_t6(ps.weapon_state_left_t6));
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
        // BO2MP_SHOT_LOG=1: each shot's BO2 kick.
        static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *LOG.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some()) {
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
    if !firing(ps.weapon_state_t6) && !firing(ps.weapon_state_left_t6) {
        kick.t6_held = false;
    }
    let stance = if ps.pm_flags & playerstate_t6::game::pm_flags::PRONE != 0 {
        1
    } else if ps.pm_flags & playerstate_t6::game::pm_flags::CROUCH != 0 {
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
    (death_cam_clip, mut spring, slots, runtimes, look, mut old_corpse, mut shakes): (
        Res<crate::occupancy::dyn_ent::DynEntPhysClip>,
        Local<super::third_person::VehicleSpring>,
        Option<Res<net::CEntitySlots>>,
        Query<&net::CEntityRuntime>,
        Option<Res<net::LookState>>,
        Local<Option<i32>>,
        ResMut<super::camera_shake::CameraShakes>,
    ),
    corpse_root: Option<Res<crate::occupancy::t6_body::LocalCorpseRoot>>,
    script_models: Query<(&WorldScriptModelInstance, &Transform), Without<FlyCamera>>,
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
        let pose = earthquake_pose(pose, &presented, clock.time(), &mut shakes);
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        for mut projection in &mut lenses {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = vertical_fov_deg(fov).to_radians();
            }
        }
        kick.horiz_fov_deg = fov;
        kick.refdef_vieworg = pose.origin;
        kick.refdef_view_angles = pose.angles;
        kick.killcam_focus_distance = focus_distance;
        kick.note_death_cam_source(clock.time(), "killcam", ps.pm_type);
        return;
    }
    if let Some(pose) = remote_missile_camera(&presented, local.0, clock.time()) {
        let pose = earthquake_pose(pose, &presented, clock.time(), &mut shakes);
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        let horiz = MISSILE_CAM_FOV;
        if let Some(actions) = actions.as_deref_mut() {
            actions.fov_scale = zoom_sensitivity(horiz, CG_FOV_DEFAULT) * actions.shellshock_look_scale;
        }
        let vertical = vertical_fov_deg(horiz).to_radians();
        for mut projection in lenses.iter_mut() {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = vertical;
            }
        }
        kick.horiz_fov_deg = horiz;
        return;
    }
    // bo2mp: riding a scorestreak vehicle - his view rides its model.
    if let Some(vehicle) = super::third_person::vehicle_view(&presented, local.0)
        && let Some((owner, at)) = script_models
            .iter()
            .find(|(m, _)| m.id.source_ordinal() == vehicle.model)
    {
        // The car where it is drawn this frame (its Transform is placed
        // after the camera, so it holds last frame's place).
        let mut at = *at;
        if let Some((origin, [pitch, yaw, roll])) = owner.gentity_number.and_then(|n| {
            super::script_model::script_mover_pose_now(slots.as_deref(), &presented, &runtimes, &clock, n)
        }) {
            at.translation = Vec3::from_array(origin);
            at.rotation = Quat::from_euler(
                EulerRot::ZYX,
                yaw.to_radians(),
                pitch.to_radians(),
                roll.to_radians(),
            );
        }
        let pushed = if vehicle.first_person {
            0.0
        } else {
            spring.step(&vehicle, at.translation, at.rotation, clock.frametime_secs())
        };
        let pose = super::third_person::vehicle_camera(
            &presented,
            local.0,
            vehicle,
            at.translation.to_array(),
            at.rotation,
            death_cam_clip.0.as_deref(),
            pushed,
            look.as_deref().map(|l| l.angles),
        );
        let pose = earthquake_pose(pose, &presented, clock.time(), &mut shakes);
        let eye = transform_from_iw_view(pose);
        for mut transform in &mut q {
            transform.translation = eye.translation;
            transform.rotation = eye.rotation;
        }
        // Its own field of view (the RC-XD's 90); a view linked to the gun
        // in his hands (the Lodestar's) zooms with it; else his.
        let held = ps.weapon;
        let linked = linked_weapon_view_fov(
            ps.link_flags,
            held,
            ps.f_weapon_pos_frac,
            weapons
                .as_ref()
                .and_then(|w| w.0.facts_of(held))
                .filter(|f| f.body_resolved)
                .map_or(0.0, |f| f.ads_zoom_fov),
        );
        let fov = if vehicle.fov > 0.0 {
            Some(vehicle.fov)
        } else {
            linked
        };
        if let Some(fov) = fov {
            if linked.is_some()
                && let Some(actions) = actions.as_deref_mut()
            {
                actions.fov_scale = zoom_sensitivity(fov, CG_FOV_DEFAULT) * actions.shellshock_look_scale;
            }
            let vertical = vertical_fov_deg(fov).to_radians();
            for mut projection in lenses.iter_mut() {
                if let Projection::Perspective(perspective) = &mut *projection {
                    perspective.fov = vertical;
                }
            }
            kick.horiz_fov_deg = fov;
        }
        return;
    }
    // His body from this death: one that showed while he was still alive
    // is an earlier death's (the new body shows a frame or two after he
    // dies, and the camera sat on the old one across the map: row 33).
    let corpse = corpse_root
        .as_ref()
        .and_then(|r| r.0)
        .filter(|(_, since)| old_corpse.is_none_or(|old| *since > old))
        .map(|(root, _)| root);
    if presented_is_third_person(&presented, local.0, view.in_killcam()) {
        let Some(pose) = death_watch_camera(&presented, local.0, death_cam_clip.0.as_deref(), corpse)
        else {
            // No collision to pull the camera back against: it holds.
            kick.note_death_cam_source(clock.time(), "frozen", ps.pm_type);
            return;
        };
        let on_body = ps.pm_type >= 10 && corpse.is_some();
        kick.note_death_cam_source(clock.time(), if on_body { "corpse" } else { "eye" }, ps.pm_type);
        let pose = earthquake_pose(pose, &presented, clock.time(), &mut shakes);
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
    if ps.pm_type < playerstate_t6::game::PM_TYPE_SPECTATOR {
        *old_corpse = corpse_root.as_ref().and_then(|r| r.0).map(|(_, since)| since);
    }
    let first_person_source = if ps.pm_type == playerstate_t6::game::PM_TYPE_SPECTATOR {
        "spectator"
    } else {
        "alive"
    };
    kick.note_death_cam_source(clock.time(), first_person_source, ps.pm_type);
    let offset = presented.view_offset();
    let xyspeed = {
        let vx = ps.velocity[0];
        let vy = ps.velocity[1];
        game_math::vec3_length([vx, vy, 0.0])
    };
    let org = ViewOrgBobInputs {
        bob_cycle: (ps.bob_cycle as u32 & 0xff) as u8,
        xyspeed,
        view_height_target: ps.view_height_target,
        pm_flags: ps.pm_flags,
        weapon_pos_frac: ps.f_weapon_pos_frac,
        perks0: ps.perks[0],
        bo2: bo2_camera(&kick),
        waterlevel: ps.waterlevel,
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
    let bob_in = match weapons.as_ref().and_then(|w| w.0.facts_of(viewmodel)) {
        Some(facts) if facts.body_resolved => Some(ViewAngleBobInputs {
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
            hold_breath_scale: {
                let held = actions
                    .as_deref()
                    .is_some_and(|a| a.client.kb.holdbreath.active);
                let full = facts.overlay_reticle != 0 && ps.f_weapon_pos_frac >= 1.0;
                // Variable Zoom: melee zooms (see tick_session_view_kick).
                kick.vzoom_scoped = facts.variable_zoom_fovs[0] > 0.0 && full;
                kick.optic_ready = facts.optic_switch && ps.f_weapon_pos_frac >= 1.0;
                if ps.f_weapon_pos_frac <= 0.0 {
                    kick.vzoom_in = false;
                }
                if kick.seeded_this_frame > 0 {
                    kick.hold_breath
                        .fired(ps.f_weapon_pos_frac, facts.overlay_reticle != 0);
                }
                kick.hold_breath.advance(
                    facts.hold_breath_to_steady && facts.overlay_reticle != 0,
                    ps.f_weapon_pos_frac,
                    held,
                    clock.frametime() as f32,
                )
            },
            weap_idle_time: kick.weap_idle_time,
            view_last_idle_factor: kick.view_last_idle_factor,
        }),
        _ => None,
    };
    let bob_angles = match bob_in {
        Some(inputs) => view_angle_bob(inputs),
        None => view_damage_angles(ViewAngleBobInputs {
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
    // bo2mp: a Black Ops II scope's drift is in the aim (queued for the next
    // command, as the kick is), so the camera does not add it again: the
    // bullet goes where the scope's crosshair is.
    let mut bob_angles = bob_angles;
    // bo2mp gun brain: a T6 gun's scope drift by BO2's rules in place of the legacy one.
    if bo2_camera(&kick) && kick.t6.facts.is_some() {
        let hb = kick.hold_breath.scale;
        let idle = kick.t6.camera(
            ps.f_weapon_pos_frac,
            ps.e_flags,
            ps.pm_flags,
            clock.frametime_secs(),
            hb,
        );
        bob_angles.pitch += idle[0] - bob_angles.cam_idle_pitch;
        bob_angles.yaw += idle[1] - bob_angles.cam_idle_yaw;
        bob_angles.cam_idle_pitch = idle[0];
        bob_angles.cam_idle_yaw = idle[1];
    }
    // bo2mp gun brain: a T6 gun's walk bob by BO2's rules in place of the
    // legacy one (the scope's bob reads the weapon's own bob factors).
    let t6_path = bo2_camera(&kick) && kick.t6.facts.is_some();
    let iw_walk = bob_in.map_or([0.0; 3], view_walk_bob_angles);
    let walk = if t6_path {
        let walk = kick.t6.view_bob(&ViewBobInput {
            frac: ps.f_weapon_pos_frac,
            e_flags: ps.e_flags,
            pm_flags: ps.pm_flags,
            bob_cycle: ps.bob_cycle as i32,
            xyspeed,
            view_height_target: ps.view_height_target,
        });
        bob_angles.pitch += walk[0] - iw_walk[0];
        bob_angles.yaw += walk[1] - iw_walk[1];
        bob_angles.roll += walk[2] - iw_walk[2];
        walk
    } else {
        iw_walk
    };
    if kick.aim_kick {
        let idle = [bob_angles.cam_idle_pitch, bob_angles.cam_idle_yaw];
        for axis in 0..2 {
            kick.idle_aim_pending[axis] += idle[axis] - kick.idle_aim_applied[axis];
            kick.idle_aim_applied[axis] = idle[axis];
        }
        bob_angles.pitch -= idle[0];
        bob_angles.yaw -= idle[1];
    }
    // bo2mp: Black Ops II's server turns the shot by the view's walk bob as
    // well (its view movement: the scope's bob and the ADS bob), so the aim
    // takes it, queued as the drift is, and the camera stops adding it.
    let bob_aim = if kick.aim_kick {
        [walk[0], walk[1]]
    } else {
        [0.0; 2]
    };
    for axis in 0..2 {
        kick.bob_aim_pending[axis] += bob_aim[axis] - kick.bob_aim_applied[axis];
        kick.bob_aim_applied[axis] = bob_aim[axis];
    }
    bob_angles.pitch -= bob_aim[0];
    bob_angles.yaw -= bob_aim[1];
    // bo2mp: the iron-sight gun turn the gun placement put in the aim; the
    // camera does not turn with the gun.
    bob_angles.pitch -= kick.gun_aim_applied[0];
    bob_angles.yaw -= kick.gun_aim_applied[1];
    // bo2mp: Black Ops II's server turns the shot by the hit kick
    // (bg_useClientDamageKick), so the bullets move with the flinch: the
    // aim takes it (queued, as the kick is) and the camera stops adding it.
    // An older gun keeps the camera-only flinch; the aim gives its share back.
    let hit = if kick.aim_kick {
        let (overlay, ads) = weapons
            .as_ref()
            .and_then(|w| w.0.facts_of(viewmodel))
            .filter(|f| f.body_resolved)
            .map_or((0, false), |f| (f.overlay_reticle, f.aim_down_sight));
        // A T6 gun's own hit tilt is part of the iron-sight gun turn.
        hit_aim_pitch(
            clock.time(),
            kick.damage_time,
            kick.v_dmg_pitch,
            kick.v_dmg_roll,
            ps.f_weapon_pos_frac,
            overlay,
            ads && !t6_path,
        )
    } else {
        0.0
    };
    kick.hit_aim_pending += hit - kick.hit_aim_applied;
    kick.hit_aim_applied = hit;
    bob_angles.pitch -= hit;
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
        kick_angles[1] + bob_angles.yaw,
        kick_angles[2] + bob_angles.roll,
    ];
    kick.refdef_view_angles = angles;
    let mut origin = [
        ps.origin[0] + offset[0],
        ps.origin[1] + offset[1],
        ps.origin[2] + offset[2] + ps.view_height_current,
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
    let min_z = ps.origin[2] + offset[2] + VIEW_ORG_BOB_Z_MIN_OFS;
    if origin[2] < min_z {
        origin[2] = min_z;
    }
    let pose = earthquake_pose(
        WorldCameraPose { origin, angles },
        &presented,
        clock.time(),
        &mut shakes,
    );
    kick.refdef_vieworg = pose.origin;
    kick.refdef_view_angles = pose.angles;
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
        weapons.as_ref().and_then(|w| w.0.facts_of(viewmodel)).map(|mut f| {
            // bo2mp: a Variable Zoom scope zoomed in.
            if kick.vzoom_in && f.variable_zoom_fovs[1] > 0.0 {
                f.ads_zoom_fov = f.variable_zoom_fovs[1];
            }
            f
        }),
        kick.b_position_to_ads,
        actions.as_deref_mut(),
    ) {
        kick.horiz_fov_deg = horiz;
    }
}

/// BO2's fixed field of view for a view linked to the gun in his hands.
const LINKED_WEAPON_VIEW_FOV: f32 = 45.0;

/// BO2: a view linked to the gun in his hands (`playerlinkweaponviewtodelta`,
/// link flag 4; the Lodestar's drone view) is a fixed 45 degrees, and the
/// gun's own ADS zoom (the Lodestar's 10) as soon as he starts to aim: no
/// blend, no fov scale. None when the view is not linked to a gun.
fn linked_weapon_view_fov(
    link_flags: u32,
    weapon: u32,
    f_weapon_pos_frac: f32,
    ads_zoom_fov: f32,
) -> Option<f32> {
    if (link_flags & LINK_FLAG_FORCE_ADS_ZOOM) == 0 || weapon == 0 {
        return None;
    }
    Some(if f_weapon_pos_frac > 0.0 && ads_zoom_fov > 0.0 {
        ads_zoom_fov
    } else {
        LINKED_WEAPON_VIEW_FOV
    })
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
    let ads_target = if facts.aim_down_sight && facts.ads_zoom_fov > 0.0 {
        facts.ads_zoom_fov
    } else {
        base_fov
    };
    let inputs = ViewFovInputs {
        cg_fov: base_fov,
        default_view: pm_type == playerstate_t6::game::PM_TYPE_INTERMISSION,
        link_flags,
        e_flags,
        has_weapon: viewmodel != 0,
        zoom: AdsZoom {
            fov: [ads_target; 3],
            zoom_in_frac: facts.ads_zoom_in_frac,
            zoom_out_frac: facts.ads_zoom_out_frac,
        },
        pos_frac: f_weapon_pos_frac,
        to_ads: b_position_to_ads,
        ..ViewFovInputs::default()
    };
    let horiz = view_fov(&inputs);
    let zoom_sensitivity = zoom_sensitivity(horiz, CG_FOV_DEFAULT);
    if let Some(actions) = actions {
        actions.fov_scale = zoom_sensitivity * actions.shellshock_look_scale;
    }
    let vertical = vertical_fov_deg(horiz).to_radians();
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
                // BO2MP_SHOT_LOG=1: each T6 gun landing (the gun dips by the dip times its scale).
                static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if let Some(factor) = kick.t6_land_factor
                    && *LOG.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
                {
                    diag::info!(
                        Fpv,
                        "t6 land: fall {:.1} dip {} gun scale {:.3} frac {:.2}",
                        fall,
                        dip,
                        factor * weapon_t6::view_bob::VIEWWEAPON_LAND_SCALE,
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
    let bullet_flinch = (perks0 & weapon_t6::view_bob::PERK_BULLETFLINCH) != 0;
    let punch = |yaw: u32, pitch: u32, count: i32| {
        if bo2 {
            weapon_t6::view_bob::damage_feedback_kick_bo2(yaw, pitch, count, viewangles, bullet_flinch)
        } else {
            Some(weapon_t6::view_bob::damage_feedback_kick(yaw, pitch, count, viewangles))
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
pub(crate) fn bo2_camera(kick: &SessionViewKick) -> bool {
    kick.aim_kick
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

/// The view shaken by the server's earthquakes, then (bo2mp) by Black Ops
/// II's camera shakes.
fn earthquake_pose(
    mut pose: WorldCameraPose,
    presented: &PresentedSnapshot,
    now_ms: i32,
    shakes: &mut super::camera_shake::CameraShakes,
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
    shakes.apply(&mut pose.angles, now_ms, pose.origin);
    pose
}

/// bo2mp: the pitch a hit adds to Black Ops II's shot. Its server builds the
/// gun's aim from the view plus the camera's hit kick, always; while the gun
/// is aimed through iron sights (not a scope), the gun model's own hit tilt
/// is added on top (sent by the client; its yaw is dropped there). Both are
/// the same curves the camera and the gun model already draw.
fn hit_aim_pitch(
    time: i32,
    damage_time: i32,
    v_dmg_pitch: f32,
    v_dmg_roll: f32,
    frac: f32,
    overlay_reticle: i32,
    aim_down_sight: bool,
) -> f32 {
    let camera = view_damage_angles(ViewAngleBobInputs {
        org: ViewOrgBobInputs {
            weapon_pos_frac: frac,
            ..ViewOrgBobInputs::default()
        },
        overlay_reticle,
        time,
        damage_time,
        v_dmg_pitch,
        v_dmg_roll,
        ..ViewAngleBobInputs::default()
    })
    .pitch;
    let iron_sights = aim_down_sight && frac != 0.0 && overlay_reticle == 0;
    let gun = if iron_sights {
        weapon_damage_kick_angles(time, damage_time, v_dmg_pitch, v_dmg_roll, frac, 0)[0]
    } else {
        0.0
    };
    camera + gun
}

/// bo2mp: how far (pitch, yaw) Black Ops II's iron-sight shot turns off
/// the view: the gun's own movement angles turned into the view's with
/// BO2's axes (forward, left, up), less the view
/// (CG_CalculateWeaponMovement_SaveAngles).
pub(crate) fn iron_gun_turn(gun_angles: [f32; 3], view: [f32; 3]) -> [f32; 2] {
    let axis = game_math::matrix_multiply(
        game_math::angles_to_axis(gun_angles),
        game_math::angles_to_axis(view),
    );
    let [pitch, yaw, _] = game_math::axis_to_angles(axis);
    [
        weapon_t6::recoil::angle_wrap180(pitch - view[0]),
        weapon_t6::recoil::angle_wrap180(yaw - view[1]),
    ]
}

#[cfg(test)]
mod iron_gun_turn_tests {
    use super::iron_gun_turn;

    #[test]
    fn a_level_view_takes_the_gun_turn_as_it_is() {
        let t = iron_gun_turn([0.0, 1.5, 0.0], [0.0, 30.0, 0.0]);
        assert!((t[0]).abs() < 1e-3 && (t[1] - 1.5).abs() < 1e-3, "{t:?}");
        let t = iron_gun_turn([-0.8, 0.0, 0.0], [0.0, 350.0, 0.0]);
        assert!((t[0] + 0.8).abs() < 1e-3 && t[1].abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn a_still_gun_adds_nothing_wherever_he_looks() {
        for view in [[0.0, 0.0, 0.0], [-40.0, 179.0, 0.0], [60.0, -90.0, 3.0]] {
            let t = iron_gun_turn([0.0; 3], view);
            assert!(t[0].abs() < 1e-3 && t[1].abs() < 1e-3, "{view:?} {t:?}");
        }
    }

    #[test]
    fn a_rolled_view_turns_the_gun_pitch_sideways() {
        // The gun pitched down 2 in a view rolled 90: the shot turns to the side.
        let t = iron_gun_turn([2.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
        assert!(t[0].abs() < 0.05 && (t[1].abs() - 2.0).abs() < 0.05, "{t:?}");
    }
}

#[cfg(test)]
mod hit_aim_tests {
    use super::hit_aim_pitch;

    const HIT: i32 = 1000;

    fn at(ms: i32, frac: f32, overlay: i32, ads: bool) -> f32 {
        hit_aim_pitch(HIT + ms, HIT, 5.0, 2.0, frac, overlay, ads)
    }

    #[test]
    fn from_the_hip_the_shot_takes_the_camera_flinch() {
        let peak = at(100, 0.0, 0, true);
        eprintln!("hip flinch in the aim at 100 ms: {peak:.2}");
        assert!((peak - 5.0).abs() < 1e-4);
        assert_eq!(hit_aim_pitch(HIT + 100, 0, 5.0, 2.0, 0.0, 0, true), 0.0);
    }

    #[test]
    fn iron_sights_add_the_gun_tilt_and_a_scope_does_not() {
        let iron = at(100, 1.0, 0, true);
        let no_ads = at(100, 1.0, 0, false);
        let scope = at(100, 1.0, 1, true);
        eprintln!("aimed in at 100 ms: iron {iron:.2} no-ads {no_ads:.2} scope {scope:.2}");
        // camera half (factor 0.5) + gun tilt (5 * 0.5)
        assert!((iron - 5.0).abs() < 1e-4);
        assert!((no_ads - 2.5).abs() < 1e-4);
        // a scope: camera only, its factor (0.5 * 1.5)
        assert!((scope - 3.75).abs() < 1e-4);
    }

    #[test]
    fn the_flinch_leaves_the_aim_after_half_a_second() {
        for frac in [0.0, 0.5, 1.0] {
            assert!(at(50, frac, 0, true) > 0.0);
            assert_eq!(at(500, frac, 0, true), 0.0);
            assert_eq!(at(700, frac, 0, true), 0.0);
        }
    }
}

#[cfg(test)]
mod linked_weapon_view_tests {
    use super::linked_weapon_view_fov;

    #[test]
    fn the_lodestar_view_snaps_to_its_zoom_when_he_aims() {
        // Not linked to a gun: his own view.
        assert_eq!(linked_weapon_view_fov(0, 233, 1.0, 10.0), None);
        assert_eq!(linked_weapon_view_fov(4, 0, 1.0, 10.0), None);
        // Linked: 45 at rest, the gun's 10 from the first bit of aim.
        assert_eq!(linked_weapon_view_fov(4, 233, 0.0, 10.0), Some(45.0));
        assert_eq!(linked_weapon_view_fov(4 | 1, 233, 0.01, 10.0), Some(10.0));
        assert_eq!(linked_weapon_view_fov(4, 233, 1.0, 10.0), Some(10.0));
        // A gun with no ADS zoom stays at 45.
        assert_eq!(linked_weapon_view_fov(4, 233, 1.0, 0.0), Some(45.0));
    }
}

#[cfg(test)]
mod hold_breath_tests {
    use super::*;

    fn run(b: &mut HoldBreath, held: bool, ms: i32) -> f32 {
        let mut s = 1.0;
        for _ in 0..ms / 8 {
            s = b.advance(true, 1.0, held, 8.0);
        }
        s
    }

    #[test]
    fn holding_steadies_then_gasps_and_blocks_a_restart() {
        let mut b = HoldBreath::default();
        assert!(run(&mut b, true, 3000) < 0.1);
        // Past the hold time: the hold ends and the drift gasps above 1.
        run(&mut b, true, 2000);
        assert!(!b.holding);
        assert!(b.scale > 1.5);
        // Still held, but the timer is not at zero: no new hold.
        run(&mut b, true, 200);
        assert!(!b.holding);
        // Let go for the whole gasp + hold time: back to normal, can hold.
        run(&mut b, false, 6000);
        assert!((b.scale - 1.0).abs() < 0.01);
        run(&mut b, true, 100);
        assert!(b.holding);
    }

    #[test]
    fn a_shot_ends_the_hold() {
        let mut b = HoldBreath::default();
        run(&mut b, true, 1000);
        assert!(b.holding);
        b.fired(1.0, true);
        assert!(!b.holding);
    }
}
