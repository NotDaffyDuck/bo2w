//! Depth-of-field settings a script asks the client for.

/// Check the six numbers of a depth-of-field request and turn them into the
/// setting the client takes. A range that closes on itself switches that side
/// off.
pub(crate) fn depth_of_field_values(
    near_start: f32,
    near_end: f32,
    far_start: f32,
    far_end: f32,
    near_blur: f32,
    far_blur: f32,
) -> Result<crate::ScriptDepthOfField, String> {
    let distances = [
        ("near start", near_start),
        ("near end", near_end),
        ("far start", far_start),
        ("far end", far_end),
    ];
    if let Some((what, _)) = distances.iter().find(|(_, d)| *d < 0.0) {
        return Err(format!("{what} must be >= 0"));
    }
    if !(4.0..=10.0).contains(&near_blur) {
        return Err("near blur should be between 4 and 10".into());
    }
    if far_blur < 0.0 || near_blur < far_blur {
        return Err("far blur should be >= 0 and <= near blur".into());
    }
    let near_on = near_end > near_start;
    let far_on = far_end > far_start && far_blur != 0.0;
    Ok(crate::ScriptDepthOfField {
        near_from: if near_on { near_start } else { 0.0 },
        near_to: if near_on { near_end } else { 0.0 },
        far_from: if far_on { far_start } else { 0.0 },
        far_to: if far_on { far_end } else { 0.0 },
        near_smear: near_blur,
        far_smear: far_blur,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_ranges_switch_a_side_off() {
        let dof = depth_of_field_values(10.0, 5.0, 100.0, 200.0, 6.0, 0.0).unwrap();
        assert_eq!((dof.near_from, dof.near_to), (0.0, 0.0));
        assert_eq!((dof.far_from, dof.far_to), (0.0, 0.0));
        let dof = depth_of_field_values(1.0, 5.0, 100.0, 200.0, 6.0, 2.0).unwrap();
        assert_eq!((dof.near_from, dof.near_to, dof.far_from, dof.far_to), (1.0, 5.0, 100.0, 200.0));
    }

    #[test]
    fn bad_numbers_are_named() {
        assert_eq!(depth_of_field_values(-1.0, 5.0, 1.0, 2.0, 6.0, 1.0).unwrap_err(), "near start must be >= 0");
        assert_eq!(depth_of_field_values(0.0, 5.0, 1.0, -2.0, 6.0, 1.0).unwrap_err(), "far end must be >= 0");
        assert_eq!(depth_of_field_values(0.0, 5.0, 1.0, 2.0, 3.0, 1.0).unwrap_err(), "near blur should be between 4 and 10");
        assert_eq!(depth_of_field_values(0.0, 5.0, 1.0, 2.0, 6.0, 7.0).unwrap_err(), "far blur should be >= 0 and <= near blur");
    }
}
