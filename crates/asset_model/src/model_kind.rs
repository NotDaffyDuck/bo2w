//! What a model is for, told from its name.

/// The part a model plays in the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    /// Drawn in the player's own view: a gun or a pair of arms.
    Fpv,
    /// The body or the head of a player's character.
    Soldier,
    /// A gun seen held by someone else or lying in the world.
    WorldWeapon,
}

/// Names that mark a model as one kind: it starts with one of `prefixes` or
/// has one of `fragments` anywhere in it.
struct NameRule {
    kind: ModelKind,
    prefixes: &'static [&'static str],
    fragments: &'static [&'static str],
}

/// Tried in order; the first rule a name satisfies decides.
const NAME_RULES: [NameRule; 3] = [
    NameRule {
        kind: ModelKind::Fpv,
        prefixes: &["viewmodel_", "viewhands_"],
        fragments: &["_viewmodel"],
    },
    NameRule {
        kind: ModelKind::WorldWeapon,
        prefixes: &["weapon_"],
        fragments: &[],
    },
    NameRule {
        kind: ModelKind::Soldier,
        prefixes: &["mp_body_", "head_"],
        fragments: &["_mp_body_", "_mp_head_"],
    },
];

/// The kind of model `name` is, if it is one of them.
pub fn model_kind(name: &str) -> Option<ModelKind> {
    NAME_RULES
        .iter()
        .find(|rule| {
            rule.prefixes.iter().any(|prefix| name.starts_with(prefix))
                || rule.fragments.iter().any(|fragment| name.contains(fragment))
        })
        .map(|rule| rule.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_tell_the_kind() {
        assert_eq!(model_kind("viewmodel_ak47"), Some(ModelKind::Fpv));
        assert_eq!(model_kind("c_usa_viewhands"), None);
        assert_eq!(model_kind("viewhands_usa"), Some(ModelKind::Fpv));
        assert_eq!(model_kind("t6_wpn_viewmodel_x"), Some(ModelKind::Fpv));
        assert_eq!(model_kind("weapon_ak47_world"), Some(ModelKind::WorldWeapon));
        assert_eq!(model_kind("mp_body_seal_a"), Some(ModelKind::Soldier));
        assert_eq!(model_kind("head_seal_a"), Some(ModelKind::Soldier));
        assert_eq!(model_kind("crate_big"), None);
    }

    #[test]
    fn a_viewmodel_name_wins_over_a_soldier_one() {
        assert_eq!(model_kind("viewmodel_mp_body_x"), Some(ModelKind::Fpv));
    }
}
