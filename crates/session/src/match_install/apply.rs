//! Carrying a loaded match into the running app.
//!
//! The loader hands over a [`ReadiedSessionSignal`]; this system decides
//! whether it is still wanted, checks it, builds the simulation and every
//! resource the match needs into one queue of commands, and applies that
//! queue only if the match is still the one the session wants.

use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::ecs::world::CommandQueue;
use bevy::prelude::*;
use net::{HostInputValve, HostLoadingPause, HostSimulationWorld, DirectoryLink};
use render_frontend::adapters::motion::debris_piece::{PropPhysicsShape, DebrisPhysLevel};
use render_frontend::prepare::scene::camera::GameplayCamera;

use super::bootstrap::{self, BetweenRoundsCamera, Start};
use super::clip;
use super::content_tables::{self, ContentInputs};
use super::plan::{self, Catalogs, Effects, Launch, MatchInstallPlan, log_world_report};
use super::refusal::{InstallRefusal, install_facts, script_refusal};
use super::script_models::{self, ModelInstall, ModelSpawn};
use crate::GameContentLedger;
use crate::startup::{StagedConsoleText, BootConsoleScript};
use assets::{ReadiedSessionSignal, LoadBackdrop, SessionLoadCancel, StagedWeapons, ReadiedLocalText};
use frame::{ServerKitRoster, StartupPersona, ProcessDuty, StartupReport, LevelPresent, OwnLevelLoadTicket, GameSessionStamp, KitPickerPassover, WorldProducts, LevelGeneration, GameSessionArmed};

/// Everything the install reads about the load that just finished: the
/// prepared payload plus the launch settings it was requested under.
#[derive(SystemParam)]
pub struct StagedGameInputs<'w, 's> {
    payload: ResMut<'w, ReadiedSessionSignal>,
    swapper: Res<'w, crate::SessionHandoverOrder>,
    link: Option<Res<'w, DirectoryLink>>,
    launch_intent: Option<Res<'w, net::DirectoryLaunchRequest>>,
    overlay: Option<ResMut<'w, LoadBackdrop>>,
    load: Option<Res<'w, assets::MapLoadProcess>>,
    abort_mark: Option<Res<'w, SessionLoadCancel>>,
    class_loadouts: Option<Res<'w, ServerKitRoster>>,
    launch_id: Option<Res<'w, StartupPersona>>,
    staging: Local<'s, Option<asset_transport::StageHandle>>,
}

/// Console input typed before the match existed is held back until the
/// install has finished, whether it worked or not.
#[derive(SystemParam)]
pub struct GameInstallConsoleAccess<'w> {
    held_startup: ResMut<'w, BootConsoleScript>,
    console_lines: ResMut<'w, StagedConsoleText>,
}

impl GameInstallConsoleAccess<'_> {
    fn flush_startup(&mut self) {
        self.console_lines.0.append(&mut self.held_startup.commands);
    }
}

#[derive(SystemParam)]
pub struct SessionSetupHostPart<'w> {
    runtime_role: Res<'w, ProcessDuty>,
    prediction: Option<Res<'w, net::ClientGuessState>>,
    input_gate: Res<'w, HostInputValve>,
}

#[derive(SystemParam)]
pub struct SessionSetupDisplayPart<'w> {
    report: Option<ResMut<'w, StartupReport>>,
    camera: Res<'w, GameplayCamera>,
}

#[derive(SystemParam)]
pub struct SessionSetupSeatPart<'w> {
    world_flag: ResMut<'w, LevelPresent>,
    /// What the front end set up (game type, bots) when it started this.
    setup: Option<Res<'w, crate::Bo2mpMatchSetup>>,
}

/// Which load this is.
struct Load {
    ask_id: u64,
    key: OwnLevelLoadTicket,
    zone: String,
}

/// The match the master connection says is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveMatch {
    in_play: bool,
    key: GameSessionStamp,
    link_gen: u64,
}

