//! Everything the live effects put on screen in one frame: sprites, ribbons, particle
//! clouds, lights and models.
//!
//! `assemble_effect_vertices` hands the effect walker six hooks, one per kind of output. A hook finds
//! the element's definition in the catalog, reads its curves at the element's current age,
//! and answers `None` when the element is not to be drawn.

use std::cell::RefCell;
use std::sync::Arc;

use asset_game::{FxDefinitions, SelfContainedEffectDef, SelfContainedEffectElement};
use fx::{SwarmCopy, ParticleGlowCopy, MeshParticleCopy, EffectSpriteInstance};
use fx::{PartDrawInputs, RibbonDrawInputs, RibbonSampleInputs};
use fx::{CornerBuildOutput, BakedShadeSource, EffectSystemHost};
use fx::{StreakPaintRecipe, StreakSampleLook};
use fx_t6::runtime;

use super::catalog::DrawLookup;
use super::t6::{NONUNIFORM_SCALE, light_color_bgr, light_scale, swap_red_blue};
use super::{EffectDrawHide, EffectVertexHoles};

/// Blue, green, red, alpha.

/// The three bytes of a sampled light.

/// What `assemble_effect_vertices` hands back.

/// Used for an element whose colour curve says nothing.
const FALLBACK_BGRA: [u8; 4] = [u8::MAX; 4];

/// Ticks a drop counter up without ever wrapping.
fn inc(slot: &mut u32) {
    *slot = slot.saturating_add(1);
}

/// Adds "it was missing" bookkeeping to an `Option` without changing it.
trait CountMiss<T> {
    fn count_miss(self, slot: &mut u32) -> Option<T>;
}

impl<T> CountMiss<T> for Option<T> {
    fn count_miss(self, slot: &mut u32) -> Option<T> {
        if self.is_none() {
            inc(slot);
        }
        self
    }
}

/// A size that is positive, or not a number at all (which is let through untouched).
fn drawable_size(size: f32) -> bool {
    size.is_nan() || size > 0.0
}

/// A size that has to be there and has to be drawable. `absent` counts a missing sample,
/// `unusable` one that is zero or negative.
fn required_size(sample: Option<f32>, absent: &mut u32, unusable: &mut u32) -> Option<f32> {
    let size = sample.count_miss(absent)?;
    drawable_size(size).then_some(size).count_miss(unusable)
}

/// What the lighting step does to one element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TintCase {
    /// The element does not ask for light.
    NotWanted,
    /// It asks, but the light was never sampled.
    NoSample,
    /// Lit with plain white.
    White,
    /// Lit with a colour sampled at its position.
    AtPoint,
}

impl TintCase {
    fn of(fraction: u8, origin_of_light: BakedShadeSource) -> Self {
        if !runtime::elem_uses_lighting_frac(fraction) {
            return Self::NotWanted;
        }
        match origin_of_light {
            BakedShadeSource::Missing => Self::NoSample,
            BakedShadeSource::White => Self::White,
            BakedShadeSource::AtPointRgb => Self::AtPoint,
        }
    }

    fn lights(self) -> bool {
        match self {
            Self::White | Self::AtPoint => true,
            Self::NotWanted | Self::NoSample => false,
        }
    }
}

/// An element that asks for it is tinted by the light its effect sits in, provided that
/// light was sampled at all.
pub(super) fn light_bgra(
    bgra: [u8; 4],
    fraction: u8,
    sample: [u8; 3],
    origin_of_light: BakedShadeSource,
) -> [u8; 4] {
    match TintCase::of(fraction, origin_of_light).lights() {
        true => runtime::apply_lighting_frac_bgra(bgra, sample, fraction),
        false => bgra,
    }
}

