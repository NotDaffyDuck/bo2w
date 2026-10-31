//! Data behind the class editor: which rows a class has, what each row can be
//! set to, and the weapons and perks one class slot holds.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use asset_core::{AssetIdentity, AssetCategoryScope};
use asset_game::{
    AttachmentChoice, LoadoutPlacedGroup, LoadoutAbilitySlot, LoadoutGunShowcase, SnapshotStringTable,
    ConfigurationRefusal, FamilyKey, FamilySlot, LoadoutRules, WeaponFamily, WeaponLedger,
    WeaponSelection,
};
use bevy::prelude::*;
use frame::CustomClassGunChoice;

use crate::classes::icons::{loadout_surface_pack_stem, loadout_gun_thumb};


/// Value of a perk row that has nothing picked.
pub const NO_PERK: &str = concat!("specialty", "_null");

const NOT_READY: &str = "Weapon catalog is not ready";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadoutEditorLine {
    Primary,
    Secondary,
    Lethal,
    Tactical,
    Perk1,
    Perk2,
    Perk3,
}

impl LoadoutEditorLine {
    /// Every row in the order the editor stacks them.
    pub const ALL: [Self; 7] = {
        use LoadoutEditorLine::*;
        [Primary, Secondary, Lethal, Tactical, Perk1, Perk2, Perk3]
    };

    /// Heading shown for each row, paired with the localised-string key that
    /// supplies it in the game's own menus.
    const WORDING: [(&str, &str); 7] = [
        ("Primary", "@MENU_PRIMARY_CAPS"),
        ("Secondary", "@MENU_SECONDARY_CAPS"),
        ("Lethal", "@MENU_EQUIPMENT_CAPS"),
        ("Tactical", "@MENU_SPECIAL_GRENADE_CAPS"),
        ("Perk 1", "@MENU_PERK1_CAPS"),
        ("Perk 2", "@MENU_PERK2_CAPS"),
        ("Perk 3", "@MENU_PERK3_CAPS"),
    ];

    pub fn label(self) -> &'static str {
        Self::WORDING[self as usize].0
    }

    /// Localised-string key for this row's heading.
    pub fn text_lookup_token(self) -> &'static str {
        Self::WORDING[self as usize].1
    }

    /// Which perk column the row picks from, if it is a perk row at all.
    pub fn perk_slot(self) -> Option<LoadoutAbilitySlot> {
        let first_perk = Self::Perk1 as usize;
        (self as usize)
            .checked_sub(first_perk)
            .and_then(|offset| LoadoutAbilitySlot::ALL_KINDS.get(offset).copied())
    }

    pub fn to_byte_value(self) -> u8 {
        self as u8
    }

    pub fn from_u8(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().nth(byte.into())
    }
}

/// All the weapons and perks a class can be built from, and the registry
/// that checks them.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct LoadoutRoster {
    pub resolver: CatalogResolver,
    pub previews: BTreeMap<String, LoadoutGunShowcase>,
    pub left_out: Vec<(String, String)>,
    pub perks: [Vec<String>; 3],
    pub tactical: Vec<CustomClassGunChoice>,
    pub lethal: Vec<CustomClassGunChoice>,
    pub secondary: Vec<CustomClassGunChoice>,
    pub primary: Vec<CustomClassGunChoice>,
    /// Changes every time the offers are swapped, which tells open menus to
    /// redraw.
    pub swap_count: u64,
}

impl Default for LoadoutRoster {
    /// Empty: no weapons, and each perk column holds just the "none" entry
    /// until a registry supplies the real ones.
    fn default() -> Self {
        Self {
            resolver: CatalogResolver::default(),
            previews: BTreeMap::default(),
            left_out: Vec::default(),
            perks: std::array::from_fn(|_| vec![NO_PERK.to_owned()]),
            tactical: Vec::default(),
            lethal: Vec::default(),
            secondary: Vec::default(),
            primary: Vec::default(),
            swap_count: u64::default(),
        }
    }
}

/// The picker category of an offer, taken from its item group.
fn authored_category(offer: &CustomClassGunChoice) -> Option<LoadoutPlacedGroup> {
    let group = offer.loot_bundle.as_ref()?;
    asset_game::loadout_category_of_group(group)
}

/// The namespace a key text belongs to, when the text parses as a key.
fn namespace_of(key: &str) -> Option<AssetCategoryScope> {
    AssetIdentity::parse(key).ok().map(|parsed| parsed.namespace)
}

