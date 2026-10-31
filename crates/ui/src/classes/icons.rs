//! Picture lookup for the class screens: which image stem stands for a
//! weapon, attachment or perk, and the cache those images are loaded into.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures_lite::future};
use frame::showcase_classes;

use crate::classes::setup::chooser_thumb_stems;
use crate::lui_hud::PictureCache;

/// Folder the interface pictures are read from, once it is known.
#[derive(Resource, Clone, Debug, Default)]
pub struct MenuMediaAnchor(pub Option<PathBuf>);

#[derive(Resource, Default)]
pub struct LoadoutPickerThumbs {
    pics: PictureCache,
    /// Flipped in the same step the images land, so an open menu is rebuilt
    /// a single time.
    pub loaded_once: bool,
}

impl LoadoutPickerThumbs {
    pub fn thumb_for(&self, base: &str) -> Option<Handle<Image>> {
        self.pics.kept(base)
    }
}


/// Per-frame driver for the background decode: idle until the asset folder is
/// known, then one job is spawned; once it reports, its pictures go into the
/// cache and the system stays quiet from then on.
pub fn preload_kit_pictures(
    asset_root: Res<MenuMediaAnchor>,
    mut icons: ResMut<LoadoutPickerThumbs>,
    mut pics: ResMut<Assets<Image>>,
    mut decoding: Local<Option<Task<Vec<(String, Image)>>>>,
) {
    if icons.loaded_once {
        return;
    }

    match decoding.as_mut() {
        Some(job) => {
            let Some(finished) = future::block_on(future::poll_once(job)) else {
                return;
            };
            for (stem, image) in finished {
                icons.pics.keep(stem, &pics.add(image));
            }
            decoding.take();
            icons.loaded_once = true;
        }
        None if asset_root.0.is_some() => {
            *decoding =
                Some(AsyncComputeTaskPool::get().spawn(async { decode_class_select_icons() }));
        }
        None => {}
    }
}

/// Collects every stem the screens could ask for. There is no image source
/// for them yet, so nothing is decoded; the count is only logged.
fn decode_class_select_icons() -> Vec<(String, Image)> {
    let mut wanted: BTreeSet<&str> = chooser_thumb_stems().into_iter().collect();
    for preset in showcase_classes() {
        let loadout_names = [
            preset.primary,
            preset.secondary,
            preset.lethal,
            preset.tactical,
        ];
        let (first, second) = (preset.main_gun_parts, preset.side_gun_parts);
        let attachment_names = first.iter().chain(second);
        wanted.extend(loadout_names.into_iter().filter_map(loadout_gun_thumb));
        wanted.extend(attachment_names.filter_map(|name| loadout_addon_thumb(name)));
    }

    diag::info!(
        Ui,
        "class icons: {} asked for, none turned into pictures (no BO2 picture source yet)",
        wanted.len()
    );
    <Vec<(String, Image)>>::new()
}

/// Everything after the final '/', or the whole text when there is none.
fn last_segment(path: &str) -> &str {
    match path.rfind('/') {
        Some(at) => &path[at + 1..],
        None => path,
    }
}

/// Drops one trailing `_mp` when present.
fn without_mp_suffix(name: &str) -> &str {
    match name.strip_suffix("_mp") {
        Some(base) => base,
        None => name,
    }
}

