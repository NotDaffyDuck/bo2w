//! Putting the frame's effect models (debris, casings, anything an effect
//! element draws as a model) into the model draw plan.
//!
//! A model is dropped, with a counter bumped on the plan, when the catalog
//! does not know it, the element asks not to be drawn, it is too far for any
//! level of detail, it lies outside the view, or none of its surfaces has a
//! material. The ones that remain ask the lighting system for a sample at
//! their origin and add one draw per surface.

use bevy::prelude::*;
use fx::MeshParticleCopy;
use render_fx::ModelDrop;

use crate::{
    FxModelDrawList,
    mesh_build::{
        prop_mesh::{DetailRampArgs, prop_camera_detail},
        figure_mesh::FigureFacetDraw,
    },
};
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::model_illum_cache::{
    ModelIllumOwner, ModelIllumRequest, ModelIllumRequests, LevelModelIllumCache,
};
use crate::prepare::scene::world::LevelStage;

/// The element flag that keeps an effect model out of the main view.
const FX_ELEM_DONT_RENDER: i32 = 0x800;

/// Why every model of the frame is passed over at once.
#[derive(Debug, PartialEq, Eq)]
enum FrameBlock {
    NoCatalog,
    NoLighting,
}

/// What has to be at hand before any model can be lit and drawn.
fn frame_block(
    has_models: bool,
    has_atlas: bool,
    has_scene: bool,
    cached: bool,
) -> Option<FrameBlock> {
    if !has_models {
        Some(FrameBlock::NoCatalog)
    } else if !(has_atlas && has_scene && cached) {
        Some(FrameBlock::NoLighting)
    } else {
        None
    }
}

/// The world's baked model lighting image counts as an atlas as well.
fn scene_has_atlas(scene: Option<&LevelStage>) -> bool {
    scene.is_some_and(|world| {
        world.model_illum_image.is_some() && world.model_illum_dims.is_some()
    })
}

/// How far an instance reaches from its origin, as the catalog's radius
/// stretched by the instance's scale.
fn reach(radius: f32, size_k: f32) -> f32 {
    radius * size_k.abs()
}

fn placement(instance: &MeshParticleCopy) -> Mat4 {
    let turn = Quat::from_array(fx_t6::runtime::axis_to_quat(instance.axes_m));
    let size = Vec3::splat(instance.size_k);
    Mat4::from_scale_rotation_translation(size, turn, instance.place_at.into())
}

/// The sphere shadow casters see; never smaller than one unit.
fn caster_bound(radius: Option<f32>, instance: &MeshParticleCopy) -> Option<render_scene::XModelCasterBound> {
    let origin = instance.place_at;
    radius.map(|authored| render_scene::XModelCasterBound { origin, reach: reach(authored, instance.size_k).max(1.0) })
}

/// A reason to skip one instance; each is counted on the plan.
enum Skip {
    NoCatalog,
    DontRender,
    NoLod,
    OutOfView,
    NoMaterial,
}

impl Skip {
    fn reason(self) -> ModelDrop {
        match self {
            Skip::NoCatalog => ModelDrop::NoCatalog,
            Skip::DontRender => ModelDrop::RenderFlags,
            Skip::NoLod => ModelDrop::NoLod,
            Skip::OutOfView => ModelDrop::Culled,
            Skip::NoMaterial => ModelDrop::NoMaterial,
        }
    }
}

