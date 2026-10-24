//! A small weapon catalog, its words and ten classes, for the class menu's
//! tests. The catalog knows no weapon registry entries, so a pick that has to
//! resolve a weapon is refused and a class never resolves.

use std::sync::Arc;

use asset_game::{CacStatBar, CacWeaponPreview, LocalizeCatalog, WeaponRegistry};
use bevy::prelude::*;
use frame::{CacWeaponOffer, UiMenuDvars};
use ui::classes::setup::CatalogResolver;
use ui::{ClassLoadoutCatalog, ClassSlotState, SessionClassStore};

use crate::feature_dispatch::testing::console_app;

pub(crate) const AK47: &str = "t6:weapon/ak47_mp";
pub(crate) const MP7: &str = "t6:weapon/mp7_mp";
pub(crate) const M16: &str = "t5:weapon/m16_mp";
pub(crate) const FIVESEVEN: &str = "t6:weapon/fiveseven_mp";
pub(crate) const FRAG: &str = "t6:weapon/frag_grenade_mp";
pub(crate) const SEMTEX: &str = "t6:weapon/sticky_grenade_mp";
pub(crate) const FLASH: &str = "t6:weapon/flash_grenade_mp";

fn gun(key: &str, group: &str, attachments: &[&str]) -> CacWeaponOffer {
    CacWeaponOffer {
        key: key.into(),
        item_group: Some(group.into()),
        attachments: attachments.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn preview(
    reference: &str,
    name: &str,
    image: &str,
    bars: &[(CacStatBar, i32)],
) -> CacWeaponPreview {
    CacWeaponPreview {
        reference: reference.into(),
        name_key: format!("@{name}"),
        image: image.into(),
        desc_key: format!("@{name}_DESC"),
        bars: bars.to_vec(),
    }
}

/// Primary: ak47 (assault, takes reflex and grip), mp7 (smg, reflex), and
/// the older game's m16 (assault, none). Secondary: a fiveseven. Two
/// lethals, one tactical. Perks are the stock roster.
pub(crate) fn catalog() -> ClassLoadoutCatalog {
    let mut catalog = ClassLoadoutCatalog {
        primary: vec![
            gun(AK47, "weapon_assault", &["reflex", "grip"]),
            gun(MP7, "weapon_smg", &["reflex"]),
            gun(M16, "weapon_assault", &[]),
        ],
        secondary: vec![gun(FIVESEVEN, "weapon_pistol", &["extclip"])],
        lethal: vec![FRAG.into(), SEMTEX.into()],
        tactical: vec![FLASH.into()],
        resolver: CatalogResolver(Some(Arc::new(WeaponRegistry::default()))),
        ..ClassLoadoutCatalog::default()
    };
    catalog.previews.extend([
        (
            AK47.to_owned(),
            preview(
                "ak47",
                "WEAPON_AK47",
                "ui_ak47",
                &[
                    (CacStatBar::Accuracy, 40),
                    (CacStatBar::Damage, 120),
                    (CacStatBar::Mobility, -5),
                ],
            ),
        ),
        (
            MP7.to_owned(),
            preview("mp7", "WEAPON_MP7", "t6:material/ui_mp7", &[]),
        ),
        (
            format!("{AK47}+reflex"),
            preview("reflex", "ATTACH_REFLEX", "ui_reflex", &[]),
        ),
    ]);
    catalog
}

pub(crate) fn words() -> LocalizeCatalog {
    LocalizeCatalog::from_rows([
        ("WEAPON_AK47", "AK-47"),
        ("WEAPON_AK47_DESC", "Reliable and strong."),
        ("WEAPON_MP7", "MP7"),
        ("ATTACH_REFLEX", "Reflex Sight"),
        ("ATTACH_REFLEX_DESC", "A red dot."),
        ("MENU_PRIMARY_CAPS", "PRIMARY"),
        ("MENU_ATTACHMENTS_CAPS", "ATTACHMENTS"),
        ("PERKS_MARATHON", "Marathon"),
    ])
}

/// A usable class: the ak47 and no attachments, a fiveseven, a frag, a flash
/// and a perk in each place.
pub(crate) fn slot(name: &str) -> ClassSlotState {
    ClassSlotState {
        name: name.into(),
        primary: AK47.into(),
        primary_attachments: Vec::new(),
        secondary: FIVESEVEN.into(),
        secondary_attachments: Vec::new(),
        lethal: FRAG.into(),
        tactical: FLASH.into(),
        perk1: "specialty_marathon".into(),
        perk2: "specialty_bulletdamage".into(),
        perk3: "specialty_extendedmelee".into(),
        deathstreak: "specialty_null".into(),
        lock_reason: None,
    }
}

/// Ten classes named "Class 1" to "Class 10", the first one selected.
pub(crate) fn store() -> SessionClassStore {
    SessionClassStore {
        slots: (1..=10).map(|n| slot(&format!("Class {n}"))).collect(),
        equipped: None,
        selected: 0,
    }
}

/// A console with the class commands routed, ten classes, the catalog above
/// and its words.
pub(crate) fn menu_app() -> App {
    let mut app = console_app();
    app.insert_resource(catalog())
        .insert_resource(words())
        .insert_resource(store())
        .init_resource::<UiMenuDvars>()
        .add_systems(Update, super::route);
    app
}
