//! bo2mp gun brain (window 12): Black Ops II's gun movement on the client, for a gun the T6 facts
//! know: the gun's sway, its walk bob and its slow drift (idle), and the scope's drift on the
//! camera. The math is `weapon_t6::recoil`'s; this keeps the per-player state and feeds it.
//!
//! BO2MP_T6_MOVE_LOG=1 prints one "t6 sway:", "t6 cam:" and "t6 gun:" line per frame (and a
//! "t6 move facts:" line per weapon) for docs/gun/check_move.py.
//!
//! BO2MP_T6_BOB_LOG=1 is the light one for a whole play session: one short "t6 bob:" line per
//! frame, only while the walk cycle moves (the cycle, the speed, sprint, the gun's bob angles
//! and its final origin). Nothing on screen.

use std::sync::OnceLock;

use playerstate_t6::game::{entity_flag_bits, movement_state_bits};
use weapon_t6::pm::ads::AdsFacts;
use weapon_t6::pm::spread::SpreadFacts;
use weapon_t6::pose::{BasePose, PoseInput};
use weapon_t6::viewmodel::PERK_FASTSPRINTRECOVERY;
use weapon_t6::recoil::{
    BobInput, CameraIdleInput, IdleClocks, MovementAnglesInput, QuickScope, RecoilDvars, SwayInput,
    SwayState, camera_bob, camera_idle_angles, gun_bob, idle_angles, sway, weapon_movement_angles,
};

/// A dive to prone, in the air or sliding: Black Ops II's dive bit (movement_t6::bridge::PMF_DIVE).
pub(crate) const PMF_DIVE_ANY: u32 = 0x0020_0000;

/// The shellshock fade BO2 uses when the shock has no parms (`view.fadeTime` otherwise; this
/// client has no shellshock parms table).
pub const SHOCK_FADE_MS: i32 = 3000;

fn dvars() -> &'static RecoilDvars {
    static D: OnceLock<RecoilDvars> = OnceLock::new();
    D.get_or_init(RecoilDvars::default)
}

pub(crate) fn move_log() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_T6_MOVE_LOG").is_some())
}

fn bob_log() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_T6_BOB_LOG").is_some())
}

#[derive(Clone, Copy, Debug)]
pub struct T6Movement {
    /// The held gun's facts this frame, when the T6 path is on.
    pub facts: Option<(SpreadFacts, AdsFacts)>,
    pub sway: SwayState,
    sway_primed: bool,
    pub clocks: IdleClocks,
    /// The clock the drift reads this frame.
    pub idle_time: i32,
    gun_idle_factor: f32,
    view_idle_factor: f32,
    pub quick_scope: QuickScope,
    prior_progress: f32,
    /// The scope drift on the camera this frame (pitch, yaw).
    pub cam_idle: [f32; 2],
    facts_logged: Option<u32>,
    /// The walk cycle the last "t6 bob:" line printed.
    bob_logged: i32,
    /// BO2's own gun pose (stance, walk, sprint, dive, slide, climb), eased frame to frame.
    pub base: BasePose,
}

impl Default for T6Movement {
    fn default() -> Self {
        Self {
            facts: None,
            sway: SwayState::default(),
            sway_primed: false,
            clocks: IdleClocks::default(),
            idle_time: 0,
            gun_idle_factor: 1.0,
            view_idle_factor: 1.0,
            quick_scope: QuickScope::default(),
            prior_progress: 0.0,
            cam_idle: [0.0; 2],
            facts_logged: None,
            bob_logged: -1,
            base: BasePose::default(),
        }
    }
}

/// What the gun's angles and origin are built from.
pub struct GunMoveInput {
    pub frac: f32,
    pub e_bits: u32,
    pub movement_state_bits: u32,
    pub step_millis_elapsed: f32,
    pub sway_phase: i32,
    pub ground_velocity: f32,
    /// How deep in water the player stands: a swimmer's gun bobs its own way.
    pub waterlevel: i32,
    pub eye_height_goal: i32,
    /// What BO2's gun pose reads (BG_CalculateWeaponMovement_Base).
    pub pose: PoseInput,
    /// The recoil spring (pitch, yaw).
    pub recoil: [f32; 2],
    pub damage_kick: [f32; 3],
}

