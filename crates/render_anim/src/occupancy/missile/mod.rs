//! Projectiles in flight and scripted models (perk machines, the box,
//! power-ups, cars), drawn by the fallback pass from the first-person mesh
//! catalog, plus the projectile pass that submits missiles as scene entities.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use bevy::prelude::*;

use crate::motion::puppet_pose::HostSkeletonFrame;
use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::scene_feed::{SceneBonePoseSet, SkeletonSceneHandoff, SkeletonSceneStage};
use crate::motion::figure_pose::SkinnedMeshFacet;
use crate::occupancy::script_prop::{self, pose_script_skeleton_and_materials};
use crate::occupancy::t6_body::SlowGuard;
use crate::ThrownPaintPlan;
use crate::ModelOwnerPaint;
use crate::DRAW_ID_START_ROCKETS;
use crate::push_projectile_surfaces;
use lighting_t6::illum_query_half_size;
use asset_core::AssetCategoryScope;
use frame::{ClientStep, DrawPassStage, JobQueueStage};
use net::ShownProjectile;
use render_scene::orientation_quat_from_euler;
use render_scene::{VisualStage, HostVisualStage, PropPassMaterial, FigureFacetDraw};
use render_scene::{ModelIllumOwner, ModelIllumRequest, ModelIllumRequests};
use render_scene::{StitchMaterials, LevelModelIllumAtlas, LevelPresentFacts};
use xmodel_runtime::{ClipPlayback, MountedModel, PosedSkeleton, DObjAnim, AnimGraphRole};

mod collect;
mod draw_out;
mod logs;
mod occupy;
mod pose;
mod publish;
mod skin;
use self::collect::*;
use self::draw_out::*;
use self::logs::*;
use self::occupy::*;
use self::pose::*;
use self::publish::*;
pub use self::skin::*;

mod tests;

/// How far above a missile its light sample is taken.
pub const ROCKET_LIGHT_LIFT: f32 = 4.0;

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
    pub bone_rig: Arc<asset_model::MeshRig>,
    /// Per surface, its catalog material (none = not drawn).
    pub materials: Vec<Option<u32>>,
    pub model_to_world: Mat4,
    pub origin: [f32; 3],
    /// Where its light is read from the light grid: a script model's middle
    /// (its first model's bounds centre), not its feet, which may sit in the
    /// ground and read black.
    pub light_at: [f32; 3],
    /// Skinned positions and surface_directions in model space (an animated model);
    /// none = bind pose.
    pub posed: Option<Arc<PosedVerts>>,
    /// The parts hidden on its entity (a mannequin's shot-off head) and where
    /// this model's bones start among them; none = all drawn.
    pub hide: Option<([u32; 5], u32)>,
    /// bo2zm: a zombie's eye brightness (none = unscaled).
    pub eye_luminance: Option<f32>,
}

impl T6MissileModel {
    fn from_entry(
        entry: &asset_model::FirstPersonMeshRecord,
        world_from_local: Mat4,
        pos: [f32; 3],
    ) -> Self {
        let materials = entry
            .surface_links
            .iter()
            .map(|edge| edge.extent_position().map(|order| order as u32))
            .collect::<_>();
        Self {
            hide: None,
            posed: None,
            light_at: pos,
            eye_luminance: None,
            materials,
            bone_rig: Arc::clone(&entry.bone_rig),
            model_to_world: world_from_local,
            origin: pos,
        }
    }

    /// The triangles of a surface that remain once the model's hidden parts
    /// are left out: none when every bone the surface is skinned to is
    /// hidden; a rigid surface with some bones hidden loses those bones'
    /// triangles; anything else keeps them all.
    pub fn drawn_indices<'a>(&self, surf_ix: usize, indices: &'a [u32]) -> Option<Cow<'a, [u32]>> {
        let keep_all = Some(Cow::Borrowed(indices));
        let Some((hide, base)) = self.hide else {
            return keep_all;
        };
        let Some(bits) = self.bone_rig.face_parts.get(surf_ix) else {
            return keep_all;
        };
        if dobj_t6::all_parts_hidden(bits, &hide, base) {
            return None;
        }
        let rigid = self.bone_rig.bent.get(surf_ix).copied().flatten() == Some(false);
        if !rigid || !dobj_t6::any_part_hidden(bits, &hide, base) {
            return keep_all;
        }
        let vertex_hidden = |v: u32| {
            self.bone_rig.vert_skin.get(v as usize).is_some_and(|skin| {
                dobj_t6::part_is_hidden(&hide, base as usize + usize::from(skin.joints[0]))
            })
        };
        let kept = indices
            .chunks_exact(3)
            .filter(|triangle| !triangle.iter().copied().any(vertex_hidden))
            .flatten()
            .copied()
            .collect::<_>();
        Some(Cow::Owned(kept))
    }
}