fn live_match(link: &DirectoryLink) -> LiveMatch {
    let status = link.state();
    LiveMatch {
        in_play: status.underway(),
        key: status.identity().game_session_key(),
        link_gen: link.incarnation(),
    }
}

/// A local load (one without a match key) needs no master. Any other load has
/// to still be the match the master reports as running.
fn still_the_match(key: &OwnLevelLoadTicket, live: Option<LiveMatch>) -> bool {
    if key.game_session_key.is_none() {
        return true;
    }
    match live {
        Some(now) => now.in_play && now.key == key.game_session_key && now.link_gen == key.incarnation,
        None => false,
    }
}

/// What stands between the loaded match and the install this frame.
#[derive(Clone, Copy, Debug, Default)]
struct Conditions {
    /// The session swap in progress still wants this load.
    swap_wants: bool,
    live: Option<LiveMatch>,
    /// A joining client waits for the host's match before it installs one.
    joining: bool,
    aborted: bool,
    /// The loading screen's own work is still running.
    overlay_busy: bool,
    /// The install's row on the loading screen is already open.
    stage_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intake {
    /// Nobody wants this load any more.
    Stale,
    WaitForJoin,
    Aborted,
    /// Open the install's row and give the overlay a frame to show it before
    /// the main thread stops answering.
    OpenStage,
    Go,
}

fn intake(request_id: u64, key: &OwnLevelLoadTicket, now: Conditions) -> Intake {
    let unwanted = !now.swap_wants
        || !still_the_match(key, now.live)
        || key.own_level_ticket_no != request_id;
    if unwanted {
        return Intake::Stale;
    }
    if now.joining && key.game_session_key.is_none() {
        return Intake::WaitForJoin;
    }
    if now.aborted {
        return Intake::Aborted;
    }
    if now.overlay_busy && !now.stage_open {
        return Intake::OpenStage;
    }
    Intake::Go
}

/// Listening hosts, dedicated servers and replays own the simulation; a client
/// only predicts one.
fn holds_authority(role: &ProcessDuty) -> bool {
    role.hosts_simulation() || *role == ProcessDuty::Replay
}

/// Who holds the simulation after the install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    /// This app runs it (and predicts the local player when it has one).
    Authority,
    /// A client keeps only what it predicts.
    Predicting,
}

fn owner_of(authority: bool, has_prediction: bool) -> Option<Owner> {
    match (authority, has_prediction) {
        (true, _) => Some(Owner::Authority),
        (false, true) => Some(Owner::Predicting),
        (false, false) => None,
    }
}

fn stage<T: Resource>(queue: &mut CommandQueue, resource: T) {
    queue.push(move |world: &mut World| {
        world.insert_resource(resource);
    });
}

/// The install failed: tell the loading screen and whoever waits on the load.
fn refuse(
    why: String,
    load: Load,
    stage: Option<asset_transport::StageHandle>,
    overlay: Option<&mut LoadBackdrop>,
    failed: &mut MessageWriter<frame::LevelOpenRejected>,
) {
    diag::warn!(World, "match install refused: {why}");
    if let Some(handle) = stage {
        handle.fail();
    }
    if let Some(screen) = overlay {
        screen.fail(why.clone());
    }
    failed.write(frame::LevelOpenRejected {
        ask_number: load.ask_id,
        decrypt_secret: load.key,
        zone: load.zone,
        error: why,
    });
}

/// What the install takes from the running app.
struct AppView<'a> {
    load: &'a Load,
    began_at: Instant,
    authority: bool,
    predicted: Option<&'a net::ClientGuessState>,
    loadouts: Option<&'a ServerKitRoster>,
    camera: GameplayCamera,
    gate: HostInputValve,
    level_digest: Vec<String>,
}

/// What the map declares that is left after its resources are staged.
struct MapFactsLeft {
    airstrike_height: Option<f32>,
    gun_facts: String,
}

