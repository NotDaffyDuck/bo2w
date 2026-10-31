//! The MW2 player-state bits the moved weapon code still tests.
//!
//! TODO(fact): these are MW2's numbers, the ones the caller's player state still carries;
//! they go when that player state is BO2's. Same values as before the move.

pub mod ent_bit {
    pub const CROUCH_BIT: u32 = 0x4;
    pub const LYING_FLAT: u32 = 0x8;
    /// On a mounted turret (stand or crouch half, `turret_use`).
    pub const TURRET_MASK: u32 = 0x300;
    /// Riding a vehicle (`VEH_LinkPlayer`).
    pub const IN_VEHICLE: u32 = 0x4000;
}

pub mod secondary_state_bits {
    pub const PLAYER: u32 = 0x4;
}

pub mod movement_state_bits {
    pub const MANTLE: u32 = 0x4;
    pub const LADDER: u32 = 0x8;
    pub const SPRINTING: u32 = 0x8000;
}

pub mod weapon_state_bits {
    pub const AIM_DISABLED_BIT: u32 = 0x20;
}

pub mod buttons {
    pub const BASH_WINDUP: u32 = 0x4;
}
