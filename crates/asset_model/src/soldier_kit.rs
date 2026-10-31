//! Choosing a soldier's body, head and first-person arms from the model names
//! a zone carries, so each team gets a set that goes together.
//!
//! Model names are underscore-separated words. A team shows in a name through
//! a few words, and models of one soldier family share the rest.

use std::cmp::Reverse;

/// The models one team's soldier is drawn from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrooperOutfit {
    pub torso: String,
    pub skull: Option<String>,
    pub limbs: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoldierKitTable {
    pub allies: Option<TrooperOutfit>,
    pub axis: Option<TrooperOutfit>,
}

impl SoldierKitTable {
    /// A team's kit; a team without one wears the other team's.
    pub fn kit<'k>(&'k self, for_axis: bool) -> Option<&'k TrooperOutfit> {
        let preference = match for_axis {
            true => [&self.axis, &self.allies],
            false => [&self.allies, &self.axis],
        };
        preference.into_iter().find_map(Option::as_ref)
    }
}

/// Which kit a player wears: the client's team number decides (1 allies,
/// 2 axis); any other number falls back to the free-for-all split.
pub fn kit_belongs_to_axis(team_number: i32, ffa_slot: Option<u8>) -> bool {
    let named_team = (1..=2).contains(&team_number);
    if named_team {
        return team_number == 2;
    }
    ffa_assignment_is_axis(ffa_slot)
}

/// Free-for-all players are split by the team number the match gave them.
pub fn ffa_assignment_is_axis(ffa_slot: Option<u8>) -> bool {
    ffa_slot == Some(1)
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

/// A name that begins with `lead`, or carries `inner` after some word.
fn begins_or_carries(name: &str, lead: &str, inner: &str) -> bool {
    [name.starts_with(lead), name.contains(inner)].contains(&true)
}

pub fn names_a_body(model_name: &str) -> bool {
    begins_or_carries(model_name, "mp_body_", "_mp_body_")
}

fn is_head_model(model: &str) -> bool {
    begins_or_carries(model, "head_", "_mp_head_")
}

pub fn is_arms_model(model_name: &str) -> bool {
    let ends = (
        model_name.starts_with("viewmodel_"),
        model_name.ends_with("_arms"),
    );
    ends == (true, true)
}

/// A third-person body can carry a head and a weapon only if it has the
/// bones to hang them on.
pub fn bones_take_third_person_mounts(bones: &[String]) -> bool {
    let (head, weapon) = bones.iter().fold((false, false), |(head, weapon), bone| {
        (
            head || bone.as_str() == xmodel_runtime::THIRD_PERSON_HEAD_BONE,
            weapon || xmodel_runtime::THIRD_PERSON_GUN_BONES.contains(&bone.as_str()),
        )
    });
    head && weapon
}

fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(['_'])
}

/// The words of a name that are on `list` (`listed`) or are not.
fn words_by_list<'a>(
    name: &'a str,
    list: &'a [&str],
    listed: bool,
) -> impl Iterator<Item = &'a str> {
    words(name).filter(move |word| list.contains(word) == listed)
}

fn family_words(name: &str) -> impl Iterator<Item = &str> {
    words_by_list(name, GENERIC_WORDS, false)
}

fn class_words(name: &str) -> impl Iterator<Item = &str> {
    words_by_list(name, CLASS_WORDS, true)
}

/// The summed weight of every word of `name` that `weights` knows.
fn weigh(name: &str, weights: &[(&str, i32)]) -> i32 {
    let weight_of = |word: &str| {
        weights
            .iter()
            .find_map(|&(known, weight)| (known == word).then_some(weight))
            .unwrap_or(0)
    };
    words(name).map(weight_of).sum()
}

/// The team a name belongs to; an axis word outranks an allies word.
fn side_of(name: &str) -> Option<Side> {
    let ranked = [(AXIS_WORDS, Side::Axis), (ALLIES_WORDS, Side::Allies)];
    ranked
        .into_iter()
        .find(|&(list, _)| words(name).any(|word| list.contains(&word)))
        .map(|(_, side)| side)
}

