//! The simulation's mutable state, grouped by what changes it.
//!
//! Every group is a plain struct with `pub(super)` fields: the files of this folder reach into them, nothing else
//! does. Only `recent_kills`, `weapon_notes` and `objectives` are open to the rest of the crate.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy_ecs::prelude::Component;

use super::catalog::{SimContent, SimContentBuilder};
use super::{ClientId, MantleBlocker, PendingLocalSound, PendingPlayerCardEvent, PendingPrint};
use crate::bullet_collision::{
    COLLISION_HISTORY_TICKS, CollisionHistory, EntityCollisionCapabilities, EntityCollisionHistory,
    ShotSampleProvenance,
};
use crate::identities::{EventSequence, MatchPhase, MatchRng, ProjectileId, RngDomain, ShotId};
use crate::match_state::{ClientMatchState, EntityEventRecord, EventRecord};
use crate::spawn::MatchBootstrap;
use crate::world_objects::WorldObjectState;

/// A built player skeleton that is kept between ticks, with what it was built from.
#[derive(Clone, Debug)]
pub(super) struct PlayerDobjSlot {
    pub(super) dobj: xmodel_runtime::DObj,
    pub(super) reuse_key: xmodel_runtime::DObjReuseKey,
    pub(super) persist: i64,
}

/// Where a prediction world keeps a player it does not simulate itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PredictionRemoteBody {
    pub client: ClientId,
    pub origin: [f32; 3],
    pub life_sequence: crate::LifeSequence,
}

/// The match clock.
#[derive(Clone, Debug)]
pub(super) struct Clock {
    pub(super) bootstrap: MatchBootstrap,
    pub(super) running: bool,
    pub(super) phase: MatchPhase,
    pub(super) elapsed_ms: u32,
    pub(super) prematch: gamemode_t6::PrematchStep,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            bootstrap: MatchBootstrap::default(),
            running: false,
            phase: MatchPhase::Warmup,
            elapsed_ms: 0,
            prematch: gamemode_t6::PrematchStep::default(),
        }
    }
}

/// The seed and one random stream per use, so a draw in one never moves another.
#[derive(Clone, Debug)]
pub(super) struct Dice {
    pub(super) seed: u64,
    pub(super) spawn: MatchRng,
    pub(super) combat: MatchRng,
    pub(super) bot: MatchRng,
}

impl Dice {
    pub(super) fn seeded(seed: u64) -> Self {
        Self {
            seed,
            spawn: MatchRng::from_root(seed, RngDomain::Spawn),
            combat: MatchRng::from_root(seed, RngDomain::Combat),
            bot: MatchRng::from_root(seed, RngDomain::Bot),
        }
    }
}

impl Default for Dice {
    fn default() -> Self {
        Self::seeded(0)
    }
}

/// The next number each kind of id hands out.
#[derive(Clone, Debug)]
pub(super) struct Serials {
    pub(super) shot: ShotId,
    pub(super) projectile: ProjectileId,
    pub(super) event: EventSequence,
    pub(super) entity_event: EventSequence,
}

impl Default for Serials {
    fn default() -> Self {
        Self {
            shot: ShotId(1),
            projectile: ProjectileId(1),
            event: EventSequence(1),
            entity_event: EventSequence(1),
        }
    }
}

/// The hash of what this match plays on, and the parts it was made from.
#[derive(Clone, Debug, Default)]
pub(super) struct Digest {
    pub(super) value: u64,
    pub(super) parts: crate::ContentComponents,
}

/// Who is connected, and what the simulation remembers about each one from the last command or move.
#[derive(Clone, Debug)]
pub(super) struct Roster {
    pub(super) members: Vec<(ClientId, ClientMatchState)>,
    pub(super) prev_buttons: Vec<(ClientId, u32)>,
    pub(super) prev_angles: Vec<(ClientId, [i32; 3])>,
    /// Forward and side keys of the last command (the prone aim hold reads them).
    pub(super) prev_moves: Vec<(ClientId, [i8; 2])>,
    pub(super) walking: HashMap<ClientId, i32>,
    /// The stair step height of the last move, which eases the camera.
    pub(super) view_change: HashMap<ClientId, f32>,
    pub(super) holdrand: u32,
    pub(super) movetype: HashMap<ClientId, u8>,
    pub(super) anim_seed: u32,
}

impl Default for Roster {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            prev_buttons: Vec::new(),
            prev_angles: Vec::new(),
            prev_moves: Vec::new(),
            walking: HashMap::new(),
            view_change: HashMap::new(),
            holdrand: 0,
            movetype: HashMap::new(),
            anim_seed: 1,
        }
    }
}

/// What lag compensation reads: recent player and entity poses, and which command each shot came from.
#[derive(Clone, Debug)]
pub(super) struct Replay {
    pub(super) history: CollisionHistory,
    pub(super) entity_history: EntityCollisionHistory,
    pub(super) sample: HashMap<ClientId, ShotSampleProvenance>,
    pub(super) commands: HashMap<(ClientId, i32), ShotSampleProvenance>,
    /// Client prediction only: where this client draws the other living players (origin, velocity), for the knife
    /// lunge's target. The prediction world holds no other players of its own.
    pub(super) lunge_seen: HashMap<ClientId, ([f32; 3], [f32; 3])>,
}

