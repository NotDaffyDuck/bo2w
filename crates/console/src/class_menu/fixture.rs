//! A small weapon catalog, its words and ten classes, for the class menu's
//! tests. The catalog knows no weapon registry entries, so a pick that has to
//! resolve a weapon is refused and a class never resolves.


use asset_game::{LoadoutStatMeter, LoadoutGunShowcase, TranslationStringIndex, WeaponLedger};
use bevy::app::App;
use frame::CustomClassGunChoice;
use frame::UiMenuDvars;
use ui::classes::setup::CatalogResolver;
use ui::{LoadoutRoster, LoadoutSlotStatus, MatchLoadoutVault};

use crate::tool_cmds::testing::console_app;
use std::sync::Arc;

pub(crate) const AK47: &str = "t6:weapon/ak47_mp";
pub(crate) const MP7: &str = "t6:weapon/mp7_mp";
pub(crate) const M16: &str = "t5:weapon/m16_mp";
pub(crate) const FIVESEVEN: &str = "t6:weapon/fiveseven_mp";
pub(crate) const SAMPLE_FRAG_ASSET: &str = "t6:weapon/frag_grenade_mp";
pub(crate) const SEMTEX: &str = "t6:weapon/sticky_grenade_mp";
pub(crate) const FLASH: &str = "t6:weapon/flash_grenade_mp";

/// Loot groups the sample guns sit in.
const ASSAULT: &str = "weapon_assault";
const SMG: &str = "weapon_smg";
const PISTOL: &str = "weapon_pistol";

/// What the sample ak47 can carry.
const AK_KIT: &[&str] = &["reflex", "grip"];

fn gun(asset: &str, bundle: &str, attachments: &[&str]) -> CustomClassGunChoice {
    CustomClassGunChoice {
        key: asset.to_owned(),
        loot_bundle: Some(bundle.into()),
        attachments: attachments.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn preview(
    reference: &str,
    name: &str,
    image: &str,
    bars: &[(LoadoutStatMeter, i32)],
) -> LoadoutGunShowcase {
    LoadoutGunShowcase {
        reference: reference.into(),
        title_token: "@".to_owned() + name,
        image: image.into(),
        description_lookup: format!("@{name}_DESC"),
        bars: bars.to_vec(),
    }
}

/// The perks the stats table would offer in each slot; the stock class
/// holds the last of each.
const PERK1: [&str; 2] = ["specialty_fastreload", "specialty_marathon"];
const PERK2: [&str; 2] = ["specialty_flakjacket", "specialty_bulletdamage"];
const PERK3: [&str; 2] = ["specialty_longersprint", "specialty_extendedmelee"];

/// Primary: ak47 (assault, takes reflex and grip), mp7 (smg, reflex), and
/// the older game's m16 (assault, none). Secondary: a fiveseven. Two
/// lethals, one tactical. Perks are two per slot after none.
pub(crate) fn catalog() -> LoadoutRoster {
    let mut catalog = LoadoutRoster {
        primary: vec![
            gun(AK47, ASSAULT, AK_KIT),
            gun(MP7, SMG, &["reflex"]),
            gun(M16, ASSAULT, &[]),
        ],
        secondary: vec![gun(FIVESEVEN, PISTOL, &["extclip"])],
        lethal: vec![SAMPLE_FRAG_ASSET.into(), SEMTEX.into()],
        tactical: vec![FLASH.into()],
        resolver: CatalogResolver(Some(Arc::new(WeaponLedger::default()))),
        perks: [PERK1, PERK2, PERK3].map(|column| {
            std::iter::once(ui::classes::setup::NO_PERK)
                .chain(column)
                .map(String::from)
                .collect::<Vec<String>>()
        }),
        ..LoadoutRoster::default()
    };
    catalog.previews.extend([
        (
            AK47.to_owned(),
            preview(
                "ak47",
                "WEAPON_AK47",
                "ui_ak47",
                &[
                    (LoadoutStatMeter::Accuracy, 40),
                    (LoadoutStatMeter::Damage, 120),
                    (LoadoutStatMeter::Mobility, -5),
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

pub(crate) fn words() -> TranslationStringIndex {
    TranslationStringIndex::from_rows([
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
pub(crate) fn slot(called: &str) -> LoadoutSlotStatus {
    LoadoutSlotStatus {
        name: called.into(),
        primary: AK47.into(),
        primary_attachments: Vec::new(),
        secondary: FIVESEVEN.into(),
        secondary_attachments: Vec::new(),
        lethal: SAMPLE_FRAG_ASSET.into(),
        tactical: FLASH.into(),
        first_specialty: "specialty_marathon".into(),
        second_specialty: "specialty_bulletdamage".into(),
        third_specialty: "specialty_extendedmelee".into(),
        frozen_because: None,
    }
}

/// Ten classes named "Class 1" to "Class 10", the first one selected.
pub(crate) fn store() -> MatchLoadoutVault {
    MatchLoadoutVault {
        slots: (1..=10).map(|n| slot(&format!("Class {n}"))).collect(),
        ..Default::default()
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
        .add_systems(bevy::app::Update, super::route_class_menu);
    app
}
