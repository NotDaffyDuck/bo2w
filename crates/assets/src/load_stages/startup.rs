//! The zones every match starts from: the three that carry the shared
//! materials, light definitions and scripts, and the localized string files
//! that sit beside the map.

use super::*;
use asset_game::SnapshotStringTable;
use asset_world::SnapshotLightDef;

/// What the startup zones hold, merged in the order they load.
#[derive(Default)]
pub(super) struct StartupZones {
    pub(super) mat_list: SurfaceShaderRoster,
    pub(super) load_log: Vec<String>,
    /// The string tables of `code_post_gfx_mp`, which names the weapon files.
    pub(super) cac_tables: Vec<SnapshotStringTable>,
    pub(super) light_defs: Vec<SnapshotLightDef>,
    pub(super) scripts: crate::ScriptSources,
}

/// The startup zone whose string tables name the weapon files.
const CAC_ZONE: &str = "code_post_gfx_mp";

/// Later zones fill in over earlier ones, so `patch_mp` comes last.
const STARTUP_ORDER: [&str; 3] = [CAC_ZONE, "localized_code_post_gfx_mp", "patch_mp"];

/// A startup zone found on disk and opened.
struct OpenedZone {
    path: PathBuf,
    image: Arc<asset_transport::zone::ZoneTextureData>,
}

/// Finds `zone` next to the map (through the games root when one is set)
/// and opens it under its own progress stage.
fn open_startup_zone(
    games: Option<&asset_transport::GameInstallDir>,
    level_file_path: &Path,
    zone: &'static str,
    progress: &LoadCompletion,
) -> Result<OpenedZone, String> {
    let located = games.map_or_else(
        || locate_tree_zone(level_file_path, zone),
        |root| locate_runtime_zone(root, level_file_path, zone),
    )?;
    let stage = progress.begin_scoped(StageId::CommonAssets, zone, None);
    let open_result = open_shared_zone(&located.path);
    finish_zone_open(stage, &open_result);
    match open_result {
        Ok(image) => Ok(OpenedZone { path: located.path, image }),
        Err(fault) => Err(fault.to_string()),
    }
}

impl StartupZones {
    /// Folds one opened zone's material walk in over what came before it.
    fn absorb_zone(&mut self, zone: &str, OpenedZone { path, image }: OpenedZone, progress: &LoadCompletion) {
        let so_far = std::mem::take(&mut self.mat_list);
        let walked = zone_io(image.game).read_material_census(&path, &image, progress, so_far);
        self.load_log.extend(walked.load_log);
        let (material_tally, image_count) =
            (walked.mat_list.materials.len(), walked.mat_list.images.len());
        self.load_log
            .push(format!("startup zone {zone}: {material_tally} materials, {image_count} images so far"));
        self.mat_list = walked.mat_list;
        self.light_defs.extend(walked.light_defs);
        self.scripts.overlay(walked.scripts);
        if zone == CAC_ZONE {
            self.cac_tables = walked.cac_tables;
        }
    }
}

/// Opens the startup zones beside `level_file_path` all at once, then walks their
/// materials one after the other into a single catalog.
pub(super) async fn walk_startup_zones(
    level_file_path: Option<&PathBuf>,
    progress: &LoadCompletion,
) -> StartupZones {
    let mut merged = StartupZones::default();
    let Some(anchor) = level_file_path else { return merged };
    let games = game_dir_from_environment().ok();

    let load_threads = loader_pool();
    let pending: Vec<_> = STARTUP_ORDER
        .iter()
        .map(|&zone| {
            let (games, anchor, load_progress) = (games.clone(), anchor.clone(), progress.clone());
            let task = load_threads.spawn(async move { open_startup_zone(games.as_ref(), &anchor, zone, &load_progress) });
            (zone, task)
        })
        .collect();

    for (zone, task) in pending {
        match task.await {
            Ok(opened) => merged.absorb_zone(zone, opened, progress),
            Err(gap) => merged.load_log.push(format!("startup zone {zone} not read: {gap}")),
        }
    }
    merged
}

/// Reads the localized string files that sit beside `zone_archive_file`.
pub(super) fn read_strings_beside(
    zone_archive_file: &Path,
    notes: &mut Vec<String>,
    stage_h: &StageHandle,
) -> TranslationStringIndex {
    let root = game_dir_from_environment()
        .map_err(|unset| notes.push(format!("localize gap: BO2MP_GAMES unset ({unset})")))
        .ok();
    let Some(root) = root else { return TranslationStringIndex::default() };

    let present: Vec<_> =
        VERSUS_LANGUAGE_ZONES.iter().flat_map(|name| locate_runtime_zone(&root, zone_archive_file, name)).collect();
    // Zones that are not there are not part of the volume, so the total is
    // only known once every name has been looked for.
    stage_h.set_total(present.len() as u64);
    let strings = present.iter().fold(TranslationStringIndex::default(), |mut merged, zone| {
        let name = &zone.zone_label;
        match load_translation_lane(&zone.path) {
            Err(why) => notes.push(format!("localize gap: {name} failed: {why}")),
            Ok(part) if !part.is_empty() => {
                notes.push(format!("localize: {name} {} strings", part.len()));
                merged.merge_in(part);
            }
            Ok(_) => {}
        }
        // A zone that failed to open still counts as handled.
        stage_h.advance(1);
        merged
    });

    let summary = format!("localize: {} strings in all", strings.len());
    notes.push(summary);
    if strings.is_empty() {
        notes.push("localize gap: no localized strings found beside the map".into());
    }
    strings
}

/// Closes a zone-open stage with the size of what it read. A zone open has no
/// units to count, but its size is what the rest of the load is built out of.
pub(super) fn finish_zone_open<E>(
    stage: StageHandle,
    opened: &Result<Arc<asset_transport::zone::ZoneTextureData>, E>,
) {
    if let Some(image) = opened.as_ref().ok() {
        stage.set_bytes(image.bytes.len() as u64);
    }
    stage.finish_from(opened);
}
