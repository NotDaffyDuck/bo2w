//! Links from one asset to another after catalogs are built: either bound to
//! a catalog entry (and the zone it came from), left open for a known
//! reason, or never asked for.

use crate::{CatalogSlot, KeyDomain, ZoneCustodian};

/// Why a wanted link has no target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyCause {
    /// The source slot was a temporary that never carried a name to look up.
    ScratchFieldNoAlias,
    /// A name was there but no catalog entry carries it.
    IndexLookupFailure,
}

/// Where a bound link points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachedDestination<S: KeyDomain> {
    at: CatalogSlot<S>,
    from_zone: ZoneCustodian,
}

impl<S: KeyDomain> AttachedDestination<S> {
    pub fn index(self) -> CatalogSlot<S> {
        self.at
    }

    pub fn zone(self) -> ZoneCustodian {
        self.from_zone
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetDependencyLink<S: KeyDomain> {
    Bound(AttachedDestination<S>),
    Unresolved(DependencyCause),
    #[default]
    NothingThere,
}

impl<S: KeyDomain> AssetDependencyLink<S> {
    pub fn bind(at: CatalogSlot<S>, from_zone: ZoneCustodian) -> Self {
        Self::Bound(AttachedDestination { at, from_zone })
    }

    pub fn bind_order(slot: usize, from_zone: ZoneCustodian) -> Self {
        Self::bind(CatalogSlot::from_sequence(slot), from_zone)
    }

    /// Open link: `NothingThere` when the source had neither a slot nor a name,
    /// otherwise unresolved with the reason the name could not be used.
    pub fn unresolved_presence(had_slot: bool, had_name: bool) -> Self {
        if had_name {
            return Self::Unresolved(DependencyCause::IndexLookupFailure);
        }
        match had_slot {
            true => Self::Unresolved(DependencyCause::ScratchFieldNoAlias),
            false => Self::NothingThere,
        }
    }

    fn target(&self) -> Option<AttachedDestination<S>> {
        let Self::Bound(found) = *self else {
            return None;
        };
        Some(found)
    }

    pub fn bound(&self) -> Option<CatalogSlot<S>> {
        self.target().map(AttachedDestination::index)
    }

    pub fn extent_position(&self) -> Option<usize> {
        self.target().map(|t| t.index().order())
    }

    pub fn is_attached(&self) -> bool {
        self.target().is_some()
    }

    pub fn is_unlinked_ref(&self) -> bool {
        !(self.is_attached() || self.lacks_entry())
    }

    pub fn lacks_entry(&self) -> bool {
        *self == Self::NothingThere
    }

    /// Short label for load reports.
    pub fn dependency_cause_of(&self) -> &'static str {
        let Self::Unresolved(why) = self else {
            return if self.is_attached() { "bound" } else { "absent" };
        };
        match why {
            DependencyCause::ScratchFieldNoAlias => "unresolved:temp_field",
            DependencyCause::IndexLookupFailure => "unresolved:catalog_miss",
        }
    }
}

/// Comma list (sorted, no repeats) of the named zones the bound links came from.
pub fn linked_zone_names<'link, S: KeyDomain + 'link>(edges: impl IntoIterator<Item = &'link AssetDependencyLink<S>>) -> String {
    let mut names = std::collections::BTreeSet::new();
    for target in edges.into_iter().filter_map(AssetDependencyLink::target) {
        let zone = target.zone();
        if zone.is_classified() {
            names.insert(zone.as_str());
        }
    }
    names.into_iter().collect::<Vec<&'static str>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FxSlot, FxRange};

    #[test]
    fn open_link_reasons() {
        let missing = |in_temp, in_catalog| AssetDependencyLink::<FxRange>::unresolved_presence(in_temp, in_catalog);
        assert!(missing(false, false).lacks_entry());
        assert_eq!(missing(true, false).dependency_cause_of(), "unresolved:temp_field");
        assert_eq!(missing(false, true).dependency_cause_of(), "unresolved:catalog_miss");
    }

    #[test]
    fn bound_links_report_their_slot_and_kind() {
        let zone = ZoneCustodian::intern("zone_a");
        let bound = AssetDependencyLink::<FxRange>::bind(FxSlot::from_sequence(4), zone);
        assert_eq!(bound.extent_position(), Some(4));
        assert_eq!(AssetDependencyLink::<FxRange>::bind_order(4, zone), bound);
        assert_eq!(bound.dependency_cause_of(), "bound");
        assert!(bound.is_attached() && !bound.is_unlinked_ref() && !bound.lacks_entry());
        assert!(AssetDependencyLink::<FxRange>::unresolved_presence(true, true).is_unlinked_ref());
    }

    #[test]
    fn zone_names_come_from_bound_links_only() {
        let (late, early) = (ZoneCustodian::intern("zone_b"), ZoneCustodian::intern("zone_a"));
        let edges = [
            AssetDependencyLink::<FxRange>::bind_order(5, late),
            AssetDependencyLink::bind_order(6, early),
            AssetDependencyLink::bind_order(7, late),
            AssetDependencyLink::bind_order(8, ZoneCustodian::UNCLASSIFIED),
            AssetDependencyLink::NothingThere,
        ];
        assert_eq!("zone_a,zone_b", linked_zone_names(&edges));
    }
}
