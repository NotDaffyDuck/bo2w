//! The impact-effect table: which effect a hit plays, by impact type and
//! surface.
//!
//! The table has one row per kind of impact. A row holds an effect for each
//! of the 32 ordinary surfaces, then four for flesh (body or head, wounded
//! or killed). The surface comes from bits 20 to 25 of a trace's surface
//! flags.

/// How many ordinary surfaces a row has an effect for. The last (31) is the
/// riot shield.
pub const FX_IMPACT_NONFLESH_COUNT: usize = 32;

/// How many flesh effects a row ends with.
pub const FX_IMPACT_FLESH_COUNT: usize = 4;

/// The surface that is flesh.
pub const FX_SURF_TYPE_FLESH: usize = 7;

/// The surface-flag bit that marks a hit where the shot left the surface
/// rather than entered it.
pub const FX_IMPACT_EXIT_SURFACE_FLAG: u32 = 1 << 2;

/// One row of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxImpactEntry {
    /// The effect per ordinary surface (addresses).
    pub nonflesh: [u32; FX_IMPACT_NONFLESH_COUNT],
    /// The effect per flesh hit (addresses).
    pub flesh: [u32; FX_IMPACT_FLESH_COUNT],
}

/// Where the table's rows start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxImpactTable {
    /// Address of the first row.
    pub entries: u32,
}

/// The row an impact type (1 to 15) uses, and whether the shot went in or
/// came out. Some types have a row for each side. `None` for type 0 (no
/// impact) and anything past 15.
pub fn impact_table_row(impact_type: i32, exit: bool) -> Option<usize> {
    let side = usize::from(exit);
    Some(match impact_type {
        1 => side,
        2 => 3 + side,
        3 => 7 + side,
        4 => 9 + side,
        5 => 5 + side,
        6 => 11,
        7 | 8 => 12,
        9 => 13,
        10 => 14,
        11 => 15,
        12 => 16,
        13 => 17,
        14 => 19,
        15 => 20,
        _ => return None,
    })
}

/// Which of the four flesh effects a hit plays, from [`flesh_hit_flags`].
pub fn flesh_effect_index(hit_flags: u32) -> usize {
    let mut index = (hit_flags & 1) * 2;
    if hit_flags & 2 != 0 {
        index |= 1;
    }
    index as usize
}

/// The flags a flesh hit is looked up by: bit 0 for a head hit, bit 1 for a
/// kill.
pub const fn flesh_hit_flags(head: bool, fatal: bool) -> u32 {
    (head as u32) | ((fatal as u32) << 1)
}

/// The surface type in a trace's surface flags.
pub const fn surface_type_index(surface_flags: u32) -> usize {
    ((surface_flags >> 20) & 0x3f) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many rows the table has.
    const ROWS: usize = 21;

    #[test]
    fn the_surface_type_is_six_bits_from_bit_twenty() {
        assert_eq!(surface_type_index(31 << 20), 31);
        assert_eq!(surface_type_index(0x3f << 20), 0x3f);
        assert_eq!(surface_type_index((1 << 26) | (5 << 20)), 5);
        assert_eq!(surface_type_index(0x000f_ffff), 0);
    }

    #[test]
    fn every_impact_type_lands_inside_the_table() {
        for impact_type in 1..=15 {
            for exit in [false, true] {
                let row = impact_table_row(impact_type, exit).unwrap();
                assert!(row < ROWS, "type {impact_type} exit {exit}: row {row}");
            }
        }
        assert_eq!(impact_table_row(0, false), None);
        assert_eq!(impact_table_row(16, false), None);
        assert_eq!(impact_table_row(-1, true), None);
    }

    #[test]
    fn the_sided_types_use_neighbouring_rows() {
        for impact_type in [1, 2, 3, 4, 5] {
            let enter = impact_table_row(impact_type, false).unwrap();
            let exit = impact_table_row(impact_type, true).unwrap();
            assert_eq!(exit, enter + 1, "type {impact_type}");
        }
        assert_eq!(impact_table_row(6, false), impact_table_row(6, true));
    }

    #[test]
    fn grenade_bounce_and_explosion_share_a_row() {
        assert_eq!(impact_table_row(7, false), impact_table_row(8, true));
    }

    #[test]
    fn flesh_hits_pick_one_of_four_effects() {
        assert_eq!(flesh_effect_index(flesh_hit_flags(false, false)), 0);
        assert_eq!(flesh_effect_index(flesh_hit_flags(false, true)), 1);
        assert_eq!(flesh_effect_index(flesh_hit_flags(true, false)), 2);
        assert_eq!(flesh_effect_index(flesh_hit_flags(true, true)), 3);
        assert!(flesh_effect_index(flesh_hit_flags(true, true)) < FX_IMPACT_FLESH_COUNT);
    }
}
