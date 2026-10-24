//! Where the first-person weapon model is lit from.

/// Roll the view takes at full lean, in degrees.
const LEAN_ROLL: f32 = 16.0;
/// How far a full lean slides the viewpoint sideways, in inches.
const LEAN_SLIDE: f32 = 20.0;

/// The player's eye position, slid sideways by however far they are leaning.
pub fn viewmodel_lighting_origin(
    origin: [f32; 3],
    view_height_current: f32,
    view_yaw: f32,
    leanf: f32,
) -> [f32; 3] {
    let eye = [origin[0], origin[1], origin[2] + view_height_current];
    game_math::add_lean_to_position(eye, view_yaw, leanf, LEAN_ROLL, LEAN_SLIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_upright_lights_from_the_eye() {
        assert_eq!(
            viewmodel_lighting_origin([10.0, 20.0, 30.0], 60.0, 123.0, 0.0),
            [10.0, 20.0, 90.0]
        );
    }

    #[test]
    fn leaning_slides_the_origin_sideways_not_forward() {
        let leaning = viewmodel_lighting_origin([0.0; 3], 60.0, 0.0, 1.0);
        assert!(leaning[1].abs() > 1.0);
        assert!(leaning[0].abs() < 1e-3);
    }

    #[test]
    fn leaning_the_other_way_mirrors_the_slide() {
        let right = viewmodel_lighting_origin([0.0; 3], 0.0, 0.0, 1.0);
        let left = viewmodel_lighting_origin([0.0; 3], 0.0, 0.0, -1.0);
        assert!((right[1] + left[1]).abs() < 1e-3);
    }
}