fn stage_map_facts(queue: &mut CommandQueue, facts: assets::MapFacts) -> MapFactsLeft {
    let assets::MapFacts {
        airstrike_height,
        script_sound: map_sound,
        team_settings,
        t6_hud_icons,
        t6_hud_fonts,
        t6_ui,
        t6_screen_filters,
        t6_playeranim,
        t6_ragdoll,
        t6_anim_speeds,
        t6_gun_facts,
        ..
    } = facts;
    stage(queue, asset_audio::SessionMapScriptSound(map_sound));
    stage(queue, asset_game::SessionTeamSettings(team_settings));
    stage(queue, t6_hud_icons);
    stage(queue, t6_hud_fonts);
    stage(queue, t6_ui);
    stage(queue, t6_screen_filters);
    // Other players' third-person animations, and how a dead body goes limp.
    stage(
        queue,
        sim::body_anim_script::T6PlayerAnimsRes::from_text(t6_playeranim.as_ref(), &t6_anim_speeds),
    );
    stage(
        queue,
        sim::ragdoll_defs::T6RagdollRes::from_text(t6_ragdoll.as_deref()),
    );
    MapFactsLeft {
        airstrike_height,
        gun_facts: t6_gun_facts,
    }
}

fn stage_effects(queue: &mut CommandQueue, effects: Effects) {
    stage(queue, effects.catalog);
    stage(queue, effects.type10);
    stage(queue, effects.models);
    stage(queue, effects.impacts);
    stage(queue, effects.tracers);
}

/// Stage every catalog the draw side reads; the weapons stay with the caller,
/// who still needs them for the class and manifest checks.
fn stage_catalogs(queue: &mut CommandQueue, catalogs: Catalogs) -> StagedWeapons {
    let Catalogs {
        weapons,
        first_person_meshes,
        bodies,
        level_placed_weapons: held_models,
        shell_visual_models: shot_meshes,
        model_traversal: walk_tables,
        animation_refs,
        death,
    } = catalogs;
    stage(queue, first_person_meshes);
    stage(queue, bodies);
    stage(queue, held_models);
    stage(queue, shot_meshes);
    stage(queue, walk_tables);
    stage(queue, animation_refs);
    stage(queue, death);
    weapons
}

/// Give the simulation its scripts and start them. Returns the line the
/// script boundary log prints for this install.
fn start_scripts(
    game: &mut sim::SimWorld,
    scripts: plan::GameScripts,
    zone: &str,
) -> Result<String, InstallRefusal> {
    let plan::GameScripts {
        program,
        t6,
        level,
        entries,
        dvars,
        gametype,
    } = scripts;
    let facts = install_facts(zone, gametype, &program, entries.len());
    game.install_gsc_program(program, sim::gsc_host::NativeRegistry::default(), level)
        .map_err(|fault| script_refusal(zone, gametype, "install", &fault))?;
    for (name, value) in &dvars {
        game.set_gsc_dvar(name, value);
    }
    if let Some(t6) = t6 {
        match game.install_t6(t6) {
            Ok(()) => game.set_gsc_dvar("bo2zm_t6", "1"),
            Err(error) => diag::warn!(Sim, "bo2zm t6 scripts not installed: {error}"),
        }
    }
    for entry in entries {
        game.start_gsc(&entry, sim::gsc_host::Value::level(), vec![])
            .map_err(|fault| script_refusal(zone, gametype, "entry", &fault))?;
    }
    Ok(facts)
}

/// A Black Ops II map's windows take damage by BO2's rules.
fn install_glass(
    game: &mut sim::SimWorld,
    scene: &render_frontend::prepare::scene::world::LevelStage,
) {
    let Some(glass) = scene.t6_glass.as_ref().filter(|g| !g.server.is_empty()) else {
        return;
    };
    let panes = glass
        .server
        .iter()
        .enumerate()
        .map(|(ordinal, pane)| {
            let pane = sim::T6GlassPane {
                origin: pane.origin,
                absmin: pane.absmin,
                absmax: pane.absmax,
                hit_point_ceiling: pane.hit_point_ceiling as f32,
            };
            (ordinal as u32, pane)
        })
        .collect();
    game.world_objects_mut().install_t6_glass(panes);
}

