//! The frame's ordering vocabulary: system sets for the server tick, the
//! client frame, the render stages and the render-worker command table.
//!
//! Each ordered family lists its members once (in a macro) and gets its enum,
//! its table of contents and its label function from that single list, so the
//! three cannot drift apart.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

/// Declares an enum of system sets that run in declaration order, plus its
/// table of contents and a label function.
macro_rules! ordered_sets {
    (
        $(#[$meta:meta])*
        $ty:ident, $toc:ident, $label:ident { $($member:ident),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $ty {
            $($member),+
        }

        /// Every member, in the order the family runs.
        pub const $toc: &[$ty] = &[$($ty::$member),+];

        /// The member's name as a static string.
        pub fn $label(set: &$ty) -> &'static str {
            match set {
                $($ty::$member => stringify!($member)),+
            }
        }
    };
}

/// Declares zero-sized marker sets other systems can order against.
macro_rules! marker_sets {
    ($($(#[$meta:meta])* $name:ident;)+) => {
        $(
            $(#[$meta])*
            #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
            pub struct $name;
        )+
    };
}

ordered_sets! {
    /// The server tick, front to back.
    ServerStage, SERVER_STAGES, server_stage_name {
        Advance, Ingress, Gather, Step, Snapshot, Fanout, Bookkeeping,
    }
}

ordered_sets! {
    /// The client frame, front to back.
    ClientStep, CLIENT_STAGES, client_stage_name {
        Load, Receive, Reconcile, Input, Predict, Send, Present, Ui, Effects, Diag,
    }
}

ordered_sets! {
    /// Render stages inside [`ClientSet::Present`] (and again, unnested, in
    /// `PostUpdate`).
    DrawPassStage, RENDER_ORDER, render_set_name {
        FrontPrep, Anim, Fx, FrontBuild, Gpu,
    }
}

/// Bracket sets around position `n` of a table of contents: everything
/// before member `n` runs before `PlayerHandoffLink(n)`, and member `n` runs
/// before `PlayerHandoffLink(n + 1)`. Lets a system say "after the first four
/// stages" without naming them.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerHandoffLink(pub u8);

/// The same brackets for [`AUTHORITY_TOC`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerHandoffLink(pub u8);

marker_sets! {
    /// Systems that finish the tick's bookkeeping.
    ServerLedgerMark;
    /// The class a player will spawn with has been resolved.
    KitEquipDone;
    /// The presented state for this frame has been published.
    ShownStatePosted;
    /// The front of a life (its first frame) has been published.
    LifeStatePosted;
    /// Effect and sound requests for this frame have been published.
    EffectAudioPosted;
    /// A session swap has been applied to the world.
    MatchSwapDone;
    /// Model lighting has been seated for the frame's scene entities.
    ModelLightPlaced;
}

/// Declares the render-worker command table: one entry per command, in the
/// order the table is kept, each with the name it prints under.
macro_rules! worker_commands {
    ($($member:ident => $name:literal),+ $(,)?) => {
        /// A command the render worker runs inside the Present stage.
        /// The discriminant is the command's index in [`WORKER_CMD_TOC`].
        #[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum JobQueueStage {
            $($member),+
        }

        /// Every command, indexed by discriminant.
        pub const WORKER_CMD_TOC: &[JobQueueStage] = &[$(JobQueueStage::$member),+];

        /// Each command's printed name, indexed like [`WORKER_CMD_TOC`].
        pub const WORKER_CMD_RETAIL_NAMES: &[&str] = &[$($name),+];
    };
}

worker_commands! {
    CellDynBrush => "cell_dyn_brush",
    CellDynModel => "cell_dyn_model",
    CellSceneEnt => "cell_scene_ent",
    DpvsEnt => "dpvs_ent",
    BoundEnt => "bound_ent",
    SpotShadowEnt => "spot_shadow_ent",
    FxNonDependent => "fx_non_dependent",
    Glass => "glass",
    FxRemaining => "fx_remaining",
    CellStatic => "cell_static",
    AddSceneEnt => "add_scene_ent",
    CellGlass => "cell_glass",
    GlassLight => "glass_light",
    GlassVerts => "glass_verts",
    MarkVerts => "mark_verts",
    DynMarkVerts => "dyn_mark_verts",
    FxVerts => "fx_verts",
    SmodelCache => "smodelcache",
    SkinModel => "skin_model",
    FxPhysics => "fx_physics",
    Physics => "physics",
}

pub fn worker_cmd_name(set: JobQueueStage) -> &'static str {
    // The discriminant is the table index (checked by the tests).
    let index = set as u8;
    WORKER_CMD_RETAIL_NAMES[usize::from(index)]
}

/// `(later, earlier)`: `later` must run after `earlier`. Commands not named
/// here are unordered against each other.
pub const WORKER_CMD_AFTER: &[(JobQueueStage, JobQueueStage)] = &[
    (JobQueueStage::FxRemaining, JobQueueStage::FxNonDependent),
    (JobQueueStage::FxRemaining, JobQueueStage::Glass),
    (JobQueueStage::CellGlass, JobQueueStage::Glass),
    (JobQueueStage::GlassLight, JobQueueStage::CellGlass),
    (JobQueueStage::GlassVerts, JobQueueStage::CellGlass),
    (JobQueueStage::GlassVerts, JobQueueStage::GlassLight),
    (JobQueueStage::DynMarkVerts, JobQueueStage::MarkVerts),
    (JobQueueStage::FxPhysics, JobQueueStage::Glass),
];

/// Commands whose results the frame waits on before it finishes.
pub const JOB_QUEUE_TAIL_BARRIER: &[JobQueueStage] = &[
    JobQueueStage::GlassVerts,
    JobQueueStage::MarkVerts,
    JobQueueStage::FxVerts,
    JobQueueStage::SmodelCache,
    JobQueueStage::SkinModel,
];

/// Commands that run on a worker thread rather than on the render thread.
pub const WORKER_CMD_NOT_RENDER_THREAD: &[JobQueueStage] = &[
    JobQueueStage::DpvsEnt,
    JobQueueStage::BoundEnt,
    JobQueueStage::SpotShadowEnt,
    JobQueueStage::FxRemaining,
];

/// Runs `toc` in order inside `schedule`, and brackets member `i` between
/// `edge(i)` and `edge(i + 1)`.
fn order_with_edges<S, E>(
    app: &mut App,
    schedule: impl ScheduleLabel + Clone,
    toc: &[S],
    edge: fn(u8) -> E,
) where
    S: SystemSet + Clone,
    E: SystemSet,
{
    for pair in toc.windows(2) {
        app.configure_sets(schedule.clone(), pair[1].clone().after(pair[0].clone()));
    }
    for (index, set) in (0u8..).zip(toc) {
        let bracketed = set.clone().after(edge(index)).before(edge(index + 1));
        app.configure_sets(schedule.clone(), bracketed);
    }
}

pub fn order_draw_stages(app: &mut App) {
    let mut previous: Option<DrawPassStage> = None;
    for &stage in RENDER_ORDER {
        App::configure_sets(app, Update, stage.in_set(ClientStep::Present));
        if let Some(before) = previous {
            // The same order holds in the unnested PostUpdate copy.
            App::configure_sets(app, Update, stage.after(before));
            app.configure_sets(PostUpdate, stage.after(before));
        }
        previous = Some(stage);
    }
}

pub fn order_server_side_stages(app: &mut App) {
    let server_tick = FixedUpdate;
    let members = SERVER_STAGES;
    order_with_edges(app, server_tick, members, ServerHandoffLink);
    let bookkeeping = ServerLedgerMark.in_set(ServerStage::Bookkeeping);
    app.configure_sets(FixedUpdate, bookkeeping);
}

pub fn order_player_side_stages(app: &mut App) {
    let members = CLIENT_STAGES;
    order_with_edges(app, Update, members, PlayerHandoffLink);
    let publication = EffectAudioPosted.in_set(ClientStep::Effects);
    App::configure_sets(app, Update, publication);
    order_job_queue_stages(app);
}

/// Every worker command sits inside Present, after the commands
/// [`WORKER_CMD_AFTER`] says it follows; lighting is seated while scene
/// entities are added.
pub fn order_job_queue_stages(app: &mut App) {
    for &cmd in WORKER_CMD_TOC {
        App::configure_sets(app, Update, cmd.in_set(ClientStep::Present));
        let follows = WORKER_CMD_AFTER.iter().filter(|edge| edge.0 == cmd).map(|edge| edge.1);
        for earlier in follows {
            App::configure_sets(app, Update, cmd.after(earlier));
        }
    }
    let seated = ModelLightPlaced.in_set(JobQueueStage::AddSceneEnt);
    App::configure_sets(app, Update, seated);
}
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Order(Vec<String>);

    fn probe<S: SystemSet>(
        app: &mut App,
        schedule: impl ScheduleLabel,
        set: S,
        tag: impl Into<String>,
    ) {
        let tag = tag.into();
        app.add_systems(
            schedule,
            (move |mut order: ResMut<Order>| order.0.push(tag.clone())).in_set(set),
        );
    }

    fn run(app: &mut App, schedule: impl ScheduleLabel) -> Vec<String> {
        app.init_resource::<Order>();
        app.world_mut().run_schedule(schedule);
        std::mem::take(&mut app.world_mut().resource_mut::<Order>().0)
    }

    /// A bare app with one family's sets configured.
    fn app_with(configure: &[fn(&mut App)]) -> App {
        let mut app = App::new();
        configure.iter().for_each(|step| step(&mut app));
        app
    }

    fn position(order: &[String], tag: &str) -> usize {
        order
            .iter()
            .position(|seen| seen == tag)
            .unwrap_or_else(|| panic!("{tag} never ran"))
    }

    #[test]
    fn tables_list_every_member_once_in_declaration_order() {
        assert_eq!(SERVER_STAGES.len(), 7);
        assert_eq!(CLIENT_STAGES.len(), 10);
        assert_eq!(WORKER_CMD_TOC.len(), 21);
        assert_eq!(WORKER_CMD_RETAIL_NAMES.len(), WORKER_CMD_TOC.len());
        assert!(WORKER_CMD_TOC.iter().zip(0usize..).all(|(cmd, at)| *cmd as usize == at), "a command is out of place");
        let server_debug: Vec<String> = SERVER_STAGES.iter().map(|set| format!("{set:?}")).collect();
        assert_eq!(SERVER_STAGES.iter().map(server_stage_name).collect::<Vec<_>>(), server_debug);
        let client: Vec<String> = CLIENT_STAGES.iter().map(|set| format!("{set:?}")).collect();
        assert_eq!(CLIENT_STAGES.iter().map(client_stage_name).collect::<Vec<_>>(), client);
        let render: Vec<String> = RENDER_ORDER.iter().map(|set| format!("{set:?}")).collect();
        assert_eq!(RENDER_ORDER.iter().map(render_set_name).collect::<Vec<_>>(), render);
    }

    #[test]
    fn worker_names_are_unique_and_looked_up_by_discriminant() {
        let distinct: std::collections::BTreeSet<&str> = WORKER_CMD_RETAIL_NAMES.iter().copied().collect();
        assert_eq!(distinct.len(), WORKER_CMD_TOC.len());
        let brush_name = worker_cmd_name(JobQueueStage::CellDynBrush);
        assert_eq!(brush_name, "cell_dyn_brush");
        assert_eq!(worker_cmd_name(JobQueueStage::SmodelCache), "smodelcache");
        assert_eq!(worker_cmd_name(JobQueueStage::Physics), "physics");
    }

    #[test]
    fn worker_side_tables_only_name_known_commands_once() {
        for list in [JOB_QUEUE_TAIL_BARRIER, WORKER_CMD_NOT_RENDER_THREAD] {
            let mut seen = Vec::new();
            for cmd in list {
                assert!(WORKER_CMD_TOC.contains(cmd));
                assert!(!seen.contains(cmd), "{cmd:?} listed twice");
                seen.push(*cmd);
            }
        }
        for (late, early) in WORKER_CMD_AFTER {
            assert_ne!(late, early);
        }
    }

    #[test]
    fn after_edges_never_loop() {
        // Following "runs after" links from any command must terminate.
        for &start in WORKER_CMD_TOC {
            let mut frontier = vec![start];
            let budget = WORKER_CMD_TOC.len() * 4;
            for visited in 1.. {
                let Some(cmd) = frontier.pop() else { break };
                assert!(visited <= budget, "cycle through {start:?}");
                let next = WORKER_CMD_AFTER.iter().filter(|(later, _)| *later == cmd);
                frontier.extend(next.map(|(_, earlier)| *earlier));
            }
        }
    }

    #[test]
    fn the_server_tick_runs_in_table_order() {
        let mut app = app_with(&[order_server_side_stages]);
        let tick = FixedUpdate.intern();
        for member in SERVER_STAGES {
            probe(&mut app, tick, member.clone(), server_stage_name(member));
        }
        let ran = run(&mut app, tick);
        let expected: Vec<&str> = SERVER_STAGES.iter().map(server_stage_name).collect();
        assert_eq!(ran, expected);
    }

    #[test]
    fn bookkeeping_marker_runs_with_the_bookkeeping_stage() {
        let mut app = app_with(&[order_server_side_stages]);
        let tick = FixedUpdate.intern();
        let marker = ServerLedgerMark;
        let fanout = ServerStage::Fanout;
        probe(&mut app, tick, marker, "marker");
        probe(&mut app, tick, fanout, "fanout");
        let ran = run(&mut app, tick);
        assert!(position(&ran, "fanout") < position(&ran, "marker"));
    }

    #[test]
    fn authority_edges_bracket_their_member() {
        let mut app = app_with(&[order_server_side_stages]);
        for (index, set) in (0u8..).zip(SERVER_STAGES) {
            probe(&mut app, FixedUpdate, set.clone(), format!("set{index}"));
        }
        for index in 0..=SERVER_STAGES.len() as u8 {
            probe(
                &mut app,
                FixedUpdate,
                ServerHandoffLink(index),
                format!("edge{index}"),
            );
        }
        let ran = run(&mut app, FixedUpdate);
        for index in 0..SERVER_STAGES.len() {
            let before = position(&ran, &format!("edge{index}"));
            let member = position(&ran, &format!("set{index}"));
            let after = position(&ran, &format!("edge{}", index + 1));
            assert!(before < member && member < after, "member {index}");
        }
    }

    #[test]
    fn the_client_frame_runs_in_table_order() {
        let mut app = app_with(&[order_player_side_stages]);
        for member in CLIENT_STAGES {
            probe(&mut app, Update, member.clone(), client_stage_name(member));
        }
        let ran = run(&mut app, Update);
        let expected: Vec<&str> = CLIENT_STAGES.iter().map(client_stage_name).collect();
        assert_eq!(ran, expected);
    }

    #[test]
    fn client_edges_bracket_their_member() {
        let mut app = app_with(&[order_player_side_stages]);
        for (index, set) in (0u8..).zip(CLIENT_STAGES) {
            probe(&mut app, Update, set.clone(), format!("set{index}"));
        }
        for index in 0..=CLIENT_STAGES.len() as u8 {
            probe(&mut app, Update, PlayerHandoffLink(index), format!("edge{index}"));
        }
        let ran = run(&mut app, Update);
        for index in 0..CLIENT_STAGES.len() {
            let before = position(&ran, &format!("edge{index}"));
            let member = position(&ran, &format!("set{index}"));
            let after = position(&ran, &format!("edge{}", index + 1));
            assert!(before < member && member < after, "member {index}");
        }
    }

    #[test]
    fn effect_publication_belongs_to_the_effects_stage() {
        let mut app = app_with(&[order_player_side_stages]);
        probe(&mut app, Update, ClientStep::Ui, "ui");
        let published = EffectAudioPosted;
        probe(&mut app, Update, published, "published");
        probe(&mut app, Update, ClientStep::Diag, "diag");
        let ran = run(&mut app, Update);
        assert!(position(&ran, "ui") < position(&ran, "published"));
        assert!(position(&ran, "published") < position(&ran, "diag"));
    }

    #[test]
    fn worker_commands_run_inside_present_and_honour_their_after_edges() {
        let mut app = app_with(&[order_player_side_stages]);
        probe(&mut app, Update, ClientStep::Send, "send");
        probe(&mut app, Update, ClientStep::Ui, "ui");
        for &cmd in WORKER_CMD_TOC {
            probe(&mut app, Update, cmd, worker_cmd_name(cmd));
        }
        let ran = run(&mut app, Update);
        assert_eq!(ran.len(), WORKER_CMD_TOC.len() + 2);
        for &cmd in WORKER_CMD_TOC {
            let at = position(&ran, worker_cmd_name(cmd));
            assert!(position(&ran, "send") < at, "{cmd:?} ran before Send");
            assert!(at < position(&ran, "ui"), "{cmd:?} ran after Ui");
        }
        for &(late, early) in WORKER_CMD_AFTER {
            assert!(
                position(&ran, worker_cmd_name(early)) < position(&ran, worker_cmd_name(late)),
                "{late:?} must follow {early:?}"
            );
        }
    }

    #[test]
    fn lighting_is_seated_as_part_of_adding_scene_entities() {
        let mut app = app_with(&[order_player_side_stages]);
        let seated = ModelLightPlaced;
        probe(&mut app, Update, seated, "lighting");
        app.add_systems(
            Update,
            (|mut order: ResMut<Order>| order.0.push("after_add".into()))
                .after(JobQueueStage::AddSceneEnt),
        );
        probe(&mut app, Update, ClientStep::Ui, "ui");
        let ran = run(&mut app, Update);
        assert!(position(&ran, "lighting") < position(&ran, "after_add"));
        assert!(position(&ran, "lighting") < position(&ran, "ui"));
    }

    #[test]
    fn render_stages_run_in_order_in_both_schedules() {
        for post in [false, true] {
            let mut app = app_with(&[order_player_side_stages, order_draw_stages]);
            for (index, &stage) in (0..).zip(RENDER_ORDER) {
                if post {
                    let late_copy = PostUpdate;
                    probe(&mut app, late_copy, stage, format!("stage{index}"));
                } else {
                    probe(&mut app, Update, stage, format!("stage{index}"));
                }
            }
            let ran = if post {
                run(&mut app, PostUpdate)
            } else {
                run(&mut app, Update)
            };
            let expected: Vec<String> = (0..RENDER_ORDER.len())
                .map(|i| format!("stage{i}"))
                .collect();
            assert_eq!(ran, expected, "post = {post}");
        }
    }

    #[test]
    fn render_stages_sit_between_send_and_ui_in_the_client_frame() {
        let mut app = app_with(&[order_player_side_stages, order_draw_stages]);
        probe(&mut app, Update, ClientStep::Send, "send");
        probe(&mut app, Update, ClientStep::Ui, "ui");
        for &stage in RENDER_ORDER {
            probe(&mut app, Update, stage, format!("{stage:?}"));
        }
        let ran = run(&mut app, Update);
        for &stage in RENDER_ORDER {
            let at = position(&ran, &format!("{stage:?}"));
            assert!(position(&ran, "send") < at && at < position(&ran, "ui"));
        }
    }
}
