use super::definitions::{FxDefinitions, SelfContainedEffectDef};
use crate::AssetCategoryScope;

/// An effect's name together with the namespace that defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxName<'a> {
    pub namespace: AssetCategoryScope,
    pub name: &'a str,
}

impl<'a> FxName<'a> {
    pub fn new(scope: AssetCategoryScope, fx_name: &'a str) -> Self {
        Self { namespace: scope, name: fx_name }
    }

    /// An effect of the base game.
    pub fn engine(name: &'a str) -> Self {
        Self::new(AssetCategoryScope::T6, name)
    }

    /// The effect this names, if the catalog has one with parts to play.
    pub fn settle(self, catalog: &'a FxDefinitions) -> Option<&'a SelfContainedEffectDef> {
        catalog.resolve_def_in(self.namespace, self.name)
    }
}

impl std::fmt::Display for FxName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