/// Put the map's script-moved models into the world, and tell the scene which
/// entities they became.
fn spawn_script_models(
    game: &mut sim::SimWorld,
    scene: &mut render_frontend::prepare::scene::world::LevelStage,
    models: &mut ModelInstall,
    spawns: &[ModelSpawn],
) {
    script_models::spawn_movers(game, spawns);
    for mover in std::mem::take(&mut models.brush_movers) {
        game.spawn_brush_mover(mover.id, mover.collision_geometry, mover.at, mover.turn)
            .expect("no free dynamic entity slot for a brush model");
    }
    script_models::stamp_entity_numbers(scene, game);
    if !spawns.is_empty() {
        let movers = game.scripted_mover_total();
        let base = sim::first_spawnable_slot();
        diag::info!(
            World,
            "script_model spawn: {movers} ET_SCRIPTMOVER from {base}"
        );
    }
}

/// Fingerprint the content this match runs on, so peers can check theirs
/// against it.
fn stage_manifest(
    queue: &mut CommandQueue,
    sim: &sim::SimWorld,
    prepared_map: &assets::ReadiedMapData,
    guns: &asset_game::WeaponLedger,
    combat: &[weapon_t6::combat_facts::GunBattleProfile],
    gear: &[sim::GadgetLiveStats],
) -> Result<(), InstallRefusal> {
    let gameplay = sim.content_fingerprint();
    let ledger = GameContentLedger::build(prepared_map, guns, combat, gear, gameplay)
        .map_err(|error| InstallRefusal::new(format!("Invalid content manifest: {error:?}")))?;
    let components = sim.content_part_fingerprints();
    let (map_name, weapon_count, digest) = (&ledger.map, ledger.weapons.len(), ledger.digest);
    let c = components;
    let (c_map, c_models, c_weapons, c_classes) = (c.map, c.meshes, c.guns, c.kits);
    diag::info!(
        Sim,
        "session manifest: ruleset=bo2 map={map_name:?} weapons={weapon_count} \
         digest={digest:016x} gameplay={gameplay:016x} c_map={c_map:016x} \
         c_models={c_models:016x} c_weapons={c_weapons:016x} c_classes={c_classes:016x}"
    );
    let Some(summary) = ledger.session_summary(components) else {
        return Err(InstallRefusal::new("Required match descriptor is missing"));
    };
    let (d_map, d_weapons, d_classes) = (summary.map, summary.weapons, summary.classes);
    diag::info!(
        Sim,
        "match descriptor: map={d_map:016x} weapons={d_weapons:016x} classes={d_classes:016x}"
    );
    stage(queue, summary);
    stage(queue, ledger);
    Ok(())
}

fn armed_prediction(
    prediction: &net::ClientGuessState,
    sim: &sim::SimWorld,
) -> net::ClientGuessState {
    let mut state = net::LocalForecast::new(prediction.0.local());
    // Prediction adopts replicated state at admission; the prepared world's
    // script runtime belongs to authority and snapshots cannot restore it.
    state.prime_from_assets(sim);
    net::ClientGuessState(state)
}

/// Decide who holds the simulation and stage it there.
fn hand_over_simulation(
    queue: &mut CommandQueue,
    sim: sim::SimWorld,
    authority: bool,
    prediction: Option<&net::ClientGuessState>,
) -> Result<(), InstallRefusal> {
    match owner_of(authority, prediction.is_some()) {
        Some(Owner::Authority) => {
            if let Some(prediction) = prediction {
                stage(queue, armed_prediction(prediction, &sim));
            }
            stage(queue, HostSimulationWorld(sim));
        }
        Some(Owner::Predicting) => {
            if let Some(prediction) = prediction {
                queue.push(|world: &mut World| {
                    world.remove_resource::<HostSimulationWorld>();
                });
                stage(queue, armed_prediction(prediction, &sim));
            }
        }
        None => {
            return Err(InstallRefusal::new(
                "No simulation owner for this execution mode",
            ));
        }
    }
    Ok(())
}

