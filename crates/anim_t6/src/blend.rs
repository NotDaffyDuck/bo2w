//! Black Ops II's client side of a third-person animation change, as
//! `BG_SetNewAnimation` does it: how long a part's new animation blends in
//! over the old one, the hold a stance change puts on the legs' blends,
//! where a moving loop starts, and (`BG_RunLerpFrameRate`) the rate a moving
//! animation plays at. The server's side (which animation, when it may
//! start, how long it holds) is weapon_t6's `third`.

/// The blend (ms) when a part's animation is cleared (new index 0).
pub const T6_CLEAR_BLEND_MS: i32 = 200;

/// The least blend (ms) into a moving animation from another.
pub const T6_BLEND_FLOOR_MOVING_MS: i32 = 133;

/// The least blend (ms) when no animation played or the old one stood still.
pub const T6_BLEND_FLOOR_STILL_MS: i32 = 167;

/// The least blend (ms) out of a moving or turning animation.
pub const T6_BLEND_FLOOR_FROM_MOVING_MS: i32 = 267;

/// A legs stance change (to or from crouch or prone) holds the legs' blends
/// to at least what is left of this (ms) from the change.
pub const T6_STANCE_TRANSITION_MS: i32 = 400;

/// `BG_PlayerAnimation_VerifyAnim`: the blend (ms) a part keeps once its
/// animation has faded out.
pub const T6_VERIFY_BLEND_MS: i32 = 167;

/// `bg_blendTimeOverride` (ms): every blend this long when >= 0.
/// BO2 default unknown (MW2 has no such dvar): off.
pub const T6_BLEND_TIME_OVERRIDE_MS: i32 = -1;

/// Each client's moving loop starts this much of a cycle after the client
/// before his (so bodies running together do not step in time).
pub const T6_LOOP_PHASE_PER_CLIENT: f32 = 0.36;

/// The loop-sync period (ms) is the old animation's length plus this ...
pub const T6_LOOP_PERIOD_PAD_MS: i32 = 200;

/// ... or this when there was none.
pub const T6_LOOP_PERIOD_MS: i32 = 1000;

/// `XAnimSetGoalWeight`: a goal weight under this is 0, and a weight rising
/// from 0 starts at this much of its goal.
pub const T6_GOAL_WEIGHT_EPS: f32 = 0.001;

/// BO2's XAnim flags (MW2 has only 1, 2, 8 and 16).
pub mod xanim_flags {
    pub const LOOP_SYNC_TIME: u32 = 1;
    pub const NONLOOP_SYNC_TIME: u32 = 2;
    /// The animation drives the root: a blend it leaves unset is 0.
    pub const SYNC_ROOT: u32 = 4;
    pub const COMPLETE: u32 = 8;
    pub const ADDITIVE: u32 = 16;
    pub const CLIENT: u32 = 32;
    pub const SEPARATE: u32 = 64;
    /// The flags a node passes to its children.
    pub const PROPOGATE_FLAGS: u32 = 99;
    pub const FORCELOAD: u32 = 128;
}

/// What `BG_SetNewAnimation` knows when a part's animation changes.
#[derive(Clone, Copy, Debug, Default)]
pub struct T6Blend {
    /// The new animation's index (0 clears the part).
    pub new_index: u16,
    /// Its `blendtime` (initialLerp, ms; negative when it gave none).
    pub initial_lerp: i32,
    /// Its `blendouttime` (finalLerp, ms; negative when it gave none), used
    /// when it gave no `blendtime`.
    pub final_lerp: i32,
    /// An animation played on the part before.
    pub old_exists: bool,
    /// The old animation moves or turns (move or rot speed).
    pub old_moving: bool,
    /// The new animation moves (move speed).
    pub new_moving: bool,
    /// The new animation has [`xanim_flags::SYNC_ROOT`].
    pub sync_root: bool,
    /// What is left (ms) of a legs stance change's hold ([`t6_stance_hold_ms`]);
    /// 0 for the torso.
    pub stance_hold_ms: i32,
    /// The legs' first animation ever (or a paused demo): no blend.
    pub first: bool,
}

/// The blend (ms) a part's new animation fades in over: its own
/// `blendtime` (or `blendouttime`), else BO2's floors by what moves.
pub fn t6_blend_ms(b: &T6Blend) -> i32 {
    if b.first {
        return 0;
    }
    let mut blend = if b.new_index == 0 {
        T6_CLEAR_BLEND_MS
    } else if b.initial_lerp >= 0 {
        b.initial_lerp
    } else {
        b.final_lerp
    };
    if T6_BLEND_TIME_OVERRIDE_MS >= 0 {
        blend = T6_BLEND_TIME_OVERRIDE_MS;
    }
    let from_old = || {
        if b.old_exists && b.old_moving {
            T6_BLEND_FLOOR_FROM_MOVING_MS
        } else {
            T6_BLEND_FLOOR_STILL_MS
        }
    };
    let floor = if b.new_index == 0 {
        from_old()
    } else if blend >= 1 {
        -1
    } else if b.sync_root && blend < 0 {
        0
    } else if b.new_moving && b.old_exists {
        T6_BLEND_FLOOR_MOVING_MS
    } else {
        from_old()
    };
    blend.max(floor).max(b.stance_hold_ms)
}

