//! The sky model: one unlit model drawn at the world origin behind
//! everything, planned once from the world scene.

use super::prop_mesh::{PropGpuPlan, pack_prop_meshes, live_model_tables, unlit_pass_material};
use super::figure_mesh::FigureFacetDraw;
use crate::prepare::scene::spawn::LevelSpawnJob;
use crate::prepare::scene::world::LevelStage;
use bevy::prelude::*;
use render_frame::InputChangeCounters;

#[derive(Resource, Default)]
pub struct SkyDomeDrawPlan {
    pub geometry: PropGpuPlan,
    pub draws: Vec<FigureFacetDraw>,
    pub refusals: Vec<SkyModelRejection>,
    pub era: u64,
    pub revisions: InputChangeCounters,
}

/// Why a sky surface is not drawn.
#[derive(Debug)]
pub enum SkyModelRejection {
    FacetMaterialMissing { surface: u32 },
    MaterialUnsorted { material: assets::SurfaceMaterialId }, MaterialAbsent { material: assets::SurfaceMaterialId },
}

/// A sky surface's draw: identity transform, no lighting, no scene object.
fn sky_surface_draw(surface: u32, material: u32) -> FigureFacetDraw {
    let colour_refusal = None;
    let identity = Mat4::IDENTITY;
    FigureFacetDraw {
        surface, material, map_from_self: identity, illum_handle: 0,
        queued_illum: None,
        colour_refusal,
        object_tag: u16::MAX,
        stage_light_index: 0,
        env_probe_index: 0,
        compact_illum: None,
        is_scoped_view: false,
        stage_actor_num: None,
        caster_bound: None,
    }
}

pub fn make_sky_model_draw_plan(
    mut cmds: Commands, scene: Option<Res<LevelStage>>,
    job: Option<Res<LevelSpawnJob>>, prior: Option<Res<SkyDomeDrawPlan>>,
) {
    if prior.is_some() {
        return;
    }
    let Some((scene, job)) = scene.zip(job) else {
        return;
    };
    let Some(model) = &scene.sky_dome_model else {
        return;
    };
    let (images, _, _) = job.stitch_image_handles();
    if images.is_empty() {
        return;
    }
    let catalog = &scene.live_material_ledger;
    let (mut geom, _, _) = pack_prop_meshes(std::slice::from_ref(model));
    let packed = std::mem::take(&mut geom.meshes[0].facets_by_detail[0]);
    let mut draws = vec![];
    let mut refused = Vec::new();
    for &(facet, material_ref) in &packed {
        let Some(authored) = material_ref else {
            refused.push(SkyModelRejection::FacetMaterialMissing { surface: facet });
            continue;
        };
        let Some(derived) = catalog.derived(authored) else {
            refused.push(SkyModelRejection::MaterialAbsent { material: authored });
            continue;
        };
        let Some(ordinal) = catalog.ranked_materials.rank_for_asset_id(authored.order()) else {
            refused.push(SkyModelRejection::MaterialUnsorted { material: authored });
            continue;
        };
        let maps = live_model_tables(Some(authored), catalog, images);
        draws.push(sky_surface_draw(facet, geom.materials.len() as u32));
        let materials = &mut geom.materials;
        materials.push(unlit_pass_material(maps, derived.draw_sort_key, Some(ordinal.get())));
    }
    geom.meshes[0].facets_by_detail[0] = packed;
    let drawn = draws.len();
    let (name, refused_count) = (&model.name, refused.len());
    diag::info!(
        World,
        "skybox tess: model={name} surfaces={drawn} refused={refused_count}"
    );
    if !refused.is_empty() {
        diag::warn!(World, "skybox tess refusals: model={name} {refused:?}");
    }
    let mut revisions = InputChangeCounters::default();
    revisions.stamp_topology(geom.indices(), geom.facet_ranges(), geom.decoded_vertices().len());
    revisions.bump_surfaces();
    revisions.advance_drawlist_stamp();
    cmds.insert_resource(SkyDomeDrawPlan { geometry: geom, draws, refusals: refused, era: 1, revisions });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sky_surface_draws_unlit_at_the_origin() {
        let draw = sky_surface_draw(4, 2);
        assert_eq!((draw.surface, draw.material), (4, 2));
        assert_eq!(draw.map_from_self, Mat4::default());
        assert_eq!(draw.object_tag, u16::MAX);
        assert!(draw.compact_illum.is_none() && draw.queued_illum.is_none());
    }
}
