//! The MW2 player-state bits the moved weapon code still tests.
//!
//! TODO(fact): these are MW2's numbers, the ones the caller's player state still carries;
//! they go when that player state is BO2's. Same values as before the move.

pub mod eflags {
    pub const DUCK: u32 = 0x4;
    pub const PRONE: u32 = 0x8;
}

pub mod other_flags {
    pub const PLAYER: u32 = 0x4;
}

pub mod pm_flags {
    pub const MANTLE: u32 = 0x4;
    pub const LADDER: u32 = 0x8;
    pub const SPRINTING: u32 = 0x8000;
}

pub mod weap_flags {
    pub const NO_ADS: u32 = 0x20;
}

pub mod buttons {
    pub const MELEE_CHARGE: u32 = 0x4;
}
