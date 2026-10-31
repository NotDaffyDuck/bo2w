//! What a bullet can run into, what a trace reports, and the per-segment record a firing leaves behind.

use trace_t6::{TraceHitType, surface_kind_of_flags};
use weapon_t6::penetration::{FACE_KIND_BODY, pierce_face_kind};

use crate::match_ids::{LifeCount, ScriptedModel};
use crate::sim_parts::SeatNo;

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
pub(super) const GLASS_HIT_CODE: i32 = TraceHitType::Glass as i32;

/// Who owns a scripted model's collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServerModelSource {
    ScriptModel(ScriptedModel),
}

impl ServerModelSource {
    pub const fn scripted_model(self) -> Option<ScriptedModel> {
        match self {
            Self::ScriptModel(seat) => Some(seat),
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
pub enum HitTargetId {
    World { surface_flags: u32, contents: u32, glass_hit_code: u16 },
    Player { client: SeatNo, life: LifeCount, hitloc: u8 },
    PosedBone { owner: ServerModelSource, bone: u16, body_part_category: u8, surface_flags: u32 },
    AttachedBrush { owner: ServerModelSource, collision_model_ref: u32, surface_flags: u32 },
}

impl HitTargetId {
    /// The surface-flags word that tells a bullet what this is made of. A player is flesh; a riot shield is its own
    /// surface.
    pub(super) fn surface_flags(self) -> u32 {
        match self {
            // bo2zm: a body's hit boxes (zombies) are flesh, as a player is: their hits play the blood effect.
            Self::PosedBone { surface_flags: 0, .. } => FACE_KIND_BODY << 20,
            Self::World { surface_flags: face_bits, .. }
            | Self::PosedBone { surface_flags: face_bits, .. }
            | Self::AttachedBrush { surface_flags: face_bits, .. } => face_bits,
            Self::Player { hitloc, .. } if hitloc == HITLOC_SHIELD => SURF_TYPE_RIOTSHIELD << 20,
            Self::Player { .. } => FACE_KIND_BODY << 20,
        }
    }

    /// The surface-type index effects and sounds are chosen by.
    pub(super) fn surf_type(self) -> u8 {
        match self {
            Self::Player { hitloc, .. } if hitloc == HITLOC_SHIELD => SURF_TYPE_RIOTSHIELD as u8,
            Self::Player { .. } => FACE_KIND_BODY as u8,
            other => surface_kind_of_flags(other.surface_flags()),
        }
    }

    /// The surface the penetration table is read for, given the one before it.
    pub(super) fn depth_surf(self, previous: u32) -> u32 {
        pierce_face_kind(self.surface_flags(), previous)
    }

    /// Contents bits. Only the world carries them.
    pub(super) fn contents(self) -> u32 {
        match self {
            Self::World { contents: stuff_bits, .. } => stuff_bits,
            _ => 0,
        }
    }

    pub(super) fn glass_hit_code(self) -> u16 {
        match self {
            Self::World { glass_hit_code, .. } => glass_hit_code,
            _ => 0,
        }
    }

    /// The glass pane this is, if it is one (the encoded id is one more than the pane's).
    pub(super) fn pane(self) -> Option<u16> {
        self.glass_hit_code().checked_sub(1)
    }

    /// BO2's hit type and id for a segment ending here.
    pub(super) fn hit_kind(self, began_inside: bool) -> (i32, u16) {
        match self {
            Self::World { glass_hit_code, .. } if !began_inside && glass_hit_code != 0 => (GLASS_HIT_CODE, glass_hit_code),
            Self::World { .. } => (HITTYPE_ENTITY, trace_t6::WORLD_ENTITY_SLOT),
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

    pub(super) fn client(self) -> Option<SeatNo> {
        match self {
            Self::Player { client: who, .. } => Some(who),
            _ => None,
        }
    }

    /// The scripted model this belongs to. After a hit the next trace skips the model it just struck, as BO2 does by
    /// ignoring the entity it advanced past.
    pub(super) fn script_model(self) -> Option<ScriptedModel> {
        match self {
            Self::PosedBone { owner: holder, .. } | Self::AttachedBrush { owner: holder, .. } => holder.scripted_model(),
            _ => None,
        }
    }
}

/// A hit on a sky surface.
pub(crate) fn is_sky(collider: HitTargetId) -> bool {
    collider.surface_flags() & SURF_SKY != 0
}

/// Why a trace was not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayRejectCause {
    InfiniteOrNanInput,
}

/// What one straight trace found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayResult {
    Miss { end: [f32; 3] },
    Hit { fraction: f32, end: [f32; 3], normal: [f32; 3], struck: HitTargetId },
    BeganInside { struck: Option<HitTargetId>, end: [f32; 3] },
    Invalid { reason: RayRejectCause },
}

/// Which of BO2's two bullet routines produced a segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShotRouteKind {
    #[default]
    Extended,
    Penetrate,
}

/// One leg of a bullet's flight: where it began and ended and what it struck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotPathLeg {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// The surface normal at the impact. An exit leg carries the bullet's forward direction instead.
    pub face_dir: [f32; 3],
    pub face_kind: u8,
    pub face_bits: u32,
    pub passed_through_wall: bool,
    pub depth: f32,
    pub harm_multiplier: f32,
    pub path: ShotRouteKind,
    pub collider: Option<HitTargetId>,
    pub began_inside: bool,
    pub glass_hit_code: u16,
    pub impact_category: i32,
    pub struck_entity_num: u16,
}

impl ShotPathLeg {
    /// The surface's name in BO2's list of 32; anything past the end is "default".
    pub fn material_label(self) -> &'static str {
        weapon_t6::surface::T6_SURFACE_TYPE_NAMES.get(usize::from(self.face_kind)).copied().unwrap_or("default")
    }
}

/// A line to trace and who it should not see.
#[derive(Clone, Copy, Debug)]
pub struct ShotRayRequest {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub bits: u32,
    /// The shooter: never hit.
    pub ignore: Option<SeatNo>,
    /// A player already hit this shot: skipped, so a bullet goes on through him.
    pub skip_client_hit: Option<SeatNo>,
    pub ignore_model: Option<ScriptedModel>,
}
