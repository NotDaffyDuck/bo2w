//! Turning a family plus attachment picks into a weapon id, and a weapon id
//! back into the picks that make it.

use super::{
    AttachmentOption, ConfigurationRefusal, FamilyContent, LoadoutRules,
    ResolvedConfiguration, WeaponFamilies, WeaponFamily, WeaponSelection,
};
use crate::AssetNamespace;

/// Attachment names are lower case, each once, in alphabetical order, with
/// the empty and "none" picks dropped.
fn canonical(picks: &[String]) -> Vec<String> {
    let mut names: Vec<String> = picks
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .filter(|name| !name.is_empty() && name != "none")
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The weapon name a family and its (canonical) attachments are authored as.
fn weapon_name(family: &WeaponFamily, attachments: &[String]) -> String {
    if attachments.is_empty() {
        family.key.base.clone()
    } else {
        format!("{}_{}", family.key.base, attachments.join("_"))
    }
}

impl WeaponFamilies {
    /// Gives every weapon the selection it is the canonical form of. A name
    /// like `base_a_b` only counts when resolving that selection lands back on
    /// the same weapon.
    pub(super) fn index_weapons(&mut self, content: &impl FamilyContent) {
        let mut games: Vec<AssetNamespace> = self.families.iter().map(|f| f.key.namespace).collect();
        games.sort_by_key(|game| game.as_str());
        games.dedup();

        for game in games {
            // Longest base first, so `base_x` never matches the shorter `base`.
            let mut bases: Vec<&WeaponFamily> =
                self.families.iter().filter(|f| f.key.namespace == game).collect();
            bases.sort_by_key(|family| std::cmp::Reverse(family.key.base.len()));

            let mut found = Vec::new();
            for (id, name) in content.names_in(game) {
                let hit = bases.iter().find_map(|family| {
                    let picks = self.picks_in_name(family, &name)?;
                    Some(WeaponSelection::with(family.key.clone(), &picks))
                });
                if let Some(selection) = hit {
                    found.push((id, selection));
                }
            }
            for (id, selection) in found {
                let lands_here = self
                    .resolve(&selection, LoadoutRules::default(), content)
                    .is_ok_and(|resolved| resolved.id == id);
                if lands_here {
                    self.by_id.insert(id, selection);
                }
            }
        }
    }

    /// The attachments a weapon name spells out after the family's base.
    fn picks_in_name(&self, family: &WeaponFamily, name: &str) -> Option<Vec<String>> {
        let name = name.strip_suffix("_mp").unwrap_or(name);
        if name == family.key.base {
            return Some(Vec::new());
        }
        let rules = self.rules.get(&family.key.namespace)?;
        let tail = name.strip_prefix(&family.key.base)?.strip_prefix('_')?;
        let picks: Vec<String> = tail.split('_').map(str::to_owned).collect();
        let all_known = picks.iter().all(|pick| rules.known.contains_key(pick));
        (all_known && canonical(&picks) == picks).then_some(picks)
    }

    fn allows_together(&self, namespace: AssetNamespace, a: &str, b: &str) -> bool {
        self.rules
            .get(&namespace)
            .is_none_or(|rules| rules.allows_together(a, b))
    }

    fn family_of(&self, selection: &WeaponSelection) -> Result<&WeaponFamily, ConfigurationRefusal> {
        let key = selection
            .family
            .as_ref()
            .ok_or_else(|| ConfigurationRefusal::UnknownFamily(String::new()))?;
        self.family(key)
            .ok_or_else(|| ConfigurationRefusal::UnknownFamily(key.asset_key()))
    }

    /// The weapon a selection stands for, or the first reason it cannot be one.
    pub(crate) fn resolve(
        &self,
        selection: &WeaponSelection,
        limits: LoadoutRules,
        content: &impl FamilyContent,
    ) -> Result<ResolvedConfiguration, ConfigurationRefusal> {
        let family = self.family_of(selection)?;
        let key = &family.key;
        let namespace = key.namespace;
        let picks = canonical(&selection.attachments);

        let known = self.rules.get(&namespace).map(|rules| &rules.known);
        for pick in &picks {
            if !known.is_some_and(|known| known.contains_key(pick)) {
                return Err(ConfigurationRefusal::UnknownAttachment(pick.clone()));
            }
            if !family.attachments.iter().any(|choice| choice.name == *pick) {
                return Err(ConfigurationRefusal::NotOffered(pick.clone()));
            }
        }
        if picks.len() > limits.max_attachments {
            return Err(ConfigurationRefusal::RuleRestricted(format!(
                "at most {} attachments per weapon",
                limits.max_attachments
            )));
        }
        for (at, a) in picks.iter().enumerate() {
            if let Some(b) = picks[at + 1..].iter().find(|b| !self.allows_together(namespace, a, b)) {
                return Err(ConfigurationRefusal::Incompatible {
                    a: a.clone(),
                    b: b.clone(),
                });
            }
        }

        let name = weapon_name(family, &picks);
        let id = content
            .lookup(namespace, &name)
            .ok_or_else(|| ConfigurationRefusal::MissingContent(format!("{name}_mp")))?;
        content.admission(id)?;
        Ok(ResolvedConfiguration {
            id,
            selection: WeaponSelection::with(key.clone(), &picks),
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
        let key = &family.key;
        let current = canonical(&selection.attachments);
        let options = family
            .attachments
            .iter()
            .map(|choice| {
                let selected = current.contains(&choice.name);
                let mut flipped = current.clone();
                if selected {
                    flipped.retain(|name| *name != choice.name);
                } else {
                    flipped.push(choice.name.clone());
                }
                let toggle = self
                    .resolve(&WeaponSelection::with(key.clone(), &flipped), limits, content)
                    .map(|resolved| resolved.id);
                AttachmentOption {
                    choice: choice.clone(),
                    selected,
                    toggle,
                }
            })
            .collect();
        Ok(options)
    }
}
