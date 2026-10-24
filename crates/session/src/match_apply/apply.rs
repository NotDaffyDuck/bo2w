//! Carrying a loaded match into the running app.
//!
//! The loader hands over a [`PreparedMatchReady`]; this system decides
//! whether it is still wanted, checks it, builds the simulation and every
//! resource the match needs into one queue of commands, and applies that
//! queue only if the match is still the one the session wants.

use std::time::Instant;

use assets::{
    LoadingScreen, MatchLoadAbort, PreparedLocalizedStrings, PreparedMatchReady, PreparedWeapons,
};
use bevy::ecs::system::SystemParam;
use bevy::ecs::world::CommandQueue;
use bevy::prelude::*;
use frame::{
    ClassSelectHandoff, HasWorld, HostClassLoadouts, LaunchIdentity, LaunchReport, LocalLoadKey,
    MatchInstalled, MatchKey, RuntimeRole, WorldGeneration, WorldProducts,
};
use net::{AuthorityInputGate, AuthorityLoadHold, AuthorityWorld, MasterBridge};
use render_frontend::adapters::anim::dyn_ent::{DynEntPhysClip, DynEntPhysWorld};
use render_frontend::prepare::scene::camera::SimCamera;

use super::bootstrap::{self, IntermissionView, Start};
use super::clip;
use super::content::{self, ContentInputs};
use super::plan::{self, Catalogs, Effects, Launch, MatchInstallPlan, log_world_report};
use super::refusal::{InstallRefusal, install_facts, script_refusal};
use super::script_models::{self, ModelInstall, ModelSpawn};
use crate::SessionContentManifest;
use crate::startup::{PendingConsoleLines, StartupCommands};

/// What the app hands to the install: the loaded match and the settings it
/// was asked for under.
#[derive(SystemParam)]
pub struct PreparedMatchSource<'w, 's> {
    ready: ResMut<'w, PreparedMatchReady>,
    swap: Res<'w, crate::SessionSwapRequest>,
    bridge: Option<Res<'w, MasterBridge>>,
    intent: Option<Res<'w, net::MasterLaunchIntent>>,
    loading: Option<ResMut<'w, LoadingScreen>>,
    load: Option<Res<'w, assets::MapLoadProcess>>,
    abort: Option<Res<'w, MatchLoadAbort>>,
    host_classes: Option<Res<'w, HostClassLoadouts>>,
    identity: Option<Res<'w, LaunchIdentity>>,
    install_stage: Local<'s, Option<asset_transport::StageHandle>>,
}

/// Console lines typed before the match existed wait until the install is
/// over, one way or the other.
#[derive(SystemParam)]
pub struct MatchInstallConsole<'w> {
    startup: ResMut<'w, StartupCommands>,
    pending: ResMut<'w, PendingConsoleLines>,
}

impl MatchInstallConsole<'_> {
    fn release_startup(&mut self) {
        self.pending.0.append(&mut self.startup.lines);
    }
}

#[derive(SystemParam)]
pub struct MatchInstallAuthority<'w> {
    role: Res<'w, RuntimeRole>,
    prediction: Option<Res<'w, net::ClientPredictionState>>,
    input_gate: Res<'w, AuthorityInputGate>,
}

#[derive(SystemParam)]
pub struct MatchInstallPresentation<'w> {
    probe: Option<ResMut<'w, LaunchReport>>,
    camera: Res<'w, SimCamera>,
}

#[derive(SystemParam)]
pub struct MatchInstallOccupancy<'w> {
    has_world: ResMut<'w, HasWorld>,
    /// What the front end set up (game type, bots) when it started this.
    setup: Option<Res<'w, crate::Bo2mpMatchSetup>>,
}

/// Which load this is.
struct Load {
    request_id: u64,
    key: LocalLoadKey,
    zone: String,
}

/// The match the master connection says is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveMatch {
    in_match: bool,
    key: MatchKey,
    incarnation: u64,
}

fn live_match(bridge: &MasterBridge) -> LiveMatch {
    let state = bridge.state();
    LiveMatch {
        in_match: state.in_match(),
        key: state.identity().match_key(),
        incarnation: bridge.incarnation(),
    }
}

