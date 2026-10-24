//! bo2mp gun brain (window 12): Black Ops II's gun movement on the client, for a gun the T6 facts
//! know: the gun's sway, its walk bob and its slow drift (idle), and the scope's drift on the
//! camera. The math is `weapon_t6::recoil`'s; this keeps the per-player state and feeds it.
//!
//! IW4L_T6_MOVE_LOG=1 prints one "t6 sway:", "t6 cam:" and "t6 gun:" line per frame (and a
//! "t6 move facts:" line per weapon) for docs/gun/check_move.py.
//!
//! IW4L_T6_BOB_LOG=1 is the light one for a whole play session: one short "t6 bob:" line per
//! frame, only while the walk cycle moves (the cycle, the speed, sprint, the gun's bob angles
//! and its final origin). Nothing on screen.

use playerstate_iw4::{eflags, pm_flags};
use weapon_t6::pm::ads::AdsFacts;
use weapon_t6::pm::spread::SpreadFacts;
use weapon_t6::recoil::{
    BobInput, CameraIdleInput, IdleClocks, MovementAnglesInput, QuickScope, RecoilDvars, SwayInput,
    SwayState, camera_idle_angles, gun_bob, idle_angles, sway, weapon_movement_angles,
};

/// A dive to prone, in the air or sliding (movement_iw4::dive PMF_DIVE | PMF_DIVE_SLIDE).
const PMF_DIVE_ANY: u32 = 0x0100_0000 | 0x0200_0000;

/// The shellshock fade BO2 uses when the shock has no parms (`view.fadeTime` otherwise; this
/// client has no shellshock parms table).
pub const SHOCK_FADE_MS: i32 = 3000;

fn dvars() -> &'static RecoilDvars {
    static D: std::sync::OnceLock<RecoilDvars> = std::sync::OnceLock::new();
    D.get_or_init(RecoilDvars::default)
}

pub(crate) fn move_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IW4L_T6_MOVE_LOG").is_some())
}

fn bob_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IW4L_T6_BOB_LOG").is_some())
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
    last_frac: f32,
    /// The scope drift on the camera this frame (pitch, yaw).
    pub cam_idle: [f32; 2],
    facts_logged: Option<u32>,
    /// The walk cycle the last "t6 bob:" line printed.
    bob_logged: i32,
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
            last_frac: 0.0,
            cam_idle: [0.0; 2],
            facts_logged: None,
            bob_logged: -1,
        }
    }
}

