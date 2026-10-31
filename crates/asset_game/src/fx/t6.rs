//! Black Ops II effects, read into the effect catalog.
//!
//! `asset_t6` hands an element over as its raw bytes plus the names it points
//! at. The velocity and visual samples are copied as stored; the element type
//! is renumbered to the one the effect runtime plays by, and the flag bits are
//! kept as the zone has them.

use fastfile_t6::layout::{FxElemDef, FxElemVelStateSample, FxElemVisStateSample};
use fx_t6::runtime::{EmitterRecipeLens, ParticleRecipeLens, Vec3Spread, StreakCorner};

use super::definitions::{EffectRoster, SelfContainedEffectDef};
use super::look_kind;
use super::elem_look::{SelfContainedEffectElement, SelfContainedTrailDef, SelfContainedEffectLook, pending};
use crate::AssetCategoryScope;

/// The runtime's element type for the number a zone stores.
pub fn t6_elem_type(raw: u8) -> u8 {
    match raw {
        0 => look_kind::ALWAYS_FACING,
        1 | 2 => look_kind::ROTATED_BOX,
        3 | 4 => look_kind::STREAK_END,
        5 => look_kind::STREAK_BODY,
        6 => look_kind::PUFF,
        code => code,
    }
}

fn t6_elem_view(el: &asset_t6::FxElemRef) -> (ParticleRecipeLens, [u8; 8]) {
    let f = |off: usize| el.float_from_bytes(off);
    let i = |off: usize| el.int_from_bytes(off);
    let range = |off: usize| [f(off), f(off + 4)];
    let b = |off: usize| el.raw.get(off).copied().unwrap_or(0);
    let mut atlas = [0u8; 8];
    if let Some(src) = el.raw.get(FxElemDef::atlas..FxElemDef::atlas + 8) {
        atlas.copy_from_slice(src);
    }
    // The zone packs the entry count with an index range into the last two
    // bytes (count in the low byte: 516 is 4 entries on a 2x2 atlas); the
    // element keeps a plain 16-bit count there.
    let count = u16::from(atlas[6]).max(1);
    atlas[6..8].copy_from_slice(&count.to_le_bytes());
    let view = ParticleRecipeLens {
        flags: i(FxElemDef::flags),
        birth_corner_a: i(FxElemDef::spawn),
        birth_corner_b: i(FxElemDef::spawn + 4),
        birth_span_floor: f(FxElemDef::spawnRange),
        birth_span_swing: f(FxElemDef::spawnRange + 4),
        appear_blend_reach: range(FxElemDef::fadeInRange),
        vanish_blend_reach: range(FxElemDef::fadeOutRange),
        birth_view_reject_reach: f(FxElemDef::spawnFrustumCullRadius),
        birth_wait_floor_ms: i(FxElemDef::spawnDelayMsec),
        birth_wait_swing_ms: i(FxElemDef::spawnDelayMsec + 4),
        lifespan_window_floor: i(FxElemDef::lifeSpanMsec),
        lifespan_window_swing: i(FxElemDef::lifeSpanMsec + 4),
        birth_point: core::array::from_fn(|a| range(FxElemDef::spawnOrigin + a * 8)),
        birth_ring_reach_floor: f(FxElemDef::spawnOffsetRadius),
        birth_ring_reach_swing: f(FxElemDef::spawnOffsetRadius + 4),
        birth_lift_floor: f(FxElemDef::spawnOffsetHeight),
        birth_lift_swing: f(FxElemDef::spawnOffsetHeight + 4),
        birth_orientation: core::array::from_fn(|a| range(FxElemDef::spawnAngles + a * 8)),
        twirl_rate: core::array::from_fn(|a| range(FxElemDef::angularVelocity + a * 8)),
        birth_spin_angle: range(FxElemDef::initialRotation),
        fall_pull_floor: f(FxElemDef::gravity),
        fall_pull_swing: f(FxElemDef::gravity + 4),
        mirror_strength: range(FxElemDef::reflectionFactor),
        bounce_box_bottom: core::array::from_fn(|a| f(FxElemDef::collMins + a * 4)),
        bounce_box_top: core::array::from_fn(|a| f(FxElemDef::collMaxs + a * 4)),
        shape_kind: t6_elem_type(b(FxElemDef::elemType)),
        look_variant_total: b(FxElemDef::visualCount),
        speed_keyframe_total: b(FxElemDef::velIntervalCount),
        look_keyframe_total: b(FxElemDef::visStateIntervalCount),
        shading_blend: b(FxElemDef::lightingFrac),
        collide_with_pickups: 0,
        line: b(FxElemDef::elemType) == 4,
        paint_rank: b(FxElemDef::sortOrder),
        emission_spacing: range(FxElemDef::emitDist),
        emission_spacing_spread: range(FxElemDef::emitDistVariance),
    };
    (view, atlas)
}

