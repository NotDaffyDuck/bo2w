//! Where a new element goes in the draw order.

/// The squared distance from the camera to a world point.
pub fn sort_dist_to_cam_sq(camera_origin: [f32; 3], scene_point: [f32; 3]) -> f32 {
    let [step_along_x, dy, dz] = core::array::from_fn(|k| camera_origin[k] - scene_point[k]);
    step_along_x * step_along_x + dy * dy + dz * dz
}

/// Whether an element already in the list is drawn before a new one. Element
/// types above 3 always are; an element with no visuals never is. Otherwise
/// the lower sort order goes first, and at equal orders the farther element
/// goes first.
pub fn existing_elem_sorts_before_new(
    resident_kind: u8,
    resident_visuals: u8,
    resident_rank: u8,
    resident_eye_sq: f32,
    incoming_rank: u8,
    incoming_eye_sq: f32,
) -> bool {
    match (resident_kind, resident_visuals) {
        (4.., _) => true,
        (_, 0) => false,
        _ => match resident_rank.cmp(&incoming_rank) {
            core::cmp::Ordering::Equal => incoming_eye_sq < resident_eye_sq,
            order => order.is_lt(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_is_squared() {
        assert_eq!(sort_dist_to_cam_sq([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]), 25.0);
    }

    #[test]
    fn high_types_always_go_first() {
        assert!(existing_elem_sorts_before_new(4, 0, 9, 0.0, 0, 100.0));
    }

    #[test]
    fn an_element_with_no_visuals_goes_after() {
        assert!(!existing_elem_sorts_before_new(1, 0, 0, 100.0, 5, 0.0));
    }

    #[test]
    fn the_sort_order_decides_before_the_distance() {
        assert!(existing_elem_sorts_before_new(1, 1, 2, 0.0, 3, 100.0));
        assert!(!existing_elem_sorts_before_new(1, 1, 4, 100.0, 3, 0.0));
    }

    #[test]
    fn at_the_same_order_the_farther_goes_first() {
        assert!(existing_elem_sorts_before_new(1, 1, 3, 50.0, 3, 10.0));
        assert!(!existing_elem_sorts_before_new(1, 1, 3, 10.0, 3, 50.0));
        assert!(!existing_elem_sorts_before_new(1, 1, 3, 10.0, 3, 10.0));
    }
}
