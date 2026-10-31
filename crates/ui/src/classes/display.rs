//! Text and picture names the class screens show for catalog entries.

use asset_core::{AssetIdentity, AssetCategoryScope};

use crate::{LoadoutRoster, ClassPickerFolder, display_gun_title};

/// Localized text for `key` (a leading `@` is ignored), or `fallback` when
/// the catalog has none.
pub fn localized(loc: &asset_game::TranslationStringIndex, key: &str, fallback: &str) -> String {
    let text = loc.text(key.trim_start_matches('@'));
    text.unwrap_or(fallback).to_owned()
}

pub fn category_label(folder: ClassPickerFolder, loc: &asset_game::TranslationStringIndex) -> String {
    let Some(category) = folder.category else {
        return localized(loc, "MENU_EQUIPMENT_CAPS", "Equipment");
    };
    localized(loc, category.text_lookup_token().unwrap_or(""), category.screen_caption())
}

/// What a picker entry is called: its catalog name, else the perk's name,
/// else a tidied version of the key itself (for `weapon+attachment` keys,
/// the attachment).
pub fn label(
    key: &str,
    catalog: &LoadoutRoster,
    loc: &asset_game::TranslationStringIndex,
) -> String {
    if key.is_empty() {
        return localized(loc, "MENU_NONE", "None");
    }
    let catalog_name = catalog
        .previews
        .get(key)
        .and_then(|preview| loc.text(preview.title_token.trim_start_matches('@')));
    if let Some(name) = catalog_name {
        return name.to_owned();
    }

    let perk_key = format!("PERKS_{}", key.trim_start_matches("specialty_").to_uppercase());
    if let Some(name) = loc.text(&perk_key) {
        return name.to_owned();
    }
    let tail = key.rsplit_once('+').map_or(key, |(_, attachment)| attachment);
    display_gun_title(tail)
}

/// Asset key of the picture for `key`; empty when it has none.
pub fn preview_image(key: &str, catalog: &LoadoutRoster) -> String {
    let Some(preview) = catalog.previews.get(key) else {
        return String::new();
    };
    let image = &preview.image;
    if image.is_empty() || AssetIdentity::parse(image).is_ok() {
        return image.clone();
    }
    // A bare material name: file it under the namespace of the entry itself.
    let namespace = AssetIdentity::parse(key).map_or(AssetCategoryScope::T6, |parsed| parsed.namespace);
    format!("{}:material/{image}", namespace.as_str())
}
