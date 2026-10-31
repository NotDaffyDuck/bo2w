//! Script fog: one fog setting, and a timed fade from one setting to another.

use std::f32::consts::LN_2;

/// The bright wedge of fog around the sun: its colour and direction, the angles (in degrees from the sun) it starts to
/// fade at and is gone by, and how strong it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptSunFog {
    pub tint_rgb: [f32; 3],
    pub sun_way: [f32; 3],
    pub from_deg: f32,
    pub to_deg: f32,
    pub scale: f32,
}

/// One fog setting. The fog is clear up to `start_dist`, then thickens until it is half opaque `halfway_dist` further
/// on, and never gets thicker than `max_opacity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptFogParams {
    pub from_dist: f32,
    pub half_dist: f32,
    pub tint_rgb: [f32; 3],
    pub thickest: f32,
    pub sun: Option<ScriptSunFog>,
}

fn unit_range(value: &f32) -> bool {
    (0.0..=1.0).contains(value)
}

impl ScriptSunFog {
    fn valid(self) -> bool {
        self.tint_rgb.iter().all(unit_range)
            && self.sun_way.iter().all(|c| c.is_finite())
            && (0.0..=180.0).contains(&self.from_deg)
            && (self.from_deg..=180.0).contains(&self.to_deg)
            && self.scale.is_finite()
            && self.scale >= 0.0
    }
}

impl ScriptFogParams {
    /// Whether the setting is one the renderer can draw: finite distances, a halfway distance above zero, colours and
    /// opacity from 0 to 1, and a sun wedge that is itself in range.
    pub fn valid(self) -> bool {
        let density = self.density();
        self.from_dist.is_finite()
            && self.from_dist >= 0.0
            && self.half_dist.is_finite()
            && self.half_dist > 0.0
            && density.is_finite()
            && density > 0.0
            && self.tint_rgb.iter().all(unit_range)
            && unit_range(&self.thickest)
            && self.sun.is_none_or(ScriptSunFog::valid)
    }

    /// How fast the fog thickens with distance: the number that halves the clear light over `halfway_dist`. A fog with
    /// no halfway distance has none.
    pub fn density(self) -> f32 {
        if self.half_dist <= 0.0 { 0.0 } else { LN_2 / self.half_dist }
    }

    /// The fog setting that has the given density, as its halfway distance (none for a density of zero).
    fn halfway_for_density(density: f32) -> f32 {
        if density == 0.0 { 0.0 } else { LN_2 / density }
    }
}

/// A fade of the scene's fog from one setting to another, starting at `start_ms` and taking `duration_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptFog {
    pub from: ScriptFogParams,
    pub to: ScriptFogParams,
    pub began_ms: i32,
    pub lasts_ms: i32,
}

/// How far along a fade is, as a fraction.
#[derive(Clone, Copy)]
struct Progress(f32);

impl Progress {
    fn mix(self, from: f32, to: f32) -> f32 {
        from + self.0 * (to - from)
    }

    fn mix3(self, from: [f32; 3], till: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| self.mix(from[axis], till[axis]))
    }

    /// Blends two colours the way the renderer holds them: each channel is first rounded to a whole 0 to 255 value, the
    /// blend is rounded to a whole value too, and the result is scaled back to 0 to 1.
    fn mix_color(self, from: [f32; 3], till: [f32; 3]) -> [f32; 3] {
        let whole = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0 + 0.5).floor();
        std::array::from_fn(|axis| self.mix(whole(from[axis]), whole(till[axis])).round() / 255.0)
    }
}

