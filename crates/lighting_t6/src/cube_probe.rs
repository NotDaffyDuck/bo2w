//! Choosing which reflection probe lights a point: the nearest one.

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (step_along_x, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    step_along_x * step_along_x + dy * dy + dz * dz
}

/// The nearest probe to `point`, never probe 0 (the level-wide default). With
/// no probe past it, 0.
pub fn nearest_cube_probe(spots: &[[f32; 3]], at: [f32; 3]) -> u8 {
    let mut nearest = 0u8;
    let mut best_distance = f32::MAX;
    for (index, &origin) in spots.iter().enumerate().skip(1) {
        let distance = distance_squared(at, origin);
        if distance < best_distance {
            nearest = index as u8;
            best_distance = distance;
        }
    }
    nearest
}

/// The nearest probe to `point` among those a cell lists. Indices past the
/// probe table are ignored; an empty list gives 0.
pub fn nearest_cube_in_cells(
    spots: &[[f32; 3]],
    picks: &[u8],
    at: [f32; 3],
) -> u8 {
    let mut nearest = 0u8;
    let mut best_distance = f32::MAX;
    for &light_no in picks {
        let Some(&origin) = spots.get(usize::from(light_no)) else {
            continue;
        };
        let distance = distance_squared(at, origin);
        if distance < best_distance {
            nearest = light_no;
            best_distance = distance;
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINS: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [-6.0, 0.0, 0.0],
    ];

    #[test]
    fn the_default_probe_is_never_chosen_by_distance() {
        assert_eq!(nearest_cube_probe(&ORIGINS, [0.0; 3]), 2);
    }

    #[test]
    fn the_nearest_probe_past_the_default_wins() {
        assert_eq!(
            nearest_cube_probe(&ORIGINS, [-5.0, 0.0, 0.0]),
            3
        );
        assert_eq!(
            nearest_cube_probe(&ORIGINS, [9.0, 0.0, 0.0]),
            1
        );
    }

    #[test]
    fn with_no_probe_past_the_default_the_answer_is_zero() {
        assert_eq!(nearest_cube_probe(&ORIGINS[..1], [1.0; 3]), 0);
        assert_eq!(nearest_cube_probe(&[], [1.0; 3]), 0);
    }

    #[test]
    fn the_first_of_two_equally_near_probes_wins() {
        let origins = [[0.0; 3], [2.0, 0.0, 0.0], [-2.0, 0.0, 0.0]];
        assert_eq!(nearest_cube_probe(&origins, [0.0; 3]), 1);
    }

    #[test]
    fn a_cell_list_only_considers_the_probes_it_names() {
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[1, 3], [2.0, 0.0, 0.0]),
            1
        );
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[0, 1, 2], [2.0, 0.0, 0.0]),
            2
        );
        // A cell list may name the default probe, and then it can win.
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[0, 1, 2], [-1.0, 0.0, 0.0]),
            0
        );
    }

    #[test]
    fn probes_past_the_table_are_skipped() {
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[9, 1], [0.0; 3]),
            1
        );
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[9], [0.0; 3]),
            0
        );
    }

    #[test]
    fn an_empty_cell_list_gives_zero() {
        assert_eq!(
            nearest_cube_in_cells(&ORIGINS, &[], [0.0; 3]),
            0
        );
    }
}
