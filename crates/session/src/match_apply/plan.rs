//! Checking a loaded match before any of it is published.
//!
//! Everything that can say no happens here, while the world is still
//! untouched: the scripts compile, the game mode is one we run, the map has
//! something to draw. What comes out is a plan the install can carry out
//! without a refusal left in it.

use std::sync::Arc;

use asset_core::AssetNamespace;
use asset_game::LocalizeCatalog;
use asset_world::ClipCollision;
use assets::{
    MatchType10SoundHints, PreparedBodies, PreparedDestructibleDeath, PreparedFpvMeshes,
    PreparedMap, PreparedMatch, PreparedProjectileMeshes, PreparedWeapons, PreparedWorldWeapons,
    PreparedXAnims, PreparedXModelWalkCensus, ScriptSources, T6ScriptSet,
};
use frame::{HostMatchRules, LaunchIdentity};
use gamemode_t6::GameModeKind;
use render_frontend::prepare::scene::world::{WorldScene, world_scene_from_draw};
use render_fx::{PreparedFxCatalog, PreparedFxModels, PreparedImpactFx, PreparedTracers};

use super::config::{self, MATCH_CONFIG};
use super::refusal::{InstallRefusal, script_refusal};
use super::script_models::{ModelInstall, ModelSpawn, model_install, model_spawns};
use super::t6_install::{self, T6Inputs};
use crate::Bo2mpMatchSetup;

/// The scripts the match runs, ready to hand to the simulation.
pub(super) struct GameScripts {
    pub program: sim::script::Program,
    /// Black Ops II's own scripts, when the match runs them beside the stubs.
    pub t6: Option<sim::T6Install>,
    pub level: sim::script::LevelData,
    /// The functions the scripts start with.
    pub entries: Vec<String>,
    pub dvars: Vec<(String, String)>,
    pub gametype: &'static str,
}

/// The catalogs the match keeps, as the asset side loaded them.
pub(super) struct Catalogs {
    pub weapons: PreparedWeapons,
    pub fpv_meshes: PreparedFpvMeshes,
    pub bodies: PreparedBodies,
    pub world_weapons: PreparedWorldWeapons,
    pub projectile_meshes: PreparedProjectileMeshes,
    pub xmodel_walk: PreparedXModelWalkCensus,
    pub xanims: PreparedXAnims,
    pub death: PreparedDestructibleDeath,
}

/// Effects and tracers, and the sounds the effects name.
pub(super) struct Effects {
    pub catalog: PreparedFxCatalog,
    pub type10: MatchType10SoundHints,
    pub models: PreparedFxModels,
    pub impacts: PreparedImpactFx,
    pub tracers: PreparedTracers,
}

/// What bullets and bodies meet in this map.
pub(super) struct MapPhysics {
    pub clip: Option<Arc<ClipCollision>>,
    pub pen_table: weapon_t6::penetration::PenetrationDepthTable,
    pub pen_table_loaded: bool,
    pub hit_locations: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
}

/// A match that passed every check.
pub(super) struct MatchInstallPlan {
    pub kind: GameModeKind,
    pub scripts: GameScripts,
    pub scene: WorldScene,
    pub catalogs: Catalogs,
    pub effects: Effects,
    pub physics: MapPhysics,
    pub map: PreparedMap,
    pub strings: LocalizeCatalog,
    pub models: ModelInstall,
    pub model_spawns: Vec<ModelSpawn>,
    /// Shellshock definitions: the map's own, then Black Ops II's.
    pub shocks: Vec<(String, String)>,
}

/// What the check needs to know about the launch besides the match itself.
pub(super) struct Launch<'a> {
    pub zone: &'a str,
    pub mode_selection: Option<&'a sim::HostGameModeSelection>,
    pub rules: Option<&'a HostMatchRules>,
    pub identity: Option<&'a LaunchIdentity>,
    /// What the front end set up (game type, bots) when it started this.
    pub setup: Option<&'a Bo2mpMatchSetup>,
}

pub(super) fn log_world_report(report: &[String]) {
    if report.is_empty() {
        diag::info!(World, "world report: no rows");
        return;
    }
    diag::info!(World, "world report: {} rows", report.len());
    for row in report {
        diag::info!(World, "world report: {row}");
    }
}