impl ScriptFog {
    /// The fog at `clock_millis`: the `from` setting at the start, the `to` setting once the fade is over (or at once if it
    /// has no duration), and a straight blend between. Fading to a setting with a sun wedge, from one without, grows
    /// the wedge from a dark one of zero width.
    pub fn sample(self, clock_millis: i32) -> ScriptFogParams {
        let elapsed = clock_millis.wrapping_sub(self.began_ms).max(0);
        let span = self.lasts_ms;
        if span <= 0 || elapsed >= span {
            return self.to;
        }
        let progress = Progress(elapsed as f32 / span as f32);
        let (from, to) = (self.from, self.to);
        ScriptFogParams {
            from_dist: progress.mix(from.from_dist, to.from_dist),
            half_dist: ScriptFogParams::halfway_for_density(progress.mix(from.density(), to.density())),
            tint_rgb: progress.mix_color(from.tint_rgb, to.tint_rgb),
            thickest: progress.mix(from.thickest, to.thickest),
            sun: to.sun.map(|target| {
                let start = from.sun.unwrap_or(ScriptSunFog {
                    tint_rgb: from.tint_rgb,
                    sun_way: Default::default(),
                    from_deg: Default::default(),
                    to_deg: Default::default(),
                    scale: 1.0,
                });
                ScriptSunFog {
                    tint_rgb: progress.mix_color(start.tint_rgb, target.tint_rgb),
                    sun_way: progress.mix3(start.sun_way, target.sun_way),
                    from_deg: progress.mix(start.from_deg, target.from_deg),
                    to_deg: progress.mix(start.to_deg, target.to_deg),
                    scale: progress.mix(start.scale, target.scale),
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(near_span: f32, mid_span: f32) -> ScriptFogParams {
        ScriptFogParams { from_dist: near_span, half_dist: mid_span, tint_rgb: [0.5; 3], thickest: 0.8, sun: None }
    }

    fn sun() -> ScriptSunFog {
        ScriptSunFog { tint_rgb: [1.0, 0.5, 0.0], sun_way: [0.0, 0.0, 1.0], from_deg: 10.0, to_deg: 40.0, scale: 2.0 }
    }

    fn fade(from: ScriptFogParams, to: ScriptFogParams) -> ScriptFog {
        ScriptFog { from, to, began_ms: 1000, lasts_ms: 2000 }
    }

    #[test]
    fn density_is_ln2_over_the_halfway_distance_and_zero_without_one() {
        assert_eq!(fog(0.0, 100.0).density(), LN_2 / 100.0);
        assert_eq!(fog(0.0, 0.0).density(), 0.0);
        assert_eq!(fog(0.0, -5.0).density(), 0.0);
    }

    #[test]
    fn an_ordinary_fog_is_valid() {
        assert!(fog(0.0, 100.0).valid());
        assert!(fog(500.0, 1.0).valid());
        assert!(ScriptFogParams { sun: Some(sun()), ..fog(0.0, 100.0) }.valid());
    }

    #[test]
    fn distances_that_are_negative_zero_or_not_numbers_are_not_valid() {
        assert!(!fog(-1.0, 100.0).valid());
        assert!(!fog(0.0, 0.0).valid());
        assert!(!fog(f32::NAN, 100.0).valid());
        assert!(!fog(0.0, f32::INFINITY).valid());
    }

    #[test]
    fn colour_and_opacity_must_be_between_zero_and_one() {
        assert!(!ScriptFogParams { tint_rgb: [0.0, 1.1, 0.0], ..fog(0.0, 100.0) }.valid());
        assert!(!ScriptFogParams { tint_rgb: [-0.1, 0.0, 0.0], ..fog(0.0, 100.0) }.valid());
        assert!(!ScriptFogParams { thickest: 1.5, ..fog(0.0, 100.0) }.valid());
        assert!(ScriptFogParams { tint_rgb: [0.0, 1.0, 0.0], thickest: 1.0, ..fog(0.0, 100.0) }.valid());
    }

    #[test]
    fn a_sun_wedge_must_be_in_range_too() {
        let with = |sun_ray| ScriptFogParams { sun: Some(sun_ray), ..fog(0.0, 100.0) }.valid();
        assert!(with(sun()));
        assert!(!with(ScriptSunFog { from_deg: -1.0, ..sun() }));
        assert!(!with(ScriptSunFog { from_deg: 181.0, ..sun() }));
        assert!(!with(ScriptSunFog { to_deg: 5.0, ..sun() }), "ends before it begins");
        assert!(!with(ScriptSunFog { to_deg: 200.0, ..sun() }));
        assert!(!with(ScriptSunFog { scale: -1.0, ..sun() }));
        assert!(!with(ScriptSunFog { sun_way: [0.0, f32::NAN, 1.0], ..sun() }));
        assert!(!with(ScriptSunFog { tint_rgb: [2.0, 0.0, 0.0], ..sun() }));
    }

    #[test]
    fn a_fade_is_the_old_setting_at_its_start_and_the_new_one_when_over() {
        let f = fade(fog(0.0, 100.0), fog(200.0, 50.0));
        assert_eq!(f.sample(1000).from_dist, 0.0);
        assert_eq!(f.sample(0).from_dist, 0.0, "before it starts it is still the old fog");
        assert_eq!(f.sample(3000), f.to);
        assert_eq!(f.sample(99_000), f.to);
    }

    #[test]
    fn a_fade_with_no_duration_is_at_the_new_setting_at_once() {
        let f = ScriptFog { lasts_ms: 0, ..fade(fog(0.0, 100.0), fog(200.0, 50.0)) };
        assert_eq!(f.sample(1000), f.to);
    }

    #[test]
    fn halfway_through_a_fade_the_distance_is_halfway_between() {
        let mid = fade(fog(0.0, 100.0), fog(200.0, 100.0)).sample(2000);
        assert_eq!(mid.from_dist, 100.0);
        assert_eq!(mid.thickest, 0.8);
    }

    #[test]
    fn the_density_is_blended_not_the_halfway_distance() {
        let mid = fade(fog(0.0, 10.0), fog(0.0, 20.0)).sample(2000);
        let expected_density = (fog(0.0, 10.0).density() + fog(0.0, 20.0).density()) / 2.0;
        assert!((mid.density() - expected_density).abs() < 1e-6);
        assert!((mid.half_dist - 13.333_333).abs() < 1e-3, "{}", mid.half_dist);
    }

    #[test]
    fn colours_blend_in_whole_steps_of_a_255th() {
        let black_to_white = ScriptFogParams { tint_rgb: [0.0; 3], ..fog(0.0, 100.0) };
        let white = ScriptFogParams { tint_rgb: [1.0; 3], ..fog(0.0, 100.0) };
        let mid = fade(black_to_white, white).sample(2000);
        assert_eq!(mid.tint_rgb, [128.0 / 255.0; 3]);
        let quarter = fade(black_to_white, white).sample(1500);
        assert_eq!(quarter.tint_rgb, [64.0 / 255.0; 3]);
    }

    #[test]
    fn a_new_sun_wedge_grows_from_a_dark_one_of_no_width() {
        let target = ScriptFogParams { sun: Some(sun()), ..fog(0.0, 100.0) };
        let mid = fade(fog(0.0, 100.0), target).sample(2000).sun.unwrap();
        assert_eq!(mid.from_deg, 5.0);
        assert_eq!(mid.to_deg, 20.0);
        assert_eq!(mid.scale, 1.5);
        assert_eq!(mid.sun_way, [0.0, 0.0, 0.5]);
    }

    #[test]
    fn a_sun_wedge_that_is_not_in_the_new_setting_is_dropped_during_the_fade() {
        let from = ScriptFogParams { sun: Some(sun()), ..fog(0.0, 100.0) };
        assert_eq!(fade(from, fog(0.0, 100.0)).sample(2000).sun, None);
    }
}
