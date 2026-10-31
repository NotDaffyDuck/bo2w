//! Per-client match state and what the server publishes about it each snapshot.

mod audience;
mod blocker;
mod client;
mod client_meta;
mod dvars;
mod enemy_model;
mod loadout;
mod menu_cmds;
mod meta_sync;
mod pickups;
mod radar;
mod reasons;
mod records;
mod remote;
mod seat_script;
mod sim_event;
mod snapshot_meta;
mod screen_fx;

pub use audience::EventRecipients;
pub use blocker::OrientedBlocker;
pub use client::{PlayerMatchRecord, InputReceipt, ScriptControls};
pub use client_meta::{SeatTickNotes, ClientSnapshotMeta, T6ViewInputs};
pub use dvars::ScriptDvars;
pub use enemy_model::EnemyModel;
pub use loadout::{
    LoadoutClassSpec, LoadoutRefusal, ClassRejectReason, ConfigurationChangeRejectReason, HandoutRefusal, GiveRejectReason, KitPlan, LoadoutSpec,
    PERSONAL_CLASS_SLOTS, PersonalClass,
};
pub use menu_cmds::{LocationSelection, MENU_COMMAND_TAIL, MenuOrder, MenuCommandKind};
pub use pickups::{DroppedRounds, DroppedItemAmmo, GrabbedItemNote, ItemPickupRecord};
pub use radar::RadarMode;
pub use records::{DebrisRecord, HappeningBody, EntityEventPayload, ThingHappening, EntityEventRecord, OccurrenceEntry, EventRecord, ScatterPuff, PelletFxRecord};
pub use remote::SteeredMissile;
pub use seat_script::{DeathReelBoard, KillcamHud, ScriptSeat};
pub use sim_event::{OCCURRENCE_TABLE, MatchOccurrence, OccurrenceTableRow, occurrence_needs_delivery};
pub use snapshot_meta::{DiceTrace, RngDebugMeta, TickLedgerNotes, SnapshotMeta};
pub use screen_fx::{ScriptBlur, ScriptDepthOfField, ViewEffects, VisionChange, is_postfx_dvar};

/// Where a client is between connecting and the end of the match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerSessionStage {
    #[default]
    Connecting,
    PickingLoadout,
    AwaitingFirstLife,
    Alive,
    Dead,
    AwaitingNextLife,
    Spectating,
    Intermission,
}

/// What ended the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchOverCause {
    PointsReached,
    TimeLimit,
}
