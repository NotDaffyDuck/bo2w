//! Live effects in, drawable things out: sprites, lights, models, particle
//! clouds and ribbon meshes. The caller decides what a part looks like
//! (material, size, colour); this module walks the pools, drops what cannot
//! be seen, and asks the caller to dress whatever is left.

use std::iter::successors;
use std::sync::Arc;

use fx_t6::runtime::{
    DICE_LANE_LIFESPAN, ParticleShape, BillboardSheetCell, StreakLaidCorner, StreakPiecePaintStatus,
    StreakCorner, VisualParticleCloud,
};
use fx_t6::runtime::{
    elem_norm_time, elem_random_seed, random_table_u16, sample_life_span_msec, trail_compute_u,
    trail_emit_index_quad, trail_emit_segment_verts, trail_uncompress_basis, vec3_length_sq,
    vec3_normalize,
};

use crate::particle_bin::{NO_PARTICLE_TICKET, ParticleBinSlot, particle_bin_of_ticket};
use crate::facing::elem_world_origin;
use crate::host::{LiveEffect, BakedShadeSource, EffectSystemHost};
use crate::ribbon::{NO_STREAK_TICKET, RibbonPointCell, live_sample, live_trail};

const ZERO: u32 = 0;


/// Everything known about one live part at the moment it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct PartDrawInputs<'n> {
    pub definition_name: &'n str,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub part_shape: u8,
    pub spot: [f32; 3],
    pub frame: [[f32; 3]; 3],
    pub life_frac: f32,
    pub lived_ms: i32,
    pub particle_lifespan_ms: i32,
    pub part_seed: u32,
    pub frame_index: u8,
    pub launch_speed_vec: [f32; 3],
    pub bit_set: i32,
    pub settled_share: u8,
    pub compact_illum: [u8; 3],
    pub packed_lighting_src: BakedShadeSource,
    pub part_no: u16,
}

/// Which ribbon is being drawn.
#[derive(Clone, Copy, Debug)]
pub struct RibbonDrawInputs<'n> {
    pub definition_name: &'n str,
    pub ledger_index: u16,
    pub definition_slot: u8,
}

/// One sample along a ribbon, as the caller is asked to dress it.
#[derive(Clone, Copy, Debug)]
pub struct RibbonSampleInputs<'n> {
    pub definition_name: &'n str,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub sample_frame_index: u8,
    pub sample_spot: [f32; 3],
    pub emit_spacing: f32,
    pub window_open_ms: i32,
    pub age_frac: f32,
    pub lived_ms: i32,
    pub particle_lifespan_ms: i32,
    pub seed_bits: u32,
}

/// The authored shape of a ribbon: its cross-section and how it tiles.
#[derive(Clone, Debug)]
pub struct StreakPaintRecipe {
    pub look_variant_total: u8,
    pub mode_bits: i32,
    pub lifespan_floor: i32,
    pub lifespan_spread: i32,
    pub scroll_period_millis: i32,
    pub recurrence_distance: i32,
    pub vertex_points: Vec<StreakCorner>,
    pub index_run_values: Vec<u16>,
    pub surface_title: String,
    pub shader_slot: Option<usize>,
}

/// How one ribbon sample looks right now.
#[derive(Clone, Copy, Debug)]
pub struct StreakSampleLook {
    pub size: [f32; 2],
    pub tint_rgba: [u8; 4],
    pub spin_now: f32,
}

#[derive(Clone, Debug)]
pub struct EffectSpriteInstance {
    pub where_at: [f32; 3],
    pub start_extent: f32,
    pub second_extent: f32,
    pub rgba_tint: [u8; 4],
    pub shape_kind: u8,
    pub definition_name: Arc<str>,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub surface_title: Arc<str>,
    pub shader_at: Option<usize>,
    pub turn_rows: [[f32; 3]; 3],
    pub spin_rad: f32,
    pub move_dir: [f32; 3],
    pub atlas: BillboardSheetCell,
    pub bit_word: i32,
    /// A BO2 line elem: a tail that runs ahead of its part, along its
    /// velocity: the zombies' eye beams (they start behind the eye and move
    /// out of the face), the fires' flame sheets (they start low and rise).
    pub line: bool,
    pub lit_sample: [u8; 3],
    pub shading_blend: u8,
    pub packed_lighting_src: BakedShadeSource,
}

#[derive(Clone, Debug, Default)]
pub struct StreakMeshCopy {
    pub definition_name: String,
    pub definition_slot: u8,
    pub surface_title: String,
    pub shader_slot_ix: Option<usize>,
    pub vertex_points: Vec<StreakLaidCorner>,
    pub pair_indices: Vec<[u16; 2]>,
}

/// What one pass over the pools produced, plus a count of every part that was
/// passed over and why.
#[derive(Clone, Debug, Default)]
pub struct CornerBuildOutput {
    pub sprites: Vec<EffectSpriteInstance>,
    pub trail_strips: Vec<StreakMeshCopy>,
    pub dormant_skips: u32,
    pub no_visual_skips: u32,
    pub no_handler_skips: u32,
    pub unsupported_kind_skips: u32,
    pub cloud_skips: u32,
    pub model_skips: u32,
    pub omni_light_skips: u32,
    pub skipped_cone_light: u32,
    pub other_kind_skips: u32,
    pub lookup_misses: u32,
    pub stopped_skips: u32,
    pub no_ribbon_def_skips: u32,
    pub clouds: Vec<SwarmCopy>,
    pub omni_lights: Vec<ParticleGlowCopy>,
    pub spot_lights: Vec<ParticleGlowCopy>,
    pub models: Vec<MeshParticleCopy>,
}

