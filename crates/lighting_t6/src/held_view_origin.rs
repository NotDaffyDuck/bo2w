//! Where the first-person weapon model is lit from.

/// Roll the view takes at full lean, in degrees.
const LEAN_ROLL: f32 = 16.0;
/// How far a full lean slides the viewpoint sideways, in inches.
const LEAN_SLIDE: f32 = 20.0;

/// The player's eye position, slid sideways by however far they are leaning.
pub fn held_view_light_point(
    at: [f32; 3],
    eye_rise: f32,
    look_heading: f32,
    tilt_sideways: f32,
) -> [f32; 3] {
    let eye = [at[0], at[1], at[2] + eye_rise];
    game_math::lean_offset(eye, look_heading, tilt_sideways, LEAN_ROLL, LEAN_SLIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_upright_lights_from_the_eye() {
        assert_eq!(
            held_view_light_point([10.0, 20.0, 30.0], 60.0, 123.0, 0.0),
            [10.0, 20.0, 90.0]
        );
    }

    #[test]
    fn leaning_slides_the_origin_sideways_not_forward() {
        let leaning = held_view_light_point([0.0; 3], 60.0, 0.0, 1.0);
        assert!(leaning[1].abs() > 1.0);
        assert!(leaning[0].abs() < 1e-3);
    }

    #[test]
    fn leaning_the_other_way_mirrors_the_slide() {
        let right = held_view_light_point([0.0; 3], 0.0, 0.0, 1.0);
        let left = held_view_light_point([0.0; 3], 0.0, 0.0, -1.0);
        assert!((right[1] + left[1]).abs() < 1e-3);
    }
}
