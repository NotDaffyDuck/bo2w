//! What the class editor works on: the rows of a class, the choices each row
//! offers, and the loadout of one class slot.

use std::collections::BTreeMap;
use std::sync::Arc;

use asset_core::{AssetKey, AssetNamespace};
use asset_game::{
    CacAuthoredCategory, CacPerkSlot, CacWeaponPreview, CapturedStringTable, ConfigurationRefusal,
    FamilyKey, FamilySlot, LoadoutRules, WeaponRegistry, WeaponSelection,
};
use bevy::prelude::*;

use crate::classes::icons::{cac_material_iwd_stem, cac_weapon_image};

/// The "nothing chosen" perk every perk row can fall back to.
pub const NO_PERK: &str = "specialty_null";

const NOT_READY: &str = "Weapon catalog is not ready";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassEditRow {
    Primary,
    Secondary,
    Lethal,
    Tactical,
    Perk1,
    Perk2,
    Perk3,
    Deathstreak,
}

impl ClassEditRow {
    /// Top to bottom, as the editor lists them.
    pub const ALL: [Self; 8] = [
        Self::Primary,
        Self::Secondary,
        Self::Lethal,
        Self::Tactical,
        Self::Perk1,
        Self::Perk2,
        Self::Perk3,
        Self::Deathstreak,
    ];

    pub fn label(self) -> &'static str {
        const LABELS: [&str; 8] = [
            "Primary",
            "Secondary",
            "Lethal",
            "Tactical",
            "Perk 1",
            "Perk 2",
            "Perk 3",
            "Deathstreak",
        ];
        LABELS[self as usize]
    }

    /// Localisation key of the row heading.
    pub fn loc_key(self) -> &'static str {
        const KEYS: [&str; 8] = [
            "@MENU_PRIMARY_CAPS",
            "@MENU_SECONDARY_CAPS",
            "@MENU_EQUIPMENT_CAPS",
            "@MENU_SPECIAL_GRENADE_CAPS",
            "@MENU_PERK1_CAPS",
            "@MENU_PERK2_CAPS",
            "@MENU_PERK3_CAPS",
            "@MENU_DEATHSTREAK_CAPS",
        ];
        KEYS[self as usize]
    }

    /// The perk column a row draws from; `None` for the weapon rows.
    pub fn perk_slot(self) -> Option<CacPerkSlot> {
        match self {
            Self::Perk1 => Some(CacPerkSlot::Perk1),
            Self::Perk2 => Some(CacPerkSlot::Perk2),
            Self::Perk3 => Some(CacPerkSlot::Perk3),
            Self::Deathstreak => Some(CacPerkSlot::Deathstreak),
            Self::Primary | Self::Secondary | Self::Lethal | Self::Tactical => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        Self::ALL.get(usize::from(value)).copied()
    }
}

/// Everything a class may be built from, plus the registry that vouches for it.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct ClassLoadoutCatalog {
    /// Bumped whenever the offers are replaced, so open menus rebuild.
    pub revision: u64,
    pub primary: Vec<frame::CacWeaponOffer>,
    pub secondary: Vec<frame::CacWeaponOffer>,
    pub lethal: Vec<frame::CacWeaponOffer>,
    pub tactical: Vec<frame::CacWeaponOffer>,
    pub perks: [Vec<String>; 3],
    pub deathstreak: Vec<String>,
    pub excluded: Vec<(String, String)>,
    pub previews: BTreeMap<String, CacWeaponPreview>,
    pub resolver: CatalogResolver,
}

impl Default for ClassLoadoutCatalog {
    /// No weapons yet; the perks start from the built-in rosters.
    fn default() -> Self {
        let roster = |row| cac_roster(row).map(str::to_owned).collect::<Vec<_>>();
        Self {
            revision: 0,
            primary: Vec::new(),
            secondary: Vec::new(),
            lethal: Vec::new(),
            tactical: Vec::new(),
            perks: [
                roster(ClassEditRow::Perk1),
                roster(ClassEditRow::Perk2),
                roster(ClassEditRow::Perk3),
            ],
            deathstreak: roster(ClassEditRow::Deathstreak),
            excluded: Vec::new(),
            previews: BTreeMap::new(),
            resolver: CatalogResolver::default(),
        }
    }
}

