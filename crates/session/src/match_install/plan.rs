//! Checking a loaded match before any of it is published.
//!
//! Everything that can say no happens here, while the world is still
//! untouched: the scripts compile, the game mode is one we run, the map has
//! something to draw. What comes out is a plan the install can carry out
//! without a refusal left in it.

use std::sync::Arc;

use asset_core::AssetCategoryScope;
use asset_game::TranslationStringIndex;
use asset_world::CollisionClipData;
use frame::{HostMatchRules, StartupPersona};
use gamemode_t6::RulesetKind;
use render_frontend::prepare::scene::world::{LevelStage, level_stage_from_draw};

use super::config::{self, MATCH_CONFIG};
use super::refusal::{InstallRefusal, script_refusal};
use super::script_models::{ModelInstall, ModelSpawn, model_install, model_spawns};
use super::t6_install::{self, T6Inputs};
use crate::Bo2mpMatchSetup;
use assets::{StagedWeapons, ReadiedFpvMeshes, ReadiedCharacterBodies, ReadiedWorldWeapons, ReadiedProjectileMeshes, ReadiedXModelWalkCensus, StagedXAnims, ReadiedBreakableDeath, RoundTenthKindAudioClues, ReadiedMapData, ScriptSources, T6ScriptSet, ReadiedSession};
use render_fx::{ReadyFxBook, ReadyFxProps, ReadyHitFx, StagedStreaks};

/// The scripts the match runs, ready to hand to the simulation.
pub(super) struct GameScripts {
    pub program: sim::gsc_host::Program,
    /// Black Ops II's own scripts, when the match runs them beside the stubs.
    pub t6: Option<sim::T6Install>,
    pub level: sim::gsc_host::LevelData,
    /// The functions the scripts start with.
    pub entries: Vec<String>,
    pub dvars: Vec<(String, String)>,
    pub gametype: &'static str,
}

/// The catalogs the match keeps, as the asset side loaded them.
pub(super) struct Catalogs {
    pub weapons: StagedWeapons,
    pub first_person_meshes: ReadiedFpvMeshes,
    pub bodies: ReadiedCharacterBodies,
    pub level_placed_weapons: ReadiedWorldWeapons,
    pub shell_visual_models: ReadiedProjectileMeshes,
    pub model_traversal: ReadiedXModelWalkCensus,
    pub animation_refs: StagedXAnims,
    pub death: ReadiedBreakableDeath,
}

/// Effects and tracers, and the sounds the effects name.
pub(super) struct Effects {
    pub catalog: ReadyFxBook,
    pub type10: RoundTenthKindAudioClues,
    pub models: ReadyFxProps,
    pub impacts: ReadyHitFx,
    pub tracers: StagedStreaks,
}

/// What bullets and bodies meet in this map.
pub(super) struct MapPhysics {
    pub clip: Option<Arc<CollisionClipData>>,
    pub pen_table: weapon_t6::penetration::BulletPierceDepthChart,
    pub wall_pierce_table_ready: bool,
    pub hit_locations: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
}

/// A match that passed every check.
pub(super) struct MatchInstallPlan {
    pub kind: RulesetKind,
    pub scripts: GameScripts,
    pub scene: LevelStage,
    pub catalogs: Catalogs,
    pub effects: Effects,
    pub physics: MapPhysics,
    pub map: ReadiedMapData,
    pub strings: TranslationStringIndex,
    pub models: ModelInstall,
    pub model_spawns: Vec<ModelSpawn>,
    /// Shellshock definitions: the map's own, then Black Ops II's.
    pub shocks: Vec<(String, String)>,
}

/// What the check needs to know about the launch besides the match itself.
pub(super) struct Launch<'a> {
    pub zone: &'a str,
    pub mode_selection: Option<&'a sim::HostChosenMode>,
    pub rules: Option<&'a HostMatchRules>,
    pub identity: Option<&'a StartupPersona>,
    /// What the front end set up (game type, bots) when it started this.
    pub setup: Option<&'a Bo2mpMatchSetup>,
}

