//! Zones: which game's zone format is read, and which loaded zone owns an
//! asset.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Zone formats this engine reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoneGame {
    /// PC zones, version 147.
    #[default]
    T6,
}

impl ZoneGame {
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
pub struct ZoneOwner(&'static str);

fn kept_names() -> &'static Mutex<HashSet<&'static str>> {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

impl ZoneOwner {
    /// No zone known.
    pub const UNKNOWN: Self = Self("");
    /// Owner used by tests and fixtures.
    pub const TEST: Self = Self("test");

    pub fn intern(name: &str) -> Self {
        if name.is_empty() {
            return Self::UNKNOWN;
        }
        let mut names = kept_names().lock().unwrap_or_else(|e| e.into_inner());
        match names.get(name) {
            Some(&kept) => Self(kept),
            None => {
                let kept: &'static str = Box::leak(name.into());
                names.insert(kept);
                Self(kept)
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }

    pub fn is_known(&self) -> bool {
        !self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_same_owner() {
        let a = ZoneOwner::intern("mp_nuketown_2020");
        let b = ZoneOwner::intern(&String::from("mp_nuketown_2020"));
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.as_str(), b.as_str()));
        assert!(!ZoneOwner::intern("").is_known());
        assert_eq!(ZoneOwner::default(), ZoneOwner::UNKNOWN);
    }

    #[test]
    fn zone_game_tag() {
        assert_eq!(ZoneGame::from_prefix(ZoneGame::T6.prefix()), Some(ZoneGame::T6));
        assert_eq!(ZoneGame::from_prefix("iw"), None);
    }
}
