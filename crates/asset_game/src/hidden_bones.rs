//! Which bones of a weapon model its attachments hide.

use crate::weapon::WeaponLedger;

/// The bone names `gun_catalog_no`'s attachments hide, parent weapon included.
pub fn effective_hide_tags(registry: &WeaponLedger, gun_catalog_no: u32) -> Vec<String> {
    registry.hidden_tags_for(gun_catalog_no).to_vec()
}

/// Whether `bone` or any bone above it is named in `concealed_markers`, ignoring
/// ASCII case. `ancestor_for` gives a bone's parent; a bone with no name in
/// `joint_labels` is never hidden by its own name.
pub fn joint_under_concealed_parent(
    joint_labels: &[String],
    ancestor_for: impl Fn(usize) -> Option<usize>,
    joint_ix: usize,
    concealed_markers: &[String],
) -> bool {
    let hides = |index: usize| {
        joint_labels
            .get(index)
            .is_some_and(|name| concealed_markers.iter().any(|tag| tag.eq_ignore_ascii_case(name)))
    };
    std::iter::successors(Some(joint_ix), |&index| ancestor_for(index)).any(hides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["root", "clip", "mag_latch"].map(String::from).to_vec()
    }

    fn owner(index: usize) -> Option<usize> {
        index.checked_sub(1)
    }

    #[test]
    fn a_bone_under_a_hidden_one_is_hidden() {
        let hidden = vec!["CLIP".to_owned()];
        assert!(joint_under_concealed_parent(&names(), owner, 2, &hidden));
        assert!(joint_under_concealed_parent(&names(), owner, 1, &hidden));
        assert!(!joint_under_concealed_parent(&names(), owner, 0, &hidden));
    }

    #[test]
    fn nothing_is_hidden_without_tags_or_names() {
        assert!(!joint_under_concealed_parent(&names(), owner, 2, &[]));
        assert!(!joint_under_concealed_parent(&[], owner, 2, &["clip".to_owned()]));
    }
}
