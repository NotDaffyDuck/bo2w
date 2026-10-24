//! Black Ops II effects, read into the effect catalog.
//!
//! `asset_t6` hands an element over as its raw bytes plus the names it points
//! at. The velocity and visual samples are copied as stored; the element type
//! is renumbered to the one the effect runtime plays by, and the flag bits are
//! kept as the zone has them.

use fastfile_t6::layout::{FxElemDef as e, FxElemVelStateSample, FxElemVisStateSample};
use fx_t6::runtime::{FxEffectDefView, FxElemDefView, FxElemVec3Range, FxTrailVertex};

use super::definitions::{FxCatalog, OwnedFxEffectDef};
use super::elem_type;
use super::visual::{OwnedFxElemDef, OwnedFxTrailDef, OwnedFxVisual, pending_child};
use crate::AssetNamespace;

/// The runtime's element type for the number a zone stores.
pub fn t6_elem_type(raw: u8) -> u8 {
    match raw {
        0 => elem_type::BILLBOARD,
        1 | 2 => elem_type::ORIENTED,
        3 | 4 => elem_type::TAIL,
        5 => elem_type::TRAIL,
        6 => elem_type::CLOUD,
        other => other,
    }
}

fn t6_elem_view(el: &asset_t6::FxElemRef) -> (FxElemDefView, [u8; 8]) {
    let f = |off: usize| el.f32_at(off);
    let i = |off: usize| el.i32_at(off);
    let range = |off: usize| [f(off), f(off + 4)];
    let b = |off: usize| el.raw.get(off).copied().unwrap_or(0);
    let mut atlas = [0u8; 8];
    if let Some(src) = el.raw.get(e::atlas..e::atlas + 8) {
        atlas.copy_from_slice(src);
    }
    // The zone packs the entry count with an index range into the last two
    // bytes (count in the low byte: 516 is 4 entries on a 2x2 atlas); the
    // element keeps a plain 16-bit count there.
    let count = u16::from(atlas[6]).max(1);
    atlas[6..8].copy_from_slice(&count.to_le_bytes());
    let view = FxElemDefView {
        flags: i(e::flags),
        spawn_a: i(e::spawn),
        spawn_b: i(e::spawn + 4),
        spawn_range_base: f(e::spawnRange),
        spawn_range_amplitude: f(e::spawnRange + 4),
        fade_in_range: range(e::fadeInRange),
        fade_out_range: range(e::fadeOutRange),
        spawn_frustum_cull_radius: f(e::spawnFrustumCullRadius),
        spawn_delay_msec_base: i(e::spawnDelayMsec),
        spawn_delay_msec_amplitude: i(e::spawnDelayMsec + 4),
        life_span_msec_base: i(e::lifeSpanMsec),
        life_span_msec_amplitude: i(e::lifeSpanMsec + 4),
        spawn_origin: core::array::from_fn(|a| range(e::spawnOrigin + a * 8)),
        spawn_offset_radius_base: f(e::spawnOffsetRadius),
        spawn_offset_radius_amplitude: f(e::spawnOffsetRadius + 4),
        spawn_offset_height_base: f(e::spawnOffsetHeight),
        spawn_offset_height_amplitude: f(e::spawnOffsetHeight + 4),
        spawn_angles: core::array::from_fn(|a| range(e::spawnAngles + a * 8)),
        angular_velocity: core::array::from_fn(|a| range(e::angularVelocity + a * 8)),
        initial_rotation: range(e::initialRotation),
        gravity_base: f(e::gravity),
        gravity_amplitude: f(e::gravity + 4),
        reflection_factor: range(e::reflectionFactor),
        coll_mins: core::array::from_fn(|a| f(e::collMins + a * 4)),
        coll_maxs: core::array::from_fn(|a| f(e::collMaxs + a * 4)),
        elem_type: t6_elem_type(b(e::elemType)),
        visual_count: b(e::visualCount),
        vel_interval_count: b(e::velIntervalCount),
        vis_state_interval_count: b(e::visStateIntervalCount),
        lighting_frac: b(e::lightingFrac),
        use_item_clip: 0,
        line: b(e::elemType) == 4,
        sort_order: b(e::sortOrder),
        emit_dist: range(e::emitDist),
        emit_dist_variance: range(e::emitDistVariance),
    };
    (view, atlas)
}