/// Looks up the weapon a folder entry stands for, by its item group.
fn authored_category(offer: &frame::CacWeaponOffer) -> Option<CacAuthoredCategory> {
    offer
        .item_group
        .as_deref()
        .and_then(asset_game::cac_category_from_item_group)
}

fn in_namespace(offer: &frame::CacWeaponOffer, namespace: AssetNamespace) -> bool {
    AssetKey::parse(&offer.key).is_ok_and(|key| key.namespace == namespace)
}

impl ClassLoadoutCatalog {
    pub fn from_weapon_registry(registry: Arc<WeaponRegistry>) -> Self {
        let mut catalog = Self::default();
        let families = registry.weapon_families();

        for family in families.offered() {
            let key = family.key.asset_key();

            catalog.previews.insert(
                key.clone(),
                CacWeaponPreview {
                    reference: family.key.base.clone(),
                    name_key: format!("@{}", family.display_key.trim_start_matches('@')),
                    image: family.image.clone(),
                    ..Default::default()
                },
            );
            for choice in &family.attachments {
                let preview = CacWeaponPreview {
                    reference: choice.name.clone(),
                    name_key: format!("@{}", choice.caption_key),
                    image: choice.icon.clone(),
                    desc_key: format!("@{}", choice.desc_key),
                    bars: Vec::new(),
                };
                catalog
                    .previews
                    .insert(attachment_preview_key(&key, &choice.name), preview);
            }

            let list = match family.slot {
                FamilySlot::Primary => &mut catalog.primary,
                FamilySlot::Secondary => &mut catalog.secondary,
                FamilySlot::Lethal => &mut catalog.lethal,
                FamilySlot::Tactical => &mut catalog.tactical,
                FamilySlot::Other => continue,
            };
            list.push(frame::CacWeaponOffer {
                key,
                item_group: Some(family.item_group.clone()),
                attachments: family.attachments.iter().map(|c| c.name.clone()).collect(),
            });
        }

        catalog.excluded = families.excluded().to_vec();
        catalog.resolver = CatalogResolver(Some(registry));
        catalog
    }

    /// Swaps in the authored wording of every preview that a loaded stats
    /// table has an entry for.
    pub fn with_weapon_tables(mut self, tables: &[(AssetNamespace, CapturedStringTable)]) -> Self {
        let stats_tables = tables
            .iter()
            .filter(|(_, table)| asset_game::is_stats_table_name(&table.name));
        for (namespace, table) in stats_tables {
            for (key, preview) in self.previews.iter_mut() {
                let ours = AssetKey::parse(key).is_ok_and(|parsed| parsed.namespace == *namespace);
                if !ours {
                    continue;
                }
                if let Some(authored) = asset_game::weapon_preview(table, key) {
                    *preview = authored;
                }
            }
        }
        self
    }

    pub fn options(&self, row: ClassEditRow) -> Vec<String> {
        match row {
            ClassEditRow::Perk1 => self.perks[0].clone(),
            ClassEditRow::Perk2 => self.perks[1].clone(),
            ClassEditRow::Perk3 => self.perks[2].clone(),
            ClassEditRow::Deathstreak => self.deathstreak.clone(),
            weapon_row => self
                .offers(weapon_row)
                .iter()
                .map(|offer| offer.key.clone())
                .collect(),
        }
    }

    /// The weapon offers behind a row; perk rows have none.
    fn offers(&self, row: ClassEditRow) -> &[frame::CacWeaponOffer] {
        match row {
            ClassEditRow::Primary => &self.primary,
            ClassEditRow::Secondary => &self.secondary,
            ClassEditRow::Lethal => &self.lethal,
            ClassEditRow::Tactical => &self.tactical,
            _ => &[],
        }
    }

