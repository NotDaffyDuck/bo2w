//! Turning a family plus attachment picks into a weapon id, and a weapon id
//! back into the picks that make it.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AttachmentOption, ConfigurationRefusal, FamilyContent, LoadoutRules,
    ResolvedConfiguration, WeaponFamilies, WeaponFamily, WeaponSelection,
};
use crate::AssetCategoryScope;

/// Attachment names are lower case, each once, in alphabetical order, with
/// the empty and "none" picks dropped.
fn canonical(picks: &[String]) -> Vec<String> {
    picks
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .filter(|name| !matches!(name.as_str(), "" | "none"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The weapon name a family and its (canonical) attachments are authored as.
fn weapon_name(family: &WeaponFamily, attachments: &[String]) -> String {
    std::iter::once(family.key.base.as_str())
        .chain(attachments.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("_")
}

impl WeaponFamilies {
    /// Gives every weapon the selection it is the canonical form of. A name
    /// like `base_a_b` only counts when resolving that selection lands back on
    /// the same weapon.
    pub(super) fn index_weapons(&mut self, content: &impl FamilyContent) {
        let games: BTreeMap<&str, AssetCategoryScope> = self
            .families
            .iter()
            .map(|family| (family.key.game.as_str(), family.key.game))
            .collect();

        for game in games.into_values() {
            let claimed = self.named_selections(game, content);
            let canonical_ones: Vec<_> = claimed
                .into_iter()
                .filter(|(gun_no, selection)| {
                    self.settle(selection, LoadoutRules::default(), content)
                        .is_ok_and(|resolved| resolved.id == *gun_no)
                })
                .collect();
            self.canon_by_id.extend(canonical_ones);
        }
    }

    /// Each weapon of a game whose name spells a family plus attachments.
    fn named_selections(
        &self,
        game: AssetCategoryScope,
        content: &impl FamilyContent,
    ) -> Vec<(u32, WeaponSelection)> {
        // Longest base first, so `base_x` never matches the shorter `base`.
        let mut bases: Vec<&WeaponFamily> =
            self.families.iter().filter(|f| f.key.game == game).collect();
        bases.sort_by_key(|family| std::cmp::Reverse(family.key.base.len()));

        content
            .names_in(game)
            .into_iter()
            .filter_map(|(pick_no, name)| {
                let selection = bases.iter().find_map(|family| {
                    let picks = self.picks_in_name(family, &name)?;
                    Some(WeaponSelection::with(family.key.clone(), &picks))
                })?;
                Some((pick_no, selection))
            })
            .collect()
    }

    /// The attachments a weapon name spells out after the family's base.
    fn picks_in_name(&self, family: &WeaponFamily, name: &str) -> Option<Vec<String>> {
        let name = name.strip_suffix("_mp").unwrap_or(name);
        let tail = name.strip_prefix(family.key.base.as_str())?;
        let Some(tail) = tail.strip_prefix('_') else {
            return tail.is_empty().then(Vec::new);
        };
        let rules = self.rules.get(&family.key.game)?;
        let picks: Vec<String> = tail.split('_').map(str::to_owned).collect();
        let spelled_canonically = canonical(&picks) == picks;
        (spelled_canonically && picks.iter().all(|pick| rules.known.contains_key(pick))).then_some(picks)
    }

    /// The first two picks, in order, that the combo table bans together.
    fn first_clash<'a>(
        &self,
        namespace: AssetCategoryScope,
        picks: &'a [String],
    ) -> Option<(&'a String, &'a String)> {
        let rules = self.rules.get(&namespace)?;
        picks.iter().enumerate().find_map(|(at, a)| {
            let b = picks[at + 1..].iter().find(|b| !rules.allows_together(a, b))?;
            Some((a, b))
        })
    }

    fn family_of(&self, selection: &WeaponSelection) -> Result<&WeaponFamily, ConfigurationRefusal> {
        let Some(key) = selection.family.as_ref() else {
            return Err(ConfigurationRefusal::UnknownFamily(String::new()));
        };
        self.family(key)
            .ok_or_else(|| ConfigurationRefusal::UnknownFamily(key.asset_key()))
    }

    /// The first rule the (canonical) picks break on this family.
    fn vet(
        &self,
        family: &WeaponFamily,
        picks: &[String],
        limits: LoadoutRules,
    ) -> Result<(), ConfigurationRefusal> {
        let namespace = family.key.game;
        let known = self.rules.get(&namespace).map(|rules| &rules.known);
        picks.iter().try_for_each(|pick| {
            let in_game = known.is_some_and(|known| known.contains_key(pick));
            let on_weapon = family.attachments.iter().any(|choice| choice.name == *pick);
            match (in_game, on_weapon) {
                (false, _) => Err(ConfigurationRefusal::UnknownAttachment(pick.clone())),
                (true, false) => Err(ConfigurationRefusal::NotOffered(pick.clone())),
                (true, true) => Ok(()),
            }
        })?;
        if picks.len() > limits.max_attachments {
            return Err(ConfigurationRefusal::RuleRestricted(format!(
                "at most {} attachments per weapon",
                limits.max_attachments
            )));
        }
        match self.first_clash(namespace, picks) {
            Some((a, b)) => Err(ConfigurationRefusal::Incompatible { a: a.clone(), b: b.clone() }),
            None => Ok(()),
        }
    }

    /// The weapon a selection stands for, or the first reason it cannot be one.
    pub(crate) fn settle(
        &self,
        selection: &WeaponSelection,
        limits: LoadoutRules,
        content: &impl FamilyContent,
    ) -> Result<ResolvedConfiguration, ConfigurationRefusal> {
        let family = self.family_of(selection)?;
        let picks = canonical(&selection.attachments);
        self.vet(family, &picks, limits)?;

        let name = weapon_name(family, &picks);
        let id = content
            .lookup(family.key.game, &name)
            .ok_or_else(|| ConfigurationRefusal::AbsentResource(format!("{name}_mp")))?;
        content.admission(id)?;
        Ok(ResolvedConfiguration {
            id,
            selection: WeaponSelection::with(family.key.clone(), &picks),
        })
    }

    /// Every attachment the family offers, with what flipping it would give.
    pub(crate) fn attachment_options(
        &self,
        selection: &WeaponSelection,
        limits: LoadoutRules,
        content: &impl FamilyContent,
    ) -> Result<Vec<AttachmentOption>, ConfigurationRefusal> {
        let family = self.family_of(selection)?;
        let current = canonical(&selection.attachments);
        Ok(family
            .attachments
            .iter()
            .map(|choice| {
                let mut flipped: Vec<String> =
                    current.iter().filter(|name| **name != choice.name).cloned().collect();
                let selected = flipped.len() < current.len();
                if !selected {
                    flipped.push(choice.name.clone());
                }
                let toggle = self
                    .settle(&WeaponSelection::with(family.key.clone(), &flipped), limits, content)
                    .map(|resolved| resolved.id);
                AttachmentOption { choice: choice.clone(), selected, toggle }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::SnapshotStringTable;
    use super::super::{FamilyKey, FamilySlot};

    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    /// A table with only the listed (column, text) cells filled in, row by row.
    fn grid(sheet_name: &str, cols: usize, rows: &[&[(usize, &str)]]) -> SnapshotStringTable {
        let mut cells = vec![String::new(); cols * rows.len()];
        for (row, filled) in rows.iter().enumerate() {
            for &(col, text) in filled.iter() {
                cells[row * cols + col] = text.to_owned();
            }
        }
        SnapshotStringTable { name: sheet_name.to_owned(), col_count: cols, height: rows.len(), grid: cells }
    }

    /// The loaded weapons, by name. Id 99 is one the game refuses.
    struct Loaded(HashMap<&'static str, u32>);

    impl FamilyContent for Loaded {
        fn lookup(&self, namespace: AssetCategoryScope, gun_name: &str) -> Option<u32> {
            (namespace == T6).then(|| self.0.get(gun_name).copied()).flatten()
        }

        fn secondary_hand_category(&self, id: u32) -> i32 {
            match id {
                8 => 1,
                9 => 2,
                _ => 0,
            }
        }

        fn admission(&self, id: u32) -> Result<(), ConfigurationRefusal> {
            match id {
                99 => Err(ConfigurationRefusal::AbsentResource("refused_mp".into())),
                _ => Ok(()),
            }
        }

        fn names_in(&self, _namespace: AssetCategoryScope) -> Vec<(u32, String)> {
            let mut all: Vec<_> = self.0.iter().map(|(name, id)| (*id, (*name).to_owned())).collect();
            all.sort();
            all
        }
    }

    fn loaded() -> Loaded {
        Loaded(HashMap::from([
            ("an94", 1),
            ("an94_acog", 2),
            ("an94_suppressed", 3),
            ("an94_acog_suppressed", 4),
            ("an94_suppressed_acog", 5),
            ("an94_bogus", 6),
            ("kard", 7),
            ("frag", 8),
            ("flash", 9),
            ("widget", 10),
            ("refused", 99),
        ]))
    }

    fn families() -> WeaponFamilies {
        let attachments = grid(
            "mp/attachmentTable.csv",
            8,
            &[
                &[],
                &[(2, "optic"), (3, "cap_acog"), (4, "ACOG"), (6, "icon_acog"), (7, "desc_acog")],
                &[(2, "muzzle"), (4, "suppressed")],
                &[(2, "underbarrel"), (4, "grip")],
                &[(2, "none"), (4, "bookkeeping")],
                &[(2, "optic")],
            ],
        );
        let combos = grid(
            "mp/attachmentCombos.csv",
            4,
            &[
                &[],
                &[(0, "acog"), (1, "no"), (3, "No")],
                &[(0, "suppressed")],
                &[(0, "grip")],
            ],
        );
        let stats = grid(
            "mp/statsTable.csv",
            22,
            &[
                &[],
                &[(2, "weapon_assault"), (3, "WEAPON_AN94"), (4, "an94"), (6, "img_an94"),
                  (11, "suppressed"), (12, "ACOG"), (13, "grip"), (14, "acog"), (15, "unknown")],
                &[(2, "weapon_pistol"), (4, "kard")],
                &[(2, "weapon_pistol"), (4, "refused")],
                &[(2, "weapon_pistol"), (4, "absent")],
                &[(2, "weapon_pistol"), (4, "weapon_null")],
                &[(0, "7"), (2, "perk_x"), (4, "specialty_fastreload"), (13, "specialty1")],
                &[(2, "weapon_assault"), (4, "ACOG")],
                &[(2, "weapon_assault"), (3, "DUPLICATE"), (4, "an94")],
                &[(2, "weapon_grenade"), (4, "frag")],
                &[(2, "weapon_grenade"), (4, "flash")],
                &[(2, "weapon_misc"), (4, "widget")],
            ],
        );
        WeaponFamilies::build(&[(T6, attachments), (T6, combos), (T6, stats)], &loaded())
    }

    fn an94(picks: &[&str]) -> WeaponSelection {
        let picks: Vec<String> = picks.iter().map(|pick| (*pick).to_owned()).collect();
        WeaponSelection::with(FamilyKey::new(T6, "an94"), &picks)
    }

    #[test]
    fn picks_are_lower_cased_sorted_and_stripped_of_blanks() {
        let picks = ["Suppressed", "none", "", "ACOG", "acog"].map(String::from);
        let built = families();
        let family = &built.families()[0];
        assert_eq!(canonical(&picks), ["acog", "suppressed"]);
        assert_eq!(weapon_name(family, &canonical(&picks)), "an94_acog_suppressed");
        assert_eq!(weapon_name(family, &[]), "an94");
    }

    #[test]
    fn the_tables_give_families_slots_and_sorted_attachments() {
        let families = families();
        let found: Vec<_> = families.families().iter().map(|f| (f.key.base.as_str(), f.bay)).collect();
        assert_eq!(
            found,
            [
                ("an94", FamilySlot::Primary),
                ("kard", FamilySlot::Secondary),
                ("refused", FamilySlot::Secondary),
                ("absent", FamilySlot::Secondary),
                ("frag", FamilySlot::Lethal),
                ("flash", FamilySlot::Tactical),
                ("widget", FamilySlot::Other),
            ]
        );
        let first = &families.families()[0];
        let offered: Vec<_> = first.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(offered, ["acog", "grip", "suppressed"]);
        assert_eq!((first.display_key.as_str(), first.image.as_str(), first.base), ("WEAPON_AN94", "img_an94", Some(1)));
        assert_eq!(first.attachments[0].icon, "icon_acog");
        let left_out: Vec<_> = families.excluded().iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(left_out, ["t6:weapon/refused_mp", "t6:weapon/absent_mp", "t6:weapon/widget_mp"]);
        assert_eq!(families.offered().count(), 4);
        assert_eq!(families.perk(7).map(|perk| perk.reference.as_str()), Some("specialty_fastreload"));
    }

    #[test]
    fn a_selection_becomes_the_weapon_its_canonical_name_spells() {
        let families = families();
        let content = loaded();
        let resolved = families.settle(&an94(&["Suppressed", "acog", "none"]), LoadoutRules::default(), &content);
        assert_eq!(resolved.map(|r| (r.id, r.selection.attachments)), Ok((4, vec!["acog".to_owned(), "suppressed".to_owned()])));
        let bare = families.settle(&an94(&[]), LoadoutRules::default(), &content);
        assert_eq!(bare.map(|r| r.id), Ok(1));
    }

    #[test]
    fn a_selection_that_cannot_be_a_weapon_says_why() {
        let families = families();
        let content = loaded();
        let rules = LoadoutRules::default();
        let refusal = |selection: &WeaponSelection, limits| families.settle(selection, limits, &content).unwrap_err();
        assert_eq!(refusal(&WeaponSelection::default(), rules), ConfigurationRefusal::UnknownFamily(String::new()));
        let nowhere = WeaponSelection::bare(FamilyKey::new(T6, "nowhere"));
        assert_eq!(refusal(&nowhere, rules), ConfigurationRefusal::UnknownFamily("t6:weapon/nowhere_mp".into()));
        assert_eq!(refusal(&an94(&["nope"]), rules), ConfigurationRefusal::UnknownAttachment("nope".into()));
        let kard = WeaponSelection::with(FamilyKey::new(T6, "kard"), &["acog".to_owned()]);
        assert_eq!(refusal(&kard, rules), ConfigurationRefusal::NotOffered("acog".into()));
        assert_eq!(
            refusal(&an94(&["grip", "acog"]), rules),
            ConfigurationRefusal::Incompatible { a: "acog".into(), b: "grip".into() }
        );
        assert_eq!(
            refusal(&an94(&["acog", "suppressed"]), LoadoutRules { max_attachments: 1 }),
            ConfigurationRefusal::RuleRestricted("at most 1 attachments per weapon".into())
        );
        assert_eq!(
            refusal(&an94(&["grip", "suppressed"]), rules),
            ConfigurationRefusal::AbsentResource("an94_grip_suppressed_mp".into())
        );
        let refused = WeaponSelection::bare(FamilyKey::new(T6, "refused"));
        assert_eq!(refusal(&refused, rules), ConfigurationRefusal::AbsentResource("refused_mp".into()));
    }

    #[test]
    fn each_option_shows_what_flipping_it_would_give() {
        let families = families();
        let content = loaded();
        let rules = LoadoutRules::default();
        let options = families.attachment_options(&an94(&["acog"]), rules, &content).unwrap();
        let shown: Vec<_> = options.iter().map(|o| (o.choice.name.as_str(), o.selected, o.toggle.clone())).collect();
        assert_eq!(
            shown,
            [
                ("acog", true, Ok(1)),
                ("grip", false, Err(ConfigurationRefusal::Incompatible { a: "acog".into(), b: "grip".into() })),
                ("suppressed", false, Ok(4)),
            ]
        );
        assert!(families.attachment_options(&WeaponSelection::default(), rules, &content).is_err());
    }

    #[test]
    fn only_canonical_names_that_resolve_back_are_described() {
        let families = families();
        assert_eq!(families.describe(1), Some(&an94(&[])));
        assert_eq!(families.describe(2), Some(&an94(&["acog"])));
        assert_eq!(families.describe(4), Some(&an94(&["acog", "suppressed"])));
        // Spelled out of order, an unknown attachment, and a weapon the game refuses.
        assert_eq!([5, 6, 99].map(|id| families.describe(id)), [None, None, None]);
        assert_eq!(families.describe(7), Some(&WeaponSelection::bare(FamilyKey::new(T6, "kard"))));
    }
}