fn log_destructible_death_assets(death: &PreparedDestructibleDeath) {
    for row in &death.0 {
        diag::info!(
            World,
            "destructible death clip {}: {}",
            row.clip_hint,
            row.clip.edge_kind()
        );
        diag::info!(
            World,
            "destructible death husk {}: {}",
            row.husk_hint,
            row.husk.edge_kind()
        );
    }
}

/// Only a Black Ops II map has objectives to bind.
fn namespace_refusal(namespace: Option<AssetNamespace>) -> Option<InstallRefusal> {
    namespace
        .is_some_and(|namespace| namespace != AssetNamespace::T6)
        .then(|| {
            InstallRefusal::new("faction table missing for objective bindings (not a BO2 map)")
        })
}

/// The map's own name, without the namespace in front of it (`t6:mp_x`).
fn map_name(zone: &str) -> &str {
    zone.rsplit(':').next().unwrap_or(zone)
}

/// The front end's choice, when it was made for this map.
fn setup_for<'a>(zone: &str, setup: Option<&'a Bo2mpMatchSetup>) -> Option<&'a Bo2mpMatchSetup> {
    setup.filter(|setup| map_name(zone) == setup.map)
}

/// Which game type the Black Ops II scripts run, and whether they run at all.
#[derive(Debug, PartialEq, Eq)]
struct T6Run {
    gametype: String,
    scripts: bool,
}

/// A multiplayer map runs its game type (the test override, else the front
/// end's, else team deathmatch) but its scripts only when asked for or when
/// the front end started the match; any other map always runs its scripts.
fn t6_run(
    zone: &str,
    scripts_asked: bool,
    gametype_override: Option<String>,
    setup: Option<&Bo2mpMatchSetup>,
) -> T6Run {
    let multiplayer = map_name(zone).starts_with("mp_");
    let gametype = if multiplayer {
        gametype_override
            .or_else(|| {
                setup
                    .map(|setup| setup.gametype.clone())
                    .filter(|gametype| !gametype.is_empty())
            })
            .unwrap_or_else(|| "tdm".to_owned())
    } else {
        "zstandard".to_owned()
    };
    T6Run {
        gametype,
        scripts: !multiplayer || scripts_asked || setup.is_some(),
    }
}

/// The script name of the game type, from the first of the mode's names whose
/// script exists; the mode's own token when none does.
fn pick_gametype(kind: GameModeKind, exists: impl Fn(&str) -> bool) -> &'static str {
    kind.script_tokens()
        .iter()
        .copied()
        .find(|token| exists(token))
        .unwrap_or(kind.token())
}

/// The scripts, as the script compiler reads them.
struct Sources(ScriptSources);

impl sim::script::SourceResolver for Sources {
    fn read(&self, module: &str) -> Result<String, String> {
        self.0
            .read(module)
            .map(|bytes| sim::script::decode_source(&bytes))
    }

    fn read_bytes(&self, module: &str) -> Result<Vec<u8>, String> {
        self.0.read(module)
    }
}

