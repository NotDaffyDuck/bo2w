//! Turning the colour sets a lookup gathered into one answer: which way to
//! build the result, and the fixed-point weights to blend with.

/// Bytes in one colour set.
const COLORS_STRIDE: u32 = 0xa8;
/// Weights are blended as eighths of a byte: they must add up to this.
const FIXED_WEIGHT_TOTAL: i32 = 0x100;

/// The byte offset of colour set `index` in the grid's colour table.
pub const fn light_grid_colors_byte_offset(index: u32) -> u32 {
    index.wrapping_mul(COLORS_STRIDE)
}

/// The colour set a grid falls back to: its last.
pub const fn light_grid_default_colors_index(color_count: u32) -> Option<u32> {
    color_count.checked_sub(1)
}

/// Scales each weight to 0 to 256 and fixes the rounding so they add up to
/// exactly 256, by giving the difference to the largest (the last, on a tie).
/// `None` when there is nothing to blend or no room in `out`.
pub fn light_grid_fixed_point_blend_weights(
    weights: &[f32],
    inverse_total: f32,
    out: &mut [u16],
) -> Option<()> {
    if weights.is_empty() || weights.len() > out.len() {
        return None;
    }
    let scale = inverse_total * 256.0;
    let mut sum = 0i32;
    let mut largest = 0usize;
    let mut largest_value = 0u16;
    for (index, &weight) in weights.iter().enumerate() {
        let scaled = libm::floorf(weight * scale + 0.5);
        let value = scaled.clamp(0.0, 65535.0) as u16;
        out[index] = value;
        sum = sum.wrapping_add(i32::from(value));
        if value >= largest_value {
            largest_value = value;
            largest = index;
        }
    }
    let shortfall = FIXED_WEIGHT_TOTAL.wrapping_sub(sum);
    out[largest] = i32::from(out[largest]).wrapping_add(shortfall) as u16;
    Some(())
}

/// How the sampled point's colours get built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightGridAtPointPath {
    /// One colour set: copy it.
    SetFromIndex,
    /// Several: blend them by weight.
    BlendAndSet,
    /// None found, but the fallback colour set can stand in.
    MissingUseIndex,
    /// None found and no usable fallback: the default colours.
    SetDefault,
}

/// The light index reported for a point that found no corners.
pub const LIGHT_GRID_ATPOINT_EMPTY_PRIMARY: u8 = 1;

/// The primary-light index a path hands back.
pub const fn light_grid_atapoint_return_primary(path: LightGridAtPointPath, picked: u8) -> u8 {
    match path {
        LightGridAtPointPath::SetFromIndex | LightGridAtPointPath::BlendAndSet => picked,
        LightGridAtPointPath::MissingUseIndex => LIGHT_GRID_ATPOINT_EMPTY_PRIMARY,
        LightGridAtPointPath::SetDefault => 0,
    }
}

/// What decides a lookup that found no corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridAtPointEmptyGate {
    pub prefer_default_when_missing: bool,
    pub fallback_colors_index: u32,
    pub show_missing_light_grid: bool,
    pub color_count: u32,
}

