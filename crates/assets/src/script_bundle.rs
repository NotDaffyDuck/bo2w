//! The script files a match is started from, found by module name.

use std::collections::{BTreeMap, BTreeSet};

use crate::script_names::collect_quoted;
use crate::script_set::T6ScriptSet;

/// The scripts and entity text of a match, plus the Black Ops II extras a
/// map brings.
#[derive(Clone, Debug, Default)]
pub struct ScriptSources {
    /// What a Black Ops II map adds. Not set for any other kind of map.
    pub t6: Option<T6ScriptSet>,
    modules: BTreeMap<String, Vec<u8>>,
    entities: Option<String>,
}

/// A string table a script reads: `cells` holds `rows` rows of `columns` cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptTable {
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<String>,
}

/// Returned by [`ScriptSources::tables`] while nothing supplies tables.
static NO_TABLES: BTreeMap<String, ScriptTable> = BTreeMap::new();

/// The key a module is stored under: forward slashes, lower case.
fn module_key(name: &str) -> String {
    name.replace('\\', "/").to_ascii_lowercase()
}

impl ScriptSources {
    /// The compiled or source bytes of `module`, which must already be in
    /// stored form (forward slashes, lower case).
    pub fn read(&self, module: &str) -> Result<Vec<u8>, String> {
        self.modules
            .get(module)
            .cloned()
            .ok_or_else(|| format!("missing script asset {module}.gsc"))
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// The map's entity text, when it has any.
    pub fn entities(&self) -> Option<&str> {
        self.entities.as_deref()
    }

    /// Standalone string tables. Nothing fills these in, so it is always
    /// empty; it stays until the session stops asking.
    pub fn tables(&self) -> &BTreeMap<String, ScriptTable> {
        &NO_TABLES
    }

    /// A standalone config file by name. Nothing fills these in, so there is
    /// never one; it stays until the session stops asking.
    pub fn config(&self, _name: &str) -> Option<&str> {
        None
    }

    /// Standalone shellshock files as (name, text). Nothing fills these in,
    /// so it is always empty; it stays until the session stops asking.
    pub fn shocks(&self) -> impl Iterator<Item = (&str, &str)> {
        std::iter::empty()
    }

    /// Every name the scripts and the entity text put in quotes, which is
    /// where a script says what it loads.
    pub(crate) fn asset_names(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for module in self.modules.values() {
            collect_quoted(module, &mut names);
        }
        if let Some(entities) = &self.entities {
            collect_quoted(entities.as_bytes(), &mut names);
        }
        names
    }

    pub(crate) fn insert_source(&mut self, module: &str, source: String) {
        self.modules.insert(module_key(module), source.into_bytes());
    }

    pub(crate) fn set_entities(&mut self, entities: String) {
        self.entities = Some(entities);
    }

    /// Lays `other` over this set: its modules replace same-named ones, and
    /// its entity text and Black Ops II extras replace ours when it has any.
    pub(crate) fn overlay(&mut self, other: Self) {
        self.modules.extend(other.modules);
        self.entities = other.entities.or(self.entities.take());
        if other.t6.is_some() {
            self.t6 = other.t6;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(modules: &[(&str, &str)]) -> ScriptSources {
        let mut sources = ScriptSources::default();
        for (name, text) in modules {
            sources.insert_source(name, (*text).to_owned());
        }
        sources
    }

    #[test]
    fn modules_are_stored_under_a_normalised_name() {
        let sources = with(&[("Maps\\MP\\Main", "x")]);
        assert_eq!(sources.read("maps/mp/main"), Ok(b"x".to_vec()));
        assert_eq!(sources.len(), 1);
        assert!(!sources.is_empty());
    }

    #[test]
    fn a_missing_module_names_itself() {
        let err = ScriptSources::default().read("maps/none").unwrap_err();
        assert_eq!(err, "missing script asset maps/none.gsc");
    }

    #[test]
    fn overlaying_replaces_by_name_and_keeps_the_rest() {
        let mut base = with(&[("a", "old"), ("b", "kept")]);
        base.set_entities("base".to_owned());
        base.overlay(with(&[("a", "new")]));
        assert_eq!(base.read("a"), Ok(b"new".to_vec()));
        assert_eq!(base.read("b"), Ok(b"kept".to_vec()));
        assert_eq!(base.entities.as_deref(), Some("base"));

        let mut later = ScriptSources::default();
        later.set_entities("later".to_owned());
        base.overlay(later);
        assert_eq!(base.entities.as_deref(), Some("later"));
    }

    #[test]
    fn asset_names_come_from_modules_and_entities() {
        let mut sources = with(&[("m", "load(\"fx/a\"); // \"hidden\"")]);
        sources.set_entities("{ \"model\" \"prop_b\" }".to_owned());
        let names: Vec<_> = sources.asset_names().into_iter().collect();
        assert_eq!(names, ["fx/a", "model", "prop_b"]);
    }
}
