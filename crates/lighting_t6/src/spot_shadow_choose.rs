//! Picking which lights cast a spot shadow this frame.
//!
//! At most four lights get a shadow tile. Every time the scene clock moves,
//! each light in use is scored by how much it should matter to the viewer, the
//! best few are offered to a short history, and the history decides what is
//! drawn: lights fade in when they first win a place and fade out when they
//! lose it, so a shadow never pops.

use crate::scene_light::GFX_LIGHT_TYPE_DIR;
use crate::spot_shadow::{
    SpotShadowSlotPlan, spot_shadow_slot_plan, spot_shadow_take_slot, spot_shadow_tile,
};
use crate::spot_shadow_view::spot_shadow_lookup;

/// Shadow-casting spot lights per frame: the atlas has four tiles.
pub const SPOT_SHADOW_SM_LIGHT_CAP: usize = 4;

/// Weights that turn a light colour into one brightness.
pub const SPOT_SHADOW_SCORE_LUMA: [f32; 3] = [0.2989, 0.587, 0.114];

/// sm_sunEnable when the dvar is not set.
pub const SM_SUN_ENABLE_DEFAULT: bool = true;

/// sm_enable when the dvar is not set.
const SM_ENABLE_DEFAULT: bool = true;

/// Light type that is never dropped for distance or volume.
const CULL_EXEMPT_TYPE: u8 = 6;

/// A light the map marks as emissive outranks every scored light.
const EMISSIVE_SCORE: f32 = 100_000_000.0;

/// A fading light is dropped once its fade falls below this.
const FADE_DROP: f32 = 0.01;

const MILLISECONDS_TO_SECONDS: f32 = 0.001;

/// A negative clock step is a wrapped one: add one full turn of the counter.
const CLOCK_WRAP: f32 = 4_294_967_296.0;

/// Lights that can be tracked: one bit each in a 256-bit set.
const USED_WORDS: usize = 8;

/// A light, as the choice sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowableLight {
    pub can_use_shadow_map: bool,
    pub color: [f32; 3],
    pub dir: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub light_type: u8,
    pub cos_half_fov_expanded: f32,
    /// A viewer farther than this from the light drops it.
    pub cull_dist: i16,
    /// The light has a shadow map volume and the viewer is outside it.
    pub view_outside_shadowmap_volume: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpotShadowHistoryEntry {
    pub light_index: u8,
    pub fading_out: bool,
    pub score: f32,
    pub fade: f32,
}

