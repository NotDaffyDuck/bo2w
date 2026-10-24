//! Turning the live effects into things to draw: sprites, lights, models,
//! particle clouds and ribbon meshes. What a part looks like (material, size,
//! colour) is the caller's business; this walks the pools and hands each
//! visible part to the caller with everything it needs to decide.

use std::sync::Arc;

use fx_t6::runtime::{
    FX_RAND_CH_LIFE, FxElemType, FxSpriteAtlasUv, FxTrailEmittedVert, FxTrailSegmentDrawState,
    FxTrailVertex, GfxParticleCloud, elem_norm_time, elem_random_seed, random_table_u16,
    sample_life_span_msec, trail_compute_u, trail_emit_index_quad, trail_emit_segment_verts,
    trail_uncompress_basis, vec3_length_sq, vec3_normalize,
};

use crate::elem::{FX_ELEM_HANDLE_NONE, FxElemSlot, elem_slot_for_handle};
use crate::orient::elem_world_origin;
use crate::system::{FxEffectSlot, FxPackedLightingSrc, FxSystemHost};
use crate::trail::{FX_TRAIL_HANDLE_NONE, trail_elem_slot_for_handle, trail_slot_for_handle};

/// Everything known about one live part at the moment it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct FxDrawElemContext<'a> {
    pub def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
    pub elem_type: u8,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub norm_time: f32,
    pub age_msec: i32,
    pub life_msec: i32,
    pub elem_random_seed: u32,
    pub sequence: u8,
    pub base_vel: [f32; 3],
    pub flags: i32,
    pub at_rest_fraction: u8,
    pub packed_lighting: [u8; 3],
    pub packed_lighting_src: FxPackedLightingSrc,
    pub elem_handle: u16,
}

/// Which ribbon is being drawn.
#[derive(Clone, Copy, Debug)]
pub struct FxDrawTrailContext<'a> {
    pub def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
}

/// One sample along a ribbon, as the caller is asked to dress it.
#[derive(Clone, Copy, Debug)]
pub struct FxDrawTrailSampleContext<'a> {
    pub def_name: &'a str,
    pub catalog_index: u16,
    pub def_index: u8,
    pub sample_sequence: u8,
    pub sample_origin: [f32; 3],
    pub spawn_dist: f32,
    pub msec_begin: i32,
    pub norm_time: f32,
    pub age_msec: i32,
    pub life_msec: i32,
    pub elem_random_seed: u32,
}

/// The authored shape of a ribbon: its cross-section and how it tiles.
#[derive(Clone, Debug)]
pub struct FxTrailDrawDef {
    pub visual_count: u8,
    pub flags: i32,
    pub life_base: i32,
    pub life_amp: i32,
    pub scroll_time_msec: i32,
    pub repeat_dist: i32,
    pub verts: Vec<FxTrailVertex>,
    pub inds: Vec<u16>,
    pub material_name: String,
    pub material_index: Option<usize>,
}

/// How one ribbon sample looks right now.
#[derive(Clone, Copy, Debug)]
pub struct FxTrailSampleVisual {
    pub size: [f32; 2],
    pub color_rgba: [u8; 4],
    pub rotation: f32,
}

#[derive(Clone, Debug)]
pub struct FxSpriteInstance {
    pub origin: [f32; 3],
    pub size0: f32,
    pub size1: f32,
    pub color_rgba: [u8; 4],
    pub elem_type: u8,
    pub def_name: Arc<str>,
    pub catalog_index: u16,
    pub def_index: u8,
    pub material_name: Arc<str>,
    pub material_index: Option<usize>,
    pub axis: [[f32; 3]; 3],
    pub rotation_rad: f32,
    pub vel_dir: [f32; 3],
    pub atlas: FxSpriteAtlasUv,
    pub flags: i32,
    /// A BO2 line elem: a tail that runs ahead of its part, along its
    /// velocity: the zombies' eye beams (they start behind the eye and move
    /// out of the face), the fires' flame sheets (they start low and rise).
    pub line: bool,
    pub lighting_sample: [u8; 3],
    pub lighting_frac: u8,
    pub packed_lighting_src: FxPackedLightingSrc,
}

