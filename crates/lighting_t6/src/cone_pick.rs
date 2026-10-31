//! Picking which lights cast a spot shadow this frame.
//!
//! At most four lights get a shadow tile. Every time the scene clock moves,
//! each light in use is scored by how much it should matter to the viewer, the
//! best few are offered to a short history, and the history decides what is
//! drawn: lights fade in when they first win a place and fade out when they
//! lose it, so a shadow never pops.

use crate::stage_light::LAMP_KIND_PARALLEL;
use crate::cone_shadow::{
    ConeSlotPlan, plan_cone_slots, claim_cone_slot, spot_shadow_tile,
};
use crate::spot_shadow_view::spot_shadow_lookup;

/// Shadow-casting spot lights per frame: the atlas has four tiles.
pub const CONE_TILE_COUNT: usize = 4;

/// Weights that turn a light colour into one brightness.
pub const LAMP_LUMA_MIX: [f32; 3] = [0.2989, 0.587, 0.114];

/// sm_sunEnable when the dvar is not set.
pub const SUN_SHADOW_ENABLED_BY_DEFAULT: bool = true;

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
pub struct ConeCasterOffer {
    pub supports_depth_shade: bool,
    pub color: [f32; 3],
    pub aim: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub kind: u8,
    pub widened_half_view_cosine: f32,
    /// A viewer farther than this from the light drops it.
    pub hide_dist: i16,
    /// The light has a shadow map volume and the viewer is outside it.
    pub view_outside_shadowmap_volume: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeShadowChooseTunables {
    pub cone_enable: bool,
    pub light_budget: u32,
    pub fade_secs: f32,
    pub viewer_projection_reach: f32,
    pub cone_project_frac: f32,
    pub spot_quality: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowSubmission {
    pub shadow_light_slot: u8,
    pub fade_level: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotShadowSelector {
    /// The lights to draw, in tile order.
    pub emit: [Option<SpotShadowSubmission>; CONE_TILE_COUNT],
    /// Lights that have a shadow map this frame: the sun and the emitted ones.
    pub shadow_map_present: [u32; USED_WORDS],
    pub tile: u32,
}

/// Word and mask of a light in a used set, or nothing past the set's end.
fn bit_place(index: u32) -> Option<(usize, u32)> {
    let word = (index / 32) as usize;
    (word < USED_WORDS).then(|| (word, 1u32 << (index % 32)))
}

fn used_bit(word_count: &[u32; USED_WORDS], light_no: u32) -> bool {
    bit_place(light_no).is_some_and(|(word, mask)| word_count[word] & mask != 0)
}

pub fn mark_light_used(word_count: &mut [u32; USED_WORDS], light_no: u32) {
    if let Some((bit_word, bitmask)) = bit_place(light_no) {
        word_count[bit_word] |= bitmask;
    }
}

fn clear_used_bit(word_count: &mut [u32; USED_WORDS], light_no: u32) {
    if let Some((bit_word, bitmask)) = bit_place(light_no) {
        word_count[bit_word] &= !bitmask;
    }
}

/// Whether shadow choosing runs at all: sm_enable allows it (on when unset)
/// and the view has not turned shadows off.
pub fn cone_pick_enabled(sm_enable: Option<bool>, view_allows_shadows: Option<bool>) -> bool {
    let dvar_on = sm_enable.unwrap_or(SM_ENABLE_DEFAULT);
    dvar_on && view_allows_shadows != Some(false)
}

/// With the sun's shadow switched off, the sun lights count as unused.
pub fn sun_off_frees_lights(sm_daylight_enable: bool, sunlight_primary_flag_index: u32) -> bool {
    matches!((sm_daylight_enable, sunlight_primary_flag_index), (false, 1..))
}

/// Marks lights 1 through the sun as unused.
pub fn unmark_lights_to_sun(used: &mut [u32; USED_WORDS], sunlight_primary_flag_index: u32) {
    (1..=sunlight_primary_flag_index).for_each(|index| clear_used_bit(used, index));
}

/// How far the fade moves for a clock step, or nothing when fading is off
/// (a fade time of zero).
fn fade_delta(dt: i32, fade_secs: f32) -> Option<f32> {
    (fade_secs != 0.0).then(|| {
        let wrap = if dt < 0 { CLOCK_WRAP } else { 0.0 };
        (dt as f32 + wrap) * (MILLISECONDS_TO_SECONDS / fade_secs)
    })
}

fn dot3(p: [f32; 3], q: [f32; 3]) -> f32 {
    p[0] * q[0] + p[1] * q[1] + p[2] * q[2]
}

/// Whether the viewer is too far from the light, or outside its volume. The
/// sun and the exempt type are never dropped.
fn viewer_drops(eye_origin: [f32; 3], light: &ConeCasterOffer) -> bool {
    if matches!(light.kind, LAMP_KIND_PARALLEL | CULL_EXEMPT_TYPE) {
        return false;
    }
    let away: [f32; 3] = std::array::from_fn(|axis| eye_origin[axis] - light.origin[axis]);
    let reach = f32::from(light.hide_dist);
    light.view_outside_shadowmap_volume || dot3(away, away) > reach * reach
}

/// Everything one update scores lights against.
struct Scorer<'a> {
    eye_origin: [f32; 3],
    /// The point ahead of the viewer that distances are measured from.
    look_at: [f32; 3],
    sunlight_primary_flag_index: u32,
    emissive: Option<u32>,
    dvars: &'a ConeShadowChooseTunables,
    brightness_weights: [f32; 3],
}

impl<'a> Scorer<'a> {
    fn new(
        eye_origin: [f32; 3],
        eye_forward: [f32; 3],
        dvars: &'a ConeShadowChooseTunables,
        brightness_weights: [f32; 3],
    ) -> Self {
        let ahead = dvars.viewer_projection_reach;
        Self {
            eye_origin,
            look_at: std::array::from_fn(|axis| eye_origin[axis] + ahead * eye_forward[axis]),
            sunlight_primary_flag_index: 0,
            emissive: None,
            dvars,
            brightness_weights,
        }
    }

    /// Brightness times reach over distance, the distance running to a point
    /// inside the light's cone.
    fn weight(&self, light: &ConeCasterOffer) -> f32 {
        let pull = -light.radius * self.dvars.cone_project_frac;
        let gap: [f32; 3] = std::array::from_fn(|axis| {
            (light.origin[axis] - self.look_at[axis]) + pull * light.aim[axis]
        });
        let distance = libm::sqrtf(dot3(gap, gap));
        dot3(light.color, self.brightness_weights) * light.radius / (distance + 1.0)
    }

    /// The score a light competes with, or nothing when it is no candidate:
    /// light zero, the sun lights, dropped or unshadowable lights, or every
    /// light with spot shadows off.
    fn candidate(&self, index: u32, light: &ConeCasterOffer) -> Option<f32> {
        let out = index <= self.sunlight_primary_flag_index
            || !self.dvars.cone_enable
            || !light.supports_depth_shade
            || viewer_drops(self.eye_origin, light);
        if out {
            None
        } else if self.emissive == Some(index) {
            Some(EMISSIVE_SCORE)
        } else {
            Some(self.weight(light))
        }
    }
}

/// sm_maxLights caps the candidate list and the history, under the atlas's
/// four.
fn list_cap(light_budget: u32) -> usize {
    light_budget.min(CONE_TILE_COUNT as u32) as usize
}

/// The best `room` offers, highest first; an offer goes after any equal ones.
fn best_few(room: usize, offers: impl Iterator<Item = (u32, f32)>) -> Vec<(u32, f32)> {
    let mut best = Vec::with_capacity(room + 1);
    for (index, score) in offers {
        let beaten = best.iter().rev().take_while(|&&(_, above)| score > above).count();
        let at = best.len() - beaten;
        if at < room {
            best.insert(at, (index, score));
            best.truncate(room);
        }
    }
    best
}

/// The set bits of a word, highest first.
fn bits_descending(word: u32) -> impl Iterator<Item = u32> {
    (0..32u32).rev().filter(move |bit| (word >> bit) & 1 != 0)
}

/// The lights in use among the first `count`, word by word, each word's
/// lights from the highest down.
fn used_indices(used: &[u32; USED_WORDS], count: u32) -> impl Iterator<Item = u32> + '_ {
    let words = (count.saturating_add(31) >> 5) as usize;
    used.iter()
        .take(words)
        .zip((0u32..).step_by(32))
        .flat_map(|(&word, first)| bits_descending(word).map(move |bit| first + bit))
}

/// One light the history tracks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    light: u8,
    score: f32,
    /// How strongly its shadow shows, up to one.
    fade: f32,
    /// It was not re-offered and is on its way out.
    leaving: bool,
}