/// A load with no match key is a local one and needs no master; any other
/// must still be the match the master is running.
fn still_the_match(key: &LocalLoadKey, live: Option<LiveMatch>) -> bool {
    key.match_key.is_none()
        || live.is_some_and(|live| {
            live.in_match && live.key == key.match_key && live.incarnation == key.incarnation
        })
}

/// What stands between the loaded match and the install this frame.
#[derive(Clone, Copy, Debug, Default)]
struct Conditions {
    /// The session swap in progress still wants this load.
    accepted: bool,
    live: Option<LiveMatch>,
    /// A joining client waits for the host's match before it installs one.
    joining: bool,
    aborted: bool,
    /// The loading screen's own work is still running.
    loading: bool,
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

fn intake(request_id: u64, key: &LocalLoadKey, now: Conditions) -> Intake {
    if !now.accepted || !still_the_match(key, now.live) || key.local_load_request_id != request_id {
        return Intake::Stale;
    }
    if key.match_key.is_none() && now.joining {
        return Intake::WaitForJoin;
    }
    if now.aborted {
        return Intake::Aborted;
    }
    if now.loading && !now.stage_open {
        return Intake::OpenStage;
    }
    Intake::Go
}

/// Listening hosts, dedicated servers and replays own the simulation; a client
/// only predicts one.
fn holds_authority(role: &RuntimeRole) -> bool {
    role.runs_authority() || *role == RuntimeRole::Replay
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
    error: String,
    load: Load,
    stage: Option<asset_transport::StageHandle>,
    loading: Option<&mut LoadingScreen>,
    failed: &mut MessageWriter<frame::MapLoadFailed>,
) {
    diag::warn!(World, "match install refused: {error}");
    if let Some(stage) = stage {
        stage.fail();
    }
    if let Some(loading) = loading {
        loading.fail(error.clone());
    }
    failed.write(frame::MapLoadFailed {
        request_id: load.request_id,
        load_key: load.key,
        zone: load.zone,
        error,
    });
}

/// What the install takes from the running app.
struct AppView<'a> {
    load: &'a Load,
    started: Instant,
    authority: bool,
    prediction: Option<&'a net::ClientPredictionState>,
    host_classes: Option<&'a HostClassLoadouts>,
    camera: SimCamera,
    gate: AuthorityInputGate,
    world_report: Vec<String>,
}

/// What the map declares that is left after its resources are staged.
struct MapFactsLeft {
    airstrike_height: Option<f32>,
    gun_facts: String,
}

