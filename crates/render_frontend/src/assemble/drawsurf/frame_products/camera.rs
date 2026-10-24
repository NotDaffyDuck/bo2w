//! The lists drawn from the camera: colour, emissive, distortion and the
//! dynamic-light pass.
//!
//! The lanes (static world, models, effects) each keep their own sorted draw
//! list. Every frame this module merges them into the product lists the
//! renderer reads, trying the cheapest way that is still right:
//!
//! 1. *Nothing moved.* Same draws, same payload as last frame: reuse the
//!    published colour and emissive lists as they are.
//! 2. *Payload moved.* Same draws, new payload: write the new rows into the
//!    shape of last frame's lists straight from the lanes.
//! 3. *Draws moved.* Merge the lanes, drop what the current settings leave
//!    out, collapse repeats.
//!
//! A [`CameraStamp`] of everything the lists depend on tells the cases apart.

use bevy::prelude::*;
use render_frame::{
    FrameProduct, FrameProductKind, FrameProductStatus, MissingProductCause, ProductTarget,
};

use crate::assemble::drawsurf::retained::{RetainedDrawItem, StaticDrawLane};
use super::super::setup::TechType;
use super::super::{
    FxDrawLane, MapPrimaryLightTypes, SpotShadowMapLights, SunShadowMapPresent, XModelDrawLane,
};
use super::bank::{COLOUR_MISSING, RenderFrameProducts};
use super::dynamic_lights::{Reach, fill_light_list};
use super::lanes::{LaneMerge, Lanes, fill_exact, merge_into};
use super::slot::{Build, ListSlot};
use super::stamps::{CameraStamp, bytes_id};
use super::tech::{TechRule, kept_without_distortion};
use super::{ColourDrawMethod, DistortionSettings, FrameAssemblyInputs, MaterialGeneration};
use crate::prepare::scene::gfx_scene::HostGfxScene;
use crate::prepare::scene::world::WorldScene;

/// The camera lists being built, kept between frames so the cheaper paths
/// have last frame's shape to work from.
#[derive(Resource, Debug)]
pub struct CameraProducts {
    pub(super) colour: ListSlot,
    pub(super) emissive: ListSlot,
    pub(super) light: ListSlot,
    pub(super) distortion: FrameProduct,
    last: Option<CameraStamp>,
}

impl Default for CameraProducts {
    fn default() -> Self {
        Self {
            colour: ListSlot::new(FrameProductKind::Colour),
            emissive: ListSlot::new(FrameProductKind::Emissive),
            light: ListSlot::new(FrameProductKind::Light),
            distortion: super::bank::blank(FrameProductKind::Distortion),
            last: None,
        }
    }
}

/// Whether a draw belongs to the sort key the distortion pass handles.
fn distortion_sort_keep(row: &RetainedDrawItem, sort_key: u32) -> bool {
    u32::from(dpvs_t6::unpack(dpvs_t6::GfxDrawSurf { packed: row.key }).primary_sort_key)
        == sort_key
}

/// Rebuilds the distortion list: `previous` (last frame's published list) is
/// the starting point, `keep` picks the rows. With `same_shape` the rows are
/// written over it in place; otherwise, or if they do not fit, the lanes are
/// merged and filtered from scratch.
fn refresh_distortion(
    out: &mut FrameProduct,
    previous: &FrameProduct,
    lanes: Lanes<'_>,
    same_shape: bool,
    keep: impl Fn(&RetainedDrawItem) -> bool,
) {
    out.ordered_draws.clone_from(&previous.ordered_draws);
    let overlaid = same_shape
        && fill_exact(
            &mut out.ordered_draws,
            LaneMerge::new(lanes).filter(|row| keep(row)),
        );
    if !overlaid {
        merge_into(&mut out.ordered_draws, lanes);
        out.ordered_draws.retain(|row| keep(row));
    }
}

