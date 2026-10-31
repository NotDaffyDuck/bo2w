use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::Node;
use bevy::ui::PositionType;
use bevy::ui::UiTargetCamera;
use frame::LifeStatePosted;
use frame::ShellScreen;
use frame::StartupPersona;
use frame::StartupReport;
use frame::SpawnLifeBegun;
use frame::MenuPaintPass;
use net::ClientStep;
use net::CardSlotCommand;
use net::MenuOpenCommand;
use net::FrameUpdateTally;

use crate::flash::WhiteoutJob;
use crate::flash::WhiteoutCache;
use crate::flash::drive_whiteout;
use crate::gaps::HudGapBook;
use crate::gaps::log_hud_shortages;
use crate::draw_batch;
use crate::draw_batch::ListLatch;
use crate::draw_batch::OverlayMeshBatch;
use crate::draw_batch::LayerRequests;
use crate::draw_batch::LayerJob;
use crate::hud_elems::{
    HudElemsBackRaster, HudElemsRaster, spawn_hud_elems, spawn_hud_elems_back, update_hud_elems,
};
use crate::images::HudPictures;
use crate::iris::ScopeStripFill;
use crate::iris::spawn_scope;
use crate::iris::update_scope;
use crate::name_tags::OverheadNamesRaster;
use crate::aim_mark::CrosshairScopeHold;
use crate::surface::{FlatOverlayCanvas, refresh_hud_surface};
use crate::aim_mark::spawn_aim_mark;
use bevy::prelude::Visibility;

/// The root of every HUD node except the scope picture.
#[derive(Component)]
struct MainHudRoot;

/// `Some(true)` while the HUD roots are shown, `Some(false)` while hidden;
/// `None` before the first visibility pass.
#[derive(Resource, Default, Clone, Debug)]
struct RootsShown {
    last_pass: Option<bool>,
}

/// The scope picture and its black surround. Black Ops II draws them on the
/// match world's frame, under the match HUD (score panel, ammo and killfeed
/// stay visible over the surround), so they live in a root of their own on
/// the world camera, below the LUI HUD root (`GlobalZIndex(0)`), and not on
/// the top UI camera with the rest of this crate's nodes.
#[derive(Component)]
struct ScopeRoot;

/// Where the HUD's frame timers last stopped: `since_open` is the start of the
/// present stage, `since_layer` the end of the last HUD stage.
#[derive(Resource, Default)]
struct HudLaps {
    since_open: Option<Instant>,
    since_layer: Option<Instant>,
}

#[derive(Component)]
struct MenuBackdropCamera;

pub struct OverlayModule;

impl bevy::app::Plugin for OverlayModule {
    fn build(&self, app: &mut App) {
        // Starts the millisecond clock now, so the first HUD frame does not
        // read a clock that began late.
        let _ = crate::clock::milliseconds();

        let steps: [fn(&mut App); 8] = [
            draw_batch::install,
            add_hud_state,
            frame::register_ui_contracts,
            crate::name_tags::register,
            crate::rangefinder::install_rangefinder,
            add_root_systems,
            add_layer_systems,
            add_flush_systems,
        ];
        steps.into_iter().for_each(|step| step(app));
    }
}

fn add_hud_state(app: &mut App) {
    app.add_message::<CardSlotCommand>();
    app.add_message::<MenuOpenCommand>();
    app.init_resource::<FlatOverlayCanvas>();
    app.init_resource::<RootsShown>();
    app.init_resource::<HudLaps>();
    app.init_resource::<HudPictures>();
    app.init_resource::<crate::menus::ScriptMenus>();
    app.init_resource::<HudGapBook>();
    app.init_resource::<CrosshairScopeHold>();
    app.init_resource::<ScopeStripFill>();
    app.init_resource::<WhiteoutCache>();
    app.init_resource::<WhiteoutJob>();
}

/// Setting the roots up: who the HUD is for, what it draws on, and whether it
/// shows at all.
fn add_root_systems(app: &mut App) {
    app.add_message::<SpawnLifeBegun>().add_systems(
        Update,
        (
            lap_open,
            take_games_root,
            take_map_zone_tree,
            prime_hud_pictures,
            ensure_hud_roots,
            aim_scope_root,
            keep_menu_backdrop_camera,
            lap_setup,
            bevy::ecs::schedule::ApplyDeferred,
            lap_deferred,
            show_or_hide_roots,
            lap_visibility,
        )
            .chain()
            .in_set(ClientStep::Ui),
    );
}

