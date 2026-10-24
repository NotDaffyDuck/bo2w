//! Which bones of a weapon model its attachments hide.

use crate::weapon::WeaponRegistry;

/// The bone names `weapon_id`'s attachments hide, parent weapon included.
pub fn effective_hide_tags(registry: &WeaponRegistry, weapon_id: u32) -> Vec<String> {
    registry.hide_tags_of(weapon_id).to_vec()
}

/// Whether `bone` or any bone above it is named in `hidden_tags`, ignoring
/// ASCII case. `parent_of` gives a bone's parent; a bone with no name in
/// `bone_names` is never hidden by its own name.
pub fn bone_has_hidden_ancestor(
    bone_names: &[String],
    parent_of: impl Fn(usize) -> Option<usize>,
    bone: usize,
    hidden_tags: &[String],
) -> bool {
    let hides = |index: usize| {
        bone_names
            .get(index)
            .is_some_and(|name| hidden_tags.iter().any(|tag| tag.eq_ignore_ascii_case(name)))
    };
    std::iter::successors(Some(bone), |&index| parent_of(index)).any(hides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["root", "clip", "mag_latch"].map(String::from).to_vec()
    }

    fn parent(index: usize) -> Option<usize> {
        index.checked_sub(1)
    }

    #[test]
    fn a_bone_under_a_hidden_one_is_hidden() {
        let hidden = vec!["CLIP".to_owned()];
        assert!(bone_has_hidden_ancestor(&names(), parent, 2, &hidden));
        assert!(bone_has_hidden_ancestor(&names(), parent, 1, &hidden));
        assert!(!bone_has_hidden_ancestor(&names(), parent, 0, &hidden));
    }

    #[test]
    fn nothing_is_hidden_without_tags_or_names() {
        assert!(!bone_has_hidden_ancestor(&names(), parent, 2, &[]));
        assert!(!bone_has_hidden_ancestor(&[], parent, 2, &["clip".to_owned()]));
    }
}