/// `light_bgra`, with the outcome also written into `tally`.
pub(super) fn light_counted(
    tally: &mut EffectVertexHoles,
    slot: u8,
    bgra: [u8; 4],
    fraction: u8,
    sample: [u8; 3],
    origin_of_light: BakedShadeSource,
) -> [u8; 4] {
    let case = TintCase::of(fraction, origin_of_light);
    if case != TintCase::NotWanted {
        inc(&mut tally.shading_blend);
    }
    match case {
        TintCase::NotWanted => {}
        TintCase::NoSample => {
            tally.light_fraction_default_slots.push(slot);
            inc(&mut tally.light_lookup_failures);
        }
        TintCase::White => {
            inc(&mut tally.lit_draw_total);
            inc(&mut tally.white_fallback_total);
        }
        TintCase::AtPoint => {
            inc(&mut tally.lit_draw_total);
            inc(&mut tally.point_sampled_lit_total);
        }
    }
    light_bgra(bgra, fraction, sample, origin_of_light)
}

/// One element's visual curves at one moment of its life.
struct CurveReader<'e> {
    samples: &'e [u8],
    step_total: u8,
    spin_start: [f32; 2],
    seed: u32,
    t: f32,
}

impl<'e> CurveReader<'e> {
    fn new(elem: &'e SelfContainedEffectElement, seed: u32, t: f32) -> Self {
        CurveReader {
            samples: &elem.visibility_readings,
            step_total: elem.view.look_keyframe_total,
            spin_start: elem.view.birth_spin_angle,
            seed,
            t,
        }
    }

    fn roll(&self, lane: u32) -> f32 {
        runtime::random_table_f32(self.seed, lane)
    }

    fn read<T>(&self, curve: fn(&[u8], u8, f32, f32) -> Option<T>, channel: u32) -> Option<T> {
        let roll = self.roll(channel);
        curve(self.samples, self.step_total, self.t, roll)
    }

    fn start_extent(&self) -> Option<f32> {
        self.read(runtime::evaluate_size0, runtime::DICE_LANE_FIRST_EXTENT)
    }

    /// The second size is drawn with the first one's roll.
    fn second_extent(&self) -> Option<f32> {
        self.read(runtime::evaluate_size1, runtime::DICE_LANE_FIRST_EXTENT)
    }

    fn scale(&self) -> Option<f32> {
        self.read(runtime::evaluate_scale, runtime::DICE_LANE_GROWTH)
    }

    /// Blue, green, red, alpha.
    fn blue_green_red_alpha(&self) -> Option<[u8; 4]> {
        self.read(runtime::evaluate_color_bgra, runtime::DICE_LANE_TINT)
    }

    fn spin(&self, particle_lifespan_ms: i32) -> Option<f32> {
        runtime::evaluate_rotation_total(
            self.samples,
            self.step_total,
            self.t,
            self.seed,
            self.spin_start,
            particle_lifespan_ms as f32,
        )
    }
}

/// How an element moves and turns, as far as drawing it cares.
struct Motion<'a, 'n> {
    elem: &'a SelfContainedEffectElement,
    ctx: &'a PartDrawInputs<'n>,
}

impl Motion<'_, '_> {
    /// Orientation after the element has spun for `msec`.
    fn axis_after(&self, msec: f32) -> [[f32; 3]; 3] {
        let view = &self.elem.view;
        runtime::get_elem_angles_axis(
            view.birth_orientation,
            view.twirl_rate,
            self.ctx.part_seed,
            msec,
            self.ctx.frame,
        )
    }

    /// Orientation of an element whose spin stops when it comes to rest.
    fn settled_axis(&self) -> [[f32; 3]; 3] {
        let PartDrawInputs {
            lived_ms,
            particle_lifespan_ms,
            settled_share: rest,
            ..
        } = *self.ctx;
        let spin_msec = runtime::clamp_elem_rotation_time(lived_ms as f32, particle_lifespan_ms as f32, rest);
        self.axis_after(spin_msec)
    }

    fn velocity(&self) -> [f32; 3] {
        let ctx = self.ctx;
        let (age, life) = (ctx.lived_ms as f32, ctx.particle_lifespan_ms.max(1) as f32);
        let graphs = (&self.elem.vel_graph_local, &self.elem.vel_graph_world);
        let drift = ctx.launch_speed_vec;
        runtime::get_velocity_at_time(
            ctx.bit_set,
            drift,
            age,
            life,
            graphs.0,
            graphs.1,
            ctx.frame,
            ctx.part_seed,
        )
    }
}

/// The kinds of sprite the sprite hook is asked to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteKind {
    FacesCamera,
    FaceAligned,
    Tail,
}

