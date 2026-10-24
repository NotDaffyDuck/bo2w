//! Moving clip-map surface flags into the layout the game logic reads.

/// The mantle bits of a Black Ops II surface flag word, and where the game
/// logic keeps them.
const SOURCE_MANTLE_ON: u32 = 0x0400_0000;
const SOURCE_MANTLE_OVER: u32 = 0x0800_0000;
const MANTLE_ON: u32 = 0x0200_0000;
const MANTLE_OVER: u32 = 0x0400_0000;
/// Every bit in the range the mantle flags occupy on either side.
const MANTLE_RANGE: u32 = 0x1e00_0000;

/// Re-files the mantle bits of a surface flag word. The source's mount bit is
/// not a mantle-over, so only the two named bits carry across; the rest of
/// the range is cleared and every other bit is left alone.
pub fn t5_surface_flags(flags: u32) -> u32 {
    let mut out = flags & !MANTLE_RANGE;
    if flags & SOURCE_MANTLE_ON != 0 {
        out |= MANTLE_ON;
    }
    if flags & SOURCE_MANTLE_OVER != 0 {
        out |= MANTLE_OVER;
    }
    out
}

/// Which of a box's six axial flags goes to which plane: plane `n` takes the
/// flags the box stored at `AXIAL_SOURCE[n]`.
const AXIAL_SOURCE: [usize; 6] = [3, 0, 4, 1, 5, 2];

/// Puts a box brush's six per-side flags in plane order.
pub(crate) fn t5_axial_plane_flags(per_side: [u32; 6]) -> [u32; 6] {
    AXIAL_SOURCE.map(|side| per_side[side])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mantle_bit_moves_down_one_place() {
        assert_eq!(t5_surface_flags(SOURCE_MANTLE_ON), MANTLE_ON);
        assert_eq!(t5_surface_flags(SOURCE_MANTLE_OVER), MANTLE_OVER);
        assert_eq!(t5_surface_flags(SOURCE_MANTLE_ON | SOURCE_MANTLE_OVER), MANTLE_ON | MANTLE_OVER);
    }

    #[test]
    fn the_rest_of_the_mantle_range_is_cleared_and_other_bits_stay() {
        assert_eq!(t5_surface_flags(0x1000_0000), 0);
        assert_eq!(t5_surface_flags(0x0200_0000), 0);
        assert_eq!(t5_surface_flags(0x0000_0123 | SOURCE_MANTLE_ON), 0x0000_0123 | MANTLE_ON);
        assert_eq!(t5_surface_flags(0xe100_0001), 0xe100_0001);
    }

    #[test]
    fn axial_flags_are_put_in_plane_order() {
        assert_eq!(t5_axial_plane_flags([10, 11, 12, 13, 14, 15]), [13, 10, 14, 11, 15, 12]);
    }
}