/// Whether a model may go to the `wanted` team; a model of no team goes to either.
fn fits_side(wanted: Option<Side>, model: &str) -> bool {
    match (wanted, side_of(model)) {
        (Some(want), Some(found)) => want == found,
        _ => true,
    }
}

/// How many of `words` are among `among`.
fn shared<'a>(words: impl Iterator<Item = &'a str>, among: &[&str]) -> usize {
    words.filter(|word| among.contains(word)).count()
}

/// How many of `bodies` share a family word with `model`.
fn relatives_in(model: &str, bodies: &[&str]) -> usize {
    let own: Vec<&str> = family_words(model).collect();
    let mut count = 0;
    for &other in bodies {
        if shared(family_words(other), &own) > 0 {
            count += 1;
        }
    }
    count
}

/// The highest-scoring name with a positive score; ties go to the name that
/// sorts first.
fn best_positive<'a>(
    names: impl Iterator<Item = &'a str>,
    score: impl Fn(&str) -> i32,
) -> Option<&'a str> {
    let (top, Reverse(name)) = names.map(|name| (score(name), Reverse(name))).max()?;
    (top > 0).then_some(name)
}

/// The best-scoring choice that may go to the `wanted` team, as an owned name.
fn best_for_side(choices: &[&str], wanted: Option<Side>, score: impl Fn(&str) -> i32) -> Option<String> {
    let allowed = choices.iter().copied().filter(|model| fits_side(wanted, model));
    best_positive(allowed, score).map(String::from)
}

/// A body's standing in its pool: everyday-ness, then family size, then the
/// name that sorts first.
fn body_rank<'n>(name: &'n str, pool: &[&str]) -> (i32, usize, Reverse<&'n str>) {
    (
        weigh(name, BODY_WEIGHTS),
        relatives_in(name, pool),
        Reverse(name),
    )
}

fn pick_body<'a>(bodies: &[&'a str], side: Option<Side>) -> Option<&'a str> {
    let on_side = |name: &&str| side.is_none() || side_of(name) == side;
    let pool: Vec<&str> = bodies
        .iter()
        .filter(|name| on_side(name))
        .copied()
        .collect();
    pool.iter()
        .copied()
        .max_by_key(|&name| body_rank(name, &pool))
}

fn head_for(body: &str, choices: &[&str], team: Option<Side>) -> Option<String> {
    let family: Vec<&str> = family_words(body).collect();
    best_for_side(choices, team, |candidate| {
        weigh(candidate, HEAD_WEIGHTS) + shared(family_words(candidate), &family) as i32
    })
}

fn arms_for(body: &str, choices: &[&str], team: Option<Side>) -> Option<String> {
    let family: Vec<&str> = family_words(body).collect();
    let classes: Vec<&str> = class_words(body).collect();
    // Sharing the body's loadout class counts four times a shared family word.
    const CLASS_FACTOR: usize = 4;
    best_for_side(choices, team, |candidate| {
        let own_family =
            family_words(candidate).filter(|word| !matches!(*word, "viewmodel" | "arms"));
        let score =
            shared(own_family, &family) + CLASS_FACTOR * shared(class_words(candidate), &classes);
        score as i32
    })
}

/// The model names a zone carries, sorted into the three kinds a kit needs.
struct Pools<'a> {
    bodies: Vec<&'a str>,
    heads: Vec<&'a str>,
    arms: Vec<&'a str>,
}

impl<'a> Pools<'a> {
    fn sorted(names: &'a [String]) -> Self {
        let of_kind = |is_kind: fn(&str) -> bool| -> Vec<&'a str> {
            names
                .iter()
                .map(String::as_str)
                .filter(|name| is_kind(name))
                .collect()
        };
        Pools {
            bodies: of_kind(names_a_body),
            heads: of_kind(is_head_model),
            arms: of_kind(is_arms_model),
        }
    }

    /// The kit for a team (or for no team in particular); it needs a body.
    fn kit_for(&self, team: Option<Side>) -> Option<TrooperOutfit> {
        pick_body(&self.bodies, team).map(|body| TrooperOutfit {
            skull: head_for(body, &self.heads, team),
            limbs: arms_for(body, &self.arms, team),
            torso: body.into(),
        })
    }
}