    /// Replaces the perk choices with the ones a perk table lists. A table
    /// with no rows leaves the built-in rosters alone.
    pub fn with_perk_table(mut self, table: &CapturedStringTable) -> Self {
        let rows = asset_game::perk_rows(table);
        if rows.is_empty() {
            return self;
        }
        let column = |slot: CacPerkSlot| -> Vec<String> {
            rows.iter()
                .filter(|row| row.reference == NO_PERK || row.slot == slot)
                .map(|row| row.reference.clone())
                .collect()
        };
        self.perks = [
            column(CacPerkSlot::Perk1),
            column(CacPerkSlot::Perk2),
            column(CacPerkSlot::Perk3),
        ];
        self.deathstreak = column(CacPerkSlot::Deathstreak);

        for row in rows {
            let preview = CacWeaponPreview {
                reference: row.reference.clone(),
                name_key: row.name_key,
                image: row.image,
                desc_key: row.desc_key,
                ..Default::default()
            };
            self.previews.insert(row.reference, preview);
        }
        self
    }

    /// The folders a weapon row is sorted into, in display order.
    pub fn categories_for(&self, row: ClassEditRow) -> Vec<ClassPickerFolder> {
        let by_category = matches!(row, ClassEditRow::Primary | ClassEditRow::Secondary);
        let mut folders: Vec<ClassPickerFolder> = self
            .offers(row)
            .iter()
            .filter_map(|offer| {
                let namespace = AssetKey::parse(&offer.key).ok()?.namespace;
                let category = if by_category {
                    Some(authored_category(offer)?)
                } else {
                    None
                };
                Some(ClassPickerFolder {
                    namespace,
                    category,
                })
            })
            .collect();
        folders.sort_by_key(|folder| (folder.namespace.as_str(), folder.category));
        folders.dedup();
        folders
    }

    pub fn keys_in_category(&self, row: ClassEditRow, folder: ClassPickerFolder) -> Vec<String> {
        self.offers(row)
            .iter()
            .filter(|offer| in_namespace(offer, folder.namespace))
            .filter(|offer| {
                folder
                    .category
                    .is_none_or(|wanted| authored_category(offer) == Some(wanted))
            })
            .map(|offer| offer.key.clone())
            .collect()
    }

    /// Weapon rows are browsed through folders; perk rows are one flat list.
    pub fn uses_categories(row: ClassEditRow) -> bool {
        row.perk_slot().is_none()
    }

    pub fn attachments(&self, row: ClassEditRow, weapon: &str) -> &[String] {
        match self.offers(row).iter().find(|offer| offer.key == weapon) {
            Some(offer) => &offer.attachments,
            None => &[],
        }
    }

    fn registry(&self) -> Result<&WeaponRegistry, String> {
        self.resolver.0.as_deref().ok_or_else(|| NOT_READY.to_owned())
    }

    /// Whether the whole class could be equipped as it stands.
    pub fn validate_class(&self, slot: &ClassSlotState) -> Result<(), String> {
        let registry = self.registry()?;
        let row = session::ClassRow::from(&frame::HostClassSlot::from(slot));
        session::loadout::resolve_personal_class(&row, registry)?;
        Ok(())
    }

    /// Whether the value in one row of the slot is acceptable.
    pub fn validate_edit(&self, slot: &ClassSlotState, row: ClassEditRow) -> Result<(), String> {
        let registry = self.registry()?;
        let value = slot.row_value(row);
        if !Self::uses_categories(row) {
            return if self.options(row).iter().any(|option| option == value) {
                Ok(())
            } else {
                Err("Perk is unavailable".to_owned())
            };
        }
        let attachments: &[String] = match row {
            ClassEditRow::Primary => &slot.primary_attachments,
            ClassEditRow::Secondary => &slot.secondary_attachments,
            _ => &[],
        };
        session::resolve_class_weapon(registry, value, attachments, slot.loadout_rules())?;
        Ok(())
    }