impl SpriteKind {
    fn of(elem_type: u8) -> Self {
        if elem_type == runtime::ParticleShape::Tail as u8 {
            Self::Tail
        } else if elem_type == runtime::ParticleShape::FaceAligned as u8 {
            Self::FaceAligned
        } else {
            Self::FacesCamera
        }
    }
}

/// The second size of a sprite. A tail needs a drawable one. A sprite that scales wide by
/// tall takes its height when there is a usable one, and otherwise stays square.
fn second_size(
    curves: &CurveReader<'_>,
    kind: SpriteKind,
    wide_by_tall: bool,
    tally: &mut EffectVertexHoles,
) -> Option<f32> {
    if kind == SpriteKind::Tail {
        return required_size(
            curves.second_extent(),
            &mut tally.no_second_extent_sample,
            &mut tally.second_extent_nonpositive_total,
        );
    }
    let height = curves.second_extent().filter(|h| *h > 0.0);
    Some(height.filter(|_| wide_by_tall).unwrap_or(0.0))
}

/// Alpha after distance fading. Fading all the way to nothing leaves no sprite.
fn faded_alpha(alpha: u8, fade: Option<u32>) -> Option<u8> {
    let Some(fade) = fade else {
        return Some(alpha);
    };
    let scaled = u32::from(alpha) * fade.min(255) / 255;
    u8::try_from(scaled).ok().filter(|a| *a > 0)
}

/// The number of particles in a cloud: the authored base, plus up to the authored extra
/// as the roll allows, kept within what the cloud template can hold.
fn particle_count(density: [i32; 2], roll: f32) -> u32 {
    let [base, extra] = density;
    let wanted = extra.max(0) as f32 * roll + base as f32;
    wanted.clamp(1.0, runtime::SWARM_STAMP_SPECKS as f32) as u32
}

/// Something that may block sight, noted while drawing and handed to the sight-blocker
/// buffer once the walk over the effects is finished.
struct Blocker {
    spot: [f32; 3],
    reach_r: f32,
    alpha: u8,
    part_bits: i32,
    fades: [[f32; 2]; 2],
}

impl Blocker {
    fn for_elem(elem: &SelfContainedEffectElement, spot: [f32; 3], reach_r: f32, alpha: u8) -> Self {
        let view = &elem.view;
        Blocker {
            spot,
            reach_r,
            alpha,
            part_bits: view.flags,
            fades: [view.appear_blend_reach, view.vanish_blend_reach],
        }
    }

    fn commit(self, host: &mut EffectSystemHost, eye: [f32; 3]) {
        let Blocker {
            spot,
            reach_r: radius,
            alpha,
            part_bits: elem_flags,
            fades: [ease_on, ease_off],
        } = self;
        let buffer = &mut host.vis_blocker_read;
        runtime::vis_blocker_add_prepared(
            buffer, elem_flags, spot, radius, alpha, ease_on, ease_off, eye,
        );
    }
}

/// The effect and material name strings of the sprite drawn last. Neighbouring sprites
/// mostly share both, so they share the allocations too. Lives for one pass.
#[derive(Clone)]
struct Names {
    effect: Arc<str>,
    material: Arc<str>,
}

#[derive(Default)]
struct NameMemo {
    last: Option<Names>,
}

impl NameMemo {
    fn share(&mut self, effect: &str, material: &str) -> (Arc<str>, Arc<str>) {
        let reused = self
            .last
            .as_ref()
            .filter(|n| *n.effect == *effect && *n.material == *material)
            .cloned();
        let names = reused.unwrap_or_else(|| {
            let fresh = Names {
                effect: Arc::from(effect),
                material: Arc::from(material),
            };
            self.last = Some(fresh.clone());
            fresh
        });
        (names.effect, names.material)
    }
}

/// The lighting an element asks for, bundled for the tinting calls.
#[derive(Clone, Copy)]
struct Lighting {
    fraction: u8,
    rgb: [u8; 3],
    origin: BakedShadeSource,
}

impl Lighting {
    fn of(elem: &SelfContainedEffectElement, ctx: &PartDrawInputs<'_>) -> Self {
        Lighting {
            fraction: elem.view.shading_blend,
            rgb: ctx.compact_illum,
            origin: ctx.packed_lighting_src,
        }
    }

