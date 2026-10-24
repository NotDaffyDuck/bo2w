//! Script earthquakes: a shaking of the player's view that is strongest at the quake's origin and dies away with
//! distance and time.

use std::f32::consts::TAU;

/// The most earthquakes that can be running at once.
pub const MAX_SCRIPT_EARTHQUAKES: usize = 64;

/// How far round a circle the shake's phase moves from one quake id to the next, so quakes started together do not
/// shake in step (the golden angle, in radians).
const PHASE_STEP: f32 = 2.399_963_1;

/// How many shakes a second each of the view's three angles gets; different primes of a sort, so the shake wanders
/// rather than repeating.
const ANGLE_RATES: [f32; 3] = [17.0, 23.0, 11.0];

/// One earthquake: where it is centred, how hard it shakes (0 to 1 and beyond, capped at 1), how far it reaches
/// (0 means everywhere), and when it starts and how long it lasts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptEarthquake {
    pub id: u32,
    pub origin: [f32; 3],
    pub scale: f32,
    pub radius: f32,
    pub start_ms: i32,
    pub duration_ms: i32,
}

fn distance_between(a: [f32; 3], b: [f32; 3]) -> f32 {
    let squares: f32 = a.iter().zip(b).map(|(a, b)| (a - b) * (a - b)).sum();
    squares.sqrt()
}

impl ScriptEarthquake {
    /// Whether the quake is one that can be run: finite place, strength and reach that are not negative, and a duration.
    pub fn valid(self) -> bool {
        self.origin.iter().all(|c| c.is_finite())
            && self.scale.is_finite()
            && self.scale >= 0.0
            && self.radius.is_finite()
            && self.radius >= 0.0
            && self.duration_ms > 0
    }

    /// Whether the quake is going at `now_ms`: from its start up to, not including, its end.
    pub fn active(self, now_ms: i32) -> bool {
        (0..self.duration_ms).contains(&now_ms.wrapping_sub(self.start_ms))
    }

