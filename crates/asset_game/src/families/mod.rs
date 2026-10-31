//! Weapon families: the weapons a class can carry, the attachments each one
//! offers, and the concrete weapon that a family plus an attachment choice
//! stands for.

mod resolve;
mod tables;

use std::collections::HashMap;

use crate::{AssetIdentity, AssetVariety, AssetCategoryScope, SnapshotStringTable};
use tables::AttachmentRules;

/// Limits a class puts on one weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadoutRules {
    pub max_attachments: usize,
}

impl Default for LoadoutRules {
    fn default() -> Self {
        Self { max_attachments: 2 }
    }
}

/// A weapon family: the game it belongs to and its base name, lower case and
/// without the `_mp` suffix.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FamilyKey {
    pub game: AssetCategoryScope,
    pub base: String,
}

impl FamilyKey {
    pub fn new(namespace: AssetCategoryScope, base: &str) -> Self {
        let lower = base.to_ascii_lowercase();
        let base = lower.strip_suffix("_mp").unwrap_or(&lower).to_owned();
        Self { game: namespace, base }
    }

    /// Accepts a full weapon asset key or `game:name`.
    pub fn parse(raw: &str) -> Option<Self> {
        if let Ok(key) = AssetIdentity::parse(raw) {
            return (key.kind == AssetVariety::Weapon).then(|| Self::new(key.namespace, &key.name));
        }
        let (game, name) = raw.split_once(':')?;
        let namespace = AssetCategoryScope::parse(game)?;
        let plain = !name.is_empty() && !name.contains('/');
        plain.then(|| Self::new(namespace, name))
    }

    /// `game:name`, with a leading `game_` taken off the name.
    pub fn short(&self) -> String {
        let game = self.game.as_str();
        let name = self
            .base
            .strip_prefix(game)
            .and_then(|rest| rest.strip_prefix('_'))
            .unwrap_or(&self.base);
        format!("{game}:{name}")
    }

    pub fn asset_key(&self) -> String {
        format!("{}:weapon/{}_mp", self.game.as_str(), self.base)
    }
}

impl core::fmt::Display for FamilyKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.asset_key())
    }
}

/// The class slot a family fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilySlot {
    Primary,
    Secondary,
    Lethal,
    Tactical,
    /// Authored, but not something a class can pick.
    Other,
}

/// One attachment a family offers, with the text and picture the class menu
/// shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentChoice {
    pub name: String,
    pub point: String,
    pub caption_key: String,
    pub icon: String,
    pub description_lookup: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponFamily {
    pub key: FamilyKey,
    pub loot_bundle: String,
    pub bay: FamilySlot,
    pub display_key: String,
    pub image: String,
    /// The bare weapon's id, when the loaded content has it.
    pub base: Option<u32>,
    pub attachments: Vec<AttachmentChoice>,
}

/// A family and the attachments picked on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct WeaponSelection {
    pub family: Option<FamilyKey>,
    pub attachments: Vec<String>,
}

impl WeaponSelection {
    pub fn bare(family: FamilyKey) -> Self {
        Self::with(family, &[])
    }

    pub fn with(family: FamilyKey, attachments: &[String]) -> Self {
        Self {
            family: Some(family),
            attachments: attachments.to_vec(),
        }
    }
}

/// Why a selection cannot become a weapon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationRefusal {
    UnknownFamily(String),
    UnknownAttachment(String),
    NotOffered(String),
    Incompatible {
        a: String,
        b: String,
    },
    RuleRestricted(String),
    AbsentResource(String),
    MissingDependency {
        gun_name: String,
        gap_kind: &'static str,
        name: String,
    },
}

impl ConfigurationRefusal {
    /// A stable short name for logs and console output.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownFamily(_) => "weapon.unknown_family",
            Self::UnknownAttachment(_) => "weapon.unknown_attachment",
            Self::NotOffered(_) => "weapon.not_offered",
            Self::Incompatible { .. } => "weapon.incompatible",
            Self::RuleRestricted(_) => "weapon.rule_restricted",
            Self::AbsentResource(_) => "weapon.missing_content",
            Self::MissingDependency { .. } => "weapon.missing_dependency",
        }
    }
}