    fn tint(self, bgra: [u8; 4]) -> [u8; 4] {
        light_bgra(bgra, self.fraction, self.rgb, self.origin)
    }

    fn tint_counted(self, tally: &mut EffectVertexHoles, slot: u8, bgra: [u8; 4]) -> [u8; 4] {
        light_counted(tally, slot, bgra, self.fraction, self.rgb, self.origin)
    }
}

/// An element together with the effect it belongs to.
struct Found<'a> {
    effect: &'a SelfContainedEffectDef,
    elem: &'a SelfContainedEffectElement,
}

/// What the six hooks share during one pass.
struct Frame<'a> {
    catalog: DrawLookup<'a>,
    cull: EffectDrawHide<'a>,
    pending: RefCell<Vec<Blocker>>,
}

impl<'a> Frame<'a> {
    fn find_part(&self, ledger_index: u16, definition_slot: u8) -> Option<Found<'a>> {
        let effect = self.catalog.look_up(ledger_index)?;
        effect
            .elems
            .get(usize::from(definition_slot))
            .map(|elem| Found { effect, elem })
    }

    fn defer(&self, blocker: Blocker) {
        self.pending.borrow_mut().push(blocker);
    }

    /// The cull switch, the planes, and how many planes there are.
    fn cull_args(&self) -> (bool, &'a [[f32; 4]], u32) {
        let EffectDrawHide { element_drawn, plane_list: planes } = self.cull;
        (element_drawn, planes, planes.len() as u32)
    }

    fn offscreen_cloud(&self, ctx: &PartDrawInputs<'_>, sizes: [f32; 2], size_k: f32) -> bool {
        let (on, planes, plane_total) = self.cull_args();
        let [start_extent, second_extent] = sizes;
        runtime::cull_cloud(
            on,
            planes,
            plane_total,
            ctx.bit_set,
            ctx.spot,
            start_extent,
            second_extent,
            size_k,
        )
    }

    fn offscreen_light(&self, ctx: &PartDrawInputs<'_>, reach_r: f32) -> bool {
        let (on, planes, plane_total) = self.cull_args();
        runtime::cull_elem_light(on, planes, plane_total, ctx.bit_set, ctx.spot, reach_r)
    }

    /// A billboard, oriented or tail sprite. Each way of being dropped is tallied in `tally`.
    fn make_sprite(
        &self,
        elem_ctx: PartDrawInputs<'_>,
        view_eye: [f32; 3],
        tally: &mut EffectVertexHoles,
        memo: &mut NameMemo,
    ) -> Option<EffectSpriteInstance> {
        let ctx = &elem_ctx;
        let effect = self
            .catalog
            .look_up(ctx.ledger_index)
            .count_miss(&mut tally.without_definition)?;
        let elem = effect
            .elems
            .get(usize::from(ctx.definition_slot))
            .count_miss(&mut tally.without_element)?;
        let (mat_slot, mat_name) = elem
            .main_surface()
            .count_miss(&mut tally.without_material_look)?;

        let kind = SpriteKind::of(ctx.part_shape);
        let curves = CurveReader::new(elem, ctx.part_seed, ctx.life_frac);
        let motion = Motion { elem, ctx };
        let lit = Lighting::of(elem, ctx);

        let start_extent = required_size(
            curves.start_extent(),
            &mut tally.no_first_extent_sample,
            &mut tally.first_extent_nonpositive_total,
        )?;
        let second_extent = second_size(&curves, kind, ctx.bit_set & NONUNIFORM_SCALE != 0, tally)?;

        // Near the eye and far away the element fades; faded to nothing it is not drawn.
        // Sight blocking uses the alpha the curve gave, before any fading.
        let [blue, green, red, curve_alpha] = curves
            .blue_green_red_alpha()
            .count_miss(&mut tally.substitute_colour_count)
            .unwrap_or(FALLBACK_BGRA);
        let [in_near, in_far] = elem.view.appear_blend_reach;
        let [out_near, out_far] = elem.view.vanish_blend_reach;
        let distance = runtime::vec3_distance(ctx.spot, view_eye);
        let fade = runtime::evaluate_distance_fade(distance, in_near, in_far, out_near, out_far);
        let alpha = faded_alpha(curve_alpha, fade)?;
        let tinted = lit.tint_counted(tally, ctx.definition_slot, [blue, green, red, alpha]);

        let axis = match kind {
            SpriteKind::FaceAligned => motion.axis_after(ctx.lived_ms as f32),
            _ => ctx.frame,
        };
        let vel_dir = (kind == SpriteKind::Tail)
            .then(|| runtime::vec3_normalize(motion.velocity()))
            .unwrap_or_default();
        let (age_ms, step) = (ctx.lived_ms, ctx.frame_index);
        let atlas = runtime::sprite_atlas_uv(
            &elem.atlas,
            ctx.part_seed,
            step,
            age_ms,
            ctx.life_frac,
        );

        self.defer(Blocker::for_elem(elem, ctx.spot, start_extent, curve_alpha));
        let (shared_def, shared_mat) = memo.share(ctx.definition_name, mat_name);
        let PartDrawInputs {
            spot: at,
            part_shape: etype,
            ledger_index: cat,
            definition_slot: slot,
            bit_set: elem_flags,
            ..
        } = *ctx;
        let sprite = EffectSpriteInstance {
            definition_name: shared_def,
            surface_title: shared_mat,
            ledger_index: cat,
            definition_slot: slot,
            shape_kind: etype,
            bit_word: elem_flags,
            where_at: at,
            turn_rows: axis,
            move_dir: vel_dir,
            start_extent,
            second_extent,
            atlas,
            rgba_tint: swap_red_blue(tinted),
            spin_rad: curves.spin(ctx.particle_lifespan_ms).unwrap_or(0.0),
            shader_at: Some(mat_slot),
            line: elem.view.line,
            shading_blend: lit.fraction,
            lit_sample: lit.rgb,
            packed_lighting_src: lit.origin,
        };
        Some(sprite)
    }

    /// The cross-section a ribbon element is stamped along, if it has one worth drawing.
    fn make_ribbon(&self, ctx: RibbonDrawInputs<'_>, definition_slot: u8) -> Option<StreakPaintRecipe> {
        let Found { elem, .. } = self.find_part(ctx.ledger_index, definition_slot)?;
        let shape = elem.trail_kind.as_ref()?;
        if shape.vertex_points.is_empty() || shape.index_run_values.len() < 2 || shape.recurrence_distance == 0 {
            return None;
        }
        let (mat_slot, mat_name) = elem.main_surface()?;
        let view = &elem.view;
        let def = StreakPaintRecipe {
            shader_slot: Some(mat_slot),
            surface_title: mat_name.to_owned(),
            vertex_points: shape.vertex_points.clone(),
            index_run_values: shape.index_run_values.clone(),
            recurrence_distance: shape.recurrence_distance,
            scroll_period_millis: shape.scroll_period_millis,
            lifespan_spread: view.lifespan_window_swing,
            lifespan_floor: view.lifespan_window_floor,
            mode_bits: view.flags,
            look_variant_total: view.look_variant_total,
        };
        Some(def)
    }

    /// How one point along a ribbon looks.
    fn make_ribbon_sample(&self, ctx: RibbonSampleInputs<'_>) -> Option<StreakSampleLook> {
        let Found { elem, .. } = self.find_part(ctx.ledger_index, ctx.definition_slot)?;
        let curves = CurveReader::new(elem, ctx.seed_bits, ctx.age_frac);
        let start_extent = curves.start_extent().filter(|s| drawable_size(*s))?;
        let color = curves.blue_green_red_alpha().unwrap_or(FALLBACK_BGRA);

        self.defer(Blocker::for_elem(elem, ctx.sample_spot, start_extent, color[3]));
        let visual = StreakSampleLook {
            spin_now: curves.spin(ctx.particle_lifespan_ms).unwrap_or(0.0),
            tint_rgba: swap_red_blue(color),
            size: [start_extent, curves.second_extent().unwrap_or(start_extent)],
        };
        Some(visual)
    }

    fn make_cloud(&self, elem_ctx: PartDrawInputs<'_>) -> Option<SwarmCopy> {
        let ctx = &elem_ctx;
        let Found { elem, .. } = self.find_part(ctx.ledger_index, ctx.definition_slot)?;
        let curves = CurveReader::new(elem, ctx.part_seed, ctx.life_frac);
        let start_extent = curves.start_extent()?;
        let scale = curves.scale()?;
        let second_extent = curves.second_extent().unwrap_or(start_extent);
        if self.offscreen_cloud(ctx, [start_extent, second_extent], scale) {
            return None;
        }

        let tinted = Lighting::of(elem, ctx).tint(curves.blue_green_red_alpha().unwrap_or(FALLBACK_BGRA));
        let motion = Motion { elem, ctx };
        let (cat, slot, name) = (ctx.ledger_index, ctx.definition_slot, ctx.definition_name);
        // Clouds that do not name a particle count keep the template's own.
        let scale_roll = curves.roll(runtime::DICE_LANE_GROWTH);
        let cloud = SwarmCopy {
            cloud: runtime::build_cloud(
                ctx.spot,
                motion.settled_axis(),
                start_extent,
                second_extent,
                scale,
                swap_red_blue(tinted),
                ctx.bit_set,
                motion.velocity(),
                ctx.lived_ms as f32,
            ),
            particles: elem.cloud_density.map(|d| particle_count(d, scale_roll)),
            ledger_index: cat,
            definition_slot: slot,
            definition_name: name.to_owned(),
        };
        Some(cloud)
    }

    fn make_light(&self, elem_ctx: PartDrawInputs<'_>) -> Option<ParticleGlowCopy> {
        let ctx = &elem_ctx;
        let Found { effect, elem } = self.find_part(ctx.ledger_index, ctx.definition_slot)?;
        let curves = CurveReader::new(elem, ctx.part_seed, ctx.life_frac);
        let radius = curves.start_extent().filter(|r| drawable_size(*r))?;
        let strength = light_scale(curves.scale()?, effect.namespace);
        if self.offscreen_light(ctx, radius) {
            return None;
        }
        let color = curves.blue_green_red_alpha()?;
        let motion = Motion { elem, ctx };
        let PartDrawInputs {
            spot: at,
            definition_slot: slot,
            definition_name: name,
            ..
        } = *ctx;
        let light = ParticleGlowCopy {
            frame_rows: motion.settled_axis(),
            tint_blue_first: light_color_bgr(swap_red_blue(color), strength, effect.namespace),
            is_cone: ctx.part_shape == runtime::ParticleShape::SpotLight as u8,
            anchor: at,
            rad: radius,
            definition_slot: slot,
            definition_name: name.to_owned(),
        };
        Some(light)
    }

    fn make_model(&self, elem_ctx: PartDrawInputs<'_>) -> Option<MeshParticleCopy> {
        let ctx = &elem_ctx;
        let Found { elem, .. } = self.find_part(ctx.ledger_index, ctx.definition_slot)?;
        let seed = ctx.part_seed;
        let edge = elem.model_link(seed)?;
        let mesh_slot_number = edge.extent_position()?;
        let scale = CurveReader::new(elem, seed, ctx.life_frac)
            .scale()
            .filter(|s| *s != 0.0)?;
        let motion = Motion { elem, ctx };
        let PartDrawInputs {
            part_no: handle,
            bit_set: elem_flags,
            spot: at,
            definition_slot: slot,
            definition_name: name,
            ..
        } = *ctx;
        let model = MeshParticleCopy {
            definition_name: name.to_owned(),
            definition_slot: slot,
            mesh_slot_number,
            size_k: scale,
            place_at: at,
            copy_bits: elem_flags,
            part_id: handle,
            axes_m: motion.settled_axis(),
        };
        Some(model)
    }
}