/// Each team's kit from the model names on hand. When no model names a team,
/// one kit built from every model goes to the allies.
pub fn kit_table_from(model_names: &[String]) -> SoldierKitTable {
    let pools = Pools::sorted(model_names);
    let [allies, axis] = [Side::Allies, Side::Axis].map(|side| pools.kit_for(Some(side)));
    let mut kits = SoldierKitTable { allies, axis };
    let nobody_named = [&kits.allies, &kits.axis].iter().all(|slot| slot.is_none());
    if nobody_named {
        kits.allies = pools.kit_for(None);
    }
    kits
}

/// The arms that go with a body: the best of `offered` on the body's own team.
pub fn arms_matching_body(body: &str, offered: &[&str]) -> Option<String> {
    arms_for(body, offered, side_of(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kits_of(list: &[&str]) -> SoldierKitTable {
        let owned: Vec<String> = list.iter().map(|name| name.to_string()).collect();
        kit_table_from(&owned)
    }

    fn picks(kit: &TrooperOutfit) -> (&str, Option<&str>, Option<&str>) {
        (kit.torso.as_str(), kit.skull.as_deref(), kit.limbs.as_deref())
    }

    #[test]
    fn each_team_gets_its_everyday_soldier() {
        let kits = kits_of(&[
            "mp_body_seal_sniper_a",
            "head_op_a",
            "viewmodel_seal_arms",
            "mp_body_op_assault_a",
            "viewmodel_op_arms",
            "head_seal_a",
            "mp_body_seal_assault_a",
        ]);
        let allies = kits.allies.as_ref().expect("allies kit");
        let axis = kits.axis.as_ref().expect("axis kit");
        assert_eq!(
            picks(allies),
            (
                "mp_body_seal_assault_a",
                Some("head_seal_a"),
                Some("viewmodel_seal_arms")
            )
        );
        assert_eq!(
            picks(axis),
            ("mp_body_op_assault_a", Some("head_op_a"), None)
        );
    }

    #[test]
    fn names_of_no_team_make_one_kit_for_the_allies() {
        let kits = kits_of(&["mp_body_plain_assault"]);
        let worn = kits.allies.as_ref().map(|kit| kit.torso.as_str());
        assert_eq!((worn, kits.axis), (Some("mp_body_plain_assault"), None));
    }

    #[test]
    fn a_team_without_a_kit_wears_the_others() {
        let kits = kits_of(&["mp_body_op_assault_a"]);
        assert_eq!(kits.allies, None);
        for for_axis in [false, true] {
            let worn = kits.kit(for_axis).map(|kit| kit.torso.as_str());
            assert_eq!(worn, Some("mp_body_op_assault_a"));
        }
    }

    #[test]
    fn arms_follow_the_bodys_team_and_class() {
        let offered = [
            "viewmodel_op_arms",
            "viewmodel_seal_standard_arms",
            "viewmodel_seal_arms",
        ];
        assert_eq!(
            arms_matching_body("mp_body_seal_standard_a", &offered).as_deref(),
            Some("viewmodel_seal_standard_arms")
        );
    }

    #[test]
    fn the_client_team_number_decides_before_the_free_for_all_split() {
        let cases = [
            (0, None, false),
            (0, Some(1), true),
            (1, Some(1), false),
            (2, None, true),
            (7, Some(0), false),
        ];
        for (team, slot, axis) in cases {
            assert_eq!(
                kit_belongs_to_axis(team, slot),
                axis,
                "team {team} slot {slot:?}"
            );
        }
    }

    #[test]
    fn model_kinds_are_told_by_their_names() {
        assert!(names_a_body("mp_body_op_a") && names_a_body("x_mp_body_op_a"));
        assert!(!names_a_body("head_op_a"));
        assert!(is_head_model("head_op_a") && is_head_model("x_mp_head_op_a"));
        assert!(is_arms_model("viewmodel_arms") && is_arms_model("viewmodel_op_arms"));
        assert!(!is_arms_model("viewmodel_op"));
    }
}
