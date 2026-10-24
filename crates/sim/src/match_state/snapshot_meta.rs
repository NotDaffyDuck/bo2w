//! The match-wide side information published with every snapshot.

use super::client_meta::ClientSnapshotMeta;
use super::dvars::ScriptDvars;
use super::enemy_model::EnemyModel;
use super::pickups::{DroppedItemAmmo, ItemPickupRecord};
use super::blocker::OrientedBlocker;
use super::records::{DebrisRecord, EntityEventRecord, EventRecord, PelletFxRecord};
use crate::identities::MatchPhase;
use crate::world::ClientId;
use crate::world_objects::WorldObjectSnapshot;

/// How far the random streams have been drawn, kept for desync hunting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RngDebugMeta {
    pub root_seed: u64,
    pub scheme: u32,
    pub spawn_draws: u64,
    pub combat_draws: u64,
    pub bot_draws: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapshotMeta {
    // The match.
    pub objectives: crate::ObjectiveMatch,
    pub phase: MatchPhase,
    pub match_elapsed_ms: u32,
    pub prematch: gamemode_t6::PrematchStep,
    pub score_limit: i32,
    pub time_limit_ms: u32,
    pub kind: gamemode_t6::GameModeKind,
    pub clients: Vec<(ClientId, ClientSnapshotMeta)>,

    // This tick's events.
    pub journal: Vec<EventRecord>,
    pub entity_events: Vec<EntityEventRecord>,
    pub pellet_fx: Vec<PelletFxRecord>,
    /// Pieces that broke off something this tick.
    pub debris: Vec<DebrisRecord>,

    // Which configstring slots are taken.
    pub sound_aliases: crate::SoundAliasCsOccupied,
    pub effect_names: crate::EffectNameCsOccupied,
    pub hud_materials: crate::HudMaterialCsOccupied,
    pub hud_strings: crate::HudStringCsOccupied,

    pub rng: RngDebugMeta,

    // The world and what stands in it.
    pub world_objects: WorldObjectSnapshot,
    pub area_entities: Option<crate::AreaEntityWorldSnapshot>,
    pub entity_dobjs: Vec<(crate::AuthorityModelOwner, xmodel_runtime::DObjSemanticState)>,
    pub entities: Vec<entity_t6::EntityState>,
    pub script_movers: Vec<crate::ScriptMoverGentity>,
    pub entity_kernel: crate::EntityKernelSnapshot,
    pub corpses: crate::PlayerCorpsePool,
    pub item_ammo: Vec<DroppedItemAmmo>,
    pub item_pickups: Vec<ItemPickupRecord>,
    /// Solid collision models (Nuketown's patch walls, perk machines, the Mystery Box) as turned boxes; player
    /// movement on both sides clips against them.
    pub oriented_blockers: Vec<OrientedBlocker>,
    /// The Black Ops II movement rules the game runs with, so a second PC's own guess of its moves (it runs no
    /// scripts) follows them too. `None` until the rules have been sent.
    pub feel: Option<movement_t6::bridge::Bo2Feel>,
    /// Script models that show the other side another model (BO2's `setenemymodel`). Each viewer's own copy swaps
    /// them in (`net::policy::seat`); none go out.
    pub enemy_models: Vec<EnemyModel>,
}

impl SnapshotMeta {
    pub fn for_client(&self, id: ClientId) -> Option<&ClientSnapshotMeta> {
        self.clients.iter().find(|(owner, _)| *owner == id).map(|(_, meta)| meta)
    }

    /// The dvars a script reads for `id`: that client's own, then the server's.
    pub fn script_dvars(&self, id: ClientId) -> ScriptDvars<'_> {
        let own: &[(String, String)] = match self.for_client(id) {
            Some(meta) => &meta.client_dvars,
            None => &[],
        };
        ScriptDvars::new(own, &self.objectives.server_info)
    }

    /// Re-dresses the script models for `viewer`: each model whose side is not the viewer's, and that is not the
    /// viewer's own, takes its enemy look. The rows are used up.
    pub fn swap_enemy_models(&mut self, viewer: ClientId) {
        if self.enemy_models.is_empty() {
            return;
        }
        let viewer_team = self.for_client(viewer).map_or(entity_t6::team::FREE, |meta| meta.client_state_team);
        for row in std::mem::take(&mut self.enemy_models) {
            if row.shows_own_look_to(viewer, viewer_team) {
                continue;
            }
            for (_, state) in self.entity_dobjs.iter_mut().filter(|(owner, _)| owner.script_model() == Some(row.id)) {
                let composition = &mut state.composition;
                match composition.models.first_mut() {
                    Some(base) if base.model != row.model => {
                        base.model = row.model.clone();
                        composition.revision ^= 0x8000_0000;
                    }
                    _ => {}
                }
            }
        }
    }
}
