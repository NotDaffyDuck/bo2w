//! What a bullet can run into, what a trace reports, and the per-segment record a firing leaves behind.

use trace_t6::{TraceHitType, surface_type_from_flags};
use weapon_t6::penetration::{SURF_TYPE_FLESH, depth_surface_type};

use crate::identities::{LifeSequence, ScriptModelId};
use crate::world::ClientId;

/// The hit location of a bullet a riot shield stopped ("riotshield" in BO2's scripts).
pub const HITLOC_SHIELD: u8 = weapon_t6::hitloc::HITLOC_RIOTSHIELD;
/// BO2's surface index for a riot shield: its impact effects play and nothing goes through. (The older surface list
/// puts it at 29, which BO2 uses for "player".)
pub const SURF_TYPE_RIOTSHIELD: u32 = 31;
/// The sky bit in a surface-flags word.
pub const SURF_SKY: u32 = 0x4;
/// A bullet reaches this far into a sky brush whatever the penetration table says.
pub const SKY_PENETRATION_DEPTH: f32 = 100.0;

/// The kind of hit a segment reports for an entity, and for the world, in BO2's numbering.
pub(super) const HITTYPE_ENTITY: i32 = TraceHitType::Entity as i32;
pub(super) const HITTYPE_GLASS: i32 = TraceHitType::Glass as i32;

/// Who owns a scripted model's collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityModelOwner {
    ScriptModel(ScriptModelId),
}

impl AuthorityModelOwner {
    pub const fn script_model(self) -> Option<ScriptModelId> {
        match self {
            Self::ScriptModel(id) => Some(id),
        }
    }

    /// The number this owner sorts by when two hits are equally far.
    pub(super) const fn order_key(self) -> u32 {
        match self {
            Self::ScriptModel(id) => id.0,
        }
    }
}

/// The thing a trace ended on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColliderId {
    World { surface_flags: u32, contents: u32, glass_encoded: u16 },
    Player { client: ClientId, life: LifeSequence, hitloc: u8 },
    EntityDObjBone { owner: AuthorityModelOwner, bone: u16, part_classification: u8, surface_flags: u32 },
    EntityLinkedBrush { owner: AuthorityModelOwner, cmodel_handle: u32, surface_flags: u32 },
}

impl ColliderId {
    /// The surface-flags word that tells a bullet what this is made of. A player is flesh; a riot shield is its own
    /// surface.
    pub(super) fn surface_flags(self) -> u32 {
        match self {
            Self::World { surface_flags, .. }
            | Self::EntityDObjBone { surface_flags, .. }
            | Self::EntityLinkedBrush { surface_flags, .. } => surface_flags,
            Self::Player { hitloc, .. } if hitloc == HITLOC_SHIELD => SURF_TYPE_RIOTSHIELD << 20,
            Self::Player { .. } => SURF_TYPE_FLESH << 20,
        }
    }

    /// The surface-type index effects and sounds are chosen by.
    pub(super) fn surf_type(self) -> u8 {
        match self {
            Self::Player { hitloc, .. } if hitloc == HITLOC_SHIELD => SURF_TYPE_RIOTSHIELD as u8,
            Self::Player { .. } => SURF_TYPE_FLESH as u8,
            other => surface_type_from_flags(other.surface_flags()),
        }
    }

    /// The surface the penetration table is read for, given the one before it.
    pub(super) fn depth_surf(self, previous: u32) -> u32 {
        depth_surface_type(self.surface_flags(), previous)
    }

    /// Contents bits. Only the world carries them.
    pub(super) fn contents(self) -> u32 {
        match self {
            Self::World { contents, .. } => contents,
            _ => 0,
        }
    }

    pub(super) fn glass_encoded(self) -> u16 {
        match self {
            Self::World { glass_encoded, .. } => glass_encoded,
            _ => 0,
        }
    }

    /// The glass pane this is, if it is one (the encoded id is one more than the pane's).
    pub(super) fn pane(self) -> Option<u16> {
        self.glass_encoded().checked_sub(1)
    }

    /// BO2's hit type and id for a segment ending here.
    pub(super) fn hit_kind(self, startsolid: bool) -> (i32, u16) {
        match self {
            Self::World { glass_encoded, .. } if !startsolid && glass_encoded != 0 => (HITTYPE_GLASS, glass_encoded),
            Self::World { .. } => (HITTYPE_ENTITY, trace_t6::ENTITYNUM_WORLD),
            _ => (HITTYPE_ENTITY, 0),
        }
    }

    pub(super) fn is_world(self) -> bool {
        matches!(self, Self::World { .. })
    }

    pub(super) fn is_player(self) -> bool {
        matches!(self, Self::Player { .. })
    }

    /// Anything that is not the static world: a bullet that hits it hurts it.
    pub(super) fn is_damageable(self) -> bool {
        !self.is_world()
    }

    pub(super) fn is_shield(self) -> bool {
        matches!(self, Self::Player { hitloc, .. } if hitloc == HITLOC_SHIELD)
    }

    pub(super) fn client(self) -> Option<ClientId> {
        match self {
            Self::Player { client, .. } => Some(client),
            _ => None,
        }
    }

    /// The scripted model this belongs to. After a hit the next trace skips the model it just struck, as BO2 does by
    /// ignoring the entity it advanced past.
    pub(super) fn script_model(self) -> Option<ScriptModelId> {
        match self {
            Self::EntityDObjBone { owner, .. } | Self::EntityLinkedBrush { owner, .. } => owner.script_model(),
            _ => None,
        }
    }
}

/// A hit on a sky surface.
pub(crate) fn is_sky(collider: ColliderId) -> bool {
    collider.surface_flags() & SURF_SKY != 0
}

/// Why a trace was not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceInvalidReason {
    NonFiniteInput,
}

/// What one straight trace found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TraceOutcome {
    Miss { end: [f32; 3] },
    Hit { fraction: f32, end: [f32; 3], normal: [f32; 3], collider: ColliderId },
    StartSolid { collider: Option<ColliderId>, end: [f32; 3] },
    Invalid { reason: TraceInvalidReason },
}

/// Which of BO2's two bullet routines produced a segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BulletPath {
    #[default]
    Extended,
    Penetrate,
}

/// One leg of a bullet's flight: where it began and ended and what it struck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletTraceSegment {
    pub start: [f32; 3],
    pub end: [f32; 3],
    /// The surface normal at the impact. An exit leg carries the bullet's forward direction instead.
    pub normal: [f32; 3],
    pub surf_type: u8,
    pub surface_flags: u32,
    pub penetrated: bool,
    pub thickness: f32,
    pub damage_mult: f32,
    pub path: BulletPath,
    pub collider: Option<ColliderId>,
    pub startsolid: bool,
    pub glass_encoded: u16,
    pub hit_type: i32,
    pub hit_id: u16,
}

impl BulletTraceSegment {
    /// The surface's name in BO2's list of 32; anything past the end is "default".
    pub fn surf_name(self) -> &'static str {
        weapon_t6::surface::T6_SURFACE_TYPE_NAMES.get(usize::from(self.surf_type)).copied().unwrap_or("default")
    }
}

/// A line to trace and who it should not see.
#[derive(Clone, Copy, Debug)]
pub struct BulletTraceQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub mask: u32,
    /// The shooter: never hit.
    pub ignore: Option<ClientId>,
    /// A player already hit this shot: skipped, so a bullet goes on through him.
    pub ignore_hit: Option<ClientId>,
    pub ignore_model: Option<ScriptModelId>,
}