/// How far (0..1) a part's new animation has blended in, `blend_ms` after
/// it started at `switched`; the old one weighs the rest.
pub fn t6_blend_weight(now: i32, switched: i32, blend_ms: i32) -> f32 {
    ((now - switched) as f32 / blend_ms.max(1) as f32).clamp(0.0, 1.0)
}

/// `BG_PlayerAnimation_VerifyAnim`: an animation whose weight has reached 0
/// is cleared from its part (no animation, so the next one blends in from
/// none, by the [`T6_VERIFY_BLEND_MS`] floor).
pub fn t6_verify_anim(weight: f32) -> bool {
    weight <= 0.0
}

/// What is left (ms) of a stance change's hold on the legs' blends, from
/// when it ends (`stanceTransitionTime`, the change's time plus
/// [`T6_STANCE_TRANSITION_MS`]).
pub fn t6_stance_hold_ms(stance_transition_time: i32, now: i32) -> i32 {
    (stance_transition_time - now).max(0)
}

/// Where (a fraction of its cycle) a moving loop starts when it takes over
/// from an animation that was not one: by the clock, over the old
/// animation's length (ms) plus 200 (1000 with none), and 0.36 of a cycle
/// further per client number.
pub fn t6_loop_start(now: i32, old_len_ms: Option<i32>, client: u32) -> f32 {
    let period = old_len_ms.map_or(T6_LOOP_PERIOD_MS, |l| l.max(0) + T6_LOOP_PERIOD_PAD_MS);
    let at = now.rem_euclid(period) as f32 / period as f32;
    (at + client as f32 * T6_LOOP_PHASE_PER_CLIENT).fract()
}

/// The least rate a moving animation plays at.
pub const T6_RATE_MIN: f32 = 0.1;

/// Under this a ladder climb stands still.
pub const T6_RATE_ZERO_EPS: f32 = 0.01;

/// Up to this a moving animation plays at its speed over its own.
pub const T6_RATE_CAP: f32 = 2.0;

/// A ladder climb's most.
pub const T6_RATE_LADDER_CAP: f32 = 4.0;

/// A slow animation's most (move speed under 20) ...
pub const T6_RATE_SHORT_CAP: f32 = 3.0;

/// ... easing to [`T6_RATE_CAP`] between these move speeds (units/s).
pub const T6_RATE_LERP_MIN_SPEED: f32 = 20.0;
pub const T6_RATE_LERP_MAX_SPEED: f32 = 150.0;

/// A turn-in-place animation plays between these (by his turn speed over
/// its own).
pub const T6_TURN_RATE_MIN: f32 = 0.4;
pub const T6_TURN_RATE_MAX: f32 = 1.0;

/// `BG_RunLerpFrameRate` for an animation that turns but does not move
/// (`move_speed` 0, `rot_speed` set): his turn speed (the yaw change since
/// the last sample, wrapped to -180..180, made positive, per second) over
/// its own, held to 0.4..1. None when it has no `rot_speed` either (the
/// anim script parser zeroes it for some events and the mantles): BO2 plays
/// it at 1.0.
pub fn t6_turn_rate(turn_speed: f32, rot_speed: f32) -> Option<f32> {
    (rot_speed > 0.0).then(|| (turn_speed / rot_speed).clamp(T6_TURN_RATE_MIN, T6_TURN_RATE_MAX))
}

