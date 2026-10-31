//! Zones: which game's zone format is read, and which loaded zone owns an
//! asset.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Zone formats this engine reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoneTitleKind {
    /// PC zones, version 147.
    #[default]
    T6,
}

impl ZoneTitleKind {
    /// Short tag used in file names and reports.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::T6 => "t6",
        }
    }

    pub fn from_prefix(tag: &str) -> Option<Self> {
        (tag == Self::T6.prefix()).then_some(Self::T6)
    }
}

/// Name of the zone an asset was loaded from. Cheap to copy: names are kept
/// once for the life of the process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ZoneCustodian(&'static str);

fn kept_names() -> &'static Mutex<HashSet<&'static str>> {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

impl ZoneCustodian {
    /// No zone known.
    pub const UNCLASSIFIED: Self = Self("");
    /// Owner used by tests and fixtures.
    pub const TEST: ZoneCustodian = ZoneCustodian("test");

    pub fn intern(label: &str) -> Self {
        if label.is_empty() {
            return Self::UNCLASSIFIED;
        }
        let mut names = kept_names().lock().unwrap_or_else(|e| e.into_inner());
        match names.get(label) {
            Some(&kept) => Self(kept),
            None => {
                let kept: &'static str = Box::leak(label.into());
                names.insert(kept);
                Self(kept)
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }

    pub fn is_classified(&self) -> bool {
        self.0 != Self::UNCLASSIFIED.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_same_owner() {
        let a = ZoneCustodian::intern("mp_nuketown_2020");
        let b = ZoneCustodian::intern(&String::from("mp_nuketown_2020"));
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.as_str(), b.as_str()));
        assert!(!ZoneCustodian::intern("").is_classified());
        assert!(a.is_classified() && ZoneCustodian::TEST.is_classified());
        assert_eq!(ZoneCustodian::default(), ZoneCustodian::UNCLASSIFIED);
    }

    #[test]
    fn zone_game_tag() {
        assert_eq!(ZoneTitleKind::from_prefix(ZoneTitleKind::T6.prefix()), Some(ZoneTitleKind::T6));
        assert_eq!(ZoneTitleKind::from_prefix("iw"), None);
    }
}
