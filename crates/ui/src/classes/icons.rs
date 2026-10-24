//! Picture lookup for the class screens: which image stem stands for a
//! weapon, attachment or perk, and the cache those images are loaded into.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures_lite::future};
use frame::showcase_classes;

use crate::classes::setup::picker_icon_stems;

/// Folder the interface pictures are read from, once it is known.
#[derive(Resource, Clone, Debug, Default)]
pub struct UiAssetRoot(pub Option<PathBuf>);

#[derive(Resource, Default)]
pub struct ClassSelectIconCache {
    pub images: HashMap<String, Handle<Image>>,
    /// Set together with the images, so an open menu rebuilds exactly once.
    pub warmed: bool,
}

impl ClassSelectIconCache {
    pub fn get(&self, stem: &str) -> Option<Handle<Image>> {
        self.images.get(stem).cloned()
    }
}

type DecodedClassIcons = Vec<(String, Image)>;

/// Starts decoding the class pictures on a worker thread and, when it
/// finishes, moves them into the cache.
pub fn warm_class_select_icons(
    root: Res<UiAssetRoot>,
    mut cache: ResMut<ClassSelectIconCache>,
    mut images: ResMut<Assets<Image>>,
    mut decoding: Local<Option<Task<DecodedClassIcons>>>,
) {
    if cache.warmed {
        return;
    }
    if let Some(task) = decoding.as_mut() {
        if let Some(decoded) = future::block_on(future::poll_once(task)) {
            for (stem, image) in decoded {
                cache.images.insert(stem, images.add(image));
            }
            cache.warmed = true;
            *decoding = None;
        }
        return;
    }
    if root.0.is_some() {
        let task = AsyncComputeTaskPool::get().spawn(async { decode_class_select_icons() });
        *decoding = Some(task);
    }
}

/// Works out which stems the screens would want. No image source for them
/// exists yet, so nothing is decoded and the count is only reported.
fn decode_class_select_icons() -> DecodedClassIcons {
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    for preset in showcase_classes() {
        let weapons = [
            preset.primary,
            preset.secondary,
            preset.lethal,
            preset.tactical,
        ];
        wanted.extend(weapons.into_iter().filter_map(cac_weapon_image));
        wanted.extend(
            preset
                .primary_attachments
                .iter()
                .chain(preset.secondary_attachments)
                .filter_map(|attachment| cac_attachment_image(attachment)),
        );
        wanted.extend(
            preset
                .perks
                .iter()
                .chain([&preset.deathstreak])
                .map(|perk| cac_material_iwd_stem(perk)),
        );
    }
    wanted.extend(picker_icon_stems());

    diag::info!(
        Ui,
        "class icons: {} wanted, none decoded (no BO2 image source yet)",
        wanted.len()
    );
    Vec::new()
}

/// The part of a path after its last slash.
fn last_segment(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, tail)| tail)
}

fn without_mp_suffix(name: &str) -> &str {
    name.strip_suffix("_mp").unwrap_or(name)
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

/// Picture stem for a weapon given by name or path, if one is known.
pub fn cac_weapon_image(weapon_name: &str) -> Option<&'static str> {
    let stem = without_mp_suffix(last_segment(weapon_name)).to_ascii_lowercase();
    WEAPON_PICTURES
        .iter()
        .find(|(name, _)| *name == stem)
        .map(|&(_, picture)| picture)
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

/// The picture stem for a material, which is the material's own name unless
/// its picture was filed under another one.
pub fn cac_material_iwd_stem(material: &str) -> &str {
    RENAMED_MATERIALS
        .iter()
        .find(|(name, _)| *name == material)
        .map_or(material, |&(_, picture)| picture)
}

/// Picture stem for an attachment. A compound name such as `acog_mp` or
/// `m4_acog` is matched by its last recognised word.
pub fn cac_attachment_image(token: &str) -> Option<&'static str> {
    let token = last_segment(token).trim();
    let name = without_mp_suffix(token).to_ascii_lowercase();
    name.split('_')
        .rev()
        .find_map(attachment_material)
        .or_else(|| attachment_material(&name))
}

fn attachment_material(word: &str) -> Option<&'static str> {
    let stem = match word {
        "acog" => "weapon_attachment_acog",
        "eotech" => "weapon_attachment_eotech",
        "reflex" => "weapon_attachment_reflex",
        "thermal" => "weapon_attachment_thermal",
        "silencer" | "suppressor" => "weapon_attachment_suppressor",
        "grip" => "weapon_attachment_grip",
        "fmj" => "weapon_attachment_fmj",
        "xmags" | "mags" => "weapon_attachment_mags",
        "gl" | "m203" | "gp25" => "weapon_attachment_m203",
        "shotgun" => "weapon_attachment_shotgun",
        "heartbeat" => "weapon_attachment_heartbeat",
        "akimbo" => "weapon_attachment_akimbo",
        "tactical" => "weapon_attachment_tactical",
        _ => return None,
    };
    Some(stem)
}

/// Display form of a weapon name: no path, no `_mp`, spaces, upper case.
pub fn pretty_weapon_name(weapon: &str) -> String {
    without_mp_suffix(last_segment(weapon))
        .replace('_', " ")
        .to_ascii_uppercase()
}
