//! Turning the live elements of every effect into what the renderer draws: sprites,
//! trails, particle clouds, lights and models.
//!
//! `generate_verts_with_trails` walks the live effects and asks for each element's look
//! through six callbacks. Every callback here reads the element's definition from the
//! catalog, samples its curves at the element's age, and returns `None` for an element
//! that should not be drawn.

use std::cell::RefCell;
use std::sync::Arc;

use asset_game::{FxDefinitions, OwnedFxEffectDef, OwnedFxElemDef};
use fx::{
    FxCloudInstance, FxDrawElemContext, FxDrawTrailContext, FxDrawTrailSampleContext,
    FxElemLightInstance, FxGenerateVertsOut, FxModelInstance, FxPackedLightingSrc,
    FxSpriteInstance, FxSystemHost, FxTrailDrawDef, FxTrailSampleVisual,
    generate_verts_with_trails,
};
use fx_t6::runtime::{
    FX_PARTICLE_CLOUD_TEMPLATE_CELLS, FX_RAND_CH_COLOR, FX_RAND_CH_SCALE, FX_RAND_CH_SIZE0,
    FxElemType, apply_lighting_frac_bgra, build_cloud, clamp_elem_rotation_time, cull_cloud,
    cull_elem_light, elem_light_color_bgr, elem_uses_lighting_frac, evaluate_color_bgra,
    evaluate_distance_fade, evaluate_rotation_total, evaluate_scale, evaluate_size0,
    evaluate_size1, get_elem_angles_axis, get_velocity_at_time, random_table_f32,
    sprite_atlas_uv, vec3_distance, vec3_normalize, vis_blocker_add_prepared,
};

use super::catalog::DrawLookup;
use super::t6::{NONUNIFORM_SCALE, light_scale, swap_red_blue};
use super::{FxDrawCull, FxVertsGaps};

/// What an element is drawn with when its colour curve gives nothing.
const WHITE: [u8; 4] = [255; 4];

fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

/// The light-tinting step: an element with a lighting fraction is tinted by the light the
/// effect sits in, unless that light was never sampled.
pub(super) fn light_bgra(
    bgra: [u8; 4],
    fraction: u8,
    sample: [u8; 3],
    src: FxPackedLightingSrc,
) -> [u8; 4] {
    if !elem_uses_lighting_frac(fraction) || src == FxPackedLightingSrc::Missing {
        return bgra;
    }
    apply_lighting_frac_bgra(bgra, sample, fraction)
}

/// `light_bgra`, also recording in `gaps` how the tint was (or was not) found.
pub(super) fn light_counted(
    gaps: &mut FxVertsGaps,
    def_index: u8,
    bgra: [u8; 4],
    fraction: u8,
    sample: [u8; 3],
    src: FxPackedLightingSrc,
) -> [u8; 4] {
    if elem_uses_lighting_frac(fraction) {
        bump(&mut gaps.lighting_frac);
        match src {
            FxPackedLightingSrc::Missing => {
                bump(&mut gaps.lighting_miss);
                gaps.lighting_frac_def_indices.push(def_index);
            }
            FxPackedLightingSrc::White => {
                bump(&mut gaps.lighting_applied);
                bump(&mut gaps.lighting_white);
            }
            FxPackedLightingSrc::AtPointRgb => {
                bump(&mut gaps.lighting_applied);
                bump(&mut gaps.lighting_atpoint);
            }
        }
    }
    light_bgra(bgra, fraction, sample, src)
}

/// One element's visual curves, read at one moment of its life.
struct Visuals<'e> {
    elem: &'e OwnedFxElemDef,
    seed: u32,
    norm_time: f32,
}

impl<'e> Visuals<'e> {
    fn at(elem: &'e OwnedFxElemDef, seed: u32, norm_time: f32) -> Self {
        Self {
            elem,
            seed,
            norm_time,
        }
    }

