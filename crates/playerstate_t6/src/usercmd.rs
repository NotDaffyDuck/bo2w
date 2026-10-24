//! BO2's `usercmd_s`: one frame of a player's input.

use crate::Weapon;

/// BO2's `bitarray<64>`, the buttons held this frame. Button `n` is bit
/// `31 - n % 32` of word `n / 32`: button 0 is the first word's top bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ButtonBits(pub [u32; 2]);

impl ButtonBits {
    const fn mask(n: u32) -> u32 {
        0x8000_0000 >> (n & 31)
    }

    /// `bitarray<64>::testBit`.
    #[must_use]
    pub const fn test(&self, n: u32) -> bool {
        self.0[(n >> 5) as usize] & Self::mask(n) != 0
    }

    /// `bitarray<64>::setBit`.
    pub fn set(&mut self, n: u32) {
        self.0[(n >> 5) as usize] |= Self::mask(n);
    }

    /// `bitarray<64>::resetBit`.
    pub fn reset(&mut self, n: u32) {
        self.0[(n >> 5) as usize] &= !Self::mask(n);
    }

    /// `bitarray<64>::anyBitsOn`.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.0[0] != 0 || self.0[1] != 0
    }
}

/// BO2's button numbers, as its script checks read them
/// (`attackbuttonpressed` and the rest) and its client sets them
/// (CL_CmdButtons, CL_KeyMove, CL_AddCurrentStanceToCmd, and the cgame's
/// extra buttons).
pub mod button {
    pub const ATTACK: u32 = 0;
    pub const SPRINT: u32 = 1;
    pub const MELEE: u32 = 2;
    pub const USE: u32 = 3;
    pub const RELOAD: u32 = 4;
    /// Use pressed through the reload key (`usebuttonpressed` takes either).
    pub const USE_RELOAD: u32 = 5;
    /// The stance the client asks for: prone, or crouch.
    pub const PRONE: u32 = 8;
    pub const CROUCH: u32 = 9;
    pub const JUMP: u32 = 10;
    pub const ADS: u32 = 11;
    /// The crouch or prone key is held down (not toggled).
    pub const STANCE_HELD: u32 = 12;
    /// Hold breath (`+breath`).
    pub const BREATH: u32 = 13;
    pub const FRAG: u32 = 14;
    pub const SECONDARY_OFFHAND: u32 = 15;
    /// The location selector's pick is made.
    pub const LOCATION_CONFIRM: u32 = 16;
    /// The location selector is backed out of, or a held grenade whose
    /// weapon allows it is put away.
    pub const CANCEL: u32 = 17;
    /// The night vision action slot.
    pub const NIGHTVISION: u32 = 18;
    /// The vehicle's gas, three bits from here (an analog trigger).
    pub const GAS: u32 = 19;
    pub const REVERSE: u32 = 22;
    pub const HANDBRAKE: u32 = 23;
    pub const THROW: u32 = 24;
    pub const CHANGE_SEAT: u32 = 28;
    /// A menu holds the keys (any but the scoreboard).
    pub const MENU: u32 = 29;
    pub const VEHICLE_ATTACK: u32 = 34;
    pub const VEHICLE_ATTACK_SECOND: u32 = 35;
    pub const VEHICLE_MOVE_UP: u32 = 36;
    pub const VEHICLE_MOVE_DOWN: u32 = 37;
    /// The client's semi-auto quick-tap roll came up: the gun jams.
    pub const JAM: u32 = 38;
    /// The client's stance is the dive to prone.
    pub const DIVE_TO_PRONE: u32 = 44;
    pub const ACTION_SLOT_1: u32 = 45;
    pub const ACTION_SLOT_2: u32 = 46;
    pub const ACTION_SLOT_3: u32 = 47;
    pub const ACTION_SLOT_4: u32 = 48;
    /// Two quick use or reload taps.
    pub const DOUBLE_TAP: u32 = 50;
    pub const INVENTORY: u32 = 53;
    /// Throw back a grenade (`+throwback`).
    pub const THROWBACK: u32 = 59;
    pub const STANCE: u32 = 62;
    pub const VEHICLE_BOOST: u32 = 63;
}

/// BO2's `lastInput`: the device the player used last.
pub mod last_input {
    pub const GAMEPAD: u16 = 0;
    pub const MOUSECLICK: u16 = 2;
}

/// BO2's `usercmd_s` (less the Wii U pointer fields, which the PC game
/// never fills).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UserCmd {
    pub server_time: i32,
    pub button_bits: ButtonBits,
    /// View angles in BO2's 16-bit short-angle units.
    pub angles: [i32; 3],
    pub weapon: Weapon,
    pub off_hand_weapon: Weapon,
    pub last_weapon_alt_mode_switch: Weapon,
    pub forwardmove: i8,
    pub rightmove: i8,
    pub upmove: i8,
    pub pitchmove: i8,
    pub yawmove: i8,
    pub gun_pitch: i16,
    pub gun_yaw: i16,
    /// The knife lunge's target entity.
    pub melee_charge_ent: u16,
    pub melee_charge_dist: u8,
    pub rollmove: f32,
    pub selected_location: [i8; 2],
    pub selected_yaw: u8,
    pub airburst_mark_distance: u16,
    pub last_input: u16,
}

impl UserCmd {
    /// Is this button held in this command?
    #[must_use]
    pub const fn held(&self, button: u32) -> bool {
        self.button_bits.test(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_are_bo2s_order() {
        let mut b = ButtonBits::default();
        b.set(button::ATTACK);
        assert_eq!(b.0, [0x8000_0000, 0]);
        b.set(button::ADS);
        assert_eq!(b.0[0] >> 20 & 1, 1);
        b.set(button::STANCE);
        assert_eq!(b.0[1] >> 1 & 1, 1);
        b.set(button::INVENTORY);
        assert_eq!(b.0[1] & 0x400, 0x400);
        assert!(b.test(button::STANCE) && !b.test(button::JUMP));
        let mut stance = ButtonBits::default();
        stance.set(button::CROUCH);
        stance.set(button::STANCE_HELD);
        stance.set(button::DIVE_TO_PRONE);
        assert_eq!(stance.0, [0x0040_0000 | 0x0008_0000, 0x0008_0000]);
        b.reset(button::ATTACK);
        assert!(!b.test(button::ATTACK) && b.any());
    }
}
