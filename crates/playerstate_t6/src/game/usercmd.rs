#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UserCmd {
    pub server_time: i32,
    pub buttons: u32,
    pub angles: [i32; 3],
    pub weapon: u16,
    pub weapon_mapped: u16,
    pub off_hand_index: u16,
    pub forwardmove: i8,
    pub rightmove: i8,
    pub melee_charge_yaw: f32,
    pub melee_charge_dist: u8,
    /// bo2mp: the knife lunge's target (BO2 sends the entity with the lunge press).
    pub melee_charge_ent: Option<u16>,
    pub selected_location: [u8; 3],
    pub remote_control: [u8; 2],
    /// bo2mp: the device the player used last ([`last_input`]); BO2 swaps the dual-wield
    /// triggers by it (a mouse's left button fires the left gun).
    pub last_input: u8,
}

/// bo2mp: BO2's `lastInput` values.
pub mod last_input {
    pub const GAMEPAD: u8 = 0;
    pub const MOUSECLICK: u8 = 2;
}

pub mod buttons {
    pub const ATTACK: u32 = 0x1;

    pub const SPRINT: u32 = 1 << 1;

    pub const MELEE_CHARGE: u32 = 0x4;

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

    pub const LOCATION_CANCEL: u32 = 0x20000;

    pub const THROW: u32 = 0x80000;

    pub const REMOTE_CONTROL: u32 = 0x100000;

    pub const OFFHAND_HOLD_CANCEL: u32 = 0x200000;

    /// The vehicle seat button (`+weapnext_inventory`, wheel up): the
    /// gunner's FLIR toggle polls it through `changeseatbuttonpressed`.
    pub const CHANGE_SEAT: u32 = 0x400000;

    /// Black Ops II's inventory button (its own bit is usercmd word 1, 0x400):
    /// set on each command while the client is switching to the inventory
    /// weapon (the care package / streak held on switch-weapon). Script reads
    /// it through `inventorybuttonpressed`.
    pub const INVENTORY: u32 = 0x800000;

    /// bo2mp gun brain: Black Ops II's jam button (its own bit is usercmd
    /// word 1, 0x2000000): the client's semi-auto quick-tap roll came up,
    /// held until the fire key is let go. The BO2 gun jams instead of
    /// firing (weapon_t6 `cmd::button::JAM`, same value).
    pub const JAM: u32 = 0x1000000;

    /// bo2mp: Black Ops II's double-tap button (its own bit is usercmd word 1, 0x2000): two
    /// quick Use/Reload taps on the client. A new press tells the script `doubletap_detonate`
    /// (C4's alt detonate) before the move.
    pub const DOUBLE_TAP: u32 = 0x2000000;

    pub const SPRINT_INTERFERING: u32 = 0xcc35;
}
