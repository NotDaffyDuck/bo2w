//! Choosing a soldier's body, head and first-person arms from the model names
//! a zone carries, so each team gets a set that goes together.
//!
//! Model names are underscore-separated words. A team shows in a name through
//! a few words, and models of one soldier family share the rest.

use std::cmp::Reverse;

/// The models one team's soldier is drawn from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoldierKit {
    pub body: String,
    pub head: Option<String>,
    pub arms: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoldierKits {
    pub allies: Option<SoldierKit>,
    pub axis: Option<SoldierKit>,
}

impl SoldierKits {
    /// A team's kit; a team without one wears the other team's.
    pub fn kit(&self, axis: bool) -> Option<&SoldierKit> {
        let (own, other) = match axis {
            true => (&self.axis, &self.allies),
            false => (&self.allies, &self.axis),
        };
        own.as_ref().or(other.as_ref())
    }
}

/// Free-for-all players are split by the team number the match gave them.
pub fn ffa_assignment_is_axis(ffa_team: Option<u8>) -> bool {
    ffa_team == Some(1)
}

/// Which kit a player wears: the client's team number decides (1 allies,
/// 2 axis); any other number falls back to the free-for-all split.
pub fn kit_assignment_is_axis(client_state_team: i32, ffa_team: Option<u8>) -> bool {
    match client_state_team {
        1 => false,
        2 => true,
        _ => ffa_assignment_is_axis(ffa_team),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Allies,
    Axis,
}

const AXIS_WORDS: &[&str] = &[
    "op", "opforce", "militia", "miltia", "airborne", "vtn", "nva", "rus", "spet", "spetwin",
];
const ALLIES_WORDS: &[&str] = &[
    "ally", "allies", "tf141", "udt", "seal", "army", "usa", "sog", "cia", "ciawin",
];
/// Words naming a loadout class rather than a soldier family.
const CLASS_WORDS: &[&str] = &["standard", "armor", "camo", "flak", "utility"];
/// Words that say what a model is or how it is kitted, never whose it is.
const GENERIC_WORDS: &[&str] = &[
    "mp", "body", "head", "assault", "smg", "lmg", "shotgun", "sniper", "riot", "ghillie", "op",
    "ally", "allies", "c", "standard", "armor", "camo", "flak", "utility", "1", "2", "3", "4", "5",
];
/// How much a word in a body's name makes it the team's everyday soldier.
const BODY_WEIGHTS: &[(&str, i32)] = &[
    ("assault", 3),
    ("standard", 3),
    ("a", 1),
    ("armor", 1),
    ("smg", -1),
    ("lmg", -1),
    ("shotgun", -1),
    ("camo", -1),
    ("flak", -1),
    ("utility", -1),
    ("sniper", -6),
    ("ghillie", -8),
    ("riot", -10),
];
const HEAD_WEIGHTS: &[(&str, i32)] = &[("hat", -1), ("sniper", -6), ("ghillie", -8), ("riot", -10)];

pub fn is_body_model(name: &str) -> bool {
    name.starts_with("mp_body_") || name.contains("_mp_body_")
}

fn is_head_model(name: &str) -> bool {
    name.starts_with("head_") || name.contains("_mp_head_")
}

pub fn is_arms_model(name: &str) -> bool {
    name.starts_with("viewmodel_") && name.ends_with("_arms")
}

/// A third-person body can carry a head and a weapon only if it has the
/// bones to hang them on.
pub fn body_has_tp_attach_bones(bone_names: &[String]) -> bool {
    let has = |tag: &str| bone_names.iter().any(|bone| bone == tag);
    has(xmodel_runtime::TP_HEAD_ATTACH_TAG)
        && xmodel_runtime::TP_WEAPON_ATTACH_TAGS.iter().any(|tag| has(tag))
}

fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split('_')
}

fn weigh(name: &str, weights: &[(&str, i32)]) -> i32 {
    words(name)
        .filter_map(|word| weights.iter().find(|(known, _)| *known == word))
        .map(|(_, weight)| weight)
        .sum()
}

/// The team a name belongs to; an axis word outranks an allies word.
fn side_of(name: &str) -> Option<Side> {
    let any = |list: &[&str]| words(name).any(|word| list.contains(&word));
    if any(AXIS_WORDS) {
        Some(Side::Axis)
    } else if any(ALLIES_WORDS) {
        Some(Side::Allies)
    } else {
        None
    }
}

/// Whether a name may serve `side`; a name of no team serves either.
fn serves(side: Option<Side>, name: &str) -> bool {
    match (side, side_of(name)) {
        (Some(wanted), Some(actual)) => wanted == actual,
        _ => true,
    }
}

fn family_words(name: &str) -> impl Iterator<Item = &str> {
    words(name).filter(|word| !GENERIC_WORDS.contains(word))
}

fn class_words(name: &str) -> impl Iterator<Item = &str> {
    words(name).filter(|word| CLASS_WORDS.contains(word))
}

/// How many bodies in `pool` share a family word with `name`.
fn family_size(name: &str, pool: &[&str]) -> usize {
    let own: Vec<&str> = family_words(name).collect();
    pool.iter()
        .filter(|other| family_words(other).any(|word| own.contains(&word)))
        .count()
}

