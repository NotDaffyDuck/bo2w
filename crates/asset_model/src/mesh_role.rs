//! Telling what a model is for from the name it was saved under.

/// The job a model does in a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshRole {
    /// Seen in the player's own view: a gun or a pair of arms.
    ViewModelPerspective,
    /// A character's body or head.
    Soldier,
    /// A gun in someone else's hands, or lying on the ground.
    ThirdPersonWeapon,
}

/// Does `name` begin with one of `starts` or hold one of `inside`?
fn named_like(name: &str, starts: &[&str], inside: &[&str]) -> bool {
    starts.iter().any(|head| name.starts_with(head)) || inside.iter().any(|piece| name.contains(piece))
}

/// The role `name` marks a model as having, if any. A view-model name wins
/// over a world-gun name, and that over a soldier name.
pub fn role_from_name(name: &str) -> Option<MeshRole> {
    if named_like(name, &["viewmodel_", "viewhands_"], &["_viewmodel"]) {
        Some(MeshRole::ViewModelPerspective)
    } else if name.starts_with("weapon_") {
        Some(MeshRole::ThirdPersonWeapon)
    } else if named_like(name, &["mp_body_", "head_"], &["_mp_body_", "_mp_head_"]) {
        Some(MeshRole::Soldier)
    } else {
        None
    }
}

/// Older spellings other crates still call; the rename sweep moves them over.
pub fn mesh_role_for(name: &str) -> Option<MeshRole> {
    role_from_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_name_gets_its_role() {
        let table = [
            ("viewmodel_ak47", Some(MeshRole::ViewModelPerspective)),
            ("c_usa_viewhands", None),
            ("viewhands_usa", Some(MeshRole::ViewModelPerspective)),
            ("t6_wpn_viewmodel_x", Some(MeshRole::ViewModelPerspective)),
            ("weapon_ak47_world", Some(MeshRole::ThirdPersonWeapon)),
            ("mp_body_seal_a", Some(MeshRole::Soldier)),
            ("head_seal_a", Some(MeshRole::Soldier)),
            ("crate_big", None),
        ];
        for (name, role) in table {
            assert_eq!(role_from_name(name), role, "{name}");
        }
    }

    #[test]
    fn a_view_model_name_beats_a_soldier_one() {
        assert_eq!(role_from_name("viewmodel_mp_body_x"), Some(MeshRole::ViewModelPerspective));
    }
}
