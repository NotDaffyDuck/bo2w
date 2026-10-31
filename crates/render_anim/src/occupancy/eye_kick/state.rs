//! The local player's view-kick memory: everything the camera, the aim and the gun
//! placement carry from one frame to the next.

use bevy::prelude::*;

use super::breath::HoldBreath;
use crate::motion::kick_state::ViewRecoilState;
use crate::motion::gun_sway::SwaySpring;
use crate::occupancy::t6_movement::T6Movement;

/// How many hurt flashes the view still owes a flinch, for damage that comes
/// with no direction (the bots' hits and the like).
#[derive(Resource, Default)]
pub struct QueuedHurtFlinch(pub u32);

/// Moves a carried pair toward a new target: whatever differs from what is
/// already carried is queued as owed to the next command.
pub(super) fn retarget<const N: usize>(carried: &mut [f32; N], owed: &mut [f32; N], target_in: [f32; N]) {
    for axis in 0..N {
        owed[axis] += target_in[axis] - carried[axis];
        carried[axis] = target_in[axis];
    }
}

#[derive(Resource, Default)]
pub struct CameraRecoilState {
    /// The gun's kick as the camera, the aim and the gun placement read it.
    pub state: ViewRecoilState,
    /// The gun's sway as the view turns.
    pub sway: SwaySpring,

    /// Where the gun placement last moved the gun (offset from rest).
    pub placement_shift: [f32; 3],
    /// How the gun placement last turned the gun.
    pub placement_turn: [f32; 3],
    /// The scope drift's clock, carried to the next frame.
    pub gun_idle_clock: i32,
    /// The gun's idle sway strength last frame.
    pub previous_idle_ratio: f32,
    /// The camera's idle sway strength last frame.
    pub view_prev_sway_ratio: f32,

    /// How far the last landing pushed the view (negative is down).
    pub touchdown_dip: f32,
    /// When that landing happened, in client time.
    pub touchdown_stamp: i32,
    /// The size of that landing's dip, in whole units.
    pub landing_dip: i32,
    /// The viewmodel's vertical landing offset this frame.
    pub land_z_viewweapon: f32,
    /// The same offset spread over the view's forward, right and up axes.
    pub gun_landing_dip: [f32; 3],

    /// The angles the camera was drawn with this frame.
    pub frame_look_angles: [f32; 3],
    /// The place the camera was drawn from this frame.
    pub frame_eye_position: [f32; 3],
    /// The horizontal field of view the camera was drawn with.
    pub horizontal_view_degrees: f32,
    /// During a killcam, how far away the thing in focus is.
    pub killcam_focus_distance: Option<f32>,

    /// When the last hit flinch started (0 for none).
    pub hurt_stamp_ms: i32,
    /// That flinch's pitch.
    pub hurt_kick_pitch: f32,
    /// That flinch's roll.
    pub hurt_kick_roll: f32,

    // What the previous frame looked like, to tell what changed.
    pub(super) last_weapon_id: u32,
    pub(super) prev_origin: [f32; 3],
    pub(super) prev_velocity: [f32; 3],
    pub(super) prev_standing_on: i32,
    pub(super) land_seen: bool,
    pub(super) prev_damage_event: u32,
    pub(super) damage_seen: bool,

    /// How many shots this frame fed the kick.
    pub freshly_primed_count: u32,
    pub(super) last_weapon_pos_frac: f32,
    /// True while the gun is moving toward the sights, false while it comes down.
    pub snap_to_sights: bool,

    /// bo2zm: the held gun is Black Ops II's, whose view kick moves the
    /// aim itself (bullets follow the sights), not just the camera.
    pub aim_kick: bool,

    /// bo2zm: how much of the kick (pitch, yaw) is already in the aim.
    pub(super) aim_applied: [f32; 2],

    /// bo2mp: a scoped Black Ops II gun's drift (its ADS idle) moves the
    /// aim as well, so the bullet goes to the scope's crosshair: how much
    /// of it is already in the aim, and what the next command still owes.
    pub(super) idle_aim_applied: [f32; 2],
    pub(super) idle_aim_pending: [f32; 2],

