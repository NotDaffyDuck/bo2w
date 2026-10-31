//! The common set: the products every match shares (the common zone's
//! weapons, models, effects and scripts plus the startup zones' materials),
//! prepared once and handed out by key.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::*;
use crate::zone_io::SharedAssetTally;
use asset_core::AssetCategoryScope;
use asset_game::{SnapshotStringTable, EffectMeshRoster, SelfContainedImpactFxTable, TracerRoster};
use asset_world::{SnapshotLightDef, ScreenGradeLook, GradeVisionDecodeFault, LevelModelPlacementRoster};

static NEXT_COMMON_ID: AtomicU64 = AtomicU64::new(1);

/// Which runtime `common_mp` zone a set was prepared from; two matches that
/// name the same zone share one set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommonKey {
    runtime: Option<PathBuf>,
}

impl CommonKey {
    pub(super) fn for_match(runtime: Option<&Path>) -> Self {
        Self {
            runtime: runtime.map(Path::to_path_buf),
        }
    }
}

impl std::fmt::Display for CommonKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.runtime {
            Some(folder) => write!(f, "{}", folder.display()),
            None => f.write_str("<no runtime common_mp>"),
        }
    }
}

/// Everything a match copies from the common set before it adds its map.
#[derive(Clone)]
pub(super) struct CommonProducts {
    pub(super) scripts: crate::ScriptSources,
    pub(super) material_seed: SurfaceShaderRoster,
    pub(super) pooled_skins: asset_model::PooledMeshSkins,
    pub(super) stage_models: LevelModelPlacementRoster,
    pub(super) light_defs: Vec<SnapshotLightDef>,
    pub(super) pen_rows: weapon_t6::penetration::BulletPierceDepthChart,
    pub(super) wall_pierce_table_ready: bool,
    pub(super) lochit_table: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub(super) tracers: TracerRoster,
    pub(super) model_traversal: crate::ReadiedXModelWalkCensus,
    pub(super) film_visions: std::collections::BTreeMap<String, Result<ScreenGradeLook, GradeVisionDecodeFault>>,
    pub(super) weapons: WeaponBuild,
    pub(super) first_person_meshes: FpvMeshBuild,
    pub(super) level_placed_weapons: WorldWeaponBuild,
    pub(super) shell_visual_models: ProjectileMeshBuild,
    pub(super) animation_refs: XAnimBuild,
    pub(super) fx: EffectRoster,
    pub(super) effect_meshes: EffectMeshRoster,
    pub(super) impact_effect: Option<SelfContainedImpactFxTable>,
    pub(super) strings: TranslationStringIndex,
    pub(super) load_log: Vec<String>,
    pub(super) translation_summary: Vec<String>,
}

pub(super) struct CommonSet {
    pub(super) id: u64,
    pub(super) key: CommonKey,
    pub(super) stage_outputs: CommonProducts,
    pub(super) prepared_ms: f32,
    pub(super) ready_at: Instant,
}

/// One preparation of a set, in progress or finished.
struct CommonFlight {
    key: CommonKey,
    set: async_lock::OnceCell<Option<Arc<CommonSet>>>,
}

static CURRENT: Mutex<Option<Arc<CommonFlight>>> = Mutex::new(None);

fn current() -> std::sync::MutexGuard<'static, Option<Arc<CommonFlight>>> {
    CURRENT.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Ends a flight when its task is dropped without having stored a set, so
/// that nobody waits on it for ever.
struct FlightGuard(Option<Arc<CommonFlight>>);

impl Drop for FlightGuard {
    fn drop(&mut self) {
        let Some(flight) = self.0.take() else {
            return;
        };
        let mut held_slot = current();
        if held_slot
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &flight))
        {
            *held_slot = None;
        }
        drop(held_slot);
        let _ = flight.set.set_blocking(None);
    }
}

/// The finished set for `key`, if one has landed.
pub(super) fn landed_common(key: &CommonKey) -> Option<Arc<CommonSet>> {
    let slot = current();
    let flight = slot.as_ref().filter(|flight| flight.key == *key)?;
    flight.set.get().cloned().flatten()
}

/// The set for `key`: the one already held, the one being prepared, or a new
/// one. The second value says which.
pub(super) async fn ensure_common(key: CommonKey) -> (Arc<CommonSet>, &'static str) {
    let (flight, reach) = {
        let mut slot = current();
        match slot.as_ref() {
            Some(flight) if flight.key == key => {
                let reach = if flight.set.get().is_some() {
                    "reused"
                } else {
                    "joined in flight"
                };
                (Arc::clone(flight), reach)
            }
            _ => {
                let flight = Arc::new(CommonFlight {
                    key: key.clone(),
                    set: async_lock::OnceCell::new(),
                });
                if let Some(previous) = slot.replace(Arc::clone(&flight)) {
                    diag::info!(World, "common set: {} replaced by {key}", previous.key);
                }
                let running = Arc::clone(&flight);
                loader_pool()
                    .spawn(async move {
                        let mut guard = FlightGuard(Some(running));
                        let set = prepare_common(key).await;
                        if let Some(flight) = guard.0.take() {
                            let _ = flight.set.set(Some(set)).await;
                        }
                    })
                    .detach();
                (flight, "prepared")
            }
        }
    };
    diag::info!(World, "common set: {reach} for {}", flight.key);
    match flight.set.wait().await {
        Some(set) => (Arc::clone(set), reach),
        None => panic!("common set for {} was not prepared", flight.key),
    }
}