#[derive(Clone, Debug)]
pub struct SwarmCopy {
    pub definition_name: String,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub cloud: VisualParticleCloud,
    /// bo2zm: a Black Ops II cloud's particle count (legacy clouds: none, the
    /// shape bits in `cloud.flags` decide).
    pub particles: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ParticleGlowCopy {
    pub definition_name: String,
    pub definition_slot: u8,
    pub is_cone: bool,
    pub anchor: [f32; 3],
    pub rad: f32,
    pub tint_blue_first: [f32; 3],
    pub frame_rows: [[f32; 3]; 3],
}

#[derive(Clone, Debug)]
pub struct MeshParticleCopy {
    pub definition_name: String,
    pub definition_slot: u8,
    pub mesh_slot_number: usize,
    pub part_id: u16,
    pub place_at: [f32; 3],
    pub axes_m: [[f32; 3]; 3],
    pub size_k: f32,
    pub copy_bits: i32,
}

/// Effect status bit: stopped, nothing of it is drawn any more.
const STATUS_NOT_DRAWN: u32 = 0x4000;

/// Ribbon flag: orient the cross-section from the direction of travel rather
/// than from the stored basis.
const TRAIL_FACES_TRAVEL: i32 = 0x4000;

/// Position of the alpha byte in a colour.
const ALPHA: usize = 3;

fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

/// The caller's hooks. A hook left out answers `None` every time.
#[derive(Default)]
struct Callbacks<'f> {
    trail_def: Option<&'f mut dyn for<'c> FnMut(RibbonDrawInputs<'c>, u8) -> Option<StreakPaintRecipe>>,
    trail_sample: Option<&'f mut dyn for<'c> FnMut(RibbonSampleInputs<'c>) -> Option<StreakSampleLook>>,
    model: Option<&'f mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<MeshParticleCopy>>,
    light: Option<&'f mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<ParticleGlowCopy>>,
    cloud: Option<&'f mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<SwarmCopy>>,
    sprite: Option<&'f mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<EffectSpriteInstance>>,
}

/// Why a part is not drawn before anyone is asked about it.
#[derive(Clone, Copy)]
enum Skip {
    Invisible,
    Dormant,
    UnknownType,
    NullHandler,
}

/// The part's kind when it is alive and has something to draw.
fn admit(now: i32, elem: &ParticleBinSlot) -> Result<ParticleShape, Skip> {
    if elem.look_variant_total < 1 {
        return Err(Skip::Invisible);
    }
    let last_msec = elem.window_open_ms.wrapping_add(elem.lifespan_window_ms);
    if !(elem.window_open_ms..last_msec).contains(&now) {
        return Err(Skip::Dormant);
    }
    let kind = ParticleShape::from_u8(elem.part_kind).ok_or(Skip::UnknownType)?;
    match kind.particle_painter_absent() {
        true => Err(Skip::NullHandler),
        false => Ok(kind),
    }
}

/// An effect that sits in the ring.
fn resident(host: &EffectSystemHost, slot: usize) -> Option<&LiveEffect> {
    host.emitter_in_slot(slot).filter(|fx| fx.in_ring)
}

/// The part after `handle` in its list, if there is one.
fn next_part(host: &EffectSystemHost, handle: u16) -> Option<u16> {
    let at = particle_bin_of_ticket(handle)?;
    let following = host.elems.get(at)?.following_particle_ticket;
    Some(following).filter(|&h| h != NO_PARTICLE_TICKET)
}

/// The sample behind `handle`, with its handle, when it is in use.
fn sample_at(host: &EffectSystemHost, handle: u16) -> Option<(u16, &RibbonPointCell)> {
    live_sample(host, handle).map(|slot| (handle, &host.trail_elems[slot]))
}

fn elem_context<'n>(
    now: i32,
    fx: &'n LiveEffect,
    part: &ParticleBinSlot,
    part_ticket: u16,
) -> PartDrawInputs<'n> {
    let lived_ms = now.wrapping_sub(part.window_open_ms);
    let particle_lifespan_ms = part.lifespan_window_ms;
    let rng_seed = elem_random_seed(fx.dice_start, part.nth, part.window_open_ms);
    let spawn = part.spawn_facing(rng_seed);
    let origin = elem_world_origin(
        part.at_point,
        part.bits,
        &fx.current_frame(),
        &fx.spawn_frame(),
        Some(spawn),
    );
    PartDrawInputs {
        definition_name: &fx.definition_name,
        ledger_index: fx.ledger_index,
        definition_slot: part.definition_slot,
        part_shape: part.part_kind,
        spot: origin,
        frame: fx.orient_rows,
        life_frac: elem_norm_time(lived_ms, particle_lifespan_ms),
        lived_ms,
        particle_lifespan_ms,
        part_seed: rng_seed,
        frame_index: part.nth,
        launch_speed_vec: part.launch_speed_vec,
        bit_set: part.bits,
        settled_share: part.settled_share,
        compact_illum: fx.compact_illum,
        packed_lighting_src: fx.packed_lighting_src,
        part_no: part_ticket,
    }
}

/// The identity of one ribbon being meshed.
struct Ribbon<'n> {
    name: &'n str,
    catalog: u16,
    def: u8,
    seed: u16,
    first: u16,
    last: u16,
}

/// One walk over the pools: reads the host, asks the hooks, fills `out`.
struct Pass<'h, 'f> {
    host: &'h EffectSystemHost,
    hooks: Callbacks<'f>,
    out: CornerBuildOutput,
}

impl<'h, 'f> Pass<'h, 'f> {
    fn new(host: &'h EffectSystemHost, hooks: Callbacks<'f>) -> Self {
        Self {
            host,
            hooks,
            out: CornerBuildOutput::default(),
        }
    }