fn in_namespace(offer: &CustomClassGunChoice, namespace: AssetCategoryScope) -> bool {
    namespace_of(&offer.key) == Some(namespace)
}

fn family_preview(family: &WeaponFamily) -> LoadoutGunShowcase {
    let heading = family.display_key.trim_start_matches('@');
    LoadoutGunShowcase {
        reference: family.key.base.clone(),
        title_token: ["@", heading].concat(),
        image: family.image.clone(),
        ..Default::default()
    }
}

fn attachment_preview(choice: &AttachmentChoice) -> LoadoutGunShowcase {
    LoadoutGunShowcase {
        reference: choice.name.clone(),
        title_token: ["@", choice.caption_key.as_str()].concat(),
        description_lookup: ["@", choice.description_lookup.as_str()].concat(),
        image: choice.icon.clone(),
        bars: vec![],
    }
}

impl LoadoutRoster {
    pub fn from_weapon_registry(registry: Arc<WeaponLedger>) -> Self {
        let mut built = Self::default();
        let families = registry.weapon_families();
        for family in families.offered() {
            let key = family.key.asset_key();
            built.previews.insert(key.clone(), family_preview(family));
            built
                .previews
                .extend(family.attachments.iter().map(|choice| {
                    (
                        attachment_preview_key(&key, &choice.name),
                        attachment_preview(choice),
                    )
                }));
            if let Some(list) = built.list_for(family.bay) {
                list.push(CustomClassGunChoice {
                    attachments: family.attachments.iter().map(|c| c.name.clone()).collect(),
                    loot_bundle: Some(family.loot_bundle.clone()),
                    key,
                });
            }
        }
        built.add_perks(families.perks());
        built.left_out = families.excluded().to_vec();
        Self {
            resolver: CatalogResolver(Some(registry)),
            ..built
        }
    }

    /// The offer list a family of this slot is filed under, if any.
    fn list_for(&mut self, slot: FamilySlot) -> Option<&mut Vec<CustomClassGunChoice>> {
        Some(match slot {
            FamilySlot::Other => return None,
            FamilySlot::Tactical => &mut self.tactical,
            FamilySlot::Lethal => &mut self.lethal,
            FamilySlot::Secondary => &mut self.secondary,
            FamilySlot::Primary => &mut self.primary,
        })
    }

    /// Replaces a preview with its authored text and numbers wherever one of
    /// the loaded stats tables knows the weapon.
    pub fn with_weapon_tables(mut self, tables: &[(AssetCategoryScope, SnapshotStringTable)]) -> Self {
        let stats_tables = tables
            .iter()
            .filter(|entry| asset_game::names_scoreboard_sheet(&entry.1.name));
        for (space, table) in stats_tables {
            self.previews
                .iter_mut()
                .filter(|(gun, _)| namespace_of(gun) == Some(*space))
                .for_each(|(key, preview)| {
                    if let Some(replacement) = asset_game::gun_preview(table, key) {
                        *preview = replacement;
                    }
                });
        }
        self
    }

    pub fn options(&self, row: LoadoutEditorLine) -> Vec<String> {
        match row.perk_slot() {
            Some(column) => self.perks[column.position()].clone(),
            None => self.row_offers(row).iter().map(|o| o.key.clone()).collect(),
        }
    }

    /// Weapon offers for a row; empty for the perk rows.
    fn row_offers(&self, row: LoadoutEditorLine) -> &[CustomClassGunChoice] {
        let by_row = [&self.primary, &self.secondary, &self.lethal, &self.tactical];
        by_row.get(row as usize).map_or(&[], |list| list.as_slice())
    }

    /// Files each perk under the column of its slot (after the "none" entry)
    /// and remembers how to show it.
    fn add_perks(&mut self, rows: &[asset_game::LoadoutPerkEntry]) {
        for perk in rows {
            self.perks[perk.bay.position()].push(perk.reference.clone());
        }
        self.previews.extend(rows.iter().map(|perk| {
            let shown = LoadoutGunShowcase {
                reference: perk.reference.clone(),
                title_token: perk.title_token.clone(),
                description_lookup: perk.description_lookup.clone(),
                image: perk.image.clone(),
                ..Default::default()
            };
            (perk.reference.clone(), shown)
        }));
    }