#[derive(Clone, Debug, Default)]
pub struct FxTrailMeshInstance {
    pub def_name: String,
    pub def_index: u8,
    pub material_name: String,
    pub material_index: Option<usize>,
    pub verts: Vec<FxTrailEmittedVert>,
    pub index_pairs: Vec<[u16; 2]>,
}

/// What one pass over the pools produced, plus a count of every part that was
/// passed over and why.
#[derive(Clone, Debug, Default)]
pub struct FxGenerateVertsOut {
    pub sprites: Vec<FxSpriteInstance>,
    pub trail_meshes: Vec<FxTrailMeshInstance>,
    pub skipped_dormant: u32,
    pub skipped_no_visual: u32,
    pub skipped_null_handler: u32,
    pub skipped_unsupported_type: u32,
    pub skipped_cloud: u32,
    pub skipped_model: u32,
    pub skipped_omni_light: u32,
    pub skipped_spot_light: u32,
    pub skipped_other_type: u32,
    pub skipped_no_lookup: u32,
    pub skipped_stopped: u32,
    pub skipped_no_trail_def: u32,
    pub clouds: Vec<FxCloudInstance>,
    pub omni_lights: Vec<FxElemLightInstance>,
    pub spot_lights: Vec<FxElemLightInstance>,
    pub models: Vec<FxModelInstance>,
}

#[derive(Clone, Debug)]
pub struct FxCloudInstance {
    pub def_name: String,
    pub catalog_index: u16,
    pub def_index: u8,
    pub cloud: GfxParticleCloud,
    /// bo2zm: a Black Ops II cloud's particle count (legacy clouds: none, the
    /// shape bits in `cloud.flags` decide).
    pub particles: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct FxElemLightInstance {
    pub def_name: String,
    pub def_index: u8,
    pub is_spot: bool,
    pub origin: [f32; 3],
    pub radius: f32,
    pub color_bgr: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

#[derive(Clone, Debug)]
pub struct FxModelInstance {
    pub def_name: String,
    pub def_index: u8,
    pub model_index: usize,
    pub elem_handle: u16,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub scale: f32,
    pub flags: i32,
}

/// Effect status bit: stopped, nothing of it is drawn any more.
const STATUS_NOT_DRAWN: u32 = 0x4000;

/// Ribbon flag: orient the cross-section from the direction of travel rather
/// than from the stored basis.
const TRAIL_FACES_TRAVEL: i32 = 0x4000;

fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

/// The callbacks that dress an ordinary part, by kind.
struct PartSinks<'a> {
    sprite: &'a mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxSpriteInstance>,
    cloud: &'a mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxCloudInstance>,
    light: &'a mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxElemLightInstance>,
    model: &'a mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxModelInstance>,
}

/// The callbacks that dress a ribbon.
struct TrailSinks<'a> {
    definition: &'a mut dyn FnMut(FxDrawTrailContext<'_>, u8) -> Option<FxTrailDrawDef>,
    sample: &'a mut dyn FnMut(FxDrawTrailSampleContext<'_>) -> Option<FxTrailSampleVisual>,
}

