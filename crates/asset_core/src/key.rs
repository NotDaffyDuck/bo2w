//! Text keys that name one asset: `<namespace>:<kind>/<name>`, e.g.
//! `t6:weapon/an94_mp`. Consoles, menus and tests use them to point at an
//! asset without holding a catalog index.

use std::fmt;

use crate::ZoneGame;

/// Which game's content tree an asset was read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AssetNamespace {
    /// Older content tree. No zone of this kind is read today; the variant
    /// stays while other crates still match on it.
    T5,
    #[default]
    T6,
}

const NAMESPACE_TOKENS: [(AssetNamespace, &str); 2] =
    [(AssetNamespace::T5, "t5"), (AssetNamespace::T6, "t6")];

impl AssetNamespace {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T5 => NAMESPACE_TOKENS[0].1,
            Self::T6 => NAMESPACE_TOKENS[1].1,
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        NAMESPACE_TOKENS
            .iter()
            .find(|(_, t)| *t == token)
            .map(|(ns, _)| *ns)
    }

    pub fn from_zone_game(game: ZoneGame) -> Self {
        match game {
            ZoneGame::T6 => Self::T6,
        }
    }
}

/// The asset families a key can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Weapon,
    Map,
    XModel,
    Anim,
    Material,
    Localize,
}

const KIND_TOKENS: [(AssetKind, &str); 6] = [
    (AssetKind::Weapon, "weapon"),
    (AssetKind::Map, "map"),
    (AssetKind::XModel, "xmodel"),
    (AssetKind::Anim, "anim"),
    (AssetKind::Material, "material"),
    (AssetKind::Localize, "localize"),
];

impl AssetKind {
    pub const fn as_str(self) -> &'static str {
        KIND_TOKENS[self as usize].1
    }

    pub fn parse(token: &str) -> Option<Self> {
        KIND_TOKENS
            .iter()
            .find(|(_, t)| *t == token)
            .map(|(kind, _)| *kind)
    }
}

/// A material looked up by name inside one namespace.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaterialKey {
    pub namespace: AssetNamespace,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetKey {
    pub namespace: AssetNamespace,
    pub kind: AssetKind,
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

impl AssetKey {
    pub fn new(
        namespace: AssetNamespace,
        kind: AssetKind,
        name: impl Into<String>,
    ) -> Result<Self, KeyError> {
        let name = name.into();
        if name.is_empty() || name.contains([':', '/']) {
            return Err(KeyError::BadName);
        }
        Ok(Self { namespace, kind, name })
    }

    /// Reads `ns:kind/name`. Only the first `/` splits kind from name, so a
    /// parsed name may itself hold more slashes.
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let (ns_token, rest) = text.split_once(':').ok_or(KeyError::NoNamespace)?;
        let namespace = AssetNamespace::parse(ns_token).ok_or(KeyError::BadNamespace)?;
        let (kind_token, name) = rest.split_once('/').ok_or(KeyError::BadKind)?;
        let kind = AssetKind::parse(kind_token).ok_or(KeyError::BadKind)?;
        if name.is_empty() {
            return Err(KeyError::BadName);
        }
        Ok(Self { namespace, kind, name: name.to_string() })
    }

    pub fn display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for AssetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ns, kind) = (self.namespace.as_str(), self.kind.as_str());
        write!(f, "{ns}:{kind}/{}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_round_trips() {
        let key = AssetKey::parse("t6:weapon/an94_mp").unwrap();
        assert_eq!(key.namespace, AssetNamespace::T6);
        assert_eq!(key.kind, AssetKind::Weapon);
        assert_eq!(key.name, "an94_mp");
        assert_eq!(key.display(), "t6:weapon/an94_mp");
    }

    #[test]
    fn key_refusals() {
        assert_eq!(AssetKey::parse("weapon/x"), Err(KeyError::NoNamespace));
        assert_eq!(AssetKey::parse("q3:weapon/x"), Err(KeyError::BadNamespace));
        assert_eq!(AssetKey::parse("t6:sound/x"), Err(KeyError::BadKind));
        assert_eq!(AssetKey::parse("t6:map/"), Err(KeyError::BadName));
        let made = AssetKey::new(AssetNamespace::T6, AssetKind::Map, "a/b");
        assert_eq!(made, Err(KeyError::BadName));
    }

    #[test]
    fn every_kind_token_parses_back() {
        for (kind, token) in KIND_TOKENS {
            assert_eq!(kind.as_str(), token);
            assert_eq!(AssetKind::parse(token), Some(kind));
        }
        assert_eq!(AssetNamespace::parse("t6"), Some(AssetNamespace::T6));
    }
}