    fn samples(&self) -> &'e [u8] {
        self.elem.vis_samples.as_slice()
    }

    fn intervals(&self) -> u8 {
        self.elem.view.vis_state_interval_count
    }

    fn roll(&self, channel: u32) -> f32 {
        random_table_f32(self.seed, channel)
    }

    fn size0(&self) -> Option<f32> {
        let roll = self.roll(FX_RAND_CH_SIZE0);
        evaluate_size0(self.samples(), self.intervals(), self.norm_time, roll)
    }

    /// The second size shares the first one's random roll.
    fn size1(&self) -> Option<f32> {
        let roll = self.roll(FX_RAND_CH_SIZE0);
        evaluate_size1(self.samples(), self.intervals(), self.norm_time, roll)
    }

    fn scale_roll(&self) -> f32 {
        self.roll(FX_RAND_CH_SCALE)
    }

    fn scale(&self) -> Option<f32> {
        evaluate_scale(
            self.samples(),
            self.intervals(),
            self.norm_time,
            self.scale_roll(),
        )
    }

    /// Blue, green, red, alpha.
    fn color_bgra(&self) -> Option<[u8; 4]> {
        let roll = self.roll(FX_RAND_CH_COLOR);
        evaluate_color_bgra(self.samples(), self.intervals(), self.norm_time, roll)
    }

    fn rotation(&self, life_msec: i32) -> Option<f32> {
        evaluate_rotation_total(
            self.samples(),
            self.intervals(),
            self.norm_time,
            self.seed,
            self.elem.view.initial_rotation,
            life_msec as f32,
        )
    }
}

/// The element's orientation after spinning for `time` milliseconds.
fn spun_axis(elem: &OwnedFxElemDef, ctx: &FxDrawElemContext<'_>, time: f32) -> [[f32; 3]; 3] {
    get_elem_angles_axis(
        elem.view.spawn_angles,
        elem.view.angular_velocity,
        ctx.elem_random_seed,
        time,
        ctx.axis,
    )
}

/// The orientation of an element that stops spinning once it has come to rest.
fn resting_axis(elem: &OwnedFxElemDef, ctx: &FxDrawElemContext<'_>) -> [[f32; 3]; 3] {
    let time = clamp_elem_rotation_time(
        ctx.age_msec as f32,
        ctx.life_msec as f32,
        ctx.at_rest_fraction,
    );
    spun_axis(elem, ctx, time)
}

/// Where the element is heading right now.
fn velocity_now(elem: &OwnedFxElemDef, ctx: &FxDrawElemContext<'_>) -> [f32; 3] {
    get_velocity_at_time(
        ctx.flags,
        ctx.base_vel,
        ctx.age_msec as f32,
        ctx.life_msec.max(1) as f32,
        elem.vel_graph_local.as_slice(),
        elem.vel_graph_world.as_slice(),
        ctx.axis,
        ctx.elem_random_seed,
    )
}

/// A drawn sprite or trail sample that may block sight, reported after the pass so the
/// sight-blocker buffer is written once the walk over the effects has ended.
struct Blocker {
    flags: i32,
    origin: [f32; 3],
    size0: f32,
    alpha: u8,
    fade_in: [f32; 2],
    fade_out: [f32; 2],
}

impl Blocker {
    fn of(elem: &OwnedFxElemDef, origin: [f32; 3], size0: f32, alpha: u8) -> Self {
        Self {
            flags: elem.view.flags,
            origin,
            size0,
            alpha,
            fade_in: elem.view.fade_in_range,
            fade_out: elem.view.fade_out_range,
        }
    }
}

/// Most neighbouring sprites come from the same effect and material, so they share one
/// pair of name strings instead of allocating two per sprite. Lives for one pass.
#[derive(Default)]
struct NameCache(Option<(Arc<str>, Arc<str>)>);

impl NameCache {
    fn names(&mut self, effect: &str, material: &str) -> (Arc<str>, Arc<str>) {
        if let Some((cached_effect, cached_material)) = &self.0
            && &**cached_effect == effect
            && &**cached_material == material
        {
            return (Arc::clone(cached_effect), Arc::clone(cached_material));
        }
        let fresh: (Arc<str>, Arc<str>) = (Arc::from(effect), Arc::from(material));
        self.0 = Some(fresh.clone());
        fresh
    }
}