    /// Folders to browse a weapon row by, ordered by namespace and category.
    pub fn groupings_of(&self, row: LoadoutEditorLine) -> Vec<ClassPickerFolder> {
        let split_by_category = matches!(row, LoadoutEditorLine::Primary | LoadoutEditorLine::Secondary);
        let folder_of = |offer: &CustomClassGunChoice| {
            let namespace = namespace_of(&offer.key)?;
            let category = match split_by_category {
                true => Some(authored_category(offer)?),
                false => None,
            };
            Some(ClassPickerFolder {
                namespace,
                category,
            })
        };
        let listed = self.row_offers(row);
        let by_order: BTreeMap<_, _> = listed
            .iter()
            .filter_map(folder_of)
            .map(|folder| ((folder.namespace.as_str(), folder.category), folder))
            .collect();
        by_order.into_values().collect()
    }

    pub fn keys_in_category(&self, row: LoadoutEditorLine, folder: ClassPickerFolder) -> Vec<String> {
        let pool = self.row_offers(row);
        let fits = |offer: &&CustomClassGunChoice| {
            in_namespace(offer, folder.namespace)
                && folder
                    .category
                    .map_or(true, |wanted| authored_category(offer) == Some(wanted))
        };
        pool
            .iter()
            .filter(fits)
            .map(|offer| offer.key.to_owned())
            .collect()
    }

    /// Weapon rows are browsed folder by folder; perk rows are a flat list.
    pub fn uses_categories(row: LoadoutEditorLine) -> bool {
        !matches!(
            row,
            LoadoutEditorLine::Perk1 | LoadoutEditorLine::Perk2 | LoadoutEditorLine::Perk3
        )
    }

    pub fn attachments(&self, row: LoadoutEditorLine, gun: &str) -> &[String] {
        let found = self.row_offers(row).iter().find(|offer| offer.key == gun);
        found.map_or(&[], |offer| offer.attachments.as_slice())
    }

    fn registry(&self) -> Result<&WeaponLedger, String> {
        match &self.resolver.0 {
            Some(shared) => Ok(shared),
            None => Err(NOT_READY.to_string()),
        }
    }

    /// Checks that the whole class could be put on as it is.
    pub fn validate_class(&self, slot: &LoadoutSlotStatus) -> Result<(), String> {
        let registry = self.registry()?;
        let as_row = session::ClassRow::from(&frame::ServerKitSlot::from(slot));
        session::loadout::resolve_personal_class(&as_row, registry).map(drop)
    }

    /// Checks the one row of the slot named by `row`.
    pub fn validate_edit(&self, slot: &LoadoutSlotStatus, row: LoadoutEditorLine) -> Result<(), String> {
        let registry = self.registry()?;
        let current = slot.value_in(row);
        if !Self::uses_categories(row) {
            return match self
                .options(row)
                .iter()
                .position(|choice| choice == current)
            {
                Some(_) => Result::Ok(()),
                None => Err("Perk is unavailable".to_owned()),
            };
        }
        session::resolve_class_weapon(
            registry,
            current,
            slot.attachments_of(row),
            slot.loadout_rules(),
        )
        .map(drop)
    }

    pub fn check_attachments(
        &self,
        gun: &str,
        attachments: &[String],
        rules: LoadoutRules,
    ) -> Result<(), ConfigurationRefusal> {
        let registry = self
            .resolver
            .0
            .as_deref()
            .ok_or_else(|| ConfigurationRefusal::AbsentResource(NOT_READY.into()))?;
        let family = FamilyKey::parse(gun)
            .ok_or_else(|| ConfigurationRefusal::UnknownFamily(gun.to_owned()))?;
        registry
            .resolve_configuration(&WeaponSelection::with(family, attachments), rules)
            .map(drop)
    }
}

/// Where an attachment's preview sits in the map, beside its weapon's own.
pub fn attachment_preview_key(weapon: &str, attachment: &str) -> String {
    [weapon, attachment].join("+")
}

/// A shared handle on the registry, equal to another only when both name the
/// same registry, so comparing catalogs never walks the registry's contents.
#[derive(Clone, Default)]
pub struct CatalogResolver(pub Option<Arc<WeaponLedger>>);

impl PartialEq for CatalogResolver {
    fn eq(&self, other: &Self) -> bool {
        let address = |handle: &Self| handle.0.as_ref().map(Arc::as_ptr);
        address(self) == address(other)
    }
}

