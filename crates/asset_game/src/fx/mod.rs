//! Effects: what each one is made of, found by name.

mod definitions;
mod name;
mod t6;
mod visual;

use std::collections::HashMap;

pub use definitions::{FxCatalog, FxDefinitions, OwnedFxEffectDef};
pub use name::FxName;
pub use t6::{T6_FIRST_PERSON_FLASH_SCALE, t6_elem_type, t6_first_person_flash_name};
pub use visual::{FxBankSound, MaterialSlot, OwnedFxElemDef, OwnedFxTrailDef, OwnedFxVisual};

use crate::AssetRef;

/// The kinds of effect element, by the number the zones store.
pub mod elem_type {
    pub const BILLBOARD: u8 = 0;
    pub const ORIENTED: u8 = 1;
    pub const TAIL: u8 = 2;
    pub const TRAIL: u8 = 3;
    pub const CLOUD: u8 = 4;
    pub const MODEL: u8 = 7;
    pub const OMNI_LIGHT: u8 = 8;
    pub const SPOT_LIGHT: u8 = 9;
    pub const SOUND: u8 = 10;
    pub const DECAL: u8 = 11;
    pub const RUNNER: u8 = 12;
}

/// A material name as the colour-map table keys it: without a reference mark.
#[inline]
pub fn material_bind_name(name: &str) -> &str {
    AssetRef::bare_name(name)
}

/// Files an effect material's colour image under its bind name; the first
/// image for a name stays.
pub fn insert_fx_color_image<V>(map: &mut HashMap<String, V>, authored_name: &str, image: V) {
    let bind = material_bind_name(authored_name);
    if !bind.is_empty() {
        map.entry(bind.to_owned()).or_insert(image);
    }
}

/// Gives every image filed under a marked name a copy under the bare name too,
/// unless the bare name has an image of its own. Returns how many were added.
pub fn alias_fx_color_map_stubs<V: Clone>(map: &mut HashMap<String, V>) -> usize {
    let aliases: Vec<(String, V)> = map
        .iter()
        .filter_map(|(name, image)| {
            let bare = material_bind_name(name);
            (bare != name && !map.contains_key(bare)).then(|| (bare.to_owned(), image.clone()))
        })
        .collect();
    let added = aliases.len();
    map.extend(aliases);
    added
}

pub fn lookup_fx_color_image<'a, V>(map: &'a HashMap<String, V>, authored_name: &str) -> Option<&'a V> {
    map.get(material_bind_name(authored_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_name_is_filed_under_its_bare_name() {
        let mut map = HashMap::new();
        insert_fx_color_image(&mut map, ",wisp", 1);
        insert_fx_color_image(&mut map, "wisp", 2);
        insert_fx_color_image(&mut map, "", 3);
        assert_eq!(map.len(), 1);
        assert_eq!(lookup_fx_color_image(&map, ",wisp"), Some(&1));
    }

    #[test]
    fn stubs_are_aliased_to_the_bare_name_unless_it_has_its_own_image() {
        let mut map = HashMap::from([
            (",smoke".to_owned(), 1),
            (",fire".to_owned(), 2),
            ("fire".to_owned(), 3),
        ]);
        assert_eq!(alias_fx_color_map_stubs(&mut map), 1);
        assert_eq!(map["smoke"], 1);
        assert_eq!(map["fire"], 3);
    }
}