/// What the camera's walk bob reads ([`T6Movement::view_bob`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewBobInput {
    pub frac: f32,
    pub flags_e: u32,
    pub movement_state_bits: u32,
    pub sway_phase: i32,
    pub ground_velocity: f32,
    pub eye_height_goal: i32,
}

impl T6Movement {
    /// A new life: everything starts over.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A weapon change: the sway starts from the current view; the idle clocks run on (BO2 only
    /// ever adds to them).
    pub fn weapon_changed(&mut self) {
        self.sway = SwayState::default();
        self.sway_primed = false;
        self.quick_scope = QuickScope::default();
        self.facts_logged = None;
    }

    /// One frame of the sway (and the client's anti-quickscope copy, which the camera reads).
    pub fn tick_sway(&mut self, weapon_idx: u32, frac: f32, view: [f32; 3], dt: f32, msec: i32, shock_ms_left: i32) {
        let Some((s, a)) = self.facts else {
            return;
        };
        if move_log() && self.facts_logged != Some(weapon_idx) {
            self.facts_logged = Some(weapon_idx);
            log_facts(weapon_idx, &s, &a);
        }
        self.quick_scope.step(&a, self.prior_progress, frac, msec);
        self.prior_progress = frac;
        if !self.sway_primed {
            self.sway.last_view = view;
            self.sway_primed = true;
        }
        let before = self.sway;
        let input = SwayInput { frac, look_rotation: view, dt, shock_ms_left, shock_fade_ms: SHOCK_FADE_MS };
        sway(&mut self.sway, &s, &input);
        if move_log() {
            diag::info!(
                ViewModelPerspective,
                "t6 sway: dt {:.6} frac {:.5} view [{:.5}, {:.5}] last [{:.5}, {:.5}] shock {} prev [{:.6}, {:.6}, {:.6}, {:.6}] now [{:.6}, {:.6}, {:.6}, {:.6}]",
                dt,
                frac,
                view[0],
                view[1],
                before.last_view[0],
                before.last_view[1],
                shock_ms_left,
                before.angles[0],
                before.angles[1],
                before.side,
                before.up,
                self.sway.angles[0],
                self.sway.angles[1],
                self.sway.side,
                self.sway.up
            );
        }
    }

    /// The camera's frame: step the idle clocks, then the scope's drift (pitch, yaw).
    pub fn camera(&mut self, frac: f32, e_flag_bits: u32, pm: u32, dt: f32, hold_breath: f32) -> [f32; 2] {
        let Some((s, a)) = self.facts else {
            self.cam_idle = [0.0; 2];
            return self.cam_idle;
        };
        let before = self.clocks;
        self.idle_time = self.clocks.step(&s, frac, hold_breath, dt);
        let input = CameraIdleInput {
            frac,
            prone: e_flag_bits & entity_flag_bits::LYING_FLAT != 0 && pm & PMF_DIVE_ANY == 0,
            ducked: e_flag_bits & entity_flag_bits::CROUCH_BIT != 0,
            step_millis_elapsed: dt,
            idle_time: self.idle_time,
            hold_breath,
            quick_scope_scale: self.quick_scope.scale,
            aqs_sway_factor: a.aqs_sway_factor,
        };
        let f0 = self.view_idle_factor;
        self.cam_idle = camera_idle_angles(&mut self.view_idle_factor, &s, &input, dvars());
        if move_log() {
            diag::info!(
                ViewModelPerspective,
                "t6 cam: dt {:.6} frac {:.5} hb {:.5} qs {:.5} clock ps {} hip {} -> ps {} hip {} stance {} {} factor {:.5} -> {:.5} idle [{:.6}, {:.6}]",
                dt,
                frac,
                hold_breath,
                self.quick_scope.scale,
                before.ps,
                before.hip,
                self.clocks.ps,
                self.clocks.hip,
                u8::from(input.prone),
                u8::from(input.ducked),
                f0,
                self.view_idle_factor,
                self.cam_idle[0],
                self.cam_idle[1]
            );
        }
        self.cam_idle
    }

