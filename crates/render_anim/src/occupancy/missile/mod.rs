//! Projectiles in flight and scripted models (perk machines, the box,
//! power-ups, cars), drawn by the fallback pass from the first-person mesh
//! catalog, plus the projectile pass that submits missiles as scene entities.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use bevy::prelude::*;

use crate::anim::dobj_pose::HostDObjPoseFrame;
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::scene_submission::{AnimDObjSceneSkels, AnimDObjSceneSubmission, AnimSceneSubmit};
use crate::anim::xmodel_pose::PosedModelSurface;
use crate::occupancy::script_model::{self, pose_script_dobj_with_materials};
use crate::occupancy::t6_body::SlowGuard;
use crate::{
    MissileDrawPlan, MissileOwnerDraw, XMODEL_OBJECT_ID_MISSILE_BASE, append_missile_surfaces,
    radius_lighting_box_half,
};
use asset_core::AssetNamespace;
use frame::{ClientSet, RenderSet, WorkerCmdSet};
use net::PresentedProjectile;
use render_scene::{
    GfxScene, HostGfxScene, ModelLightingOwner, ModelLightingRequest, ModelLightingRequests,
    SmodelPassMaterial, TessMaterials, WorldModelLightingAtlas, WorldPresentFacts,
    XModelSurfaceDraw, scene_quat_from_angles,
};
use xmodel_runtime::{AnimInstance, Attach, DObj, DObjAnim, XAnimSemanticNodeKind};

mod collect;
mod draw;
mod logs;
mod occupy;
mod pose;
mod publish;
mod skin;
use self::collect::*;
use self::draw::*;
use self::logs::*;
use self::occupy::*;
use self::pose::*;
use self::publish::*;
pub use self::skin::*;

#[cfg(test)]
mod tests;

/// How far above a missile its light sample is taken.
pub const MISSILE_LIGHTING_Z_OFS: f32 = 4.0;

/// Projectiles in flight (a thrown grenade, a rocket, a ballistic knife's
/// blade): each one's model, its surfaces' materials and where it is.
#[derive(Resource, Default, Clone)]
pub struct T6MissileModels {
    pub items: Vec<T6MissileModel>,
}

/// Scripted models the snapshot presents, drawn by the same pass as
/// projectiles: each model of the mover's composition (base and attachments)
/// in bind pose, or animated, where the mover is.
#[derive(Resource, Default, Clone)]
pub struct T6ScriptModels {
    pub items: Vec<T6MissileModel>,
}

#[derive(Clone)]
pub struct T6MissileModel {
    pub skel: Arc<asset_model::ModelSkel>,
    /// Per surface, its catalog material (none = not drawn).
    pub materials: Vec<Option<u32>>,
    pub world_from_local: Mat4,
    pub origin: [f32; 3],
    /// Where its light is read from the light grid: a script model's middle
    /// (its first model's bounds centre), not its feet, which may sit in the
    /// ground and read black.
    pub light_at: [f32; 3],
    /// Skinned positions and normals in model space (an animated model);
    /// none = bind pose.
    pub posed: Option<Arc<PosedVerts>>,
    /// The parts hidden on its entity (a mannequin's shot-off head) and where
    /// this model's bones start among them; none = all drawn.
    pub hide: Option<([u32; 5], u32)>,
}

impl T6MissileModel {
    fn from_entry(
        entry: &asset_model::FpvMeshEntry,
        world_from_local: Mat4,
        origin: [f32; 3],
    ) -> Self {
        Self {
            skel: entry.skel.clone(),
            materials: entry
                .material_edges
                .iter()
                .map(|edge| edge.bound_index().map(|i| i as u32))
                .collect(),
            world_from_local,
            origin,
            light_at: origin,
            posed: None,
            hide: None,
        }
    }

    /// The triangles of a surface that remain once the model's hidden parts
    /// are left out: none when every bone the surface is skinned to is
    /// hidden; a rigid surface with some bones hidden loses those bones'
    /// triangles; anything else keeps them all.
    pub fn drawn_indices<'a>(&self, surface: usize, indices: &'a [u32]) -> Option<Cow<'a, [u32]>> {
        let keep_all = Some(Cow::Borrowed(indices));
        let Some((hide, base)) = self.hide else {
            return keep_all;
        };
        let Some(bits) = self.skel.surface_part_bits.get(surface) else {
            return keep_all;
        };
        if dobj_t6::surface_hidden_whole(bits, &hide, base) {
            return None;
        }
        let rigid = self.skel.surface_deformed.get(surface).copied().flatten() == Some(false);
        if !rigid || !dobj_t6::surface_hidden(bits, &hide, base) {
            return keep_all;
        }
        let vertex_hidden = |v: u32| {
            self.skel.vert_skin.get(v as usize).is_some_and(|skin| {
                dobj_t6::hide_part_bit(&hide, base as usize + usize::from(skin.bones[0]))
            })
        };
        let kept = indices
            .chunks_exact(3)
            .filter(|triangle| !triangle.iter().copied().any(vertex_hidden))
            .flatten()
            .copied()
            .collect();
        Some(Cow::Owned(kept))
    }
}