impl Default for Replay {
    fn default() -> Self {
        Self {
            history: CollisionHistory::with_capacity(COLLISION_HISTORY_TICKS),
            entity_history: EntityCollisionHistory::with_capacity(COLLISION_HISTORY_TICKS),
            sample: HashMap::new(),
            commands: HashMap::new(),
            lunge_seen: HashMap::new(),
        }
    }
}

/// Player bodies: the built skeletons, the last failure to build one, and the remote players of a prediction world.
#[derive(Clone, Debug, Default)]
pub(super) struct Bodies {
    pub(super) dobjs: HashMap<u32, PlayerDobjSlot>,
    pub(super) materialize_error: Option<String>,
    pub(super) remote: Vec<PredictionRemoteBody>,
}

/// Where things are: the area grid, the entity kernel, collision owners and the model library.
#[derive(Clone, Debug, Default)]
pub(super) struct Space {
    pub(super) area: Option<crate::area_world::AreaWorld>,
    pub(super) caps: Vec<EntityCollisionCapabilities>,
    pub(super) models: Arc<BTreeMap<String, Option<Arc<xmodel_runtime::RetainedModelCapability>>>>,
    pub(super) kernel: crate::gentity::EntityKernel,
    pub(super) phases: Vec<crate::gentity::KernelPhase>,
    pub(super) dying_missiles: Vec<entity_t6::EntityState>,
}

/// Everything the tick produces for the outside to pick up.
#[derive(Clone, Debug, Default)]
pub(super) struct Outbox {
    pub(super) journal: Vec<EventRecord>,
    pub(super) entity_events: Vec<EntityEventRecord>,
    pub(super) pellet_fx: Vec<crate::PelletFxRecord>,
    /// The pieces that broke off this tick.
    pub(super) debris: Vec<crate::DebrisRecord>,
    pub(super) cards: Vec<PendingPlayerCardEvent>,
    pub(super) prints: Vec<PendingPrint>,
    pub(super) sounds: Vec<PendingLocalSound>,
    pub(super) pickups: Vec<crate::ItemPickupRecord>,
}

/// Boxes and rules the scripts changed at run time.
#[derive(Clone, Debug, Default)]
pub(super) struct Blockers {
    /// Solid turned boxes.
    pub(super) oriented: Vec<crate::OrientedBlocker>,
    /// Rows of `setenemymodel`.
    pub(super) enemy_models: Vec<crate::EnemyModel>,
    /// Boxes a script barred tactical insertions from, as world mins and maxs.
    pub(super) tac_insert: Vec<([f32; 3], [f32; 3])>,
    /// Where each climber lands, held against the others.
    pub(super) mantle: Vec<MantleBlocker>,
    /// The movement rules: the game's own, or on the guessing side the ones from the last snapshot.
    pub(super) feel: Option<movement_t6::bridge::Bo2Feel>,
}

/// The four config-string tables that hand out indices by name.
#[derive(Clone, Debug, Default)]
pub(super) struct Strings {
    pub(super) sound_alias: crate::SoundAliasCs,
    pub(super) effect_name: crate::EffectNameCs,
    pub(super) hud_material: crate::HudMaterialCs,
    pub(super) hud_string: crate::HudStringCs,
}

/// The script-made HUD elements.
#[derive(Clone, Debug, Default)]
pub(super) struct HudSlots {
    pub(super) elems: Vec<crate::hudelem::GameHudElemSlot>,
    pub(super) pulse_sounds: crate::hudelem::PulseFxSoundIds,
}

#[derive(Component, Clone, Debug)]
pub struct SimState {
    pub(super) content: Arc<SimContent>,
    pub(super) clock: Clock,
    pub(super) dice: Dice,
    pub(super) serials: Serials,
    pub(super) digest: Digest,
    pub(super) roster: Roster,
    pub(super) replay: Replay,
    pub(super) bodies: Bodies,
    pub(super) space: Space,
    pub(super) outbox: Outbox,
    pub(super) blockers: Blockers,
    pub(super) strings: Strings,
    pub(super) hud: HudSlots,
    pub(super) world_objects: WorldObjectState,
    pub(super) corpses: crate::PlayerCorpsePool,
    pub(super) publish_snapshot: bool,

    pub(crate) recent_kills: Vec<(ClientId, i32, u32)>,
    pub(crate) weapon_notes: Vec<crate::equipment::WeaponNote>,
    pub objectives: crate::ObjectiveMatch,
}

impl Default for SimState {
    fn default() -> Self {
        let mut state = Self {
            content: SimContentBuilder::default().finish(),
            clock: Clock::default(),
            dice: Dice::default(),
            serials: Serials::default(),
            digest: Digest::default(),
            roster: Roster::default(),
            replay: Replay::default(),
            bodies: Bodies::default(),
            space: Space::default(),
            outbox: Outbox::default(),
            blockers: Blockers::default(),
            strings: Strings::default(),
            hud: HudSlots::default(),
            world_objects: WorldObjectState::default(),
            corpses: crate::PlayerCorpsePool::default(),
            publish_snapshot: true,
            recent_kills: Vec::new(),
            weapon_notes: Vec::new(),
            objectives: crate::ObjectiveMatch::default(),
        };
        state.refresh_digest();
        state
    }
}