/// One model's vertices after skinning.
#[derive(Clone, Default)]
pub struct PosedVerts {
    pub positions: Vec<[f32; 3]>,
    pub surface_directions: Vec<[f32; 3]>,
    /// Or skinned on the graphics card instead: the skin joint_transforms of the
    /// model's own bones (its bone 0 first), and no vertices here.
    pub bones: Vec<Mat4>,
}

#[derive(Resource, Default)]
pub struct RocketSceneSlot {
    pub rows: Vec<LiveRocketEntry>,
}

pub struct LiveRocketEntry {
    pub index: usize,
    pub entry_id: Option<u32>,
    pub entnum: Option<u32>,
    pub weapon: u32,
    pub ignited: bool,
    pub origin: [f32; 3],
    pub rot: [f32; 3],
    pub name: String,
    pub namespace: AssetCategoryScope,
    pub illum_origin: [f32; 3],
    pub lit_by_owner: ModelIllumOwner,
    /// The clip its client script plays on it and how far in (0..1), else
    /// it is drawn in its rest pose.
    pub reel: Option<(&'static str, f32)>,
}

/// Which effects each missile has already started.
#[derive(Resource, Default)]
pub struct RocketAttachState {
    pub rows: HashMap<u32, RocketAttachRow>,
    pub guessed_rows_dropped: u64,
    pub posing_shortfalls: u64,
    pub playback_shortfalls: u64,
    pub ignition_shortfalls: u64,
}

pub struct RocketAttachRow {
    pub projectile: Option<u32>,
    pub weapon: u32,
    pub trail_done: bool,
    pub marker_effect_done: bool,
    pub ignition_effect_done: bool,
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

pub fn rocket_light_point(origin: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = origin;
    [x, y, z + ROCKET_LIGHT_LIFT]
}

pub fn rocket_pose_table(
    uploaded: Option<&assets::ReadiedProjectileMeshes>,
) -> Option<&asset_model::MissileMeshRoster> {
    let catalog = &uploaded?.0;
    (!catalog.is_empty()).then_some(catalog)
}

pub fn rocket_light_atlas<'a>(
    own: Option<&'a LevelModelIllumAtlas>,
    shared: Option<&'a LevelModelIllumAtlas>,
) -> Option<&'a LevelModelIllumAtlas> {
    own.or(shared)
}

pub(crate) fn rocket_model_to_world(origin: [f32; 3], angs: [f32; 3]) -> Mat4 {
    let [pitch, yaw, roll] = angs.map(f32::to_radians);
    let rotation = Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll);
    Mat4::from_rotation_translation(rotation, Vec3::from_array(origin))
}

pub fn wire_rocket_systems(application: &mut App) {
    let present = ClientStep::Present;
    let cells = JobQueueStage::CellDynModel;
    let skinning = JobQueueStage::SkinModel;
    let fx_tail = JobQueueStage::FxRemaining;

    application.init_resource::<RocketAttachState>();
    application.init_resource::<RocketPoseOutput>();
    application.init_resource::<RocketSceneSlot>();
    application.init_resource::<T6ScriptModels>();
    application.init_resource::<T6MissileModels>();
    application.init_resource::<ThrownPaintPlan>();

    application.add_systems(Update, collect_t6_missiles.in_set(DrawPassStage::Anim).before(cells));
    // After the script models take this frame's snapshot; otherwise their
    // place was this frame's or last frame's, whichever system the
    // scheduler ran first.
    application.add_systems(
        Update,
        collect_t6_script_models
            .in_set(DrawPassStage::Anim)
            .after(script_prop::ScriptPropDrawStage)
            .before(cells),
    );
    application.add_systems(
        Update,
        occupy_projectile_scene_ents
            .in_set(DrawPassStage::Anim)
            .before(cells)
            .in_set(render_scene::RenderSceneInsert)
            .in_set(SkeletonSceneStage),
    );
    application.add_systems(
        Update,
        publish_t6_script_model_poses
            .after(skinning)
            .before(fx_tail)
            .in_set(present),
    );
    application.add_systems(
        Update,
        emit_rocket_skeleton_poses
            .after(occupy_projectile_scene_ents)
            .after(skinning)
            .before(fx_tail)
            .in_set(present),
    );
    // bo2zm: a BO2 missile's bones, for the effects bolted to them (a planted claymore's lasers).
    application.add_systems(
        Update,
        publish_t6_missile_dobj_poses
            .after(occupy_projectile_scene_ents)
            .after(emit_rocket_skeleton_poses)
            .after(skinning)
            .before(fx_tail)
            .in_set(present),
    );
    application.add_systems(
        Update,
        (pose_shells, push_missile_draws)
            .chain()
            .after(JobQueueStage::CellSceneEnt)
            .after(JobQueueStage::DpvsEnt)
            .before(cells)
            .in_set(skinning),
    );
}