fn t6_visuals(el: &asset_t6::FxElemRef, view: &FxElemDefView) -> Vec<OwnedFxVisual> {
    let is_light = matches!(view.elem_type, elem_type::OMNI_LIGHT | elem_type::SPOT_LIGHT);
    if is_light || el.visuals.is_empty() {
        return vec![OwnedFxVisual::None];
    }
    el.visuals
        .iter()
        .map(|visual| match visual {
            asset_t6::FxVisualRef::Material(name) => OwnedFxVisual::material(name),
            asset_t6::FxVisualRef::Mark([first, second]) => OwnedFxVisual::mark(first, second),
            asset_t6::FxVisualRef::Model(name) => OwnedFxVisual::model(name),
            asset_t6::FxVisualRef::Effect(name) => OwnedFxVisual::runner(name),
            asset_t6::FxVisualRef::Sound(name) => OwnedFxVisual::sound(name),
            asset_t6::FxVisualRef::Light(_) | asset_t6::FxVisualRef::None => OwnedFxVisual::None,
        })
        .collect()
}

fn t6_trail(el: &asset_t6::FxElemRef) -> Option<OwnedFxTrailDef> {
    let trail = el.trail.as_ref()?;
    // A split distance of (almost) nothing means no splitting by distance: the
    // Warthog's contrail stores a denormal, whose inverse is infinite and asked
    // the trail for billions of pieces a frame.
    let inv_split_dist = Some(1.0 / trail.split_dist)
        .filter(|inverse| trail.split_dist > 0.0 && inverse.is_finite())
        .unwrap_or(0.0);
    Some(OwnedFxTrailDef {
        scroll_time_msec: trail.scroll_time_msec,
        repeat_dist: trail.repeat_dist,
        inv_split_dist,
        inv_split_arc_dist: 0.0,
        inv_split_time: 0.0,
        verts: trail
            .verts
            .iter()
            .map(|v| FxTrailVertex {
                pos: [v[0], v[1]],
                normal: [v[2], v[3]],
                tex_coord: v[4],
            })
            .collect(),
        inds: trail.inds.clone(),
    })
}

/// The velocity graph of one frame of reference: each fencepost's base and
/// amplitude. A velocity sample holds the local graph first, then the world
/// graph half a stride along.
fn parse_vel_graph_channel(samples: &[u8], fenceposts: usize, world: bool) -> Vec<FxElemVec3Range> {
    const STRIDE: usize = 0x60;
    let channel_start = if world { 0x30 } else { 0 };
    (0..fenceposts)
        .map_while(|post| {
            let at = post * STRIDE + channel_start;
            let bytes = samples.get(at..at + 24)?;
            let float = |n: usize| f32::from_le_bytes([bytes[n], bytes[n + 1], bytes[n + 2], bytes[n + 3]]);
            Some(FxElemVec3Range {
                base: core::array::from_fn(|axis| float(axis * 4)),
                amplitude: core::array::from_fn(|axis| float(12 + axis * 4)),
            })
        })
        .collect()
}

fn t6_elem(el: &asset_t6::FxElemRef) -> OwnedFxElemDef {
    let (view, atlas) = t6_elem_view(el);
    let vel_count = usize::from(view.vel_interval_count) + 1;
    let vel_samples = el
        .vel_samples
        .get(..vel_count * FxElemVelStateSample::SIZE)
        .unwrap_or(&el.vel_samples)
        .to_vec();
    let vis_count = usize::from(view.vis_state_interval_count) + 1;
    let mut vis_samples = el
        .vis_samples
        .get(..vis_count * FxElemVisStateSample::SIZE)
        .unwrap_or(&el.vis_samples)
        .to_vec();
    // A sample stores each colour red first; the draw wants blue first (flames
    // drew blue-white and smoke dark red until they were swapped).
    for sample in vis_samples.chunks_exact_mut(FxElemVisStateSample::SIZE) {
        for half in [0, FxElemVisStateSample::SIZE / 2] {
            sample.swap(half, half + 2);
        }
    }
    // The effects spawned along the way are bound once the catalog is full.
    let (effect_on_impact, on_impact_name) = pending_child(&el.effect_on_impact);
    let (effect_on_death, on_death_name) = pending_child(&el.effect_on_death);
    let (effect_emitted, emitted_name) = pending_child(&el.effect_emitted);
    OwnedFxElemDef {
        vel_graph_local: parse_vel_graph_channel(&vel_samples, vel_count, false),
        vel_graph_world: parse_vel_graph_channel(&vel_samples, vel_count, true),
        visuals: t6_visuals(el, &view),
        trail_def: if view.elem_type == elem_type::TRAIL { t6_trail(el) } else { None },
        // A cloud stores how many particles it holds in the place other
        // elements keep their shape bits: a base, and a random extra on top
        // (512 and up to 512 more on Nuketown's ash and ember clouds).
        cloud_density: (view.elem_type == elem_type::CLOUD).then(|| [el.i32_at(e::u), el.i32_at(e::u + 4)]),
        atlas,
        view,
        vel_samples,
        vis_samples,
        effect_on_impact,
        effect_on_death,
        effect_emitted,
        on_impact_name,
        on_death_name,
        emitted_name,
    }
}

