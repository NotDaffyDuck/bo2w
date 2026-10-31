//! Turning the colour sets a lookup gathered into one answer: which way to
//! build the result, and the fixed-point weights to blend with.

/// Bytes in one colour set.
const COLORS_STRIDE: u32 = 0xa8;
/// Weights are blended as eighths of a byte: they must add up to this.
const FIXED_WEIGHT_TOTAL: i32 = 0x100;

/// The byte offset of colour set `index` in the grid's colour table.
pub const fn tint_table_start(index: u32) -> u32 {
    index.wrapping_mul(COLORS_STRIDE)
}

/// The colour set a grid falls back to: its last.
pub const fn fallback_tint_index(tint_key_total: u32) -> Option<u32> {
    tint_key_total.checked_sub(1)
}

/// Scales each weight to 0 to 256 and fixes the rounding so they add up to
/// exactly 256, by giving the difference to the largest (the last, on a tie).
/// `None` when there is nothing to blend or no room in `out`.
pub fn grid_quantized_mix_factors(
    mix: &[f32],
    inverse_total: f32,
    out: &mut [u16],
) -> Option<()> {
    if mix.is_empty() || mix.len() > out.len() {
        return None;
    }
    let scale = inverse_total * 256.0;
    let mut sum = 0i32;
    let mut largest = 0usize;
    let mut largest_value = 0u16;
    for (light_no, &share) in mix.iter().enumerate() {
        let scaled = libm::floorf(share * scale + 0.5);
        let value = scaled.clamp(0.0, 65535.0) as u16;
        out[light_no] = value;
        sum = sum.wrapping_add(i32::from(value));
        if value >= largest_value {
            largest_value = value;
            largest = light_no;
        }
    }
    let shortfall = FIXED_WEIGHT_TOTAL.wrapping_sub(sum);
    out[largest] = i32::from(out[largest]).wrapping_add(shortfall) as u16;
    Some(())
}

/// How the sampled point's colours get built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridProbeFallbackBranch {
    /// One colour set: copy it.
    AssignedByIndex,
    /// Several: blend them by weight.
    MixThenStore,
    /// None found, but the fallback colour set can stand in.
    UnassignedUseSlot,
    /// None found and no usable fallback: the default colours.
    UseDefaults,
}

/// The light index reported for a point that found no corners.
pub const GRID_SAMPLE_VACANT_SUN_BAND: u8 = 1;

/// The primary-light index a path hands back.
pub const fn point_primary_light(path: GridProbeFallbackBranch, picked: u8) -> u8 {
    match path {
        GridProbeFallbackBranch::AssignedByIndex | GridProbeFallbackBranch::MixThenStore => picked,
        GridProbeFallbackBranch::UnassignedUseSlot => GRID_SAMPLE_VACANT_SUN_BAND,
        GridProbeFallbackBranch::UseDefaults => 0,
    }
}

/// What decides a lookup that found no corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridProbeEmptyGuard {
    pub fall_back_to_default: bool,
    pub default_color_slot: u32,
    pub flag_absent_light_grid: bool,
    pub tint_key_total: u32,
}