/// `BG_RunLerpFrameRate`'s limits on a moving animation's rate (his speed
/// over the animation's own `move_speed`): at least 0.1 (0 for a ladder
/// climb under 0.01); above 2, at most 4 on a ladder, else 3 easing to 2
/// over move speeds 20..150. The same as MW2's.
pub fn t6_moving_rate(scale: f32, move_speed: f32, ladder: bool) -> f32 {
    if scale < T6_RATE_MIN {
        return if ladder && scale < T6_RATE_ZERO_EPS { 0.0 } else { T6_RATE_MIN };
    }
    if scale <= T6_RATE_CAP {
        return scale;
    }
    let max = if ladder {
        T6_RATE_LADDER_CAP
    } else if move_speed > T6_RATE_LERP_MAX_SPEED {
        T6_RATE_CAP
    } else if move_speed >= T6_RATE_LERP_MIN_SPEED {
        T6_RATE_SHORT_CAP
            - (move_speed - T6_RATE_LERP_MIN_SPEED) / (T6_RATE_LERP_MAX_SPEED - T6_RATE_LERP_MIN_SPEED)
    } else {
        T6_RATE_SHORT_CAP
    };
    scale.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(new_index: u16) -> T6Blend {
        T6Blend { new_index, initial_lerp: -1, final_lerp: -1, old_exists: true, ..T6Blend::default() }
    }

    #[test]
    fn floors_by_what_moves() {
        assert_eq!(t6_blend_ms(&T6Blend { new_moving: true, ..change(5) }), 133);
        assert_eq!(t6_blend_ms(&change(5)), 167);
        assert_eq!(t6_blend_ms(&T6Blend { old_moving: true, ..change(5) }), 267);
        // No old animation: the still floor, even into a moving one.
        let none = T6Blend { old_exists: false, new_moving: true, ..change(5) };
        assert_eq!(t6_blend_ms(&none), 167);
        // A root-synced animation with no blend of its own: none.
        assert_eq!(t6_blend_ms(&T6Blend { sync_root: true, old_moving: true, ..change(5) }), 0);
    }

    #[test]
    fn own_blend_then_clear_then_first() {
        assert_eq!(t6_blend_ms(&T6Blend { initial_lerp: 50, old_moving: true, ..change(5) }), 50);
        // blendouttime stands in for a missing blendtime.
        assert_eq!(t6_blend_ms(&T6Blend { final_lerp: 80, ..change(5) }), 80);
        // A zero blendtime still takes the floor.
        assert_eq!(t6_blend_ms(&T6Blend { initial_lerp: 0, ..change(5) }), 167);
        assert_eq!(t6_blend_ms(&T6Blend { initial_lerp: 50, ..change(0) }), 200);
        assert_eq!(t6_blend_ms(&T6Blend { old_moving: true, ..change(0) }), 267);
        assert_eq!(t6_blend_ms(&T6Blend { first: true, ..change(5) }), 0);
    }

    #[test]
    fn stance_change_holds_the_legs() {
        let until = 1000 + T6_STANCE_TRANSITION_MS;
        assert_eq!(t6_stance_hold_ms(until, 1100), 300);
        assert_eq!(t6_stance_hold_ms(until, 1500), 0);
        let b = T6Blend { initial_lerp: 50, stance_hold_ms: t6_stance_hold_ms(until, 1100), ..change(5) };
        assert_eq!(t6_blend_ms(&b), 300);
    }

    #[test]
    fn loops_start_apart_per_client() {
        assert_eq!(t6_loop_start(500, None, 0), 0.5);
        assert!((t6_loop_start(500, None, 1) - 0.86).abs() < 1e-6);
        assert!((t6_loop_start(500, None, 2) - 0.22).abs() < 1e-5);
        // Over the old animation's length plus 200.
        assert_eq!(t6_loop_start(1200, Some(600), 0), 0.5);
    }

    #[test]
    fn weight_and_verify() {
        assert_eq!(t6_blend_weight(1000, 1000, 200), 0.0);
        assert_eq!(t6_blend_weight(1100, 1000, 200), 0.5);
        assert_eq!(t6_blend_weight(1300, 1000, 200), 1.0);
        // A blend of 0 is in at once.
        assert_eq!(t6_blend_weight(1000, 1000, 0), 0.0);
        assert_eq!(t6_blend_weight(1001, 1000, 0), 1.0);
        // The old animation is cleared once the new one is all in.
        assert!(t6_verify_anim(1.0 - t6_blend_weight(1300, 1000, 200)));
        assert!(!t6_verify_anim(1.0 - t6_blend_weight(1100, 1000, 200)));
    }

    #[test]
    fn turn_rate_limits() {
        assert_eq!(t6_turn_rate(90.0, 0.0), None);
        assert_eq!(t6_turn_rate(10.0, 90.0), Some(0.4));
        assert_eq!(t6_turn_rate(63.0, 90.0), Some(0.7));
        assert_eq!(t6_turn_rate(300.0, 90.0), Some(1.0));
    }

    #[test]
    fn moving_rate_limits() {
        assert_eq!(t6_moving_rate(0.05, 100.0, false), 0.1);
        assert_eq!(t6_moving_rate(0.005, 100.0, true), 0.0);
        assert_eq!(t6_moving_rate(1.5, 100.0, false), 1.5);
        assert_eq!(t6_moving_rate(9.0, 100.0, true), 4.0);
        assert_eq!(t6_moving_rate(9.0, 10.0, false), 3.0);
        assert_eq!(t6_moving_rate(9.0, 85.0, false), 2.5);
        assert_eq!(t6_moving_rate(9.0, 200.0, false), 2.0);
    }
}