/// The element flag for a part drawn with the first-person gun. The game's own
/// first-person gas flashes, muzzle breaks and shell casings carry it; the Ray
/// Gun's first-person flash does without.
const T6_FX_ELEM_DRAW_WITH_VIEWMODEL: i32 = 0x1000;

/// How much smaller a far-view muzzle flash is drawn on the player's own gun.
/// Our choice, not the files': the Python, M14 and Olympia (and the other guns
/// the load report lists) name the flash other players see as their
/// first-person flash too, and its fireballs grow to 23-48 units around a
/// barrel 25 units from the eye, filling the screen. At a fifth, the Python's
/// peaks on screen about as big as the Pack-a-Punched Python's own
/// first-person flash (measured side by side).
pub const T6_FIRST_PERSON_FLASH_SCALE: f32 = 0.2;

/// The name a far-view flash's first-person copy is stored under.
pub fn t6_first_person_flash_name(name: &str) -> String {
    format!("{name}#1p")
}

/// Scales every f32 in `bytes` (little-endian, 4 bytes each) by `scale`.
fn scale_floats(bytes: &mut [u8], scale: f32) {
    for word in bytes.chunks_exact_mut(4) {
        let value = f32::from_le_bytes([word[0], word[1], word[2], word[3]]) * scale;
        word.copy_from_slice(&value.to_le_bytes());
    }
}

/// The first-person copy of one element: sizes, offsets, speeds and reach times
/// `scale`, drawn with the gun. Lights keep their reach, and parts the effect
/// already keeps off the eye (a distance fade: its muzzle smoke) stay as they
/// are.
fn t6_first_person_elem(el: &OwnedFxElemDef, scale: f32) -> OwnedFxElemDef {
    let mut out = el.clone();
    if matches!(
        el.view.elem_type,
        elem_type::OMNI_LIGHT | elem_type::SPOT_LIGHT | elem_type::SOUND | elem_type::DECAL | elem_type::RUNNER
    ) {
        return out;
    }
    if el.view.fade_in_range[1] != 0.0 || el.view.fade_out_range[1] != 0.0 {
        return out;
    }
    let view = &mut out.view;
    view.flags |= T6_FX_ELEM_DRAW_WITH_VIEWMODEL;
    for axis in &mut view.spawn_origin {
        axis[0] *= scale;
        axis[1] *= scale;
    }
    for reach in [
        &mut view.spawn_offset_radius_base,
        &mut view.spawn_offset_radius_amplitude,
        &mut view.spawn_offset_height_base,
        &mut view.spawn_offset_height_amplitude,
        &mut view.gravity_base,
        &mut view.gravity_amplitude,
        &mut view.spawn_frustum_cull_radius,
    ] {
        *reach *= scale;
    }
    for edge in view.coll_mins.iter_mut().chain(view.coll_maxs.iter_mut()) {
        *edge *= scale;
    }
    for dist in view.emit_dist.iter_mut().chain(view.emit_dist_variance.iter_mut()) {
        *dist *= scale;
    }
    // A visual sample holds a base then an amplitude, each: colour (4 bytes),
    // rotation delta, rotation total, size[2], scale. The rotation is an
    // angle; the two sizes and the scale are lengths.
    let half = FxElemVisStateSample::SIZE / 2;
    for sample in out.vis_samples.chunks_exact_mut(FxElemVisStateSample::SIZE) {
        for at in [12, 16, 20].into_iter().flat_map(|field| [field, half + field]) {
            scale_floats(&mut sample[at..at + 4], scale);
        }
    }
    // Every velocity sample value is a distance (local velocity, world
    // velocity, total delta).
    scale_floats(&mut out.vel_samples, scale);
    let vel_count = usize::from(out.view.vel_interval_count) + 1;
    out.vel_graph_local = parse_vel_graph_channel(&out.vel_samples, vel_count, false);
    out.vel_graph_world = parse_vel_graph_channel(&out.vel_samples, vel_count, true);
    out
}