/// Everything the live effects put on screen this frame, seen from `camera`, plus a count
/// of the elements that were dropped and why.
pub fn assemble_effect_vertices(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    culling: EffectDrawHide<'_>,
    eye: [f32; 3],
) -> (CornerBuildOutput, EffectVertexHoles) {
    let frame = Frame {
        catalog: DrawLookup::new(catalog),
        cull: culling,
        pending: RefCell::default(),
    };
    let mut drops = EffectVertexHoles::default();
    let mut memo = NameMemo::default();

    let harvest = {
        let mut sprite_hook =
            |ctx: PartDrawInputs<'_>| frame.make_sprite(ctx, eye, &mut drops, &mut memo);
        let mut ribbon_hook = |ctx: RibbonDrawInputs<'_>, slot: u8| frame.make_ribbon(ctx, slot);
        let mut sample_hook = |ctx: RibbonSampleInputs<'_>| frame.make_ribbon_sample(ctx);
        let mut cloud_hook = |ctx: PartDrawInputs<'_>| frame.make_cloud(ctx);
        let mut light_hook = |ctx: PartDrawInputs<'_>| frame.make_light(ctx);
        let mut model_hook = |ctx: PartDrawInputs<'_>| frame.make_model(ctx);
        fx::build_corners_and_streaks(
            host,
            &mut sprite_hook,
            &mut ribbon_hook,
            &mut sample_hook,
            &mut cloud_hook,
            &mut light_hook,
            &mut model_hook,
        )
    };

    for blocker in frame.pending.take() {
        blocker.commit(host, eye);
    }
    (drops.catalog_index_total, drops.catalog_name_total) = frame.catalog.counts();
    (harvest, drops)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_CURVES: CurveReader<'static> = CurveReader {
        samples: &[],
        step_total: 0,
        spin_start: [0.0; 2],
        seed: 7,
        t: 0.5,
    };

    #[test]
    fn nan_sizes_pass_but_flat_and_negative_ones_do_not() {
        assert!(drawable_size(0.25));
        assert!(drawable_size(f32::NAN));
        assert!(!drawable_size(0.0));
        assert!(!drawable_size(-0.0));
        assert!(!drawable_size(-3.0));
    }

    #[test]
    fn required_size_counts_absent_and_unusable_apart() {
        let (mut absent, mut unusable) = (0, 0);
        let fine = required_size(Some(2.0), &mut absent, &mut unusable);
        assert_eq!(fine, Some(2.0));
        assert_eq!(required_size(None, &mut absent, &mut unusable), None);
        assert_eq!(required_size(Some(0.0), &mut absent, &mut unusable), None);
        assert_eq!(required_size(Some(-1.0), &mut absent, &mut unusable), None);
        assert_eq!((absent, unusable), (1, 2));
    }

    #[test]
    fn counters_stop_at_the_top() {
        let mut slot = u32::MAX;
        inc(&mut slot);
        assert_eq!(slot, u32::MAX);
        assert_eq!(None::<u8>.count_miss(&mut slot), None);
        let mut small = 0;
        assert_eq!(Some(4).count_miss(&mut small), Some(4));
        assert_eq!(small, 0);
    }

    #[test]
    fn distance_fade_scales_alpha_and_can_remove_it() {
        assert_eq!(faded_alpha(200, None), Some(200));
        assert_eq!(faded_alpha(200, Some(255)), Some(200));
        assert_eq!(faded_alpha(200, Some(1000)), Some(200));
        assert_eq!(faded_alpha(200, Some(128)), Some(100));
        assert_eq!(faded_alpha(200, Some(0)), None);
        assert_eq!(faded_alpha(1, Some(100)), None);
        assert_eq!(faded_alpha(0, None), Some(0));
    }

    #[test]
    fn particle_counts_stay_inside_the_template() {
        let cells = runtime::SWARM_STAMP_SPECKS as u32;
        assert_eq!(particle_count([10, 0], 0.9), 10);
        assert_eq!(particle_count([10, 20], 0.5), 20);
        assert_eq!(particle_count([10, -5], 1.0), 10);
        assert_eq!(particle_count([0, 0], 0.0), 1);
        assert_eq!(particle_count([-4, 0], 0.0), 1);
        assert_eq!(particle_count([i32::MAX, 0], 0.0), cells);
    }

    #[test]
    fn sprite_kind_follows_the_element_type() {
        use runtime::ParticleShape;
        let expected = [
            (ParticleShape::Tail, SpriteKind::Tail),
            (ParticleShape::FaceAligned, SpriteKind::FaceAligned),
            (ParticleShape::FacesCamera, SpriteKind::FacesCamera),
            (ParticleShape::Cloud, SpriteKind::FacesCamera),
        ];
        for (ty, kind) in expected {
            assert_eq!(SpriteKind::of(ty as u8), kind);
        }
    }

    #[test]
    fn tint_cases_cover_every_source() {
        assert_eq!(TintCase::of(0, BakedShadeSource::Missing), TintCase::NotWanted);
        assert_eq!(TintCase::of(0, BakedShadeSource::White), TintCase::NotWanted);
        assert_eq!(TintCase::of(90, BakedShadeSource::Missing), TintCase::NoSample);
        assert_eq!(TintCase::of(90, BakedShadeSource::White), TintCase::White);
        assert_eq!(TintCase::of(90, BakedShadeSource::AtPointRgb), TintCase::AtPoint);
        assert!(TintCase::AtPoint.lights() && TintCase::White.lights());
        assert!(!TintCase::NoSample.lights() && !TintCase::NotWanted.lights());
    }

    #[test]
    fn untinted_or_unsampled_light_leaves_the_colour_alone() {
        let color = [9, 99, 199, 255];
        let light = [250, 20, 120];
        let tinted = runtime::apply_lighting_frac_bgra(color, light, 200);
        assert_ne!(tinted, color);
        assert_eq!(light_bgra(color, 200, light, BakedShadeSource::White), tinted);
        assert_eq!(light_bgra(color, 0, light, BakedShadeSource::White), color);
        assert_eq!(light_bgra(color, 200, light, BakedShadeSource::Missing), color);

        let mut tally = EffectVertexHoles::default();
        let out = light_counted(&mut tally, 4, color, 200, light, BakedShadeSource::Missing);
        assert_eq!(out, color);
        let out = light_counted(&mut tally, 5, color, 200, light, BakedShadeSource::AtPointRgb);
        assert_eq!(out, tinted);
        assert_eq!(tally.light_fraction_default_slots, vec![4]);
        assert_eq!((tally.light_lookup_failures, tally.lit_draw_total), (1, 1));
        assert_eq!((tally.white_fallback_total, tally.point_sampled_lit_total), (0, 1));
    }

    #[test]
    fn name_memo_shares_until_either_name_changes() {
        let mut memo = NameMemo::default();
        let (e1, m1) = memo.share("fx/spark", "mtl_spark");
        let (e2, m2) = memo.share("fx/spark", "mtl_spark");
        assert!(Arc::ptr_eq(&e1, &e2) && Arc::ptr_eq(&m1, &m2));

        let (e3, m3) = memo.share("fx/spark", "mtl_smoke");
        assert!(!Arc::ptr_eq(&m1, &m3));
        assert_eq!((&*e3, &*m3), ("fx/spark", "mtl_smoke"));
        let (e4, _) = memo.share("fx/smoke", "mtl_smoke");
        assert_eq!(&*e4, "fx/smoke");
        assert!(!Arc::ptr_eq(&e3, &e4));
    }

    #[test]
    fn empty_curves_read_as_nothing() {
        assert_eq!(NO_CURVES.start_extent(), None);
        assert_eq!(NO_CURVES.second_extent(), None);
        assert_eq!(NO_CURVES.scale(), None);
        assert_eq!(NO_CURVES.blue_green_red_alpha(), None);
        assert_eq!(NO_CURVES.spin(500), None);
        assert_eq!(NO_CURVES.roll(3), runtime::random_table_f32(7, 3));
    }

    #[test]
    fn second_size_by_sprite_kind() {
        let mut seen = EffectVertexHoles::default();
        let flat_kinds = [
            (SpriteKind::FacesCamera, false),
            (SpriteKind::FacesCamera, true),
            (SpriteKind::FaceAligned, true),
        ];
        for (kind, wide) in flat_kinds {
            assert_eq!(second_size(&NO_CURVES, kind, wide, &mut seen), Some(0.0));
        }
        assert_eq!(seen.no_second_extent_sample, 0);

        let tail = second_size(&NO_CURVES, SpriteKind::Tail, false, &mut seen);
        assert_eq!(tail, None);
        assert_eq!((seen.no_second_extent_sample, seen.second_extent_nonpositive_total), (1, 0));
    }
}
