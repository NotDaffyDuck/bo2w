//! The simulation's mutable state, grouped by what changes it.
//!
//! Every group is a plain struct with `pub(super)` fields: the files of this folder reach into them, nothing else
//! does. Only `latest_frag_log`, `weapon_notes` and `objectives` are open to the rest of the crate.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy_ecs::prelude::Component;

use super::content_book::{SimContent, SimContentBuilder};
use super::{SeatNo, MantleBlocker, PendingLocalSound, QueuedCallingCardEvent, PendingPrint};
use crate::shot_trace::{
    HITBOX_TIMELINE_LENGTH, HitboxTimeline, EntityHitAbilities, EntityHitboxTimeline,
    ShotRecordOrigin,
};
use crate::match_ids::{EventStamp, GameRoundStage, MatchRandom, ThrownId, RandomStreamKind, BulletNo};
use crate::player_ledger::{PlayerMatchRecord, ThingHappening, OccurrenceEntry};
use crate::spawn_pick::MatchStartSetup;
use crate::glass_and_props::MapPropState;

/// A built player skeleton that is kept between ticks, with what it was built from.
#[derive(Clone, Debug)]
pub(super) struct AvatarModelSlot {
    pub(super) dobj: xmodel_runtime::PosedSkeleton,
    pub(super) reuse_key: xmodel_runtime::SkeletonCacheKey,
    pub(super) persist: i64,
}

/// Where a prediction world keeps a player it does not simulate itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PredictedRemotePlayer {
    pub client: SeatNo,
    pub spot: [f32; 3],
    pub life_sequence: crate::LifeCount,
}

/// The match clock.
#[derive(Clone, Debug)]
pub(super) struct Clock {
    pub(super) bootstrap: MatchStartSetup,
    pub(super) going: bool,
    pub(super) phase: GameRoundStage,
    pub(super) elapsed_ms: u32,
    pub(super) prematch: gamemode_t6::WarmupStage,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            bootstrap: MatchStartSetup::default(),
            going: false,
            phase: GameRoundStage::Warmup,
            elapsed_ms: 0,
            prematch: gamemode_t6::WarmupStage::default(),
        }
    }
}

/// The seed and one random stream per use, so a draw in one never moves another.
#[derive(Clone, Debug)]
pub(super) struct Dice {
    pub(super) seed: u64,
    pub(super) spawn: MatchRandom,
    pub(super) combat: MatchRandom,
    pub(super) bot: MatchRandom,
}

impl Dice {
    pub(super) fn seeded(seed: u64) -> Self {
        Self {
            seed,
            spawn: MatchRandom::from_root(seed, RandomStreamKind::Spawn),
            combat: MatchRandom::from_root(seed, RandomStreamKind::Combat),
            bot: MatchRandom::from_root(seed, RandomStreamKind::Bot),
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
    pub(super) shot: BulletNo,
    pub(super) projectile: ThrownId,
    pub(super) event: EventStamp,
    pub(super) entity_event: EventStamp,
}

impl Default for Serials {
    fn default() -> Self {
        Self {
            shot: BulletNo(1),
            projectile: ThrownId(1),
            event: EventStamp(1),
            entity_event: EventStamp(1),
        }
    }
}

/// The hash of what this match plays on, and the parts it was made from.
#[derive(Clone, Debug, Default)]
pub(super) struct Digest {
    pub(super) value: u64,
    pub(super) parts: crate::ContentPartDigests,
}

/// Who is connected, and what the simulation remembers about each one from the last command or move.
#[derive(Clone, Debug)]
pub(super) struct Roster {
    pub(super) members: Vec<(SeatNo, PlayerMatchRecord)>,
    pub(super) prev_buttons: Vec<(SeatNo, u32)>,
    pub(super) prev_angles: Vec<(SeatNo, [i32; 3])>,
    /// Forward and side keys of the last command (the prone aim hold reads them).
    pub(super) prev_moves: Vec<(SeatNo, [i8; 2])>,
    pub(super) walking: HashMap<SeatNo, i32>,
    /// The stair step height of the last move, which eases the camera.
    pub(super) view_change: HashMap<SeatNo, f32>,
    pub(super) rand_state: u32,
    pub(super) movetype: HashMap<SeatNo, u8>,
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
            rand_state: 0,
            movetype: HashMap::new(),
            anim_seed: 1,
        }
    }
}