impl FxCatalog {
    /// For a flash a gun names for both views (the one other players see), adds
    /// its first-person copy (`t6_first_person_flash_name`). Returns the
    /// largest flame size before and after; `None` when the effect is not
    /// here, already has parts drawn with the gun, or plays other effects.
    pub fn add_t6_first_person_flash(&mut self, name: &str) -> Option<(f32, f32)> {
        let def = self.get_in(AssetNamespace::T6, name)?;
        let drawn = |elem: &OwnedFxElemDef| elem.view.elem_type <= elem_type::MODEL;
        let already_first_person = |elem: &OwnedFxElemDef| {
            elem.view.elem_type == elem_type::RUNNER
                || (drawn(elem) && elem.view.flags & T6_FX_ELEM_DRAW_WITH_VIEWMODEL != 0)
        };
        if def.elems.iter().any(already_first_person) {
            return None;
        }
        let mut copy = def.clone();
        copy.name = t6_first_person_flash_name(name);
        copy.elems = def
            .elems
            .iter()
            .map(|elem| t6_first_person_elem(elem, T6_FIRST_PERSON_FLASH_SCALE))
            .collect();
        // The largest flame (the base size of any visual sample) among the
        // parts the copy draws with the gun, before and after.
        let scaled: Vec<usize> = (0..copy.elems.len())
            .filter(|&at| drawn(&copy.elems[at]) && copy.elems[at].view.flags & T6_FX_ELEM_DRAW_WITH_VIEWMODEL != 0)
            .collect();
        let largest = |elems: &[OwnedFxElemDef]| {
            scaled
                .iter()
                .flat_map(|&at| elems[at].vis_samples.chunks_exact(FxElemVisStateSample::SIZE))
                .map(|sample| f32::from_le_bytes([sample[12], sample[13], sample[14], sample[15]]).abs())
                .fold(0.0f32, f32::max)
        };
        let before = largest(&def.elems);
        let after = largest(&copy.elems);
        self.insert_owned(copy);
        Some((before, after))
    }

    /// Reads one effect in. False for a reference stub (a comma-named asset
    /// another zone defines) or an unnamed one.
    pub fn capture_t6(&mut self, fx: &asset_t6::FxEffectRef) -> bool {
        if fx.name.is_empty() || fx.name.starts_with(',') {
            return false;
        }
        let view = FxEffectDefView {
            flags: i32::from(fx.flags),
            msec_looping_life: fx.msec_looping_life,
            looping_count: i32::from(fx.looping_count),
            one_shot_count: i32::from(fx.one_shot_count),
            emission_count: i32::from(fx.emission_count),
        };
        self.insert_owned(OwnedFxEffectDef {
            namespace: AssetNamespace::T6,
            name: fx.name.clone(),
            view,
            elems: fx.elems.iter().map(t6_elem).collect(),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_element_types_are_renumbered_for_the_runtime() {
        assert_eq!(t6_elem_type(0), elem_type::BILLBOARD);
        assert_eq!(t6_elem_type(2), elem_type::ORIENTED);
        assert_eq!(t6_elem_type(4), elem_type::TAIL);
        assert_eq!(t6_elem_type(5), elem_type::TRAIL);
        assert_eq!(t6_elem_type(6), elem_type::CLOUD);
        assert_eq!(t6_elem_type(11), elem_type::DECAL);
        assert_eq!(t6_elem_type(12), elem_type::RUNNER);
    }

    #[test]
    fn a_velocity_graph_is_read_one_frame_at_a_time() {
        let mut samples = vec![0u8; 0x60 * 2];
        let mut put = |at: usize, value: f32| samples[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0, 1.0);
        put(12, 0.5);
        put(0x30 + 8, 7.0);
        put(0x60 + 4, 2.0);
        let local = parse_vel_graph_channel(&samples, 2, false);
        let world = parse_vel_graph_channel(&samples, 2, true);
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].base, [1.0, 0.0, 0.0]);
        assert_eq!(local[0].amplitude, [0.5, 0.0, 0.0]);
        assert_eq!(local[1].base, [0.0, 2.0, 0.0]);
        assert_eq!(world[0].base, [0.0, 0.0, 7.0]);
        // A short buffer gives as many fenceposts as it holds.
        assert_eq!(parse_vel_graph_channel(&samples[..0x60 + 20], 2, false).len(), 1);
    }

    #[test]
    fn the_first_person_copy_is_named_after_the_flash_and_scales_every_float() {
        let mut bytes = Vec::new();
        for value in [1.0f32, 2.0, 3.0] {
            bytes.extend(value.to_le_bytes());
        }
        scale_floats(&mut bytes, 0.5);
        let scaled: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        assert_eq!(scaled, [0.5, 1.0, 1.5]);
        assert_eq!(t6_first_person_flash_name("muzzle"), "muzzle#1p");
    }
}