    pub fn check_attachments(
        &self,
        weapon: &str,
        attachments: &[String],
        rules: LoadoutRules,
    ) -> Result<(), ConfigurationRefusal> {
        let Some(registry) = self.resolver.0.as_deref() else {
            return Err(ConfigurationRefusal::MissingContent(NOT_READY.into()));
        };
        let Some(family) = FamilyKey::parse(weapon) else {
            return Err(ConfigurationRefusal::UnknownFamily(weapon.to_owned()));
        };
        let selection = WeaponSelection::with(family, attachments);
        registry.resolve_configuration(&selection, rules)?;
        Ok(())
    }
}

/// Key under which an attachment's preview is stored next to its weapon's.
pub fn attachment_preview_key(weapon: &str, attachment: &str) -> String {
    format!("{weapon}+{attachment}")
}

/// A shared handle on the registry. Two handles are equal when they point at
/// the same registry, so a catalog compares by identity rather than contents.
#[derive(Clone, Default)]
pub struct CatalogResolver(pub Option<Arc<WeaponRegistry>>);

impl PartialEq for CatalogResolver {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(ours), Some(theirs)) => Arc::ptr_eq(ours, theirs),
            (ours, theirs) => ours.is_none() && theirs.is_none(),
        }
    }
}

impl Eq for CatalogResolver {}

impl std::fmt::Debug for CatalogResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = if self.0.is_some() { "registry" } else { "none" };
        write!(f, "CatalogResolver({state})")
    }
}

/// One class as the editor holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassSlotState {
    pub name: String,
    pub primary: String,
    pub primary_attachments: Vec<String>,
    pub secondary: String,
    pub secondary_attachments: Vec<String>,
    pub lethal: String,
    pub tactical: String,
    pub perk1: String,
    pub perk2: String,
    pub perk3: String,
    pub deathstreak: String,
    /// Why the class cannot be equipped right now, if it cannot.
    pub lock_reason: Option<String>,
}

fn owned_list(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

impl ClassSlotState {
    pub fn from_host_slot(slot: &frame::HostClassSlot) -> Self {
        let [perk1, perk2, perk3] = slot.perks.clone();
        Self {
            name: slot.name.clone(),
            primary: slot.primary.clone(),
            primary_attachments: slot.primary_attachments.clone(),
            secondary: slot.secondary.clone(),
            secondary_attachments: slot.secondary_attachments.clone(),
            lethal: slot.lethal.clone(),
            tactical: slot.tactical.clone(),
            perk1,
            perk2,
            perk3,
            deathstreak: slot.deathstreak.clone(),
            lock_reason: None,
        }
    }

    /// A preset names its empty perk columns with an empty string.
    pub fn from_preset(preset: &crate::ClassPreset) -> Self {
        let perk = |column: usize| match preset.perks[column] {
            "" => NO_PERK.to_owned(),
            named => named.to_owned(),
        };
        Self {
            name: preset.name.to_owned(),
            primary: preset.primary.to_owned(),
            primary_attachments: owned_list(preset.primary_attachments),
            secondary: preset.secondary.to_owned(),
            secondary_attachments: owned_list(preset.secondary_attachments),
            lethal: preset.lethal.to_owned(),
            tactical: preset.tactical.to_owned(),
            perk1: perk(0),
            perk2: perk(1),
            perk3: perk(2),
            deathstreak: preset.deathstreak.to_owned(),
            lock_reason: None,
        }
    }

    pub fn row_value(&self, row: ClassEditRow) -> &str {
        match row {
            ClassEditRow::Primary => &self.primary,
            ClassEditRow::Secondary => &self.secondary,
            ClassEditRow::Lethal => &self.lethal,
            ClassEditRow::Tactical => &self.tactical,
            ClassEditRow::Perk1 => &self.perk1,
            ClassEditRow::Perk2 => &self.perk2,
            ClassEditRow::Perk3 => &self.perk3,
            ClassEditRow::Deathstreak => &self.deathstreak,
        }
    }

    /// The first perk decides which attachment rules apply to the class.
    pub fn loadout_rules(&self) -> LoadoutRules {
        LoadoutRules::for_class(&self.perk1)
    }

    /// Sets a row. A new weapon starts without the old weapon's attachments.
    pub fn set_row(&mut self, row: ClassEditRow, value: String) {
        let weapon = match row {
            ClassEditRow::Primary => Some((&mut self.primary, &mut self.primary_attachments)),
            ClassEditRow::Secondary => {
                Some((&mut self.secondary, &mut self.secondary_attachments))
            }
            _ => None,
        };
        if let Some((current, attachments)) = weapon {
            if *current != value {
                *current = value;
                attachments.clear();
            }
            return;
        }
        let target = match row {
            ClassEditRow::Lethal => &mut self.lethal,
            ClassEditRow::Tactical => &mut self.tactical,
            ClassEditRow::Perk1 => &mut self.perk1,
            ClassEditRow::Perk2 => &mut self.perk2,
            ClassEditRow::Perk3 => &mut self.perk3,
            _ => &mut self.deathstreak,
        };
        *target = value;
    }
}

/// One folder of the picker: a namespace, split by weapon category for the
/// weapon rows that have categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassPickerFolder {
    pub namespace: AssetNamespace,
    pub category: Option<CacAuthoredCategory>,
}