/// What the gun's angles and origin are built from (the parts the iw4 placement still makes).
pub struct GunMoveInput {
    pub frac: f32,
    pub e_flags: u32,
    pub pm_flags: u32,
    pub frametime: f32,
    pub bob_cycle: i32,
    pub xyspeed: f32,
    pub view_height_target: i32,
    /// The stance pose (BG_CalculateWeaponMovement_Base) and its offset.
    pub base_angles: [f32; 3],
    pub base_origin: [f32; 3],
    /// The recoil spring (pitch, yaw).
    pub recoil: [f32; 2],
    pub damage_kick: [f32; 3],
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
    pub fn tick_sway(&mut self, weapon: u32, frac: f32, view: [f32; 3], dt: f32, msec: i32, shock_ms_left: i32) {
        let Some((s, a)) = self.facts else {
            return;
        };
        if move_log() && self.facts_logged != Some(weapon) {
            self.facts_logged = Some(weapon);
            log_facts(weapon, &s, &a);
        }
        self.quick_scope.step(&a, self.last_frac, frac, msec);
        self.last_frac = frac;
        if !self.sway_primed {
            self.sway.last_view = view;
            self.sway_primed = true;
        }
        let before = self.sway;
        let input = SwayInput { frac, view_angles: view, dt, shock_ms_left, shock_fade_ms: SHOCK_FADE_MS };
        sway(&mut self.sway, &s, &input);
        if move_log() {
            diag::info!(
                Fpv,
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
    pub fn camera(&mut self, frac: f32, e_flags: u32, pm: u32, dt: f32, hold_breath: f32) -> [f32; 2] {
        let Some((s, a)) = self.facts else {
            self.cam_idle = [0.0; 2];
            return self.cam_idle;
        };
        let before = self.clocks;
        self.idle_time = self.clocks.step(&s, frac, hold_breath, dt);
        let input = CameraIdleInput {
            frac,
            prone: e_flags & eflags::PRONE != 0 && pm & PMF_DIVE_ANY == 0,
            ducked: e_flags & eflags::DUCK != 0,
            frametime: dt,
            idle_time: self.idle_time,
            hold_breath,
            quick_scope_scale: self.quick_scope.scale,
            aqs_sway_factor: a.aqs_sway_factor,
        };
        let f0 = self.view_idle_factor;
        self.cam_idle = camera_idle_angles(&mut self.view_idle_factor, &s, &input, dvars());
        if move_log() {
            diag::info!(
                Fpv,
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

    /// The gun's angles and origin (BG_CalculateWeaponMovement + BG_SetWeaponMovementAngles), or
    /// `None` off the T6 path.
    pub fn gun(&mut self, i: &GunMoveInput) -> Option<([f32; 3], [f32; 3])> {
        let (s, _) = self.facts?;
        let d = dvars();
        let bob_in = BobInput {
            bob_cycle: i.bob_cycle & 0xff,
            xyspeed: i.xyspeed,
            waterlevel: 0,
            dtp: i.pm_flags & PMF_DIVE_ANY != 0,
            view_height_target: i.view_height_target,
            sprinting: i.pm_flags & pm_flags::SPRINTING != 0,
            frac: i.frac,
        };
        let bob = gun_bob(&s, &bob_in, d);
        let prone = i.e_flags & eflags::PRONE != 0;
        let ducked = i.e_flags & eflags::DUCK != 0;
        let f0 = self.gun_idle_factor;
        let idle = idle_angles(&mut self.gun_idle_factor, &s, i.frac, prone, ducked, i.frametime, self.idle_time, d);
        let mut angles = weapon_movement_angles(&MovementAnglesInput {
            base: i.base_angles,
            frac: i.frac,
            sway: self.sway.angles,
            leanf: 0.0,
            recoil: i.recoil,
            bob,
            idle,
            overlay: s.overlay,
        });
        for (a, k) in angles.iter_mut().zip(i.damage_kick) {
            *a += k;
        }
        let k = (1.0 - i.frac * 2.0).max(0.0);
        let mut origin = [i.base_origin[0] * k, i.base_origin[1] * k, i.base_origin[2] * k];
        origin[1] -= self.sway.side;
        origin[2] += self.sway.up;
        weapon_iw4::weapon_bob_rotate_origin(
            &mut origin,
            weapon_iw4::WeaponBobState { pitch: bob[0], yaw: bob[1], roll: bob[2] },
        );
        if move_log() {
            diag::info!(
                Fpv,
                "t6 gun: dt {:.6} frac {:.5} clock {} stance {} {} factor {:.5} -> {:.5} bob in {} {:.4} {} {} {} bob [{:.6}, {:.6}, {:.6}] idle [{:.6}, {:.6}, {:.6}] sway [{:.6}, {:.6}, {:.6}, {:.6}] base [{:.5}, {:.5}, {:.5}] [{:.5}, {:.5}, {:.5}] recoil [{:.5}, {:.5}] dmg [{:.5}, {:.5}, {:.5}] angles [{:.6}, {:.6}, {:.6}] origin [{:.6}, {:.6}, {:.6}]",
                i.frametime,
                i.frac,
                self.idle_time,
                u8::from(prone),
                u8::from(ducked),
                f0,
                self.gun_idle_factor,
                bob_in.bob_cycle,
                bob_in.xyspeed,
                u8::from(bob_in.dtp),
                bob_in.view_height_target,
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
                i.base_angles[0],
                i.base_angles[1],
                i.base_angles[2],
                i.base_origin[0],
                i.base_origin[1],
                i.base_origin[2],
                i.recoil[0],
                i.recoil[1],
                i.damage_kick[0],
                i.damage_kick[1],
                i.damage_kick[2],
                angles[0],
                angles[1],
                angles[2],
                origin[0],
                origin[1],
                origin[2]
            );
        }
        if bob_log() && bob_in.bob_cycle != self.bob_logged {
            self.bob_logged = bob_in.bob_cycle;
            diag::info!(
                Fpv,
                "t6 bob: dt {:.6} cycle {} speed {:.3} sprint {} ads {:.3} bob [{:.5}, {:.5}, {:.5}] origin [{:.4}, {:.4}, {:.4}]",
                i.frametime,
                bob_in.bob_cycle,
                bob_in.xyspeed,
                u8::from(bob_in.sprinting),
                i.frac,
                bob[0],
                bob[1],
                bob[2],
                origin[0],
                origin[1],
                origin[2]
            );
        }
        Some((angles, origin))
    }
}

fn log_facts(weapon: u32, s: &SpreadFacts, a: &AdsFacts) {
    let d = dvars();
    let w = |v: weapon_t6::pm::spread::SwaySettings| {
        format!(
            "{},{},{},{},{},{}",
            v.max_angle, v.lerp_speed, v.pitch_scale, v.yaw_scale, v.horiz_scale, v.vert_scale
        )
    };
    diag::info!(
        Fpv,
        "t6 move facts: weapon {} ads {} overlay {} laser {} idle {},{},{},{},{},{} adsbob {} sprintbob {},{} dtpbob {},{} sway {} adssway {} shock {} aqs {} {} {} {} {} dvars aqs {} style {} zero {} bob {},{},{},{},{},{} stand {},{} duck {},{} prone {},{} sprint {},{} dtp {},{}",
        weapon,
        u8::from(s.aim_down_sight),
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
