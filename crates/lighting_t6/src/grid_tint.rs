//! Blending and shrinking colour sets. A set is 168 bytes: the 56 texels of
//! the outer shell of a 4x4x4 cube, three bytes each.

/// Bytes in one colour set.
pub const TINT_PAYLOAD_LEN: usize = 0xa8;

/// Added before the shift so the blend rounds to nearest.
const ROUND_BEFORE_SHIFT: u16 = 0x7f;
/// Where the eight corner texels of the shell start, as byte offsets (each is
/// then red, green, blue).
const CORNER_TEXEL_OFFSETS: [usize; 8] = [0x00, 0x09, 0x24, 0x2d, 0x78, 0x81, 0x9c, 0xa5];

/// Blends colour sets by fixed-point weight (weights add up to 256) into
/// `out`. False when there is nothing to blend, the counts disagree, or a set
/// or `out` is not the right size.
pub fn sum_cell_tints(sets: &[&[u8]], mix: &[u16], dest: &mut [u8]) -> bool {
    if sets.is_empty()
        || sets.len() != mix.len()
        || dest.len() != TINT_PAYLOAD_LEN
        || sets
            .iter()
            .any(|row| row.len() != TINT_PAYLOAD_LEN)
    {
        return false;
    }
    let mut sums = [0u16; TINT_PAYLOAD_LEN];
    for (row, &weight) in sets.iter().zip(mix) {
        for (sum, &byte) in sums.iter_mut().zip(row.iter()) {
            *sum = sum.wrapping_add(u16::from(byte).wrapping_mul(weight));
        }
    }
    for (byte, sum) in dest.iter_mut().zip(sums) {
        *byte = (sum.wrapping_add(ROUND_BEFORE_SHIFT) >> 8) as u8;
    }
    true
}

/// A colour set boiled down to one colour: the average of its eight corner
/// texels, with the weight it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedCellTint {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub share: u8,
}

/// Averages the shell's corner texels. `None` if `colors` is shorter than a
/// colour set.
pub fn pack_cell_tints(colors: &[u8], weight: u8) -> Option<PackedCellTint> {
    if colors.len() < TINT_PAYLOAD_LEN {
        return None;
    }
    // Start at 4 so the shift by 3 (a divide by 8) rounds to nearest.
    let mut sums = [4u32; 3];
    for &offset in &CORNER_TEXEL_OFFSETS {
        for (channel, sum) in sums.iter_mut().enumerate() {
            *sum += u32::from(colors[offset + channel]);
        }
    }
    Some(PackedCellTint {
        r: (sums[0] >> 3) as u8,
        g: (sums[1] >> 3) as u8,
        b: (sums[2] >> 3) as u8,
        share: weight,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_filled(value: u8) -> [u8; TINT_PAYLOAD_LEN] {
        [value; TINT_PAYLOAD_LEN]
    }

    #[test]
    fn one_full_weight_set_comes_back_unchanged() {
        let row = set_filled(100);
        let mut dest = [0u8; TINT_PAYLOAD_LEN];
        assert!(sum_cell_tints(&[&row], &[256], &mut dest));
        assert_eq!(dest, row);
    }

    #[test]
    fn two_sets_blend_by_weight() {
        let dark = set_filled(0);
        let bright = set_filled(200);
        let mut dest = [0u8; TINT_PAYLOAD_LEN];
        assert!(sum_cell_tints(
            &[&dark, &bright],
            &[192, 64],
            &mut dest
        ));
        assert_eq!(dest, set_filled(50));
    }

    #[test]
    fn mismatched_or_missing_input_is_refused() {
        let row = set_filled(1);
        let short = [1u8; 10];
        let mut dest = [0u8; TINT_PAYLOAD_LEN];
        assert!(!sum_cell_tints(&[], &[], &mut dest));
        assert!(!sum_cell_tints(
            &[&row],
            &[128, 128],
            &mut dest
        ));
        assert!(!sum_cell_tints(&[&short], &[256], &mut dest));
        let mut small = [0u8; 4];
        assert!(!sum_cell_tints(&[&row], &[256], &mut small));
    }

    #[test]
    fn compress_averages_the_corner_texels() {
        let mut colors = set_filled(0);
        for offset in CORNER_TEXEL_OFFSETS {
            colors[offset] = 80;
            colors[offset + 1] = 40;
            colors[offset + 2] = 8;
        }
        let packed = pack_cell_tints(&colors, 0xff).unwrap();
        assert_eq!(
            packed,
            PackedCellTint {
                r: 80,
                g: 40,
                b: 8,
                share: 0xff
            }
        );
    }

    #[test]
    fn texels_off_the_corners_do_not_count() {
        let mut colors = set_filled(0);
        colors[3] = 255;
        colors[4] = 255;
        let packed = pack_cell_tints(&colors, 1).unwrap();
        assert_eq!((packed.r, packed.g, packed.b), (0, 0, 0));
    }

    #[test]
    fn the_average_rounds_to_nearest() {
        let mut colors = set_filled(0);
        // 3 over eight texels is 0.375: rounds down; 4 is 0.5: rounds up.
        colors[0] = 3;
        colors[1] = 4;
        let packed = pack_cell_tints(&colors, 0).unwrap();
        assert_eq!((packed.r, packed.g), (0, 1));
    }

    #[test]
    fn a_short_set_cannot_be_compressed() {
        assert_eq!(pack_cell_tints(&[0u8; 167], 0), None);
    }
}