/// What was chosen on earlier frames, best score first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConePickMemory {
    held: Vec<Held>,
    /// The used set of the last update.
    lit_before: [u32; USED_WORDS],
    /// Scene time of the last update.
    clock: u32,
}

impl ConePickMemory {
    /// Lights no longer in use go at once; lights still in use first turn to
    /// leaving, then lose fade each update until they vanish. A light that
    /// goes is replaced by the last one.
    fn age(&mut self, in_use: &[u32; USED_WORDS], fade_step: f32) {
        let mut at = 0;
        while let Some(held) = self.held.get_mut(at) {
            let gone = match (used_bit(in_use, u32::from(held.light)), held.leaving) {
                (false, _) => true,
                (true, false) => {
                    held.leaving = true;
                    false
                }
                (true, true) => {
                    held.fade -= fade_step;
                    held.fade < FADE_DROP
                }
            };
            if gone {
                self.held.swap_remove(at);
            } else {
                at += 1;
            }
        }
    }

    /// A light already held is refreshed and stays; a new one needs room and
    /// shows fully at once, unless it was lit last update, when it eases in.
    /// Either way it then climbs past every weaker light above it.
    fn offer(&mut self, light: u8, score: f32, delta: f32, room: usize) {
        let at = match self.held.iter().position(|held| held.light == light) {
            Some(at) => {
                let held = &mut self.held[at];
                held.score = score;
                held.leaving = false;
                held.fade = (held.fade + delta).min(1.0);
                at
            }
            None if self.held.len() < room => {
                let eases = used_bit(&self.lit_before, u32::from(light));
                self.held.push(Held {
                    light,
                    score,
                    fade: if eases { delta } else { 1.0 },
                    leaving: false,
                });
                self.held.len() - 1
            }
            None => return,
        };
        let climb = self.held[..at]
            .iter()
            .rev()
            .take_while(|above| score > above.score)
            .count();
        self.held[at - climb..=at].rotate_right(1);
    }