/// Weapon name to picture stem.
const WEAPON_PICTURES: &[(&str, &str)] = &[
    ("ak47", "weapon_ak47"),
    ("m4", "weapon_m4carbine"),
    ("famas", "weapon_famas"),
    ("scar", "weapon_scar_h"),
    ("tar21", "weapon_tavor"),
    ("fal", "weapon_fnfal"),
    ("m16", "weapon_m16a4"),
    ("masada", "weapon_masada"),
    ("fn2000", "weapon_fn2000"),
    ("ump45", "weapon_ump45_iron"),
    ("mp5k", "weapon_mp5k"),
    ("uzi", "weapon_mini_uzi"),
    ("p90", "weapon_p90"),
    ("kriss", "weapon_kriss"),
    ("rpd", "weapon_rpd"),
    ("sa80", "weapon_sa80"),
    ("mg4", "weapon_mg4"),
    ("m240", "weapon_m240"),
    ("aug", "weapon_steyraug"),
    ("barrett", "weapon_barrett50cal"),
    ("cheytac", "weapon_cheytac_scope"),
    ("wa2000", "weapon_wa2000"),
    ("m21", "weapon_m14ebr"),
    ("ranger", "weapon_ranger"),
    ("model1887", "weapon_model1887"),
    ("striker", "weapon_striker"),
    ("aa12", "weapon_aa12"),
    ("m1014", "weapon_benelli_m4"),
    ("spas12", "weapon_spas12"),
    ("usp", "weapon_usp_45"),
    ("beretta", "weapon_m9beretta"),
    ("deserteagle", "weapon_desert_eagle"),
    ("coltanaconda", "weapon_colt_anaconda"),
    ("glock", "weapon_glock"),
    ("beretta393", "weapon_beretta393"),
    ("pp2000", "weapon_pp2000"),
    ("tmp", "weapon_tmp"),
    ("at4", "weapon_at4"),
    ("rpg", "weapon_rpg7"),
    ("stinger", "weapon_stinger"),
    ("javelin", "weapon_javelin"),
    ("riotshield", "weapon_riot_shield"),
    ("semtex", "cardicon_semtex"),
    ("frag_grenade", "weapon_fraggrenade"),
    ("throwingknife", "cardicon_throwing_knive"),
    ("claymore", "weapon_claymore"),
    ("c4", "weapon_c4"),
    ("flash_grenade", "weapon_flashbang"),
    ("smoke_grenade", "weapon_smokegrenade"),
    ("concussion_grenade", "weapon_concgrenade"),
    ("flare", "specialty_tactical_insert"),
];

/// Picture stem for a weapon given by name or path, if one is known. Path,
/// a trailing `_mp` and letter case do not matter.
pub fn loadout_gun_thumb(weapon_name: &str) -> Option<&'static str> {
    let key = without_mp_suffix(last_segment(weapon_name)).to_ascii_lowercase();
    WEAPON_PICTURES
        .iter()
        .find_map(|&(name, picture)| (name == key).then_some(picture))
}

/// Materials whose picture goes by a different name.
const RENAMED_MATERIALS: &[(&str, &str)] = &[
    ("specialty_onemanarmy", "specialty_one_man_army"),
    ("specialty_coldblooded", "specialty_cold_blooded"),
    ("specialty_dangerclose", "specialty_danger_close"),
    ("specialty_localjammer", "specialty_scrambler"),
    ("equipment_frag", "weapon_fraggrenade"),
    ("equipment_semtex", "cardicon_semtex"),
    ("equipment_c4", "weapon_c4"),
    ("equipment_claymore", "weapon_claymore"),
    ("equipment_throwing_knife", "cardicon_throwing_knive"),
    ("equipment_flare", "specialty_tactical_insert"),
    ("weapon_semtex", "cardicon_semtex"),
    ("weapon_cheytac", "weapon_cheytac_scope"),
    ("killiconmelee", "cardicon_throwing_knive"),
];

/// Picture stem for a material: a handful were filed under another name, all
/// the rest use the material name as it is.
pub fn loadout_surface_pack_stem(material: &str) -> &str {
    for &(name, picture) in RENAMED_MATERIALS {
        if name == material {
            return picture;
        }
    }
    material
}

/// Picture stem for an attachment. Compound names (`acog_mp`, `m4_acog`) are
/// resolved by the right-most word that is a known attachment.
pub fn loadout_addon_thumb(token: &str) -> Option<&'static str> {
    let lowered = without_mp_suffix(last_segment(token).trim()).to_ascii_lowercase();
    // No attachment word contains an underscore, so testing the words alone
    // also covers a plain single-word name.
    lowered.rsplit('_').find_map(attachment_material)
}