/// Everything the callbacks share during one pass.
struct Drawer<'a> {
    lookup: DrawLookup<'a>,
    cull: FxDrawCull<'a>,
    blockers: RefCell<Vec<Blocker>>,
}

impl<'a> Drawer<'a> {
    fn plane_count(&self) -> u32 {
        self.cull.planes.len() as u32
    }

    fn find_elem(
        &self,
        catalog_index: u16,
        def_index: u8,
    ) -> Option<(&'a OwnedFxEffectDef, &'a OwnedFxElemDef)> {
        let effect = self.lookup.find(catalog_index)?;
        let elem = effect.elems.get(usize::from(def_index))?;
        Some((effect, elem))
    }

    fn block(&self, blocker: Blocker) {
        self.blockers.borrow_mut().push(blocker);
    }

    /// A billboard, oriented or tail sprite. Every reason for dropping one is counted in
    /// `gaps`.
    fn sprite(
        &self,
        ctx: FxDrawElemContext<'_>,
        camera: [f32; 3],
        gaps: &mut FxVertsGaps,
        names: &mut NameCache,
    ) -> Option<FxSpriteInstance> {
        let Some(effect) = self.lookup.find(ctx.catalog_index) else {
            bump(&mut gaps.no_def);
            return None;
        };
        let Some(elem) = effect.elems.get(usize::from(ctx.def_index)) else {
            bump(&mut gaps.no_elem);
            return None;
        };
        let Some((material_index, material_name)) = elem.primary_material() else {
            bump(&mut gaps.no_material_visual);
            return None;
        };
        let visuals = Visuals::at(elem, ctx.elem_random_seed, ctx.norm_time);
        let is_tail = ctx.elem_type == FxElemType::Tail as u8;

        let Some(size0) = visuals.size0() else {
            bump(&mut gaps.no_size0_sample);
            return None;
        };
        if size0 <= 0.0 {
            bump(&mut gaps.size0_not_positive);
            return None;
        }
        let size1 = if is_tail {
            let Some(size1) = visuals.size1() else {
                bump(&mut gaps.no_size1_sample);
                return None;
            };
            if size1 <= 0.0 {
                bump(&mut gaps.size1_not_positive);
                return None;
            }
            size1
        } else if ctx.flags & NONUNIFORM_SCALE != 0 {
            // Wide-by-tall sprite; without a usable height it stays square.
            visuals.size1().filter(|height| *height > 0.0).unwrap_or(0.0)
        } else {
            0.0
        };

        let mut color = visuals.color_bgra().unwrap_or_else(|| {
            bump(&mut gaps.color_fallback);
            WHITE
        });
        // Sight blocking uses the alpha as sampled, before the distance fade below.
        let sampled_alpha = color[3];

        // Elements keep clear of the eye and far ones fade out; one faded to nothing is
        // not drawn at all.
        let fade = evaluate_distance_fade(
            vec3_distance(ctx.origin, camera),
            elem.view.fade_in_range[0],
            elem.view.fade_in_range[1],
            elem.view.fade_out_range[0],
            elem.view.fade_out_range[1],
        );
        if let Some(fade) = fade {
            let faded = u32::from(color[3]) * fade.min(255) / 255;
            if faded == 0 {
                return None;
            }
            color[3] = faded as u8;
        }
        let color = light_counted(
            gaps,
            ctx.def_index,
            color,
            elem.view.lighting_frac,
            ctx.packed_lighting,
            ctx.packed_lighting_src,
        );

        let axis = if ctx.elem_type == FxElemType::Oriented as u8 {
            spun_axis(elem, &ctx, ctx.age_msec as f32)
        } else {
            ctx.axis
        };
        let vel_dir = if is_tail {
            vec3_normalize(velocity_now(elem, &ctx))
        } else {
            [0.0; 3]
        };
        let atlas = sprite_atlas_uv(
            &elem.atlas,
            ctx.elem_random_seed,
            ctx.sequence,
            ctx.age_msec,
            ctx.norm_time,
        );

        self.block(Blocker::of(elem, ctx.origin, size0, sampled_alpha));
        let (def_name, material_name) = names.names(ctx.def_name, material_name);
        Some(FxSpriteInstance {
            origin: ctx.origin,
            size0,
            size1,
            color_rgba: swap_red_blue(color),
            elem_type: ctx.elem_type,
            def_name,
            catalog_index: ctx.catalog_index,
            def_index: ctx.def_index,
            material_name,
            material_index: Some(material_index),
            axis,
            rotation_rad: visuals.rotation(ctx.life_msec).unwrap_or(0.0),
            vel_dir,
            atlas,
            flags: ctx.flags,
            line: elem.view.line,
            lighting_sample: ctx.packed_lighting,
            lighting_frac: elem.view.lighting_frac,
            packed_lighting_src: ctx.packed_lighting_src,
        })
    }