    /// bo2mp: a hit moves the aim the way Black Ops II's server moves the
    /// shot (`hit_aim_pitch`): how much pitch is in the aim, and what the
    /// next command still owes. Kept over a weapon switch, so the aim gives
    /// back exactly what the hit put in.
    pub(super) hit_aim_applied: f32,
    pub(super) hit_aim_pending: f32,

    /// bo2mp: Black Ops II's server turns the shot by the view's walk bob
    /// (the scope's bob and the ADS bob), so the aim takes it as it takes
    /// the drift: how much (pitch, yaw) is in the aim, and what the next
    /// command still owes. Kept over a weapon switch, like the hit.
    pub(super) bob_aim_applied: [f32; 2],
    pub(super) bob_aim_pending: [f32; 2],

    /// bo2mp: through iron sights Black Ops II's server shoots along the
    /// gun (the view turned by the gun's own movement): how much of the
    /// gun's turn (pitch, yaw) is in the aim, what the next command still
    /// owes, and whether the gun placement measured it this frame (when it
    /// did not, the aim gives it back).
    pub(super) gun_aim_applied: [f32; 2],
    pub(super) gun_aim_pending: [f32; 2],
    pub(super) gun_aim_set: bool,

    /// bo2mp: Sprint / Hold Breath while scoped.
    pub hold_breath: HoldBreath,

    /// bo2mp: a Variable Zoom scope zoomed in (melee while scoped toggles
    /// it; it opens wide every time he aims).
    pub vzoom_in: bool,
    /// The melee presses already taken (`ViewAimAccumulator::zoom_presses`).
    pub(super) vzoom_presses: u32,
    /// Scoped on a Variable Zoom gun, for the input to read.
    pub(super) vzoom_scoped: bool,
    /// Aiming through a Hybrid Optic, for the input to read.
    pub(super) optic_ready: bool,

    /// bo2mp: BO2MP_DEATHCAMLOG - what the view was made from last frame.
    pub(super) death_cam_source: Option<&'static str>,

    /// bo2mp gun brain: BO2's own kick for a gun the T6 facts know (the shot's kick, the
    /// camera kick settling, the gun's recoil spring); `state` then shows this copy.
    pub(super) t6_kick: weapon_t6::recoil::KickState,
    /// The random seed BO2's client draws the kick from.
    pub(super) t6_seed: u32,
    /// The trigger has stayed down since the last shot: the gun has not left firing since it
    /// (a shot with it set builds up the ADS recoil reduction; the first shot of a burst has none).
    pub(super) t6_held: bool,
    /// The gun's landing scale over the legacy one (a rangefinder aiming dips the gun less).
    pub(super) t6_land_factor: Option<f32>,
    /// The aim fraction that landing scale was read at (for the BO2MP_SHOT_LOG land line).
    pub(super) t6_land_frac: f32,
    /// BO2's gun sway, bob and drift, and the scope's drift on the camera (window 12).
    pub t6: T6Movement,
}

/// Whether the held gun is a Black Ops II one, so the Black Ops II camera
/// rules apply (bo2zm).
pub(crate) fn bo2_camera(kick: &CameraRecoilState) -> bool {
    kick.aim_kick
}

impl CameraRecoilState {
    /// Wipes the gun placement's carried movement and idle strengths.
    fn forget_gun_placement(&mut self) {
        (self.placement_shift, self.placement_turn) = ([0.0; 3], [0.0; 3]);
        self.gun_idle_clock = 0;
        (self.previous_idle_ratio, self.view_prev_sway_ratio) = (0.0, 0.0);
    }

    /// Wipes what the landing dip and the fall detection remember.
    fn forget_landing(&mut self) {
        (self.touchdown_dip, self.land_z_viewweapon) = (0.0, 0.0);
        (self.touchdown_stamp, self.landing_dip) = (0, 0);
        self.gun_landing_dip = [0.0; 3];
        (self.prev_origin, self.prev_velocity) = ([0.0; 3], [0.0; 3]);
        (self.land_seen, self.prev_standing_on) = (false, 0);
    }

    /// Wipes the hit flinch and the damage event it was told by.
    fn forget_hit(&mut self) {
        (self.hurt_kick_pitch, self.hurt_kick_roll) = (0.0, 0.0);
        (self.hurt_stamp_ms, self.prev_damage_event) = (0, 0);
        self.damage_seen = false;
    }