    /// The camera's walk bob (pitch, yaw, roll) by BO2's rules: a scope bobs with the gun's own
    /// bob (BG_CalculateViewMovement_BobAngles: the gun's bob factors, faded in by the aim), and
    /// any aimed gun adds the view's bob times `fAdsViewBobMult` (_AdsBob; not on a turret or
    /// vehicle). BO2's server turns the shot by both.
    pub fn view_bob(&self, i: &ViewBobInput) -> [f32; 3] {
        let Some((s, _)) = self.facts else {
            return [0.0; 3];
        };
        let d = dvars();
        let bob_in = BobInput {
            sway_phase: i.sway_phase & 0xff,
            ground_velocity: i.ground_velocity,
            waterlevel: 0,
            dtp: i.movement_state_bits & PMF_DIVE_ANY != 0,
            eye_height_goal: i.eye_height_goal,
            sprinting: i.movement_state_bits & movement_state_bits::SPRINTING != 0,
            frac: i.frac,
        };
        let mut result = [0.0f32; 3];
        if s.overlay {
            let bob = gun_bob(&s, &bob_in, d);
            result = [bob[0] * i.frac, bob[1] * i.frac, bob[2] * i.frac];
        }
        if i.frac > 0.0 && i.flags_e & weapon_t6::view_bob::EFLAGS_TURRET_VEHICLE == 0 && s.ads_view_bob_mult > 0.0 {
            let k = i.frac * s.ads_view_bob_mult;
            let [h, v] = camera_bob(&bob_in, i.ground_velocity, d);
            result[0] -= v * k;
            result[1] -= h * k;
        }
        result
    }