/// Sprites only: the other kinds are skipped.
pub fn generate_verts(
    host: &FxSystemHost,
    mut on_elem: impl FnMut(FxDrawElemContext<'_>) -> Option<FxSpriteInstance>,
) -> FxGenerateVertsOut {
    generate_verts_with_trails(
        host,
        &mut on_elem,
        &mut |_, _| None,
        &mut |_| None,
        &mut |_| None,
        &mut |_| None,
        &mut |_| None,
    )
}

/// Walks every effect in the active window of the ring, parts first and then
/// ribbons, and collects what the callbacks make of them.
pub fn generate_verts_with_trails(
    host: &FxSystemHost,
    on_elem: &mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxSpriteInstance>,
    on_trail_def: &mut dyn FnMut(FxDrawTrailContext<'_>, u8) -> Option<FxTrailDrawDef>,
    on_trail_sample: &mut dyn FnMut(FxDrawTrailSampleContext<'_>) -> Option<FxTrailSampleVisual>,
    on_cloud: &mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxCloudInstance>,
    on_light: &mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxElemLightInstance>,
    on_model: &mut dyn FnMut(FxDrawElemContext<'_>) -> Option<FxModelInstance>,
) -> FxGenerateVertsOut {
    let mut parts = PartSinks {
        sprite: on_elem,
        cloud: on_cloud,
        light: on_light,
        model: on_model,
    };
    let mut trails = TrailSinks {
        definition: on_trail_def,
        sample: on_trail_sample,
    };
    let mut out = FxGenerateVertsOut::default();

    let end = host.first_new_effect as u32;
    let mut ring_pos = host.first_active_effect as u32;
    while ring_pos != end {
        let handle = host.handle_at_ring(ring_pos);
        if let Some(slot) = host.slot_index_for_handle(handle) {
            draw_parts(host, slot, &mut parts, &mut out);
            draw_trails(host, slot, &mut trails, &mut out);
        }
        ring_pos = ring_pos.wrapping_add(1);
    }
    out
}

/// Draws every part of one effect, all three classes in order.
fn draw_parts(
    host: &FxSystemHost,
    effect_slot: usize,
    sinks: &mut PartSinks<'_>,
    out: &mut FxGenerateVertsOut,
) {
    let Some(effect) = host.effect_at(effect_slot).filter(|e| e.ring_resident) else {
        return;
    };
    if effect.status & STATUS_NOT_DRAWN != 0 {
        bump(&mut out.skipped_stopped);
        return;
    }

    for &head in &effect.first_elem_handle[..3] {
        let mut handle = head;
        while handle != FX_ELEM_HANDLE_NONE {
            // The link is read before the part is drawn.
            let following = elem_slot_for_handle(handle)
                .and_then(|slot| host.elems.get(slot))
                .map_or(FX_ELEM_HANDLE_NONE, |e| e.next_elem_handle);
            draw_part(host, effect, handle, sinks, out);
            handle = following;
        }
    }
}

fn elem_context<'a>(
    host: &FxSystemHost,
    effect: &'a FxEffectSlot,
    elem: &FxElemSlot,
    handle: u16,
) -> FxDrawElemContext<'a> {
    let age_msec = host.msec_now.wrapping_sub(elem.msec_begin);
    let seed = elem_random_seed(effect.random_seed, elem.sequence, elem.msec_begin);
    let origin = elem_world_origin(
        elem.origin,
        elem.flags,
        &effect.frame_now(),
        &effect.frame_when_played(),
        Some(elem.orient_spawn_params(seed)),
    );
    FxDrawElemContext {
        def_name: effect.def_name.as_str(),
        catalog_index: effect.catalog_index,
        def_index: elem.def_index,
        elem_type: elem.elem_type,
        origin,
        axis: effect.axis,
        norm_time: elem_norm_time(age_msec, elem.life_span_msec),
        age_msec,
        life_msec: elem.life_span_msec,
        elem_random_seed: seed,
        sequence: elem.sequence,
        base_vel: elem.base_vel,
        flags: elem.flags,
        at_rest_fraction: elem.at_rest_fraction,
        packed_lighting: effect.packed_lighting,
        packed_lighting_src: effect.packed_lighting_src,
        elem_handle: handle,
    }
}

fn draw_part(
    host: &FxSystemHost,
    effect: &FxEffectSlot,
    handle: u16,
    sinks: &mut PartSinks<'_>,
    out: &mut FxGenerateVertsOut,
) {
    let Some(elem) = elem_slot_for_handle(handle)
        .and_then(|slot| host.elems.get(slot))
        .filter(|e| e.occupied)
    else {
        return;
    };

    if elem.visual_count == 0 {
        bump(&mut out.skipped_no_visual);
        return;
    }
    let born = elem.msec_begin;
    let dies = born.wrapping_add(elem.life_span_msec);
    if host.msec_now < born || host.msec_now >= dies {
        bump(&mut out.skipped_dormant);
        return;
    }

    let Some(kind) = FxElemType::from_u8(elem.elem_type) else {
        bump(&mut out.skipped_unsupported_type);
        bump(&mut out.skipped_other_type);
        return;
    };
    if kind.draw_elem_handler_is_null() {
        bump(&mut out.skipped_null_handler);
        return;
    }

    let ctx = elem_context(host, effect, elem, handle);
    match kind {
        FxElemType::Billboard | FxElemType::Oriented | FxElemType::Tail => {
            match (sinks.sprite)(ctx) {
                Some(sprite) => out.sprites.push(sprite),
                None => bump(&mut out.skipped_no_lookup),
            }
        }
        FxElemType::Cloud => match (sinks.cloud)(ctx) {
            Some(cloud) if cloud.cloud.placement_scale != 0.0 && cloud.cloud.size0 != 0.0 => {
                out.clouds.push(cloud);
            }
            _ => bump(&mut out.skipped_cloud),
        },
        FxElemType::Model => {
            bump(&mut out.skipped_unsupported_type);
            match (sinks.model)(ctx) {
                Some(model) if model.scale != 0.0 => out.models.push(model),
                _ => bump(&mut out.skipped_model),
            }
        }
        FxElemType::OmniLight | FxElemType::SpotLight => {
            bump(&mut out.skipped_unsupported_type);
            match (sinks.light)(ctx) {
                Some(light) if light.is_spot => out.spot_lights.push(light),
                Some(light) => out.omni_lights.push(light),
                None if kind == FxElemType::SpotLight => bump(&mut out.skipped_spot_light),
                None => bump(&mut out.skipped_omni_light),
            }
        }
        _ => {
            bump(&mut out.skipped_unsupported_type);
            bump(&mut out.skipped_other_type);
        }
    }
}

/// Draws every ribbon of one effect.
fn draw_trails(
    host: &FxSystemHost,
    effect_slot: usize,
    sinks: &mut TrailSinks<'_>,
    out: &mut FxGenerateVertsOut,
) {
    let Some(effect) = host.effect_at(effect_slot).filter(|e| e.ring_resident) else {
        return;
    };
    if effect.status & STATUS_NOT_DRAWN != 0 {
        return;
    }

    let mut handle = effect.first_trail_handle;
    while handle != FX_TRAIL_HANDLE_NONE {
        let Some(trail) = trail_slot_for_handle(handle)
            .and_then(|slot| host.trails.get(slot))
            .filter(|t| t.occupied)
        else {
            break;
        };
        let def_index = trail.def_index as u8;
        let which = FxDrawTrailContext {
            def_name: effect.def_name.as_str(),
            catalog_index: effect.catalog_index,
            def_index,
        };
        match (sinks.definition)(which, def_index) {
            None => bump(&mut out.skipped_no_trail_def),
            Some(shape) => {
                let ribbon = Ribbon {
                    def_name: effect.def_name.as_str(),
                    catalog_index: effect.catalog_index,
                    def_index,
                    effect_seed: effect.random_seed,
                    first_sample: trail.first_elem_handle,
                    last_sample: trail.last_elem_handle,
                };
                build_ribbon(host, &ribbon, &shape, &mut *sinks.sample, out);
            }
        }
        handle = trail.next_trail_handle;
    }
}

/// The identity of one ribbon being meshed.
struct Ribbon<'a> {
    def_name: &'a str,
    catalog_index: u16,
    def_index: u8,
    effect_seed: u16,
    first_sample: u16,
    last_sample: u16,
}

fn build_ribbon(
    host: &FxSystemHost,
    ribbon: &Ribbon<'_>,
    shape: &FxTrailDrawDef,
    dress: &mut dyn FnMut(FxDrawTrailSampleContext<'_>) -> Option<FxTrailSampleVisual>,
    out: &mut FxGenerateVertsOut,
) {
    if shape.visual_count == 0 {
        bump(&mut out.skipped_no_visual);
        return;
    }
    if ribbon.first_sample == FX_TRAIL_HANDLE_NONE {
        return;
    }
    let tiling_is_sane =
        shape.repeat_dist != 0 && !shape.verts.is_empty() && shape.inds.len() >= 2 && shape.inds.len() % 2 == 0;
    if !tiling_is_sane {
        bump(&mut out.skipped_no_trail_def);
        return;
    }

    let now = host.msec_now;

    // Samples laid in the future are not there yet; start at the first one
    // that is.
    let mut cursor = ribbon.first_sample;
    let mut head = None;
    while cursor != FX_TRAIL_HANDLE_NONE {
        let Some(sample) = trail_elem_slot_for_handle(cursor)
            .and_then(|slot| host.trail_elems.get(slot))
            .filter(|s| s.occupied)
        else {
            break;
        };
        if sample.msec_begin <= now {
            head = Some((cursor, sample.spawn_dist));
            break;
        }
        cursor = sample.next_trail_elem_handle;
    }
    let Some((head_handle, head_dist)) = head else {
        return;
    };

    let u_scroll = trail_compute_u(head_dist, shape.repeat_dist, shape.scroll_time_msec, now) as f32;
    let verts_per_segment = shape.verts.len() as u16;

    let mut mesh = FxTrailMeshInstance {
        def_name: ribbon.def_name.to_owned(),
        def_index: ribbon.def_index,
        material_name: shape.material_name.clone(),
        material_index: shape.material_index,
        verts: Vec::new(),
        index_pairs: Vec::new(),
    };

    let mut prev: Option<(FxTrailSegmentDrawState, f32)> = None;
    let mut segments: u16 = 0;
    let mut any_unlooked = false;

    let mut handle = head_handle;
    while handle != FX_TRAIL_HANDLE_NONE {
        let Some(sample) = trail_elem_slot_for_handle(handle)
            .and_then(|slot| host.trail_elems.get(slot))
            .filter(|s| s.occupied)
        else {
            break;
        };
        let following = sample.next_trail_elem_handle;
        if sample.msec_begin > now {
            handle = following;
            continue;
        }

        let seed = (u32::from(ribbon.effect_seed)
            .wrapping_add(u32::from(sample.sequence).wrapping_mul(0x128)))
            % 0x1df;
        let life_msec = sample_life_span_msec(
            shape.life_base,
            shape.life_amp,
            random_table_u16(seed, FX_RAND_CH_LIFE),
        );
        let age_msec = now.wrapping_sub(sample.msec_begin);
        let norm_time = elem_norm_time(age_msec, life_msec);

        let look = dress(FxDrawTrailSampleContext {
            def_name: ribbon.def_name,
            catalog_index: ribbon.catalog_index,
            def_index: ribbon.def_index,
            sample_sequence: sample.sequence,
            sample_origin: sample.origin,
            spawn_dist: sample.spawn_dist,
            msec_begin: sample.msec_begin,
            norm_time,
            age_msec,
            life_msec,
            elem_random_seed: seed,
        });
        let Some(look) = look else {
            any_unlooked = true;
            handle = following;
            continue;
        };

        let mut basis = trail_uncompress_basis(&sample.basis);
        if shape.flags & TRAIL_FACES_TRAVEL != 0 && handle != ribbon.first_sample {
            if let Some((before, _)) = &prev {
                if let Some(turned) = basis_along_travel(before.pos_world, sample.origin) {
                    basis = turned;
                }
            }
        }

        let mut state = FxTrailSegmentDrawState {
            pos_world: sample.origin,
            basis,
            rotation: look.rotation,
            size: look.size,
            u_coord: sample.spawn_dist / (shape.repeat_dist as f32) + u_scroll,
            color_rgba: look.color_rgba,
        };

        // A sample that has not expired yet makes a segment; the one that
        // crosses the expiry edge also brings a blended segment at the edge.
        if norm_time < 1.0 {
            if handle == ribbon.first_sample {
                if sample.sequence == 0 {
                    state.color_rgba[3] = 0;
                }
            } else if let Some((before, before_norm)) = &prev {
                if *before_norm >= 1.0 {
                    let at = (1.0 - before_norm) / (norm_time - before_norm);
                    push_segment(&shape.verts, &blend(before, &state, at), &mut mesh.verts);
                    segments = segments.saturating_add(1);
                }
                if handle == ribbon.last_sample {
                    state.color_rgba[3] = 0;
                }
            }
            push_segment(&shape.verts, &state, &mut mesh.verts);
            segments = segments.saturating_add(1);
        }

        prev = Some((state, norm_time));
        handle = following;
    }

    if any_unlooked {
        bump(&mut out.skipped_no_lookup);
    }

    // Each pair of adjacent segments is stitched with the cross-section's
    // index list.
    let mut first_vert: u16 = 0;
    for _ in 1..segments {
        for edge in shape.inds.chunks_exact(2) {
            let quad = trail_emit_index_quad(edge[0], edge[1], first_vert, verts_per_segment);
            mesh.index_pairs.extend_from_slice(&quad);
        }
        first_vert = first_vert.wrapping_add(verts_per_segment);
    }

    if !mesh.verts.is_empty() {
        out.trail_meshes.push(mesh);
    }
}

/// A cross-section whose width runs sideways of the line from `from` to `to`
/// and whose height is perpendicular to both. `None` when the line is
/// vertical (no sideways direction exists).
fn basis_along_travel(from: [f32; 3], to: [f32; 3]) -> Option<[[f32; 3]; 2]> {
    let heading = vec3_normalize([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
    let sideways = [heading[1], -heading[0], 0.0];
    (vec3_length_sq(sideways) > 0.0).then(|| {
        let sideways = vec3_normalize(sideways);
        let upwards = vec3_normalize([
            heading[1] * sideways[2] - heading[2] * sideways[1],
            heading[2] * sideways[0] - heading[0] * sideways[2],
            heading[0] * sideways[1] - heading[1] * sideways[0],
        ]);
        [sideways, upwards]
    })
}

/// The state a fraction `t` of the way from `a` to `b`; colour stays `a`'s.
fn blend(a: &FxTrailSegmentDrawState, b: &FxTrailSegmentDrawState, t: f32) -> FxTrailSegmentDrawState {
    let mix = |x: f32, y: f32| x + t * (y - x);
    let mix3 = |x: [f32; 3], y: [f32; 3]| [mix(x[0], y[0]), mix(x[1], y[1]), mix(x[2], y[2])];
    FxTrailSegmentDrawState {
        pos_world: mix3(a.pos_world, b.pos_world),
        basis: [mix3(a.basis[0], b.basis[0]), mix3(a.basis[1], b.basis[1])],
        rotation: mix(a.rotation, b.rotation),
        size: [mix(a.size[0], b.size[0]), mix(a.size[1], b.size[1])],
        u_coord: mix(a.u_coord, b.u_coord),
        color_rgba: a.color_rgba,
    }
}

/// Appends one cross-section's worth of vertices for `state`.
fn push_segment(
    section: &[FxTrailVertex],
    state: &FxTrailSegmentDrawState,
    verts: &mut Vec<FxTrailEmittedVert>,
) {
    let blank = FxTrailEmittedVert {
        xyz: [0.0; 3],
        color_rgba: [0; 4],
        u: 0.0,
        v: 0.0,
        texcoord_packed: 0,
        normal_packed: 0,
        tangent_packed: 0,
    };
    let from = verts.len();
    verts.resize(from + section.len(), blank);
    trail_emit_segment_verts(section, state, &mut verts[from..]);
}
