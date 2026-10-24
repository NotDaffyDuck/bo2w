use super::definitions::{FxDefinitions, OwnedFxEffectDef};
use crate::AssetNamespace;

/// An effect's name together with the namespace that defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxName<'a> {
    pub namespace: AssetNamespace,
    pub name: &'a str,
}

impl<'a> FxName<'a> {
    pub fn new(namespace: AssetNamespace, name: &'a str) -> Self {
        Self { namespace, name }
    }

    /// An effect of the base game.
    pub fn engine(name: &'a str) -> Self {
        Self::new(AssetNamespace::T6, name)
    }

    /// The effect this names, if the catalog has one with parts to play.
    pub fn resolve(self, catalog: &'a FxDefinitions) -> Option<&'a OwnedFxEffectDef> {
        catalog.resolve_def_in(self.namespace, self.name)
    }
}

impl std::fmt::Display for FxName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