/// The highest-scoring name with a positive score; ties go to the name that
/// sorts first.
fn best_positive<'a>(
    names: impl Iterator<Item = &'a str>,
    score: impl Fn(&str) -> i32,
) -> Option<&'a str> {
    names
        .max_by_key(|name| (score(name), Reverse(*name)))
        .filter(|name| score(name) > 0)
}

fn pick_body<'a>(bodies: &[&'a str], side: Option<Side>) -> Option<&'a str> {
    let pool: Vec<&str> = bodies
        .iter()
        .copied()
        .filter(|name| side.is_none_or(|side| side_of(name) == Some(side)))
        .collect();
    pool.iter().copied().max_by_key(|name| {
        (
            weigh(name, BODY_WEIGHTS),
            family_size(name, &pool),
            Reverse(*name),
        )
    })
}

fn pick_head(body: &str, heads: &[&str], side: Option<Side>) -> Option<String> {
    let family: Vec<&str> = family_words(body).collect();
    let score = |name: &str| {
        let shared = family_words(name).filter(|word| family.contains(word)).count();
        shared as i32 + weigh(name, HEAD_WEIGHTS)
    };
    best_positive(heads.iter().copied().filter(|name| serves(side, name)), score)
        .map(str::to_owned)
}

fn pick_arms(body: &str, arms: &[&str], side: Option<Side>) -> Option<String> {
    let family: Vec<&str> = family_words(body).collect();
    let classes: Vec<&str> = class_words(body).collect();
    // Sharing the body's loadout class counts four times a shared family word.
    let score = |name: &str| {
        let shared = family_words(name)
            .filter(|word| !matches!(*word, "viewmodel" | "arms"))
            .filter(|word| family.contains(word))
            .count();
        let same_class = class_words(name).filter(|word| classes.contains(word)).count();
        (shared + 4 * same_class) as i32
    };
    best_positive(arms.iter().copied().filter(|name| serves(side, name)), score)
        .map(str::to_owned)
}

fn kit_for(
    bodies: &[&str],
    heads: &[&str],
    arms: &[&str],
    side: Option<Side>,
) -> Option<SoldierKit> {
    let body = pick_body(bodies, side)?;
    Some(SoldierKit {
        body: body.to_owned(),
        head: pick_head(body, heads, side),
        arms: pick_arms(body, arms, side),
    })
}

/// Each team's kit from the model names on hand. When no model names a team,
/// one kit built from every model goes to the allies.
pub fn soldier_kits(names: &[String]) -> SoldierKits {
    let select = |keep: fn(&str) -> bool| -> Vec<&str> {
        names.iter().map(String::as_str).filter(|name| keep(name)).collect()
    };
    let (bodies, heads, arms) = (select(is_body_model), select(is_head_model), select(is_arms_model));

    let mut kits = SoldierKits {
        allies: kit_for(&bodies, &heads, &arms, Some(Side::Allies)),
        axis: kit_for(&bodies, &heads, &arms, Some(Side::Axis)),
    };
    if kits.allies.is_none() && kits.axis.is_none() {
        kits.allies = kit_for(&bodies, &heads, &arms, None);
    }
    kits
}

/// The arms that go with a body: the best of `arms` on the body's own team.
pub fn arms_for_body(body: &str, arms: &[&str]) -> Option<String> {
    pick_arms(body, arms, side_of(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn each_team_gets_its_everyday_soldier() {
        let kits = soldier_kits(&names(&[
            "mp_body_seal_assault_a",
            "mp_body_seal_sniper_a",
            "mp_body_op_assault_a",
            "head_seal_a",
            "head_op_a",
            "viewmodel_seal_arms",
            "viewmodel_op_arms",
        ]));
        let allies = kits.allies.expect("allies kit");
        assert_eq!(allies.body, "mp_body_seal_assault_a");
        assert_eq!(allies.head.as_deref(), Some("head_seal_a"));
        assert_eq!(allies.arms.as_deref(), Some("viewmodel_seal_arms"));
        assert_eq!(kits.axis.expect("axis kit").body, "mp_body_op_assault_a");
    }

    #[test]
    fn names_of_no_team_make_one_kit_for_the_allies() {
        let kits = soldier_kits(&names(&["mp_body_plain_assault"]));
        assert_eq!(kits.allies.map(|kit| kit.body).as_deref(), Some("mp_body_plain_assault"));
        assert!(kits.axis.is_none());
    }

    #[test]
    fn a_team_without_a_kit_wears_the_others() {
        let kits = soldier_kits(&names(&["mp_body_op_assault_a"]));
        assert!(kits.allies.is_none());
        assert_eq!(kits.kit(false).map(|kit| kit.body.as_str()), Some("mp_body_op_assault_a"));
    }

    #[test]
    fn arms_follow_the_bodys_team_and_class() {
        let arms = ["viewmodel_op_arms", "viewmodel_seal_standard_arms", "viewmodel_seal_arms"];
        let picked = arms_for_body("mp_body_seal_standard_a", &arms);
        assert_eq!(picked.as_deref(), Some("viewmodel_seal_standard_arms"));
    }

    #[test]
    fn the_client_team_number_decides_before_the_free_for_all_split() {
        assert!(kit_assignment_is_axis(2, None));
        assert!(!kit_assignment_is_axis(1, Some(1)));
        assert!(kit_assignment_is_axis(0, Some(1)));
        assert!(!kit_assignment_is_axis(0, None));
    }
}
