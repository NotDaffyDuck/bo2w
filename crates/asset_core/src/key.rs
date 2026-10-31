//! Text keys that name one asset: `<namespace>:<kind>/<name>`, e.g.
//! `t6:weapon/an94_mp`. Consoles, menus and tests use them to point at an
//! asset without holding a catalog index.

use std::fmt;

use crate::ZoneTitleKind;

/// Which game's content tree an asset was read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AssetCategoryScope {
    /// Older content tree. No zone of this kind is read today; the variant
    /// stays while other crates still match on it.
    T5,
    #[default]
    T6,
}

const NAMESPACE_TOKENS: [(AssetCategoryScope, &str); 2] =
    [(AssetCategoryScope::T5, "t5"), (AssetCategoryScope::T6, "t6")];

impl AssetCategoryScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T5 => NAMESPACE_TOKENS[0].1,
            Self::T6 => NAMESPACE_TOKENS[1].1,
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        NAMESPACE_TOKENS
            .iter()
            .find(|(_, t)| *t == word)
            .map(|(ns, _)| *ns)
    }

    pub fn from_zone_game(game: ZoneTitleKind) -> Self {
        match game {
            ZoneTitleKind::T6 => Self::T6,
        }
    }
}

/// The asset families a key can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetVariety {
    Weapon,
    Map,
    XModel,
    Anim,
    Material,
    Localize,
}

const KIND_TOKENS: [(AssetVariety, &str); 6] = [
    (AssetVariety::Weapon, "weapon"),
    (AssetVariety::Map, "map"),
    (AssetVariety::XModel, "xmodel"),
    (AssetVariety::Anim, "anim"),
    (AssetVariety::Material, "material"),
    (AssetVariety::Localize, "localize"),
];

impl AssetVariety {
    pub const fn as_str(self) -> &'static str {
        KIND_TOKENS[self as usize].1
    }

    pub fn parse(word: &str) -> Option<Self> {
        KIND_TOKENS
            .iter()
            .find(|(_, t)| *t == word)
            .map(|pair| pair.0)
    }
}

/// A material looked up by name inside one namespace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaterialKey {
    pub namespace: AssetCategoryScope,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetIdentity {
    pub namespace: AssetCategoryScope,
    pub kind: AssetVariety,
    pub name: String,
}

/// Why a key string or key parts were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// No `ns:` in front.
    NoNamespace,
    BadNamespace,
    /// No `kind/`, or a kind this crate does not know.
    BadKind,
    /// Empty name, or a name holding a `:` or `/` separator.
    BadName,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self {
            Self::NoNamespace => "no namespace (expected `t6:kind/name`)",
            Self::BadNamespace => "namespace not known",
            Self::BadKind => "kind missing or not known",
            Self::BadName => "name empty or holds a separator",
        };
        write!(f, "asset key: {why}")
    }
}

impl std::error::Error for KeyError {}

impl AssetIdentity {
    pub fn new(
        namespace: AssetCategoryScope,
        kind: AssetVariety,
        name: impl Into<String>,
    ) -> Result<AssetIdentity, KeyError> {
        let name: String = name.into();
        let usable = !name.is_empty() && !name.contains([':', '/']);
        usable
            .then(|| Self { namespace, kind, name })
            .ok_or(KeyError::BadName)
    }

    /// Reads `ns:kind/name`. Only the first `/` splits kind from name, so a
    /// parsed name may itself hold more slashes.
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let (ns_token, rest) = text.split_once(':').ok_or(KeyError::NoNamespace)?;
        let namespace = AssetCategoryScope::parse(ns_token).ok_or(KeyError::BadNamespace)?;
        let (kind_token, name) = rest.split_once('/').ok_or(KeyError::BadKind)?;
        let kind = AssetVariety::parse(kind_token).ok_or(KeyError::BadKind)?;
        if name.is_empty() {
            return Err(KeyError::BadName);
        }
        Ok(Self { namespace, kind, name: name.to_string() })
    }

    pub fn display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for AssetIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ns, kind) = (self.namespace.as_str(), self.kind.as_str());
        let name = &self.name;
        write!(f, "{ns}:{kind}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_round_trips() {
        let key = AssetIdentity::parse("t6:weapon/an94_mp").unwrap();
        assert_eq!(key.namespace, AssetCategoryScope::T6);
        assert_eq!(key.kind, AssetVariety::Weapon);
        assert_eq!(key.name, "an94_mp");
        assert_eq!(key.display(), "t6:weapon/an94_mp");
    }

    #[test]
    fn key_refusals() {
        assert_eq!(AssetIdentity::parse("weapon/x"), Err(KeyError::NoNamespace));
        assert_eq!(AssetIdentity::parse("q3:weapon/x"), Err(KeyError::BadNamespace));
        assert_eq!(AssetIdentity::parse("t6:sound/x"), Err(KeyError::BadKind));
        assert_eq!(AssetIdentity::parse("t6:map/"), Err(KeyError::BadName));
        let made = AssetIdentity::new(AssetCategoryScope::T6, AssetVariety::Map, "a/b");
        assert_eq!(made, Err(KeyError::BadName));
        let empty = AssetIdentity::new(AssetCategoryScope::T6, AssetVariety::Map, "");
        assert_eq!(empty, Err(KeyError::BadName));
        let colon = AssetIdentity::new(AssetCategoryScope::T6, AssetVariety::Map, "a:b");
        assert_eq!(colon, Err(KeyError::BadName));
        let ok = AssetIdentity::new(AssetCategoryScope::T6, AssetVariety::Map, "mp_x").unwrap();
        assert_eq!(ok.to_string(), "t6:map/mp_x");
    }

    #[test]
    fn every_kind_token_parses_back() {
        for (kind, token) in KIND_TOKENS {
            assert_eq!(kind.as_str(), token);
            assert_eq!(AssetVariety::parse(token), Some(kind));
        }
        assert_eq!(AssetCategoryScope::parse("t6"), Some(AssetCategoryScope::T6));
    }
}
