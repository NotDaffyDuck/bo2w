//! The sky model: one unlit model drawn at the world origin behind
//! everything, planned once from the world scene.

use super::smodel::{SmodelGpuPlan, pack_smodel_meshes, runtime_maps, unlit_pass_material};
use super::xmodel::XModelSurfaceDraw;
use crate::prepare::scene::{spawn::WorldSpawnJob, world::WorldScene};
use bevy::prelude::*;
use render_frame::SourceRevisions;

#[derive(Resource, Default)]
pub struct SkyModelDrawPlan {
    pub geometry: SmodelGpuPlan,
    pub draws: Vec<XModelSurfaceDraw>,
    pub refusals: Vec<SkyModelRefusal>,
    pub generation: u64,
    pub revisions: SourceRevisions,
}

/// Why a sky surface is not drawn.
#[derive(Debug)]
pub enum SkyModelRefusal {
    SurfaceMaterialMissing { surface: u32 },
    MaterialNotSorted { material: assets::MaterialIndex },
    MaterialUnavailable { material: assets::MaterialIndex },
}

/// A sky surface's draw: identity transform, no lighting, no scene object.
fn sky_surface_draw(surface: u32, material: u32) -> XModelSurfaceDraw {
    XModelSurfaceDraw {
        surface,
        material,
        world_from_local: Mat4::IDENTITY,
        lighting_handle: 0,
        pending_lighting: None,
        colour_refusal: None,
        object_id: u16::MAX,
        scene_light_index: 0,
        reflection_probe_index: 0,
        packed_lighting: None,
        is_scope: false,
        scene_entnum: None,
        caster_bound: None,
    }
}

pub fn build_sky_model_draw_plan(
    mut commands: Commands,
    scene: Option<Res<WorldScene>>,
    job: Option<Res<WorldSpawnJob>>,
    existing: Option<Res<SkyModelDrawPlan>>,
) {
    if existing.is_some() {
        return;
    }
    let (Some(scene), Some(job)) = (scene, job) else {
        return;
    };
    let Some(model) = &scene.sky_model else {
        return;
    };
    let (images, _, _) = job.tess_image_handles();
    if images.is_empty() {
        return;
    }
    let catalog = &scene.runtime_material_catalog;
    let (mut geometry, _, _) = pack_smodel_meshes(std::slice::from_ref(model));
    let packed = std::mem::take(&mut geometry.meshes[0].surfaces_by_lod[0]);
    let mut draws = Vec::new();
    let mut refusals = Vec::new();
    for &(surface, authored) in &packed {
        let Some(authored) = authored else {
            refusals.push(SkyModelRefusal::SurfaceMaterialMissing { surface });
            continue;
        };
        let Some(derived) = catalog.derived(authored) else {
            refusals.push(SkyModelRefusal::MaterialUnavailable { material: authored });
            continue;
        };
        let Some(ordinal) = catalog.sorted_materials.ordinal_for_asset_id(authored.order()) else {
            refusals.push(SkyModelRefusal::MaterialNotSorted { material: authored });
            continue;
        };
        let maps = runtime_maps(Some(authored), catalog, images);
        draws.push(sky_surface_draw(surface, geometry.materials.len() as u32));
        geometry
            .materials
            .push(unlit_pass_material(maps, derived.sort_key, Some(ordinal.get())));
    }
    geometry.meshes[0].surfaces_by_lod[0] = packed;
    diag::info!(
        World,
        "skybox tess: model={} surfaces={} refused={}",
        model.name,
        draws.len(),
        refusals.len()
    );
    if !refusals.is_empty() {
        diag::warn!(
            World,
            "skybox tess refusals: model={} {refusals:?}",
            model.name
        );
    }
    let mut revisions = SourceRevisions::default();
    revisions.set_topology_from(
        geometry.indices(),
        geometry.surface_ranges(),
        geometry.decoded_vertices().len(),
    );
    revisions.bump_surfaces();
    revisions.bump_draws();
    commands.insert_resource(SkyModelDrawPlan {
        geometry,
        draws,
        refusals,
        generation: 1,
        revisions,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sky_surface_draws_unlit_at_the_origin() {
        let draw = sky_surface_draw(4, 2);
        assert_eq!((draw.surface, draw.material), (4, 2));
        assert_eq!(draw.world_from_local, Mat4::IDENTITY);
        assert_eq!(draw.object_id, u16::MAX);
        assert!(draw.packed_lighting.is_none() && draw.pending_lighting.is_none());
    }
}