impl Eq for CatalogResolver {}

impl std::fmt::Debug for CatalogResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            Some(_) => "CatalogResolver(registry)",
            None => "CatalogResolver(none)",
        })
    }
}

/// A class as the editor keeps it while it is being changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadoutSlotStatus {
    pub name: String,
    /// Set when the class cannot be used at the moment, with the reason.
    pub frozen_because: Option<String>,
    pub first_specialty: String,
    pub second_specialty: String,
    pub third_specialty: String,
    pub lethal: String,
    pub tactical: String,
    pub primary: String,
    pub primary_attachments: Vec<String>,
    pub secondary: String,
    pub secondary_attachments: Vec<String>,
}

fn owned_list(items: &[&str]) -> Vec<String> {
    items.iter().copied().map(String::from).collect()
}

/// Presets leave an unused perk column as an empty string.
fn perk_or_none(reference: &str) -> String {
    if reference.is_empty() {
        NO_PERK.to_owned()
    } else {
        reference.to_owned()
    }
}

impl LoadoutSlotStatus {
    pub fn from_host_slot(slot: &frame::ServerKitSlot) -> Self {
        let owned = slot.clone();
        let [first_specialty, perk2, perk3] = owned.perks;
        Self {
            frozen_because: Option::default(),
            name: owned.name,
            first_specialty,
            second_specialty: perk2,
            third_specialty: perk3,
            lethal: owned.lethal,
            tactical: owned.tactical,
            primary: owned.primary,
            primary_attachments: owned.main_gun_parts,
            secondary: owned.secondary,
            secondary_attachments: owned.side_gun_parts,
        }
    }

    pub fn from_preset(preset: &crate::KitTemplate) -> Self {
        let [first_specialty, perk2, perk3] = preset.perks.map(perk_or_none);
        Self {
            name: preset.name.into(),
            frozen_because: Option::default(),
            first_specialty,
            second_specialty: perk2,
            third_specialty: perk3,
            lethal: preset.lethal.into(),
            tactical: preset.tactical.into(),
            primary: preset.primary.into(),
            primary_attachments: owned_list(preset.main_gun_parts),
            secondary: preset.secondary.into(),
            secondary_attachments: owned_list(preset.side_gun_parts),
        }
    }

    pub fn value_in(&self, row: LoadoutEditorLine) -> &str {
        let in_row_order = [
            &self.primary,
            &self.secondary,
            &self.lethal,
            &self.tactical,
            &self.first_specialty,
            &self.second_specialty,
            &self.third_specialty,
        ];
        in_row_order[row as usize].as_str()
    }

    /// The attachments chosen on a row; only the two gun rows carry any.
    fn attachments_of(&self, row: LoadoutEditorLine) -> &[String] {
        match row {
            LoadoutEditorLine::Primary => &self.primary_attachments,
            LoadoutEditorLine::Secondary => &self.secondary_attachments,
            _ => &[],
        }
    }

    /// The limits applied to attachments on this class.
    pub fn loadout_rules(&self) -> LoadoutRules {
        LoadoutRules::default()
    }

    /// Puts `value` in a row. Switching to a different gun drops the
    /// attachments that belonged to the old one.
    pub fn put_in(&mut self, row: LoadoutEditorLine, value: String) {
        let (field, owned_attachments) = match row {
            LoadoutEditorLine::Primary => (&mut self.primary, Some(&mut self.primary_attachments)),
            LoadoutEditorLine::Secondary => (&mut self.secondary, Some(&mut self.secondary_attachments)),
            LoadoutEditorLine::Lethal => (&mut self.lethal, None),
            LoadoutEditorLine::Tactical => (&mut self.tactical, None),
            LoadoutEditorLine::Perk1 => (&mut self.first_specialty, None),
            LoadoutEditorLine::Perk2 => (&mut self.second_specialty, None),
            LoadoutEditorLine::Perk3 => (&mut self.third_specialty, None),
        };
        if let Some(attachments) = owned_attachments {
            if *field == value {
                return;
            }
            attachments.clear();
        }
        *field = value;
    }
}

/// A folder in the picker: one namespace, further divided by weapon category
/// on the rows that have categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassPickerFolder {
    pub namespace: AssetCategoryScope,
    pub category: Option<LoadoutPlacedGroup>,
}

