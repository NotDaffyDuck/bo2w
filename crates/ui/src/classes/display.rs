//! Text and picture names the class screens show for catalog entries.

use asset_core::{AssetKey, AssetNamespace};

use crate::{ClassLoadoutCatalog, ClassPickerFolder, pretty_weapon_name};

/// Localized text for `key` (a leading `@` is ignored), or `fallback` when
/// the catalog has none.
pub fn localized(loc: &asset_game::LocalizeCatalog, key: &str, fallback: &str) -> String {
    let text = loc.text(key.trim_start_matches('@'));
    text.unwrap_or(fallback).to_owned()
}

pub fn category_label(folder: ClassPickerFolder, loc: &asset_game::LocalizeCatalog) -> String {
    let Some(category) = folder.category else {
        return localized(loc, "MENU_EQUIPMENT_CAPS", "Equipment");
    };
    localized(loc, category.loc_key().unwrap_or(""), category.menu_label())
}

/// What a picker entry is called: its catalog name, else the perk's name,
/// else a tidied version of the key itself (for `weapon+attachment` keys,
/// the attachment).
pub fn label(
    key: &str,
    catalog: &ClassLoadoutCatalog,
    loc: &asset_game::LocalizeCatalog,
) -> String {
    if key.is_empty() {
        return localized(loc, "MENU_NONE", "None");
    }
    let catalog_name = catalog
        .previews
        .get(key)
        .and_then(|preview| loc.text(preview.name_key.trim_start_matches('@')));
    if let Some(name) = catalog_name {
        return name.to_owned();
    }

    let perk_key = format!("PERKS_{}", key.trim_start_matches("specialty_").to_uppercase());
    if let Some(name) = loc.text(&perk_key) {
        return name.to_owned();
    }
    let tail = key.rsplit_once('+').map_or(key, |(_, attachment)| attachment);
    pretty_weapon_name(tail)
}

/// Asset key of the picture for `key`; empty when it has none.
pub fn preview_image(key: &str, catalog: &ClassLoadoutCatalog) -> String {
    let Some(preview) = catalog.previews.get(key) else {
        return String::new();
    };
    let image = &preview.image;
    if image.is_empty() || AssetKey::parse(image).is_ok() {
        return image.clone();
    }
    // A bare material name: file it under the namespace of the entry itself.
    let namespace = AssetKey::parse(key).map_or(AssetNamespace::T6, |parsed| parsed.namespace);
    format!("{}:material/{image}", namespace.as_str())
}
