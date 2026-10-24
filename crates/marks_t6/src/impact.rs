//! Whether an impact leaves marks at all, and on what.

/// What an impact's marks are cut against. Each has its own material in the effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkFragmentsAgainst {
    WorldBrushes,
    Models,
}

/// How far from the camera an impact still leaves a mark, by default.
pub const FX_MARKS_RANGE_DEFAULT: f32 = 1500.0;

/// Marks are on (`fx_marks`) and the effect system was not told to make none.
pub const fn impact_mark_outer_gate(fx_marks: bool, no_marks: bool) -> bool {
    fx_marks && !no_marks
}

/// The impact is closer to the camera than `range`. With no camera yet every impact is in
/// range; a range of zero or less lets none through.
pub fn impact_mark_within_range(camera: Option<[f32; 3]>, origin: [f32; 3], range: f32) -> bool {
    let Some(camera) = camera else {
        return true;
    };
    if range <= 0.0 {
        return false;
    }
    let d = [
        origin[0] - camera[0],
        origin[1] - camera[1],
        origin[2] - camera[2],
    ];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range
}

/// An impact on a bolted effect (a stored bolt other than 0xff) leaves no world mark.
pub const fn impact_mark_skip_world_from_stored_bolt(bolt: u8) -> bool {
    bolt != 0xff
}

/// Marks go on models when entity marks or static-model marks are on.
pub const fn impact_mark_models_generate(fx_marks_ents: bool, fx_marks_smodels: bool) -> bool {
    fx_marks_ents || fx_marks_smodels
}

/// The material for the cut: the effect's materials are listed models first, then world.
pub fn impact_mark_material(
    materials: [Option<&str>; 2],
    against: MarkFragmentsAgainst,
) -> Option<&str> {
    match against {
        MarkFragmentsAgainst::WorldBrushes => materials[1],
        MarkFragmentsAgainst::Models => materials[0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates() {
        assert!(impact_mark_outer_gate(true, false));
        assert!(!impact_mark_outer_gate(true, true));
        assert!(!impact_mark_outer_gate(false, false));
        assert!(impact_mark_skip_world_from_stored_bolt(0));
        assert!(!impact_mark_skip_world_from_stored_bolt(0xff));
        assert!(impact_mark_models_generate(false, true));
        assert!(!impact_mark_models_generate(false, false));
    }

    #[test]
    fn range_is_strict_and_needs_a_camera() {
        assert!(impact_mark_within_range(None, [9e9; 3], 0.0));
        assert!(impact_mark_within_range(
            Some([0.0; 3]),
            [3.0, 4.0, 0.0],
            5.1
        ));
        assert!(!impact_mark_within_range(
            Some([0.0; 3]),
            [3.0, 4.0, 0.0],
            5.0
        ));
        assert!(!impact_mark_within_range(Some([0.0; 3]), [0.0; 3], 0.0));
    }

    #[test]
    fn world_and_model_materials() {
        let materials = [Some("mc_bullet"), Some("wc_bullet")];
        assert_eq!(
            impact_mark_material(materials, MarkFragmentsAgainst::WorldBrushes),
            Some("wc_bullet")
        );
        assert_eq!(
            impact_mark_material(materials, MarkFragmentsAgainst::Models),
            Some("mc_bullet")
        );
    }
}