/// Attachment words (any one of each group) to picture stem.
const ATTACHMENT_PICTURES: &[(&[&str], &str)] = &[
    (&["tactical"], "weapon_attachment_tactical"),
    (&["akimbo"], "weapon_attachment_akimbo"),
    (&["heartbeat"], "weapon_attachment_heartbeat"),
    (&["shotgun"], "weapon_attachment_shotgun"),
    (&["gl", "m203", "gp25"], "weapon_attachment_m203"),
    (&["xmags", "mags"], "weapon_attachment_mags"),
    (&["fmj"], "weapon_attachment_fmj"),
    (&["grip"], "weapon_attachment_grip"),
    (&["silencer", "suppressor"], "weapon_attachment_suppressor"),
    (&["thermal"], "weapon_attachment_thermal"),
    (&["reflex"], "weapon_attachment_reflex"),
    (&["eotech"], "weapon_attachment_eotech"),
    (&["acog"], "weapon_attachment_acog"),
];

fn attachment_material(word: &str) -> Option<&'static str> {
    ATTACHMENT_PICTURES
        .iter()
        .find(|(words, _)| words.contains(&word))
        .map(|&(_, picture)| picture)
}

/// Display form of a weapon name: path and `_mp` removed, underscores turned
/// into spaces, letters upper-cased.
pub fn display_gun_title(weapon: &str) -> String {
    let base = without_mp_suffix(last_segment(weapon));
    base.chars()
        .map(|c| {
            if c == '_' {
                ' '
            } else {
                c.to_ascii_uppercase()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_lookup_ignores_path_suffix_and_case() {
        assert_eq!(loadout_gun_thumb("weapon/AK47_mp"), Some("weapon_ak47"));
        assert_eq!(loadout_gun_thumb("m4"), Some("weapon_m4carbine"));
        assert_eq!(
            loadout_gun_thumb("a/b/c/flare_mp"),
            Some("specialty_tactical_insert")
        );
        assert_eq!(loadout_gun_thumb("nothing_mp"), None);
        assert_eq!(loadout_gun_thumb(""), None);
        assert_eq!(loadout_gun_thumb("m4_mp_mp"), None);
        assert_eq!(loadout_gun_thumb("m4/"), None);
    }

    #[test]
    fn material_stem_is_renamed_or_unchanged() {
        assert_eq!(
            loadout_surface_pack_stem("specialty_localjammer"),
            "specialty_scrambler"
        );
        assert_eq!(
            loadout_surface_pack_stem("killiconmelee"),
            "cardicon_throwing_knive"
        );
        assert_eq!(loadout_surface_pack_stem("specialty_other"), "specialty_other");
        assert_eq!(loadout_surface_pack_stem(""), "");
    }

    #[test]
    fn attachment_matches_last_known_word() {
        assert_eq!(
            loadout_addon_thumb("acog_mp"),
            Some("weapon_attachment_acog")
        );
        assert_eq!(
            loadout_addon_thumb("x/m4_ACOG"),
            Some("weapon_attachment_acog")
        );
        assert_eq!(
            loadout_addon_thumb("  silencer_mp "),
            Some("weapon_attachment_suppressor")
        );
        assert_eq!(
            loadout_addon_thumb("gl_grip"),
            Some("weapon_attachment_grip")
        );
        assert_eq!(
            loadout_addon_thumb("grip_unknown"),
            Some("weapon_attachment_grip")
        );
        assert_eq!(loadout_addon_thumb("gp25"), Some("weapon_attachment_m203"));
        assert_eq!(
            loadout_addon_thumb("xmags"),
            Some("weapon_attachment_mags")
        );
        assert_eq!(loadout_addon_thumb("bogus_word"), None);
        assert_eq!(loadout_addon_thumb(""), None);
        assert_eq!(loadout_addon_thumb("_"), None);
    }

    #[test]
    fn pretty_name_strips_path_and_suffix() {
        assert_eq!(display_gun_title("weapon/deserteagle_mp"), "DESERTEAGLE");
        assert_eq!(display_gun_title("frag_grenade_mp"), "FRAG GRENADE");
        assert_eq!(display_gun_title("a_mp/b"), "B");
        assert_eq!(display_gun_title("_mp"), "");
    }

    #[test]
    fn cache_get_clones_stored_handle() {
        let mut cache = LoadoutPickerThumbs::default();
        assert!(cache.thumb_for("x").is_none());
        cache.pics.keep("x".into(), &Handle::default());
        assert!(cache.thumb_for("x").is_some());
        assert!(cache.thumb_for("y").is_none());
    }
}