    /// Every effect in the active window of the ring, parts before ribbons.
    fn run(mut self) -> CornerBuildOutput {
        let host = self.host;
        let window_start = host.first_active_effect as u32;
        let window_len = (host.first_new_effect as u32).wrapping_sub(window_start);
        for step in 0..window_len {
            let handle = host.ring_entry_handle(window_start.wrapping_add(step));
            let Some(slot) = host.ring_index_of(handle) else {
                continue;
            };
            self.parts_of(slot);
            self.trails_of(slot);
        }
        self.out
    }

    /// All three part lists of one effect, in order.
    fn parts_of(&mut self, at: usize) {
        let host = self.host;
        let Some(effect) = resident(host, at) else {
            return;
        };
        if effect.state_bits & STATUS_NOT_DRAWN != 0 {
            bump(&mut self.out.stopped_skips);
            return;
        }
        for head in effect.lead_particle_ticket.into_iter().take(3) {
            let first = Some(head).filter(|&h| h != NO_PARTICLE_TICKET);
            for handle in successors(first, |&h| next_part(host, h)) {
                self.one_part(effect, handle);
            }
        }
    }

    fn one_part(&mut self, effect: &LiveEffect, fx_ref: u16) {
        let host = self.host;
        let live = particle_bin_of_ticket(fx_ref)
            .and_then(|slot| host.elems.get(slot))
            .filter(|elem| elem.is_taken);
        let Some(elem) = live else {
            return;
        };
        let kind = match admit(host.clock_ms_now, elem) {
            Ok(k) => k,
            Err(why) => return self.count_skip(why),
        };

        let ctx = elem_context(host.clock_ms_now, effect, elem, fx_ref);
        match kind {
            ParticleShape::FacesCamera | ParticleShape::FaceAligned | ParticleShape::Tail => self.sprite(ctx),
            ParticleShape::Cloud => self.cloud(ctx),
            ParticleShape::Model => self.model(ctx),
            ParticleShape::RoundGlow => self.light(ctx, false),
            ParticleShape::SpotLight => self.light(ctx, true),
            _ => {
                bump(&mut self.out.unsupported_kind_skips);
                bump(&mut self.out.other_kind_skips);
            }
        }
    }

    fn count_skip(&mut self, why: Skip) {
        let out = &mut self.out;
        match why {
            Skip::Invisible => bump(&mut out.no_visual_skips),
            Skip::Dormant => bump(&mut out.dormant_skips),
            Skip::NullHandler => bump(&mut out.no_handler_skips),
            Skip::UnknownType => {
                bump(&mut out.unsupported_kind_skips);
                bump(&mut out.other_kind_skips);
            }
        }
    }

    fn sprite(&mut self, ctx: PartDrawInputs<'_>) {
        let made = self.hooks.sprite.as_mut().and_then(|dress| dress(ctx));
        match made {
            Some(found) => self.out.sprites.push(found),
            None => bump(&mut self.out.lookup_misses),
        }
    }

    fn cloud(&mut self, ctx: PartDrawInputs<'_>) {
        let made = self.hooks.cloud.as_mut().and_then(|dress| dress(ctx));
        let visible = |c: &SwarmCopy| c.cloud.layout_size_ratio != 0.0 && c.cloud.start_extent != 0.0;
        match made.filter(visible) {
            Some(found) => self.out.clouds.push(found),
            None => bump(&mut self.out.cloud_skips),
        }
    }

    fn model(&mut self, ctx: PartDrawInputs<'_>) {
        bump(&mut self.out.unsupported_kind_skips);
        let made = self.hooks.model.as_mut().and_then(|dress| dress(ctx));
        match made.filter(|m| m.size_k != 0.0) {
            Some(found) => self.out.models.push(found),
            None => bump(&mut self.out.model_skips),
        }
    }

    /// `spot_kind` is what the part's type says; the instance says which list
    /// it goes to.
    fn light(&mut self, draw: PartDrawInputs<'_>, spot_kind: bool) {
        bump(&mut self.out.unsupported_kind_skips);
        let out = &mut self.out;
        let Some(found) = self.hooks.light.as_mut().and_then(|dress| dress(draw)) else {
            let missed = match spot_kind {
                true => &mut out.skipped_cone_light,
                false => &mut out.omni_light_skips,
            };
            return bump(missed);
        };
        let shelf = match found.is_cone {
            true => &mut out.spot_lights,
            false => &mut out.omni_lights,
        };
        shelf.push(found);
    }

    /// Every ribbon of one effect.
    fn trails_of(&mut self, at: usize) {
        let host = self.host;
        let Some(effect) = resident(host, at) else {
            return;
        };
        if effect.state_bits & STATUS_NOT_DRAWN != 0 {
            return;
        }

        let mut cursor = effect.first_ribbon_ticket;
        while let Some(trail_slot) = live_trail(host, cursor) {
            let trail = &host.trails[trail_slot];
            let def = trail.definition_slot as u8;
            let probe = RibbonDrawInputs {
                definition_name: &effect.definition_name,
                ledger_index: effect.ledger_index,
                definition_slot: def,
            };
            let shape = self.hooks.trail_def.as_mut().and_then(|ask| ask(probe, def));
            match shape {
                None => bump(&mut self.out.no_ribbon_def_skips),
                Some(shape) => {
                    let ribbon = Ribbon {
                        name: &effect.definition_name,
                        catalog: effect.ledger_index,
                        def,
                        seed: effect.dice_start,
                        first: trail.lead_particle_ticket,
                        last: trail.last_elem_handle,
                    };
                    self.mesh(&ribbon, &shape);
                }
            }
            cursor = trail.next_trail_handle;
        }
    }

