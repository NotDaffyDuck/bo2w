//! The engine commands a script is allowed to name.

use std::collections::BTreeMap;

use super::program::Realm;

/// Plain commands (`getdvar(..)`) and commands sent to a thing
/// (`player giveweapon(..)`) have separate name spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Namespace {
    Function,
    Method,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Builtin {
    pub namespace: Namespace,
    pub name: &'static str,
}

impl Builtin {
    pub const fn new(namespace: Namespace, called: &'static str) -> Self {
        Self { namespace, name: called }
    }
}

/// Every command one game's scripts may call.
#[derive(Clone, Debug)]
pub struct Catalog {
    realm: Realm,
    by_key: BTreeMap<(Namespace, &'static str), Builtin>,
}

impl Catalog {
    pub fn new(realm: Realm, builtins: &[Builtin]) -> Self {
        Self {
            realm,
            by_key: builtins
                .iter()
                .map(|builtin| ((builtin.namespace, builtin.name), *builtin))
                .collect(),
        }
    }

    pub fn realm(&self) -> Realm {
        self.realm
    }

    pub fn get(&self, namespace: Namespace, name: &str) -> Option<Builtin> {
        self.by_key
            .iter()
            .find(|((space, known), _)| *space == namespace && *known == name)
            .map(|(_, builtin)| *builtin)
    }
}