/// The runtime `common_mp` opened under its own stage, with any open error
/// already worded; `None` (and a skipped stage) when there is no such zone.
fn open_common_zone(
    anchor: Option<PathBuf>,
    tracker: &LoadCompletion,
) -> Option<(PathBuf, Result<Arc<asset_transport::zone::ZoneTextureData>, String>)> {
    const SCOPE: &str = "common_mp";
    let Some(zone_path) = anchor else {
        tracker.record_skipped_scoped(StageId::CommonAssets, SCOPE);
        return None;
    };
    let stage = tracker.begin_scoped(StageId::CommonAssets, SCOPE, None);
    let image = open_shared_zone(&zone_path);
    finish_zone_open(stage, &image);
    Some((zone_path, image.map_err(|fault| fault.to_string())))
}

/// The localized strings beside the runtime `common_mp`, and what reading
/// them had to say.
fn read_common_strings(anchor: Option<&Path>, tracker: &LoadCompletion) -> (TranslationStringIndex, Vec<String>) {
    let mut notes = Vec::new();
    let found = match anchor {
        None => {
            tracker.record_skipped(StageId::Localization);
            notes.push("localize: no runtime common_mp, so every string is a gap".to_owned());
            TranslationStringIndex::default()
        }
        Some(beside) => {
            let stage = tracker.begin(StageId::Localization, None);
            let read = read_strings_beside(beside, &mut notes, &stage);
            stage.done();
            read
        }
    };
    (found, notes)
}

async fn prepare_common(key: CommonKey) -> Arc<CommonSet> {
    let t_start = Instant::now();
    let tracker = LoadCompletion::default();

    // The three reads below do not depend on each other.
    let (anchor, track) = (key.runtime.clone(), tracker.clone());
    let startup_task = loader_pool().spawn(async move { walk_startup_zones(anchor.as_ref(), &track).await });
    let (anchor, track) = (key.runtime.clone(), tracker.clone());
    let common_zone_task = loader_pool().spawn(async move { open_common_zone(anchor, &track) });
    let (anchor, track) = (key.runtime.clone(), tracker.clone());
    let strings_task = loader_pool().spawn(async move { read_common_strings(anchor.as_deref(), &track) });

    let startup = startup_task.await;
    let census = match common_zone_task.await {
        Some((zone_path, Ok(image))) => {
            zone_io(image.game).read_shared_multiplayer(&zone_path, &image, &tracker, true, startup.mat_list)
        }
        unopened => {
            let open_error = unopened.and_then(|(_, opened)| opened.err());
            SharedAssetTally {
                load_log: open_error.map(|error| format!("common_mp: open zone: {error}")).into_iter().collect(),
                material_population: startup.mat_list,
                ..Default::default()
            }
        }
    };

    // patch_mp loads after common_mp, so its scripts win.
    let mut scripts = census.scripts;
    scripts.overlay(startup.scripts);
    let light_rows: Vec<SnapshotLightDef> = startup.light_defs.into_iter().chain(census.light_defs).collect();
    let mut load_log: Vec<String> = startup.load_log.into_iter().chain(census.load_log).collect();
    load_log.push(format!("common FPV meshes: {}", census.fpv.len()));

    let cac_tables: Vec<(AssetCategoryScope, SnapshotStringTable)> = startup
        .cac_tables
        .into_iter()
        .chain(census.cac_tables)
        .map(|table| (AssetCategoryScope::T6, table))
        .collect();
    let mut weapons = census.weapons;
    weapons.set_family_tables(cac_tables);
    weapons.link_fpv_meshes(&census.fpv);
    weapons.resolve_fpv_hands(&census.fpv, &CharacterMeshRoster::default());
    weapons.link_world_meshes(&census.level_placed_weapons);

    let (local_text, translation_summary) = strings_task.await;
    load_log.extend_from_slice(&tracker.duration_digest());
    let prepared_ms = t_start.elapsed().as_secs_f32() * 1000.0;
    diag::info!(World, "common set: {key} prepared in {prepared_ms:.0}ms");

    let mut mat_seed = census.material_population;
    mat_seed.mark_images_common_owned();
    Arc::new(CommonSet {
        id: NEXT_COMMON_ID.fetch_add(1, Ordering::Relaxed),
        key,
        stage_outputs: CommonProducts {
            scripts,
            material_seed: mat_seed,
            pooled_skins: census.pooled_skins,
            stage_models: census.stage_models,
            light_defs: light_rows,
            pen_rows: census.pen_table,
            wall_pierce_table_ready: census.wall_pierce_table_ready,
            lochit_table: census.lochit_table,
            tracers: census.tracers,
            model_traversal: census.model_traversal,
            film_visions: census.vision_sets,
            weapons,
            first_person_meshes: census.fpv,
            level_placed_weapons: census.level_placed_weapons,
            shell_visual_models: census.shell_visual_models,
            animation_refs: census.animation_refs,
            fx: census.fx,
            effect_meshes: census.effect_meshes,
            impact_effect: census.impact_effect,
            strings: local_text,
            load_log,
            translation_summary,
        },
        prepared_ms,
        ready_at: Instant::now(),
    })
}
