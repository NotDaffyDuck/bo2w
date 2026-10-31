//! The simulation's world: one [`MatchSimulation`] holds a match, and the files of this folder are its parts.
//!
//! * `state`: the fields, grouped by what changes them.
//! * `catalog`: the frozen map and weapon content a state plays on.
//! * `whole_match`, `roster`, `space`: the clock and ids, the connected clients, the entity space.
//! * `sweep`: traces through the map's clip data.
//! * `rewind`, `bodies`: lag compensation and the player bodies it rewinds.
//! * `sync`: snapshots out and snapshots adopted.
//! * `outbox`: what a tick leaves for the outside to take.
//! * `census`: debug dumps of collision.
//! * `loadout`: player-state helpers for giving weapons.

mod bodies;
mod content_book;
mod census;
mod loadout;
mod outbox;
mod rewind;
mod roster;
mod whole_match;
mod space;
mod state;
mod sweep;
mod sync;
#[cfg(test)]
mod tests;

pub use content_book::{
    PlayerOutfitHitbox, SolidBrush, CollisionTreeData, CollisionSubmodels, CollisionTriangleData, SimContent, SimContentBuilder,
    SimTriggerHull, WeaponPart, WeaponRawKinds, WeaponScriptSounds, WeaponSetup,
};
pub use census::HitboxReportRow;
pub(crate) use loadout::{grant_dual_wield_weapon, script_gun_wants_two_hands, pack_weapon, fresh_avatar_at};
pub use loadout::empty_avatar;
pub use state::MatchSimulation;
#[cfg(test)]
pub(crate) use sweep::clip_move_to_bmodels;
pub(crate) use sweep::{
    T6Model, clip_move_to_bmodels_struck, linked_model, standing_pane_struck, sweep_box_trace, t6_brushes_trace, t6_static_trace,
    t6_transformed_trace,
};

/// A simulation tick number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StepNo(pub u32);

/// A connected client's number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeatNo(pub u32);

/// What a player card is told to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueuedCallingCardKind {
    SetSlot { source: SeatNo, slot: i32 },
    OpenMenu { config_slot_number: i32 },
    Splash { card_key: String, slot: i32, spare: i32 },
}

/// A player-card instruction waiting for its recipient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedCallingCardEvent {
    pub recipient: SeatNo,
    pub kind: QueuedCallingCardKind,
}

/// A line of text a script printed, waiting to go to the console of `recipient` (everyone when none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPrint {
    pub recipient: Option<SeatNo>,
    pub bold: bool,
    pub template: String,
    pub arg: String,
}

/// A sound only one client hears, started or stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingLocalSound {
    pub recipient: SeatNo,
    pub halt: bool,
    pub alias_index: u8,
}

/// Collision contents of a player's body.
pub(crate) const SOLID_PLAYER_BODY: u32 = 0x0200_0000;
/// Collision contents that stop players only (the player trace mask has it).
pub(crate) const CONTENTS_PLAYERCLIP: u32 = 0x0001_0000;

/// A box held where a climb ends: the climber's own traces pass through it, everybody else's stop at it, until
/// [`MANTLE_BLOCK_TIME_BUFFER_MS`] after the climb is over, so nobody takes the spot first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MantleBlocker {
    /// The climber's client number.
    pub owner: u16,
    /// Where the climb ends.
    pub spot: [f32; 3],
    /// Level time the climb ends.
    pub ends: i32,
    /// Level time the box goes.
    pub until: i32,
}

/// How long after a climb ends its landing box stays (ms).
pub(crate) const MANTLE_BLOCK_TIME_BUFFER_MS: i32 = 500;

/// Older spellings other crates still call; the rename sweep moves them over.
pub use self::SeatNo as ClientId;
pub use self::StepNo as Tick;
