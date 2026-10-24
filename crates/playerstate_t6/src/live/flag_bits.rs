//! Bit sets stored in [`PlayerState`](super::PlayerState). Each bit sits where BO2 keeps it; only the bits something
//! reads are named.

/// `pm_flags`: what the movement code is doing with the player this tick.
pub mod pm_flags {
    pub const PRONE: u32 = 1 << 0;
    pub const CROUCH: u32 = 1 << 1;
    /// Climbing over or onto something.
    pub const MANTLE: u32 = 1 << 2;
    pub const LADDER: u32 = 1 << 3;
    /// The aim button is held, whether or not the gun is up yet.
    pub const ADS_INTENT: u32 = 1 << 4;
    pub const BACKWARDS_RUN: u32 = 1 << 5;
    pub const WALKING: u32 = 1 << 6;
    /// A hard landing is slowing the player down.
    pub const TIME_HARDLANDING: u32 = 1 << 7;
    pub const PRONEMOVE_OVERRIDDEN: u32 = 1 << 9;
    pub const LADDER_FALL: u32 = 1 << 13;
    pub const JUMPING: u32 = 1 << 14;
    pub const SPRINTING: u32 = 1 << 15;
    pub const SHELLSHOCKED: u32 = 1 << 16;
    /// In the middle of a knife lunge.
    pub const MELEE_CHARGE: u32 = 1 << 17;
    /// Sprint is not allowed until the button is let go.
    pub const SPRINT_BLOCKED: u32 = 1 << 18;
}

/// `e_flags`: the player's entity flags, the part other clients see.
pub mod eflags {
    /// The origin jumped; clients snap instead of blending.
    pub const TELEPORT: u32 = 1 << 1;
    pub const DUCK: u32 = 1 << 2;
    pub const PRONE: u32 = 1 << 3;
    pub const HIDDEN: u32 = 1 << 5;
    /// The bit a kill-cam seat takes from the watching player rather than from the archived killer.
    pub const KILLCAM_PRESERVED: u32 = 1 << 7;
    pub const DEAD: u32 = 1 << 18;
    /// The player's radar is being jammed.
    pub const RADAR_JAM: u32 = 1 << 21;
}

/// `other_flags`: client-side view state.
pub mod other_flags {
    /// The record belongs to a live player (cleared on kill-cam seats).
    pub const PLAYER: u32 = 1 << 2;
    /// An EMP is scrambling the HUD.
    pub const EMP_JAMMED: u32 = 1 << 6;
    /// The record is a kill-cam seat watched from behind.
    pub const DEAD_KILLCAM_TPV: u32 = 1 << 11;
    /// Thermal view is on.
    pub const INFRARED: u32 = 1 << 12;
}

/// `weap_flags`: per-player gun view state.
pub mod weap_flags {
    /// A grenade or equipment is out, so the view model shows the offhand.
    pub const OFFHAND_VIEW: u32 = 1 << 1;
    /// Aiming down the sights is blocked.
    pub const NO_ADS: u32 = 1 << 5;
    pub const NIGHT_VISION: u32 = 1 << 6;
    pub const DOUBLEBARREL_RECOIL: u32 = 1 << 9;
    pub const RECOIL_SCALE: u32 = 1 << 10;
}

/// `mantle_state.flags`: the climb in progress. BO2-style climbs pack their length in milliseconds into the top half.
pub mod mantle_flags {
    use super::pm_flags;
    use crate::game::PlayerState;

    pub const OVER: i32 = 1;
    pub const ACTIVE: i32 = 1 << 3;
    pub const QUICK: i32 = 1 << 4;
    pub const FAST_MANTLE: i32 = 1 << 6;
    /// Raise the gun again before the climb ends.
    pub const BO2_WEAPON_UP: i32 = 1 << 8;
    /// The climb plays its own first-person gun animation.
    pub const BO2_WEAPON_ANIM: i32 = 1 << 9;
    /// The climb runs on BO2's timing (length in the top 16 bits).
    pub const BO2: i32 = 1 << 10;
    /// Where the climb length starts.
    pub const BO2_DURATION_SHIFT: i32 = 16;

    /// The climb length stored in the top half of the flags, read as unsigned.
    #[must_use]
    pub const fn bo2_duration_ms(flags: i32) -> i32 {
        (flags as u32 >> BO2_DURATION_SHIFT) as i32
    }

    /// How far through a BO2-style climb the player is, from 0 to 1. `None` when not climbing that way; a climb with
    /// no length counts as finished.
    #[must_use]
    pub fn bo2_progress(ps: &PlayerState) -> Option<f32> {
        let climb = ps.mantle_state;
        let bo2_climb = ps.pm_flags & pm_flags::MANTLE != 0 && climb.flags & BO2 != 0;
        if !bo2_climb {
            return None;
        }
        let length = bo2_duration_ms(climb.flags);
        Some(if length > 0 { (climb.timer as f32 / length as f32).clamp(0.0, 1.0) } else { 1.0 })
    }

    /// True while a BO2-style climb that raises the gun early is running.
    #[must_use]
    pub fn bo2_weapon_up(ps: &PlayerState) -> bool {
        ps.mantle_state.flags & BO2_WEAPON_UP != 0 && bo2_progress(ps).is_some()
    }
}

// Perk bits the game checks by name.
pub const PERK_PISTOLDEATH: u32 = 1 << 7;
pub const PERK_QUIETER: u32 = 1 << 8;
pub const PERK_SCAVENGER: u32 = 1 << 22;
pub const PERK_COLDBLOODED: u32 = 1 << 27;
pub const PERK_HEARTBREAKER: u32 = 1 << 28;
