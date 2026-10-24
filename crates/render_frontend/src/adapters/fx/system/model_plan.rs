//! Putting the frame's effect models (debris, casings, anything an effect
//! element draws as a model) into the model draw plan.
//!
//! A model is dropped, with a counter bumped on the plan, when the catalog
//! does not know it, the element asks not to be drawn, it is too far for any
//! level of detail, it lies outside the view, or none of its surfaces has a
//! material. The ones that remain ask the lighting system for a sample at
//! their origin and add one draw per surface.

use bevy::prelude::*;
use fx::FxModelInstance;

use crate::assemble::drawsurf::{
    FxModelDrawPlan,
    tess::{
        smodel::{LodRampArgs, smodel_camera_lod},
        xmodel::XModelSurfaceDraw,
    },
};
use crate::prepare::scene::{
    model_lighting_atlas::WorldModelLightingAtlas,
    model_lighting_cache::{
        ModelLightingOwner, ModelLightingRequest, ModelLightingRequests, WorldModelLightingCache,
    },
    world::WorldScene,
};

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
    has_cache: bool,
) -> Option<FrameBlock> {
    if !has_models {
        Some(FrameBlock::NoCatalog)
    } else if !(has_atlas && has_scene && has_cache) {
        Some(FrameBlock::NoLighting)
    } else {
        None
    }
}

/// The world's baked model lighting image counts as an atlas as well.
fn scene_has_atlas(scene: Option<&WorldScene>) -> bool {
    scene.is_some_and(|world| {
        world.model_lighting_image.is_some() && world.model_lighting_dims.is_some()
    })
}

/// How far an instance reaches from its origin, as the catalog's radius
/// stretched by the instance's scale.
fn reach(radius: f32, scale: f32) -> f32 {
    radius * scale.abs()
}

fn placement(instance: &FxModelInstance) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(instance.scale),
        Quat::from_array(fx_t6::runtime::axis_to_quat(instance.axis)),
        Vec3::from_array(instance.origin),
    )
}

/// The sphere shadow casters see; never smaller than one unit.
fn caster_bound(radius: Option<f32>, instance: &FxModelInstance) -> Option<render_scene::XModelCasterBound> {
    radius.map(|radius| render_scene::XModelCasterBound {
        origin: instance.origin,
        radius: reach(radius, instance.scale).max(1.0),
    })
}

/// A reason to skip one instance; each is counted on the plan.
enum Skip {
    NoCatalog,
    DontRender,
    NoLod,
    OutOfView,
    NoMaterial,
}

