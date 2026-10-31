//! Bit sets stored in [`AvatarState`](super::AvatarState). Each bit sits where BO2 keeps it; only the bits something
//! reads are named.

/// `movement_state_bits`: what the movement code is doing with the player this tick.
pub mod movement_state_bits {
    pub const LYING_FLAT: u32 = 1 << 0;
    pub const DUCK_BUTTON_BIT: u32 = 1 << 1;
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
    /// Leaning is not allowed (the riot shield's carry, `AllowLean(0)`).
    pub const NO_LEAN: u32 = crate::flags::movement_state_bits::NO_LEAN as u32;
}

/// `e_flags`: the player's entity flags, the part other clients see.
pub mod entity_flag_bits {
    /// The origin jumped; clients snap instead of blending.
    pub const TELEPORT: u32 = 1 << 1;
    pub const CROUCH_BIT: u32 = 1 << 2;
    pub const LYING_FLAT: u32 = 1 << 3;
    pub const HIDDEN: u32 = 1 << 5;
    /// The bit a kill-cam seat takes from the watching player rather than from the archived killer.
    pub const KILLCAM_PRESERVED: u32 = 1 << 7;
    /// On a mounted turret: the stance it is fired from is one half or both (`turret_use`).
    pub const TURRET_STAND: u32 = crate::flags::e_flags::TURRET_STAND as u32;
    pub const TURRET_CROUCH: u32 = crate::flags::e_flags::TURRET_CROUCH as u32;
    pub const TURRET_MASK: u32 = crate::flags::e_flags::TURRET_MASK as u32;
    /// Mantling (`Mantle_Start`).
    pub const MANTLE: u32 = crate::flags::e_flags::MANTLE as u32;
    pub const DEAD: u32 = 1 << 18;
    /// The player's radar is being jammed.
    pub const RADAR_JAM: u32 = 1 << 21;
}

/// `secondary_state_bits`: client-side view state.
pub mod secondary_state_bits {
    /// The record belongs to a live player (cleared on kill-cam seats).
    pub const PLAYER: u32 = 1 << 2;
    /// An EMP is scrambling the HUD.
    pub const EMP_JAMMED: u32 = 1 << 6;
    /// The record is a kill-cam seat watched from behind.
    pub const DEAD_KILLCAM_TPV: u32 = 1 << 11;
    /// Thermal view is on.
    pub const INFRARED: u32 = 1 << 12;
}

/// `weapon_state_bits`: per-player gun view state.
pub mod weapon_state_bits {
    /// A grenade or equipment is out, so the view model shows the offhand.
    pub const OFFHAND_VIEW: u32 = 1 << 1;
    /// Aiming down the sights is blocked.
    pub const AIM_DISABLED_BIT: u32 = 1 << 5;
    pub const NIGHT_VISION: u32 = 1 << 6;
    pub const DOUBLEBARREL_RECOIL: u32 = 1 << 9;
    pub const RECOIL_SCALE: u32 = 1 << 10;
}

/// `mantle_state.flags`: the climb in progress. BO2-style climbs pack their length in milliseconds into the top half.
pub mod ledge_climb_bits {
    use super::movement_state_bits;
    use crate::game::AvatarState;

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
    pub fn bo2_progress(ps: &AvatarState) -> Option<f32> {
        let climb = ps.mantle_state;
        let bo2_climb = ps.movement_state_bits & movement_state_bits::MANTLE != 0 && climb.flags & BO2 != 0;
        if !bo2_climb {
            return None;
        }
        let length = bo2_duration_ms(climb.flags);
        Some(if length > 0 { (climb.timer as f32 / length as f32).clamp(0.0, 1.0) } else { 1.0 })
    }

    /// True while a BO2-style climb that raises the gun early is running.
    #[must_use]
    pub fn bo2_weapon_up(ps: &AvatarState) -> bool {
        ps.mantle_state.flags & BO2_WEAPON_UP != 0 && bo2_progress(ps).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags;

    #[test]
    fn the_live_bits_are_the_bo2_bits() {
        assert_eq!(entity_flag_bits::TURRET_STAND, 0x100);
        assert_eq!(entity_flag_bits::TURRET_CROUCH, 0x200);
        assert_eq!(entity_flag_bits::TURRET_MASK, 0x300);
        assert_eq!(entity_flag_bits::MANTLE, 0x8000);
        assert_eq!(entity_flag_bits::DEAD, flags::e_flags::DEAD as u32);
        assert_eq!(entity_flag_bits::CROUCH_BIT, flags::e_flags::CROUCHING as u32);
        assert_eq!(entity_flag_bits::LYING_FLAT, flags::e_flags::LYING_FLAT as u32);
        assert_eq!(movement_state_bits::NO_LEAN, 0x1000_0000);
        assert_eq!(movement_state_bits::SPRINTING, flags::movement_state_bits::SPRINT as u32);
        assert_eq!(movement_state_bits::LADDER_FALL, flags::movement_state_bits::LADDER_FALL as u32);
    }
}