/// One model's vertices after skinning.
#[derive(Clone, Default)]
pub struct PosedVerts {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Or skinned on the graphics card instead: the skin matrices of the
    /// model's own bones (its bone 0 first), and no vertices here.
    pub bones: Vec<Mat4>,
}

#[derive(Resource, Default)]
pub struct MissileOccupancy {
    pub rows: Vec<OccupiedMissile>,
}

pub struct OccupiedMissile {
    pub index: usize,
    pub id: Option<u32>,
    pub entnum: Option<u32>,
    pub weapon: u32,
    pub ignited: bool,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub name: String,
    pub namespace: AssetNamespace,
    pub lighting_origin: [f32; 3],
    pub lighting_owner: ModelLightingOwner,
    /// The clip its client script plays on it and how far in (0..1), else
    /// it is drawn in its rest pose.
    pub clip: Option<(&'static str, f32)>,
}

/// Which effects each missile has already started.
#[derive(Resource, Default)]
pub struct MissileBoltState {
    pub rows: HashMap<u32, MissileBoltRow>,
    pub predicted_rows_skipped: u64,
    pub pose_gaps: u64,
    pub play_gaps: u64,
    pub ignition_gaps: u64,
}

pub struct MissileBoltRow {
    pub projectile: Option<u32>,
    pub weapon: u32,
    pub trail_played: bool,
    pub beacon_played: bool,
    pub ignition_played: bool,
    pub ignition_fx_played: bool,
}

/// The Hunter Killer drone's launch animation: its client script waits a
/// tenth of a second after the throw, then plays 25 frames at 30 a second
/// once and holds the last. The catalog names the weapon bare, the scripts
/// with an `_mp` suffix.
fn missile_client_clip(weapon: &str, ms_since_spawn: i32) -> Option<(&'static str, f32)> {
    const LAUNCH_MS: f32 = 25.0 / 30.0 * 1000.0;
    if weapon.strip_suffix("_mp").unwrap_or(weapon) != "missile_drone_projectile" {
        return None;
    }
    let progress = (ms_since_spawn - 100) as f32 / LAUNCH_MS;
    Some(("o_drone_hunter_launch", progress.clamp(0.0, 1.0)))
}

pub fn missile_lighting_origin(origin: [f32; 3]) -> [f32; 3] {
    [origin[0], origin[1], origin[2] + MISSILE_LIGHTING_Z_OFS]
}

pub fn missile_pose_catalog(
    assets: Option<&assets::PreparedProjectileMeshes>,
) -> Option<&asset_model::ProjectileMeshCatalog> {
    assets
        .map(|prepared| &prepared.0)
        .filter(|catalog| !catalog.is_empty())
}

pub fn missile_lighting_atlas<'a>(
    atlas: Option<&'a WorldModelLightingAtlas>,
    scene_atlas: Option<&'a WorldModelLightingAtlas>,
) -> Option<&'a WorldModelLightingAtlas> {
    atlas.or(scene_atlas)
}

pub(crate) fn missile_world_from_local(origin: [f32; 3], angles: [f32; 3]) -> Mat4 {
    let [pitch, yaw, roll] = angles.map(f32::to_radians);
    Mat4::from_rotation_translation(
        Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll),
        Vec3::from_array(origin),
    )
}

pub fn register_missile_systems(app: &mut App) {
    app.init_resource::<MissileDrawPlan>()
        .init_resource::<T6MissileModels>()
        .init_resource::<T6ScriptModels>()
        .init_resource::<MissileOccupancy>()
        .init_resource::<MissilePoseProduct>()
        .init_resource::<MissileBoltState>()
        .add_systems(
            Update,
            collect_t6_missiles
                .in_set(RenderSet::Anim)
                .before(WorkerCmdSet::CellDynModel),
        )
        // After the script models take this frame's snapshot; otherwise their
        // place was this frame's or last frame's, whichever system the
        // scheduler ran first.
        .add_systems(
            Update,
            collect_t6_script_models
                .in_set(RenderSet::Anim)
                .after(script_model::ScriptModelDrawSet)
                .before(WorkerCmdSet::CellDynModel),
        )
        .add_systems(
            Update,
            occupy_missile_scene_ents
                .in_set(RenderSet::Anim)
                .before(WorkerCmdSet::CellDynModel)
                .in_set(render_scene::GfxSceneAdd)
                .in_set(AnimSceneSubmit),
        )
        .add_systems(
            Update,
            publish_t6_script_model_poses
                .after(WorkerCmdSet::SkinModel)
                .before(WorkerCmdSet::FxRemaining)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            publish_missile_dobj_poses
                .after(occupy_missile_scene_ents)
                .after(WorkerCmdSet::SkinModel)
                .before(WorkerCmdSet::FxRemaining)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            (pose_missiles, append_missile_draws)
                .chain()
                .after(WorkerCmdSet::CellSceneEnt)
                .after(WorkerCmdSet::DpvsEnt)
                .before(WorkerCmdSet::CellDynModel)
                .in_set(WorkerCmdSet::SkinModel),
        );
}
