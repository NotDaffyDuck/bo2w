//! The match-wide side information published with every snapshot.

mod enemy;
mod rng;

pub use rng::{DiceTrace, RngDebugMeta};

use super::blocker::OrientedBlocker;
use super::client_meta::SeatTickNotes;
use super::dvars::ScriptDvars;
use super::enemy_model::EnemyModel;
use super::records::DebrisRecord;
use crate::sim_parts::SeatNo;

/// Everything a snapshot says about the match beyond the clients' own entity states. Which fields reach a peer is
/// decided by the snapshot wire format, not here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickLedgerNotes {
    // Match state.
    pub phase: crate::match_ids::GameRoundStage,
    pub kind: gamemode_t6::RulesetKind,
    pub prematch: gamemode_t6::WarmupStage,
    pub match_clock_ms: u32,
    pub score_limit: i32,
    pub time_limit_ms: u32,
    pub objectives: crate::MatchBoard,
    pub clients: Vec<(SeatNo, SeatTickNotes)>,

    // Events of this tick.
    pub journal: Vec<super::records::OccurrenceEntry>,
    pub entity_events: Vec<super::records::ThingHappening>,
    pub pellet_fx: Vec<super::records::ScatterPuff>,
    /// Pieces that broke off something this tick.
    pub debris: Vec<DebrisRecord>,

    // Config strings: the slots taken in each table.
    pub sound_aliases: Vec<(u8, String)>,
    pub effect_names: Vec<(u8, String)>,
    pub hud_materials: Vec<(u8, String)>,
    pub hud_strings: Vec<(u16, String)>,

    /// Where each random stream has got to; compared across machines when hunting a desync.
    pub rng: rng::DiceTrace,

    // The world.
    pub world_objects: crate::glass_and_props::PropsSnapshot,
    pub area_entities: Option<crate::ZoneThingTable>,
    pub entity_kernel: crate::FrozenSlotTable,
    pub entities: Vec<entity_t6::EntitySnapshot>,
    pub entity_dobjs: Vec<(crate::ServerModelSource, xmodel_runtime::RigPoseRecord)>,
    pub script_movers: Vec<crate::MoverSlot>,
    pub corpses: crate::CorpseYard,
    pub item_ammo: Vec<super::pickups::DroppedRounds>,
    pub item_pickups: Vec<super::pickups::GrabbedItemNote>,
    /// Solid collision models (Nuketown's patch walls, perk machines, the Mystery Box) as turned boxes. Movement on
    /// both sides clips against them.
    pub oriented_blockers: Vec<OrientedBlocker>,
    /// (spectator, the teammate they watch): a player who bled out in co-op sees through a living teammate's eyes
    /// (BO2's `SpectatorClientEndFrame`). Server only, not on the wire.
    pub follows: Vec<(SeatNo, SeatNo)>,
    /// The Black Ops II movement rules the game runs with. A second PC predicts its own moves without running scripts
    /// and needs them. `None` until the rules have been sent.
    pub feel: Option<movement_t6::bridge::Bo2Feel>,
    /// Script models that show the other side a different model (BO2's `setenemymodel`). Each viewer's copy swaps them
    /// in with `swap_enemy_models`; none of these rows go out.
    pub enemy_models: Vec<EnemyModel>,
}
/// Older spelling SnapshotMeta still calls; the rename sweep moves it over.
pub use self::TickLedgerNotes as SnapshotMeta;

impl TickLedgerNotes {
    /// The meta of client `id`, if the snapshot has one.
    pub fn for_client(&self, who: SeatNo) -> Option<&SeatTickNotes> {
        self.clients.iter().find_map(|(owner, meta)| (*owner == who).then_some(meta))
    }

    /// The dvars a script reads for `id`: the client's own first, then the server's.
    pub fn script_dvars(&self, id: SeatNo) -> ScriptDvars<'_> {
        let own = self.for_client(id).map_or(&[][..], |meta| meta.client_dvars.as_slice());
        ScriptDvars::new(own, &self.objectives.server_info)
    }
}
