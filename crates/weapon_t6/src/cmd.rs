//! The player's input for one weapon tick (the weapon part of BO2's `usercmd_s`).

/// Button bits, same values as the game's wire command word.
pub mod button {
    pub const ATTACK: u32 = 0x1;
    pub const SPRINT: u32 = 0x2;
    pub const MELEE: u32 = 0x4;
    pub const USE: u32 = 0x8;
    pub const RELOAD: u32 = 0x10;
    pub const USE_RELOAD: u32 = 0x20;
    pub const PRONE: u32 = 0x100;
    pub const CROUCH: u32 = 0x200;
    pub const JUMP: u32 = 0x400;
    pub const ADS: u32 = 0x800;
    pub const STANCE_HELD: u32 = 0x1000;
    pub const BREATH: u32 = 0x2000;
    pub const FRAG: u32 = 0x4000;
    pub const SMOKE: u32 = 0x8000;
    pub const LOCATION_SELECT: u32 = 0x10000;
    /// BO2 button bit 46: switch to the other offhand (tactical/lethal).
    pub const OFFHAND_SWITCH: u32 = 0x20000;
    /// BO2 button bit 7: +toggleads_throw held.
    pub const TOGGLE_ADS: u32 = 0x40000;
    /// BO2 button bit 2: change zoom (vzoom).
    pub const CHANGE_ZOOM: u32 = 0x80000;
    /// BO2 button bit 57 (command word 1, mask 0x2000000): the client's semi-auto jam roll came
    /// up (IN_Attack_Down, see docs/gun/FIRE.md 3.2). The fire step jams the gun instead of
    /// shooting. Our wire carries it in this free bit.
    pub const JAM: u32 = 0x1000000;
}

/// BO2 `LastInput_t`: the device the player last used. Picks which trigger fires which gun of a
/// dual-wield pair ([`crate::pm::dual::fire_button`]).
pub mod last_input {
    pub const GAMEPAD: u8 = 0;
    pub const MOUSEMOVE: u8 = 1;
    pub const MOUSECLICK: u8 = 2;
    pub const KEYBOARD: u8 = 3;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct T6Cmd {
    /// Frame length in ms (BO2 `pml.msec`).
    pub msec: i32,
    pub server_time: i32,
    /// The weapon the player has selected.
    pub weapon: u32,
    /// Offhand the player asked for, 0 = none.
    pub off_hand_weapon: u32,
    pub last_weapon_alt_mode_switch: u32,
    pub buttons: u32,
    /// The previous command's buttons (BO2 `oldcmd.buttons`), for presses that are edges.
    pub old_buttons: u32,
    /// The client's melee lunge target (`None` = BO2's 0x3ff, no target).
    pub melee_charge_ent: Option<u16>,
    /// Distance to the lunge target.
    pub melee_charge_dist: u8,
    /// BO2 `cmd.lastInput` ([`last_input`]).
    pub last_input: u8,
    pub forwardmove: i8,
    pub rightmove: i8,
}

impl T6Cmd {
    pub fn held(&self, bits: u32) -> bool {
        self.buttons & bits != 0
    }

    /// Held now and not held in the previous command.
    pub fn pressed(&self, bits: u32) -> bool {
        self.buttons & bits != 0 && self.old_buttons & bits == 0
    }

    /// Held in the previous command.
    pub fn was_held(&self, bits: u32) -> bool {
        self.old_buttons & bits != 0
    }
}