    /// Drops the kick and the scope drift from what the aim carries. (A
    /// weapon switch only does this much.)
    fn forget_kick_aim(&mut self) {
        self.aim_applied = [0.0; 2];
        self.idle_aim_applied = [0.0; 2];
        self.idle_aim_pending = [0.0; 2];
    }

    /// Drops the hit, the walk bob and the gun turn from what the aim carries.
    fn forget_other_aim(&mut self) {
        (self.hit_aim_applied, self.hit_aim_pending) = (0.0, 0.0);
        for pair in [
            &mut self.bob_aim_applied,
            &mut self.bob_aim_pending,
            &mut self.gun_aim_applied,
            &mut self.gun_aim_pending,
        ] {
            *pair = [0.0; 2];
        }
        self.gun_aim_set = false;
    }

    /// A new life starts from a calm view: nothing carried over except what
    /// the next frame sets anyway (the gun in hand, the fov, the camera pose).
    pub fn reset_on_respawn(&mut self) {
        ViewRecoilState::restart(&mut self.state);
        SwaySpring::settle_zero(&mut self.sway);
        self.forget_gun_placement();
        self.forget_landing();
        self.forget_hit();
        self.forget_kick_aim();
        self.forget_other_aim();
        (self.freshly_primed_count, self.last_weapon_pos_frac) = (0, 0.0);
        self.snap_to_sights = true;
        self.hold_breath = HoldBreath::default();
        (self.vzoom_in, self.vzoom_scoped, self.optic_ready) = (false, false, false);
        self.t6.clear();
    }

    /// Switching to another gun starts its kick, sway and placement afresh.
    /// The aim keeps the kick it has (only new kick moves it), and keeps
    /// the hit, the walk bob and the gun turn, so it can give them back.
    pub(super) fn on_weapon_changed(&mut self, weapon_idx: u32) {
        ViewRecoilState::restart(&mut self.state);
        SwaySpring::settle_zero(&mut self.sway);
        self.t6_kick = Default::default();
        self.t6_held = false;
        self.forget_kick_aim();
        self.forget_gun_placement();
        self.hold_breath = HoldBreath::default();
        (self.freshly_primed_count, self.last_weapon_pos_frac) = (0, 0.0);
        self.last_weapon_id = weapon_idx;
        self.snap_to_sights = true;
        self.t6.weapon_changed();
    }

    /// bo2mp: the gun's turn the aim should hold now (zero off iron
    /// sights); the change goes to the next command.
    pub(crate) fn set_gun_aim(&mut self, tgt: [f32; 2]) {
        retarget(&mut self.gun_aim_applied, &mut self.gun_aim_pending, tgt);
        self.gun_aim_set = true;
    }

    /// bo2mp: the part of the view angles that is the aim's own movement
    /// (the drift, the hit, the bob, the gun's turn). Black Ops II's sway
    /// follows the view and its kick only, so it must not chase these.
    pub(super) fn aim_movement(&self) -> [f32; 2] {
        let [idle_pitch, idle_yaw] = self.idle_aim_applied;
        let [bob_pitch, bob_yaw] = self.bob_aim_applied;
        let [weapon_tilt_angle, weapon_swing_angle] = self.gun_aim_applied;
        [
            idle_pitch + self.hit_aim_applied + bob_pitch + weapon_tilt_angle,
            idle_yaw + bob_yaw + weapon_swing_angle,
        ]
    }

    /// Hands over what the aim is owed (pitch, yaw) by the hit, the walk bob
    /// and the iron-sight gun turn measured last frame, and zeroes it. A gun
    /// turn the placement did not measure last frame is gone: the aim gives
    /// it back.
    pub(super) fn take_aim_debts(&mut self) -> [f32; 2] {
        if !self.gun_aim_set {
            self.set_gun_aim([0.0; 2]);
        }
        self.gun_aim_set = false;
        let [bob_pitch, bob_yaw] = std::mem::take(&mut self.bob_aim_pending);
        let [weapon_tilt_angle, weapon_swing_angle] = std::mem::take(&mut self.gun_aim_pending);
        let hit_pitch = std::mem::take(&mut self.hit_aim_pending);
        [hit_pitch + bob_pitch + weapon_tilt_angle, bob_yaw + weapon_swing_angle]
    }
}
