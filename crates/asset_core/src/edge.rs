//! Links from one asset to another after catalogs are built: either bound to
//! a catalog entry (and the zone it came from), left open for a known
//! reason, or never asked for.

use std::collections::BTreeSet;

use crate::{CatalogIndex, IndexSpace, ZoneOwner};

/// Why a wanted link has no target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetEdgeReason {
    /// The source slot was a temporary that never carried a name to look up.
    TempFieldNotAliasable,
    /// A name was there but no catalog entry carries it.
    CatalogMiss,
}

/// Where a bound link points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundTarget<S: IndexSpace> {
    at: CatalogIndex<S>,
    from_zone: ZoneOwner,
}

impl<S: IndexSpace> BoundTarget<S> {
    pub fn index(self) -> CatalogIndex<S> {
        self.at
    }

    pub fn zone(self) -> ZoneOwner {
        self.from_zone
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetEdge<S: IndexSpace> {
    Bound(BoundTarget<S>),
    Unresolved(AssetEdgeReason),
    #[default]
    Absent,
}

impl<S: IndexSpace> AssetEdge<S> {
    pub fn bind(at: CatalogIndex<S>, from_zone: ZoneOwner) -> Self {
        Self::Bound(BoundTarget { at, from_zone })
    }

    pub fn bind_order(slot: usize, from_zone: ZoneOwner) -> Self {
        Self::bind(CatalogIndex::from_order(slot), from_zone)
    }

    /// Open link: `Absent` when the source had neither a slot nor a name,
    /// otherwise unresolved with the reason the name could not be used.
    pub fn unresolved_presence(had_slot: bool, had_name: bool) -> Self {
        match (had_slot, had_name) {
            (false, false) => Self::Absent,
            (_, false) => Self::Unresolved(AssetEdgeReason::TempFieldNotAliasable),
            (_, true) => Self::Unresolved(AssetEdgeReason::CatalogMiss),
        }
    }

    fn target(&self) -> Option<BoundTarget<S>> {
        match *self {
            Self::Bound(t) => Some(t),
            _ => None,
        }
    }

    pub fn bound(&self) -> Option<CatalogIndex<S>> {
        self.target().map(BoundTarget::index)
    }

    pub fn bound_index(&self) -> Option<usize> {
        self.bound().map(CatalogIndex::order)
    }

    pub fn is_bound(&self) -> bool {
        self.target().is_some()
    }

    pub fn is_unresolved(&self) -> bool {
        matches!(self, Self::Unresolved(_))
    }

    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    /// Short label for load reports.
    pub fn edge_kind(&self) -> &'static str {
        match self {
            Self::Bound(_) => "bound",
            Self::Unresolved(AssetEdgeReason::TempFieldNotAliasable) => "unresolved:temp_field",
            Self::Unresolved(AssetEdgeReason::CatalogMiss) => "unresolved:catalog_miss",
            Self::Absent => "absent",
        }
    }

    pub fn report(&self) -> &'static str {
        self.edge_kind()
    }
}

/// Comma list (sorted, no repeats) of the named zones the bound links came from.
pub fn bound_zone_names<'a, S, I>(edges: I) -> String
where
    S: IndexSpace + 'a,
    I: IntoIterator<Item = &'a AssetEdge<S>>,
{
    let zones: BTreeSet<&'static str> = edges
        .into_iter()
        .filter_map(AssetEdge::target)
        .map(|t| t.zone())
        .filter(ZoneOwner::is_known)
        .map(ZoneOwner::as_str)
        .collect();
    zones.into_iter().collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FxIndex, FxSpace};

    #[test]
    fn open_link_reasons() {
        type E = AssetEdge<FxSpace>;
        assert!(E::unresolved_presence(false, false).is_absent());
        assert_eq!(E::unresolved_presence(true, false).edge_kind(), "unresolved:temp_field");
        assert_eq!(E::unresolved_presence(false, true).edge_kind(), "unresolved:catalog_miss");
    }

    #[test]
    fn zone_names_come_from_bound_links_only() {
        let a = ZoneOwner::intern("zone_b");
        let b = ZoneOwner::intern("zone_a");
        let edges = [
            AssetEdge::bind(FxIndex::from_order(4), a),
            AssetEdge::bind_order(5, b),
            AssetEdge::bind_order(6, a),
            AssetEdge::bind_order(7, ZoneOwner::UNKNOWN),
            AssetEdge::Absent,
        ];
        assert_eq!(edges[0].bound_index(), Some(4));
        assert_eq!(bound_zone_names(&edges), "zone_a,zone_b");
    }
}
