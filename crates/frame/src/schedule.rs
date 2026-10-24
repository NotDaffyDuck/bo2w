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
        $ty:ident, $toc:ident, $label:ident { $($variant:ident),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $ty {
            $($variant),+
        }

        /// Every member, in the order the family runs.
        pub const $toc: &[$ty] = &[$($ty::$variant),+];

        /// The member's name as a static string.
        pub fn $label(set: &$ty) -> &'static str {
            match set {
                $($ty::$variant => stringify!($variant)),+
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
    AuthoritySet, AUTHORITY_TOC, authority_set_name {
        Advance, Ingress, Gather, Step, Snapshot, Fanout, Bookkeeping,
    }
}

ordered_sets! {
    /// The client frame, front to back.
    ClientSet, CLIENT_TOC, client_set_name {
        Load, Receive, Reconcile, Input, Predict, Send, Present, Ui, Effects, Diag,
    }
}

/// Render stages inside [`ClientSet::Present`] (and again, unnested, in
/// `PostUpdate`).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderSet {
    FrontendPrepare,
    Anim,
    Fx,
    FrontendAssemble,
    Gpu,
}

const RENDER_ORDER: [RenderSet; 5] = [
    RenderSet::FrontendPrepare,
    RenderSet::Anim,
    RenderSet::Fx,
    RenderSet::FrontendAssemble,
    RenderSet::Gpu,
];

/// Bracket sets around position `n` of a table of contents: everything
/// before member `n` runs before `ClientEdge(n)`, and member `n` runs
/// before `ClientEdge(n + 1)`. Lets a system say "after the first four
/// stages" without naming them.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientEdge(pub u8);

/// The same brackets for [`AUTHORITY_TOC`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthorityEdge(pub u8);

marker_sets! {
    /// Systems that finish the tick's bookkeeping.
    AuthorityBookkeeping;
    /// The class a player will spawn with has been resolved.
    ClassEquipResolved;
    /// The presented state for this frame has been published.
    PresentedPublished;
    /// The front of a life (its first frame) has been published.
    LifeFrontPublished;
    /// Effect and sound requests for this frame have been published.
    FxSoundPublished;
    /// A session swap has been applied to the world.
    SessionSwapApplied;
    /// Model lighting has been seated for the frame's scene entities.
    ModelLightingSeated;
}

/// Declares the render-worker command table: one entry per command, in the
/// order the table is kept, each with the name it prints under.
macro_rules! worker_commands {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// A command the render worker runs inside the Present stage.
        /// The discriminant is the command's index in [`WORKER_CMD_TOC`].
        #[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum WorkerCmdSet {
            $($variant),+
        }

        /// Every command, indexed by discriminant.
        pub const WORKER_CMD_TOC: &[WorkerCmdSet] = &[$(WorkerCmdSet::$variant),+];

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

pub fn worker_cmd_name(set: WorkerCmdSet) -> &'static str {
    WORKER_CMD_RETAIL_NAMES[set as usize]
}

/// `(later, earlier)`: `later` must run after `earlier`. Commands not named
/// here are unordered against each other.
pub const WORKER_CMD_AFTER: &[(WorkerCmdSet, WorkerCmdSet)] = &[
    (WorkerCmdSet::FxRemaining, WorkerCmdSet::FxNonDependent),
    (WorkerCmdSet::FxRemaining, WorkerCmdSet::Glass),
    (WorkerCmdSet::CellGlass, WorkerCmdSet::Glass),
    (WorkerCmdSet::GlassLight, WorkerCmdSet::CellGlass),
    (WorkerCmdSet::GlassVerts, WorkerCmdSet::CellGlass),
    (WorkerCmdSet::GlassVerts, WorkerCmdSet::GlassLight),
    (WorkerCmdSet::DynMarkVerts, WorkerCmdSet::MarkVerts),
    (WorkerCmdSet::FxPhysics, WorkerCmdSet::Glass),
];

/// Commands whose results the frame waits on before it finishes.
pub const WORKER_CMD_END_FENCE: &[WorkerCmdSet] = &[
    WorkerCmdSet::GlassVerts,
    WorkerCmdSet::MarkVerts,
    WorkerCmdSet::FxVerts,
    WorkerCmdSet::SmodelCache,
    WorkerCmdSet::SkinModel,
];

/// Commands that run on a worker thread rather than on the render thread.
pub const WORKER_CMD_NOT_RENDER_THREAD: &[WorkerCmdSet] = &[
    WorkerCmdSet::DpvsEnt,
    WorkerCmdSet::BoundEnt,
    WorkerCmdSet::SpotShadowEnt,
    WorkerCmdSet::FxRemaining,
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
        app.configure_sets(
            schedule.clone(),
            set.clone().after(edge(index)).before(edge(index + 1)),
        );
    }
}

