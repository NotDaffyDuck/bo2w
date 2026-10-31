//! The per-cell record the light grid keeps in its entry table.

/// One grid cell: the colour set to blend, the strongest light reaching the
/// cell, and a bit per corner saying whether that corner wants a sight check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridCellEntry {
    pub vertex_color_slot: u16,
    pub key_light_index: u8,
    pub wants_ray_check: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_compares_by_all_three_fields() {
        let entry = GridCellEntry {
            vertex_color_slot: 4,
            key_light_index: 2,
            wants_ray_check: 0x81,
        };
        assert_eq!(entry, entry);
        assert_ne!(
            entry,
            GridCellEntry {
                wants_ray_check: 0,
                ..entry
            }
        );
    }
}