    /// Turns the due samples of one ribbon into a strip of cross-sections
    /// stitched together.
    fn mesh(&mut self, rb: &Ribbon<'_>, shape: &StreakPaintRecipe) {
        if shape.look_variant_total == 0 {
            return bump(&mut self.out.no_visual_skips);
        }
        if rb.first == NO_STREAK_TICKET {
            return;
        }
        let sane = shape.recurrence_distance != 0
            && !shape.vertex_points.is_empty()
            && shape.index_run_values.len() >= 2
            && shape.index_run_values.len() % 2 == 0;
        if !sane {
            return bump(&mut self.out.no_ribbon_def_skips);
        }

        let host = self.host;
        let now = host.clock_ms_now;
        let due: Vec<(u16, &RibbonPointCell)> = successors(sample_at(host, rb.first), |&(_, s)| {
            sample_at(host, s.next_trail_elem_handle)
        })
        .filter(|(_, sample)| sample.laid_by(now))
        .collect();
        let Some(&(_, lead)) = due.first() else {
            return;
        };

        let scroll = trail_compute_u(lead.spawn_dist, shape.recurrence_distance, shape.scroll_period_millis, now);
        let scroll = scroll as f32;
        let mut strip: Vec<StreakLaidCorner> = Vec::new();
        let mut prev: Option<(StreakPiecePaintStatus, f32)> = None;
        let mut unlooked = false;

        for (ticket, sample) in due {
            let rseed = sample_seed(rb.seed, sample.sequence);
            let particle_lifespan_ms = sample_life_span_msec(
                shape.lifespan_floor,
                shape.lifespan_spread,
                random_table_u16(rseed, DICE_LANE_LIFESPAN),
            );
            let lived_ms = now.wrapping_sub(sample.window_open_ms);
            let norm_time = elem_norm_time(lived_ms, particle_lifespan_ms);

            let ask = RibbonSampleInputs {
                definition_name: rb.name,
                ledger_index: rb.catalog,
                definition_slot: rb.def,
                sample_frame_index: sample.sequence,
                sample_spot: sample.spot_xyz,
                emit_spacing: sample.spawn_dist,
                window_open_ms: sample.window_open_ms,
                age_frac: norm_time,
                lived_ms,
                particle_lifespan_ms,
                seed_bits: rseed,
            };
            let Some(look) = self.hooks.trail_sample.as_mut().and_then(|dress| dress(ask)) else {
                unlooked = true;
                continue;
            };

            let is_first = ticket == rb.first;
            let turned = prev
                .filter(|_| shape.mode_bits & TRAIL_FACES_TRAVEL != 0 && !is_first)
                .and_then(|(before, _)| basis_along_travel(before.scene_point, sample.spot_xyz));
            let mut seg = StreakPiecePaintStatus {
                scene_point: sample.spot_xyz,
                basis: turned.unwrap_or_else(|| trail_uncompress_basis(&sample.basis)),
                rotation: look.spin_now,
                size: look.size,
                across_texel: sample.spawn_dist / shape.recurrence_distance as f32 + scroll,
                tint: look.tint_rgba,
            };

            // A sample that has not expired makes a segment. The first
            // sample of a fresh ribbon fades in, the last one fades out, and
            // the one that crosses the expiry edge brings a blended segment
            // at the edge.
            let fresh = norm_time < 1.0;
            if fresh {
                let mut transparent = false;
                match (prev, is_first) {
                    (_, true) => transparent = sample.sequence == 0,
                    (Some((before, before_norm)), false) => {
                        if before_norm >= 1.0 {
                            let at = (1.0 - before_norm) / (norm_time - before_norm);
                            push_segment(&shape.vertex_points, &blend(&before, &seg, at), &mut strip);
                        }
                        transparent = ticket == rb.last;
                    }
                    (None, false) => {}
                }
                if transparent {
                    seg.tint[ALPHA] = 0;
                }
                push_segment(&shape.vertex_points, &seg, &mut strip);
            }
            prev = Some((seg, norm_time));
        }

        if unlooked {
            bump(&mut self.out.lookup_misses);
        }
        if strip.is_empty() {
            return;
        }

        let per_section = shape.vertex_points.len();
        let sections = strip.len() / per_section;
        let stitched = (0..sections.saturating_sub(1)).flat_map(|n| {
            let base = (n as u16).wrapping_mul(per_section as u16);
            shape.index_run_values.chunks_exact(2).flat_map(move |pair| {
                trail_emit_index_quad(pair[0], pair[1], base, per_section as u16)
            })
        });
        self.out.trail_strips.push(StreakMeshCopy {
            definition_name: rb.name.to_owned(),
            definition_slot: rb.def,
            surface_title: shape.surface_title.clone(),
            shader_slot_ix: shape.shader_slot,
            vertex_points: strip,
            pair_indices: stitched.collect(),
        });
    }
}

/// Sprites only: the other kinds are skipped.
pub fn build_corners(
    host: &EffectSystemHost,
    mut dress_sprite: impl FnMut(PartDrawInputs<'_>) -> Option<EffectSpriteInstance>,
) -> CornerBuildOutput {
    let hooks = Callbacks {
        sprite: Some(&mut dress_sprite),
        ..Callbacks::default()
    };
    Pass::new(host, hooks).run()
}

/// Walks every effect in the active window of the ring, parts first and then
/// ribbons, and collects what the callbacks make of them.
pub fn build_corners_and_streaks(
    host: &EffectSystemHost,
    dress_part: &mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<EffectSpriteInstance>,
    ribbon_def: &mut dyn for<'c> FnMut(RibbonDrawInputs<'c>, u8) -> Option<StreakPaintRecipe>,
    dress_sample: &mut dyn for<'c> FnMut(RibbonSampleInputs<'c>) -> Option<StreakSampleLook>,
    dress_cloud: &mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<SwarmCopy>,
    dress_light: &mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<ParticleGlowCopy>,
    dress_model: &mut dyn for<'c> FnMut(PartDrawInputs<'c>) -> Option<MeshParticleCopy>,
) -> CornerBuildOutput {
    let hooks = Callbacks {
        trail_def: Some(ribbon_def),
        trail_sample: Some(dress_sample),
        model: Some(dress_model),
        light: Some(dress_light),
        cloud: Some(dress_cloud),
        sprite: Some(dress_part),
    };
    Pass::new(host, hooks).run()
}