pub(crate) fn execute_camera_products(
    inputs: Res<FrameAssemblyInputs>,
    lanes: (Res<StaticDrawLane>, Res<XModelDrawLane>, Res<FxDrawLane>),
    draw_method: Res<ColourDrawMethod>,
    distortion_settings: Res<DistortionSettings>,
    generation: Res<MaterialGeneration>,
    lights: Option<Res<MapPrimaryLightTypes>>,
    scene: Option<Res<WorldScene>>,
    gfx: Option<Res<HostGfxScene>>,
    sun_present: Res<SunShadowMapPresent>,
    spot_lights: Res<SpotShadowMapLights>,
    published: Res<RenderFrameProducts>,
    mut owner: ResMut<CameraProducts>,
) {
    let _span = perf::Span::HostPostExecuteMs.enter();
    let (static_lane, xmodel_lane, fx_lane) = lanes;
    let (_, static_colour, static_emissive, static_distortion, world_run_surfs) =
        static_lane.live_for(inputs.world_generation);
    let sort_key = scene
        .as_deref()
        .and_then(|scene| scene.cull.as_ref())
        .and_then(|cull| cull.sort_key_distortion);
    let map_types = lights.as_ref().map_or(&[][..], |lights| lights.types.as_slice());
    let spot_shadowed = spot_lights.0.as_slice();
    let distortion_on = distortion_settings.enabled;

    let colour_tech = draw_method.tech_type();
    let emissive_tech = draw_method.emissive_tech_type();
    let stamp = CameraStamp {
        static_membership: if static_lane.world_generation == inputs.world_generation {
            static_lane.membership_revision
        } else {
            0
        },
        xmodel_membership: xmodel_lane.membership_revision,
        fx_membership: fx_lane.membership_revision,
        xmodel_payload: xmodel_lane.payload_revision,
        fx_payload: fx_lane.payload_revision,
        catalog: inputs.catalog_generation.0,
        world_generation: inputs.world_generation.0,
        colour_tech: colour_tech.0,
        emissive_tech: emissive_tech.0,
        dfog: inputs.dfog,
        sun_present: sun_present.0,
        distortion_enabled: distortion_on,
        distortion_sort_key: sort_key,
        light_types_id: bytes_id(map_types),
        spot_lights_id: bytes_id(spot_shadowed),
    };
    let build = Build {
        generation_id: static_lane.generation_id,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let colour_rule = TechRule::Lit {
        base: colour_tech,
        light_types: map_types,
        dfog: inputs.dfog,
        sun_shadow_map: sun_present.0,
        spot_shadowed,
    };
    let emissive_rule = TechRule::Single(emissive_tech);
    let all_light_types = super::super::dlight_receivers::combined_light_types(map_types, &inputs);
    let light_rule = TechRule::DynamicLight {
        light_types: &all_light_types,
        spot_shadowed,
    };
    let target = ProductTarget::Core3dViewColour;

    let CameraProducts {
        colour,
        emissive,
        light,
        distortion,
        last,
    } = owner.as_mut();
    let unchanged = last.is_some_and(|prev| prev.same_composition(&stamp) && prev.same_payload(&stamp));
    let same_shape = last.is_some_and(|prev| prev.same_composition(&stamp));
    let published_distortion = published.product(FrameProductKind::Distortion);

    let fill_light = |light: &mut ListSlot, colour: &ListSlot| {
        fill_light_list(
            light,
            &colour.product.ordered_draws,
            &inputs,
            Reach {
                scene: scene.as_deref(),
                gfx: gfx.as_deref().map(|gfx| &gfx.scene),
                world_run_surfs,
            },
            &light_rule,
            colour_tech,
            &build,
        );
    };

    // 1. nothing moved: last frame's lists stand.
    if unchanged
        && colour.carry_over(published.product(FrameProductKind::Colour))
        && emissive.carry_over(published.product(FrameProductKind::Emissive))
    {
        colour.ready(&build, colour_tech, target);
        emissive.ready(&build, emissive_tech, target);
        distortion.generation_id = inputs.catalog_generation;
        distortion
            .ordered_draws
            .clone_from(&published_distortion.ordered_draws);
        distortion.status = published_distortion.status;
        fill_light(light, colour);
        *last = Some(stamp);
        return;
    }

    colour.begin(COLOUR_MISSING);
    emissive.begin(COLOUR_MISSING);
    distortion.begin_fill(COLOUR_MISSING);

    let colour_lanes = [static_colour, &xmodel_lane.colour, &fx_lane.colour];
    let emissive_lanes = [static_emissive, &xmodel_lane.emissive, &fx_lane.emissive];

    // 2. payload moved: same shape as last frame, rows rewritten from the lanes.
    let overlaid = same_shape
        && colour.overlay(
            published.product(FrameProductKind::Colour),
            colour_lanes,
            |row| distortion_on || kept_without_distortion(&generation.catalog, &colour_rule, row),
        )
        && emissive.overlay(
            published.product(FrameProductKind::Emissive),
            emissive_lanes,
            |row| distortion_on || kept_without_distortion(&generation.catalog, &emissive_rule, row),
        );
    if overlaid {
        colour.ready(&build, colour_tech, target);
        emissive.ready(&build, emissive_tech, target);
    } else {
        // 3. draws moved: merge, filter, collapse.
        let passes: [(&mut ListSlot, Lanes<'_>, &TechRule<'_>, TechType); 2] = [
            (&mut *colour, colour_lanes, &colour_rule, colour_tech),
            (&mut *emissive, emissive_lanes, &emissive_rule, emissive_tech),
        ];
        for (slot, lanes, rule, tech) in passes {
            merge_into(&mut slot.product.ordered_draws, lanes);
            if !distortion_on {
                slot.product
                    .ordered_draws
                    .retain(|row| kept_without_distortion(&generation.catalog, rule, row));
            }
            slot.collapse(&build, rule, tech, target, same_shape);
        }
    }

    distortion.generation_id = inputs.catalog_generation;
    distortion.status = match sort_key {
        Some(sort_key) if distortion_on => {
            refresh_distortion(
                distortion,
                published_distortion,
                [static_distortion, &xmodel_lane.distortion, &fx_lane.distortion],
                same_shape,
                |row| distortion_sort_keep(row, sort_key),
            );
            FrameProductStatus::ResolveReady {
                target: ProductTarget::ResolvedPostSun,
            }
        }
        Some(_) => FrameProductStatus::ResolveReady {
            target: ProductTarget::ResolvedPostSun,
        },
        None => FrameProductStatus::Missing(MissingProductCause::MissingDistortionSortKey),
    };

    fill_light(light, colour);
    *last = Some(stamp);
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::super::lanes::tests::{Lcg, row};
    use super::super::publish::publish_frame_products;
    use super::*;
    use crate::assemble::drawsurf::{SunProduct, SpotProduct};

    #[test]
    fn distortion_overlay_matches_a_rebuild() {
        let a = [row(1, 1), row(3, 2), row(5, 3)];
        let b = [row(2, 4), row(3, 5)];
        let lanes: Lanes<'_> = [&a, &b, &[]];
        let keep = |r: &RetainedDrawItem| r.key != 2;

        // a previous list of the right length: overlay in place
        let mut previous = super::super::bank::blank(FrameProductKind::Distortion);
        previous.ordered_draws = vec![row(0, 0); 4];
        let mut out = super::super::bank::blank(FrameProductKind::Distortion);
        refresh_distortion(&mut out, &previous, lanes, true, keep);

        // the same through the from-scratch path
        let mut scratch = super::super::bank::blank(FrameProductKind::Distortion);
        refresh_distortion(&mut scratch, &previous, lanes, false, keep);
        assert_eq!(out.ordered_draws.len(), 4);
        assert_eq!(
            format!("{:?}", out.ordered_draws),
            format!("{:?}", scratch.ordered_draws)
        );
    }

    #[test]
    fn a_shape_mismatch_falls_back_to_the_merge() {
        let a = [row(1, 1), row(3, 2)];
        let lanes: Lanes<'_> = [&a, &[], &[]];
        let mut previous = super::super::bank::blank(FrameProductKind::Distortion);
        previous.ordered_draws = vec![row(0, 0); 5];
        let mut out = super::super::bank::blank(FrameProductKind::Distortion);
        refresh_distortion(&mut out, &previous, lanes, true, |_| true);
        assert_eq!(out.ordered_draws.len(), 2);
    }

    // ---- whole-system: incremental frames must equal from-scratch frames ----

    const KINDS: [FrameProductKind; 4] = [
        FrameProductKind::Colour,
        FrameProductKind::Emissive,
        FrameProductKind::Distortion,
        FrameProductKind::Light,
    ];

    fn blank_world() -> World {
        let mut world = World::new();
        world.init_resource::<FrameAssemblyInputs>();
        world.init_resource::<StaticDrawLane>();
        world.init_resource::<XModelDrawLane>();
        world.init_resource::<FxDrawLane>();
        world.init_resource::<ColourDrawMethod>();
        world.init_resource::<DistortionSettings>();
        world.init_resource::<MaterialGeneration>();
        world.init_resource::<SunShadowMapPresent>();
        world.init_resource::<SpotShadowMapLights>();
        world.init_resource::<RenderFrameProducts>();
        world.init_resource::<CameraProducts>();
        world.init_resource::<SunProduct>();
        world.init_resource::<SpotProduct>();
        world
    }

    #[derive(Clone, Default)]
    struct Spec {
        statics: Vec<RetainedDrawItem>,
        xmodel: Vec<RetainedDrawItem>,
        fx: Vec<RetainedDrawItem>,
        distortion_on: bool,
        sun: bool,
        dfog: bool,
    }

    fn sorted_rows(rng: &mut Lcg, len: usize, tag: u16) -> Vec<RetainedDrawItem> {
        let mut rows: Vec<_> = (0..len)
            .map(|i| row(rng.next(6) << 8, tag * 100 + (rng.next(4) as u16) + i as u16 % 2))
            .collect();
        rows.sort_by_key(|r| (r.key, crate::assemble::drawsurf::retained::draw_order_tie(&r.kind)));
        rows
    }

    fn load(world: &mut World, spec: &Spec) {
        world.resource_mut::<StaticDrawLane>().colour = spec.statics.clone();
        world.resource_mut::<StaticDrawLane>().emissive = spec.statics.iter().step_by(2).copied().collect();
        {
            let mut lane = world.resource_mut::<XModelDrawLane>();
            lane.colour = spec.xmodel.clone();
            lane.emissive = spec.xmodel.iter().step_by(2).copied().collect();
        }
        {
            let mut lane = world.resource_mut::<FxDrawLane>();
            lane.colour = spec.fx.clone();
            lane.emissive = spec.fx.iter().step_by(3).copied().collect();
        }
        world.resource_mut::<DistortionSettings>().enabled = spec.distortion_on;
        world.resource_mut::<SunShadowMapPresent>().0 = spec.sun;
        world.resource_mut::<FrameAssemblyInputs>().dfog = spec.dfog;
    }

    fn run_frame(world: &mut World) {
        world.run_system_once(execute_camera_products).unwrap();
        world.run_system_once(publish_frame_products).unwrap();
    }

    fn lists(world: &World) -> Vec<String> {
        let published = world.resource::<RenderFrameProducts>().published();
        KINDS
            .iter()
            .map(|&kind| format!("{:?}", published.product(kind)))
            .collect()
    }

    #[test]
    fn incremental_frames_equal_fresh_frames() {
        let mut rng = Lcg(0x5eed);
        let mut spec = Spec {
            statics: sorted_rows(&mut rng, 9, 1),
            xmodel: sorted_rows(&mut rng, 7, 2),
            fx: sorted_rows(&mut rng, 5, 3),
            distortion_on: true,
            ..Default::default()
        };
        let mut live = blank_world();
        let mut payload_rev = 0u64;
        let mut membership_rev = 0u64;

        for frame in 0..60 {
            match rng.next(7) {
                0 => {}
                1 | 2 => {
                    // payload only: same draws, different material rank
                    for r in spec.xmodel.iter_mut().chain(spec.fx.iter_mut()) {
                        r.material_rank = (rng.next(5)) as _;
                    }
                    payload_rev += 1;
                    let mut lane = live.resource_mut::<XModelDrawLane>();
                    lane.payload_revision = payload_rev;
                    drop(lane);
                    live.resource_mut::<FxDrawLane>().payload_revision = payload_rev;
                }
                3 => {
                    let count = 3 + rng.next(8) as usize;
                    spec.xmodel = sorted_rows(&mut rng, count, 2);
                    membership_rev += 1;
                    live.resource_mut::<XModelDrawLane>().membership_revision = membership_rev;
                }
                4 => {
                    let count = rng.next(6) as usize;
                    spec.fx = sorted_rows(&mut rng, count, 3);
                    membership_rev += 1;
                    live.resource_mut::<FxDrawLane>().membership_revision = membership_rev;
                }
                5 => {
                    spec.distortion_on = !spec.distortion_on;
                }
                _ => {
                    spec.sun = !spec.sun;
                }
            }
            load(&mut live, &spec);
            run_frame(&mut live);

            // the revisions are part of a list's digest, so the fresh world carries the same ones
            let mut fresh = blank_world();
            {
                let mut lane = fresh.resource_mut::<XModelDrawLane>();
                lane.membership_revision = live.resource::<XModelDrawLane>().membership_revision;
                lane.payload_revision = live.resource::<XModelDrawLane>().payload_revision;
            }
            {
                let mut lane = fresh.resource_mut::<FxDrawLane>();
                lane.membership_revision = live.resource::<FxDrawLane>().membership_revision;
                lane.payload_revision = live.resource::<FxDrawLane>().payload_revision;
            }
            load(&mut fresh, &spec);
            run_frame(&mut fresh);

            let (got, want) = (lists(&live), lists(&fresh));
            for (kind, (got, want)) in KINDS.iter().zip(got.iter().zip(&want)) {
                assert_eq!(got, want, "frame {frame}, {kind:?}");
            }
        }
    }
}