/// The HUD layers, in draw order, with a stage timer after each group.
fn add_layer_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            lap_surfaces_open,
            refresh_hud_surface,
            crate::aim_mark::refresh_reticle,
            lap_stage_close::<0>,
            update_scope,
            crate::sight::update_sight,
            lap_stage_close::<1>,
            lap_stage_close::<2>,
            drive_whiteout,
            lap_stage_close::<3>,
            lap_stage_close::<4>,
            lap_stage_close::<5>,
            update_hud_elems,
            crate::menus::track_lui_menus,
            lap_stage_close::<7>,
            log_hud_shortages,
            lap_stage_close::<8>,
            lap_surfaces_close,
        )
            .chain()
            .in_set(LifeStatePosted),
    );
}

/// The tessellated layers are flushed to the GPU list between the last HUD
/// stage and the gap report.
fn add_flush_systems(app: &mut App) {
    app.add_systems(
        Update,
        (
            begin_tess_frame,
            flush_whiteout_tess,
            flush_overhead_names_tess,
            flush_hud_elems_tess,
        )
            .chain()
            .after(lap_stage_close::<7>)
            .before(log_hud_shortages)
            .in_set(LifeStatePosted),
    );
    app.add_systems(
        Update,
        blank_tess_while_hud_hidden
            .after(lap_surfaces_close)
            .in_set(LifeStatePosted),
    );
}

fn hud_root_should_show(shown_screen: ShellScreen, draw_ui: bool) -> bool {
    draw_ui && matches!(shown_screen, ShellScreen::InGame | ShellScreen::KitPicker)
}

/// Whether enough menus are dimming the world that the HUD should stay down.
/// Two or more of the black menu dims stacked (the pause menu, then a popup
/// over it) are applied in the world pass and not drawn as UI (`lui_hud`),
/// so nothing would darken the HUD under them: the hip-fire crosshair and
/// the overhead name skulls showed through the popup's 0.8 panel at full
/// white. The game darkens the HUD by both dims to about nothing and the
/// panel covers the rest, so the HUD stays down while both are up. With a
/// single dim (the pause menu) the dim is a UI node over the HUD root and
/// darkens it like the world.
fn dims_keep_hud_down(world_dim: Option<f32>, menu_dims: Option<u32>) -> bool {
    world_dim.is_some_and(|dim| dim >= 2.0) || menu_dims.is_some_and(|count| count >= 2)
}

/// A frame where the HUD is hidden (and not the main menu) must not leave the
/// last frame's geometry on screen.
fn blank_tess_while_hud_hidden(
    shown: Res<RootsShown>,
    app_screen: Res<ShellScreen>,
    mut latches: Query<&mut ListLatch>,
    mut frame: ResMut<OverlayMeshBatch>,
) {
    if shown.last_pass == Some(true) || *app_screen == ShellScreen::MainMenu {
        return;
    }
    frame.reset_geometry();
    frame.visible &= false;
    latches.iter_mut().for_each(|mut latch| latch.hide());
}

fn begin_tess_frame(surface: Res<FlatOverlayCanvas>, mut frame: ResMut<OverlayMeshBatch>) {
    let _body = draw_batch::BodyTimer::start();
    let ready = surface.has_size();
    *frame = OverlayMeshBatch {
        visible: ready,
        canvas_w: if ready { surface.width() } else { 0.0 },
        canvas_h: if ready { surface.height() } else { 0.0 },
        ..OverlayMeshBatch::default()
    };
}

fn flush_whiteout_tess(
    cache: Res<WhiteoutCache>,
    job: Res<WhiteoutJob>,
    mut frame: ResMut<OverlayMeshBatch>,
) {
    let _body = draw_batch::BodyTimer::start();
    frame.saved_screen_sequence = cache.grab_serial;
    let uploading = matches!(*job, WhiteoutJob::Upload) && !cache.overlay.is_empty();
    if uploading {
        [&cache.grab, &cache.overlay]
            .into_iter()
            .for_each(|packed| frame.append_list(packed));
    }
}

fn take_games_root(mut hud_images: ResMut<HudPictures>, identity: Option<Res<StartupPersona>>) {
    let install = identity
        .as_deref()
        .map(|launch| launch.titles_install_dir.as_path())
        .filter(|root| !root.as_os_str().is_empty());
    if let Some(root) = install {
        hud_images.point_at_install(root);
    }
}

fn take_map_zone_tree(mut hud_images: ResMut<HudPictures>, report: Option<Res<StartupReport>>) {
    if let Some(zone_archive_file) = report.as_deref().and_then(|launch| launch.zone_archive_file.as_ref().ok()) {
        hud_images.add_map_zone(zone_archive_file);
    }
}

fn prime_hud_pictures(mut pictures: ResMut<HudPictures>, mut assets: ResMut<Assets<Image>>) {
    pictures.warm_first_frame(&mut assets);
}

/// What the main-menu backdrop camera is built from.
fn menu_backdrop_camera() -> impl Bundle {
    let behind_everything = Camera {
        order: -1,
        ..default()
    };
    (
        (Camera3d::default(), behind_everything),
        (
            bevy::camera::CompositingSpace::Srgb,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy::render::view::Msaa::Off,
        ),
        (frame::UiCamera, MenuBackdropCamera),
    )
}