/// What was chosen on earlier frames, best score first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpotShadowHistory {
    /// Lights in use at the last update.
    pub was_used: [u32; USED_WORDS],
    pub entries: [SpotShadowHistoryEntry; SPOT_SHADOW_SM_LIGHT_CAP],
    pub entry_count: u32,
    pub last_update_time: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowChooseDvars {
    pub spot_enable: bool,
    pub max_lights: u32,
    pub fade_time: f32,
    pub eye_project_dist: f32,
    pub spot_project_frac: f32,
    pub spot_quality: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowEmit {
    pub light_index: u8,
    pub fade: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotShadowChoose {
    /// The lights to draw, in tile order.
    pub emit: [Option<SpotShadowEmit>; SPOT_SHADOW_SM_LIGHT_CAP],
    /// Lights that have a shadow map this frame: the sun and the emitted ones.
    pub has_shadow_map: [u32; USED_WORDS],
    pub tile: u32,
}

fn used_bit(words: &[u32; USED_WORDS], index: u32) -> bool {
    words
        .get((index >> 5) as usize)
        .is_some_and(|word| word & (1 << (index & 31)) != 0)
}

pub fn set_used_bit(words: &mut [u32; USED_WORDS], index: u32) {
    if let Some(word) = words.get_mut((index >> 5) as usize) {
        *word |= 1 << (index & 31);
    }
}

fn clear_used_bit(words: &mut [u32; USED_WORDS], index: u32) {
    if let Some(word) = words.get_mut((index >> 5) as usize) {
        *word &= !(1 << (index & 31));
    }
}

/// Whether shadow choosing runs at all: sm_enable allows it (on when unset)
/// and the view has not turned shadows off.
pub fn generate_runs_choose(sm_enable: Option<bool>, view_allows_shadows: Option<bool>) -> bool {
    sm_enable.unwrap_or(SM_ENABLE_DEFAULT) && view_allows_shadows != Some(false)
}

/// With the sun's shadow switched off, the sun lights count as unused.
pub fn generate_clears_used_bits_through_sun(sm_sun_enable: bool, sun_primary: u32) -> bool {
    !sm_sun_enable && sun_primary != 0
}

/// Marks lights 1 through the sun as unused.
pub fn clear_used_bits_1_through_sun(used: &mut [u32; USED_WORDS], sun_primary: u32) {
    for index in 1..=sun_primary {
        clear_used_bit(used, index);
    }
}

/// How far the fade moves for a clock step, or nothing when fading is off
/// (a fade time of zero).
fn fade_delta(time_delta: i32, fade_time: f32) -> Option<f32> {
    if fade_time == 0.0 {
        return None;
    }
    let mut elapsed = time_delta as f32;
    if time_delta < 0 {
        elapsed += CLOCK_WRAP;
    }
    Some(elapsed * (MILLISECONDS_TO_SECONDS / fade_time))
}

/// Where the viewer is looking from and which way, for scoring.
#[derive(Clone, Copy)]
struct Eye {
    origin: [f32; 3],
    forward: [f32; 3],
}

/// Brightness times reach over distance. Distance is measured from a point
/// ahead of the viewer to a point inside the light's cone.
fn score(
    eye: Eye,
    light: &SpotShadowableLight,
    dvars: &SpotShadowChooseDvars,
    luma: [f32; 3],
) -> f32 {
    let look_at = [
        eye.origin[0] + dvars.eye_project_dist * eye.forward[0],
        eye.origin[1] + dvars.eye_project_dist * eye.forward[1],
        eye.origin[2] + dvars.eye_project_dist * eye.forward[2],
    ];
    let along = -light.radius * dvars.spot_project_frac;
    let dx = (light.origin[0] - look_at[0]) + along * light.dir[0];
    let dy = (light.origin[1] - look_at[1]) + along * light.dir[1];
    let dz = (light.origin[2] - look_at[2]) + along * light.dir[2];
    let distance = libm::sqrtf(dx * dx + dy * dy + dz * dz);
    let brightness = light.color[0] * luma[0] + light.color[1] * luma[1] + light.color[2] * luma[2];
    (brightness * light.radius) / (distance + 1.0)
}

/// Whether the viewer is too far from the light, or outside its volume. The
/// sun and the exempt type are never dropped.
fn is_culled(view_origin: [f32; 3], light: &SpotShadowableLight) -> bool {
    if light.light_type == GFX_LIGHT_TYPE_DIR || light.light_type == CULL_EXEMPT_TYPE {
        return false;
    }
    let dx = view_origin[0] - light.origin[0];
    let dy = view_origin[1] - light.origin[1];
    let dz = view_origin[2] - light.origin[2];
    let reach = f32::from(light.cull_dist);
    dx * dx + dy * dy + dz * dz > reach * reach || light.view_outside_shadowmap_volume
}

/// sm_maxLights caps the candidate list and the history, under the atlas's
/// four.
fn list_cap(max_lights: u32) -> usize {
    max_lights.min(SPOT_SHADOW_SM_LIGHT_CAP as u32) as usize
}

/// The best few lights of this update, highest score first.
struct Candidates {
    entries: [(u32, f32); SPOT_SHADOW_SM_LIGHT_CAP],
    len: usize,
    cap: usize,
}

impl Candidates {
    fn new(cap: usize) -> Self {
        Self {
            entries: [(0, 0.0); SPOT_SHADOW_SM_LIGHT_CAP],
            len: 0,
            cap,
        }
    }

    /// Slots the light in by score, pushing lower ones down and the last one
    /// off the end.
    fn offer(&mut self, light_index: u32, score: f32) {
        let mut at = self.len;
        while at != 0 && score > self.entries[at - 1].1 {
            if at < self.cap {
                self.entries[at] = self.entries[at - 1];
            }
            at -= 1;
        }
        if at >= self.cap {
            return;
        }
        self.entries[at] = (light_index, score);
        self.len = (self.len + 1).min(self.cap);
    }

    fn iter(&self) -> impl Iterator<Item = (u32, f32)> + '_ {
        self.entries.iter().take(self.len).copied()
    }
}

/// The score a light competes with this frame, or nothing when it is not a
/// candidate at all.
fn candidate_score(
    light_index: u32,
    sun_primary: u32,
    emissive_light: Option<u32>,
    light: &SpotShadowableLight,
    eye: Eye,
    dvars: &SpotShadowChooseDvars,
    luma: [f32; 3],
) -> Option<f32> {
    if light_index == 0
        || light_index <= sun_primary
        || is_culled(eye.origin, light)
        || !light.can_use_shadow_map
        || !dvars.spot_enable
    {
        return None;
    }
    if emissive_light == Some(light_index) {
        return Some(EMISSIVE_SCORE);
    }
    Some(score(eye, light, dvars, luma))
}

/// Ages the history: lights no longer in use are dropped, lights still in use
/// start fading out, and fading lights lose fade until they vanish. A dropped
/// entry is replaced by the last one.
fn fade_out_history(history: &mut SpotShadowHistory, used: &[u32; USED_WORDS], delta: f32) {
    let mut at = 0;
    while at < history.entry_count as usize {
        let entry = &mut history.entries[at];
        let drop_it = if !used_bit(used, u32::from(entry.light_index)) {
            true
        } else if !entry.fading_out {
            entry.fading_out = true;
            false
        } else {
            let faded = entry.fade - delta;
            entry.fade = faded;
            faded < FADE_DROP
        };
        if drop_it {
            history.entry_count -= 1;
            history.entries[at] = history.entries[history.entry_count as usize];
        } else {
            at += 1;
        }
    }
}

/// Moves an entry up while it outscores the one before it.
fn bubble_up(history: &mut SpotShadowHistory, mut at: usize) {
    while at != 0 && history.entries[at].score > history.entries[at - 1].score {
        history.entries.swap(at, at - 1);
        at -= 1;
    }
}

/// Offers a light to the history. A light already there is refreshed and
/// stops fading out; a new one needs a free place, and starts at full fade
/// unless it was already lit last update, when it eases in.
fn history_add(
    history: &mut SpotShadowHistory,
    light_index: u8,
    score: f32,
    delta: f32,
    max_lights: u32,
) {
    let count = history.entry_count as usize;
    if let Some(at) = history.entries[..count]
        .iter()
        .position(|entry| entry.light_index == light_index)
    {
        let entry = &mut history.entries[at];
        entry.score = score;
        entry.fading_out = false;
        entry.fade = (entry.fade + delta).min(1.0);
        bubble_up(history, at);
        return;
    }
    if count >= list_cap(max_lights) {
        return;
    }
    let eased = used_bit(&history.was_used, u32::from(light_index));
    history.entries[count] = SpotShadowHistoryEntry {
        light_index,
        fading_out: false,
        score,
        fade: if eased { delta } else { 1.0 },
    };
    history.entry_count += 1;
    bubble_up(history, count);
}

/// The lights to draw, in history order. Every light drawn also counts as
/// having a shadow map.
fn emit_from_history(
    history: &SpotShadowHistory,
    lights: &[SpotShadowableLight],
    shadowable_light_count: u32,
    has_shadow_map: &mut [u32; USED_WORDS],
) -> [Option<SpotShadowEmit>; SPOT_SHADOW_SM_LIGHT_CAP] {
    let mut emit = [None; SPOT_SHADOW_SM_LIGHT_CAP];
    let mut next = 0;
    let count = (history.entry_count as usize).min(SPOT_SHADOW_SM_LIGHT_CAP);
    for entry in &history.entries[..count] {
        let index = u32::from(entry.light_index);
        let usable = index < shadowable_light_count
            && lights
                .get(index as usize)
                .is_some_and(|light| light.can_use_shadow_map);
        if !usable {
            continue;
        }
        set_used_bit(has_shadow_map, index);
        emit[next] = Some(SpotShadowEmit {
            light_index: entry.light_index,
            fade: entry.fade,
        });
        next += 1;
    }
    emit
}

/// Visits the set bits of a word from the highest down.
fn bits_descending(mut word: u32) -> impl Iterator<Item = u32> {
    std::iter::from_fn(move || {
        if word == 0 {
            return None;
        }
        let bit = 31 - word.leading_zeros();
        word &= !(1 << bit);
        Some(bit)
    })
}

/// The per-frame pick. The history only moves when the scene clock does:
/// then every light in use is scored, the best `max_lights` are offered to the
/// history, and the history is aged. The sun never competes but always counts
/// as having a shadow map. Returns nothing when the clock moved and fading is
/// off (a fade time of zero).
#[allow(clippy::too_many_arguments)]
pub fn spot_shadow_choose(
    history: &mut SpotShadowHistory,
    used: [u32; USED_WORDS],
    lights: &[SpotShadowableLight],
    view_origin: [f32; 3],
    view_forward: [f32; 3],
    scene_time: u32,
    sun_primary_index: u32,
    shadowable_light_count: u32,
    emissive_light_index: Option<u32>,
    dvars: SpotShadowChooseDvars,
    luma: [f32; 3],
) -> Option<SpotShadowChoose> {
    let mut has_shadow_map = [0u32; USED_WORDS];
    for sun_bit in 1..=sun_primary_index.min(255) {
        set_used_bit(&mut has_shadow_map, sun_bit);
    }

    let time_delta = scene_time.wrapping_sub(history.last_update_time) as i32;
    if time_delta != 0 {
        let delta = fade_delta(time_delta, dvars.fade_time)?;
        history.last_update_time = scene_time;
        fade_out_history(history, &used, delta);

        let eye = Eye {
            origin: view_origin,
            forward: view_forward,
        };
        let mut candidates = Candidates::new(list_cap(dvars.max_lights));
        let words = (shadowable_light_count.saturating_add(31) >> 5) as usize;
        for (word_index, &word) in used.iter().enumerate().take(words) {
            for bit in bits_descending(word) {
                let index = bit + 32 * word_index as u32;
                let Some(light) = lights.get(index as usize) else {
                    continue;
                };
                let scored = candidate_score(
                    index,
                    sun_primary_index,
                    emissive_light_index,
                    light,
                    eye,
                    &dvars,
                    luma,
                );
                if let Some(scored) = scored {
                    candidates.offer(index, scored);
                }
            }
        }
        for (index, scored) in candidates.iter() {
            if let Ok(index) = u8::try_from(index) {
                history_add(history, index, scored, delta, dvars.max_lights);
            }
        }
        history.was_used = used;
    }

    let emit = emit_from_history(history, lights, shadowable_light_count, &mut has_shadow_map);
    Some(SpotShadowChoose {
        emit,
        has_shadow_map,
        tile: spot_shadow_tile(dvars.spot_quality),
    })
}

/// One drawn shadow: which light, where in the atlas, and how to look it up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowEmittedSlot {
    pub slot_index: u32,
    pub light_index: u8,
    pub fade: f32,
    pub plan: SpotShadowSlotPlan,
    /// Whether the light gathers its own casters.
    pub walks_casters: bool,
    /// The lookup matrix, when the light's view could be built.
    pub lookup: Option<[f32; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotShadowFrontend {
    pub shadowable_count: u32,
    pub slots: [Option<SpotShadowEmittedSlot>; SPOT_SHADOW_SM_LIGHT_CAP],
}

/// Gives each chosen light a tile and builds its lookup matrix. Lights below
/// `caster_light_count` gather their own casters and need no near bias; the
/// rest use the dvar's.
pub fn spot_shadow_emit_frontend(
    choose: &SpotShadowChoose,
    lights: &[SpotShadowableLight],
    caster_light_count: u32,
    near_bias_dvar: f32,
) -> SpotShadowFrontend {
    let mut shadowable_count = 0;
    let mut slots = [None; SPOT_SHADOW_SM_LIGHT_CAP];
    for (place, emit) in slots.iter_mut().zip(&choose.emit) {
        let Some(emit) = emit else {
            continue;
        };
        let Some(slot_index) = spot_shadow_take_slot(&mut shadowable_count) else {
            break;
        };
        let plan = spot_shadow_slot_plan(slot_index, choose.tile);
        let light_index = u32::from(emit.light_index);
        let walks_casters = light_index < caster_light_count;
        let near_bias = if walks_casters { 0.0 } else { near_bias_dvar };
        let lookup = lights.get(light_index as usize).and_then(|light| {
            spot_shadow_lookup(
                light.origin,
                light.dir,
                light.cos_half_fov_expanded,
                light.radius,
                near_bias,
                plan,
            )
        });
        *place = Some(SpotShadowEmittedSlot {
            slot_index,
            light_index: emit.light_index,
            fade: emit.fade,
            plan,
            walks_casters,
            lookup,
        });
    }
    SpotShadowFrontend {
        shadowable_count,
        slots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_light::GFX_LIGHT_TYPE_SPOT;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn light(origin: [f32; 3], radius: f32) -> SpotShadowableLight {
        SpotShadowableLight {
            can_use_shadow_map: true,
            color: [1.0; 3],
            dir: [0.0, 0.0, -1.0],
            origin,
            radius,
            light_type: GFX_LIGHT_TYPE_SPOT,
            cos_half_fov_expanded: 0.5,
            cull_dist: i16::MAX,
            view_outside_shadowmap_volume: false,
        }
    }

    fn dvars(max_lights: u32) -> SpotShadowChooseDvars {
        SpotShadowChooseDvars {
            spot_enable: true,
            max_lights,
            fade_time: 1.0,
            eye_project_dist: 0.0,
            spot_project_frac: 0.0,
            spot_quality: 0,
        }
    }

    fn eye() -> Eye {
        Eye {
            origin: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
        }
    }

    fn used_set(indices: &[u32]) -> [u32; USED_WORDS] {
        let mut words = [0; USED_WORDS];
        for &index in indices {
            set_used_bit(&mut words, index);
        }
        words
    }

    fn entry(light_index: u8, score: f32, fade: f32) -> SpotShadowHistoryEntry {
        SpotShadowHistoryEntry {
            light_index,
            fading_out: false,
            score,
            fade,
        }
    }

    fn history_of(entries: &[SpotShadowHistoryEntry]) -> SpotShadowHistory {
        let mut history = SpotShadowHistory::default();
        history.entries[..entries.len()].copy_from_slice(entries);
        history.entry_count = entries.len() as u32;
        history
    }

    fn history_lights(history: &SpotShadowHistory) -> Vec<u8> {
        history.entries[..history.entry_count as usize]
            .iter()
            .map(|entry| entry.light_index)
            .collect()
    }

    /// Runs a choose with the sun at light 0 and the default weights.
    fn choose_at(
        history: &mut SpotShadowHistory,
        used: [u32; USED_WORDS],
        lights: &[SpotShadowableLight],
        scene_time: u32,
        max_lights: u32,
    ) -> SpotShadowChoose {
        spot_shadow_choose(
            history,
            used,
            lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            scene_time,
            0,
            lights.len() as u32,
            None,
            dvars(max_lights),
            SPOT_SHADOW_SCORE_LUMA,
        )
        .unwrap()
    }

    fn emitted(choose: &SpotShadowChoose) -> Vec<u8> {
        choose
            .emit
            .iter()
            .flatten()
            .map(|emit| emit.light_index)
            .collect()
    }

    #[test]
    fn used_bits_ignore_indices_past_the_set() {
        let mut words = [0; USED_WORDS];
        set_used_bit(&mut words, 3);
        set_used_bit(&mut words, 255);
        set_used_bit(&mut words, 256);
        assert!(used_bit(&words, 3));
        assert!(used_bit(&words, 255));
        assert!(!used_bit(&words, 4));
        assert!(!used_bit(&words, 256));
        assert_eq!(words[0], 1 << 3);
        assert_eq!(words[7], 1 << 31);
    }

    #[test]
    fn a_cleared_bit_is_unused_and_neighbours_stay() {
        let mut words = used_set(&[3, 4, 40]);
        clear_used_bit(&mut words, 3);
        clear_used_bit(&mut words, 999);
        assert!(!used_bit(&words, 3));
        assert!(used_bit(&words, 4));
        assert!(used_bit(&words, 40));
    }

    #[test]
    fn choosing_runs_unless_shadows_are_off_for_the_view_or_the_dvar() {
        assert!(generate_runs_choose(None, None));
        assert!(generate_runs_choose(Some(true), Some(true)));
        assert!(!generate_runs_choose(Some(false), None));
        assert!(!generate_runs_choose(None, Some(false)));
    }

    #[test]
    fn the_sun_bits_are_cleared_only_with_the_sun_shadow_off() {
        assert!(generate_clears_used_bits_through_sun(false, 2));
        assert!(!generate_clears_used_bits_through_sun(true, 2));
        assert!(!generate_clears_used_bits_through_sun(false, 0));
    }

    #[test]
    fn clearing_through_the_sun_keeps_light_zero_and_the_rest() {
        let mut words = used_set(&[0, 1, 2, 3, 4]);
        clear_used_bits_1_through_sun(&mut words, 3);
        assert_eq!(words[0], 0b1_0001);
    }

    #[test]
    fn a_zero_fade_time_has_no_delta() {
        assert_eq!(fade_delta(100, 0.0), None);
    }

    #[test]
    fn the_fade_delta_is_the_elapsed_fraction_of_the_fade_time() {
        assert!(close(fade_delta(100, 1.0).unwrap(), 0.1));
        assert!(close(fade_delta(100, 0.5).unwrap(), 0.2));
        assert!(close(fade_delta(1000, 2.0).unwrap(), 0.5));
    }

    #[test]
    fn a_backward_clock_step_reads_as_a_wrapped_forward_one() {
        let delta = fade_delta(-1, 1.0).unwrap();
        assert!(delta > 4_000_000.0);
    }

    #[test]
    fn the_score_is_brightness_times_radius_over_distance_plus_one() {
        // White light: the weights sum to one. 99 away, so 100 / 100.
        let l = light([99.0, 0.0, 0.0], 100.0);
        let s = score(eye(), &l, &dvars(4), SPOT_SHADOW_SCORE_LUMA);
        assert!(close(s, 1.0));
    }

    #[test]
    fn a_closer_light_scores_higher() {
        let near = light([10.0, 0.0, 0.0], 100.0);
        let far = light([50.0, 0.0, 0.0], 100.0);
        let d = dvars(4);
        assert!(
            score(eye(), &near, &d, SPOT_SHADOW_SCORE_LUMA)
                > score(eye(), &far, &d, SPOT_SHADOW_SCORE_LUMA)
        );
    }

    #[test]
    fn a_brighter_light_scores_higher() {
        let mut dim = light([10.0, 0.0, 0.0], 100.0);
        dim.color = [0.1; 3];
        let bright = light([10.0, 0.0, 0.0], 100.0);
        let d = dvars(4);
        assert!(
            score(eye(), &bright, &d, SPOT_SHADOW_SCORE_LUMA)
                > score(eye(), &dim, &d, SPOT_SHADOW_SCORE_LUMA)
        );
    }

    #[test]
    fn green_counts_for_more_than_blue() {
        let mut green = light([10.0, 0.0, 0.0], 100.0);
        green.color = [0.0, 1.0, 0.0];
        let mut blue = green;
        blue.color = [0.0, 0.0, 1.0];
        let d = dvars(4);
        assert!(
            score(eye(), &green, &d, SPOT_SHADOW_SCORE_LUMA)
                > score(eye(), &blue, &d, SPOT_SHADOW_SCORE_LUMA)
        );
    }

    #[test]
    fn distance_is_measured_from_a_point_ahead_of_the_viewer() {
        let l = light([99.0, 0.0, 0.0], 100.0);
        let mut d = dvars(4);
        d.eye_project_dist = 99.0;
        // The look-at point is on the light: 100 * 1 / (0 + 1), less the
        // little the luma weights (which sum to 0.9999) take off.
        let got = score(eye(), &l, &d, SPOT_SHADOW_SCORE_LUMA);
        assert!((got - 100.0).abs() < 0.02, "{got}");
    }

    #[test]
    fn distance_is_measured_to_a_point_down_the_cone() {
        let l = light([0.0; 3], 100.0);
        let mut d = dvars(4);
        d.spot_project_frac = 0.5;
        // Half a radius against the direction (0, 0, -1) is 50 up: 100 / 51.
        assert!(close(
            score(eye(), &l, &d, SPOT_SHADOW_SCORE_LUMA),
            100.0 / 51.0
        ));
    }

    #[test]
    fn a_far_light_is_culled_and_a_light_at_the_edge_is_not() {
        let mut l = light([100.0, 0.0, 0.0], 50.0);
        l.cull_dist = 99;
        assert!(is_culled([0.0; 3], &l));
        l.cull_dist = 100;
        assert!(!is_culled([0.0; 3], &l));
    }

    #[test]
    fn a_light_whose_volume_the_viewer_is_outside_is_culled() {
        let mut l = light([10.0, 0.0, 0.0], 50.0);
        l.view_outside_shadowmap_volume = true;
        assert!(is_culled([0.0; 3], &l));
    }

    #[test]
    fn the_sun_and_the_exempt_type_are_never_culled() {
        let mut l = light([10_000.0, 0.0, 0.0], 50.0);
        l.cull_dist = 1;
        l.view_outside_shadowmap_volume = true;
        assert!(is_culled([0.0; 3], &l));
        l.light_type = GFX_LIGHT_TYPE_DIR;
        assert!(!is_culled([0.0; 3], &l));
        l.light_type = 6;
        assert!(!is_culled([0.0; 3], &l));
    }

    #[test]
    fn the_candidate_list_is_capped_by_max_lights_and_by_the_atlas() {
        assert_eq!(list_cap(0), 0);
        assert_eq!(list_cap(2), 2);
        assert_eq!(list_cap(4), 4);
        assert_eq!(list_cap(9), 4);
    }

    #[test]
    fn candidates_are_kept_best_first() {
        let mut list = Candidates::new(4);
        list.offer(1, 5.0);
        list.offer(2, 9.0);
        list.offer(3, 7.0);
        assert_eq!(
            list.iter().collect::<Vec<_>>(),
            [(2, 9.0), (3, 7.0), (1, 5.0)]
        );
    }

    #[test]
    fn a_full_candidate_list_drops_the_worst() {
        let mut list = Candidates::new(2);
        list.offer(1, 5.0);
        list.offer(2, 9.0);
        list.offer(3, 7.0);
        assert_eq!(list.iter().collect::<Vec<_>>(), [(2, 9.0), (3, 7.0)]);
        list.offer(4, 1.0);
        assert_eq!(list.iter().collect::<Vec<_>>(), [(2, 9.0), (3, 7.0)]);
    }

    #[test]
    fn a_candidate_list_of_zero_takes_nothing() {
        let mut list = Candidates::new(0);
        list.offer(1, 5.0);
        assert_eq!(list.iter().count(), 0);
    }

    #[test]
    fn the_sun_and_light_zero_are_not_candidates() {
        let l = light([10.0, 0.0, 0.0], 100.0);
        let d = dvars(4);
        let luma = SPOT_SHADOW_SCORE_LUMA;
        assert!(candidate_score(0, 0, None, &l, eye(), &d, luma).is_none());
        assert!(candidate_score(1, 1, None, &l, eye(), &d, luma).is_none());
        assert!(candidate_score(2, 1, None, &l, eye(), &d, luma).is_some());
    }

    #[test]
    fn a_light_that_cannot_use_a_shadow_map_is_not_a_candidate() {
        let mut l = light([10.0, 0.0, 0.0], 100.0);
        l.can_use_shadow_map = false;
        assert!(
            candidate_score(1, 0, None, &l, eye(), &dvars(4), SPOT_SHADOW_SCORE_LUMA).is_none()
        );
    }

    #[test]
    fn nothing_is_a_candidate_with_spot_shadows_off() {
        let l = light([10.0, 0.0, 0.0], 100.0);
        let mut d = dvars(4);
        d.spot_enable = false;
        assert!(candidate_score(1, 0, None, &l, eye(), &d, SPOT_SHADOW_SCORE_LUMA).is_none());
    }

    #[test]
    fn the_emissive_light_outranks_any_score() {
        let l = light([10.0, 0.0, 0.0], 100.0);
        let s = candidate_score(3, 0, Some(3), &l, eye(), &dvars(4), SPOT_SHADOW_SCORE_LUMA);
        assert_eq!(s, Some(EMISSIVE_SCORE));
        let other = candidate_score(2, 0, Some(3), &l, eye(), &dvars(4), SPOT_SHADOW_SCORE_LUMA);
        assert!(other.unwrap() < EMISSIVE_SCORE);
    }

    #[test]
    fn a_new_light_enters_the_history_at_full_fade() {
        let mut history = SpotShadowHistory::default();
        history_add(&mut history, 3, 5.0, 0.1, 4);
        assert_eq!(history.entry_count, 1);
        assert_eq!(history.entries[0], entry(3, 5.0, 1.0));
    }

    #[test]
    fn a_light_lit_last_update_eases_in() {
        let mut history = SpotShadowHistory::default();
        history.was_used = used_set(&[3]);
        history_add(&mut history, 3, 5.0, 0.1, 4);
        history_add(&mut history, 4, 5.0, 0.1, 4);
        assert!(close(history.entries[0].fade, 0.1));
        assert!(close(history.entries[1].fade, 1.0));
    }

    #[test]
    fn the_history_stays_sorted_best_first() {
        let mut history = SpotShadowHistory::default();
        history_add(&mut history, 1, 5.0, 0.1, 4);
        history_add(&mut history, 2, 9.0, 0.1, 4);
        history_add(&mut history, 3, 7.0, 0.1, 4);
        assert_eq!(history_lights(&history), [2, 3, 1]);
    }

    #[test]
    fn a_full_history_turns_a_new_light_away() {
        let mut history = SpotShadowHistory::default();
        history_add(&mut history, 1, 5.0, 0.1, 2);
        history_add(&mut history, 2, 9.0, 0.1, 2);
        history_add(&mut history, 3, 99.0, 0.1, 2);
        assert_eq!(history_lights(&history), [2, 1]);
    }

    #[test]
    fn a_light_already_in_the_history_is_refreshed() {
        let mut history = history_of(&[entry(1, 9.0, 0.5), entry(2, 5.0, 0.5)]);
        history.entries[1].fading_out = true;
        history_add(&mut history, 2, 20.0, 0.25, 4);
        // It outscores light 1 now and moves up, no longer fading out.
        assert_eq!(history_lights(&history), [2, 1]);
        assert_eq!(history.entries[0].score, 20.0);
        assert!(!history.entries[0].fading_out);
        assert!(close(history.entries[0].fade, 0.75));
    }

    #[test]
    fn a_refreshed_fade_stops_at_one() {
        let mut history = history_of(&[entry(1, 9.0, 0.9)]);
        history_add(&mut history, 1, 9.0, 0.5, 4);
        assert_eq!(history.entries[0].fade, 1.0);
    }

    #[test]
    fn a_light_no_longer_in_use_leaves_the_history() {
        let mut history = history_of(&[entry(1, 9.0, 1.0), entry(2, 5.0, 1.0), entry(3, 2.0, 1.0)]);
        fade_out_history(&mut history, &used_set(&[1, 3]), 0.1);
        // Light 2 is replaced by the last entry, light 3.
        assert_eq!(history_lights(&history), [1, 3]);
    }

    #[test]
    fn a_light_still_in_use_starts_fading_out_before_it_loses_fade() {
        let mut history = history_of(&[entry(1, 9.0, 1.0)]);
        fade_out_history(&mut history, &used_set(&[1]), 0.25);
        assert!(history.entries[0].fading_out);
        assert_eq!(history.entries[0].fade, 1.0);
        fade_out_history(&mut history, &used_set(&[1]), 0.25);
        assert!(close(history.entries[0].fade, 0.75));
    }

    #[test]
    fn a_fading_light_is_dropped_when_its_fade_runs_out() {
        let mut history = history_of(&[entry(1, 9.0, 0.2)]);
        history.entries[0].fading_out = true;
        fade_out_history(&mut history, &used_set(&[1]), 0.1);
        assert_eq!(history.entry_count, 1);
        fade_out_history(&mut history, &used_set(&[1]), 0.1);
        assert_eq!(history.entry_count, 0);
    }

    #[test]
    fn bits_are_visited_from_the_highest_down() {
        let visited: Vec<_> = bits_descending(0b1010_0101).collect();
        assert_eq!(visited, [7, 5, 2, 0]);
        assert_eq!(bits_descending(0).count(), 0);
        assert_eq!(bits_descending(1 << 31).collect::<Vec<_>>(), [31]);
    }

    #[test]
    fn nothing_is_chosen_until_the_scene_clock_moves() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = SpotShadowHistory::default();
        let choose = choose_at(&mut history, used_set(&[1]), &lights, 0, 4);
        assert!(emitted(&choose).is_empty());
        assert_eq!(history.entry_count, 0);
    }

    #[test]
    fn the_best_lights_are_chosen_in_score_order() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 50.0),
            light([10.0, 0.0, 0.0], 200.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = choose_at(&mut history, used_set(&[1, 2, 3]), &lights, 1000, 4);
        assert_eq!(emitted(&choose), [2, 3, 1]);
        assert!(choose.emit[0].unwrap().fade == 1.0);
        assert_eq!(history.last_update_time, 1000);
    }

    #[test]
    fn max_lights_limits_how_many_are_chosen() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 50.0),
            light([10.0, 0.0, 0.0], 200.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = choose_at(&mut history, used_set(&[1, 2, 3]), &lights, 1000, 2);
        assert_eq!(emitted(&choose), [2, 3]);
    }

    #[test]
    fn lights_not_in_use_are_not_chosen() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 500.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = choose_at(&mut history, used_set(&[2]), &lights, 1000, 4);
        assert_eq!(emitted(&choose), [2]);
    }

    #[test]
    fn the_sun_has_a_shadow_map_but_is_never_emitted() {
        let lights = [
            light([0.0; 3], 1.0),
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = spot_shadow_choose(
            &mut history,
            used_set(&[1, 2]),
            &lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            1000,
            1,
            3,
            None,
            dvars(4),
            SPOT_SHADOW_SCORE_LUMA,
        )
        .unwrap();
        assert_eq!(emitted(&choose), [2]);
        assert!(used_bit(&choose.has_shadow_map, 1));
        assert!(used_bit(&choose.has_shadow_map, 2));
        assert!(!used_bit(&choose.has_shadow_map, 0));
    }

    #[test]
    fn the_emissive_light_is_chosen_first() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 500.0),
            light([900.0, 0.0, 0.0], 1.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = spot_shadow_choose(
            &mut history,
            used_set(&[1, 2]),
            &lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            1000,
            0,
            3,
            Some(2),
            dvars(4),
            SPOT_SHADOW_SCORE_LUMA,
        )
        .unwrap();
        assert_eq!(emitted(&choose), [2, 1]);
    }

    #[test]
    fn a_light_that_cannot_use_a_shadow_map_is_never_emitted() {
        let mut lights = vec![light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = SpotShadowHistory::default();
        let choose = choose_at(&mut history, used_set(&[1]), &lights, 1000, 4);
        assert_eq!(emitted(&choose), [1]);
        // It loses the ability later: the history still holds it, the emit skips it.
        lights[1].can_use_shadow_map = false;
        let later = choose_at(&mut history, used_set(&[1]), &lights, 1000, 4);
        assert!(emitted(&later).is_empty());
        assert!(!used_bit(&later.has_shadow_map, 1));
    }

    #[test]
    fn a_light_beyond_the_shadowable_count_is_never_emitted() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 100.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = SpotShadowHistory::default();
        let choose = spot_shadow_choose(
            &mut history,
            used_set(&[1, 2]),
            &lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            1000,
            0,
            2,
            None,
            dvars(4),
            SPOT_SHADOW_SCORE_LUMA,
        )
        .unwrap();
        assert_eq!(emitted(&choose), [1]);
    }

    #[test]
    fn a_zero_fade_time_with_a_moving_clock_chooses_nothing() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = SpotShadowHistory::default();
        let mut d = dvars(4);
        d.fade_time = 0.0;
        let choose = spot_shadow_choose(
            &mut history,
            used_set(&[1]),
            &lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            1000,
            0,
            2,
            None,
            d,
            SPOT_SHADOW_SCORE_LUMA,
        );
        assert!(choose.is_none());
    }

    #[test]
    fn the_tile_follows_the_quality_setting() {
        let lights = [light([0.0; 3], 1.0)];
        let mut history = SpotShadowHistory::default();
        let mut d = dvars(4);
        d.spot_quality = 2;
        let choose = spot_shadow_choose(
            &mut history,
            [0; USED_WORDS],
            &lights,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            0,
            0,
            1,
            None,
            d,
            SPOT_SHADOW_SCORE_LUMA,
        )
        .unwrap();
        assert_eq!(choose.tile, 2048);
    }

    #[test]
    fn a_light_that_stops_being_used_is_gone_at_the_next_update() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = SpotShadowHistory::default();
        choose_at(&mut history, used_set(&[1]), &lights, 1000, 4);
        assert_eq!(history.entry_count, 1);
        // Still used and still a candidate: it stays, not fading.
        choose_at(&mut history, used_set(&[1]), &lights, 1100, 4);
        assert_eq!(history.entry_count, 1);
        assert!(!history.entries[0].fading_out);
        // No longer used: it is gone at the next update.
        let choose = choose_at(&mut history, [0; USED_WORDS], &lights, 1200, 4);
        assert!(emitted(&choose).is_empty());
        assert_eq!(history.entry_count, 0);
    }

    fn chosen(lights: &[SpotShadowableLight], indices: &[u8]) -> SpotShadowChoose {
        let mut emit = [None; SPOT_SHADOW_SM_LIGHT_CAP];
        for (place, &light_index) in emit.iter_mut().zip(indices) {
            *place = Some(SpotShadowEmit {
                light_index,
                fade: 0.5,
            });
        }
        assert!(lights.len() >= indices.len());
        SpotShadowChoose {
            emit,
            has_shadow_map: [0; USED_WORDS],
            tile: 512,
        }
    }

    #[test]
    fn each_chosen_light_gets_the_next_tile() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let frontend = spot_shadow_emit_frontend(&chosen(&lights, &[3, 1, 4]), &lights, 0, 0.0);
        assert_eq!(frontend.shadowable_count, 3);
        let slots: Vec<_> = frontend.slots.iter().flatten().collect();
        assert_eq!(slots.len(), 3);
        for (n, slot) in slots.iter().enumerate() {
            assert_eq!(slot.slot_index, n as u32);
            assert_eq!(slot.plan, spot_shadow_slot_plan(n as u32, 512));
            assert_eq!(slot.fade, 0.5);
        }
        assert_eq!(slots[0].light_index, 3);
        assert_eq!(slots[1].light_index, 1);
        assert!(slots[0].plan.clear && !slots[1].plan.clear);
        assert!(frontend.slots[3].is_none());
    }

    #[test]
    fn lights_below_the_caster_count_walk_their_own_casters() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let frontend = spot_shadow_emit_frontend(&chosen(&lights, &[1, 3]), &lights, 2, 0.0);
        assert!(frontend.slots[0].unwrap().walks_casters);
        assert!(!frontend.slots[1].unwrap().walks_casters);
    }

    #[test]
    fn only_lights_past_the_caster_count_take_the_near_bias() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let flat = spot_shadow_emit_frontend(&chosen(&lights, &[1, 3]), &lights, 2, 0.0);
        let biased = spot_shadow_emit_frontend(&chosen(&lights, &[1, 3]), &lights, 2, 8.0);
        assert_eq!(
            flat.slots[0].unwrap().lookup,
            biased.slots[0].unwrap().lookup
        );
        assert_ne!(
            flat.slots[1].unwrap().lookup,
            biased.slots[1].unwrap().lookup
        );
    }

    #[test]
    fn a_light_missing_from_the_list_gets_a_tile_but_no_lookup() {
        let lights = vec![light([0.0; 3], 100.0); 2];
        let frontend = spot_shadow_emit_frontend(&chosen(&lights, &[1]), &[], 0, 0.0);
        let slot = frontend.slots[0].unwrap();
        assert_eq!(slot.lookup, None);
        assert_eq!(frontend.shadowable_count, 1);
    }

    #[test]
    fn a_light_whose_view_cannot_be_built_has_no_lookup() {
        // A radius of 1 puts the far plane on the near plane.
        let lights = vec![light([0.0; 3], 1.0); 2];
        let frontend = spot_shadow_emit_frontend(&chosen(&lights, &[1]), &lights, 2, 0.0);
        assert_eq!(frontend.slots[0].unwrap().lookup, None);
    }

    #[test]
    fn an_empty_choice_emits_no_slots() {
        let lights = vec![light([0.0; 3], 100.0); 2];
        let frontend = spot_shadow_emit_frontend(&chosen(&lights, &[]), &lights, 0, 0.0);
        assert_eq!(frontend.shadowable_count, 0);
        assert!(frontend.slots.iter().all(Option::is_none));
    }
}