    /// The gun's angles and origin (BG_CalculateWeaponMovement + BG_SetWeaponMovementAngles), or
    /// `None` off the T6 path.
    pub fn gun(&mut self, i: &GunMoveInput) -> Option<([f32; 3], [f32; 3])> {
        let (s, _) = self.facts?;
        let d = dvars();
        let mut pose = i.pose;
        pose.fast_sprint_recovery |= s.has_perk(PERK_FASTSPRINTRECOVERY);
        self.base.step(&s.pose, s.scoped_aiming, &pose);
        let BasePose { angles: base_angles, origin: base_origin } = self.base;
        let bob_in = BobInput {
            sway_phase: i.sway_phase & 0xff,
            ground_velocity: i.ground_velocity,
            waterlevel: i.waterlevel,
            dtp: i.movement_state_bits & PMF_DIVE_ANY != 0,
            eye_height_goal: i.eye_height_goal,
            sprinting: i.movement_state_bits & movement_state_bits::SPRINTING != 0,
            frac: i.frac,
        };
        let bob = gun_bob(&s, &bob_in, d);
        let prone = i.e_bits & entity_flag_bits::LYING_FLAT != 0;
        let ducked = i.e_bits & entity_flag_bits::CROUCH_BIT != 0;
        let f0 = self.gun_idle_factor;
        let idle = idle_angles(&mut self.gun_idle_factor, &s, i.frac, prone, ducked, i.step_millis_elapsed, self.idle_time, d);
        let mut angles = weapon_movement_angles(&MovementAnglesInput {
            base: base_angles,
            frac: i.frac,
            sway: self.sway.angles,
            tilt_sideways: 0.0,
            recoil: i.recoil,
            bob,
            idle,
            overlay: s.overlay,
        });
        for (a, k) in angles.iter_mut().zip(i.damage_kick) {
            *a += k;
        }
        let k = (1.0 - i.frac * 2.0).max(0.0);
        let mut org = [base_origin[0] * k, base_origin[1] * k, base_origin[2] * k];
        org[1] -= self.sway.side;
        org[2] += self.sway.up;
        weapon_t6::placement::weapon_bob_rotate_origin(
            &mut org,
            weapon_t6::placement::GunStrideSwayState { pitch: bob[0], yaw: bob[1], roll: bob[2] },
        );
        if move_log() {
            diag::info!(
                ViewModelPerspective,
                "t6 gun: dt {:.6} frac {:.5} clock {} stance {} {} factor {:.5} -> {:.5} bob in {} {:.4} {} {} {} bob [{:.6}, {:.6}, {:.6}] idle [{:.6}, {:.6}, {:.6}] sway [{:.6}, {:.6}, {:.6}, {:.6}] base [{:.5}, {:.5}, {:.5}] [{:.5}, {:.5}, {:.5}] recoil [{:.5}, {:.5}] dmg [{:.5}, {:.5}, {:.5}] angles [{:.6}, {:.6}, {:.6}] origin [{:.6}, {:.6}, {:.6}]",
                i.step_millis_elapsed,
                i.frac,
                self.idle_time,
                u8::from(prone),
                u8::from(ducked),
                f0,
                self.gun_idle_factor,
                bob_in.sway_phase,
                bob_in.ground_velocity,
                u8::from(bob_in.dtp),
                bob_in.eye_height_goal,
                u8::from(bob_in.sprinting),
                bob[0],
                bob[1],
                bob[2],
                idle[0],
                idle[1],
                idle[2],
                self.sway.angles[0],
                self.sway.angles[1],
                self.sway.side,
                self.sway.up,
                base_angles[0],
                base_angles[1],
                base_angles[2],
                base_origin[0],
                base_origin[1],
                base_origin[2],
                i.recoil[0],
                i.recoil[1],
                i.damage_kick[0],
                i.damage_kick[1],
                i.damage_kick[2],
                angles[0],
                angles[1],
                angles[2],
                org[0],
                org[1],
                org[2]
            );
        }
        if bob_log() && bob_in.sway_phase != self.bob_logged {
            self.bob_logged = bob_in.sway_phase;
            diag::info!(
                ViewModelPerspective,
                "t6 bob: dt {:.6} cycle {} speed {:.3} sprint {} ads {:.3} bob [{:.5}, {:.5}, {:.5}] origin [{:.4}, {:.4}, {:.4}]",
                i.step_millis_elapsed,
                bob_in.sway_phase,
                bob_in.ground_velocity,
                u8::from(bob_in.sprinting),
                i.frac,
                bob[0],
                bob[1],
                bob[2],
                org[0],
                org[1],
                org[2]
            );
        }
        Some((angles, org))
    }
}

fn log_facts(weapon: u32, s: &SpreadFacts, a: &AdsFacts) {
    let d = dvars();
    let w = |v: weapon_t6::pm::spread::SwaySettings| {
        format!(
            "{},{},{},{},{},{}",
            v.widest_cone, v.ease_rate, v.tilt_multiplier, v.turn_factor, v.sideways_factor, v.upward_factor
        )
    };
    diag::info!(
        ViewModelPerspective,
        "t6 move facts: weapon {} ads {} overlay {} laser {} idle {},{},{},{},{},{} adsbob {} sprintbob {},{} dtpbob {},{} sway {} adssway {} shock {} aqs {} {} {} {} {} dvars aqs {} style {} zero {} bob {},{},{},{},{},{} stand {},{} duck {},{} prone {},{} sprint {},{} dtp {},{}",
        weapon,
        u8::from(s.scoped_aiming),
        u8::from(s.overlay),
        u8::from(s.laser_sight),
        s.hip_idle_amount,
        s.ads_idle_amount,
        s.hip_idle_speed,
        s.ads_idle_speed,
        s.idle_crouch_factor,
        s.idle_prone_factor,
        s.ads_bob_factor,
        s.sprint_bob[0],
        s.sprint_bob[1],
        s.dtp_bob[0],
        s.dtp_bob[1],
        w(s.sway),
        w(s.ads_sway),
        s.sway_shellshock_scale,
        u8::from(a.anti_quick_scope),
        a.aqs_ms(),
        a.aqs_scale,
        a.aqs_sway_factor,
        weapon_t6::pm::ads::dvar::PLAYER_BREATH_GASP_LERP,
        u8::from(d.aqs),
        d.aqs_style,
        u8::from(d.zero_idle_movement),
        d.weapon_bob_base,
        d.weapon_bob_base_laser,
        d.weapon_bob_roll,
        d.weapon_bob_roll_laser,
        d.weapon_bob_lag,
        d.weapon_bob_max,
        d.weapon_bob_standing[0],
        d.weapon_bob_standing[1],
        d.weapon_bob_ducked[0],
        d.weapon_bob_ducked[1],
        d.weapon_bob_prone[0],
        d.weapon_bob_prone[1],
        d.weapon_bob_sprinting[0],
        d.weapon_bob_sprinting[1],
        d.weapon_bob_dtp[0],
        d.weapon_bob_dtp[1]
    );
}

