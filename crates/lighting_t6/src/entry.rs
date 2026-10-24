//! The per-cell record the light grid keeps in its entry table.

/// One grid cell: the colour set to blend, the strongest light reaching the
/// cell, and a bit per corner saying whether that corner wants a sight check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxLightGridEntry {
    pub colors_index: u16,
    pub primary_light_index: u8,
    pub needs_trace: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_compares_by_all_three_fields() {
        let entry = GfxLightGridEntry {
            colors_index: 4,
            primary_light_index: 2,
            needs_trace: 0x81,
        };
        assert_eq!(entry, entry);
        assert_ne!(
            entry,
            GfxLightGridEntry {
                needs_trace: 0,
                ..entry
            }
        );
    }
}