/// Picks the path from how many distinct colour sets the lookup found.
pub fn pick_point_route(
    colors_found: u32,
    empty: GridProbeEmptyGuard,
) -> GridProbeFallbackBranch {
    match colors_found {
        0 => {
            let fallback_exists = empty.default_color_slot < empty.tint_key_total;
            let wants_default = empty.fall_back_to_default
                && empty.default_color_slot == 0
                && empty.flag_absent_light_grid;
            if fallback_exists && !wants_default {
                GridProbeFallbackBranch::UnassignedUseSlot
            } else {
                GridProbeFallbackBranch::UseDefaults
            }
        }
        1 => GridProbeFallbackBranch::AssignedByIndex,
        _ => GridProbeFallbackBranch::MixThenStore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(prefer: bool, fallback: u32, show: bool, count: u32) -> GridProbeEmptyGuard {
        GridProbeEmptyGuard {
            fall_back_to_default: prefer,
            default_color_slot: fallback,
            flag_absent_light_grid: show,
            tint_key_total: count,
        }
    }

    #[test]
    fn colour_sets_are_168_bytes_apart() {
        assert_eq!(tint_table_start(0), 0);
        assert_eq!(tint_table_start(3), 504);
    }

    #[test]
    fn the_default_colours_are_the_last_set() {
        assert_eq!(fallback_tint_index(5), Some(4));
        assert_eq!(fallback_tint_index(0), None);
    }

    #[test]
    fn blend_weights_add_up_to_256() {
        let mut out = [0u16; 8];
        grid_quantized_mix_factors(&[0.5, 0.25, 0.25], 1.0, &mut out).unwrap();
        assert_eq!(&out[..3], &[128, 64, 64]);
        // Thirds round down to 85 each; the largest (the last) takes the one left over.
        grid_quantized_mix_factors(&[1.0, 1.0, 1.0], 1.0 / 3.0, &mut out).unwrap();
        assert_eq!(&out[..3], &[85, 85, 86]);
        assert_eq!(out[..3].iter().map(|&v| u32::from(v)).sum::<u32>(), 256);
    }

    #[test]
    fn the_largest_weight_takes_the_correction() {
        let mut out = [0u16; 8];
        // 0.1 and 0.9 of 256 round to 26 and 230 = 256: nothing to fix.
        grid_quantized_mix_factors(&[0.1, 0.9], 1.0, &mut out).unwrap();
        assert_eq!(&out[..2], &[26, 230]);
        // Under-weight input is topped up on the biggest entry.
        grid_quantized_mix_factors(&[0.25, 0.5], 1.0, &mut out).unwrap();
        assert_eq!(&out[..2], &[64, 192]);
    }

    #[test]
    fn nothing_to_blend_or_too_many_weights_is_refused() {
        let mut out = [0u16; 2];
        assert_eq!(
            grid_quantized_mix_factors(&[], 1.0, &mut out),
            None
        );
        assert_eq!(
            grid_quantized_mix_factors(&[1.0, 1.0, 1.0], 1.0, &mut out),
            None
        );
    }

    #[test]
    fn one_set_is_copied_and_several_are_blended() {
        let empty = gate(false, 0, false, 4);
        assert_eq!(
            pick_point_route(1, empty),
            GridProbeFallbackBranch::AssignedByIndex
        );
        assert_eq!(
            pick_point_route(2, empty),
            GridProbeFallbackBranch::MixThenStore
        );
        assert_eq!(
            pick_point_route(8, empty),
            GridProbeFallbackBranch::MixThenStore
        );
    }

    #[test]
    fn with_none_found_an_in_range_fallback_is_used() {
        assert_eq!(
            pick_point_route(0, gate(false, 0, false, 4)),
            GridProbeFallbackBranch::UnassignedUseSlot
        );
    }

    #[test]
    fn with_none_found_an_out_of_range_fallback_means_default() {
        assert_eq!(
            pick_point_route(0, gate(false, 4, false, 4)),
            GridProbeFallbackBranch::UseDefaults
        );
        assert_eq!(
            pick_point_route(0, gate(false, 0, false, 0)),
            GridProbeFallbackBranch::UseDefaults
        );
    }

    #[test]
    fn preferring_the_default_needs_the_first_set_and_the_debug_view() {
        assert_eq!(
            pick_point_route(0, gate(true, 0, true, 4)),
            GridProbeFallbackBranch::UseDefaults
        );
        assert_eq!(
            pick_point_route(0, gate(true, 0, false, 4)),
            GridProbeFallbackBranch::UnassignedUseSlot
        );
        assert_eq!(
            pick_point_route(0, gate(true, 1, true, 4)),
            GridProbeFallbackBranch::UnassignedUseSlot
        );
    }

    #[test]
    fn each_path_reports_its_primary() {
        use GridProbeFallbackBranch::*;
        assert_eq!(point_primary_light(AssignedByIndex, 9), 9);
        assert_eq!(point_primary_light(MixThenStore, 9), 9);
        assert_eq!(
            point_primary_light(UnassignedUseSlot, 9),
            GRID_SAMPLE_VACANT_SUN_BAND
        );
        assert_eq!(point_primary_light(UseDefaults, 9), 0);
    }
}
