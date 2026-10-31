//! Asset names as zones store them. A leading comma marks a reference: the
//! zone only points at an asset another zone defines.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetPointer {
    /// The zone carries the asset itself.
    Real(String),
    /// The zone names the asset but its body lives elsewhere.
    Borrowed(String),
}

const REFERENCE_MARK: char = ',';

impl AssetPointer {
    pub fn decode(stored: &str) -> Self {
        match Self::bare_name(stored) {
            bare if bare.len() < stored.len() => Self::Borrowed(bare.to_owned()),
            _ => Self::Real(stored.to_owned()),
        }
    }

    /// The name with any reference mark removed.
    pub fn bare_name(stored: &str) -> &str {
        match stored.split_once(REFERENCE_MARK) {
            Some(("", bare)) => bare,
            _ => stored,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Real(name) | Self::Borrowed(name) => name,
        }
    }

    pub fn is_genuine(&self) -> bool {
        !self.names_elsewhere()
    }

    pub fn names_elsewhere(&self) -> bool {
        match self {
            Self::Borrowed(_) => true,
            Self::Real(_) => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        str::is_empty(self.as_str())
    }

    pub fn spells_same(&self, twin: &Self) -> bool {
        self.as_str().eq(twin.as_str())
    }

    /// When two zones fill the same catalog slot, the newcomer wins unless it
    /// is only a reference and the slot already holds the real asset.
    pub fn incoming_owns_slot(held: &Self, newcomer: &Self) -> bool {
        !held.is_genuine() || newcomer.is_genuine()
    }
}

impl Default for AssetPointer {
    fn default() -> Self {
        Self::decode("")
    }
}

impl From<&str> for AssetPointer {
    fn from(stored: &str) -> Self {
        Self::decode(stored)
    }
}

impl From<String> for AssetPointer {
    fn from(stored: String) -> Self {
        if stored.starts_with(REFERENCE_MARK) {
            return Self::Borrowed(stored[REFERENCE_MARK.len_utf8()..].to_owned());
        }
        Self::Real(stored)
    }
}

impl AsRef<str> for AssetPointer {
    fn as_ref(&self) -> &str {
        let (Self::Real(text) | Self::Borrowed(text)) = self;
        text
    }
}

impl fmt::Display for AssetPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// A name compares equal to text that spells it, whichever kind it is.
macro_rules! equal_to_text {
    ($($text:ty),+) => {$(
        impl PartialEq<$text> for AssetPointer {
            fn eq(&self, rhs: &$text) -> bool {
                self.as_str() == AsRef::<str>::as_ref(rhs)
            }
        }
    )+};
}

equal_to_text!(str, &str, String);

/// How many names a load saw, split real against reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetPointerTally {
    pub n: usize,
    pub owned: usize,
    pub borrowed: usize,
}

impl AssetPointerTally {
    pub fn push(&mut self, entry: &AssetPointer) {
        self.n = self.n.saturating_add(1);
        let bucket = if entry.names_elsewhere() { &mut self.borrowed } else { &mut self.owned };
        *bucket += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_marks_a_reference() {
        let r = AssetPointer::decode(",mtl_gun");
        assert!(r.names_elsewhere());
        assert_eq!(r, "mtl_gun");
        assert_eq!(AssetPointer::bare_name(",mtl_gun"), "mtl_gun");
        assert!(AssetPointer::from("mtl_gun").is_genuine());
    }

    #[test]
    fn real_asset_keeps_its_slot_against_a_reference() {
        let real = AssetPointer::decode("a");
        let refr = AssetPointer::decode(",a");
        assert!(!AssetPointer::incoming_owns_slot(&real, &refr));
        assert!(AssetPointer::incoming_owns_slot(&refr, &real));
        assert!(AssetPointer::incoming_owns_slot(&real, &real));
    }

    #[test]
    fn census_counts_both_kinds() {
        let mut c = AssetPointerTally::default();
        c.push(&AssetPointer::decode("a"));
        c.push(&AssetPointer::decode(",b"));
        assert_eq!((c.n, c.owned, c.borrowed), (2, 1, 1));
    }
}
