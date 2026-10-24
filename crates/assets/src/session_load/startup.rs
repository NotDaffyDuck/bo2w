//! The zones every match starts from: the three that carry the shared
//! materials, light definitions and scripts, and the localized string files
//! that sit beside the map.

use super::*;
use asset_game::CapturedStringTable;
use asset_world::CapturedLightDef;

/// What the startup zones hold, merged in the order they load.
#[derive(Default)]
pub(super) struct StartupZones {
    pub(super) materials: MaterialCatalog,
    pub(super) report: Vec<String>,
    /// The string tables of `code_post_gfx_mp`, which names the weapon files.
    pub(super) cac_tables: Vec<CapturedStringTable>,
    pub(super) light_defs: Vec<CapturedLightDef>,
    pub(super) scripts: crate::ScriptSources,
}

/// Later zones fill in over earlier ones, so `patch_mp` comes last.
const STARTUP_ZONES: [&str; 3] = ["code_post_gfx_mp", "localized_code_post_gfx_mp", "patch_mp"];

/// Opens the startup zones beside `map_path` all at once, then walks their
/// materials one after the other into a single catalog.
pub(super) async fn walk_startup_zones(
    map_path: Option<&PathBuf>,
    progress: &LoadProgress,
) -> StartupZones {
    let Some(map_path) = map_path else {
        return StartupZones::default();
    };
    let games = games_root_from_env().ok();

    let opening = STARTUP_ZONES.map(|zone| {
        let games = games.clone();
        let map_path = map_path.clone();
        let progress = progress.clone();
        load_pool().spawn(async move {
            let found = match &games {
                Some(root) => find_runtime_zone(root, &map_path, zone),
                None => find_zone_for_tree(&map_path, zone),
            }?;
            let stage = progress.begin_scoped(StageId::CommonAssets, zone, None);
            let opened = open_zone_shared(&found.path)
                .map(|image| (found.path, image))
                .map_err(|error| error.to_string());
            if let Ok((_, image)) = &opened {
                stage.set_bytes(image.bytes.len() as u64);
            }
            stage.finish_from(&opened);
            opened
        })
    });

    let mut startup = StartupZones::default();
    for (zone, task) in STARTUP_ZONES.into_iter().zip(opening) {
        let (path, image) = match task.await {
            Ok(opened) => opened,
            Err(gap) => {
                startup
                    .report
                    .push(format!("startup zone {zone} not read: {gap}"));
                continue;
            }
        };
        let population = lane(image.game).load_material_population(
            &path,
            &image,
            progress,
            std::mem::take(&mut startup.materials),
        );
        startup.report.extend(population.report);
        startup.report.push(format!(
            "startup zone {zone}: {} materials, {} images so far",
            population.materials.materials.len(),
            population.materials.images.len(),
        ));
        startup.materials = population.materials;
        startup.light_defs.extend(population.light_defs);
        startup.scripts.overlay(population.scripts);
        if zone == "code_post_gfx_mp" {
            startup.cac_tables = population.cac_tables;
        }
    }
    startup
}

/// Reads the localized string files that sit beside `zone_ff`.
pub(super) fn read_strings_beside(
    zone_ff: &Path,
    report: &mut Vec<String>,
    stage: &StageHandle,
) -> LocalizeCatalog {
    let mut catalog = LocalizeCatalog::default();
    let root = match games_root_from_env() {
        Ok(root) => root,
        Err(error) => {
            report.push(format!("localize gap: BO2MP_GAMES unset ({error})"));
            return catalog;
        }
    };
    let found: Vec<_> = MP_LOCALIZED_ZONES
        .iter()
        .filter_map(|name| find_runtime_zone(&root, zone_ff, name).ok())
        .collect();
    // Zones that are not there are not part of the volume, so the total is
    // only known once every name has been looked for.
    stage.set_total(found.len() as u64);
    for zone in found {
        match load_localize_catalog_in_lane(&zone.path) {
            Ok(part) if part.is_empty() => {}
            Ok(part) => {
                report.push(format!("localize: {} {} strings", zone.zone_name, part.len()));
                catalog.absorb(part);
            }
            Err(error) => report.push(format!(
                "localize gap: {} failed: {error}",
                zone.zone_name
            )),
        }
        // A zone that failed to open still counts as handled.
        stage.advance(1);
    }
    report.push(format!("localize: {} strings in all", catalog.len()));
    if catalog.is_empty() {
        report.push("localize gap: no localized strings found beside the map".into());
    }
    catalog
}

/// Closes a zone-open stage with the size of what it read. A zone open has no
/// units to count, but its size is what the rest of the load is built out of.
pub(super) fn finish_zone_open<E>(
    stage: StageHandle,
    opened: &Result<Arc<asset_transport::zone::ZoneImage>, E>,
) {
    if let Ok(image) = opened {
        stage.set_bytes(image.bytes.len() as u64);
    }
    stage.finish_from(opened);
}