fn t6_visuals(el: &asset_t6::FxElemRef, view: &ParticleRecipeLens) -> Vec<SelfContainedEffectLook> {
    let lit = [look_kind::POINT_EMITTER, look_kind::CONE_EMITTER].contains(&view.shape_kind);
    if lit || el.visuals.first().is_none() {
        return Vec::from([SelfContainedEffectLook::None]);
    }
    el.visuals
        .iter()
        .map(|visual| match visual {
            asset_t6::FxVisualRef::Material(name) => SelfContainedEffectLook::material(name),
            asset_t6::FxVisualRef::Mark([first, second]) => SelfContainedEffectLook::mark(first, second),
            asset_t6::FxVisualRef::Model(name) => SelfContainedEffectLook::mdl(name),
            asset_t6::FxVisualRef::Effect(name) => SelfContainedEffectLook::runner(name),
            asset_t6::FxVisualRef::Sound(name) => SelfContainedEffectLook::sound(name),
            asset_t6::FxVisualRef::Light(_) | asset_t6::FxVisualRef::None => SelfContainedEffectLook::None,
        })
        .collect()
}

/// bo2zm: a trail's split distance as its inverse. Some of Black Ops II's
/// trails hold a whole number in that field (read as a float it is a tiny
/// subnormal: 1 / it was infinite, and one blood trail on a flying limb
/// asked for billions of samples in a frame): those read as the integer.
fn t6_inv_split_dist(split: f32) -> f32 {
    let dist = if split != 0.0 && !split.is_normal() {
        split.to_bits() as f32
    } else {
        split
    };
    static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if dist != split && !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
        diag::info!(World, "zm fx trail: split-distance bits {:#x} taken as {dist}", split.to_bits());
    }
    if dist > 0.0 && dist.is_finite() { 1.0 / dist } else { 0.0 }
}

fn t6_trail(el: &asset_t6::FxElemRef) -> Option<SelfContainedTrailDef> {
    let trail = el.trail.as_ref()?;
    // A split distance of (almost) nothing means no splitting by distance: the
    // Warthog's contrail stores a denormal, whose inverse is infinite and asked
    // the trail for billions of pieces a frame.
    let reciprocal_split_distance = t6_inv_split_dist(trail.split_dist);
    Some(SelfContainedTrailDef {
        scroll_period_millis: trail.scroll_period_millis,
        recurrence_distance: trail.recurrence_distance,
        reciprocal_split_distance,
        reciprocal_split_arc_length: 0.0,
        reciprocal_split_duration: 0.0,
        vertex_points: trail
            .vertex_points
            .iter()
            .map(|v| StreakCorner {
                spot: [v[0], v[1]],
                normal: [v[2], v[3]],
                sheet_v: v[4],
            })
            .collect(),
        index_run_values: trail.index_run_values.clone(),
    })
}

/// The velocity graph of one frame of reference: each fencepost's base and
/// amplitude. A velocity sample holds the local graph first, then the world
/// graph half a stride along.
fn parse_vel_graph_channel(samples: &[u8], knot_n: usize, world: bool) -> Vec<Vec3Spread> {
    const POST_LEN: usize = 96;
    const WORLD_AT: usize = POST_LEN / 2;
    let first = [0, WORLD_AT][usize::from(world)];
    samples
        .chunks(POST_LEN)
        .take(knot_n)
        .map_while(|post| {
            let graph = post.get(first..first + 24)?;
            let word = |n: usize| f32::from_le_bytes(graph[n * 4..n * 4 + 4].try_into().unwrap());
            Some(Vec3Spread {
                base: [word(0), word(1), word(2)],
                amplitude: [word(3), word(4), word(5)],
            })
        })
        .collect()
}

