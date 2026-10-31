//! One frame of player input, as the client sends it to the server.

/// A client's input for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputCommand {
    /// The server time the client made this frame for.
    pub authority_clock_ms: i32,
    /// Held buttons, a set of [`buttons`] bits.
    pub buttons: u32,
    /// Where the player looks, in the wire's integer angle form.
    pub angles: [i32; 3],
    /// The gun the player wants up.
    pub weapon: u16,
    pub weapon_slot_code: u16,
    /// The grenade or equipment slot chosen.
    pub secondary_hand_slot: u16,
    /// Stick or keys, forward/back and left/right.
    pub thrust_ahead: i8,
    pub thrust_lateral: i8,
    /// The knife lunge: its direction, its distance, and who it is aimed at (sent with the lunge press).
    pub lunge_heading: f32,
    pub lunge_reach: u8,
    pub melee_charge_ent: Option<u16>,
    /// The map spot picked for a streak that asks for one.
    pub chosen_map_spot: [u8; 3],
    /// Steering for a remote-controlled streak.
    pub remote_operated: [u8; 2],
    /// What the player touched last ([`last_input`]). Dual wield swaps its triggers by it: with a mouse, the left
    /// button fires the left gun.
    pub last_input: u8,
}

/// Values of [`InputCommand::last_input`].
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

/// Bits of [`InputCommand::buttons`].
pub mod buttons {
    button_at! {
        ATTACK = 0,
        SPRINT = 1,
        /// Knife lunge.
        MELEE_CHARGE = 2,
        INTERACT_KEY = 3,
        RELOAD = 4,
        /// The shared use/reload button.
        INTERACT_RELOAD_KEY = 5,
        LYING_FLAT = 8,
        DUCK_BUTTON_BIT = 9,
        LEAP = 10,
        ADS = 11,
        /// The stance button is being held (as opposed to tapped).
        STANCE_LOCK_BIT = 12,
        /// Hold breath while scoped.
        STEADY_AIM_HOLD_BIT = 13,
        /// Lethal grenade.
        SAMPLE_FRAG_ASSET = 14,
        /// Tactical grenade.
        OBSCURANT = 15,
        /// Confirms a spot on the map for a streak.
        MAP_TARGET_PICK = 16,
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
