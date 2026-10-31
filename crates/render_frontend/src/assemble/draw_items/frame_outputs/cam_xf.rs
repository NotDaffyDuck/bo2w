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
    FrameOutputItem, FrameOutputKind, FrameOutputState, AbsentOutputReason, OutputTarget,
};

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, StaticDrawTrack};
use super::super::material_setup::MethodType;
use super::super::{
    EffectDrawTrack, MapKeyLightTypes, ConeShadowMapLights, DaylightShadowMapPresent, FigureDrawTrack,
};
use super::bank::COLOUR_MISSING;
use super::bank::FrameRenderOutputs;
use super::dynamic_lights::{Reach, fill_light_list};
use super::lane_set::{LANE_COUNT, LaneMerge, fill_exact, merge_into};
use super::seat::{Build, ListSlot};
use super::stamps::{CameraStamp, bytes_id};
use super::technique_ix::{TechRule, kept_without_distortion};
use crate::assemble::draw_items::frame_outputs::{ColourPaintMethod, WarpSettings, FrameBuildFeed, MaterialEpoch};
use crate::prepare::scene::visual_stage::HostVisualStage;
use crate::prepare::scene::world::LevelStage;

/// The camera lists being built, kept between frames so the cheaper paths
/// have last frame's shape to work from.
#[derive(Resource, Debug)]
pub struct CameraOutputs {
    pub(super) colour: ListSlot,
    pub(super) emissive: ListSlot,
    pub(super) light: ListSlot,
    pub(super) distortion: FrameOutputItem,
    last: Option<CameraStamp>,
}

impl Default for CameraOutputs {
    fn default() -> Self {
        Self {
            colour: ListSlot::new(FrameOutputKind::Colour),
            emissive: ListSlot::new(FrameOutputKind::Emissive),
            light: ListSlot::new(FrameOutputKind::Light),
            distortion: super::bank::blank(FrameOutputKind::RefractionPass),
            last: None,
        }
    }
}

/// Whether a draw belongs to the sort key the distortion pass handles.
fn distortion_sort_keep(row: &HeldDrawItem, order_key: u32) -> bool {
    u32::from(dpvs_t6::unpack(dpvs_t6::SortedDraw { packed: row.key }).leading_sort_bucket)
        == order_key
}

/// Rebuilds the distortion list: `previous` (last frame's published list) is
/// the starting point, `keep` picks the rows. With `same_shape` the rows are
/// written over it in place; otherwise, or if they do not fit, the lanes are
/// merged and filtered from scratch.
fn refresh_distortion(
    out: &mut FrameOutputItem,
    previous: &FrameOutputItem,
    lanes: [&[HeldDrawItem]; LANE_COUNT],
    same_shape: bool,
    keep: impl Fn(&HeldDrawItem) -> bool,
) {
    out.draw_sequence.clone_from(&previous.draw_sequence);
    let overlaid = same_shape
        && fill_exact(
            &mut out.draw_sequence,
            LaneMerge::new(lanes).filter(|row| keep(row)),
        );
    if !overlaid {
        merge_into(&mut out.draw_sequence, lanes);
        out.draw_sequence.retain(|row| keep(row));
    }
}