    /// The held lights that can be drawn, in order, each marked as having a
    /// shadow map.
    fn drawn(
        &self,
        lights: &[ConeCasterOffer],
        shadowable: u32,
        shadow_map_present: &mut [u32; USED_WORDS],
    ) -> [Option<SpotShadowSubmission>; CONE_TILE_COUNT] {
        let drawable = |held: &&Held| {
            let index = u32::from(held.light);
            index < shadowable
                && lights.get(usize::from(held.light)).is_some_and(|l| l.supports_depth_shade)
        };
        let mut dest = [None; CONE_TILE_COUNT];
        let picks = self.held.iter().take(CONE_TILE_COUNT).filter(drawable);
        for (place, held) in dest.iter_mut().zip(picks) {
            mark_light_used(shadow_map_present, u32::from(held.light));
            *place = Some(SpotShadowSubmission { shadow_light_slot: held.light, fade_level: held.fade });
        }
        dest
    }
}

/// The per-frame pick. The history only moves when the scene clock does:
/// then every light in use is scored, the best `light_budget` are offered to the
/// history, and the history is aged. The sun never competes but always counts
/// as having a shadow map. Returns nothing when the clock moved and fading is
/// off (a fade time of zero).
#[allow(clippy::too_many_arguments)]
pub fn pick_cone_shadows(
    history: &mut ConePickMemory,
    used: [u32; USED_WORDS],
    lights: &[ConeCasterOffer],
    eye_origin: [f32; 3],
    eye_forward: [f32; 3],
    stage_time: u32,
    sun_slot: u32,
    shadowable: u32,
    emissive_light_index: Option<u32>,
    dvars: ConeShadowChooseTunables,
    brightness_weights: [f32; 3],
) -> Option<SpotShadowSelector> {
    let mut shadow_map_present = [0u32; USED_WORDS];
    (1..=sun_slot.min(255)).for_each(|sun| mark_light_used(&mut shadow_map_present, sun));

    let tick = stage_time.wrapping_sub(history.clock) as i32;
    if tick != 0 {
        let fade_step = fade_delta(tick, dvars.fade_secs)?;
        history.clock = stage_time;
        history.age(&used, fade_step);

        let scorer = Scorer {
            sunlight_primary_flag_index: sun_slot,
            emissive: emissive_light_index,
            ..Scorer::new(eye_origin, eye_forward, &dvars, brightness_weights)
        };
        let offers = used_indices(&used, shadowable).filter_map(|index| {
            let light = lights.get(index as usize)?;
            scorer.candidate(index, light).map(|score| (index, score))
        });
        let room = list_cap(dvars.light_budget);
        for (index, score) in best_few(room, offers) {
            if let Ok(light) = u8::try_from(index) {
                history.offer(light, score, fade_step, room);
            }
        }
        history.lit_before = used;
    }

    Some(SpotShadowSelector {
        emit: history.drawn(lights, shadowable, &mut shadow_map_present),
        shadow_map_present,
        tile: spot_shadow_tile(dvars.spot_quality),
    })
}