    /// The shape a trail element is stamped along, when it has one worth drawing.
    fn trail_def(
        &self,
        ctx: FxDrawTrailContext<'_>,
        def_index: u8,
    ) -> Option<FxTrailDrawDef> {
        let (_, elem) = self.find_elem(ctx.catalog_index, def_index)?;
        let trail = elem.trail_def.as_ref()?;
        if trail.verts.is_empty() || trail.inds.len() < 2 || trail.repeat_dist == 0 {
            return None;
        }
        let (material_index, material_name) = elem.primary_material()?;
        Some(FxTrailDrawDef {
            visual_count: elem.view.visual_count,
            flags: elem.view.flags,
            life_base: elem.view.life_span_msec_base,
            life_amp: elem.view.life_span_msec_amplitude,
            scroll_time_msec: trail.scroll_time_msec,
            repeat_dist: trail.repeat_dist,
            verts: trail.verts.clone(),
            inds: trail.inds.clone(),
            material_name: material_name.to_owned(),
            material_index: Some(material_index),
        })
    }

    /// How one point along a trail looks.
    fn trail_sample(&self, ctx: FxDrawTrailSampleContext<'_>) -> Option<FxTrailSampleVisual> {
        let (_, elem) = self.find_elem(ctx.catalog_index, ctx.def_index)?;
        let visuals = Visuals::at(elem, ctx.elem_random_seed, ctx.norm_time);
        let size0 = visuals.size0()?;
        if size0 <= 0.0 {
            return None;
        }
        let size1 = visuals.size1().unwrap_or(size0);
        let color = visuals.color_bgra().unwrap_or(WHITE);

        self.block(Blocker::of(elem, ctx.sample_origin, size0, color[3]));
        Some(FxTrailSampleVisual {
            size: [size0, size1],
            color_rgba: swap_red_blue(color),
            rotation: visuals.rotation(ctx.life_msec).unwrap_or(0.0),
        })
    }

    fn cloud(&self, ctx: FxDrawElemContext<'_>) -> Option<FxCloudInstance> {
        let (_, elem) = self.find_elem(ctx.catalog_index, ctx.def_index)?;
        let visuals = Visuals::at(elem, ctx.elem_random_seed, ctx.norm_time);
        let size0 = visuals.size0()?;
        let size1 = visuals.size1().unwrap_or(size0);
        let scale = visuals.scale()?;
        if cull_cloud(
            self.cull.elem_draw,
            self.cull.planes,
            self.plane_count(),
            ctx.flags,
            ctx.origin,
            size0,
            size1,
            scale,
        ) {
            return None;
        }
        let color = light_bgra(
            visuals.color_bgra().unwrap_or(WHITE),
            elem.view.lighting_frac,
            ctx.packed_lighting,
            ctx.packed_lighting_src,
        );
        // The definition's particle count, plus a random extra that grows with the scale
        // roll; clouds without one take the template's default.
        let particles = elem.cloud_density.map(|[base, extra]| {
            let count = base as f32 + extra.max(0) as f32 * visuals.scale_roll();
            count.clamp(1.0, FX_PARTICLE_CLOUD_TEMPLATE_CELLS as f32) as u32
        });
        Some(FxCloudInstance {
            particles,
            def_name: ctx.def_name.to_owned(),
            catalog_index: ctx.catalog_index,
            def_index: ctx.def_index,
            cloud: build_cloud(
                ctx.origin,
                resting_axis(elem, &ctx),
                size0,
                size1,
                scale,
                swap_red_blue(color),
                ctx.flags,
                velocity_now(elem, &ctx),
                ctx.age_msec as f32,
            ),
        })
    }