pub(super) fn log_world_report(rows: &[String]) {
    let count = rows.len();
    if count == 0 {
        diag::info!(World, "world report: no rows");
        return;
    }
    diag::info!(World, "world report: {count} rows");
    for said in rows {
        diag::info!(World, "world report: {said}");
    }
}

fn log_destructible_death_assets(assets_in: &ReadiedBreakableDeath) {
    for entry in &assets_in.0 {
        let (collision_clue, clip_edge) = (&entry.collision_clue, entry.clip.dependency_cause_of());
        diag::info!(World, "destructible death clip {collision_clue}: {clip_edge}");
        let (shell_corpse_clue, husk_edge) = (&entry.shell_corpse_clue, entry.husk.dependency_cause_of());
        diag::info!(World, "destructible death husk {shell_corpse_clue}: {husk_edge}");
    }
}

/// Only a Black Ops II map has objectives to bind.
fn namespace_refusal(namespace: Option<AssetCategoryScope>) -> Option<InstallRefusal> {
    namespace
        .is_some_and(|namespace| namespace != AssetCategoryScope::T6)
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

/// The game type token a match runs, the scripts and the ruleset alike: on a
/// multiplayer map the test variable's, else the front end's (`ui_gametype`,
/// for its own map); elsewhere the test variable's.
fn gametype_token(zone: &str, from_env: Option<String>, setup: Option<&Bo2mpMatchSetup>) -> Option<String> {
    if !map_name(zone).starts_with("mp_") {
        return from_env;
    }
    from_env.or_else(|| {
        setup_for(zone, setup)
            .map(|setup| setup.gametype.clone())
            .filter(|gametype| !gametype.is_empty())
    })
}

/// Which game type the Black Ops II scripts run, and whether they run at all.
#[derive(Debug, PartialEq, Eq)]
struct T6Run {
    gametype: String,
    /// bo2zm: the Zombies start location the menu picked ("" = the map's own).
    location: String,
    scripts: bool,
}

/// A multiplayer map runs its game type (the test override, else the front
/// end's, else team deathmatch) but its scripts only when asked for or when
/// the front end started the match; any other map always runs its scripts, in
/// the game type and start location the menu picked (`zm_start`), else the
/// map's own.
fn t6_run(
    zone: &str,
    scripts_asked: bool,
    gametype_override: Option<String>,
    setup: Option<&Bo2mpMatchSetup>,
    zm_start: Option<(String, String)>,
) -> T6Run {
    let multiplayer = map_name(zone).starts_with("mp_");
    let gametype = if multiplayer {
        gametype_token(zone, gametype_override, setup).unwrap_or_else(|| "tdm".to_owned())
    } else {
        zm_start.as_ref().map_or_else(
            || sim::zm_gametype(zone).to_owned(),
            |(_, mode)| mode.clone(),
        )
    };
    let location = zm_start.map(|(location, _)| location).unwrap_or_default();
    T6Run {
        gametype,
        location,
        scripts: !multiplayer || scripts_asked || setup.is_some(),
    }
}

/// The script name of the game type, from the first of the mode's names whose
/// script exists; the mode's own token when none does.
fn pick_gametype(kind: RulesetKind, exists: impl Fn(&str) -> bool) -> &'static str {
    kind.script_tokens()
        .iter()
        .copied()
        .find(|token| exists(token))
        .unwrap_or(kind.token())
}

/// The scripts, as the script compiler reads them.
struct Sources(ScriptSources);

impl sim::gsc_host::SourceResolver for Sources {
    fn read(&self, module: &str) -> Result<String, String> {
        self.0
            .read(module)
            .map(|bytes| sim::gsc_host::decode_source(&bytes))
    }

    fn read_bytes(&self, module: &str) -> Result<Vec<u8>, String> {
        self.0.read(module)
    }
}

/// Black Ops II's own rows win over a same-named row from the host game.
fn with_t6_strings(strings: TranslationStringIndex, set: Option<&T6ScriptSet>) -> TranslationStringIndex {
    let Some(set) = set else {
        return strings;
    };
    let mut merged = TranslationStringIndex::from_rows(
        set.strings
            .iter()
            .map(|(id, text)| (id.as_str(), text.as_str())),
    );
    merged.merge_in(strings);
    merged
}