pub(crate) fn execute_camera_outputs(
    inputs: Res<FrameBuildFeed>,
    lanes: (Res<StaticDrawTrack>, Res<FigureDrawTrack>, Res<EffectDrawTrack>),
    draw_method: Res<ColourPaintMethod>,
    distortion_settings: Res<WarpSettings>,
    generation: Res<MaterialEpoch>,
    lights: Option<Res<MapKeyLightTypes>>,
    scene: Option<Res<LevelStage>>,
    gfx: Option<Res<HostVisualStage>>,
    daylight_present: Res<DaylightShadowMapPresent>,
    spot_lights: Res<ConeShadowMapLights>,
    published: Res<FrameRenderOutputs>,
    mut holder: ResMut<CameraOutputs>,
) {
    let _span = perf::Span::ServerAfterRunMillis.enter();
    let (static_lane, model_lane, effect_lane) = lanes;
    let (_, fixed_colour, fixed_emissive, fixed_distortion, level_run_facets) =
        static_lane.alive_for(inputs.level_generation);
    let sort_key = scene
        .as_deref()
        .and_then(|scene| scene.culling.as_ref())
        .and_then(|cull| cull.warp_sort_key);
    let map_types = lights.as_ref().map_or(&[][..], |lights| lights.types.as_slice());
    let spot_has_shadow = spot_lights.0.as_slice();
    let distortion_on = distortion_settings.enabled;

    let colour_tech = draw_method.shading_technique_kind();
    let emissive_tech = draw_method.emissive_tech_type();
    let stamp = CameraStamp {
        static_membership: if static_lane.level_generation == inputs.level_generation {
            static_lane.membership_edition
        } else {
            0
        },
        xmodel_membership: model_lane.membership_edition,
        fx_membership: effect_lane.membership_edition,
        xmodel_payload: model_lane.payload_edition,
        fx_payload: effect_lane.payload_edition,
        catalog: inputs.ledger_generation.0,
        level_generation: inputs.level_generation.0,
        colour_tech: colour_tech.0,
        emissive_tech: emissive_tech.0,
        depth_haze: inputs.depth_haze,
        daylight_present: daylight_present.0,
        distortion_enabled: distortion_on,
        distortion_sort_key: sort_key,
        light_types_id: bytes_id(map_types),
        spot_lights_id: bytes_id(spot_has_shadow),
    };
    let build = Build {
        revision_stamp: static_lane.revision_stamp,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let colour_rule = TechRule::Lit {
        base: colour_tech,
        light_types: map_types,
        depth_haze: inputs.depth_haze,
        daylight_shadow_map: daylight_present.0,
        spot_shadowed: spot_has_shadow,
    };
    let emissive_rule = TechRule::Single(emissive_tech);
    let all_light_types = super::super::lamp_receivers::combined_light_types(map_types, &inputs);
    let light_rule = TechRule::DynamicLight {
        light_types: &all_light_types,
        spot_shadowed: spot_has_shadow,
    };
    let target = OutputTarget::MainViewColor;

    let CameraOutputs {
        colour: rgb_val,
        emissive: self_lit,
        light,
        distortion: warp_pass,
        last,
    } = holder.as_mut();
    let unchanged = last.is_some_and(|prev| prev.same_composition(&stamp) && prev.same_payload(&stamp));
    let same_shape = last.is_some_and(|prev| prev.same_composition(&stamp));
    let published_distortion = published.product(FrameOutputKind::RefractionPass);

    let fill_light = |light: &mut ListSlot, colour: &ListSlot| {
        fill_light_list(
            light,
            &colour.product.draw_sequence,
            &inputs,
            Reach {
                scene: scene.as_deref(),
                gfx: gfx.as_deref().map(|gfx| &gfx.scene),
                level_run_facets,
            },
            &light_rule,
            colour_tech,
            &build,
        );
    };

    // 1. nothing moved: last frame's lists stand.
    if unchanged
        && rgb_val.carry_over(published.product(FrameOutputKind::Colour))
        && self_lit.carry_over(published.product(FrameOutputKind::Emissive))
    {
        rgb_val.ready(&build, colour_tech, target);
        self_lit.ready(&build, emissive_tech, target);
        warp_pass.revision_stamp = inputs.ledger_generation;
        warp_pass
            .draw_sequence
            .clone_from(&published_distortion.draw_sequence);
        warp_pass.state = published_distortion.state;
        fill_light(light, rgb_val);
        *last = Some(stamp);
        return;
    }

    rgb_val.begin(COLOUR_MISSING);
    self_lit.begin(COLOUR_MISSING);
    warp_pass.start_fill(COLOUR_MISSING);

    let colour_lanes = [fixed_colour, &model_lane.tint_rgb, &effect_lane.colour];
    let emissive_lanes = [fixed_emissive, &model_lane.glow, &effect_lane.emissive];

    // 2. payload moved: same shape as last frame, rows rewritten from the lanes.
    let overlaid = same_shape
        && rgb_val.overlay(
            published.product(FrameOutputKind::Colour),
            colour_lanes,
            |row| distortion_on || kept_without_distortion(&generation.catalog, &colour_rule, row),
        )
        && self_lit.overlay(
            published.product(FrameOutputKind::Emissive),
            emissive_lanes,
            |row| distortion_on || kept_without_distortion(&generation.catalog, &emissive_rule, row),
        );
    if overlaid {
        rgb_val.ready(&build, colour_tech, target);
        self_lit.ready(&build, emissive_tech, target);
    } else {
        // 3. draws moved: merge, filter, collapse.
        let passes: [(&mut ListSlot, [&[HeldDrawItem]; LANE_COUNT], &TechRule<'_>, MethodType); 2] = [
            (&mut *rgb_val, colour_lanes, &colour_rule, colour_tech),
            (&mut *self_lit, emissive_lanes, &emissive_rule, emissive_tech),
        ];
        for (slot, lanes, rule, tech) in passes {
            merge_into(&mut slot.product.draw_sequence, lanes);
            if !distortion_on {
                slot.product
                    .draw_sequence
                    .retain(|row| kept_without_distortion(&generation.catalog, rule, row));
            }
            slot.collapse(&build, rule, tech, target, same_shape);
        }
    }

    warp_pass.revision_stamp = inputs.ledger_generation;
    warp_pass.state = match sort_key {
        Some(sort_key) if distortion_on => {
            refresh_distortion(
                warp_pass,
                published_distortion,
                [fixed_distortion, &model_lane.warp, &effect_lane.distortion],
                same_shape,
                |row| distortion_sort_keep(row, sort_key),
            );
            FrameOutputState::ResolveReady {
                target: OutputTarget::FinishedAfterSun,
            }
        }
        Some(_) => FrameOutputState::ResolveReady {
            target: OutputTarget::FinishedAfterSun,
        },
        None => FrameOutputState::Missing(AbsentOutputReason::RefractionOrderKeyAbsent),
    };

    fill_light(light, rgb_val);
    *last = Some(stamp);
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::super::lane_set::tests::{Lcg, row};
    use super::super::publish::publish_frame_outputs;
    use super::*;
    use crate::assemble::draw_items::DaylightOutput;
    use crate::assemble::draw_items::ConeOutput;

    #[test]
    fn distortion_overlay_matches_a_rebuild() {
        let a = [row(1, 1), row(3, 2), row(5, 3)];
        let b = [row(2, 4), row(3, 5)];
        let lanes: [&[HeldDrawItem]; LANE_COUNT] = [&a, &b, &[]];
        let keep = |r: &HeldDrawItem| r.key != 2;

        // a previous list of the right length: overlay in place
        let mut previous = super::super::bank::blank(FrameOutputKind::RefractionPass);
        previous.draw_sequence = vec![row(0, 0); 4];
        let mut out = super::super::bank::blank(FrameOutputKind::RefractionPass);
        refresh_distortion(&mut out, &previous, lanes, true, keep);

        // the same through the from-scratch path
        let mut scratch = super::super::bank::blank(FrameOutputKind::RefractionPass);
        refresh_distortion(&mut scratch, &previous, lanes, false, keep);
        assert_eq!(out.draw_sequence.len(), 4);
        assert_eq!(
            format!("{:?}", out.draw_sequence),
            format!("{:?}", scratch.draw_sequence)
        );
    }

    #[test]
    fn a_shape_mismatch_falls_back_to_the_merge() {
        let a = [row(1, 1), row(3, 2)];
        let lanes: [&[HeldDrawItem]; LANE_COUNT] = [&a, &[], &[]];
        let mut previous = super::super::bank::blank(FrameOutputKind::RefractionPass);
        previous.draw_sequence = vec![row(0, 0); 5];
        let mut out = super::super::bank::blank(FrameOutputKind::RefractionPass);
        refresh_distortion(&mut out, &previous, lanes, true, |_| true);
        assert_eq!(out.draw_sequence.len(), 2);
    }

    // ---- whole-system: incremental frames must equal from-scratch frames ----

    const KINDS: [FrameOutputKind; 4] = [
        FrameOutputKind::Colour,
        FrameOutputKind::Emissive,
        FrameOutputKind::RefractionPass,
        FrameOutputKind::Light,
    ];

    fn blank_world() -> World {
        let mut world = World::new();
        world.init_resource::<FrameBuildFeed>();
        world.init_resource::<StaticDrawTrack>();
        world.init_resource::<FigureDrawTrack>();
        world.init_resource::<EffectDrawTrack>();
        world.init_resource::<ColourPaintMethod>();
        world.init_resource::<WarpSettings>();
        world.init_resource::<MaterialEpoch>();
        world.init_resource::<DaylightShadowMapPresent>();
        world.init_resource::<ConeShadowMapLights>();
        world.init_resource::<FrameRenderOutputs>();
        world.init_resource::<CameraOutputs>();
        world.init_resource::<DaylightOutput>();
        world.init_resource::<ConeOutput>();
        world
    }

    #[derive(Clone, Default)]
    struct Spec {
        statics: Vec<HeldDrawItem>,
        xmodel: Vec<HeldDrawItem>,
        fx: Vec<HeldDrawItem>,
        distortion_on: bool,
        sun: bool,
        depth_haze: bool,
    }

    fn sorted_rows(rng: &mut Lcg, len: usize, tag: u16) -> Vec<HeldDrawItem> {
        let mut rows: Vec<_> = (0..len)
            .map(|i| row(rng.next(6) << 8, tag * 100 + (rng.next(4) as u16) + i as u16 % 2))
            .collect();
        rows.sort_by_key(|r| (r.key, crate::assemble::draw_items::kept_lanes::draw_order_tie(&r.kind)));
        rows
    }

    fn load(world: &mut World, spec: &Spec) {
        world.resource_mut::<StaticDrawTrack>().tint_rgb = spec.statics.clone();
        world.resource_mut::<StaticDrawTrack>().glow = spec.statics.iter().step_by(2).copied().collect();
        {
            let mut lane = world.resource_mut::<FigureDrawTrack>();
            lane.tint_rgb = spec.xmodel.clone();
            lane.glow = spec.xmodel.iter().step_by(2).copied().collect();
        }
        {
            let mut lane = world.resource_mut::<EffectDrawTrack>();
            lane.colour = spec.fx.clone();
            lane.emissive = spec.fx.iter().step_by(3).copied().collect();
        }
        world.resource_mut::<WarpSettings>().enabled = spec.distortion_on;
        world.resource_mut::<DaylightShadowMapPresent>().0 = spec.sun;
        world.resource_mut::<FrameBuildFeed>().depth_haze = spec.depth_haze;
    }

    fn run_frame(world: &mut World) {
        world.run_system_once(execute_camera_outputs).unwrap();
        world.run_system_once(publish_frame_outputs).unwrap();
    }

    fn lists(world: &World) -> Vec<String> {
        let published = world.resource::<FrameRenderOutputs>().published();
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
                        r.material_order_no = (rng.next(5)) as _;
                    }
                    payload_rev += 1;
                    let mut lane = live.resource_mut::<FigureDrawTrack>();
                    lane.payload_edition = payload_rev;
                    drop(lane);
                    live.resource_mut::<EffectDrawTrack>().payload_edition = payload_rev;
                }
                3 => {
                    let count = 3 + rng.next(8) as usize;
                    spec.xmodel = sorted_rows(&mut rng, count, 2);
                    membership_rev += 1;
                    live.resource_mut::<FigureDrawTrack>().membership_edition = membership_rev;
                }
                4 => {
                    let count = rng.next(6) as usize;
                    spec.fx = sorted_rows(&mut rng, count, 3);
                    membership_rev += 1;
                    live.resource_mut::<EffectDrawTrack>().membership_edition = membership_rev;
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
                let mut lane = fresh.resource_mut::<FigureDrawTrack>();
                lane.membership_edition = live.resource::<FigureDrawTrack>().membership_edition;
                lane.payload_edition = live.resource::<FigureDrawTrack>().payload_edition;
            }
            {
                let mut lane = fresh.resource_mut::<EffectDrawTrack>();
                lane.membership_edition = live.resource::<EffectDrawTrack>().membership_edition;
                lane.payload_edition = live.resource::<EffectDrawTrack>().payload_edition;
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