/// Picks the path from how many distinct colour sets the lookup found.
pub fn light_grid_atapoint_select_path(
    colors_found: u32,
    empty: LightGridAtPointEmptyGate,
) -> LightGridAtPointPath {
    match colors_found {
        0 => {
            let fallback_exists = empty.fallback_colors_index < empty.color_count;
            let wants_default = empty.prefer_default_when_missing
                && empty.fallback_colors_index == 0
                && empty.show_missing_light_grid;
            if fallback_exists && !wants_default {
                LightGridAtPointPath::MissingUseIndex
            } else {
                LightGridAtPointPath::SetDefault
            }
        }
        1 => LightGridAtPointPath::SetFromIndex,
        _ => LightGridAtPointPath::BlendAndSet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(prefer: bool, fallback: u32, show: bool, count: u32) -> LightGridAtPointEmptyGate {
        LightGridAtPointEmptyGate {
            prefer_default_when_missing: prefer,
            fallback_colors_index: fallback,
            show_missing_light_grid: show,
            color_count: count,
        }
    }

    #[test]
    fn colour_sets_are_168_bytes_apart() {
        assert_eq!(light_grid_colors_byte_offset(0), 0);
        assert_eq!(light_grid_colors_byte_offset(3), 504);
    }

    #[test]
    fn the_default_colours_are_the_last_set() {
        assert_eq!(light_grid_default_colors_index(5), Some(4));
        assert_eq!(light_grid_default_colors_index(0), None);
    }

    #[test]
    fn blend_weights_add_up_to_256() {
        let mut out = [0u16; 8];
        light_grid_fixed_point_blend_weights(&[0.5, 0.25, 0.25], 1.0, &mut out).unwrap();
        assert_eq!(&out[..3], &[128, 64, 64]);
        // Thirds round down to 85 each; the largest (the last) takes the one left over.
        light_grid_fixed_point_blend_weights(&[1.0, 1.0, 1.0], 1.0 / 3.0, &mut out).unwrap();
        assert_eq!(&out[..3], &[85, 85, 86]);
        assert_eq!(out[..3].iter().map(|&v| u32::from(v)).sum::<u32>(), 256);
    }

    #[test]
    fn the_largest_weight_takes_the_correction() {
        let mut out = [0u16; 8];
        // 0.1 and 0.9 of 256 round to 26 and 230 = 256: nothing to fix.
        light_grid_fixed_point_blend_weights(&[0.1, 0.9], 1.0, &mut out).unwrap();
        assert_eq!(&out[..2], &[26, 230]);
        // Under-weight input is topped up on the biggest entry.
        light_grid_fixed_point_blend_weights(&[0.25, 0.5], 1.0, &mut out).unwrap();
        assert_eq!(&out[..2], &[64, 192]);
    }

    #[test]
    fn nothing_to_blend_or_too_many_weights_is_refused() {
        let mut out = [0u16; 2];
        assert_eq!(
            light_grid_fixed_point_blend_weights(&[], 1.0, &mut out),
            None
        );
        assert_eq!(
            light_grid_fixed_point_blend_weights(&[1.0, 1.0, 1.0], 1.0, &mut out),
            None
        );
    }

    #[test]
    fn one_set_is_copied_and_several_are_blended() {
        let empty = gate(false, 0, false, 4);
        assert_eq!(
            light_grid_atapoint_select_path(1, empty),
            LightGridAtPointPath::SetFromIndex
        );
        assert_eq!(
            light_grid_atapoint_select_path(2, empty),
            LightGridAtPointPath::BlendAndSet
        );
        assert_eq!(
            light_grid_atapoint_select_path(8, empty),
            LightGridAtPointPath::BlendAndSet
        );
    }

    #[test]
    fn with_none_found_an_in_range_fallback_is_used() {
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(false, 0, false, 4)),
            LightGridAtPointPath::MissingUseIndex
        );
    }

    #[test]
    fn with_none_found_an_out_of_range_fallback_means_default() {
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(false, 4, false, 4)),
            LightGridAtPointPath::SetDefault
        );
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(false, 0, false, 0)),
            LightGridAtPointPath::SetDefault
        );
    }

    #[test]
    fn preferring_the_default_needs_the_first_set_and_the_debug_view() {
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(true, 0, true, 4)),
            LightGridAtPointPath::SetDefault
        );
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(true, 0, false, 4)),
            LightGridAtPointPath::MissingUseIndex
        );
        assert_eq!(
            light_grid_atapoint_select_path(0, gate(true, 1, true, 4)),
            LightGridAtPointPath::MissingUseIndex
        );
    }

    #[test]
    fn each_path_reports_its_primary() {
        use LightGridAtPointPath::*;
        assert_eq!(light_grid_atapoint_return_primary(SetFromIndex, 9), 9);
        assert_eq!(light_grid_atapoint_return_primary(BlendAndSet, 9), 9);
        assert_eq!(
            light_grid_atapoint_return_primary(MissingUseIndex, 9),
            LIGHT_GRID_ATPOINT_EMPTY_PRIMARY
        );
        assert_eq!(light_grid_atapoint_return_primary(SetDefault, 9), 0);
    }
}
