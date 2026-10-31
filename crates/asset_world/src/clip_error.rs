//! Why a map's clip data could not be turned into collision tables.

use core::fmt;

/// A reason the collision tables could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidsTraceFault {
    /// The zone kept no brush tables for the map.
    NoBrushTables,
    /// The brush bytes ended before the tables did.
    Truncated,
    /// There are more leaf-brush entries than a leaf can index.
    LeafSolidIndexExceeded,
    /// The Black Ops II clip map refused a table: which one, and which row.
    Store(clipmap_t6::StoreError),
}

impl fmt::Display for SolidsTraceFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBrushTables => f.write_str("the zone kept no clip brush tables"),
            Self::Truncated => f.write_str("the clip brush bytes end before the tables do"),
            Self::LeafSolidIndexExceeded => {
                f.write_str("the leaf-brush table is too long for a leaf's first-brush and count fields")
            }
            Self::Store(error) => write!(f, "BO2 {error}"),
        }
    }
}

impl std::error::Error for SolidsTraceFault {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_reads_as_a_sentence() {
        for error in [
            SolidsTraceFault::NoBrushTables,
            SolidsTraceFault::Truncated,
            SolidsTraceFault::LeafSolidIndexExceeded,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