fn stage_map_facts(queue: &mut CommandQueue, facts: assets::MapFacts) -> MapFactsLeft {
    let assets::MapFacts {
        airstrike_height,
        script_sound,
        team_settings,
        t6_hud_icons,
        t6_hud_fonts,
        t6_ui,
        t6_playeranim,
        t6_ragdoll,
        t6_anim_speeds,
        t6_gun_facts,
        ..
    } = facts;
    stage(queue, asset_audio::SessionMapScriptSound(script_sound));
    stage(queue, asset_game::SessionTeamSettings(team_settings));
    stage(queue, t6_hud_icons);
    stage(queue, t6_hud_fonts);
    stage(queue, t6_ui);
    // Other players' third-person animations, and how a dead body goes limp.
    stage(
        queue,
        sim::t6_playeranim::T6PlayerAnimsRes::from_text(t6_playeranim.as_ref(), &t6_anim_speeds),
    );
    stage(
        queue,
        sim::t6_ragdoll::T6RagdollRes::from_text(t6_ragdoll.as_deref()),
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
fn stage_catalogs(queue: &mut CommandQueue, catalogs: Catalogs) -> PreparedWeapons {
    let Catalogs {
        weapons,
        fpv_meshes,
        bodies,
        world_weapons,
        projectile_meshes,
        xmodel_walk,
        xanims,
        death,
    } = catalogs;
    stage(queue, fpv_meshes);
    stage(queue, bodies);
    stage(queue, world_weapons);
    stage(queue, projectile_meshes);
    stage(queue, xmodel_walk);
    stage(queue, xanims);
    stage(queue, death);
    weapons
}

/// Give the simulation its scripts and start them. Returns the line the
/// script boundary log prints for this install.
fn start_scripts(
    sim: &mut sim::SimWorld,
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
    sim.install_gsc_program(program, sim::script::NativeRegistry::default(), level)
        .map_err(|fault| script_refusal(zone, gametype, "install", &fault))?;
    for (name, value) in &dvars {
        sim.set_gsc_dvar(name, value);
    }
    if let Some(t6) = t6 {
        match sim.install_t6(t6) {
            Ok(()) => sim.set_gsc_dvar("bo2zm_t6", "1"),
            Err(error) => diag::warn!(Sim, "bo2zm t6 scripts not installed: {error}"),
        }
    }
    for entry in entries {
        sim.start_gsc(&entry, sim::script::Value::level(), Vec::new())
            .map_err(|fault| script_refusal(zone, gametype, "entry", &fault))?;
    }
    Ok(facts)
}

/// A Black Ops II map's windows take damage by BO2's rules.
fn install_glass(
    sim: &mut sim::SimWorld,
    scene: &render_frontend::prepare::scene::world::WorldScene,
) {
    let Some(glass) = scene.t6_glass.as_ref().filter(|g| !g.server.is_empty()) else {
        return;
    };
    let panes = glass
        .server
        .iter()
        .enumerate()
        .map(|(index, pane)| {
            let pane = sim::T6GlassPane {
                origin: pane.origin,
                absmin: pane.absmin,
                absmax: pane.absmax,
                max_health: pane.max_health as f32,
            };
            (index as u32, pane)
        })
        .collect();
    sim.world_objects_mut().install_t6_glass(panes);
}

/// Put the map's script-moved models into the world, and tell the scene which
/// entities they became.
fn spawn_script_models(
    sim: &mut sim::SimWorld,
    scene: &mut render_frontend::prepare::scene::world::WorldScene,
    models: &mut ModelInstall,
    spawns: &[ModelSpawn],
) {
    script_models::spawn_movers(sim, spawns);
    for mover in std::mem::take(&mut models.brush_movers) {
        sim.spawn_brush_mover(mover.id, mover.cmodel, mover.origin, mover.angles)
            .expect("no free dynamic entity slot for a brush model");
    }
    script_models::stamp_entity_numbers(scene, sim);
    if !spawns.is_empty() {
        diag::info!(
            World,
            "script_model spawn: {} ET_SCRIPTMOVER from {}",
            sim.script_mover_count(),
            sim::gentity_spawn_base()
        );
    }
}

/// Fingerprint the content this match runs on, so peers can check theirs
/// against it.
fn stage_manifest(
    queue: &mut CommandQueue,
    sim: &sim::SimWorld,
    map: &assets::PreparedMap,
    weapons: &asset_game::WeaponRegistry,
    combat: &[weapon_t6::combat_facts::WeaponCombatFacts],
    equipment: &[sim::EquipmentRuntimeFacts],
) -> Result<(), InstallRefusal> {
    let manifest =
        SessionContentManifest::build(map, weapons, combat, equipment, sim.content_digest())
            .map_err(|error| InstallRefusal::new(format!("Invalid content manifest: {error:?}")))?;
    let components = sim.content_components();
    diag::info!(
        Sim,
        "session manifest: ruleset=bo2 map={:?} weapons={} digest={:016x} \
         gameplay={:016x} c_map={:016x} c_models={:016x} c_weapons={:016x} c_classes={:016x}",
        manifest.map,
        manifest.weapons.len(),
        manifest.digest,
        sim.content_digest(),
        components.map,
        components.models,
        components.weapons,
        components.classes
    );
    let Some(descriptor) = manifest.match_descriptor(components) else {
        return Err(InstallRefusal::new("Required match descriptor is missing"));
    };
    diag::info!(
        Sim,
        "match descriptor: map={:016x} weapons={:016x} classes={:016x}",
        descriptor.map,
        descriptor.weapons,
        descriptor.classes
    );
    stage(queue, descriptor);
    stage(queue, manifest);
    Ok(())
}

fn armed_prediction(
    prediction: &net::ClientPredictionState,
    sim: &sim::SimWorld,
) -> net::ClientPredictionState {
    let mut state = net::ClientPrediction::new(prediction.0.local());
    // Prediction adopts replicated state at admission; the prepared world's
    // script runtime belongs to authority and snapshots cannot restore it.
    state.arm_from_content(sim);
    net::ClientPredictionState(state)
}

/// Decide who holds the simulation and stage it there.
fn hand_over_simulation(
    queue: &mut CommandQueue,
    sim: sim::SimWorld,
    authority: bool,
    prediction: Option<&net::ClientPredictionState>,
) -> Result<(), InstallRefusal> {
    match owner_of(authority, prediction.is_some()) {
        Some(Owner::Authority) => {
            if let Some(prediction) = prediction {
                stage(queue, armed_prediction(prediction, &sim));
            }
            stage(queue, AuthorityWorld(sim));
        }
        Some(Owner::Predicting) => {
            if let Some(prediction) = prediction {
                queue.push(|world: &mut World| {
                    world.remove_resource::<AuthorityWorld>();
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
        started,
        authority,
        prediction,
        host_classes,
        mut camera,
        mut gate,
        world_report,
    } = app;
    let MatchInstallPlan {
        kind,
        scripts,
        mut scene,
        catalogs,
        effects,
        physics,
        mut map,
        strings,
        mut models,
        model_spawns,
        shocks,
    } = plan;
    let mut queue = CommandQueue::default();
    let spawn_count = map.spawns.len();
    let facts = stage_map_facts(&mut queue, std::mem::take(&mut map.facts));
    stage(&mut queue, PreparedLocalizedStrings(strings));
    stage_effects(&mut queue, effects);

    let plan::MapPhysics {
        clip: clip_data,
        pen_table,
        pen_table_loaded,
        hit_locations,
    } = physics;
    let configured = content::configure(ContentInputs {
        weapons: &catalogs.weapons.0,
        bodies: &catalogs.bodies.0,
        xanims: &catalogs.xanims,
        namespace: map.namespace.unwrap_or(asset_core::AssetNamespace::T6),
        gun_facts_dump: &facts.gun_facts,
        pen_table,
        pen_table_loaded,
        hit_locations,
        shocks: &shocks,
    });
    let content::Configured {
        content,
        combat,
        equipment,
        gun_facts,
        viewmodel,
    } = configured;
    stage(&mut queue, sim::t6_viewmodel::T6GunFactsRes(gun_facts));
    stage(&mut queue, sim::t6_viewmodel::T6VmTableRes(viewmodel));
    let (offers, excluded) = content::class_offers(catalogs.weapons.0.weapon_families());
    let weapons = stage_catalogs(&mut queue, catalogs);
    gate.local_cmds_enabled = false;
    stage(&mut queue, DynEntPhysWorld::default());
    stage(&mut queue, DynEntPhysClip(clip_data.clone()));

    let mut sim = sim::SimWorld::new();
    clip::install_collision(&mut sim, content, clip::clip_parts(clip_data)?, &mut models);
    let roster = bootstrap::roster(host_classes, &weapons.0, &combat, &equipment);
    let score_limit = std::env::var("BO2MP_SCORE_LIMIT").ok();
    let begun = bootstrap::start_match(
        &mut sim,
        Start {
            kind,
            spawns: &map.spawns,
            view: scene.intermission_view.map(|view| IntermissionView {
                origin: view.origin,
                angles: view.angles,
            }),
            airstrike_height: facts.airstrike_height,
            score_limit: score_limit.as_deref(),
        },
        roster,
        &mut camera,
        &mut gate,
    )?;

    let script_facts = start_scripts(&mut sim, scripts, &load.zone)?;
    let load_hold = AuthorityLoadHold(sim.clip_brush_count() > 0);
    install_glass(&mut sim, &scene);
    sim.world_objects_mut()
        .set_map_round_epoch(load.key.match_key.match_epoch);
    spawn_script_models(&mut sim, &mut scene, &mut models, &model_spawns);

    let class_handoff = ClassSelectHandoff {
        pending: !scene.batches.is_empty(),
        primary: offers.primary,
        secondary: offers.secondary,
        lethal: offers.lethal,
        tactical: offers.tactical,
        excluded,
        lock_reasons: begun.lock_reasons,
    };
    stage_manifest(&mut queue, &sim, &map, &weapons.0, &combat, &equipment)?;
    stage(&mut queue, weapons);
    diag::info!(
        World,
        "match preparation: {:.1}ms on the main thread (scene handoff, sim boot, catalogs)",
        started.elapsed().as_secs_f32() * 1000.0
    );
    hand_over_simulation(&mut queue, sim, authority, prediction)?;

    camera.freeze_fly = true;
    stage(&mut queue, WorldProducts::from_walk(scene.products_id));
    stage(&mut queue, scene);
    stage(&mut queue, camera);
    stage(&mut queue, gate);
    stage(&mut queue, load_hold);
    stage(&mut queue, class_handoff);
    stage(&mut queue, crate::LiveWorldIdentity { load_key: load.key });
    stage(&mut queue, HasWorld(true));
    stage(&mut queue, WorldGeneration::from_install(load.request_id));

    let (request_id, load_key, zone) = (load.request_id, load.key, load.zone.clone());
    let sim_gap = begun.gap;
    queue.push(move |world: &mut World| {
        if let Some(mut probe) = world.get_resource_mut::<LaunchReport>() {
            probe.sim_gap = sim_gap;
            probe.world_report = world_report;
        }
        diag::script_boundary("installed", &script_facts);
        diag::info!(
            Sim,
            "match: {} ({}) — world installed ({} dm spawn points); class select waits for admission",
            kind.display_name(),
            kind.token(),
            spawn_count
        );
        perf::match_installed(&zone, 1);
        world.write_message(MatchInstalled {
            request_id,
            load_key,
            zone,
            spawn_count,
        });
    });
    Ok(queue)
}

/// Apply a built install, unless the session moved on while it was built.
fn commit(world: &mut World, request_id: u64, key: LocalLoadKey, mut install: CommandQueue) {
    let accepted = world
        .resource::<crate::SessionSwapRequest>()
        .accepts_install(request_id);
    let live = world.get_resource::<MasterBridge>().map(live_match);
    if accepted && still_the_match(&key, live) {
        install.apply(world);
    }
}

/// Install the loaded match the frame it is ready and wanted.
#[allow(clippy::too_many_arguments)]
pub fn apply_prepared_match(
    mut commands: Commands,
    source: PreparedMatchSource,
    mut console: MatchInstallConsole,
    authority: MatchInstallAuthority,
    presentation: MatchInstallPresentation,
    occupancy: MatchInstallOccupancy,
    mode_selection: Option<Res<sim::HostGameModeSelection>>,
    rules: Option<Res<frame::HostMatchRules>>,
    mut failed: MessageWriter<frame::MapLoadFailed>,
) {
    let PreparedMatchSource {
        mut ready,
        swap,
        bridge,
        intent,
        mut loading,
        load: load_process,
        abort,
        host_classes,
        identity,
        mut install_stage,
    } = source;
    let live_load = load_process
        .as_deref()
        .filter(|process| !process.is_complete());
    let request_id = ready.request_id;
    let key = ready.load_key;
    let verdict = intake(
        request_id,
        &key,
        Conditions {
            accepted: swap.accepts_install(request_id),
            live: bridge.as_deref().map(live_match),
            joining: intent.as_deref().is_some_and(|intent| intent.is_join()),
            aborted: abort.as_deref().is_some_and(|abort| abort.0 == request_id),
            loading: live_load.is_some(),
            stage_open: install_stage.is_some(),
        },
    );
    match verdict {
        Intake::Stale => {
            diag::info!(
                World,
                "match install: discarded stale load {:?} ({})",
                key,
                ready.zone
            );
            if let Some(stage) = install_stage.take() {
                stage.cancel();
            }
            commands.remove_resource::<PreparedMatchReady>();
            return;
        }
        Intake::WaitForJoin => return,
        Intake::Aborted => {
            if let Some(stage) = install_stage.take() {
                stage.cancel();
            }
            commands.remove_resource::<PreparedMatchReady>();
            commands.remove_resource::<MatchLoadAbort>();
            diag::info!(
                World,
                "match install: aborted request #{request_id} (`{}`) — not occupying the menu",
                ready.zone
            );
            return;
        }
        Intake::OpenStage => {
            if let Some(process) = live_load {
                *install_stage = Some(
                    process
                        .progress
                        .begin(asset_transport::StageId::Install, None),
                );
            }
            diag::info!(
                World,
                "match install: yield one frame so the overlay can name the main-thread hitch"
            );
            return;
        }
        Intake::Go => {}
    }

    let MatchInstallAuthority {
        role,
        prediction,
        input_gate,
    } = authority;
    let MatchInstallPresentation { mut probe, camera } = presentation;
    let MatchInstallOccupancy {
        mut has_world,
        setup,
    } = occupancy;

    let stage_handle = install_stage.take();
    let started = Instant::now();
    let load = Load {
        request_id,
        key,
        zone: std::mem::take(&mut ready.zone),
    };
    let mut prepared = std::mem::take(&mut ready.prepared);
    commands.remove_resource::<PreparedMatchReady>();
    let world_report = std::mem::take(&mut prepared.report);
    log_world_report(&world_report);

    let checked = plan::preflight(
        prepared,
        Launch {
            zone: &load.zone,
            mode_selection: mode_selection.as_deref(),
            rules: rules.as_deref(),
            identity: identity.as_deref(),
            setup: setup.as_deref(),
        },
    );
    let plan = match checked {
        Ok(plan) => plan,
        Err(refusal) => {
            if let Some(probe) = probe.as_deref_mut() {
                if let Some(gap) = refusal.sim_gap {
                    probe.sim_gap = gap;
                }
                probe.world_report = world_report;
            }
            *has_world = HasWorld(false);
            refuse(
                refusal.error,
                load,
                stage_handle,
                loading.as_deref_mut(),
                &mut failed,
            );
            console.release_startup();
            return;
        }
    };
    let built = install_match(
        plan,
        AppView {
            load: &load,
            started,
            authority: holds_authority(&role),
            prediction: prediction.as_deref(),
            host_classes: host_classes.as_deref(),
            camera: *camera,
            gate: *input_gate,
            world_report,
        },
    );
    match built {
        Ok(install) => {
            commands.queue(move |world: &mut World| commit(world, request_id, key, install));
            if let Some(stage) = stage_handle {
                stage.done();
            }
        }
        Err(refusal) => {
            refuse(
                refusal.error,
                load,
                stage_handle,
                loading.as_deref_mut(),
                &mut failed,
            );
        }
    }
    console.release_startup();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asset_world::ClipCollision;

    use super::*;

    const REQUEST: u64 = 7;

    fn local() -> LocalLoadKey {
        LocalLoadKey::from_request(REQUEST, MatchKey::NONE, 0)
    }

    fn networked(incarnation: u64) -> LocalLoadKey {
        LocalLoadKey::from_request(REQUEST, MatchKey::new([1; 16], 3), incarnation)
    }

    fn running(incarnation: u64) -> Option<LiveMatch> {
        Some(LiveMatch {
            in_match: true,
            key: MatchKey::new([1; 16], 3),
            incarnation,
        })
    }

    fn wanted() -> Conditions {
        Conditions {
            accepted: true,
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
        over.in_match = false;
        assert!(!still_the_match(&key, Some(over)));
        let mut other = running(5).unwrap();
        other.key = MatchKey::new([2; 16], 3);
        assert!(!still_the_match(&key, Some(other)));
    }

    #[test]
    fn a_load_the_swap_no_longer_wants_is_stale() {
        let now = Conditions {
            accepted: false,
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
            accepted: false,
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
            loading: true,
            ..wanted()
        };
        assert_eq!(intake(REQUEST, &local(), now), Intake::OpenStage);
        let now = Conditions {
            loading: true,
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
        assert!(holds_authority(&RuntimeRole::Listen));
        assert!(holds_authority(&RuntimeRole::Dedicated));
        assert!(holds_authority(&RuntimeRole::Replay));
        assert!(!holds_authority(&RuntimeRole::Client));
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
        stage(&mut queue, HasWorld(true));
        assert!(world.get_resource::<HasWorld>().is_none());
        queue.apply(&mut world);
        assert!(world.resource::<HasWorld>().0);
    }

    #[test]
    fn a_client_install_clears_the_authority_world_it_inherited() {
        let mut world = World::new();
        world.insert_resource(AuthorityWorld(sim::SimWorld::new()));
        let mut queue = CommandQueue::default();
        queue.push(|world: &mut World| {
            world.remove_resource::<AuthorityWorld>();
        });
        queue.apply(&mut world);
        assert!(world.get_resource::<AuthorityWorld>().is_none());
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
        assert!(world.get_resource::<AuthorityWorld>().is_some());
        assert!(world.get_resource::<net::ClientPredictionState>().is_none());
    }

    #[test]
    fn a_bare_clip_is_refused_before_the_simulation_is_touched() {
        let none: Option<Arc<ClipCollision>> = None;
        assert!(clip::clip_parts(none).is_err());
    }
}