impl ClassPickerFolder {
    pub fn slug(self) -> String {
        let ns = self.namespace.as_str();
        self.category
            .map_or_else(|| ns.to_owned(), |cat| format!("{ns}:{}", cat.slug()))
    }
}

const PRIMARY_NAMES: &str = "\
    m4_mp famas_mp scar_mp tar21_mp fal_mp m16_mp masada_mp fn2000_mp ak47_mp \
    mp5k_mp uzi_mp p90_mp kriss_mp ump45_mp rpd_mp sa80_mp mg4_mp m240_mp aug_mp \
    barrett_mp cheytac_mp wa2000_mp m21_mp riotshield_mp";

const SECONDARY_NAMES: &str = "\
    glock_mp beretta393_mp pp2000_mp tmp_mp ranger_mp model1887_mp striker_mp \
    aa12_mp m1014_mp spas12_mp usp_mp beretta_mp deserteagle_mp coltanaconda_mp \
    at4_mp rpg_mp stinger_mp javelin_mp";

const LETHAL_NAMES: &str = "frag_grenade_mp semtex_mp throwingknife_mp claymore_mp c4_mp";

const TACTICAL_NAMES: &str = "flash_grenade_mp concussion_grenade_mp smoke_grenade_mp \
    flare_mp";

/// What a row offers before any catalog has been loaded.
fn cac_roster(row: LoadoutEditorLine) -> impl Iterator<Item = &'static str> {
    let names = match row {
        LoadoutEditorLine::Primary => PRIMARY_NAMES,
        LoadoutEditorLine::Secondary => SECONDARY_NAMES,
        LoadoutEditorLine::Lethal => LETHAL_NAMES,
        LoadoutEditorLine::Tactical => TACTICAL_NAMES,
        LoadoutEditorLine::Perk1 | LoadoutEditorLine::Perk2 | LoadoutEditorLine::Perk3 => "",
    };
    names.split_whitespace()
}

