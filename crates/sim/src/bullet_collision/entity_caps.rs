//! What a scripted entity offers a bullet: linked brushes and an animated model, and the snapshot of both that a
//! trace (or a stored history frame) carries.

use super::collider::AuthorityModelOwner;
use super::dobj_state::AuthorityDObjState;
use super::geometry::{RayBox, entity_bounds, ray_aabb_box};
use super::posed_coll::AuthorityDObjCollision;
use crate::world::{SimClipCmodels, Tick};

/// One brush model bolted to an entity, placed by origin and angles.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkedBrushCollisionBrush {
    pub cmodel_handle: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// Which moment an entity's collision describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityCollisionEpoch {
    CurrentTick,
    Historical { frame: Tick },
}

impl EntityCollisionEpoch {
    /// A small number folded into state digests.
    pub const fn digest_tag(self) -> u32 {
        match self {
            Self::CurrentTick => 1,
            Self::Historical { .. } => 2,
        }
    }
}

/// Everything a trace needs to test one entity, copied out so the entity itself need not be consulted.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionTraceGeom {
    pub owner: AuthorityModelOwner,
    pub epoch: EntityCollisionEpoch,
    pub collision: Option<AuthorityDObjCollision>,
    /// The model's own contents bits (`None` when it does not say).
    pub dobj_contents: Option<u32>,
    pub model_key: Option<String>,
    pub linked_brushes: Vec<LinkedBrushCollisionBrush>,
}

/// A scripted entity's live collision: its linked brushes, its model, and whether bullets may touch it.
#[derive(Clone, Debug)]
pub struct EntityCollisionCapabilities {
    pub owner: AuthorityModelOwner,
    epoch: EntityCollisionEpoch,
    pub dobj: Option<AuthorityDObjState>,
    pub linked_brushes: Vec<LinkedBrushCollisionBrush>,
    pub hidden: bool,
    pub solid: bool,
    pub followed_pose: Option<([f32; 3], [f32; 3])>,
    /// True for a row prediction made itself from a snapshot's brush mover (`presence::adopt_brush_movers`); such a
    /// row is dropped when its mover leaves the snapshot.
    pub from_snapshot: bool,
}

impl EntityCollisionCapabilities {
    /// A solid, visible entity as it stands this tick.
    pub fn current_tick(
        owner: AuthorityModelOwner,
        dobj: Option<AuthorityDObjState>,
        linked_brushes: Vec<LinkedBrushCollisionBrush>,
    ) -> Self {
        Self {
            owner,
            epoch: EntityCollisionEpoch::CurrentTick,
            dobj,
            linked_brushes,
            hidden: false,
            solid: true,
            followed_pose: None,
            from_snapshot: false,
        }
    }

    pub const fn epoch(&self) -> EntityCollisionEpoch {
        self.epoch
    }

    /// The brushes a bullet can hit: all of them while solid, none otherwise.
    pub fn solid_brushes(&self) -> &[LinkedBrushCollisionBrush] {
        match self.solid {
            true => &self.linked_brushes,
            false => &[],
        }
    }

    /// A cheap yes/no for "could this line touch the entity at all". It uses the same box the tracer rejects with,
    /// so the two never disagree.
    pub(crate) fn ray_may_hit(&self, cmodels: &SimClipCmodels, start: [f32; 3], end: [f32; 3]) -> bool {
        if !self.solid {
            return false;
        }
        let posed = self.dobj.as_ref().and_then(|state| state.current_collision.as_ref());
        match entity_bounds(&self.linked_brushes, posed, cmodels) {
            Some((mins, maxs)) => !matches!(ray_aabb_box(start, end, mins, maxs), RayBox::Miss),
            None => false,
        }
    }

    /// The copy a trace works from. A non-solid entity yields an empty geom, so it can never be struck.
    pub fn trace_geom(&self) -> EntityCollisionTraceGeom {
        let mut geom = EntityCollisionTraceGeom {
            owner: self.owner,
            epoch: self.epoch,
            collision: None,
            dobj_contents: None,
            model_key: None,
            linked_brushes: Vec::new(),
        };
        if !self.solid {
            return geom;
        }
        if let Some(state) = &self.dobj {
            geom.collision = state.current_collision.clone();
            geom.dobj_contents = state.capability.as_ref().and_then(|capability| capability.contents);
            geom.model_key = Some(state.current_model.clone());
        }
        geom.linked_brushes = self.linked_brushes.clone();
        geom
    }
}
