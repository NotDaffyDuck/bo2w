//! Hold breath on a scoped sniper: the timer, the gasp and the drift scale that follow from them.

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
    pub sway_factor: f32,
    /// `holdBreathTimer`, milliseconds.
    pub(super) timer_ms: i32,
    /// The hold flag (`weapFlags & 4`).
    pub(super) holding: bool,
}

impl Default for HoldBreath {
    fn default() -> Self {
        Self {
            sway_factor: 1.0,
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
pub(super) fn diff_track(target: f32, cur: f32, rate: f32, dt_secs: f32) -> f32 {
    let gap = target - cur;
    let step = rate * gap * dt_secs;
    if gap.abs() <= 0.001 || step.abs() > gap.abs() {
        target
    } else {
        cur + step
    }
}

impl HoldBreath {
    /// One frame (`PM_UpdateHoldBreath`): `eligible` (a hold-breath scope),
    /// `pos_frac` (the aim fraction), `held` (the button), `dt_ms`. Returns
    /// the drift scale.
    pub fn step_on(&mut self, eligible: bool, pos_frac: f32, held: bool, dt_ms: f32) -> f32 {
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
        let (goal, rate) = if self.holding {
            (0.0, BREATH_HOLD_LERP)
        } else {
            let full = (BREATH_GASP_TIME_MS + BREATH_HOLD_TIME_MS) as f32;
            (
                (BREATH_GASP_SCALE - 1.0) * (self.timer_ms as f32 / full) + 1.0,
                BREATH_GASP_LERP,
            )
        };
        self.sway_factor = diff_track(
            (goal - 1.0) * pos_frac + 1.0,
            self.sway_factor,
            rate,
            dt_ms / 1000.0,
        );
        self.sway_factor
    }

    /// A shot at full aim (`PM_HoldBreathFire`): the hold ends (the fire
    /// delay, 0, adds nothing to the timer).
    pub fn fired(&mut self, pos_frac: f32, hud_crosshair_layer: bool) {
        if pos_frac >= 1.0 && hud_crosshair_layer {
            self.holding = false;
        }
    }
}