/// One drawn shadow: which light, where in the atlas, and how to look it up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotShadowSubmittedSlot {
    pub slot_no: u32,
    pub shadow_light_slot: u8,
    pub fade_level: f32,
    pub plan: ConeSlotPlan,
    /// Whether the light gathers its own casters.
    pub scans_casters: bool,
    /// The lookup matrix, when the light's view could be built.
    pub lookup: Option<[f32; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotShadowFrontStage {
    pub shadow_candidate_total: u32,
    pub slots: [Option<SpotShadowSubmittedSlot>; CONE_TILE_COUNT],
}

/// Gives each chosen light a tile and builds its lookup matrix. Lights below
/// `caster_light_count` gather their own casters and need no near bias; the
/// rest use the dvar's.
pub fn submit_spot_shadow_frontend(
    choose: &SpotShadowSelector,
    lights: &[ConeCasterOffer],
    caster_light_count: u32,
    close_bias: f32,
) -> SpotShadowFrontStage {
    let build = |slot_index: u32, SpotShadowSubmission { shadow_light_slot, fade_level: fade }: SpotShadowSubmission| {
        let plan = plan_cone_slots(slot_index, choose.tile);
        let scans_casters = u32::from(shadow_light_slot) < caster_light_count;
        let bias = if scans_casters { 0.0 } else { close_bias };
        let lookup = lights.get(usize::from(shadow_light_slot)).and_then(|l| {
            spot_shadow_lookup(l.origin, l.aim, l.widened_half_view_cosine, l.radius, bias, plan)
        });
        SpotShadowSubmittedSlot {
            slot_no: slot_index,
            shadow_light_slot,
            fade_level: fade,
            plan,
            scans_casters,
            lookup,
        }
    };
    let mut frontend = SpotShadowFrontStage {
        shadow_candidate_total: 0,
        slots: [None; CONE_TILE_COUNT],
    };
    for (place, pick) in choose.emit.iter().enumerate() {
        let Some(pick) = *pick else { continue };
        let Some(slot_index) = claim_cone_slot(&mut frontend.shadow_candidate_total) else {
            break;
        };
        frontend.slots[place] = Some(build(slot_index, pick));
    }
    frontend
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_light::CONE_LAMP;

    /// Fades take a second.
    const ONE_SECOND: f32 = 1.0;
    /// The test viewer looks down +x.
    const DOWN_X: [f32; 3] = [1.0, 0.0, 0.0];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// A white spot pointing down, never dropped for distance.
    fn light(origin: [f32; 3], radius: f32) -> ConeCasterOffer {
        ConeCasterOffer {
            supports_depth_shade: true,
            color: [1.0; 3],
            aim: [0.0, 0.0, -1.0],
            origin,
            radius,
            kind: CONE_LAMP,
            widened_half_view_cosine: 0.5,
            hide_dist: i16::MAX,
            view_outside_shadowmap_volume: false,
        }
    }

    fn dvars(light_budget: u32) -> ConeShadowChooseTunables {
        ConeShadowChooseTunables {
            cone_enable: true,
            light_budget,
            fade_secs: ONE_SECOND,
            viewer_projection_reach: 0.0,
            cone_project_frac: 0.0,
            spot_quality: 0,
        }
    }

    /// The viewer at the origin looking down +x.
    fn scorer(d: &ConeShadowChooseTunables) -> Scorer<'_> {
        Scorer::new([0.0; 3], DOWN_X, d, LAMP_LUMA_MIX)
    }

    fn weight(l: &ConeCasterOffer, d: &ConeShadowChooseTunables) -> f32 {
        scorer(d).weight(l)
    }

    fn candidate(
        index: u32,
        sun: u32,
        emissive: Option<u32>,
        l: &ConeCasterOffer,
        d: &ConeShadowChooseTunables,
    ) -> Option<f32> {
        let s = Scorer { sunlight_primary_flag_index: sun, emissive, ..scorer(d) };
        s.candidate(index, l)
    }

    fn used_set(indices: &[u32]) -> [u32; USED_WORDS] {
        let mut words = [0; USED_WORDS];
        indices.iter().for_each(|&index| mark_light_used(&mut words, index));
        words
    }

    fn held(light: u8, score: f32, fade: f32) -> Held {
        Held { light, score, fade, leaving: false }
    }

    fn history_of(held: &[Held]) -> ConePickMemory {
        ConePickMemory { held: held.to_vec(), ..ConePickMemory::default() }
    }

    fn held_lights(history: &ConePickMemory) -> Vec<u8> {
        history.held.iter().map(|h| h.light).collect()
    }

    /// One call to the choice; tests change the fields they care about.
    struct Ask<'a> {
        lights: &'a [ConeCasterOffer],
        used: [u32; USED_WORDS],
        time: u32,
        sun: u32,
        count: u32,
        emissive: Option<u32>,
        dvars: ConeShadowChooseTunables,
    }

    fn ask<'a>(lights: &'a [ConeCasterOffer], used: &[u32], time: u32) -> Ask<'a> {
        Ask {
            lights,
            used: used_set(used),
            time,
            sun: 0,
            count: lights.len() as u32,
            emissive: None,
            dvars: dvars(4),
        }
    }

    impl Ask<'_> {
        fn run(self, history: &mut ConePickMemory) -> Option<SpotShadowSelector> {
            pick_cone_shadows(
                history,
                self.used,
                self.lights,
                [0.0; 3],
                DOWN_X,
                self.time,
                self.sun,
                self.count,
                self.emissive,
                self.dvars,
                LAMP_LUMA_MIX,
            )
        }

        fn chosen(self, history: &mut ConePickMemory) -> Vec<u8> {
            emitted(&self.run(history).unwrap())
        }
    }

    fn emitted(choose: &SpotShadowSelector) -> Vec<u8> {
        choose.emit.iter().flatten().map(|e| e.shadow_light_slot).collect()
    }

    fn four_lights() -> [ConeCasterOffer; 4] {
        [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 50.0),
            light([10.0, 0.0, 0.0], 200.0),
            light([10.0, 0.0, 0.0], 100.0),
        ]
    }

    #[test]
    fn used_bits_ignore_indices_past_the_set() {
        let mut words = [0; USED_WORDS];
        for index in [3, 255, 256] {
            mark_light_used(&mut words, index);
        }
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
        assert!(used_bit(&words, 4) && used_bit(&words, 40));
    }

    #[test]
    fn choosing_runs_unless_shadows_are_off_for_the_view_or_the_dvar() {
        assert!(cone_pick_enabled(None, None));
        assert!(cone_pick_enabled(Some(true), Some(true)));
        assert!(!cone_pick_enabled(Some(false), None));
        assert!(!cone_pick_enabled(None, Some(false)));
    }

    #[test]
    fn the_sun_bits_are_cleared_only_with_the_sun_shadow_off() {
        assert!(sun_off_frees_lights(false, 2));
        assert!(!sun_off_frees_lights(true, 2));
        assert!(!sun_off_frees_lights(false, 0));
    }

    #[test]
    fn clearing_through_the_sun_keeps_light_zero_and_the_rest() {
        let mut words = used_set(&[0, 1, 2, 3, 4]);
        unmark_lights_to_sun(&mut words, 3);
        assert_eq!(words[0], 0b1_0001);
    }

    #[test]
    fn the_fade_delta_follows_the_fade_time() {
        assert_eq!(fade_delta(100, 0.0), None);
        for (step, time, want) in [(100, 1.0, 0.1), (100, 0.5, 0.2), (1000, 2.0, 0.5)] {
            assert!(close(fade_delta(step, time).unwrap(), want));
        }
        // A backward step reads as a wrapped forward one.
        assert!(fade_delta(-1, 1.0).unwrap() > 4_000_000.0);
    }

    #[test]
    fn the_score_is_brightness_times_radius_over_distance_plus_one() {
        // White light: the weights sum to one. 99 away, so 100 / 100.
        assert!(close(weight(&light([99.0, 0.0, 0.0], 100.0), &dvars(4)), 1.0));
    }

    #[test]
    fn closer_brighter_and_greener_lights_score_higher() {
        let d = dvars(4);
        let near = light([10.0, 0.0, 0.0], 100.0);
        let far = light([50.0, 0.0, 0.0], 100.0);
        assert!(weight(&near, &d) > weight(&far, &d));

        let dim = ConeCasterOffer { color: [0.1; 3], ..near };
        assert!(weight(&near, &d) > weight(&dim, &d));

        let green = ConeCasterOffer { color: [0.0, 1.0, 0.0], ..near };
        let blue = ConeCasterOffer { color: [0.0, 0.0, 1.0], ..near };
        assert!(weight(&green, &d) > weight(&blue, &d));
    }

    #[test]
    fn distance_is_measured_from_a_point_ahead_of_the_viewer() {
        let mut d = dvars(4);
        d.viewer_projection_reach = 99.0;
        // The look-at point is on the light: 100 * 1 / (0 + 1), less the
        // little the brightness_weights weights (which sum to 0.9999) take off.
        let got = weight(&light([99.0, 0.0, 0.0], 100.0), &d);
        assert!((got - 100.0).abs() < 0.02, "{got}");
    }

    #[test]
    fn distance_is_measured_to_a_point_down_the_cone() {
        let mut d = dvars(4);
        d.cone_project_frac = 0.5;
        // Half a radius against the direction (0, 0, -1) is 50 up: 100 / 51.
        assert!(close(weight(&light([0.0; 3], 100.0), &d), 100.0 / 51.0));
    }

    #[test]
    fn far_lights_and_lights_whose_volume_the_viewer_is_outside_are_dropped() {
        let mut l = light([100.0, 0.0, 0.0], 50.0);
        l.hide_dist = 99;
        assert!(viewer_drops([0.0; 3], &l));
        l.hide_dist = 100;
        assert!(!viewer_drops([0.0; 3], &l));
        l.view_outside_shadowmap_volume = true;
        assert!(viewer_drops([0.0; 3], &l));
    }

    #[test]
    fn the_sun_and_the_exempt_type_are_never_culled() {
        let mut l = light([10_000.0, 0.0, 0.0], 50.0);
        l.hide_dist = 1;
        l.view_outside_shadowmap_volume = true;
        assert!(viewer_drops([0.0; 3], &l));
        for exempt in [LAMP_KIND_PARALLEL, 6] {
            l.kind = exempt;
            assert!(!viewer_drops([0.0; 3], &l));
        }
    }

    #[test]
    fn the_candidate_list_is_capped_by_max_lights_and_by_the_atlas() {
        let caps: Vec<_> = [0, 2, 4, 9].into_iter().map(list_cap).collect();
        assert_eq!(caps, [0, 2, 4, 4]);
    }

    #[test]
    fn the_best_few_are_kept_best_first() {
        let offers = [(1, 5.0), (2, 9.0), (3, 7.0)];
        assert_eq!(best_few(4, offers.into_iter()), [(2, 9.0), (3, 7.0), (1, 5.0)]);
        // A full list drops the worst, and turns away anything below it.
        assert_eq!(best_few(2, offers.into_iter()), [(2, 9.0), (3, 7.0)]);
        let more = offers.into_iter().chain([(4, 1.0)]);
        assert_eq!(best_few(2, more), [(2, 9.0), (3, 7.0)]);
        assert!(best_few(0, offers.into_iter()).is_empty());
    }

    #[test]
    fn the_sun_light_zero_and_unshadowable_lights_are_not_candidates() {
        let l = light([10.0, 0.0, 0.0], 100.0);
        let d = dvars(4);
        assert!(candidate(0, 0, None, &l, &d).is_none());
        assert!(candidate(1, 1, None, &l, &d).is_none());
        assert!(candidate(2, 1, None, &l, &d).is_some());
        let blocked = ConeCasterOffer { supports_depth_shade: false, ..l };
        assert!(candidate(1, 0, None, &blocked, &d).is_none());
    }

    #[test]
    fn nothing_is_a_candidate_with_spot_shadows_off() {
        let mut d = dvars(4);
        d.cone_enable = false;
        assert!(candidate(1, 0, None, &light([10.0, 0.0, 0.0], 100.0), &d).is_none());
    }

    #[test]
    fn the_emissive_light_outranks_any_score() {
        let l = light([10.0, 0.0, 0.0], 100.0);
        let d = dvars(4);
        assert_eq!(candidate(3, 0, Some(3), &l, &d), Some(EMISSIVE_SCORE));
        assert!(candidate(2, 0, Some(3), &l, &d).unwrap() < EMISSIVE_SCORE);
    }

    #[test]
    fn a_new_light_enters_the_history_at_full_fade() {
        let mut history = ConePickMemory::default();
        history.offer(3, 5.0, 0.1, 4);
        assert_eq!(history.held, [held(3, 5.0, 1.0)]);
    }

    #[test]
    fn a_light_lit_last_update_eases_in() {
        let mut history = ConePickMemory { lit_before: used_set(&[3]), ..ConePickMemory::default() };
        history.offer(3, 5.0, 0.1, 4);
        history.offer(4, 5.0, 0.1, 4);
        assert!(close(history.held[0].fade, 0.1));
        assert!(close(history.held[1].fade, 1.0));
    }

    #[test]
    fn the_history_stays_sorted_and_a_full_one_turns_new_lights_away() {
        let mut roomy = ConePickMemory::default();
        let mut tight = ConePickMemory::default();
        for (light, score) in [(1, 5.0), (2, 9.0), (3, 7.0)] {
            roomy.offer(light, score, 0.1, 4);
        }
        for (light, score) in [(1, 5.0), (2, 9.0), (3, 99.0)] {
            tight.offer(light, score, 0.1, 2);
        }
        assert_eq!(held_lights(&roomy), [2, 3, 1]);
        assert_eq!(held_lights(&tight), [2, 1]);
    }

    #[test]
    fn a_light_already_in_the_history_is_refreshed() {
        let mut history = history_of(&[held(1, 9.0, 0.5), held(2, 5.0, 0.5)]);
        history.held[1].leaving = true;
        history.offer(2, 20.0, 0.25, 4);
        // It outscores light 1 now and moves up, no longer fading out.
        assert_eq!(held_lights(&history), [2, 1]);
        let top = history.held[0];
        assert_eq!(top.score, 20.0);
        assert!(!top.leaving);
        assert!(close(top.fade, 0.75));
    }

    #[test]
    fn a_refreshed_fade_stops_at_one() {
        let mut history = history_of(&[held(1, 9.0, 0.9)]);
        history.offer(1, 9.0, 0.5, 4);
        assert_eq!(history.held[0].fade, 1.0);
    }

    #[test]
    fn a_light_no_longer_in_use_leaves_the_history() {
        let mut history = history_of(&[held(1, 9.0, 1.0), held(2, 5.0, 1.0), held(3, 2.0, 1.0)]);
        history.age(&used_set(&[1, 3]), 0.1);
        // Light 2 is replaced by the last entry, light 3.
        assert_eq!(held_lights(&history), [1, 3]);
    }

    #[test]
    fn a_light_still_in_use_starts_fading_out_before_it_loses_fade() {
        let mut history = history_of(&[held(1, 9.0, 1.0)]);
        let used = used_set(&[1]);
        history.age(&used, 0.25);
        assert!(history.held[0].leaving);
        assert_eq!(history.held[0].fade, 1.0);
        history.age(&used, 0.25);
        assert!(close(history.held[0].fade, 0.75));
    }

    #[test]
    fn a_fading_light_is_dropped_when_its_fade_runs_out() {
        let mut history = history_of(&[Held { leaving: true, ..held(1, 9.0, 0.2) }]);
        let used = used_set(&[1]);
        history.age(&used, 0.1);
        assert_eq!(history.held.len(), 1);
        history.age(&used, 0.1);
        assert!(history.held.is_empty());
    }

    #[test]
    fn bits_are_visited_from_the_highest_down() {
        assert_eq!(bits_descending(0b1010_0101).collect::<Vec<_>>(), [7, 5, 2, 0]);
        assert_eq!(bits_descending(0).count(), 0);
        assert_eq!(bits_descending(1 << 31).collect::<Vec<_>>(), [31]);
    }

    #[test]
    fn nothing_is_chosen_until_the_scene_clock_moves() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = ConePickMemory::default();
        assert!(ask(&lights, &[1], 0).chosen(&mut history).is_empty());
        assert!(history.held.is_empty());
    }

    #[test]
    fn the_best_lights_are_chosen_in_score_order() {
        let lights = four_lights();
        let mut history = ConePickMemory::default();
        let choose = ask(&lights, &[1, 2, 3], 1000).run(&mut history).unwrap();
        assert_eq!(emitted(&choose), [2, 3, 1]);
        assert!(choose.emit[0].unwrap().fade_level == 1.0);
        assert_eq!(history.clock, 1000);
    }

    #[test]
    fn max_lights_limits_how_many_are_chosen() {
        let lights = four_lights();
        let mut history = ConePickMemory::default();
        let narrow = Ask { dvars: dvars(2), ..ask(&lights, &[1, 2, 3], 1000) };
        assert_eq!(narrow.chosen(&mut history), [2, 3]);
    }

    #[test]
    fn lights_not_in_use_are_not_chosen() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 500.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = ConePickMemory::default();
        assert_eq!(ask(&lights, &[2], 1000).chosen(&mut history), [2]);
    }

    #[test]
    fn the_sun_has_a_shadow_map_but_is_never_emitted() {
        let lights = [light([0.0; 3], 1.0), light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = ConePickMemory::default();
        let choose = Ask { sun: 1, ..ask(&lights, &[1, 2], 1000) }.run(&mut history).unwrap();
        assert_eq!(emitted(&choose), [2]);
        let mapped = choose.shadow_map_present;
        assert!(used_bit(&mapped, 1) && used_bit(&mapped, 2) && !used_bit(&mapped, 0));
    }

    #[test]
    fn the_emissive_light_is_chosen_first() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 500.0),
            light([900.0, 0.0, 0.0], 1.0),
        ];
        let mut history = ConePickMemory::default();
        let lit = Ask { emissive: Some(2), ..ask(&lights, &[1, 2], 1000) };
        assert_eq!(lit.chosen(&mut history), [2, 1]);
    }

    #[test]
    fn a_light_that_cannot_use_a_shadow_map_is_never_emitted() {
        let mut lights = vec![light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = ConePickMemory::default();
        assert_eq!(ask(&lights, &[1], 1000).chosen(&mut history), [1]);
        // It loses the ability later: the history still holds it, the emit skips it.
        lights[1].supports_depth_shade = false;
        let later = ask(&lights, &[1], 1000).run(&mut history).unwrap();
        assert!(emitted(&later).is_empty());
        assert!(!used_bit(&later.shadow_map_present, 1));
    }

    #[test]
    fn a_light_beyond_the_shadowable_count_is_never_emitted() {
        let lights = [
            light([0.0; 3], 1.0),
            light([10.0, 0.0, 0.0], 100.0),
            light([10.0, 0.0, 0.0], 100.0),
        ];
        let mut history = ConePickMemory::default();
        let short = Ask { count: 2, ..ask(&lights, &[1, 2], 1000) };
        assert_eq!(short.chosen(&mut history), [1]);
    }

    #[test]
    fn a_zero_fade_time_with_a_moving_clock_chooses_nothing() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = ConePickMemory::default();
        let mut still = ask(&lights, &[1], 1000);
        still.dvars.fade_secs = 0.0;
        assert!(still.run(&mut history).is_none());
    }

    #[test]
    fn the_tile_follows_the_quality_setting() {
        let lights = [light([0.0; 3], 1.0)];
        let mut history = ConePickMemory::default();
        let mut sharp = ask(&lights, &[], 0);
        sharp.dvars.spot_quality = 2;
        assert_eq!(sharp.run(&mut history).unwrap().tile, 2048);
    }

    #[test]
    fn a_light_that_stops_being_used_is_gone_at_the_next_update() {
        let lights = [light([0.0; 3], 1.0), light([10.0, 0.0, 0.0], 100.0)];
        let mut history = ConePickMemory::default();
        ask(&lights, &[1], 1000).run(&mut history);
        assert_eq!(history.held.len(), 1);
        // Still used and still a candidate: it stays, not fading.
        ask(&lights, &[1], 1100).run(&mut history);
        assert_eq!(history.held.len(), 1);
        assert!(!history.held[0].leaving);
        // No longer used: it is gone at the next update.
        assert!(ask(&lights, &[], 1200).chosen(&mut history).is_empty());
        assert!(history.held.is_empty());
    }

    /// A choice of `picks`, each at half fade, on 512 tiles.
    fn chosen(picks: &[u8]) -> SpotShadowSelector {
        let mut emit = [None; CONE_TILE_COUNT];
        for (place, &pick) in emit.iter_mut().zip(picks) {
            *place = Some(SpotShadowSubmission { shadow_light_slot: pick, fade_level: 0.5 });
        }
        SpotShadowSelector { emit, shadow_map_present: [0; USED_WORDS], tile: 512 }
    }

    #[test]
    fn each_chosen_light_gets_the_next_tile() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let frontend = submit_spot_shadow_frontend(&chosen(&[3, 1, 4]), &lights, 0, 0.0);
        assert_eq!(frontend.shadow_candidate_total, 3);
        let slots: Vec<_> = frontend.slots.iter().flatten().collect();
        assert_eq!(slots.len(), 3);
        for (n, slot) in (0u32..).zip(&slots) {
            assert_eq!(slot.slot_no, n);
            assert_eq!(slot.plan, plan_cone_slots(n, 512));
            assert_eq!(slot.fade_level, 0.5);
        }
        assert_eq!((slots[0].shadow_light_slot, slots[1].shadow_light_slot), (3, 1));
        assert!(slots[0].plan.clears_atlas && !slots[1].plan.clears_atlas);
        assert!(frontend.slots[3].is_none());
    }

    #[test]
    fn lights_below_the_caster_count_walk_their_own_casters() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let frontend = submit_spot_shadow_frontend(&chosen(&[1, 3]), &lights, 2, 0.0);
        assert!(frontend.slots[0].unwrap().scans_casters);
        assert!(!frontend.slots[1].unwrap().scans_casters);
    }

    #[test]
    fn only_lights_past_the_caster_count_take_the_near_bias() {
        let lights = vec![light([0.0; 3], 100.0); 5];
        let lookups = |bias: f32| {
            let frontend = submit_spot_shadow_frontend(&chosen(&[1, 3]), &lights, 2, bias);
            frontend.slots.map(|slot| slot.and_then(|s| s.lookup))
        };
        let (flat, biased) = (lookups(0.0), lookups(8.0));
        assert_eq!(flat[0], biased[0]);
        assert_ne!(flat[1], biased[1]);
    }

    #[test]
    fn a_light_missing_from_the_list_gets_a_tile_but_no_lookup() {
        let frontend = submit_spot_shadow_frontend(&chosen(&[1]), &[], 0, 0.0);
        assert_eq!(frontend.slots[0].unwrap().lookup, None);
        assert_eq!(frontend.shadow_candidate_total, 1);
    }

    #[test]
    fn a_light_whose_view_cannot_be_built_has_no_lookup() {
        // A radius of 1 puts the far plane on the near plane.
        let lights = vec![light([0.0; 3], 1.0); 2];
        let frontend = submit_spot_shadow_frontend(&chosen(&[1]), &lights, 2, 0.0);
        assert_eq!(frontend.slots[0].unwrap().lookup, None);
    }

    #[test]
    fn an_empty_choice_emits_no_slots() {
        let lights = vec![light([0.0; 3], 100.0); 2];
        let frontend = submit_spot_shadow_frontend(&chosen(&[]), &lights, 0, 0.0);
        assert_eq!(frontend.shadow_candidate_total, 0);
        assert!(frontend.slots.iter().all(Option::is_none));
    }
}