/// The random seed one ribbon sample draws its life span from.
fn sample_seed(emitter_dice_start: u16, sequence: u8) -> u32 {
    u32::from(sequence)
        .wrapping_mul(0x128)
        .wrapping_add(u32::from(emitter_dice_start))
        % 0x1df
}

fn sub3(to: [f32; 3], from: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| to[i] - from[i])
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A cross-section whose width runs sideways of the line from `from` to `toward`
/// and whose height is perpendicular to both. `None` when the line is
/// vertical (no sideways direction exists).
fn basis_along_travel(from: [f32; 3], toward: [f32; 3]) -> Option<[[f32; 3]; 2]> {
    let heading = vec3_normalize(sub3(toward, from));
    let level = [heading[1], -heading[0], 0.0];
    if vec3_length_sq(level) <= 0.0 {
        return None;
    }
    let width = vec3_normalize(level);
    Some([width, vec3_normalize(cross3(heading, width))])
}

fn lerp(from: f32, to: f32, share: f32) -> f32 {
    from + share * (to - from)
}

fn lerp_each<const N: usize>(from: [f32; N], to: [f32; N], share: f32) -> [f32; N] {
    std::array::from_fn(|i| lerp(from[i], to[i], share))
}

/// The state a fraction `t` of the way from `a` to `b`; colour stays `a`'s.
fn blend(a: &StreakPiecePaintStatus, b: &StreakPiecePaintStatus, t: f32) -> StreakPiecePaintStatus {
    let mut mixed = *a;
    mixed.scene_point = lerp_each(a.scene_point, b.scene_point, t);
    mixed.basis = [
        lerp_each(a.basis[0], b.basis[0], t),
        lerp_each(a.basis[1], b.basis[1], t),
    ];
    mixed.rotation = lerp(a.rotation, b.rotation, t);
    mixed.size = lerp_each(a.size, b.size, t);
    mixed.across_texel = lerp(a.across_texel, b.across_texel, t);
    mixed
}

/// Appends one cross-section's worth of vertices for `seg`.
fn push_segment(
    section: &[StreakCorner],
    seg: &StreakPiecePaintStatus,
    vertex_points: &mut Vec<StreakLaidCorner>,
) {
    let blank = StreakLaidCorner {
        spot: [0.0; 3],
        tint: <[u8; 4]>::default(),
        u: 0.0,
        v: 0.0,
        packed_surface_uv: ZERO,
        facing_word: ZERO,
        surface_dir_word: ZERO,
    };
    let start = vertex_points.len();
    vertex_points.extend(std::iter::repeat_n(blank, section.len()));
    trail_emit_segment_verts(section, seg, &mut vertex_points[start..]);
}

#[cfg(test)]
mod tests {
    use super::build_corners_and_streaks;
    use super::*;
    use crate::UNTURNED_BASIS;
    use crate::particle_bin::particle_ticket_of_bin;
    use crate::host::EffectSystemHost;
    use crate::ribbon::{claim_ribbon, claim_ribbon_point};
    use fx_t6::runtime::trail_piece_ticket_for_slot;


    fn host_with_effect() -> (EffectSystemHost, u16) {
        let mut host = EffectSystemHost::new();
        let handle = host
            .start_effect("fx/test", [0.0; 3], UNTURNED_BASIS, 0, u32::MAX, false, 3)
            .expect("room for an effect");
        host.clock_ms_now = 500;
        (host, handle)
    }

    /// Puts one part of `elem_type` into the effect's first list.
    fn add_part(host: &mut EffectSystemHost, fx_ref: u16, cell_ix: usize, elem_type: u8) {
        let e = &mut host.elems[cell_ix];
        e.is_taken = true;
        e.look_variant_total = 1;
        e.part_kind = elem_type;
        e.window_open_ms = 100;
        e.lifespan_window_ms = 1000;
        let link = &mut e.following_particle_ticket;
        *link = NO_PARTICLE_TICKET;
        let handle_of_slot = particle_ticket_of_bin(cell_ix as u32);
        let fx = host.bin_of_ticket_editable(fx_ref).unwrap();
        fx.lead_particle_ticket[0] = handle_of_slot;
    }

    fn sprite() -> EffectSpriteInstance {
        EffectSpriteInstance {
            where_at: [0.0; 3],
            start_extent: 1.0,
            second_extent: 1.0,
            rgba_tint: [255; 4],
            shape_kind: 1,
            definition_name: Arc::from("fx/test"),
            ledger_index: 3,
            definition_slot: 5,
            surface_title: Arc::from("m"),
            shader_at: None,
            turn_rows: UNTURNED_BASIS,
            spin_rad: 0.0,
            move_dir: [0.0; 3],
            atlas: BillboardSheetCell::FULL,
            bit_word: 0,
            line: false,
            lit_sample: [0; 3],
            shading_blend: 0,
            packed_lighting_src: BakedShadeSource::Missing,
        }
    }