impl ClassPickerFolder {
    pub fn slug(self) -> String {
        let namespace = self.namespace.as_str();
        match self.category {
            Some(category) => format!("{namespace}:{}", category.slug()),
            None => namespace.to_owned(),
        }
    }
}

/// The built-in choices of a row, used until a loaded catalog replaces them.
fn cac_roster(row: ClassEditRow) -> impl Iterator<Item = &'static str> {
    let words: &'static str = match row {
        ClassEditRow::Primary => {
            "m4_mp famas_mp scar_mp tar21_mp fal_mp m16_mp masada_mp fn2000_mp \
             ak47_mp mp5k_mp uzi_mp p90_mp kriss_mp ump45_mp rpd_mp sa80_mp \
             mg4_mp m240_mp aug_mp barrett_mp cheytac_mp wa2000_mp m21_mp riotshield_mp"
        }
        ClassEditRow::Secondary => {
            "glock_mp beretta393_mp pp2000_mp tmp_mp ranger_mp model1887_mp \
             striker_mp aa12_mp m1014_mp spas12_mp usp_mp beretta_mp \
             deserteagle_mp coltanaconda_mp at4_mp rpg_mp stinger_mp javelin_mp"
        }
        ClassEditRow::Lethal => "frag_grenade_mp semtex_mp throwingknife_mp claymore_mp c4_mp",
        ClassEditRow::Tactical => {
            "flash_grenade_mp concussion_grenade_mp smoke_grenade_mp flare_mp"
        }
        ClassEditRow::Perk1 => {
            "specialty_null specialty_marathon specialty_fastreload \
             specialty_scavenger specialty_onemanarmy specialty_bling"
        }
        ClassEditRow::Perk2 => {
            "specialty_null specialty_bulletdamage specialty_lightweight \
             specialty_hardline specialty_coldblooded specialty_explosivedamage"
        }
        ClassEditRow::Perk3 => {
            "specialty_null specialty_extendedmelee specialty_bulletaccuracy \
             specialty_localjammer specialty_heartbreaker specialty_detectexplosive \
             specialty_pistoldeath"
        }
        ClassEditRow::Deathstreak => {
            "specialty_null specialty_grenadepulldeath specialty_c4death \
             specialty_combathigh specialty_finalstand specialty_copycat"
        }
    };
    words.split_whitespace()
}

/// Every icon stem the picker could ask for with the built-in rosters, sorted
/// and without repeats.
pub fn picker_icon_stems() -> Vec<&'static str> {
    let mut stems: Vec<&'static str> = ClassEditRow::ALL
        .into_iter()
        .flat_map(|row| {
            let is_perk = row.perk_slot().is_some();
            cac_roster(row).filter_map(move |name| {
                if is_perk {
                    Some(cac_material_iwd_stem(name))
                } else {
                    cac_weapon_image(name)
                }
            })
        })
        .collect();
    stems.sort_unstable();
    stems.dedup();
    stems
}