/// Build the whole install into one queue of commands.
fn install_match(plan: MatchInstallPlan, app: AppView) -> Result<CommandQueue, InstallRefusal> {
    let AppView {
        load,
        began_at,
        authority,
        predicted,
        loadouts,
        mut camera,
        mut gate,
        level_digest,
    } = app;
    let MatchInstallPlan {
        kind: rules,
        scripts,
        mut scene,
        catalogs,
        effects,
        physics,
        mut map,
        strings,
        mut models,
        model_spawns: placed,
        shocks,
    } = plan;
    let mut queue = CommandQueue::default();
    let dm_spawns = map.spawns.len();
    let facts = stage_map_facts(&mut queue, std::mem::take(&mut map.facts));
    stage(&mut queue, ReadiedLocalText(strings));
    stage_effects(&mut queue, effects);

    let plan::MapPhysics {
        clip: clip_data,
        pen_table,
        wall_pierce_table_ready: pen_loaded,
        hit_locations,
    } = physics;
    let configured = content_tables::configure(ContentInputs {
        weapons: &catalogs.weapons.0,
        bodies: &catalogs.bodies.0,
        animation_refs: &catalogs.animation_refs,
        namespace: map.namespace.unwrap_or(asset_core::AssetCategoryScope::T6),
        gun_facts_dump: &facts.gun_facts,
        pen_table,
        wall_pierce_table_ready: pen_loaded,
        hit_locations,
        shocks: &shocks,
    });
    let content_tables::Configured {
        content,
        combat,
        equipment: gear,
        gun_facts,
        viewmodel,
    } = configured;
    stage(&mut queue, sim::gun_view_facts::T6GunFactsRes(gun_facts));
    stage(&mut queue, sim::gun_view_facts::T6VmTableRes(viewmodel));
    let (offers, excluded) = content_tables::class_offers(catalogs.weapons.0.weapon_families());
    let guns = stage_catalogs(&mut queue, catalogs);
    gate.local_cmds_enabled = false;
    stage(&mut queue, DebrisPhysLevel::default());
    stage(&mut queue, PropPhysicsShape(clip_data.clone()));

    let mut game = sim::SimWorld::new();
    clip::install_collision(&mut game, content, clip::clip_parts(clip_data)?, &mut models);
    let roster = bootstrap::roster(loadouts, &guns.0, &combat, &gear);
    let score_limit = std::env::var("BO2MP_SCORE_LIMIT").ok();
    let begun = bootstrap::start_match(
        &mut game,
        Start {
            kind: rules,
            spawns: &map.spawns,
            view: scene.scoreboard_eye.map(|cam| BetweenRoundsCamera {
                spot: cam.origin,
                turn: cam.turn,
            }),
            airstrike_height: facts.airstrike_height,
            score_limit: score_limit.as_deref(),
        },
        roster,
        &mut camera,
        &mut gate,
    )?;

    let script_facts = start_scripts(&mut game, scripts, &load.zone)?;
    let load_hold = HostLoadingPause(game.solid_brush_total() > 0);
    install_glass(&mut game, &scene);
    game.world_objects_mut()
        .set_map_round_epoch(load.key.game_session_key.game_generation);
    spawn_script_models(&mut game, &mut scene, &mut models, &placed);

    let has_batches = !scene.batches.is_empty();
    let class_handoff = KitPickerPassover {
        pending: has_batches,
        primary: offers.primary,
        secondary: offers.secondary,
        lethal: offers.lethal,
        tactical: offers.tactical,
        excluded,
        locked_because: begun.locked_because,
    };
    stage_manifest(&mut queue, &game, &map, &guns.0, &combat, &gear)?;
    stage(&mut queue, guns);
    diag::info!(
        World,
        "match preparation: {:.1}ms on the main thread (scene handoff, sim boot, catalogs)",
        began_at.elapsed().as_secs_f32() * 1000.0
    );
    hand_over_simulation(&mut queue, game, authority, predicted)?;

    camera.hold_free_camera = true;
    stage(&mut queue, WorldProducts::from_walk(scene.products_id));
    stage(&mut queue, scene);
    stage(&mut queue, camera);
    stage(&mut queue, gate);
    stage(&mut queue, load_hold);
    stage(&mut queue, class_handoff);
    stage(&mut queue, crate::RunningWorldLabel { decrypt_secret: load.key });
    stage(&mut queue, LevelPresent(true));
    stage(&mut queue, LevelGeneration::out_of_setup_dir(load.ask_id));

    let (ask_id, decrypt_secret, zone) = (load.ask_id, load.key, load.zone.clone());
    let stepping_lag = begun.gap;
    queue.push(move |world: &mut World| {
        if let Some(mut launch_report) = world.get_resource_mut::<StartupReport>() {
            launch_report.stepping_lag = stepping_lag;
            launch_report.level_digest = level_digest;
        }
        diag::script_boundary("installed", &script_facts);
        diag::info!(
            Sim,
            "match: {label} ({token}) — world installed ({dm_spawns} dm spawn points); \
             class select waits for admission",
            label = rules.shown_label(),
            token = rules.token(),
        );
        perf::match_ready_mark(&zone, 1);
        world.write_message(GameSessionArmed {
            ask_number: ask_id,
            decrypt_secret,
            zone,
            spawns_made: dm_spawns,
        });
    });
    Ok(queue)
}

