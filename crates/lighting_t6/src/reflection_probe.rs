//! Choosing which reflection probe lights a point: the nearest one.

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}

/// The nearest probe to `point`, never probe 0 (the level-wide default). With
/// no probe past it, 0.
pub fn nearest_reflection_probe_skip_0(origins: &[[f32; 3]], point: [f32; 3]) -> u8 {
    let mut best = 0u8;
    let mut best_distance = f32::MAX;
    for (index, &origin) in origins.iter().enumerate().skip(1) {
        let distance = distance_squared(point, origin);
        if distance < best_distance {
            best = index as u8;
            best_distance = distance;
        }
    }
    best
}

/// The nearest probe to `point` among those a cell lists. Indices past the
/// probe table are ignored; an empty list gives 0.
pub fn nearest_reflection_probe_in_cell_list(
    origins: &[[f32; 3]],
    indices: &[u8],
    point: [f32; 3],
) -> u8 {
    let mut best = 0u8;
    let mut best_distance = f32::MAX;
    for &index in indices {
        let Some(&origin) = origins.get(usize::from(index)) else {
            continue;
        };
        let distance = distance_squared(point, origin);
        if distance < best_distance {
            best = index;
            best_distance = distance;
        }
    }
    best
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
        assert_eq!(nearest_reflection_probe_skip_0(&ORIGINS, [0.0; 3]), 2);
    }

    #[test]
    fn the_nearest_probe_past_the_default_wins() {
        assert_eq!(
            nearest_reflection_probe_skip_0(&ORIGINS, [-5.0, 0.0, 0.0]),
            3
        );
        assert_eq!(
            nearest_reflection_probe_skip_0(&ORIGINS, [9.0, 0.0, 0.0]),
            1
        );
    }

    #[test]
    fn with_no_probe_past_the_default_the_answer_is_zero() {
        assert_eq!(nearest_reflection_probe_skip_0(&ORIGINS[..1], [1.0; 3]), 0);
        assert_eq!(nearest_reflection_probe_skip_0(&[], [1.0; 3]), 0);
    }

    #[test]
    fn the_first_of_two_equally_near_probes_wins() {
        let origins = [[0.0; 3], [2.0, 0.0, 0.0], [-2.0, 0.0, 0.0]];
        assert_eq!(nearest_reflection_probe_skip_0(&origins, [0.0; 3]), 1);
    }

    #[test]
    fn a_cell_list_only_considers_the_probes_it_names() {
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[1, 3], [2.0, 0.0, 0.0]),
            1
        );
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[0, 1, 2], [2.0, 0.0, 0.0]),
            2
        );
        // A cell list may name the default probe, and then it can win.
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[0, 1, 2], [-1.0, 0.0, 0.0]),
            0
        );
    }

    #[test]
    fn probes_past_the_table_are_skipped() {
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[9, 1], [0.0; 3]),
            1
        );
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[9], [0.0; 3]),
            0
        );
    }

    #[test]
    fn an_empty_cell_list_gives_zero() {
        assert_eq!(
            nearest_reflection_probe_in_cell_list(&ORIGINS, &[], [0.0; 3]),
            0
        );
    }
}
