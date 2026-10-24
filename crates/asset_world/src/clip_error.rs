//! Why a map's clip data could not be turned into collision tables.

use core::fmt;

/// A reason the collision tables could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipCollisionError {
    /// The zone kept no brush tables for the map.
    MissingTables,
    /// The brush bytes ended before the tables did.
    Truncated,
    /// There are more leaf-brush entries than a leaf can index.
    LeafbrushIndexOverflow,
    /// The Black Ops II clip map refused a table: which one, and which row.
    Store(clipmap_t6::StoreError),
}

impl fmt::Display for ClipCollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTables => f.write_str("the zone kept no clip brush tables"),
            Self::Truncated => f.write_str("the clip brush bytes end before the tables do"),
            Self::LeafbrushIndexOverflow => {
                f.write_str("the leaf-brush table is too long for a leaf's first-brush and count fields")
            }
            Self::Store(error) => write!(f, "BO2 {error}"),
        }
    }
}

impl std::error::Error for ClipCollisionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_reads_as_a_sentence() {
        for error in [
            ClipCollisionError::MissingTables,
            ClipCollisionError::Truncated,
            ClipCollisionError::LeafbrushIndexOverflow,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
