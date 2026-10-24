//! One frame of player input, as the client sends it to the server.

/// A client's input for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UserCmd {
    /// The server time the client made this frame for.
    pub server_time: i32,
    /// Held buttons, a set of [`buttons`] bits.
    pub buttons: u32,
    /// Where the player looks, in the wire's integer angle form.
    pub angles: [i32; 3],
    /// The gun the player wants up.
    pub weapon: u16,
    pub weapon_mapped: u16,
    /// The grenade or equipment slot chosen.
    pub off_hand_index: u16,
    /// Stick or keys, forward/back and left/right.
    pub forwardmove: i8,
    pub rightmove: i8,
    /// The knife lunge: its direction, its distance, and who it is aimed at (sent with the lunge press).
    pub melee_charge_yaw: f32,
    pub melee_charge_dist: u8,
    pub melee_charge_ent: Option<u16>,
    /// The map spot picked for a streak that asks for one.
    pub selected_location: [u8; 3],
    /// Steering for a remote-controlled streak.
    pub remote_control: [u8; 2],
    /// What the player touched last ([`last_input`]). Dual wield swaps its triggers by it: with a mouse, the left
    /// button fires the left gun.
    pub last_input: u8,
}

/// Values of [`UserCmd::last_input`].
pub mod last_input {
    pub const GAMEPAD: u8 = 0;
    pub const MOUSECLICK: u8 = 2;
}

/// Declares each button as the bit it sits at.
macro_rules! button_at {
    ($($(#[$doc:meta])* $name:ident = $bit:literal,)*) => {
        $($(#[$doc])* pub const $name: u32 = 1 << $bit;)*
    };
}

/// Bits of [`UserCmd::buttons`].
pub mod buttons {
    button_at! {
        ATTACK = 0,
        SPRINT = 1,
        /// Knife lunge.
        MELEE_CHARGE = 2,
        USE = 3,
        RELOAD = 4,
        /// The shared use/reload button.
        USE_RELOAD = 5,
        PRONE = 8,
        CROUCH = 9,
        JUMP = 10,
        ADS = 11,
        /// The stance button is being held (as opposed to tapped).
        STANCE_HELD = 12,
        /// Hold breath while scoped.
        BREATH = 13,
        /// Lethal grenade.
        FRAG = 14,
        /// Tactical grenade.
        SMOKE = 15,
        /// Confirms a spot on the map for a streak.
        LOCATION_SELECT = 16,
        LOCATION_CANCEL = 17,
        THROW = 19,
        REMOTE_CONTROL = 20,
        OFFHAND_HOLD_CANCEL = 21,
        /// Change seat in a vehicle (next-inventory, wheel up); a gunner's thermal toggle listens to it too.
        CHANGE_SEAT = 22,
        /// Held on every frame while the player switches to the inventory gun (a held streak such as the care
        /// package); scripts ask for it by name.
        INVENTORY = 23,
        /// The client's quick-tap roll on a semi-auto came up: the gun jams instead of firing until fire is let go.
        /// The gun brain's own jam bit has the same value.
        JAM = 24,
        /// Two quick use/reload taps; a fresh press tells the scripts to fire the alternate detonation (C4) before
        /// the move runs.
        DOUBLE_TAP = 25,
    }
}
