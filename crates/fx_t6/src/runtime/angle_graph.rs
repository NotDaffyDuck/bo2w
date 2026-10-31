//! Keeping a spinning element's drawn rotation from running ahead of the
//! simulation.

/// One over 255, as the f32 the engine used widened to f64. The byte-scaled
/// values in an element def (0..=255 of the life span) are turned back into
/// fractions with it.
pub(super) const FX_RECIP_255: f64 = 0.003_921_568_859_368_563;

/// A lead past this many milliseconds is cut hard instead of eased.
const EASE_LIMIT_MS: f64 = 300.0;
/// The most a cut lead may be left at.
const MAX_LEAD_MS: f64 = 150.0;
/// One over twice the ease limit, as an f32 widened to f64. It makes the
/// eased pull-back meet the hard cut exactly at the limit.
const EASE_RECIP: f64 = 0.001_666_666_707_023_978_2;

/// The time to draw the rotation at. `graph_byte` says how far through the
/// simulated time the rotation should be (out of 255); a draw time ahead of
/// that is pulled back, gently for a small lead and to a fixed lead past
/// 300 ms.
pub fn clamp_elem_rotation_time(paint_clock_ms: f32, sim_ms: f32, graph_byte: u8) -> f32 {
    let draw = f64::from(paint_clock_ms);
    let reference = f64::from(graph_byte) * f64::from(sim_ms) * FX_RECIP_255;
    let ahead = draw - reference;
    if 0.0 >= ahead {
        return paint_clock_ms;
    }
    if ahead > EASE_LIMIT_MS {
        return (reference + MAX_LEAD_MS) as f32;
    }
    (draw - ahead * ahead * EASE_RECIP) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_time_behind_the_reference_is_left_alone() {
        assert_eq!(clamp_elem_rotation_time(100.0, 1000.0, 255), 100.0);
        assert_eq!(clamp_elem_rotation_time(0.0, 1000.0, 0), 0.0);
    }

    #[test]
    fn a_small_lead_is_eased_back_a_little() {
        let got = clamp_elem_rotation_time(150.0, 1000.0, 0);
        assert!(got < 150.0 && got > 100.0, "{got}");
    }

    #[test]
    fn a_big_lead_is_cut_to_the_fixed_lead() {
        let got = clamp_elem_rotation_time(1000.0, 1000.0, 0);
        assert!((got - 150.0).abs() < 1e-3, "{got}");
    }

    #[test]
    fn the_byte_scales_the_reference() {
        // 128/255 of 1000 ms is about 501.96 ms; 600 ms leads it by about 98,
        // which eases back by about 16.
        let got = clamp_elem_rotation_time(600.0, 1000.0, 128);
        assert!((got - 583.98).abs() < 0.05, "{got}");
    }
}