mod view_bob_tests {
    #![cfg(test)]
    use super::*;

    fn moving(frac: f32) -> ViewBobInput {
        ViewBobInput { frac, sway_phase: 40, ground_velocity: 190.0, eye_height_goal: 60, ..ViewBobInput::default() }
    }

    fn bob_in(i: &ViewBobInput) -> BobInput {
        BobInput {
            sway_phase: i.sway_phase,
            ground_velocity: i.ground_velocity,
            waterlevel: 0,
            dtp: false,
            eye_height_goal: i.eye_height_goal,
            sprinting: false,
            frac: i.frac,
        }
    }

    fn with(s: SpreadFacts) -> T6Movement {
        T6Movement { facts: Some((s, AdsFacts::default())), ..T6Movement::default() }
    }

    #[test]
    fn from_the_hip_the_view_does_not_bob() {
        let s = SpreadFacts { overlay: true, scoped_aiming: true, ads_view_bob_mult: 0.5, ads_bob_factor: 1.0, ..SpreadFacts::default() };
        assert_eq!(with(s).view_bob(&moving(0.0)), [0.0; 3]);
        assert_eq!(T6Movement::default().view_bob(&moving(1.0)), [0.0; 3]);
    }

    #[test]
    fn iron_sights_have_only_the_ads_bob() {
        let s = SpreadFacts { scoped_aiming: true, ads_view_bob_mult: 0.5, ..SpreadFacts::default() };
        let i = moving(1.0);
        let [h, v] = camera_bob(&bob_in(&i), i.ground_velocity, dvars());
        assert!(h != 0.0 && v != 0.0, "{h} {v}");
        assert_eq!(with(s).view_bob(&i), [-v * 0.5, -h * 0.5, 0.0]);
        // Half way in: half the bob.
        let half = with(s).view_bob(&moving(0.5));
        let [h, v] = camera_bob(&bob_in(&moving(0.5)), 190.0, dvars());
        assert_eq!(half, [-v * 0.25, -h * 0.25, 0.0]);
        // On a vehicle turret: none.
        let turret = ViewBobInput { flags_e: weapon_t6::view_bob::EFLAGS_TURRET_VEHICLE, ..i };
        assert_eq!(with(s).view_bob(&turret), [0.0; 3]);
    }

    #[test]
    fn a_scope_adds_the_gun_bob_by_how_far_he_is_in() {
        let s = SpreadFacts { overlay: true, scoped_aiming: true, ads_bob_factor: 0.6, ..SpreadFacts::default() };
        let i = moving(0.5);
        let bob = gun_bob(&s, &bob_in(&i), dvars());
        assert!(bob[0] != 0.0 && bob[1] != 0.0, "{bob:?}");
        assert_eq!(with(s).view_bob(&i), [bob[0] * 0.5, bob[1] * 0.5, bob[2] * 0.5]);
    }
}