/// The icon stems the picker may need for the built-in rosters, in order and
/// each only once.
pub fn chooser_thumb_stems() -> Vec<&'static str> {
    let rows_where = |perk_rows: bool| {
        let every_row = LoadoutEditorLine::ALL;
        every_row
            .into_iter()
            .filter(move |row| row.perk_slot().is_some() == perk_rows)
            .flat_map(cac_roster)
    };
    let weapons = rows_where(false).filter_map(loadout_gun_thumb);
    let perks = rows_where(true).map(loadout_surface_pack_stem);
    weapons
        .chain(perks)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(key: &str, group: Option<&str>, atts: &[&str]) -> frame::CustomClassGunChoice {
        frame::CustomClassGunChoice {
            key: key.to_owned(),
            loot_bundle: group.map(str::to_owned),
            attachments: owned_list(atts),
        }
    }

    fn sample_catalog() -> LoadoutRoster {
        let mut cat = LoadoutRoster::default();
        cat.primary = vec![
            offer(
                "t6:weapon/an94_mp",
                Some("weapon_assault"),
                &["acog", "silencer"],
            ),
            offer("t6:weapon/mp7_mp", Some("weapon_smg"), &[]),
            offer("t5:weapon/ak47_mp", Some("weapon_assault"), &[]),
            offer("t6:weapon/odd_mp", Some("weapon_bogus"), &[]),
        ];
        cat.lethal = vec![offer("t6:weapon/frag_grenade_mp", None, &[])];
        cat
    }

    fn sample_slot() -> LoadoutSlotStatus {
        LoadoutSlotStatus {
            name: "Custom 1".into(),
            primary: "p".into(),
            primary_attachments: owned_list(&["a", "b"]),
            secondary: "s".into(),
            secondary_attachments: owned_list(&["c"]),
            lethal: "l".into(),
            tactical: "t".into(),
            first_specialty: "x1".into(),
            second_specialty: "x2".into(),
            third_specialty: "x3".into(),
            frozen_because: Option::default(),
        }
    }

    #[test]
    fn rows_keep_their_order_and_numbers() {
        for (i, row) in LoadoutEditorLine::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(row.to_byte_value()), i);
            assert_eq!(LoadoutEditorLine::from_u8(i as u8), Some(row));
        }
        assert_eq!(LoadoutEditorLine::from_u8(7), None);
        assert_eq!(LoadoutEditorLine::Tactical.label(), "Tactical");
        assert_eq!(LoadoutEditorLine::Perk3.label(), "Perk 3");
        assert_eq!(LoadoutEditorLine::Lethal.text_lookup_token(), "@MENU_EQUIPMENT_CAPS");
        assert_eq!(LoadoutEditorLine::Perk2.text_lookup_token(), "@MENU_PERK2_CAPS");
    }

    #[test]
    fn only_perk_rows_have_a_perk_column() {
        assert_eq!(LoadoutEditorLine::Perk1.perk_slot(), Some(LoadoutAbilitySlot::Perk1));
        assert_eq!(LoadoutEditorLine::Perk3.perk_slot(), Some(LoadoutAbilitySlot::Perk3));
        assert!(
            LoadoutEditorLine::ALL[..4]
                .iter()
                .all(|r| r.perk_slot().is_none())
        );
        assert!(LoadoutRoster::uses_categories(
            LoadoutEditorLine::Secondary
        ));
        assert!(!LoadoutRoster::uses_categories(LoadoutEditorLine::Perk2));
    }

    #[test]
    fn empty_catalog_offers_only_no_perk() {
        let cat = LoadoutRoster::default();
        assert_eq!(cat.swap_count, 0);
        for row in [
            LoadoutEditorLine::Perk1,
            LoadoutEditorLine::Perk2,
            LoadoutEditorLine::Perk3,
        ] {
            assert_eq!(cat.options(row), vec![NO_PERK.to_owned()]);
        }
        assert!(cat.options(LoadoutEditorLine::Primary).is_empty());
        assert!(cat.validate_class(&sample_slot()).is_err());
        assert_eq!(
            cat.validate_edit(&sample_slot(), LoadoutEditorLine::Primary),
            Err(NOT_READY.to_owned())
        );
    }

    #[test]
    fn perks_land_in_their_own_column() {
        let row = |reference: &str, slot| asset_game::LoadoutPerkEntry {
            index: 1,
            reference: reference.into(),
            title_token: format!("@N_{reference}"),
            image: format!("img_{reference}"),
            description_lookup: format!("@D_{reference}"),
            bay: slot,
            default_in: Vec::new(),
        };
        let mut cat = LoadoutRoster::default();
        cat.add_perks(&[
            row("fast", LoadoutAbilitySlot::Perk1),
            row("quiet", LoadoutAbilitySlot::Perk3),
            row("tough", LoadoutAbilitySlot::Perk1),
        ]);
        assert_eq!(cat.perks[0], ["specialty_null", "fast", "tough"]);
        assert_eq!(cat.perks[1], ["specialty_null"]);
        assert_eq!(cat.perks[2], ["specialty_null", "quiet"]);
        let shown = &cat.previews["quiet"];
        assert_eq!(shown.title_token, "@N_quiet");
        assert_eq!(shown.image, "img_quiet");
        assert_eq!(shown.description_lookup, "@D_quiet");
    }

    #[test]
    fn weapon_options_and_attachments() {
        let cat = sample_catalog();
        assert_eq!(
            cat.options(LoadoutEditorLine::Lethal),
            ["t6:weapon/frag_grenade_mp"]
        );
        assert_eq!(cat.options(LoadoutEditorLine::Primary).len(), 4);
        assert_eq!(
            cat.attachments(LoadoutEditorLine::Primary, "t6:weapon/an94_mp"),
            ["acog", "silencer"]
        );
        assert!(cat.attachments(LoadoutEditorLine::Primary, "nope").is_empty());
        assert!(
            cat.attachments(LoadoutEditorLine::Perk1, "t6:weapon/an94_mp")
                .is_empty()
        );
    }

    #[test]
    fn folders_sort_by_namespace_then_category() {
        let cat = sample_catalog();
        let folders = cat.groupings_of(LoadoutEditorLine::Primary);
        let slugs: Vec<String> = folders
            .iter()
            .copied()
            .map(ClassPickerFolder::slug)
            .collect();
        assert_eq!(slugs.len(), 3);
        assert_eq!(
            slugs[0],
            format!("t5:{}", LoadoutPlacedGroup::Assault.slug())
        );
        let mut sorted = slugs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
        // Lethal has no categories, so one folder per namespace.
        let lethal = cat.groupings_of(LoadoutEditorLine::Lethal);
        assert_eq!(lethal.len(), 1);
        assert_eq!(lethal[0].category, None);
        assert_eq!(lethal[0].slug(), "t6");
        assert!(cat.groupings_of(LoadoutEditorLine::Perk1).is_empty());
    }

    #[test]
    fn keys_in_a_folder_match_namespace_and_category() {
        let cat = sample_catalog();
        let all_t6 = ClassPickerFolder {
            namespace: AssetCategoryScope::T6,
            category: None,
        };
        assert_eq!(cat.keys_in_category(LoadoutEditorLine::Primary, all_t6).len(), 3);
        let assault = ClassPickerFolder {
            namespace: AssetCategoryScope::T6,
            category: Some(LoadoutPlacedGroup::Assault),
        };
        assert_eq!(
            cat.keys_in_category(LoadoutEditorLine::Primary, assault),
            ["t6:weapon/an94_mp"]
        );
        assert!(
            cat.keys_in_category(LoadoutEditorLine::Secondary, all_t6)
                .is_empty()
        );
    }

    #[test]
    fn attachments_check_needs_a_registry() {
        let cat = LoadoutRoster::default();
        let err = cat
            .check_attachments("t6:weapon/an94_mp", &[], LoadoutRules::default())
            .unwrap_err();
        assert!(matches!(err, ConfigurationRefusal::AbsentResource(text) if text == NOT_READY));
    }

    #[test]
    fn resolver_compares_by_identity() {
        assert_eq!(CatalogResolver::default(), CatalogResolver(None));
        assert_eq!(
            format!("{:?}", CatalogResolver::default()),
            "CatalogResolver(none)"
        );
        assert_eq!(attachment_preview_key("w", "a"), "w+a");
    }

    #[test]
    fn slot_rows_read_and_write() {
        let mut slot = sample_slot();
        let seen: Vec<&str> = LoadoutEditorLine::ALL
            .iter()
            .map(|r| slot.value_in(*r))
            .collect();
        assert_eq!(seen, ["p", "s", "l", "t", "x1", "x2", "x3"]);
        slot.put_in(LoadoutEditorLine::Primary, "p".into());
        assert_eq!(slot.primary_attachments, ["a", "b"]);
        slot.put_in(LoadoutEditorLine::Primary, "p2".into());
        assert_eq!(slot.primary, "p2");
        assert!(slot.primary_attachments.is_empty());
        assert_eq!(slot.secondary_attachments, ["c"]);
        slot.put_in(LoadoutEditorLine::Secondary, "s2".into());
        assert!(slot.secondary_attachments.is_empty());
        slot.put_in(LoadoutEditorLine::Perk2, "y".into());
        slot.put_in(LoadoutEditorLine::Tactical, "z".into());
        assert_eq!((slot.second_specialty.as_str(), slot.tactical.as_str()), ("y", "z"));
        assert_eq!(slot.first_specialty, "x1");
    }

    #[test]
    fn presets_turn_empty_perks_into_none() {
        let preset = crate::KitTemplate {
            name: "Pick",
            primary: "m4_mp",
            main_gun_parts: &["acog"],
            secondary: "usp_mp",
            side_gun_parts: Default::default(),
            lethal: "frag_grenade_mp",
            tactical: "flash_grenade_mp",
            perks: ["", "fast", ""],
        };
        let slot = LoadoutSlotStatus::from_preset(&preset);
        assert_eq!(slot.name, "Pick");
        assert_eq!(slot.primary_attachments, ["acog"]);
        assert_eq!(slot.first_specialty, NO_PERK);
        assert_eq!(slot.second_specialty, "fast");
        assert_eq!(slot.third_specialty, NO_PERK);
        assert_eq!(slot.frozen_because, None);
        let host = frame::ServerKitSlot::from(&preset);
        assert_eq!(LoadoutSlotStatus::from_host_slot(&host).primary, "m4_mp");
    }

    #[test]
    fn roster_and_icon_stems() {
        assert_eq!(cac_roster(LoadoutEditorLine::Perk1).count(), 0);
        assert!(cac_roster(LoadoutEditorLine::Primary).any(|w| w == "m4_mp"));
        assert_eq!(cac_roster(LoadoutEditorLine::Lethal).count(), 5);
        assert_eq!(cac_roster(LoadoutEditorLine::Tactical).count(), 4);
        let stems = chooser_thumb_stems();
        assert!(!stems.is_empty());
        assert!(stems.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