fn t6_elem(el: &asset_t6::FxElemRef) -> SelfContainedEffectElement {
    let (view, atlas) = t6_elem_view(el);
    let vel_count = usize::from(view.speed_keyframe_total) + 1;
    let speed_readings = el
        .speed_readings
        .get(..vel_count * FxElemVelStateSample::SIZE)
        .unwrap_or(&el.speed_readings)
        .to_vec();
    let vis_count = usize::from(view.look_keyframe_total) + 1;
    let mut visibility_readings = el
        .visibility_readings
        .get(..vis_count * FxElemVisStateSample::SIZE)
        .unwrap_or(&el.visibility_readings)
        .to_vec();
    // A sample stores each colour red first; the draw wants blue first (flames
    // drew blue-white and smoke dark red until they were swapped).
    for sample in visibility_readings.chunks_exact_mut(FxElemVisStateSample::SIZE) {
        for half in [0, FxElemVisStateSample::SIZE / 2] {
            sample.swap(half, half + 2);
        }
    }
    // The effects spawned along the way are bound once the catalog is full.
    let (effect_upon_strike, on_impact_name) = pending(&el.effect_upon_strike);
    let (effect_upon_demise, on_death_name) = pending(&el.effect_upon_demise);
    let (effect_spawned, emitted_name) = pending(&el.effect_spawned);
    SelfContainedEffectElement {
        vel_graph_local: parse_vel_graph_channel(&speed_readings, vel_count, false),
        vel_graph_world: parse_vel_graph_channel(&speed_readings, vel_count, true),
        visuals: t6_visuals(el, &view),
        trail_kind: if view.shape_kind == look_kind::STREAK_BODY { t6_trail(el) } else { None },
        // A cloud stores how many particles it holds in the place other
        // elements keep their shape bits: a base, and a random extra on top
        // (512 and up to 512 more on Nuketown's ash and ember clouds).
        cloud_density: (view.shape_kind == look_kind::PUFF).then(|| [el.int_from_bytes(FxElemDef::u), el.int_from_bytes(FxElemDef::u + 4)]),
        atlas,
        view,
        speed_readings,
        visibility_readings,
        effect_upon_strike,
        effect_upon_demise,
        effect_spawned,
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

/// Scales every f32 in `bytes` (little-endian, 4 bytes each) by `factor`.
fn scale_floats(bytes: &mut [u8], factor: f32) {
    for word in bytes.chunks_exact_mut(4) {
        let value = f32::from_le_bytes([word[0], word[1], word[2], word[3]]) * factor;
        word.copy_from_slice(&value.to_le_bytes());
    }
}

/// The first-person copy of one element: sizes, offsets, speeds and reach times
/// `scale`, drawn with the gun. Lights keep their reach, and parts the effect
/// already keeps off the eye (a distance fade: its muzzle smoke) stay as they
/// are.
fn t6_first_person_elem(el: &SelfContainedEffectElement, scale: f32) -> SelfContainedEffectElement {
    let mut out = el.clone();
    if matches!(
        el.view.shape_kind,
        look_kind::POINT_EMITTER | look_kind::CONE_EMITTER | look_kind::AUDIO_KIND | look_kind::SURFACE_STICKER | look_kind::SPRINTER
    ) {
        return out;
    }
    if el.view.appear_blend_reach[1] != 0.0 || el.view.vanish_blend_reach[1] != 0.0 {
        return out;
    }
    let view = &mut out.view;
    view.flags |= T6_FX_ELEM_DRAW_WITH_VIEWMODEL;
    for axis in &mut view.birth_point {
        axis[0] *= scale;
        axis[1] *= scale;
    }
    for reach in [
        &mut view.birth_ring_reach_floor,
        &mut view.birth_ring_reach_swing,
        &mut view.birth_lift_floor,
        &mut view.birth_lift_swing,
        &mut view.fall_pull_floor,
        &mut view.fall_pull_swing,
        &mut view.birth_view_reject_reach,
    ] {
        *reach *= scale;
    }
    for edge in view.bounce_box_bottom.iter_mut().chain(view.bounce_box_top.iter_mut()) {
        *edge *= scale;
    }
    for dist in view.emission_spacing.iter_mut().chain(view.emission_spacing_spread.iter_mut()) {
        *dist *= scale;
    }
    // A visual sample holds a base then an amplitude, each: colour (4 bytes),
    // rotation delta, rotation total, size[2], scale. The rotation is an
    // angle; the two sizes and the scale are lengths.
    let half = FxElemVisStateSample::SIZE / 2;
    for sample in out.visibility_readings.chunks_exact_mut(FxElemVisStateSample::SIZE) {
        for at in [12, 16, 20].into_iter().flat_map(|field| [field, half + field]) {
            scale_floats(&mut sample[at..at + 4], scale);
        }
    }
    // Every velocity sample value is a distance (local velocity, world
    // velocity, total delta).
    scale_floats(&mut out.speed_readings, scale);
    let vel_count = usize::from(out.view.speed_keyframe_total) + 1;
    out.vel_graph_local = parse_vel_graph_channel(&out.speed_readings, vel_count, false);
    out.vel_graph_world = parse_vel_graph_channel(&out.speed_readings, vel_count, true);
    out
}

impl EffectRoster {
    /// For a flash a gun names for both views (the one other players see), adds
    /// its first-person copy (`t6_first_person_flash_name`). Returns the
    /// largest flame size before and after; `None` when the effect is not
    /// here, already has parts drawn with the gun, or plays other effects.
    pub fn add_t6_first_person_flash(&mut self, name: &str) -> Option<(f32, f32)> {
        let def = self.get_in(AssetCategoryScope::T6, name)?;
        let drawn = |elem: &SelfContainedEffectElement| elem.view.shape_kind <= look_kind::MESH_KIND;
        let already_first_person = |elem: &SelfContainedEffectElement| {
            elem.view.shape_kind == look_kind::SPRINTER
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
        let largest = |elems: &[SelfContainedEffectElement]| {
            scaled
                .iter()
                .flat_map(|&at| elems[at].visibility_readings.chunks_exact(FxElemVisStateSample::SIZE))
                .map(|sample| f32::from_le_bytes([sample[12], sample[13], sample[14], sample[15]]).abs())
                .fold(0.0f32, f32::max)
        };
        let before = largest(&def.elems);
        let after = largest(&copy.elems);
        self.add_owned_fx(copy);
        Some((before, after))
    }

    /// Reads one effect in. False for a reference stub (a comma-named asset
    /// another zone defines) or an unnamed one.
    pub fn capture_t6(&mut self, fx: &asset_t6::FxEffectRef) -> bool {
        if fx.name.is_empty() || fx.name.starts_with(',') {
            return false;
        }
        let view = EmitterRecipeLens {
            flags: i32::from(fx.flags),
            loop_lifetime_millis: fx.loop_lifetime_millis,
            repeat_total: i32::from(fx.repeat_total),
            single_play_total: i32::from(fx.single_play_total),
            spawn_total: i32::from(fx.spawn_total),
        };
        self.add_owned_fx(SelfContainedEffectDef {
            namespace: AssetCategoryScope::T6,
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
        assert_eq!(t6_elem_type(0), look_kind::ALWAYS_FACING);
        assert_eq!(t6_elem_type(2), look_kind::ROTATED_BOX);
        assert_eq!(t6_elem_type(4), look_kind::STREAK_END);
        assert_eq!(t6_elem_type(5), look_kind::STREAK_BODY);
        assert_eq!(t6_elem_type(6), look_kind::PUFF);
        assert_eq!(t6_elem_type(11), look_kind::SURFACE_STICKER);
        assert_eq!(t6_elem_type(12), look_kind::SPRINTER);
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
        let mut bytes: Vec<u8> = [1.0f32, 2.0, 3.0].into_iter().flat_map(f32::to_le_bytes).collect();
        scale_floats(&mut bytes, 0.5);
        let scaled: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        assert_eq!(scaled, [0.5, 1.0, 1.5]);
        assert_eq!(t6_first_person_flash_name("muzzle"), "muzzle#1p");
    }
}