/// What lag compensation reads: recent player and entity poses, and which command each shot came from.
#[derive(Clone, Debug)]
pub(super) struct Replay {
    pub(super) history: HitboxTimeline,
    pub(super) entity_history: EntityHitboxTimeline,
    pub(super) sample: HashMap<SeatNo, ShotRecordOrigin>,
    pub(super) commands: HashMap<(SeatNo, i32), ShotRecordOrigin>,
    /// Client prediction only: where this client draws the other living players (origin, velocity), for the knife
    /// lunge's target. The prediction world holds no other players of its own.
    pub(super) lunge_seen: HashMap<SeatNo, ([f32; 3], [f32; 3])>,
}

impl Default for Replay {
    fn default() -> Self {
        Self {
            history: HitboxTimeline::with_capacity(HITBOX_TIMELINE_LENGTH),
            entity_history: EntityHitboxTimeline::with_capacity(HITBOX_TIMELINE_LENGTH),
            sample: HashMap::new(),
            commands: HashMap::new(),
            lunge_seen: HashMap::new(),
        }
    }
}

/// Player bodies: the built skeletons, the last failure to build one, and the remote players of a prediction world.
#[derive(Clone, Debug, Default)]
pub(super) struct Bodies {
    pub(super) skeletons: HashMap<u32, AvatarModelSlot>,
    pub(super) bake_failure: Option<String>,
    pub(super) remote: Vec<PredictedRemotePlayer>,
}

/// Where things are: the area grid, the entity kernel, collision owners and the model library.
#[derive(Clone, Debug, Default)]
pub(super) struct Space {
    pub(super) area: Option<crate::sector_links::AreaWorld>,
    pub(super) caps: Vec<EntityHitAbilities>,
    pub(super) models: Arc<BTreeMap<String, Option<Arc<xmodel_runtime::KeptModelAbility>>>>,
    pub(super) kernel: crate::ent_slots::EntitySlotArena,
    pub(super) phases: Vec<crate::ent_slots::EntityPassStage>,
    pub(super) dying_missiles: Vec<entity_t6::EntitySnapshot>,
}

/// Everything the tick produces for the outside to pick up.
#[derive(Clone, Debug, Default)]
pub(super) struct Outbox {
    pub(super) journal: Vec<OccurrenceEntry>,
    pub(super) entity_events: Vec<ThingHappening>,
    pub(super) pellet_fx: Vec<crate::ScatterPuff>,
    /// The pieces that broke off this tick.
    pub(super) debris: Vec<crate::DebrisRecord>,
    pub(super) cards: Vec<QueuedCallingCardEvent>,
    pub(super) prints: Vec<PendingPrint>,
    pub(super) sounds: Vec<PendingLocalSound>,
    pub(super) pickups: Vec<crate::GrabbedItemNote>,
}

/// Boxes and rules the scripts changed at run time.
#[derive(Clone, Debug, Default)]
pub(super) struct Blockers {
    /// Solid turned boxes.
    pub(super) oriented: Vec<crate::OrientedBlocker>,
    /// (spectator, the teammate he watches), for a player who bled out and sees through a living teammate's eyes.
    pub(super) follows: Vec<(SeatNo, SeatNo)>,
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
    pub(super) sound_alias: crate::SoundAliasConfigTable,
    pub(super) effect_label: crate::SoundAliasConfigTable,
    pub(super) hud_material: crate::SoundAliasConfigTable,
    pub(super) hud_string: crate::HudStringCs,
}

/// The script-made HUD elements.
#[derive(Clone, Debug, Default)]
pub(super) struct HudSlots {
    pub(super) elems: Vec<crate::script_overlay::ScriptHudSlot>,
    pub(super) pulse_sounds: crate::script_overlay::PulseEffectSoundSet,
}

#[derive(Component, Clone, Debug)]
pub struct MatchSimulation {
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
    pub(super) world_objects: MapPropState,
    pub(super) corpses: crate::CorpseYard,
    pub(super) is_authority: bool,

    pub(crate) latest_frag_log: Vec<(SeatNo, i32, u32)>,
    pub(crate) weapon_notes: Vec<crate::gear::WeaponNote>,
    pub objectives: crate::MatchBoard,
}

impl Default for MatchSimulation {
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
            world_objects: MapPropState::default(),
            corpses: crate::CorpseYard::default(),
            is_authority: true,
            latest_frag_log: Vec::new(),
            weapon_notes: Vec::new(),
            objectives: crate::MatchBoard::default(),
        };
        state.refresh_digest();
        state
    }
}
