//! Tinting an element's colour with the lighting sampled where it is.

/// Where each sampled colour channel (red, green, blue) lands in a BGRA
/// colour.
const SAMPLE_TO_BGRA: [usize; 3] = [2_usize, 1, 0];

/// Whether an effect def's flags ask for a lighting sample.
pub const fn effect_def_needs_lighting_sample(flags: i32) -> bool {
    flags & 1 != 0
}

/// Whether an element lets lighting tint it.
pub const fn elem_uses_lighting_frac(shading_blend: u8) -> bool {
    shading_blend != 0
}

/// One channel of `color` lit by `sample`. A sample of 128 leaves it about
/// as it was; darker samples dim it and brighter ones brighten it, by as much
/// as `shading_blend` allows.
fn light_channel(color: u8, sample: u8, shading_blend: u8) -> u8 {
    let swing = (i32::from(sample) * 2 - 255) * i32::from(shading_blend) / 255;
    let lit = swing + 255;
    (lit * i32::from(color) / 255).clamp(0, 255) as u8
}

/// A BGRA colour tinted by an RGB lighting sample. A `shading_blend` of zero
/// leaves the colour alone; alpha is never touched.
pub fn apply_lighting_frac_bgra(
    mut bgra: [u8; 4],
    probe_rgb: [u8; 3],
    shading_blend: u8,
) -> [u8; 4] {
    if shading_blend == 0 {
        return bgra;
    }
    for (sample, slot) in probe_rgb.into_iter().zip(SAMPLE_TO_BGRA) {
        bgra[slot] = light_channel(bgra[slot], sample, shading_blend);
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lighting_fraction_leaves_the_colour_alone() {
        assert_eq!(
            apply_lighting_frac_bgra([10, 20, 30, 40], [0, 0, 0], 0),
            [10, 20, 30, 40]
        );
    }

    #[test]
    fn full_dark_sample_at_full_fraction_blacks_the_channels() {
        let got = apply_lighting_frac_bgra([200, 200, 200, 77], [0, 0, 0], 255);
        assert_eq!(got, [0, 0, 0, 77]);
    }

    #[test]
    fn full_bright_sample_at_full_fraction_doubles_then_clamps() {
        let got = apply_lighting_frac_bgra([200, 100, 50, 9], [255, 255, 255], 255);
        assert_eq!(got, [255, 200, 100, 9]);
    }

    #[test]
    fn the_sample_is_red_green_blue_and_the_colour_is_bgra() {
        // Only the red sample is dark; that must black the colour's third
        // byte (red in BGRA) and leave blue and green about as they were.
        let got = apply_lighting_frac_bgra([100, 100, 100, 100], [0, 128, 128], 255);
        assert_eq!(got, [100, 100, 0, 100]);
    }

    #[test]
    fn the_flags_bit_asks_for_a_sample() {
        assert!(effect_def_needs_lighting_sample(1));
        assert!(effect_def_needs_lighting_sample(0x13));
        assert!(!effect_def_needs_lighting_sample(2));
        assert!(elem_uses_lighting_frac(1));
        assert!(!elem_uses_lighting_frac(0));
    }
}