/// The main menu draws through a 3D camera of its own; a match brings its
/// own cameras, so this one goes away with the menu.
fn keep_menu_backdrop_camera(
    mut commands: Commands,
    app_screen: Res<ShellScreen>,
    backdrops: Query<Entity, With<MenuBackdropCamera>>,
) {
    if *app_screen == ShellScreen::MainMenu {
        if backdrops.is_empty() {
            commands.spawn(menu_backdrop_camera());
        }
        return;
    }
    for camera in &backdrops {
        commands.entity(camera).despawn();
    }
}

/// A full-window, transparent root node, hidden until the visibility pass.
fn hidden_full_window_root() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0_f32),
            height: Val::Percent(100.0_f32),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0_f32, 0.0, 0.0, 0.0)),
        Visibility::Hidden,
    )
}

fn ensure_hud_roots(mut commands: Commands, existing: Query<(), With<MainHudRoot>>) {
    if existing.iter().next().is_some() {
        return;
    }
    let mut scope = commands.spawn((hidden_full_window_root(), ScopeRoot, GlobalZIndex(-1)));
    scope.with_children(|root| spawn_scope(root));
    let mut main = commands.spawn((hidden_full_window_root(), MainHudRoot));
    main.with_children(|root| {
        let layers: [fn(&mut ChildSpawnerCommands); 4] = [
            |root| crate::glyph_layer::spawn_layer_node(root, OverheadNamesRaster),
            spawn_aim_mark,
            spawn_hud_elems_back,
            spawn_hud_elems,
        ];
        layers.iter().for_each(|spawn| spawn(root));
    });
}

/// The scope root draws on the match world's camera when there is one (as
/// the LUI HUD does), otherwise on the default UI camera.
fn aim_scope_root(
    mut commands: Commands,
    world_cam: Query<
        Entity,
        (
            With<bevy::prelude::Camera3d>,
            With<bevy::camera::CompositingSpace>,
            Without<frame::UiCamera>,
        ),
    >,
    root: Query<(Entity, Option<&UiTargetCamera>), With<ScopeRoot>>,
) {
    let wanted = world_cam.iter().next();
    let stale = root
        .iter()
        .filter(|&(_, current)| current.map(|target| target.0) != wanted);
    for (scope_ent, _) in stale {
        let mut scope = commands.entity(scope_ent);
        if let Some(camera) = wanted {
            scope.insert(UiTargetCamera(camera));
        } else {
            scope.remove::<UiTargetCamera>();
        }
    }
}

fn show_or_hide_roots(
    app_screen: Res<ShellScreen>,
    draw_flag: Option<Res<MenuPaintPass>>,
    world_dim: Option<Res<frame::WorldDim>>,
    menu_dims: Option<Res<frame::MenuDims>>,
    mut shown: ResMut<RootsShown>,
    mut root_vis: Query<&mut Visibility, Or<(With<MainHudRoot>, With<ScopeRoot>)>>,
) {
    let held_down = dims_keep_hud_down(world_dim.map(|dim| dim.0), menu_dims.map(|dims| dims.0));
    let drawing = draw_flag.is_some_and(|draw| draw.0);
    let visible = hud_root_should_show(*app_screen, drawing) && !held_down;
    shown.last_pass = Some(visible);

    let wanted = visible
        .then_some(Visibility::Visible)
        .unwrap_or(Visibility::Hidden);
    root_vis
        .iter_mut()
        .for_each(|mut visibility| {
            visibility.set_if_neq(wanted);
        });
}

/// Milliseconds from `since` to `now`, when there is a `since`.
fn millis_between(since: Option<Instant>, now: Instant) -> Option<f32> {
    since.map(|start| (now - start).as_secs_f32() * 1000.0)
}

/// Milliseconds since the mark was last set (when it was), and sets it to now.
fn lap(mark: &mut Option<Instant>) -> Option<f32> {
    let now = Instant::now();
    millis_between(mark.replace(now), now)
}

fn lap_open(mut laps: ResMut<HudLaps>) {
    laps.since_open = Some(Instant::now());
}

fn lap_setup(mut laps: ResMut<HudLaps>, mut census: ResMut<FrameUpdateTally>) {
    census.overlay_root_prep_millis = lap(&mut laps.since_open);
}

fn lap_deferred(mut laps: ResMut<HudLaps>, mut census: ResMut<FrameUpdateTally>) {
    census.display_deferred_apply_millis = lap(&mut laps.since_open);
}

fn lap_visibility(laps: Res<HudLaps>, mut census: ResMut<FrameUpdateTally>) {
    census.overlay_visibility_millis = millis_between(laps.since_open, Instant::now());
}

