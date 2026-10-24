//! The common set: the products every match shares (the common zone's
//! weapons, models, effects and scripts plus the startup zones' materials),
//! prepared once and handed out by key.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::*;
use crate::lane::CommonCensus;
use asset_core::AssetNamespace;
use asset_game::{CapturedStringTable, FxModelCatalog, OwnedFxImpactTable, TracerCatalog};
use asset_world::{CapturedLightDef, FilmVision, FilmVisionParseError, MapXModelSceneCatalog};

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
            Some(path) => write!(f, "{}", path.display()),
            None => f.write_str("<no runtime common_mp>"),
        }
    }
}

/// Everything a match copies from the common set before it adds its map.
#[derive(Clone)]
pub(super) struct CommonProducts {
    pub(super) scripts: crate::ScriptSources,
    pub(super) material_seed: MaterialCatalog,
    pub(super) shared_surfaces: asset_model::SharedXModelSurfaces,
    pub(super) scene_models: MapXModelSceneCatalog,
    pub(super) light_defs: Vec<CapturedLightDef>,
    pub(super) pen_table: weapon_t6::penetration::PenetrationDepthTable,
    pub(super) pen_table_loaded: bool,
    pub(super) lochit_table: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub(super) tracers: TracerCatalog,
    pub(super) xmodel_walk: crate::PreparedXModelWalkCensus,
    pub(super) film_visions: std::collections::BTreeMap<String, Result<FilmVision, FilmVisionParseError>>,
    pub(super) weapons: WeaponBuild,
    pub(super) fpv_meshes: FpvMeshBuild,
    pub(super) world_weapons: WorldWeaponBuild,
    pub(super) projectile_meshes: ProjectileMeshBuild,
    pub(super) xanims: XAnimBuild,
    pub(super) fx: FxCatalog,
    pub(super) fx_models: FxModelCatalog,
    pub(super) impact_fx: Option<OwnedFxImpactTable>,
    pub(super) strings: LocalizeCatalog,
    pub(super) report: Vec<String>,
    pub(super) localize_report: Vec<String>,
}

pub(super) struct CommonSet {
    pub(super) id: u64,
    pub(super) key: CommonKey,
    pub(super) products: CommonProducts,
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
        let mut slot = current();
        if slot
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &flight))
        {
            *slot = None;
        }
        drop(slot);
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
                load_pool()
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

async fn prepare_common(key: CommonKey) -> Arc<CommonSet> {
    let started = Instant::now();
    let progress = LoadProgress::default();
    let pool = load_pool();
    let anchor = key.runtime.clone();

    // The three reads below do not depend on each other.
    let startup_task = {
        let anchor = anchor.clone();
        let progress = progress.clone();
        pool.spawn(async move { walk_startup_zones(anchor.as_ref(), &progress).await })
    };
    let common_zone_task = {
        let anchor = anchor.clone();
        let progress = progress.clone();
        pool.spawn(async move {
            let Some(path) = anchor else {
                progress.record_skipped_scoped(StageId::CommonAssets, "common_mp");
                return None;
            };
            let stage = progress.begin_scoped(StageId::CommonAssets, "common_mp", None);
            let opened = open_zone_shared(&path);
            finish_zone_open(stage, &opened);
            Some((path, opened))
        })
    };
    let strings_task = {
        let progress = progress.clone();
        pool.spawn(async move {
            let mut report = Vec::new();
            let strings = match &anchor {
                Some(path) => {
                    let stage = progress.begin(StageId::Localization, None);
                    let strings = read_strings_beside(path, &mut report, &stage);
                    stage.done();
                    strings
                }
                None => {
                    progress.record_skipped(StageId::Localization);
                    report.push("localize: no runtime common_mp, so every string is a gap".into());
                    LocalizeCatalog::default()
                }
            };
            (strings, report)
        })
    };

    let startup = startup_task.await;
    let census = match common_zone_task.await {
        Some((path, Ok(image))) => lane(image.game).load_common_mp(
            &path,
            &image,
            &progress,
            true,
            startup.materials,
        ),
        Some((_, Err(error))) => CommonCensus {
            material_population: startup.materials,
            report: vec![format!("common_mp: open zone: {error}")],
            ..Default::default()
        },
        None => CommonCensus {
            material_population: startup.materials,
            ..Default::default()
        },
    };

    // patch_mp loads after common_mp, so its scripts win.
    let mut scripts = census.scripts;
    scripts.overlay(startup.scripts);
    let mut light_defs = startup.light_defs;
    light_defs.extend(census.light_defs);
    let mut report = startup.report;
    report.extend(census.report);
    report.push(format!("common FPV meshes: {}", census.fpv.len()));

    let cac_tables: Vec<(AssetNamespace, CapturedStringTable)> = startup
        .cac_tables
        .into_iter()
        .chain(census.cac_tables)
        .map(|table| (AssetNamespace::T6, table))
        .collect();
    let mut weapons = census.weapons;
    weapons.set_family_tables(cac_tables);
    weapons.resolve_fpv_mesh_edges(&census.fpv);
    weapons.resolve_fpv_hands(&census.fpv, &BodyMeshCatalog::default());
    weapons.resolve_world_model_edges(&census.world_weapons);

    let (strings, localize_report) = strings_task.await;
    report.extend(progress.timing_report());
    let prepared_ms = started.elapsed().as_secs_f32() * 1000.0;
    diag::info!(World, "common set: {key} prepared in {prepared_ms:.0}ms");

    let mut material_seed = census.material_population;
    material_seed.mark_images_common_owned();
    Arc::new(CommonSet {
        id: NEXT_COMMON_ID.fetch_add(1, Ordering::Relaxed),
        key,
        products: CommonProducts {
            scripts,
            material_seed,
            shared_surfaces: census.shared_surfaces,
            scene_models: census.scene_models,
            light_defs,
            pen_table: census.pen_table,
            pen_table_loaded: census.pen_table_loaded,
            lochit_table: census.lochit_table,
            tracers: census.tracers,
            xmodel_walk: census.xmodel_walk,
            film_visions: census.film_visions,
            weapons,
            fpv_meshes: census.fpv,
            world_weapons: census.world_weapons,
            projectile_meshes: census.projectile_meshes,
            xanims: census.xanims,
            fx: census.fx,
            fx_models: census.fx_models,
            impact_fx: census.impact_fx,
            strings,
            report,
            localize_report,
        },
        prepared_ms,
        ready_at: Instant::now(),
    })
}
