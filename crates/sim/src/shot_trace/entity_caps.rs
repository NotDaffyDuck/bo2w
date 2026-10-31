//! What a scripted entity offers a bullet: linked brushes and an animated model, and the snapshot of both that a
//! trace (or a stored history frame) carries.

use super::collider::ServerModelSource;
use super::puppet_state::ServerModelState;
use super::ray_boxes::{RayBox, box_of_thing, ray_aabb_box};
use super::posed_coll::{ServerModelCollision, angles_from_matrix, matrix_from_angles};
use super::poses::wrap_180;
use crate::sim_parts::{CollisionSubmodels, StepNo};

/// One brush model bolted to an entity, placed by origin and angles.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachedSolidShape {
    pub collision_model_ref: u32,
    pub origin: [f32; 3],
    pub turn: [f32; 3],
}

/// Which moment an entity's collision describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityCollisionMoment {
    LiveMoment,
    PastFrame { frame: StepNo },
}

impl EntityCollisionMoment {
    /// A small number folded into state digests.
    pub const fn fingerprint_code(self) -> u32 {
        match self {
            Self::LiveMoment => 1,
            Self::PastFrame { .. } => 2,
        }
    }
}

/// Everything a trace needs to test one entity, copied out so the entity itself need not be consulted.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityRayShape {
    pub owner: ServerModelSource,
    pub epoch: EntityCollisionMoment,
    pub collision: Option<ServerModelCollision>,
    /// The model's own contents bits (`None` when it does not say).
    pub model_content_mask: Option<u32>,
    pub mesh_asset_name: Option<String>,
    pub attached_solids: Vec<AttachedSolidShape>,
    /// Where the model stands: origin and angles (pitch, yaw, roll), as BO2's `currentOrigin` and `currentAngles`.
    pub placed_at: Option<([f32; 3], [f32; 3])>,
}

/// `from` carried `progress` of the way to `to` in a straight line.
fn lerp(from: [f32; 3], to: [f32; 3], progress: f32) -> [f32; 3] {
    std::array::from_fn(|i| (to[i] - from[i]) * progress + from[i])
}

/// BO2 `LerpAngleVector`: each angle `progress` of the way, the short way round.
fn lerp_angles(from: [f32; 3], to: [f32; 3], progress: f32) -> [f32; 3] {
    std::array::from_fn(|i| wrap_180(to[i] - from[i]) * progress + from[i])
}

impl EntityRayShape {
    /// This row carried `progress` of the way toward `later`, as BO2's `mover_info_t::get_position` places a mover
    /// between two of its records: each bolted brush and the model move in a straight line and turn the short way
    /// round. The model's boxes and triangles go along with it, keeping this frame's shape.
    #[must_use]
    pub fn blended_toward(&self, later: &Self, progress: f32) -> Self {
        let mut row = self.clone();
        if self.attached_solids.len() == later.attached_solids.len() {
            for (solid, next) in row.attached_solids.iter_mut().zip(&later.attached_solids) {
                if solid.collision_model_ref == next.collision_model_ref && *solid != *next {
                    solid.origin = lerp(solid.origin, next.origin, progress);
                    solid.turn = lerp_angles(solid.turn, next.turn, progress);
                }
            }
        }
        if let (Some((at, aim)), Some((next_at, next_aim))) = (self.placed_at, later.placed_at) {
            if (at, aim) != (next_at, next_aim) {
                let (origin, angles) = (lerp(at, next_at, progress), lerp_angles(aim, next_aim, progress));
                let carry = matrix_from_angles(origin.into(), angles) * matrix_from_angles(at.into(), aim).inverse();
                if let Some(collision) = &mut row.collision {
                    for bone in &mut collision.bones {
                        bone.center = carry.transform_point3(bone.center.into()).to_array();
                        bone.axes = bone.axes.map(|axis| carry.transform_vector3(axis.into()).to_array());
                    }
                    if let Some(posed) = &mut collision.coll {
                        posed.placement_matrix = carry * posed.placement_matrix;
                    }
                }
                row.placed_at = Some((origin, angles));
            }
        }
        row
    }
}

/// A scripted entity's live collision: its linked brushes, its model, and whether bullets may touch it.
#[derive(Clone, Debug)]
pub struct EntityHitAbilities {
    pub owner: ServerModelSource,
    epoch: EntityCollisionMoment,
    pub dobj: Option<ServerModelState>,
    pub attached_solids: Vec<AttachedSolidShape>,
    pub unseen: bool,
    pub solid: bool,
    /// A bit per player its brushes do not stop (bo2zm: a door hidden from him); zombies and bullets still meet them.
    pub passes_for: u64,
    pub followed_pose: Option<([f32; 3], [f32; 3])>,
    /// True for a row prediction made itself from a snapshot's brush mover (`presence::adopt_brush_movers`); such a
    /// row is dropped when its mover leaves the snapshot.
    pub from_world_state: bool,
}

impl EntityHitAbilities {
    /// A solid, visible entity as it stands this tick.
    pub fn now_tick(
        owner: ServerModelSource,
        dobj: Option<ServerModelState>,
        attached_solids: Vec<AttachedSolidShape>,
    ) -> Self {
        Self {
            owner,
            epoch: EntityCollisionMoment::LiveMoment,
            dobj,
            attached_solids,
            unseen: false,
            solid: true,
            passes_for: 0,
            followed_pose: None,
            from_world_state: false,
        }
    }

    pub const fn epoch(&self) -> EntityCollisionMoment {
        self.epoch
    }

    /// The brushes a bullet can hit: all of them while solid, none otherwise.
    pub fn solid_brushes(&self) -> &[AttachedSolidShape] {
        match self.solid {
            true => &self.attached_solids,
            false => &[],
        }
    }

    /// The brushes that stop this player (`solid_brushes` less one hidden from him).
    pub fn solid_brushes_for(&self, client: u32) -> &[AttachedSolidShape] {
        if 1u64.checked_shl(client).is_some_and(|bit| self.passes_for & bit != 0) {
            &[]
        } else {
            self.solid_brushes()
        }
    }

    /// A cheap yes/no for "could this line touch the entity at all". It uses the same box the tracer rejects with,
    /// so the two never disagree.
    pub(crate) fn ray_may_hit(&self, cmodels: &CollisionSubmodels, from: [f32; 3], till: [f32; 3]) -> bool {
        if !self.solid {
            return false;
        }
        let posed = self.dobj.as_ref().and_then(|state| state.live_hitbox.as_ref());
        match box_of_thing(&self.attached_solids, posed, cmodels) {
            Some((mins, maxs)) => !matches!(ray_aabb_box(from, till, mins, maxs), RayBox::Miss),
            None => false,
        }
    }

    /// The copy a trace works from. A non-solid entity yields an empty shape_data, so it can never be struck.
    pub fn trace_geom(&self) -> EntityRayShape {
        let mut shape_data = EntityRayShape {
            owner: self.owner,
            epoch: self.epoch,
            collision: None,
            model_content_mask: None,
            mesh_asset_name: None,
            attached_solids: Vec::new(),
            placed_at: None,
        };
        if !self.solid {
            return shape_data;
        }
        if let Some(state) = &self.dobj {
            shape_data.collision = state.live_hitbox.clone();
            shape_data.model_content_mask = state.can_do.as_ref().and_then(|capability| capability.contents);
            shape_data.mesh_asset_name = Some(state.active_mesh.clone());
            shape_data.placed_at = Some(angles_from_matrix(state.placement_matrix));
        }
        shape_data.attached_solids = self.attached_solids.clone();
        shape_data
    }
}