impl core::fmt::Display for ConfigurationRefusal {
    fn fmt(&self, sink: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFamily(name) => write!(sink, "no weapon family `{name}`"),
            Self::UnknownAttachment(name) => write!(sink, "no attachment `{name}` in this game"),
            Self::NotOffered(name) => write!(sink, "`{name}` is not offered for this weapon"),
            Self::Incompatible { a, b } => write!(sink, "`{a}` and `{b}` cannot be combined"),
            Self::RuleRestricted(why) => write!(sink, "not allowed here: {why}"),
            Self::AbsentResource(name) => write!(sink, "`{name}` is not in the loaded content"),
            Self::MissingDependency { gun_name, gap_kind, name } => {
                write!(sink, "`{gun_name}` names {gap_kind} `{name}`, which is not loaded")
            }
        }
    }
}

impl std::error::Error for ConfigurationRefusal {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedConfiguration {
    pub id: u32,
    pub selection: WeaponSelection,
}

/// One row of the attachment picker: the choice, whether it is on, and what
/// toggling it would give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentOption {
    pub choice: AttachmentChoice,
    pub selected: bool,
    pub toggle: Result<u32, ConfigurationRefusal>,
}

/// What the families need to know about the loaded weapons.
pub(crate) trait FamilyContent {
    fn lookup(&self, namespace: AssetCategoryScope, gun_name: &str) -> Option<u32>;
    fn secondary_hand_category(&self, id: u32) -> i32;
    fn admission(&self, id: u32) -> Result<(), ConfigurationRefusal>;
    /// Every weapon of the game as (id, name without `_mp`).
    fn names_in(&self, namespace: AssetCategoryScope) -> Vec<(u32, String)>;
}

#[derive(Clone, Debug, Default)]
pub struct WeaponFamilies {
    families: Vec<WeaponFamily>,
    index: HashMap<FamilyKey, usize>,
    rules: HashMap<AssetCategoryScope, AttachmentRules>,
    /// The selection each weapon id comes from, when it is the canonical one.
    canon_by_id: HashMap<u32, WeaponSelection>,
    /// Families left out of the class menu, with the reason.
    excluded: Vec<(String, String)>,
    /// The perks the stats tables offer, in table order.
    perks: Vec<crate::LoadoutPerkEntry>,
}

impl WeaponFamilies {
    pub(crate) fn build(
        tables: &[(AssetCategoryScope, SnapshotStringTable)],
        content: &impl FamilyContent,
    ) -> Self {
        let mut out = Self::default();
        out.read_attachment_tables(tables);
        let stats_tables = tables.iter().filter(|(_, table)| crate::names_scoreboard_sheet(&table.name));
        for (scope, table) in stats_tables {
            out.read_family_rows(*scope, table, content);
            out.perks.extend(crate::perk_rows(table));
        }
        out.index_weapons(content);
        out
    }

    /// The perks a class can pick, in stats table order.
    pub fn perks(&self) -> &[crate::LoadoutPerkEntry] {
        &self.perks
    }

    /// The perk with the stats table item number `index`.
    pub fn perk(&self, index: u32) -> Option<&crate::LoadoutPerkEntry> {
        self.perks.iter().find(|perk| perk.index == index)
    }

    /// The perk whose reference (the specialties joined by `|`) is `reference`.
    pub fn perk_by_reference(&self, reference: &str) -> Option<&crate::LoadoutPerkEntry> {
        self.perks.iter().find(|perk| perk.reference == reference)
    }

    pub fn families(&self) -> &[WeaponFamily] {
        &self.families
    }

    pub fn family(&self, key: &FamilyKey) -> Option<&WeaponFamily> {
        self.index.get(key).map(|&at| &self.families[at])
    }

    /// Like [`Self::family`], but also tries the name with the game's own
    /// prefix put back on.
    pub fn find(&self, wanted: &FamilyKey) -> Option<&WeaponFamily> {
        self.family(wanted).or_else(|| {
            let prefixed = FamilyKey {
                game: wanted.game,
                base: format!("{}_{}", wanted.game.as_str(), wanted.base),
            };
            self.family(&prefixed)
        })
    }

    /// The families a class can pick from.
    pub fn offered(&self) -> impl Iterator<Item = &WeaponFamily> {
        self.families.iter().filter(|family| {
            let key = family.key.asset_key();
            family.bay != FamilySlot::Other && !self.excluded.iter().any(|(left_out, _)| *left_out == key)
        })
    }

    pub fn excluded(&self) -> &[(String, String)] {
        &self.excluded
    }

    /// The selection a weapon id is the canonical form of.
    pub fn describe(&self, id: u32) -> Option<&WeaponSelection> {
        self.canon_by_id.get(&id)
    }
}