pub fn configure_render_sets(app: &mut App) {
    for stage in RENDER_ORDER {
        app.configure_sets(Update, stage.in_set(ClientSet::Present));
    }
    for pair in RENDER_ORDER.windows(2) {
        app.configure_sets(Update, pair[1].after(pair[0]));
        app.configure_sets(PostUpdate, pair[1].after(pair[0]));
    }
}

pub fn configure_authority_sets(app: &mut App) {
    order_with_edges(app, FixedUpdate, AUTHORITY_TOC, AuthorityEdge);
    app.configure_sets(
        FixedUpdate,
        AuthorityBookkeeping.in_set(AuthoritySet::Bookkeeping),
    );
}

pub fn configure_client_sets(app: &mut App) {
    order_with_edges(app, Update, CLIENT_TOC, ClientEdge);
    app.configure_sets(Update, FxSoundPublished.in_set(ClientSet::Effects));
    configure_worker_cmd_sets(app);
}

pub fn configure_worker_cmd_sets(app: &mut App) {
    for &cmd in WORKER_CMD_TOC {
        app.configure_sets(Update, cmd.in_set(ClientSet::Present));
    }
    for &(later, earlier) in WORKER_CMD_AFTER {
        app.configure_sets(Update, later.after(earlier));
    }
    app.configure_sets(
        Update,
        ModelLightingSeated.in_set(WorkerCmdSet::AddSceneEnt),
    );
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

    fn position(order: &[String], tag: &str) -> usize {
        order
            .iter()
            .position(|seen| seen == tag)
            .unwrap_or_else(|| panic!("{tag} never ran"))
    }

    #[test]
    fn tables_list_every_member_once_in_declaration_order() {
        assert_eq!(AUTHORITY_TOC.len(), 7);
        assert_eq!(CLIENT_TOC.len(), 10);
        assert_eq!(WORKER_CMD_TOC.len(), 21);
        assert_eq!(WORKER_CMD_RETAIL_NAMES.len(), WORKER_CMD_TOC.len());
        for (index, cmd) in WORKER_CMD_TOC.iter().enumerate() {
            assert_eq!(*cmd as usize, index, "{cmd:?} is out of place");
        }
        for (index, set) in AUTHORITY_TOC.iter().enumerate() {
            assert_eq!(authority_set_name(set), format!("{set:?}"), "at {index}");
        }
        for (index, set) in CLIENT_TOC.iter().enumerate() {
            assert_eq!(client_set_name(set), format!("{set:?}"), "at {index}");
        }
    }

    #[test]
    fn worker_names_are_unique_and_looked_up_by_discriminant() {
        let mut names = WORKER_CMD_RETAIL_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), WORKER_CMD_TOC.len());
        assert_eq!(
            worker_cmd_name(WorkerCmdSet::CellDynBrush),
            "cell_dyn_brush"
        );
        assert_eq!(worker_cmd_name(WorkerCmdSet::SmodelCache), "smodelcache");
        assert_eq!(worker_cmd_name(WorkerCmdSet::Physics), "physics");
    }

    #[test]
    fn worker_side_tables_only_name_known_commands_once() {
        for list in [WORKER_CMD_END_FENCE, WORKER_CMD_NOT_RENDER_THREAD] {
            let mut seen = Vec::new();
            for cmd in list {
                assert!(WORKER_CMD_TOC.contains(cmd));
                assert!(!seen.contains(cmd), "{cmd:?} listed twice");
                seen.push(*cmd);
            }
        }
        for (later, earlier) in WORKER_CMD_AFTER {
            assert_ne!(later, earlier);
        }
    }

    #[test]
    fn after_edges_never_loop() {
        // Following "runs after" links from any command must terminate.
        for &start in WORKER_CMD_TOC {
            let mut frontier = vec![start];
            let mut steps = 0;
            while let Some(cmd) = frontier.pop() {
                steps += 1;
                assert!(steps <= WORKER_CMD_TOC.len() * 4, "cycle through {start:?}");
                frontier.extend(
                    WORKER_CMD_AFTER
                        .iter()
                        .filter(|(later, _)| *later == cmd)
                        .map(|(_, earlier)| *earlier),
                );
            }
        }
    }

    #[test]
    fn the_server_tick_runs_in_table_order() {
        let mut app = App::new();
        configure_authority_sets(&mut app);
        for set in AUTHORITY_TOC {
            probe(&mut app, FixedUpdate, set.clone(), authority_set_name(set));
        }
        let ran = run(&mut app, FixedUpdate);
        let expected: Vec<&str> = AUTHORITY_TOC.iter().map(authority_set_name).collect();
        assert_eq!(ran, expected);
    }

    #[test]
    fn bookkeeping_marker_runs_with_the_bookkeeping_stage() {
        let mut app = App::new();
        configure_authority_sets(&mut app);
        probe(&mut app, FixedUpdate, AuthorityBookkeeping, "marker");
        probe(&mut app, FixedUpdate, AuthoritySet::Fanout, "fanout");
        let ran = run(&mut app, FixedUpdate);
        assert!(position(&ran, "fanout") < position(&ran, "marker"));
    }

    #[test]
    fn authority_edges_bracket_their_member() {
        let mut app = App::new();
        configure_authority_sets(&mut app);
        for (index, set) in (0u8..).zip(AUTHORITY_TOC) {
            probe(&mut app, FixedUpdate, set.clone(), format!("set{index}"));
        }
        for index in 0..=AUTHORITY_TOC.len() as u8 {
            probe(
                &mut app,
                FixedUpdate,
                AuthorityEdge(index),
                format!("edge{index}"),
            );
        }
        let ran = run(&mut app, FixedUpdate);
        for index in 0..AUTHORITY_TOC.len() {
            let before = position(&ran, &format!("edge{index}"));
            let member = position(&ran, &format!("set{index}"));
            let after = position(&ran, &format!("edge{}", index + 1));
            assert!(before < member && member < after, "member {index}");
        }
    }

    #[test]
    fn the_client_frame_runs_in_table_order() {
        let mut app = App::new();
        configure_client_sets(&mut app);
        for set in CLIENT_TOC {
            probe(&mut app, Update, set.clone(), client_set_name(set));
        }
        let ran = run(&mut app, Update);
        let expected: Vec<&str> = CLIENT_TOC.iter().map(client_set_name).collect();
        assert_eq!(ran, expected);
    }

    #[test]
    fn client_edges_bracket_their_member() {
        let mut app = App::new();
        configure_client_sets(&mut app);
        for (index, set) in (0u8..).zip(CLIENT_TOC) {
            probe(&mut app, Update, set.clone(), format!("set{index}"));
        }
        for index in 0..=CLIENT_TOC.len() as u8 {
            probe(&mut app, Update, ClientEdge(index), format!("edge{index}"));
        }
        let ran = run(&mut app, Update);
        for index in 0..CLIENT_TOC.len() {
            let before = position(&ran, &format!("edge{index}"));
            let member = position(&ran, &format!("set{index}"));
            let after = position(&ran, &format!("edge{}", index + 1));
            assert!(before < member && member < after, "member {index}");
        }
    }

    #[test]
    fn effect_publication_belongs_to_the_effects_stage() {
        let mut app = App::new();
        configure_client_sets(&mut app);
        probe(&mut app, Update, ClientSet::Ui, "ui");
        probe(&mut app, Update, FxSoundPublished, "published");
        probe(&mut app, Update, ClientSet::Diag, "diag");
        let ran = run(&mut app, Update);
        assert!(position(&ran, "ui") < position(&ran, "published"));
        assert!(position(&ran, "published") < position(&ran, "diag"));
    }

    #[test]
    fn worker_commands_run_inside_present_and_honour_their_after_edges() {
        let mut app = App::new();
        configure_client_sets(&mut app);
        probe(&mut app, Update, ClientSet::Send, "send");
        probe(&mut app, Update, ClientSet::Ui, "ui");
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
        for &(later, earlier) in WORKER_CMD_AFTER {
            assert!(
                position(&ran, worker_cmd_name(earlier)) < position(&ran, worker_cmd_name(later)),
                "{later:?} must follow {earlier:?}"
            );
        }
    }

    #[test]
    fn lighting_is_seated_as_part_of_adding_scene_entities() {
        let mut app = App::new();
        configure_client_sets(&mut app);
        probe(&mut app, Update, ModelLightingSeated, "lighting");
        app.add_systems(
            Update,
            (|mut order: ResMut<Order>| order.0.push("after_add".into()))
                .after(WorkerCmdSet::AddSceneEnt),
        );
        probe(&mut app, Update, ClientSet::Ui, "ui");
        let ran = run(&mut app, Update);
        assert!(position(&ran, "lighting") < position(&ran, "after_add"));
        assert!(position(&ran, "lighting") < position(&ran, "ui"));
    }

    #[test]
    fn render_stages_run_in_order_in_both_schedules() {
        for post in [false, true] {
            let mut app = App::new();
            configure_client_sets(&mut app);
            configure_render_sets(&mut app);
            for (index, stage) in (0..).zip(RENDER_ORDER) {
                if post {
                    probe(&mut app, PostUpdate, stage, format!("stage{index}"));
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
        let mut app = App::new();
        configure_client_sets(&mut app);
        configure_render_sets(&mut app);
        probe(&mut app, Update, ClientSet::Send, "send");
        probe(&mut app, Update, ClientSet::Ui, "ui");
        for stage in RENDER_ORDER {
            probe(&mut app, Update, stage, format!("{stage:?}"));
        }
        let ran = run(&mut app, Update);
        for stage in RENDER_ORDER {
            let at = position(&ran, &format!("{stage:?}"));
            assert!(position(&ran, "send") < at && at < position(&ran, "ui"));
        }
    }
}
