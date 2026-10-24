//! Which scripts a BO2 match starts from, and the commands they may call.

use crate::script::lang::{Catalog, Realm, SourceResolver};

/// The start-up roots and entries of a BO2 map's stub scripts. Install runs the
/// struct initializer itself, before the map's structs exist.
pub struct T6Startup {
    pub roots: Vec<String>,
    pub entries: Vec<String>,
}

impl T6Startup {
    /// `map` is the zone name; its own script joins the roots when the sources
    /// carry one.
    pub fn new(resolver: &impl SourceResolver, gametype: &str, map: &str) -> Self {
        let callbacks = "maps/mp/gametypes/_callbacksetup";
        let gametype = format!("maps/mp/gametypes/{gametype}");
        let map = format!("maps/mp/{map}");
        let mut roots = vec![
            "codescripts/delete".to_owned(),
            "codescripts/struct".to_owned(),
            callbacks.to_owned(),
            gametype.clone(),
        ];
        let mut entries = vec![format!("{gametype}::main")];
        if resolver.read_bytes(&map).is_ok() {
            entries.push(format!("{map}::main"));
            roots.push(map);
        }
        entries.push(format!("{callbacks}::codecallback_startgametype"));
        Self { roots, entries }
    }
}

impl Catalog {
    /// What BO2's start-up scripts may call.
    pub fn t6() -> Self {
        Catalog::new(Realm::T6, &super::natives::builtins())
    }
}
