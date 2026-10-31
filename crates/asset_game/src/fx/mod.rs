//! Effects: what each one is made of, found by name.

mod definitions;
mod name;
mod t6;
mod elem_look;

use std::collections::HashMap;

pub use definitions::{EffectRoster, FxDefinitions, SelfContainedEffectDef};
pub use name::FxName;
pub use t6::{T6_FIRST_PERSON_FLASH_SCALE, t6_elem_type, t6_first_person_flash_name};
pub use elem_look::{FxBankSound, MaterialSlot, SelfContainedEffectElement, SelfContainedTrailDef, SelfContainedEffectLook};

use crate::AssetPointer;

/// The kinds of effect element, by the number the zones store.
pub mod look_kind {
    pub const ALWAYS_FACING: u8 = 0;
    pub const ROTATED_BOX: u8 = 1;
    pub const STREAK_END: u8 = 2;
    pub const STREAK_BODY: u8 = 3;
    pub const PUFF: u8 = 4;
    pub const MESH_KIND: u8 = 7;
    pub const POINT_EMITTER: u8 = 8;
    pub const CONE_EMITTER: u8 = 9;
    pub const AUDIO_KIND: u8 = 10;
    pub const SURFACE_STICKER: u8 = 11;
    pub const SPRINTER: u8 = 12;
}

/// A material name as the colour-map table keys it: without a reference mark.
#[inline]
pub fn material_bind_name(name: &str) -> &str {
    AssetPointer::bare_name(name)
}

/// Files an effect material's colour image under its bind name; the first
/// image for a name stays.
pub fn add_tint_picture<V>(by_bind: &mut HashMap<String, V>, designer_label: &str, image: V) {
    let bind = material_bind_name(designer_label);
    if !bind.is_empty() && !by_bind.contains_key(bind) {
        by_bind.insert(String::from(bind), image);
    }
}

/// Gives every image filed under a marked name a copy under the bare name too,
/// unless the bare name has an image of its own. Returns how many were added.
pub fn effect_tint_map_placeholders<T: Clone>(map: &mut HashMap<String, T>) -> usize {
    let aliases: Vec<(String, T)> = map
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

pub fn find_tint_picture<'m, V>(map: &'m HashMap<String, V>, designer_label: &str) -> Option<&'m V> {
    map.get(material_bind_name(designer_label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_name_is_filed_under_its_bare_name() {
        let mut filed = HashMap::new();
        let map = &mut filed;
        add_tint_picture(map, ",wisp", 1);
        add_tint_picture(map, "wisp", 2);
        add_tint_picture(map, "", 3);
        assert_eq!(filed.len(), 1);
        assert_eq!(find_tint_picture(&filed, ",wisp"), Some(&1));
    }

    #[test]
    fn stubs_are_aliased_to_the_bare_name_unless_it_has_its_own_image() {
        let mut map = HashMap::from([
            (",smoke".to_owned(), 1),
            (",fire".to_owned(), 2),
            ("fire".to_owned(), 3),
        ]);
        assert_eq!(effect_tint_map_placeholders(&mut map), 1);
        assert_eq!(map["smoke"], 1);
        assert_eq!(map["fire"], 3);
    }
}