fn count(plan: &mut FxModelDrawList, why: Skip) {
    plan.dropped.add_to(why.reason(), 1);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fill_effect_model_plan(
    inst_list: &[MeshParticleCopy],
    models: Option<&asset_game::EffectMeshRoster>,
    atlas: Option<&LevelModelIllumAtlas>,
    scene: Option<&LevelStage>,
    at_spot: &render_scene::LooseAtSpotLookup,
    eye: Vec3,
    view_volume_planes: &[[f32; 4]],
    detail_ramp: DetailRampArgs,
    cache: Option<&LevelModelIllumCache>,
    illum_requests: &mut ModelIllumRequests,
    plan: &mut FxModelDrawList,
) {
    let total = u32::try_from(inst_list.len()).unwrap_or(u32::MAX);
    plan.emitted = total;
    if inst_list.is_empty() {
        return;
    }
    let models = models.filter(|catalog| !catalog.is_empty());
    let block = frame_block(
        models.is_some(),
        atlas.is_some() || scene_has_atlas(scene),
        scene.is_some(),
        cache.is_some(),
    );
    match block {
        Some(FrameBlock::NoCatalog) => {
            plan.dropped.set_to(ModelDrop::NoCatalog, total);
            return;
        }
        Some(FrameBlock::NoLighting) => {
            plan.dropped.set_to(ModelDrop::NoLighting, total);
            return;
        }
        None => {}
    }
    let Some(models) = models else { return };

    for copy in inst_list {
        let (spot, size) = (copy.place_at, copy.size_k);
        let Some(entry) = models.record_at(copy.mesh_slot_number) else {
            count(plan, Skip::NoCatalog);
            continue;
        };
        if copy.copy_bits & FX_ELEM_DONT_RENDER != 0 {
            count(plan, Skip::DontRender);
            continue;
        }
        let Some(lod) = prop_camera_detail(entry.bone_rig.lod, spot, size.abs(), Some(eye), detail_ramp) else {
            count(plan, Skip::NoLod);
            continue;
        };
        let radius = entry.bone_rig.radius;
        let outside = radius
            .is_some_and(|authored| dpvs_t6::scene_ent_sphere_culled(spot, reach(authored, size), view_volume_planes));
        if outside {
            count(plan, Skip::OutOfView);
            continue;
        }
        let Some(asset) = plan.find_asset(copy.mesh_slot_number, lod) else {
            count(plan, Skip::NoMaterial);
            continue;
        };
        let surfaces = asset.surf_list.clone();

        let box_half = radius.map(|authored| render_anim::radius_lighting_box_half(reach(authored, size)));
        let lighting = illum_requests.queue(ModelIllumRequest {
            owner: ModelIllumOwner::EffectModel(copy.part_id),
            origin: spot,
            lookup_backup: at_spot.fallback(spot, box_half),
        });
        let local_to_world = placement(copy);
        let caster_bound = caster_bound(radius, copy);
        for (surf, material) in surfaces {
            let colour_refusal = None;
            plan.queue_draw(FigureFacetDraw {
                surface: surf,
                material,
                map_from_self: local_to_world,
                illum_handle: 0,
                queued_illum: Some(lighting),
                colour_refusal,
                object_tag: copy.part_id,
                stage_light_index: 0,
                env_probe_index: 0,
                compact_illum: None,
                is_scoped_view: false,
                stage_actor_num: None,
                caster_bound,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(scale: f32, pos: [f32; 3]) -> MeshParticleCopy {
        MeshParticleCopy {
            definition_name: String::new(),
            definition_slot: 0,
            mesh_slot_number: 0,
            part_id: 7,
            place_at: pos,
            axes_m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size_k: scale,
            copy_bits: 0,
        }
    }

    #[test]
    fn without_models_nothing_is_lit_or_counted_as_unlit() {
        assert_eq!(frame_block(false, true, true, true), Some(FrameBlock::NoCatalog));
        // The catalog is checked first.
        assert_eq!(frame_block(false, false, false, false), Some(FrameBlock::NoCatalog));
    }

    #[test]
    fn lighting_needs_atlas_scene_and_cache() {
        assert_eq!(frame_block(true, true, true, true), None);
        for missing in 0..3 {
            let have = |part: usize| part != missing;
            assert_eq!(
                frame_block(true, have(0), have(1), have(2)),
                Some(FrameBlock::NoLighting),
                "part {missing}"
            );
        }
    }

    #[test]
    fn no_scene_means_no_scene_atlas() {
        assert!(!scene_has_atlas(None));
    }

    #[test]
    fn reach_follows_the_size_not_the_sign() {
        assert_eq!(reach(10.0, 2.0), 20.0);
        assert_eq!(reach(10.0, -2.0), 20.0);
    }

    #[test]
    fn placement_scales_turns_and_moves() {
        let at = placement(&instance(2.0, [1.0, 2.0, 3.0]));
        let moved = at.transform_point3(Vec3::X);
        assert!(moved.abs_diff_eq(Vec3::new(3.0, 2.0, 3.0), 1e-5), "{moved}");
    }

    #[test]
    fn a_caster_is_at_least_one_unit_wide() {
        let tiny = caster_bound(Some(0.1), &instance(1.0, [0.0; 3])).unwrap();
        assert_eq!(tiny.reach, 1.0);
        let big = caster_bound(Some(10.0), &instance(-3.0, [4.0, 5.0, 6.0])).unwrap();
        assert_eq!(big.reach, 30.0);
        assert_eq!(big.origin, [4.0, 5.0, 6.0]);
        assert!(caster_bound(None, &instance(1.0, [0.0; 3])).is_none());
    }

    #[test]
    fn each_skip_has_its_own_counter() {
        let mut plan = FxModelDrawList::without_geometry(0);
        for why in [Skip::NoCatalog, Skip::DontRender, Skip::NoLod, Skip::OutOfView] {
            count(&mut plan, why);
        }
        count(&mut plan, Skip::OutOfView);
        count(&mut plan, Skip::NoMaterial);
        assert_eq!(
            (
                plan.dropped.count_of(ModelDrop::NoCatalog),
                plan.dropped.count_of(ModelDrop::RenderFlags),
                plan.dropped.count_of(ModelDrop::NoLod),
                plan.dropped.count_of(ModelDrop::Culled),
                plan.dropped.count_of(ModelDrop::NoMaterial),
            ),
            (1, 1, 1, 2, 1)
        );
        assert_eq!(plan.dropped.count_of(ModelDrop::NoLighting), 0);
    }

    #[test]
    fn an_empty_frame_only_records_the_total() {
        let mut plan = FxModelDrawList::without_geometry(0);
        let mut requests = ModelIllumRequests::default();
        let at_spot = render_scene::LooseAtSpotLookup::default();
        fill_effect_model_plan(
            &[],
            None,
            None,
            None,
            &at_spot,
            Vec3::ZERO,
            &[],
            DetailRampArgs::default(),
            None,
            &mut requests,
            &mut plan,
        );
        assert_eq!(plan.emitted, 0);
        assert_eq!(plan.dropped.count_of(ModelDrop::NoCatalog), 0);
    }

    #[test]
    fn a_missing_catalog_skips_the_whole_frame() {
        let mut plan = FxModelDrawList::without_geometry(0);
        let mut requests = ModelIllumRequests::default();
        let at_spot = render_scene::LooseAtSpotLookup::default();
        let frame = [instance(1.0, [0.0; 3]), instance(1.0, [5.0; 3])];
        fill_effect_model_plan(
            &frame,
            None,
            None,
            None,
            &at_spot,
            Vec3::ZERO,
            &[],
            DetailRampArgs::default(),
            None,
            &mut requests,
            &mut plan,
        );
        assert_eq!(plan.emitted, 2);
        assert_eq!(plan.dropped.count_of(ModelDrop::NoCatalog), 2);
        assert!(plan.draws().is_empty());
    }
}