fn lap_surfaces_open(mut laps: ResMut<HudLaps>, mut pass: ResMut<LayerRequests>) {
    let now = Some(Instant::now());
    laps.since_open = now;
    laps.since_layer = now;
    *pass = LayerRequests::default();
}

fn lap_stage_close<const N: usize>(
    mut laps: ResMut<HudLaps>,
    mut phases: ResMut<FrameUpdateTally>,
) {
    let spent = lap(&mut laps.since_layer);
    phases.hud_stage_millis[N] = spent.or(phases.hud_stage_millis[N]);
}

fn lap_surfaces_close(laps: Res<HudLaps>, mut census: ResMut<FrameUpdateTally>) {
    census.overlay_surface_millis = millis_between(laps.since_open, Instant::now());
    // Two different quantities, published side by side on purpose: the line
    // above is the gap between two systems, and the tess bodies below ran
    // inside part of it. Whatever else the executor put in that gap is the
    // difference, and naming it is the point.
    let (body_ms, jobs) = draw_batch::drain_body_cost();
    census.hud_tess_body_ms = Some(body_ms);
    census.hud_tess_jobs = Some(jobs);
}

/// What a flush system needs to hand one layer's job to the GPU list.
#[derive(SystemParam)]
pub(crate) struct TessSink<'w> {
    surface: Res<'w, FlatOverlayCanvas>,
    hud_images: ResMut<'w, HudPictures>,
    image_assets: ResMut<'w, Assets<Image>>,
    frame: ResMut<'w, OverlayMeshBatch>,
}

impl TessSink<'_> {
    fn deliver(&mut self, job: LayerJob, ui_node: &mut Node, latch: &mut ListLatch) {
        let (width, height) = (self.surface.width(), self.surface.height());
        draw_batch::flush_layer(
            &mut self.frame,
            &mut self.hud_images,
            &mut self.image_assets,
            (width, height),
            ui_node,
            latch,
            job,
        );
    }
}

fn flush_hud_elems_tess(
    mut sink: TessSink,
    mut pass: ResMut<LayerRequests>,
    mut front: Query<
        (&mut Node, &mut ListLatch),
        (With<HudElemsRaster>, Without<HudElemsBackRaster>),
    >,
    mut back: Query<(&mut Node, &mut ListLatch), With<HudElemsBackRaster>>,
) {
    let _body = draw_batch::BodyTimer::start();
    if !sink.surface.has_size() {
        return;
    }
    let front_job = std::mem::take(&mut pass.elems_front);
    let back_job = std::mem::take(&mut pass.elems_behind);
    if let Ok((mut host, mut latch)) = front.single_mut() {
        sink.deliver(front_job, &mut host, &mut latch);
    }
    if let Ok((mut host, mut latch)) = back.single_mut() {
        sink.deliver(back_job, &mut host, &mut latch);
    }
}

pub(crate) fn flush_overhead_names_tess(
    mut sink: TessSink,
    mut pass: ResMut<LayerRequests>,
    mut layer: Query<(&mut Node, &mut ListLatch), With<OverheadNamesRaster>>,
) {
    let _body = draw_batch::BodyTimer::start();
    if !sink.surface.has_size() {
        return;
    }
    let job = std::mem::take(&mut pass.names);
    if let Ok((mut host, mut latch)) = layer.single_mut() {
        sink.deliver(job, &mut host, &mut latch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_hud_shows_only_in_game_screens_with_ui_drawing_on() {
        assert!(hud_root_should_show(ShellScreen::InGame, true));
        assert!(hud_root_should_show(ShellScreen::KitPicker, true));
        assert!(!hud_root_should_show(ShellScreen::InGame, false));
        assert!(!hud_root_should_show(ShellScreen::MainMenu, true));
    }

    #[test]
    fn one_dim_leaves_the_hud_up_and_two_put_it_down() {
        assert!(!dims_keep_hud_down(None, None));
        assert!(!dims_keep_hud_down(Some(1.0), Some(1)));
        assert!(dims_keep_hud_down(Some(2.0), None));
        assert!(dims_keep_hud_down(None, Some(2)));
    }

    #[test]
    fn millis_between_needs_a_start_mark() {
        let now = Instant::now();
        assert_eq!(millis_between(None, now), None);
        let earlier = now - Duration::from_millis(250);
        let spent = millis_between(Some(earlier), now).expect("a mark was set");
        assert!((spent - 250.0).abs() < 1.0);
    }

    #[test]
    fn a_lap_reports_the_gap_and_restarts_the_mark() {
        let mut mark = None;
        assert_eq!(lap(&mut mark), None);
        assert!(mark.is_some());
        let first = mark;
        assert!(lap(&mut mark).is_some());
        assert!(mark >= first);
    }
}