fn shocks_of(scripts: &ScriptSources) -> Vec<(String, String)> {
    let mut shocks: Vec<(String, String)> = scripts
        .shocks()
        .map(|(label, body)| (label.to_owned(), body.to_owned()))
        .collect();
    if let Some(t6) = scripts.t6.as_ref() {
        shocks.extend(t6.shocks.iter().cloned());
    }
    shocks
}

fn string_tables(
    scripts: &ScriptSources,
) -> std::collections::BTreeMap<String, sim::gsc_host::StringTable> {
    scripts
        .tables()
        .iter()
        .map(|(name, table)| {
            let table = sim::gsc_host::StringTable {
                cols: table.columns,
                lines: table.rows,
                squares: table.cells.clone(),
            };
            (name.clone(), table)
        })
        .collect()
}

/// What the map itself tells the scripts: its entities, its radiant keys and
/// its string tables.
fn level_data(scripts: &ScriptSources) -> sim::gsc_host::LevelData {
    let keys = match scripts.config("radiant/keys.txt") {
        Some(text) => sim::gsc_host::parse_radiant_keys(text).unwrap_or_else(|error| {
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
    sim::gsc_host::LevelData {
        entities: sim::gsc_host::parse_entity_string(scripts.entities().unwrap_or("")),
        keys,
        tables: string_tables(scripts),
    }
}

/// The match settings file: the zones' copy, else the one in the game's iwds.
fn match_config(scripts: &ScriptSources, identity: Option<&StartupPersona>) -> Option<String> {
    let text = scripts.config(MATCH_CONFIG).map(str::to_owned).or_else(|| {
        let identity = identity?;
        let bytes = asset_game::read_sound_pack_member(&identity.titles_install_dir, MATCH_CONFIG)?;
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
    let from_file = config::script_dvars(config_text, zone, gametype, rules);
    let Some(text) = overrides else {
        return from_file;
    };
    let mut merged = from_file;
    merged.extend(config::dvar_overrides(text));
    merged
}

/// The compiled scripts and everything they start from.
fn game_scripts(
    scripts: ScriptSources,
    mode: RulesetKind,
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
        frame::bo2_zm_start(zone),
    );
    let t6 = sources.0.t6.as_ref().filter(|_| run.scripts).map(|set| {
        t6_install::build(
            set,
            &T6Inputs {
                zone,
                gametype: &run.gametype,
                location: &run.location,
                bots: setup.map(|s| (s.bot_friends, s.bot_enemies, s.bot_difficulty)),
                first_person_meshes: &catalogs.first_person_meshes,
                animation_refs: &catalogs.animation_refs,
            },
        )
    });
    let gametype = pick_gametype(mode, |token| {
        sources
            .0
            .read(&format!("maps/mp/gametypes/{token}"))
            .is_ok()
    });
    let level = level_data(&sources.0);
    let startup = sim::gsc_host::T6Startup::new(&sources, gametype, zone);
    let roots: Vec<&str> = startup.roots.iter().map(String::as_str).collect();
    let program = sim::gsc_host::Program::load(&sources, &roots, &sim::gsc_host::Catalog::t6())
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
    world: &mut assets::StagedLevel,
) -> Effects {
    let catalog = ReadyFxBook(fx);
    let hints: Vec<String> = catalog
        .0
        .distinct_tenth_kind_audio_clues()
        .into_iter()
        .map(str::to_owned)
        .collect();
    Effects {
        catalog,
        type10: RoundTenthKindAudioClues(hints),
        models: ReadyFxProps(std::mem::take(&mut world.effect_meshes)),
        impacts: ReadyHitFx(std::mem::take(&mut world.impact_effect)),
        tracers: StagedStreaks(tracers),
    }
}

/// Turn a loaded match into a plan, or say why it can't run.
pub(super) fn preflight(
    prepared: ReadiedSession,
    launch: Launch,
) -> Result<MatchInstallPlan, InstallRefusal> {
    let zone = launch.zone;
    let wrong_game = namespace_refusal(prepared.prepared_map.namespace);
    if let Some(denied) = wrong_game {
        return Err(denied);
    }
    let ReadiedSession {
        scripts,
        world: mut world_data,
        fx,
        materials,
        clip,
        weapons,
        first_person_meshes,
        bodies,
        level_placed_weapons: carried_weapons,
        shell_visual_models: projectiles,
        animation_refs,
        destructible_death,
        tracers,
        strings: localized,
        prepared_map: map_data,
        pen_table: penetration,
        wall_pierce_table_ready: penetration_loaded,
        lochit_table,
        model_traversal: walk_census,
        ..
    } = prepared;
    let catalogs = Catalogs {
        weapons: StagedWeapons(weapons),
        first_person_meshes: ReadiedFpvMeshes(first_person_meshes),
        bodies: ReadiedCharacterBodies(bodies),
        level_placed_weapons: ReadiedWorldWeapons(carried_weapons),
        shell_visual_models: ReadiedProjectileMeshes(projectiles),
        model_traversal: walk_census,
        animation_refs: StagedXAnims(animation_refs),
        death: ReadiedBreakableDeath(destructible_death),
    };
    log_destructible_death_assets(&catalogs.death);
    let requested = launch.mode_selection.map(|selection| selection.kind());
    let token = gametype_token(launch.zone, std::env::var("BO2MP_GAMETYPE").ok(), launch.setup);
    let kind = match config::match_kind(requested, token.as_deref()) {
        Ok(picked) => picked,
        Err(why) => {
            diag::info!(Sim, "match install refused: {why}");
            return Err(InstallRefusal::with_gap(why, why));
        }
    };
    let shocks = shocks_of(&scripts);
    let strings = with_t6_strings(localized, scripts.t6.as_ref());
    let compiled = game_scripts(scripts, kind, &catalogs, &launch)?;
    let models = model_install(&world_data);
    let spawns = model_spawns(&world_data.scripted_prop_placements);
    let effects = effects(fx, tracers, &mut world_data);
    let drawn = level_stage_from_draw(world_data, materials, &models.owners);
    let mut level = match drawn {
        Ok(built) => built,
        Err(fault) => {
            let why = format!("match install refused: {fault}");
            diag::info!(World, "{why}");
            return Err(InstallRefusal::new(why));
        }
    };
    level.t6_sun_sample_size_near = asset_game::sun_sample_size_near(zone);
    if level.batches.is_empty() {
        diag::error!(
            World,
            "map load failed: no drawable world; see world report in the game log"
        );
        return Err(InstallRefusal::new(
            "No drawable world. See world report in the game log.",
        ));
    }
    let physics = MapPhysics {
        clip,
        pen_table: penetration,
        wall_pierce_table_ready: penetration_loaded,
        hit_locations: lochit_table,
    };
    Ok(MatchInstallPlan {
        kind,
        scripts: compiled,
        scene: level,
        catalogs,
        effects,
        physics,
        map: map_data,
        strings,
        models,
        model_spawns: spawns,
        shocks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assets::ReadiedSession;

    fn setup(map_name: &str, mode: &str) -> Bo2mpMatchSetup {
        Bo2mpMatchSetup {
            map: map_name.into(),
            gametype: mode.into(),
            ..Default::default()
        }
    }

    #[test]
    fn only_a_bo2_map_has_objectives_to_bind() {
        assert!(namespace_refusal(None).is_none());
        assert!(namespace_refusal(Some(AssetCategoryScope::T6)).is_none());
        let refusal = namespace_refusal(Some(AssetCategoryScope::T5)).unwrap();
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
        let run = t6_run("t6:mp_raid", false, None, None, None);
        assert_eq!(
            run,
            T6Run {
                gametype: "tdm".into(),
                location: String::new(),
                scripts: false
            }
        );
    }

    #[test]
    fn asking_for_scripts_turns_them_on() {
        assert!(t6_run("t6:mp_raid", true, None, None, None).scripts);
    }

    #[test]
    fn a_match_the_front_end_started_runs_its_scripts_and_game_type() {
        let chosen = setup("mp_raid", "dom");
        let run = t6_run("t6:mp_raid", false, None, Some(&chosen), None);
        assert_eq!(run.gametype, "dom");
        assert!(run.scripts);
    }

    #[test]
    fn a_blank_front_end_game_type_falls_back_to_team_deathmatch() {
        let chosen = setup("mp_raid", "");
        assert_eq!(
            t6_run("t6:mp_raid", false, None, Some(&chosen), None).gametype,
            "tdm"
        );
    }

    #[test]
    fn a_test_override_beats_the_front_end_game_type() {
        let chosen = setup("mp_raid", "dom");
        let run = t6_run("t6:mp_raid", false, Some("ctf".into()), Some(&chosen), None);
        assert_eq!(run.gametype, "ctf");
    }

    #[test]
    fn kill_confirmed_reaches_the_scripts_untouched() {
        let run = t6_run("t6:mp_raid", true, Some("conf".into()), None, None);
        assert_eq!(run.gametype, "conf");
        let chosen = setup("mp_raid", "conf");
        let run = t6_run("t6:mp_raid", false, None, Some(&chosen), None);
        assert_eq!((run.gametype.as_str(), run.scripts), ("conf", true));
    }

    #[test]
    fn the_front_end_and_the_test_variable_pick_the_same_game_type_and_ruleset() {
        let chosen = setup("mp_raid", "conf");
        let from_menu = gametype_token("t6:mp_raid", None, Some(&chosen));
        let from_env = gametype_token("t6:mp_raid", Some("conf".into()), None);
        assert_eq!(from_menu.as_deref(), Some("conf"));
        assert_eq!(from_menu, from_env);
        for token in [&from_menu, &from_env] {
            assert_eq!(config::match_kind(None, token.as_deref()), Ok(RulesetKind::TeamDeathmatch));
        }
        assert_eq!(gametype_token("t6:mp_slums", None, Some(&chosen)), None);
        assert_eq!(gametype_token("t6:zm_tomb", None, Some(&setup("zm_tomb", "conf"))), None);
    }

    #[test]
    fn any_other_map_runs_its_scripts_as_standard() {
        let run = t6_run("t6:zm_tomb", false, Some("ctf".into()), None, None);
        assert_eq!(
            run,
            T6Run {
                gametype: sim::zm_gametype("t6:zm_tomb").into(),
                location: String::new(),
                scripts: true
            }
        );
    }

    #[test]
    fn the_menus_zombies_pick_sets_the_game_type_and_start_location() {
        let pick = Some(("transit".to_owned(), "zclassic".to_owned()));
        let run = t6_run("t6:zm_transit", false, None, None, pick);
        assert_eq!(
            (run.gametype.as_str(), run.location.as_str()),
            ("zclassic", "transit")
        );
    }

    #[test]
    fn the_first_script_that_exists_names_the_game_type() {
        let kind = RulesetKind::Demolition;
        let names = kind.script_tokens();
        assert!(names.len() > 1);
        let last = names[names.len() - 1];
        assert_eq!(pick_gametype(kind, |token| token == last), last);
        assert_eq!(pick_gametype(kind, |_| true), names[0]);
    }

    #[test]
    fn with_no_script_found_the_modes_own_token_is_used() {
        let kind = RulesetKind::TeamDeathmatch;
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
        let mut prepared = ReadiedSession::default();
        prepared.prepared_map.namespace = Some(AssetCategoryScope::T5);
        let launch = Launch {
            zone: "t5:mp_x",
            mode_selection: None,
            rules: None,
            identity: None,
            setup: None,
        };
        let refusal = preflight(prepared, launch).err().unwrap();
        assert!(refusal.error.contains("not a BO2 map"));
        assert_eq!(refusal.stepping_lag, None);
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
        assert!(preflight(ReadiedSession::default(), launch).is_err());
    }
}