/// Apply a built install, unless the session moved on while it was built.
fn commit(world: &mut World, ask_id: u64, key: OwnLevelLoadTicket, mut install: CommandQueue) {
    let wanted = world
        .resource::<crate::SessionHandoverOrder>()
        .takes_install(ask_id);
    let live = world.get_resource::<DirectoryLink>().map(live_match);
    if wanted && still_the_match(&key, live) {
        install.apply(world);
    }
}

/// Drops the loader's hand-over so nothing can install it a second time.
fn discard_ready(commands: &mut Commands) {
    commands.remove_resource::<ReadiedSessionSignal>();
}

/// Install the loaded match the frame it is ready and wanted.
#[allow(clippy::too_many_arguments)]
pub fn install_staged_game(
    mut commands: Commands,
    src: StagedGameInputs,
    mut console_out: GameInstallConsoleAccess,
    auth: SessionSetupHostPart,
    mut present: SessionSetupDisplayPart,
    mut occupied: SessionSetupSeatPart,
    mode_pick: Option<Res<sim::HostChosenMode>>,
    rules: Option<Res<frame::HostMatchRules>>,
    mut failed: MessageWriter<frame::LevelOpenRejected>,
) {
    let StagedGameInputs {
        payload: mut handed,
        swapper,
        link,
        launch_intent,
        overlay: mut loading_screen,
        load: load_process,
        abort_mark,
        class_loadouts,
        launch_id,
        staging: mut stage_slot,
    } = src;
    let live_load = load_process
        .as_deref()
        .filter(|process| !process.is_done());
    let (ask_id, key) = (handed.ticket, handed.decrypt_secret);
    let ruling = intake(
        ask_id,
        &key,
        Conditions {
            swap_wants: swapper.takes_install(ask_id),
            live: link.as_deref().map(live_match),
            joining: launch_intent.as_deref().is_some_and(|li| li.denotes_arrival()),
            aborted: abort_mark.as_deref().is_some_and(|mark| mark.0 == ask_id),
            overlay_busy: live_load.is_some(),
            stage_open: stage_slot.is_some(),
        },
    );
    match ruling {
        Intake::Stale => {
            diag::info!(
                World,
                "match install: discarded stale load {:?} \
                 ({})",
                key,
                handed.zone
            );
            if let Some(handle) = stage_slot.take() {
                handle.cancel();
            }
            discard_ready(&mut commands);
            return;
        }
        Intake::WaitForJoin => return,
        Intake::Aborted => {
            if let Some(handle) = stage_slot.take() {
                handle.cancel();
            }
            discard_ready(&mut commands);
            commands.remove_resource::<SessionLoadCancel>();
            diag::info!(
                World,
                "match install: aborted request #{ask_id} (`{}`) \
                 — not occupying the menu",
                handed.zone
            );
            return;
        }
        Intake::OpenStage => {
            if let Some(process) = live_load {
                let opened = process
                    .progress
                    .begin(asset_transport::StageId::Install, None);
                *stage_slot = Some(opened);
            }
            diag::info!(
                World,
                "match install: yield one frame so the overlay can name \
                 the main-thread hitch"
            );
            return;
        }
        Intake::Go => {}
    }

    let stage_handle = stage_slot.take();
    let clock = Instant::now();
    let load = Load {
        ask_id,
        key,
        zone: std::mem::take(&mut handed.zone),
    };
    let mut built_parts = std::mem::take(&mut handed.prepared);
    discard_ready(&mut commands);
    let level_digest = std::mem::take(&mut built_parts.report);
    log_world_report(&level_digest);

    let checked = plan::preflight(
        built_parts,
        Launch {
            zone: &load.zone,
            mode_selection: mode_pick.as_deref(),
            rules: rules.as_deref(),
            identity: launch_id.as_deref(),
            setup: occupied.setup.as_deref(),
        },
    );
    let plan = match checked {
        Err(blocked) => {
            if let Some(launch_report) = present.report.as_deref_mut() {
                if let Some(gap) = blocked.stepping_lag {
                    launch_report.stepping_lag = gap;
                }
                launch_report.level_digest = level_digest;
            }
            *occupied.world_flag = LevelPresent(false);
            refuse(
                blocked.error,
                load,
                stage_handle,
                loading_screen.as_deref_mut(),
                &mut failed,
            );
            console_out.flush_startup();
            return;
        }
        Ok(plan) => plan,
    };
    let built = install_match(
        plan,
        AppView {
            load: &load,
            began_at: clock,
            authority: holds_authority(&auth.runtime_role),
            predicted: auth.prediction.as_deref(),
            loadouts: class_loadouts.as_deref(),
            camera: *present.camera,
            gate: *auth.input_gate,
            level_digest,
        },
    );
    match built {
        Ok(install) => {
            commands.queue(move |world: &mut World| commit(world, ask_id, key, install));
            if let Some(handle) = stage_handle {
                handle.done();
            }
        }
        Err(blocked) => refuse(
            blocked.error,
            load,
            stage_handle,
            loading_screen.as_deref_mut(),
            &mut failed,
        ),
    }
    console_out.flush_startup();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use frame::{OwnLevelLoadTicket, GameSessionStamp, ProcessDuty, LevelPresent};

    use asset_world::CollisionClipData;

    use super::*;

    const REQUEST: u64 = 7;

    fn local() -> OwnLevelLoadTicket {
        OwnLevelLoadTicket::from_request(REQUEST, GameSessionStamp::NONE, 0)
    }

    fn networked(incarnation: u64) -> OwnLevelLoadTicket {
        OwnLevelLoadTicket::from_request(REQUEST, GameSessionStamp::new([1; 16], 3), incarnation)
    }

    fn running(incarnation: u64) -> Option<LiveMatch> {
        Some(LiveMatch {
            in_play: true,
            key: GameSessionStamp::new([1; 16], 3),
            link_gen: incarnation,
        })
    }

    fn wanted() -> Conditions {
        Conditions {
            swap_wants: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_local_load_needs_no_master() {
        assert!(still_the_match(&local(), None));
    }

    #[test]
    fn a_networked_load_needs_the_same_match_running() {
        let key = networked(5);
        assert!(still_the_match(&key, running(5)));
        assert!(!still_the_match(&key, None));
        assert!(!still_the_match(&key, running(6)));
        let mut over = running(5).unwrap();
        over.in_play = false;
        assert!(!still_the_match(&key, Some(over)));
        let mut other = running(5).unwrap();
        other.key = GameSessionStamp::new([2; 16], 3);
        assert!(!still_the_match(&key, Some(other)));
    }

    #[test]
    fn a_load_the_swap_no_longer_wants_is_stale() {
        let now = Conditions {
            swap_wants: false,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::Stale);
    }

    #[test]
    fn a_load_for_another_request_is_stale() {
        assert_eq!(intake(REQUEST + 1, &local(), wanted()), Intake::Stale);
    }

    #[test]
    fn a_networked_load_after_its_match_ended_is_stale() {
        assert_eq!(intake(REQUEST, &networked(5), wanted()), Intake::Stale);
        let now = Conditions {
            live: running(5),
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &networked(5), now), Intake::Go);
    }

    #[test]
    fn staleness_wins_over_an_abort() {
        let now = Conditions {
            swap_wants: false,
            aborted: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::Stale);
    }

    #[test]
    fn a_joining_client_waits_for_the_hosts_match() {
        let now = Conditions {
            joining: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::WaitForJoin);
        let now = Conditions {
            joining: true,
            live: running(5),
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &networked(5), now), Intake::Go);
    }

    #[test]
    fn an_aborted_load_is_dropped() {
        let now = Conditions {
            aborted: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::Aborted);
    }

    #[test]
    fn the_overlay_gets_a_frame_before_the_install_stalls() {
        let now = Conditions {
            overlay_busy: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::OpenStage);
        let now = Conditions {
            overlay_busy: true,
            stage_open: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::Go);
    }

    #[test]
    fn with_no_loading_screen_work_the_install_goes_at_once() {
        assert_eq!(intake(REQUEST, &local(), wanted()), Intake::Go);
    }

    #[test]
    fn hosts_and_replays_hold_the_simulation_but_clients_do_not() {
        assert!(holds_authority(&ProcessDuty::Listen));
        assert!(holds_authority(&ProcessDuty::Dedicated));
        assert!(holds_authority(&ProcessDuty::Replay));
        assert!(!holds_authority(&ProcessDuty::Client));
    }

    #[test]
    fn the_simulation_goes_to_the_authority_or_the_predicting_client() {
        assert_eq!(owner_of(true, true), Some(Owner::Authority));
        assert_eq!(owner_of(true, false), Some(Owner::Authority));
        assert_eq!(owner_of(false, true), Some(Owner::Predicting));
        assert_eq!(owner_of(false, false), None);
    }

    #[test]
    fn staged_resources_land_in_the_world_when_the_queue_applies() {
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        stage(&mut queue, LevelPresent(true));
        assert!(world.get_resource::<LevelPresent>().is_none());
        queue.apply(&mut world);
        assert!(world.resource::<LevelPresent>().0);
    }

    #[test]
    fn a_client_install_clears_the_authority_world_it_inherited() {
        let mut world = World::new();
        world.insert_resource(HostSimulationWorld(sim::SimWorld::new()));
        let mut queue = CommandQueue::default();
        queue.push(|world: &mut World| {
            world.remove_resource::<HostSimulationWorld>();
        });
        queue.apply(&mut world);
        assert!(world.get_resource::<HostSimulationWorld>().is_none());
    }

    #[test]
    fn nothing_is_installed_when_there_is_no_simulation_owner() {
        let mut queue = CommandQueue::default();
        let refusal = hand_over_simulation(&mut queue, sim::SimWorld::new(), false, None)
            .err()
            .unwrap();
        assert_eq!(refusal.error, "No simulation owner for this execution mode");
    }

    #[test]
    fn an_authority_without_prediction_stages_the_world_alone() {
        let mut queue = CommandQueue::default();
        hand_over_simulation(&mut queue, sim::SimWorld::new(), true, None).unwrap();
        let mut world = World::new();
        queue.apply(&mut world);
        assert!(world.get_resource::<HostSimulationWorld>().is_some());
        assert!(world.get_resource::<net::ClientGuessState>().is_none());
    }

    #[test]
    fn a_bare_clip_is_refused_before_the_simulation_is_touched() {
        let none: Option<Arc<CollisionClipData>> = None;
        assert!(clip::clip_parts(none).is_err());
    }
}