    fn only_sprites(host: &EffectSystemHost, give: bool) -> CornerBuildOutput {
        build_corners(host, |ctx| {
            assert_eq!(ctx.definition_name, "fx/test");
            assert_eq!(ctx.ledger_index, 3);
            assert_eq!(ctx.lived_ms, 400);
            assert_eq!(ctx.particle_lifespan_ms, 1000);
            assert!((ctx.life_frac - 0.4).abs() < 1e-6);
            give.then(sprite)
        })
    }

    #[test]
    fn a_live_billboard_becomes_a_sprite() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 4, 0);
        let out = only_sprites(&host, true);
        assert_eq!(out.sprites.len(), 1);
        assert_eq!(out.lookup_misses, 0);
        let out = only_sprites(&host, false);
        assert!(out.sprites.is_empty());
        assert_eq!(out.lookup_misses, 1);
    }

    #[test]
    fn parts_outside_their_life_or_without_visuals_are_counted() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 2, 0);
        host.clock_ms_now = 99;
        assert_eq!(build_corners(&host, |_| Some(sprite())).dormant_skips, 1);
        host.clock_ms_now = 1100;
        assert_eq!(build_corners(&host, |_| Some(sprite())).dormant_skips, 1);
        host.clock_ms_now = 1099;
        assert_eq!(build_corners(&host, |_| Some(sprite())).sprites.len(), 1);
        host.elems[2].look_variant_total = 0;
        let out = build_corners(&host, |_| Some(sprite()));
        assert_eq!((out.no_visual_skips, out.sprites.len()), (1, 0));
    }

    #[test]
    fn stopped_effects_draw_nothing() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 2, 0);
        host.bin_of_ticket_editable(handle).unwrap().state_bits |= 0x4000;
        let out = build_corners(&host, |_| Some(sprite()));
        assert_eq!((out.stopped_skips, out.sprites.len()), (1, 0));
    }

    #[test]
    fn odd_part_types_are_tallied_by_kind() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 2, 200);
        let out = build_corners(&host, |_| Some(sprite()));
        assert_eq!(
            (out.unsupported_kind_skips, out.other_kind_skips),
            (1, 1)
        );
        host.elems[2].part_kind = 10;
        let out = build_corners(&host, |_| Some(sprite()));
        assert_eq!(out.no_handler_skips, 1);
        assert_eq!(out.unsupported_kind_skips, 0);
    }

    fn light(is_spot: bool) -> ParticleGlowCopy {
        ParticleGlowCopy {
            definition_name: String::new(),
            definition_slot: 6,
            is_cone: is_spot,
            anchor: [0.0; 3],
            rad: 1.0,
            tint_blue_first: [1.0; 3],
            frame_rows: UNTURNED_BASIS,
        }
    }

    fn cloud(scale: f32, start_extent: f32) -> SwarmCopy {
        SwarmCopy {
            definition_name: String::new(),
            ledger_index: 0,
            definition_slot: 7,
            cloud: VisualParticleCloud {
                turn: [0.0, 0.0, 0.0, 2.0],
                spot: [0.5; 3],
                layout_size_ratio: scale,
                basis_or_speed: [0.25; 3],
                color: 0,
                start_extent,
                second_extent: 0.0,
                flags: 0,
                scale: 1.0,
            },
            particles: None,
        }
    }

    fn model(scale: f32) -> MeshParticleCopy {
        MeshParticleCopy {
            definition_name: String::new(),
            definition_slot: 8,
            mesh_slot_number: 0,
            part_id: 0,
            place_at: [0.0; 3],
            axes_m: UNTURNED_BASIS,
            size_k: scale,
            copy_bits: 0,
        }
    }

    fn run_kinds(
        host: &EffectSystemHost,
        cl: Option<SwarmCopy>,
        li: Option<ParticleGlowCopy>,
        mo: Option<MeshParticleCopy>,
    ) -> CornerBuildOutput {
        build_corners_and_streaks(
            host,
            &mut |_| None,
            &mut |_, _| None::<StreakPaintRecipe>,
            &mut |_| None,
            &mut |_| cl.clone(),
            &mut |_| li.clone(),
            &mut |_| mo.clone(),
        )
    }

    #[test]
    fn clouds_lights_and_models_go_to_their_own_lists() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 1, 4);
        let out = run_kinds(&host, Some(cloud(1.0, 2.0)), None, None);
        assert_eq!((out.clouds.len(), out.cloud_skips), (1, 0));
        assert_eq!(out.unsupported_kind_skips, 0);
        let out = run_kinds(&host, Some(cloud(0.0, 2.0)), None, None);
        assert_eq!((out.clouds.len(), out.cloud_skips), (0, 1));
        let out = run_kinds(&host, Some(cloud(1.0, 0.0)), None, None);
        assert_eq!(out.cloud_skips, 1);
        let out = run_kinds(&host, None, None, None);
        assert_eq!(out.cloud_skips, 1);

        host.elems[1].part_kind = 7;
        let out = run_kinds(&host, None, None, Some(model(2.0)));
        assert_eq!((out.models.len(), out.model_skips), (1, 0));
        assert_eq!(out.unsupported_kind_skips, 1);
        let out = run_kinds(&host, None, None, Some(model(0.0)));
        assert_eq!((out.models.len(), out.model_skips), (0, 1));

        host.elems[1].part_kind = 8;
        let out = run_kinds(&host, None, Some(light(false)), None);
        assert_eq!((out.omni_lights.len(), out.spot_lights.len()), (1, 0));
        assert_eq!(out.unsupported_kind_skips, 1);
        let out = run_kinds(&host, None, Some(light(true)), None);
        assert_eq!((out.omni_lights.len(), out.spot_lights.len()), (0, 1));
        let out = run_kinds(&host, None, None, None);
        assert_eq!((out.omni_light_skips, out.skipped_cone_light), (1, 0));
        host.elems[1].part_kind = 9;
        let out = run_kinds(&host, None, None, None);
        assert_eq!((out.omni_light_skips, out.skipped_cone_light), (0, 1));
    }

    #[test]
    fn every_part_in_the_chain_is_visited_in_order() {
        let (mut host, handle) = host_with_effect();
        add_part(&mut host, handle, 1, 0);
        add_part(&mut host, handle, 3, 0);
        host.elems[3].following_particle_ticket = particle_ticket_of_bin(1);
        let mut seen = Vec::new();
        let out = build_corners(&host, |ctx| {
            seen.push(ctx.part_no);
            Some(sprite())
        });
        assert_eq!(out.sprites.len(), 2);
        assert_eq!(seen, vec![particle_ticket_of_bin(3), particle_ticket_of_bin(1)]);
    }

    #[test]
    fn two_effects_are_both_walked() {
        let (mut host, first) = host_with_effect();
        let second = host
            .start_effect("fx/test", [0.0; 3], UNTURNED_BASIS, 0, u32::MAX, false, 3)
            .unwrap();
        add_part(&mut host, first, 1, 0);
        add_part(&mut host, second, 2, 0);
        let out = build_corners(&host, |_| Some(sprite()));
        assert_eq!(out.sprites.len(), 2);
    }

    fn cross_section() -> StreakPaintRecipe {
        let v = |x: f32| StreakCorner {
            spot: [x, 0.0],
            normal: [0.0, 1.0],
            sheet_v: 0.0,
        };
        StreakPaintRecipe {
            look_variant_total: 1,
            mode_bits: 0,
            lifespan_floor: 1000,
            lifespan_spread: 0_i32,
            scroll_period_millis: 0,
            recurrence_distance: 10,
            vertex_points: vec![v(-1.0), v(1.0)],
            index_run_values: vec![0, 1],
            surface_title: "ribbon".to_owned(),
            shader_slot: Some(5),
        }
    }

    /// A ribbon with samples at (window_open_ms, sequence, x) along the x axis.
    fn add_ribbon(host: &mut EffectSystemHost, ticket: u16, samples: &[(i32, u8, f32)]) {
        let trail_handle = claim_ribbon(host, 2).expect("a trail");
        let handles: Vec<u16> = samples
            .iter()
            .map(|&(msec, seq, x)| {
                let slot = claim_ribbon_point(host).expect("a sample");
                let s = &mut host.trail_elems[slot];
                s.is_taken = true;
                s.window_open_ms = msec;
                s.sequence = seq;
                s.spot_xyz = [x, 0.0, 0.0];
                s.spawn_dist = x;
                trail_piece_ticket_for_slot(slot as u32)
            })
            .collect();
        for (i, &h) in handles.iter().enumerate() {
            let slot = crate::ribbon::trail_elem_slot_for_handle(h).unwrap();
            host.trail_elems[slot].next_trail_elem_handle =
                handles.get(i + 1).copied().unwrap_or(NO_STREAK_TICKET);
        }
        let tslot = crate::ribbon::ribbon_index_of(trail_handle).unwrap();
        host.trails[tslot].lead_particle_ticket = handles[0];
        host.trails[tslot].last_elem_handle = *handles.last().unwrap();
        host.bin_of_ticket_editable(ticket).unwrap().first_ribbon_ticket = trail_handle;
    }

    fn visual_at(angle: f32) -> StreakSampleLook {
        StreakSampleLook {
            size: [1.0, 1.0],
            tint_rgba: [255; 4],
            spin_now: angle,
        }
    }

    fn mesh_of(host: &EffectSystemHost, def: Option<StreakPaintRecipe>) -> CornerBuildOutput {
        let mut def = def;
        build_corners_and_streaks(
            host,
            &mut |_| None,
            &mut |ctx, idx| {
                assert_eq!(
                    (ctx.definition_name, ctx.ledger_index, ctx.definition_slot),
                    ("fx/test", 3, 2)
                );
                assert_eq!(idx, 2);
                def.take()
            },
            &mut |ctx| Some(visual_at(ctx.sample_spot[0])),
            &mut |_| None,
            &mut |_| None,
            &mut |_| None,
        )
    }

    #[test]
    fn a_ribbon_becomes_one_mesh_with_faded_ends() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(
            &mut host,
            handle,
            &[(100, 0, 0.0), (200, 1, 5.0), (300, 2, 10.0)],
        );
        let out = mesh_of(&host, Some(cross_section()));
        assert_eq!(out.trail_strips.len(), 1);
        let mesh = &out.trail_strips[0];
        assert_eq!(mesh.definition_name, "fx/test");
        assert_eq!(mesh.definition_slot, 2);
        assert_eq!(mesh.surface_title, "ribbon");
        assert_eq!(mesh.shader_slot_ix, Some(5));
        assert_eq!(mesh.vertex_points.len(), 6);
        assert_eq!(mesh.pair_indices.len(), 6);
        assert_eq!(mesh.vertex_points[0].tint[3], 0);
        assert_eq!(mesh.vertex_points[1].tint[3], 0);
        assert_ne!(mesh.vertex_points[2].tint[3], 0);
        assert_eq!(mesh.vertex_points[4].tint[3], 0);
        assert_eq!(mesh.vertex_points[5].tint[3], 0);
        assert_eq!(mesh.pair_indices[0], [0, 1]);
        assert_eq!(mesh.pair_indices[1], [2, 3]);
        assert_eq!(mesh.pair_indices[2], [2, 1]);
        assert_eq!(mesh.pair_indices[3], [2, 3]);
        assert_eq!(mesh.pair_indices[4], [4, 5]);
        assert_eq!(mesh.pair_indices[5], [4, 3]);
    }

    #[test]
    fn ribbon_samples_from_the_future_are_left_out() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(
            &mut host,
            handle,
            &[(900, 0, 0.0), (100, 1, 5.0), (950, 2, 6.0), (200, 3, 7.0)],
        );
        let out = mesh_of(&host, Some(cross_section()));
        assert_eq!(out.trail_strips[0].vertex_points.len(), 4);
        host.clock_ms_now = 50;
        let out = mesh_of(&host, Some(cross_section()));
        assert!(out.trail_strips.is_empty());
    }

    #[test]
    fn an_expired_sample_brings_a_blended_segment_at_the_edge() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(
            &mut host,
            handle,
            &[(100, 5, 0.0), (460, 6, 10.0), (480, 7, 12.0)],
        );
        let mut def = cross_section();
        def.lifespan_floor = 400;
        let out = mesh_of(&host, Some(def));
        let mesh = &out.trail_strips[0];
        // sample 0 is past its life (age 400 of 400): no segment; sample 1
        // (age 40) is blended in from it first; sample 2 follows.
        assert_eq!(mesh.vertex_points.len(), 6);
        assert_eq!(mesh.vertex_points[0].tint[3], 255);
        assert_eq!(mesh.vertex_points[4].tint[3], 0);
    }

    #[test]
    fn bad_ribbon_shapes_are_counted_not_drawn() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(&mut host, handle, &[(100, 0, 0.0), (200, 1, 5.0)]);
        let mut def = cross_section();
        def.look_variant_total = 0;
        let out = mesh_of(&host, Some(def));
        assert_eq!((out.no_visual_skips, out.trail_strips.len()), (1, 0));
        for tweak in 0..4 {
            let mut def = cross_section();
            match tweak {
                0 => def.recurrence_distance = 0,
                1 => def.vertex_points.clear(),
                2 => def.index_run_values = vec![0],
                _ => def.index_run_values = vec![0, 1, 1],
            }
            let out = mesh_of(&host, Some(def));
            assert_eq!(out.no_ribbon_def_skips, 1, "tweak {tweak}");
            assert!(out.trail_strips.is_empty());
        }
        let out = mesh_of(&host, None);
        assert_eq!(out.no_ribbon_def_skips, 1);
    }

    #[test]
    fn unlooked_samples_are_counted_once() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(
            &mut host,
            handle,
            &[(100, 0, 0.0), (200, 1, 5.0), (300, 2, 7.0)],
        );
        let out = build_corners_and_streaks(
            &host,
            &mut |_| None,
            &mut |_, _| Some(cross_section()),
            &mut |_| None,
            &mut |_| None,
            &mut |_| None,
            &mut |_| None,
        );
        assert_eq!(out.lookup_misses, 1);
        assert!(out.trail_strips.is_empty());
    }

    #[test]
    fn faces_travel_turns_the_basis_along_the_line() {
        let (mut host, handle) = host_with_effect();
        add_ribbon(
            &mut host,
            handle,
            &[(100, 0, 0.0), (200, 1, 5.0), (300, 2, 10.0)],
        );
        let mut plain = cross_section();
        let a = mesh_of(&host, Some(plain.clone())).trail_strips.remove(0);
        plain.mode_bits = TRAIL_FACES_TRAVEL;
        let b = mesh_of(&host, Some(plain)).trail_strips.remove(0);
        assert_eq!(a.vertex_points.len(), b.vertex_points.len());
        // the first segment keeps the stored basis either way
        assert_eq!(a.vertex_points[0], b.vertex_points[0]);
        assert_ne!(a.vertex_points[2].spot, b.vertex_points[2].spot);
    }

    #[test]
    fn travel_basis_is_perpendicular_and_none_when_vertical() {
        let [side, up] = basis_along_travel([0.0; 3], [10.0, 0.0, 0.0]).unwrap();
        assert_eq!(side, [0.0, -1.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, -1.0]);
        let [side, up] = basis_along_travel([0.0; 3], [0.0, 4.0, 0.0]).unwrap();
        assert_eq!(side, [1.0, 0.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, -1.0]);
        assert!(basis_along_travel([0.0; 3], [0.0, 0.0, 9.0]).is_none());
        assert!(basis_along_travel([1.0; 3], [1.0; 3]).is_none());
    }

    #[test]
    fn blending_mixes_everything_but_colour() {
        let mk = |k: f32, c: u8| StreakPiecePaintStatus {
            scene_point: [k, 2.0 * k, 3.0 * k],
            basis: [[k; 3], [2.0 * k; 3]],
            rotation: k,
            size: [k, 4.0 * k],
            across_texel: k,
            tint: [c; 4],
        };
        let mid = blend(&mk(0.0, 10), &mk(4.0, 90), 0.25);
        assert_eq!(mid.scene_point, [1.0, 2.0, 3.0]);
        assert_eq!(mid.basis, [[1.0; 3], [2.0; 3]]);
        assert_eq!((mid.rotation, mid.across_texel), (1.0, 1.0));
        assert_eq!(mid.size, [1.0, 4.0]);
        assert_eq!(mid.tint, [10; 4]);
    }

    #[test]
    fn a_segment_appends_one_vertex_per_section_point() {
        let sect = cross_section().vertex_points;
        let st = StreakPiecePaintStatus {
            scene_point: [3.0, 0.0, 0.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            rotation: 0.5,
            size: [1.0, 1.0],
            across_texel: 0.0,
            tint: [9; 4],
        };
        let mut vertex_points = Vec::new();
        push_segment(&sect, &st, &mut vertex_points);
        push_segment(&sect, &st, &mut vertex_points);
        assert_eq!(vertex_points.len(), 4);
        assert_eq!(vertex_points[0], vertex_points[2]);
        assert_eq!(vertex_points[0].tint, [9; 4]);
    }
}
