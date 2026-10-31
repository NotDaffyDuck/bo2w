//! Names and pictures for the maps the front end lists.

use crate::ScreenArenaRoster;

/// Menu text for a map key such as `t6:mp_nuketown`: no namespace, no `mp_`,
/// spaces for underscores, upper case.
pub fn map_label(map: &str) -> String {
    let name = map.split_once(':').map_or(map, |(_, name)| name);
    name.trim_start_matches("mp_").replace('_', " ").to_uppercase()
}

/// Asset key of a map's preview picture; empty for an empty name. A key with
/// no namespace is taken to be a `t6` map.
pub fn map_preview(map: &str) -> String {
    let (namespace, stem) = map.split_once(':').unwrap_or(("t6", map));
    match (namespace, stem) {
        (_, "") => String::new(),
        ("t5", stem) => {
            let short = stem.trim_start_matches("mp_");
            format!("t5:material/menu_mp_map_select_{short}_big")
        }
        _ => format!("{namespace}:material/preview_{stem}"),
    }
}

/// The map names of pack number `pack`; none if there is no such pack.
pub fn pack_maps(maps: &ScreenArenaRoster, pack: usize) -> &[String] {
    match maps.0.get(pack) {
        Some(pack) => &pack.maps,
        None => &[],
    }
}