    /// How much the quake turns a view whose eye is at `eye`, as an offset to add to (pitch, yaw, roll), in degrees.
    /// Nothing while it is not running. Its size fades in a line with the eye's distance (to nothing at the quake's
    /// radius) and with the time left.
    pub fn angle_offset(self, eye: [f32; 3], now_ms: i32) -> [f32; 3] {
        if !self.active(now_ms) {
            return [0.0; 3];
        }
        let nearness = if self.radius == 0.0 {
            1.0
        } else {
            (1.0 - distance_between(eye, self.origin) / self.radius).clamp(0.0, 1.0)
        };
        let elapsed = now_ms.wrapping_sub(self.start_ms) as f32;
        let time_left = 1.0 - elapsed / self.duration_ms as f32;
        let strength = self.scale.min(1.0) * nearness * time_left;
        let phase = (self.id % 1024) as f32 * PHASE_STEP;
        ANGLE_RATES.map(|rate| (elapsed * 0.001 * rate * TAU + phase).sin() * strength)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quake() -> ScriptEarthquake {
        ScriptEarthquake { id: 3, origin: [0.0; 3], scale: 1.0, radius: 100.0, start_ms: 500, duration_ms: 1000 }
    }

    fn biggest(offset: [f32; 3]) -> f32 {
        offset.iter().fold(0.0f32, |most, &angle| most.max(angle.abs()))
    }

    #[test]
    fn an_ordinary_quake_is_valid() {
        assert!(quake().valid());
        assert!(ScriptEarthquake { radius: 0.0, scale: 0.0, ..quake() }.valid());
    }

    #[test]
    fn a_quake_with_a_bad_place_strength_reach_or_duration_is_not_valid() {
        assert!(!ScriptEarthquake { origin: [f32::NAN, 0.0, 0.0], ..quake() }.valid());
        assert!(!ScriptEarthquake { scale: -0.5, ..quake() }.valid());
        assert!(!ScriptEarthquake { scale: f32::INFINITY, ..quake() }.valid());
        assert!(!ScriptEarthquake { radius: -1.0, ..quake() }.valid());
        assert!(!ScriptEarthquake { duration_ms: 0, ..quake() }.valid());
    }

    #[test]
    fn a_quake_runs_from_its_start_up_to_but_not_including_its_end() {
        let q = quake();
        assert!(!q.active(499));
        assert!(q.active(500));
        assert!(q.active(1499));
        assert!(!q.active(1500));
    }

    #[test]
    fn a_quake_that_is_not_running_does_not_move_the_view() {
        assert_eq!(quake().angle_offset([10.0, 0.0, 0.0], 100), [0.0; 3]);
        assert_eq!(quake().angle_offset([10.0, 0.0, 0.0], 1500), [0.0; 3]);
    }

    #[test]
    fn the_shake_is_nothing_at_the_radius_and_beyond() {
        let q = quake();
        assert_eq!(q.angle_offset([100.0, 0.0, 0.0], 700), [0.0; 3]);
        assert_eq!(q.angle_offset([500.0, 0.0, 0.0], 700), [0.0; 3]);
    }

    #[test]
    fn a_nearer_eye_shakes_harder_than_a_farther_one() {
        let q = quake();
        let near = biggest(q.angle_offset([10.0, 0.0, 0.0], 700));
        let far = biggest(q.angle_offset([80.0, 0.0, 0.0], 700));
        assert!(near > far && far > 0.0, "{near} {far}");
    }

    #[test]
    fn a_quake_with_no_radius_reaches_everywhere_equally() {
        let q = ScriptEarthquake { radius: 0.0, ..quake() };
        assert_eq!(q.angle_offset([0.0; 3], 700), q.angle_offset([9999.0, 0.0, 0.0], 700));
        assert!(biggest(q.angle_offset([0.0; 3], 700)) > 0.0);
    }

    #[test]
    fn the_shake_dies_away_over_the_quakes_duration() {
        let q = ScriptEarthquake { radius: 0.0, ..quake() };
        let peak_early = (500..700).map(|ms| biggest(q.angle_offset([0.0; 3], ms))).fold(0.0f32, f32::max);
        let peak_late = (1300..1499).map(|ms| biggest(q.angle_offset([0.0; 3], ms))).fold(0.0f32, f32::max);
        assert!(peak_early > peak_late && peak_late > 0.0, "{peak_early} {peak_late}");
    }

    #[test]
    fn strength_beyond_one_is_capped_at_one() {
        let mild = ScriptEarthquake { scale: 1.0, ..quake() };
        let wild = ScriptEarthquake { scale: 5.0, ..quake() };
        assert_eq!(mild.angle_offset([20.0, 0.0, 0.0], 800), wild.angle_offset([20.0, 0.0, 0.0], 800));
    }

    #[test]
    fn no_strength_means_no_shake() {
        let still = ScriptEarthquake { scale: 0.0, ..quake() };
        assert_eq!(still.angle_offset([0.0; 3], 800), [0.0; 3]);
    }

    #[test]
    fn the_shake_depends_on_the_id_and_repeats_for_the_same_one() {
        let a = ScriptEarthquake { id: 1, ..quake() };
        let b = ScriptEarthquake { id: 2, ..quake() };
        assert_ne!(a.angle_offset([0.0; 3], 800), b.angle_offset([0.0; 3], 800));
        assert_eq!(a.angle_offset([0.0; 3], 800), a.angle_offset([0.0; 3], 800));
        let wrapped = ScriptEarthquake { id: 1 + 1024, ..quake() };
        assert_eq!(a.angle_offset([0.0; 3], 800), wrapped.angle_offset([0.0; 3], 800));
    }

    #[test]
    fn distance_is_the_straight_line_between_two_points() {
        assert_eq!(distance_between([0.0; 3], [3.0, 4.0, 0.0]), 5.0);
        assert_eq!(distance_between([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]), 0.0);
    }
}