    fn light(&self, ctx: FxDrawElemContext<'_>) -> Option<FxElemLightInstance> {
        let (effect, elem) = self.find_elem(ctx.catalog_index, ctx.def_index)?;
        let visuals = Visuals::at(elem, ctx.elem_random_seed, ctx.norm_time);
        let radius = visuals.size0()?;
        if radius <= 0.0 {
            return None;
        }
        let scale = light_scale(visuals.scale()?, effect.namespace);
        if cull_elem_light(
            self.cull.elem_draw,
            self.cull.planes,
            self.plane_count(),
            ctx.flags,
            ctx.origin,
            radius,
        ) {
            return None;
        }
        let color = visuals.color_bgra()?;
        Some(FxElemLightInstance {
            def_name: ctx.def_name.to_owned(),
            def_index: ctx.def_index,
            is_spot: ctx.elem_type == FxElemType::SpotLight as u8,
            origin: ctx.origin,
            radius,
            color_bgr: elem_light_color_bgr(swap_red_blue(color), scale),
            axis: resting_axis(elem, &ctx),
        })
    }

    fn model(&self, ctx: FxDrawElemContext<'_>) -> Option<FxModelInstance> {
        let (_, elem) = self.find_elem(ctx.catalog_index, ctx.def_index)?;
        let model_index = elem.model_edge(ctx.elem_random_seed)?.bound_index()?;
        let scale = Visuals::at(elem, ctx.elem_random_seed, ctx.norm_time).scale()?;
        if scale == 0.0 {
            return None;
        }
        Some(FxModelInstance {
            def_name: ctx.def_name.to_owned(),
            def_index: ctx.def_index,
            model_index,
            elem_handle: ctx.elem_handle,
            origin: ctx.origin,
            axis: resting_axis(elem, &ctx),
            scale,
            flags: ctx.flags,
        })
    }
}

/// Build everything the live effects draw this frame, seen from `camera`. The second value
/// counts the elements that were dropped, and why.
pub fn build_fx_verts(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cull: FxDrawCull<'_>,
    camera: [f32; 3],
) -> (FxGenerateVertsOut, FxVertsGaps) {
    let drawer = Drawer {
        lookup: DrawLookup::new(catalog),
        cull,
        blockers: RefCell::default(),
    };
    let mut gaps = FxVertsGaps::default();
    let mut names = NameCache::default();

    let mut on_elem = |ctx: FxDrawElemContext<'_>| drawer.sprite(ctx, camera, &mut gaps, &mut names);
    let mut on_trail_def =
        |ctx: FxDrawTrailContext<'_>, def_index: u8| drawer.trail_def(ctx, def_index);
    let mut on_trail_sample = |ctx: FxDrawTrailSampleContext<'_>| drawer.trail_sample(ctx);
    let mut on_cloud = |ctx: FxDrawElemContext<'_>| drawer.cloud(ctx);
    let mut on_light = |ctx: FxDrawElemContext<'_>| drawer.light(ctx);
    let mut on_model = |ctx: FxDrawElemContext<'_>| drawer.model(ctx);
    let out = generate_verts_with_trails(
        host,
        &mut on_elem,
        &mut on_trail_def,
        &mut on_trail_sample,
        &mut on_cloud,
        &mut on_light,
        &mut on_model,
    );

    for blocker in drawer.blockers.take() {
        vis_blocker_add_prepared(
            &mut host.vis_blocker_read,
            blocker.flags,
            blocker.origin,
            blocker.size0,
            blocker.alpha,
            blocker.fade_in,
            blocker.fade_out,
            camera,
        );
    }
    (gaps.catalog_index_n, gaps.catalog_name_n) = drawer.lookup.counts();
    (out, gaps)
}
