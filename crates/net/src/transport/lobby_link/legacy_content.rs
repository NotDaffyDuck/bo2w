//! The two content packs the launcher still asks the core for by name.
//!
//! Temporary: once the launcher installs its own `ContentCatalog`, this file
//! goes. Bits 0 and 1 stay reserved on the wire.

use master_protocol::ContentFlags;

use super::BoxError;
use super::content::{ContentCatalog, ContentPack};

pub const CONTENT_T5: u8 = 1 << 2;
pub const CONTENT_T6: u8 = 1 << 3;

/// The catalog the launcher's packs make, for code that has none installed.
pub(super) fn catalog() -> ContentCatalog {
    ContentCatalog::new(
        vec![
            ContentPack { bit: CONTENT_T5, prefix: "t5", label: "t5" },
            ContentPack { bit: CONTENT_T6, prefix: "t6", label: "t6" },
        ],
        Some("t6"),
    )
}

pub const fn content_inventory(t5: bool) -> ContentFlags {
    ContentFlags(if t5 { CONTENT_T5 } else { 0 })
}

pub fn content_required_by_map(map: &str) -> Result<ContentFlags, BoxError> {
    catalog().mask_for_map(map).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_without_a_prefix_needs_the_default_pack() {
        assert_eq!(content_required_by_map("mp_raid").unwrap(), ContentFlags(CONTENT_T6));
        assert_eq!(content_required_by_map("t5:zombie_theater").unwrap(), ContentFlags(CONTENT_T5));
        assert!(content_required_by_map("x:zone").is_err());
    }

    #[test]
    fn launcher_packs_have_labels_and_strays_do_not() {
        assert_eq!(catalog().describe(ContentFlags(CONTENT_T6)), "t6");
        assert_eq!(catalog().describe(ContentFlags(1 << 7)), "unknown");
        assert_eq!(content_inventory(true), ContentFlags(CONTENT_T5));
        assert_eq!(content_inventory(false), ContentFlags(0));
    }
}