fn count(plan: &mut FxModelDrawPlan, why: Skip) {
    let counter = match why {
        Skip::NoCatalog => &mut plan.skipped_no_catalog,
        Skip::DontRender => &mut plan.skipped_render_fx_flags,
        Skip::NoLod => &mut plan.skipped_no_lod,
        Skip::OutOfView => &mut plan.skipped_culled,
        Skip::NoMaterial => &mut plan.skipped_no_material,
    };
    *counter = counter.saturating_add(1);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fill_fx_model_plan(
    instances: &[FxModelInstance],
    models: Option<&asset_game::FxModelCatalog>,
    atlas: Option<&WorldModelLightingAtlas>,
    scene: Option<&WorldScene>,
    atpoint: &render_scene::DynAtPointLookup,
    eye: Vec3,
    frustum_planes: &[[f32; 4]],
    lod_ramp: LodRampArgs,
    cache: Option<&WorldModelLightingCache>,
    lighting_requests: &mut ModelLightingRequests,
    plan: &mut FxModelDrawPlan,
) {
    let total = u32::try_from(instances.len()).unwrap_or(u32::MAX);
    plan.generated = total;
    if instances.is_empty() {
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
            plan.skipped_no_catalog = total;
            return;
        }
        Some(FrameBlock::NoLighting) => {
            plan.skipped_no_lighting = total;
            return;
        }
        None => {}
    }
    let Some(models) = models else { return };

    for instance in instances {
        let Some(entry) = models.get_at(instance.model_index) else {
            count(plan, Skip::NoCatalog);
            continue;
        };
        if instance.flags & FX_ELEM_DONT_RENDER != 0 {
            count(plan, Skip::DontRender);
            continue;
        }
        let Some(lod) = smodel_camera_lod(
            entry.skel.lod,
            instance.origin,
            instance.scale.abs(),
            Some(eye),
            lod_ramp,
        ) else {
            count(plan, Skip::NoLod);
            continue;
        };
        let radius = entry.skel.radius;
        let outside = radius.is_some_and(|radius| {
            dpvs_t6::scene_ent_sphere_culled(
                instance.origin,
                reach(radius, instance.scale),
                frustum_planes,
            )
        });
        if outside {
            count(plan, Skip::OutOfView);
            continue;
        }
        let Some(asset) = plan.asset(instance.model_index, lod) else {
            count(plan, Skip::NoMaterial);
            continue;
        };
        let surfaces = asset.surfaces.clone();

        let box_half = radius
            .map(|radius| render_anim::radius_lighting_box_half(reach(radius, instance.scale)));
        let lighting = lighting_requests.request(ModelLightingRequest {
            owner: ModelLightingOwner::FxModel(instance.elem_handle),
            origin: instance.origin,
            lookup_fallback: atpoint.fallback(instance.origin, box_half),
        });
        let world_from_local = placement(instance);
        let caster_bound = caster_bound(radius, instance);
        for (surface, material) in surfaces {
            plan.push_draw(XModelSurfaceDraw {
                surface,
                material,
                world_from_local,
                lighting_handle: 0,
                pending_lighting: Some(lighting),
                colour_refusal: None,
                object_id: instance.elem_handle,
                scene_light_index: 0,
                reflection_probe_index: 0,
                packed_lighting: None,
                is_scope: false,
                scene_entnum: None,
                caster_bound,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(scale: f32, origin: [f32; 3]) -> FxModelInstance {
        FxModelInstance {
            def_name: String::new(),
            def_index: 0,
            model_index: 0,
            elem_handle: 7,
            origin,
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale,
            flags: 0,
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
        assert_eq!(tiny.radius, 1.0);
        let big = caster_bound(Some(10.0), &instance(-3.0, [4.0, 5.0, 6.0])).unwrap();
        assert_eq!(big.radius, 30.0);
        assert_eq!(big.origin, [4.0, 5.0, 6.0]);
        assert!(caster_bound(None, &instance(1.0, [0.0; 3])).is_none());
    }

    #[test]
    fn each_skip_has_its_own_counter() {
        let mut plan = FxModelDrawPlan::default();
        for why in [Skip::NoCatalog, Skip::DontRender, Skip::NoLod, Skip::OutOfView] {
            count(&mut plan, why);
        }
        count(&mut plan, Skip::OutOfView);
        count(&mut plan, Skip::NoMaterial);
        assert_eq!(
            (
                plan.skipped_no_catalog,
                plan.skipped_render_fx_flags,
                plan.skipped_no_lod,
                plan.skipped_culled,
                plan.skipped_no_material,
            ),
            (1, 1, 1, 2, 1)
        );
        assert_eq!(plan.skipped_no_lighting, 0);
    }

    #[test]
    fn an_empty_frame_only_records_the_total() {
        let mut plan = FxModelDrawPlan::default();
        let mut requests = ModelLightingRequests::default();
        let atpoint = render_scene::DynAtPointLookup::default();
        fill_fx_model_plan(
            &[],
            None,
            None,
            None,
            &atpoint,
            Vec3::ZERO,
            &[],
            LodRampArgs::default(),
            None,
            &mut requests,
            &mut plan,
        );
        assert_eq!(plan.generated, 0);
        assert_eq!(plan.skipped_no_catalog, 0);
    }

    #[test]
    fn a_missing_catalog_skips_the_whole_frame() {
        let mut plan = FxModelDrawPlan::default();
        let mut requests = ModelLightingRequests::default();
        let atpoint = render_scene::DynAtPointLookup::default();
        let frame = [instance(1.0, [0.0; 3]), instance(1.0, [5.0; 3])];
        fill_fx_model_plan(
            &frame,
            None,
            None,
            None,
            &atpoint,
            Vec3::ZERO,
            &[],
            LodRampArgs::default(),
            None,
            &mut requests,
            &mut plan,
        );
        assert_eq!(plan.generated, 2);
        assert_eq!(plan.skipped_no_catalog, 2);
        assert!(plan.draws().is_empty());
    }
}
