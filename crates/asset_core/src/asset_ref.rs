//! Asset names as zones store them. A leading comma marks a reference: the
//! zone only points at an asset another zone defines.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetRef {
    /// The zone carries the asset itself.
    Real(String),
    /// The zone names the asset but its body lives elsewhere.
    Reference(String),
}

const REFERENCE_MARK: char = ',';

impl AssetRef {
    pub fn decode(stored: &str) -> Self {
        match stored.strip_prefix(REFERENCE_MARK) {
            Some(name) => Self::Reference(name.to_string()),
            None => Self::Real(stored.to_string()),
        }
    }

    /// The name with any reference mark removed.
    pub fn bare_name(stored: &str) -> &str {
        stored.strip_prefix(REFERENCE_MARK).unwrap_or(stored)
    }

    pub fn as_str(&self) -> &str {
        let (Self::Real(name) | Self::Reference(name)) = self;
        name
    }

    pub fn is_real(&self) -> bool {
        matches!(self, Self::Real(_))
    }

    pub fn is_reference(&self) -> bool {
        !self.is_real()
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn same_name(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }

    /// When two zones fill the same catalog slot, the newcomer wins unless it
    /// is only a reference and the slot already holds the real asset.
    pub fn incoming_owns_slot(held: &Self, incoming: &Self) -> bool {
        !(held.is_real() && incoming.is_reference())
    }
}

impl Default for AssetRef {
    fn default() -> Self {
        Self::Real(String::new())
    }
}

impl From<&str> for AssetRef {
    fn from(stored: &str) -> Self {
        Self::decode(stored)
    }
}

impl From<String> for AssetRef {
    fn from(stored: String) -> Self {
        Self::decode(&stored)
    }
}

impl AsRef<str> for AssetRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for AssetRef {
    fn eq(&self, text: &str) -> bool {
        self.as_str() == text
    }
}

impl PartialEq<&str> for AssetRef {
    fn eq(&self, text: &&str) -> bool {
        self.as_str() == *text
    }
}

impl PartialEq<String> for AssetRef {
    fn eq(&self, text: &String) -> bool {
        self.as_str() == text
    }
}

/// How many names a load saw, split real against reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetRefCensus {
    pub n: usize,
    pub real: usize,
    pub reference: usize,
}

impl AssetRefCensus {
    pub fn push(&mut self, name: &AssetRef) {
        self.n += 1;
        let bucket = if name.is_real() { &mut self.real } else { &mut self.reference };
        *bucket += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_marks_a_reference() {
        let r = AssetRef::decode(",mtl_gun");
        assert!(r.is_reference());
        assert_eq!(r, "mtl_gun");
        assert_eq!(AssetRef::bare_name(",mtl_gun"), "mtl_gun");
        assert!(AssetRef::from("mtl_gun").is_real());
    }

    #[test]
    fn real_asset_keeps_its_slot_against_a_reference() {
        let real = AssetRef::decode("a");
        let refr = AssetRef::decode(",a");
        assert!(!AssetRef::incoming_owns_slot(&real, &refr));
        assert!(AssetRef::incoming_owns_slot(&refr, &real));
        assert!(AssetRef::incoming_owns_slot(&real, &real));
    }

    #[test]
    fn census_counts_both_kinds() {
        let mut c = AssetRefCensus::default();
        c.push(&AssetRef::decode("a"));
        c.push(&AssetRef::decode(",b"));
        assert_eq!((c.n, c.real, c.reference), (2, 1, 1));
    }
}