/// Black Ops II's own rows win over a same-named row from the host game.
fn with_t6_strings(strings: LocalizeCatalog, set: Option<&T6ScriptSet>) -> LocalizeCatalog {
    let Some(set) = set else {
        return strings;
    };
    let mut merged = LocalizeCatalog::from_rows(
        set.strings
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    merged.absorb(strings);
    merged
}

fn shocks_of(scripts: &ScriptSources) -> Vec<(String, String)> {
    let mut shocks: Vec<(String, String)> = scripts
        .shocks()
        .map(|(name, text)| (name.to_owned(), text.to_owned()))
        .collect();
    if let Some(t6) = scripts.t6.as_ref() {
        shocks.extend(t6.shocks.iter().cloned());
    }
    shocks
}

fn string_tables(
    scripts: &ScriptSources,
) -> std::collections::BTreeMap<String, sim::script::StringTable> {
    scripts
        .tables()
        .iter()
        .map(|(name, table)| {
            let table = sim::script::StringTable {
                columns: table.columns,
                rows: table.rows,
                cells: table.cells.clone(),
            };
            (name.clone(), table)
        })
        .collect()
}

/// What the map itself tells the scripts: its entities, its radiant keys and
/// its string tables.
fn level_data(scripts: &ScriptSources) -> sim::script::LevelData {
    let keys = match scripts.config("radiant/keys.txt") {
        Some(text) => sim::script::parse_radiant_keys(text).unwrap_or_else(|error| {
            diag::warn!(Sim, "gsc: radiant/keys.txt: {error}");
            Default::default()
        }),
        None => {
            diag::warn!(
                Sim,
                "gsc: radiant/keys.txt is not in the zones; map keys stay strings"
            );
            Default::default()
        }
    };
    sim::script::LevelData {
        entities: sim::script::parse_entity_string(scripts.entities().unwrap_or("")),
        keys,
        tables: string_tables(scripts),
    }
}

/// The match settings file: the zones' copy, else the one in the game's iwds.
fn match_config(scripts: &ScriptSources, identity: Option<&LaunchIdentity>) -> Option<String> {
    let text = scripts.config(MATCH_CONFIG).map(str::to_owned).or_else(|| {
        let identity = identity?;
        let bytes = asset_game::read_iwd_named(&identity.games_root, MATCH_CONFIG)?;
        String::from_utf8(bytes).ok()
    });
    if text.is_none() {
        diag::warn!(
            Sim,
            "gsc: {MATCH_CONFIG} is in neither the zones nor the iwds"
        );
    }
    text
}

/// The dvars the scripts start with: the settings file, the host's rules over
/// it, and then a test's own overrides (`name value;name value`).
fn assemble_dvars(
    config_text: Option<&str>,
    zone: &str,
    gametype: &str,
    rules: &[(String, String)],
    overrides: Option<&str>,
) -> Vec<(String, String)> {
    let mut dvars = config::script_dvars(config_text, zone, gametype, rules);
    if let Some(text) = overrides {
        dvars.extend(config::dvar_overrides(text));
    }
    dvars
}

/// The compiled scripts and everything they start from.
fn game_scripts(
    scripts: ScriptSources,
    kind: GameModeKind,
    catalogs: &Catalogs,
    launch: &Launch,
) -> Result<GameScripts, InstallRefusal> {
    let zone = launch.zone;
    let sources = Sources(scripts);
    let setup = setup_for(zone, launch.setup);
    let run = t6_run(
        zone,
        std::env::var("BO2MP_SCRIPTS").is_ok_and(|value| value == "1"),
        std::env::var("BO2MP_GAMETYPE").ok(),
        setup,
    );
    let t6 = sources.0.t6.as_ref().filter(|_| run.scripts).map(|set| {
        t6_install::build(
            set,
            &T6Inputs {
                zone,
                gametype: &run.gametype,
                bots: setup.map(|s| (s.bot_friends, s.bot_enemies, s.bot_difficulty)),
                fpv_meshes: &catalogs.fpv_meshes,
                xanims: &catalogs.xanims,
            },
        )
    });
    let gametype = pick_gametype(kind, |token| {
        sources
            .0
            .read(&format!("maps/mp/gametypes/{token}"))
            .is_ok()
    });
    let level = level_data(&sources.0);
    let startup = sim::script::T6Startup::new(&sources, gametype, zone);
    let roots: Vec<&str> = startup.roots.iter().map(String::as_str).collect();
    let program = sim::script::Program::load(&sources, &roots, &sim::script::Catalog::t6())
        .map_err(|fault| script_refusal(zone, gametype, "compile", &fault))?;
    let config_text = match_config(&sources.0, launch.identity);
    let rules = launch.rules.map_or(&[][..], |rules| rules.0.as_slice());
    if launch.rules.is_some() {
        diag::info!(
            Sim,
            "gsc: {} host rule dvars over {MATCH_CONFIG}",
            rules.len()
        );
    }
    let overrides = std::env::var("BO2MP_SCRIPT_DVARS").ok();
    let dvars = assemble_dvars(
        config_text.as_deref(),
        zone,
        gametype,
        rules,
        overrides.as_deref(),
    );
    Ok(GameScripts {
        program,
        t6,
        level,
        entries: startup.entries,
        dvars,
        gametype,
    })
}

fn effects(
    fx: asset_game::FxDefinitions,
    tracers: asset_game::TracerDefinitions,
    world: &mut assets::PreparedWorld,
) -> Effects {
    let catalog = PreparedFxCatalog(fx);
    let type10 = MatchType10SoundHints(
        catalog
            .0
            .unique_type10_sound_hints()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    Effects {
        catalog,
        type10,
        models: PreparedFxModels(std::mem::take(&mut world.fx_models)),
        impacts: PreparedImpactFx(std::mem::take(&mut world.impact_fx)),
        tracers: PreparedTracers(tracers),
    }
}

/// Turn a loaded match into a plan, or say why it can't run.
pub(super) fn preflight(
    prepared: PreparedMatch,
    launch: Launch,
) -> Result<MatchInstallPlan, InstallRefusal> {
    let zone = launch.zone;
    if let Some(refusal) = namespace_refusal(prepared.prepared_map.namespace) {
        return Err(refusal);
    }
    let PreparedMatch {
        scripts,
        mut world,
        fx,
        materials,
        clip,
        weapons,
        fpv_meshes,
        bodies,
        world_weapons,
        projectile_meshes,
        xanims,
        destructible_death,
        tracers,
        strings,
        prepared_map,
        pen_table,
        pen_table_loaded,
        lochit_table,
        xmodel_walk,
        ..
    } = prepared;
    let catalogs = Catalogs {
        weapons: PreparedWeapons(weapons),
        fpv_meshes: PreparedFpvMeshes(fpv_meshes),
        bodies: PreparedBodies(bodies),
        world_weapons: PreparedWorldWeapons(world_weapons),
        projectile_meshes: PreparedProjectileMeshes(projectile_meshes),
        xmodel_walk,
        xanims: PreparedXAnims(xanims),
        death: PreparedDestructibleDeath(destructible_death),
    };
    log_destructible_death_assets(&catalogs.death);
    let kind = config::match_kind(
        launch.mode_selection.map(|selection| selection.kind()),
        std::env::var("BO2MP_GAMETYPE").ok().as_deref(),
    )
    .map_err(|gap| {
        diag::info!(Sim, "match install refused: {gap}");
        InstallRefusal::with_gap(gap, gap)
    })?;
    let shocks = shocks_of(&scripts);
    let strings = with_t6_strings(strings, scripts.t6.as_ref());
    let game_scripts = game_scripts(scripts, kind, &catalogs, &launch)?;
    let models = model_install(&world);
    let model_spawns = model_spawns(&world.script_model_instances);
    let effects = effects(fx, tracers, &mut world);
    let mut scene = world_scene_from_draw(world, materials, &models.owners).map_err(|error| {
        let gap = format!("match install refused: {error}");
        diag::info!(World, "{gap}");
        InstallRefusal::new(gap)
    })?;
    scene.t6_sun_sample_size_near = asset_game::sun_sample_size_near(zone);
    if scene.batches.is_empty() {
        diag::error!(
            World,
            "map load failed: no drawable world; see world report in the game log"
        );
        return Err(InstallRefusal::new(
            "No drawable world. See world report in the game log.",
        ));
    }
    Ok(MatchInstallPlan {
        kind,
        scripts: game_scripts,
        scene,
        catalogs,
        effects,
        physics: MapPhysics {
            clip,
            pen_table,
            pen_table_loaded,
            hit_locations: lochit_table,
        },
        map: prepared_map,
        strings,
        models,
        model_spawns,
        shocks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(map: &str, gametype: &str) -> Bo2mpMatchSetup {
        Bo2mpMatchSetup {
            map: map.into(),
            gametype: gametype.into(),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_bo2_map_has_objectives_to_bind() {
        assert!(namespace_refusal(None).is_none());
        assert!(namespace_refusal(Some(AssetNamespace::T6)).is_none());
        let refusal = namespace_refusal(Some(AssetNamespace::T5)).unwrap();
        assert!(refusal.error.contains("not a BO2 map"));
    }

    #[test]
    fn a_zone_is_named_by_what_follows_its_namespace() {
        assert_eq!(map_name("t6:mp_nuketown_2020"), "mp_nuketown_2020");
        assert_eq!(map_name("mp_raid"), "mp_raid");
    }

    #[test]
    fn the_front_end_choice_counts_only_for_its_own_map() {
        let chosen = setup("mp_raid", "dom");
        assert!(setup_for("t6:mp_raid", Some(&chosen)).is_some());
        assert!(setup_for("t6:mp_slums", Some(&chosen)).is_none());
        assert!(setup_for("t6:mp_raid", None).is_none());
    }

    #[test]
    fn a_multiplayer_map_defaults_to_team_deathmatch_without_scripts() {
        let run = t6_run("t6:mp_raid", false, None, None);
        assert_eq!(
            run,
            T6Run {
                gametype: "tdm".into(),
                scripts: false
            }
        );
    }

    #[test]
    fn asking_for_scripts_turns_them_on() {
        assert!(t6_run("t6:mp_raid", true, None, None).scripts);
    }

    #[test]
    fn a_match_the_front_end_started_runs_its_scripts_and_game_type() {
        let chosen = setup("mp_raid", "dom");
        let run = t6_run("t6:mp_raid", false, None, Some(&chosen));
        assert_eq!(run.gametype, "dom");
        assert!(run.scripts);
    }

    #[test]
    fn a_blank_front_end_game_type_falls_back_to_team_deathmatch() {
        let chosen = setup("mp_raid", "");
        assert_eq!(
            t6_run("t6:mp_raid", false, None, Some(&chosen)).gametype,
            "tdm"
        );
    }

    #[test]
    fn a_test_override_beats_the_front_end_game_type() {
        let chosen = setup("mp_raid", "dom");
        let run = t6_run("t6:mp_raid", false, Some("ctf".into()), Some(&chosen));
        assert_eq!(run.gametype, "ctf");
    }

    #[test]
    fn any_other_map_runs_its_scripts_as_standard() {
        let run = t6_run("t6:zm_tomb", false, Some("ctf".into()), None);
        assert_eq!(
            run,
            T6Run {
                gametype: "zstandard".into(),
                scripts: true
            }
        );
    }

    #[test]
    fn the_first_script_that_exists_names_the_game_type() {
        let kind = GameModeKind::Demolition;
        let names = kind.script_tokens();
        assert!(names.len() > 1);
        let last = names[names.len() - 1];
        assert_eq!(pick_gametype(kind, |token| token == last), last);
        assert_eq!(pick_gametype(kind, |_| true), names[0]);
    }

    #[test]
    fn with_no_script_found_the_modes_own_token_is_used() {
        let kind = GameModeKind::TeamDeathmatch;
        assert_eq!(pick_gametype(kind, |_| false), kind.token());
    }

    #[test]
    fn test_overrides_come_after_the_hosts_rules() {
        let rules = [("scr_x".to_owned(), "1".to_owned())];
        let dvars = assemble_dvars(
            None,
            "t6:mp_raid",
            "tdm",
            &rules,
            Some("set scr_x 2;set scr_y 3"),
        );
        let find = |name: &str| {
            dvars
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(find("scr_x"), Some("2"));
        assert_eq!(find("scr_y"), Some("3"));
    }

    #[test]
    fn without_overrides_the_dvars_are_the_configs_alone() {
        assert_eq!(
            assemble_dvars(None, "t6:mp_raid", "tdm", &[], None),
            config::script_dvars(None, "t6:mp_raid", "tdm", &[])
        );
    }

    #[test]
    fn a_match_from_another_game_is_refused_before_anything_else() {
        let mut prepared = PreparedMatch::default();
        prepared.prepared_map.namespace = Some(AssetNamespace::T5);
        let launch = Launch {
            zone: "t5:mp_x",
            mode_selection: None,
            rules: None,
            identity: None,
            setup: None,
        };
        let refusal = preflight(prepared, launch).err().unwrap();
        assert!(refusal.error.contains("not a BO2 map"));
        assert_eq!(refusal.sim_gap, None);
    }

    #[test]
    fn a_match_with_no_scripts_is_refused() {
        let launch = Launch {
            zone: "t6:mp_x",
            mode_selection: None,
            rules: None,
            identity: None,
            setup: None,
        };
        assert!(preflight(PreparedMatch::default(), launch).is_err());
    }
}
