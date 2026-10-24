use std::time::Instant;

use bevy::prelude::*;
use frame::{AppScreen, LaunchIdentity, LaunchReport, LifeFrontPublished, LifeStarted, UiDraw};
use net::{ClientSet, UpdatePhaseCensus};

use crate::flash::{FlashGpuJob, FlashWhiteoutLatch, update_flash_whiteout};
use crate::gaps::{HudPresentationGaps, report_hud_gaps};
use crate::gpu_list::{self, GpuListLatch, HudTessGpuFrame, HudTessPass};
use crate::hud_elems::{
    HudElemsBackRaster, HudElemsRaster, spawn_hud_elems, spawn_hud_elems_back, update_hud_elems,
};
use crate::images::HudImages;
use crate::iris::{IrisLetterboxFill, spawn_iris, update_iris};
use crate::overhead_names::OverheadNamesRaster;
use crate::reticle::{ReticleAdsLatch, spawn_reticle, update_reticle};
use crate::surface::Hud2dSurface;

/// The root of every HUD node except the scope picture.
#[derive(Component)]
struct HudRoot;

/// `Some(1)` while the HUD roots are shown, `Some(0)` while hidden; `None`
/// before the first visibility pass.
#[derive(Resource, Default, Clone, Debug)]
pub(crate) struct HudRootVisible(pub Option<i32>);

/// The scope picture and its black surround. Black Ops II draws them on the
/// match world's frame, under the match HUD (score panel, ammo and killfeed
/// stay visible over the surround), so they live in a root of their own on
/// the world camera, below the LUI HUD root (`GlobalZIndex(0)`), and not on
/// the top UI camera with the rest of this crate's nodes.
#[derive(Component)]
struct HudIrisRoot;

/// Stamps the start of the present stage, to time the stages after it.
#[derive(Resource, Default)]
struct HudPresentStamp(Option<Instant>);

/// Stamps the end of the last HUD stage, to time the next one.
#[derive(Resource, Default)]
struct HudStageStamp(Option<Instant>);

#[derive(Component)]
struct FrontendCamera;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        gpu_list::register(app);
        // Starts the millisecond clock now, so the first HUD frame does not
        // read a clock that began late.
        let _ = crate::clock::milliseconds();

        app.init_resource::<HudImages>()
            .init_resource::<HudPresentationGaps>()
            .init_resource::<ReticleAdsLatch>()
            .init_resource::<IrisLetterboxFill>()
            .init_resource::<FlashWhiteoutLatch>()
            .init_resource::<FlashGpuJob>()
            .init_resource::<HudRootVisible>()
            .init_resource::<Hud2dSurface>()
            .init_resource::<HudPresentStamp>()
            .init_resource::<HudStageStamp>()
            .init_resource::<crate::menus::ScriptMenus>()
            .add_message::<net::SvcCardSlotCmd>()
            .add_message::<net::SvcOpenMenuCmd>();

        frame::register_ui_contracts(app);
        crate::overhead_names::register(app);
        crate::rangefinder::register(app);

        // Setting the roots up: who the HUD is for, what it draws on, and
        // whether it shows at all.
        app.add_message::<LifeStarted>().add_systems(
            Update,
            (
                hud_stamp_open,
                sync_games_root,
                sync_map_zone_tree,
                warm_hud_images,
                ensure_hud_root,
                retarget_iris_root,
                sync_frontend_camera,
                hud_stamp_setup,
                ApplyDeferred,
                hud_stamp_deferred,
                sync_hud_visibility,
                hud_stamp_visibility,
            )
                .chain()
                .in_set(ClientSet::Ui),
        );

        // The HUD layers, in draw order, with a stage timer after each group.
        app.add_systems(
            Update,
            (
                hud_surfaces_open,
                crate::surface::update_hud_surface,
                update_reticle,
                hud_stage_close::<0>,
                update_iris,
                crate::sight::update_sight,
                hud_stage_close::<1>,
                hud_stage_close::<2>,
                update_flash_whiteout,
                hud_stage_close::<3>,
                hud_stage_close::<4>,
                hud_stage_close::<5>,
                update_hud_elems,
                crate::menus::track_lui_menus,
                hud_stage_close::<7>,
                report_hud_gaps,
                hud_stage_close::<8>,
                hud_surfaces_close,
            )
                .chain()
                .in_set(LifeFrontPublished),
        );

        // The tessellated layers are flushed to the GPU list between the last
        // HUD stage and the gap report.
        app.add_systems(
            Update,
            (
                begin_hud_tess_frame,
                flush_flash_tess,
                flush_overhead_names_tess,
                flush_hud_elems_tess,
            )
                .chain()
                .after(hud_stage_close::<7>)
                .before(report_hud_gaps)
                .in_set(LifeFrontPublished),
        );
        app.add_systems(
            Update,
            hide_tess_when_hud_hidden
                .after(hud_surfaces_close)
                .in_set(LifeFrontPublished),
        );
    }
}

fn hud_root_should_show(screen: AppScreen, ui_draw: bool) -> bool {
    ui_draw && matches!(screen, AppScreen::InGame | AppScreen::ClassSelect)
}

/// A frame where the HUD is hidden (and not the main menu) must not leave the
/// last frame's geometry on screen.
fn hide_tess_when_hud_hidden(
    root: Res<HudRootVisible>,
    screen: Res<AppScreen>,
    mut frame: ResMut<HudTessGpuFrame>,
    mut latches: Query<&mut GpuListLatch>,
) {
    if root.0 == Some(1) || *screen == AppScreen::MainMenu {
        return;
    }
    frame.clear_geometry();
    frame.visible = false;
    latches.iter_mut().for_each(|mut latch| latch.mark_hidden());
}

fn begin_hud_tess_frame(surface: Res<Hud2dSurface>, mut frame: ResMut<HudTessGpuFrame>) {
    let _body = gpu_list::TessBody::open();
    *frame = HudTessGpuFrame::default();
    if surface.is_ready() {
        frame.surface_w = surface.width();
        frame.surface_h = surface.height();
        frame.visible = true;
    }
}

fn flush_flash_tess(
    latch: Res<FlashWhiteoutLatch>,
    job: Res<FlashGpuJob>,
    mut frame: ResMut<HudTessGpuFrame>,
) {
    let _body = gpu_list::TessBody::open();
    frame.saved_screen_sequence = latch.save_sequence;
    if matches!(*job, FlashGpuJob::Write) && !latch.packed.is_empty() {
        frame.append_packed(&latch.saved);
        frame.append_packed(&latch.packed);
    }
}

fn sync_games_root(mut hud_images: ResMut<HudImages>, identity: Option<Res<LaunchIdentity>>) {
    let Some(identity) = identity else {
        return;
    };
    if !identity.games_root.as_os_str().is_empty() {
        hud_images.set_games_root(&identity.games_root);
    }
}

fn sync_map_zone_tree(mut hud_images: ResMut<HudImages>, report: Option<Res<LaunchReport>>) {
    if let Some(Ok(zone_ff)) = report.as_ref().map(|report| &report.zone_ff) {
        hud_images.adopt_map_zone(zone_ff);
    }
}

fn warm_hud_images(mut hud_images: ResMut<HudImages>, mut images: ResMut<Assets<Image>>) {
    hud_images.warm_present_stems(&mut images);
}

/// The main menu draws through a 3D camera of its own; a match brings its
/// own cameras, so this one goes away with the menu.
fn sync_frontend_camera(
    mut commands: Commands,
    screen: Res<AppScreen>,
    cameras: Query<Entity, With<FrontendCamera>>,
) {
    if *screen != AppScreen::MainMenu {
        for camera in &cameras {
            commands.entity(camera).despawn();
        }
        return;
    }
    if !cameras.is_empty() {
        return;
    }
    commands.spawn((
        FrontendCamera,
        Camera3d::default(),
        bevy::camera::CompositingSpace::Srgb,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Camera {
            order: -1,
            ..default()
        },
        bevy::render::view::Msaa::Off,
        frame::UiCamera,
    ));
}

/// A full-window, transparent root node, hidden until the visibility pass.
fn root_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    }
}

fn ensure_hud_root(mut commands: Commands, existing: Query<Entity, With<HudRoot>>) {
    if !existing.is_empty() {
        return;
    }
    commands
        .spawn((
            HudIrisRoot,
            root_node(),
            BackgroundColor(Color::NONE),
            GlobalZIndex(-1),
            Visibility::Hidden,
        ))
        .with_children(|root| spawn_iris(root));
    commands
        .spawn((
            HudRoot,
            root_node(),
            BackgroundColor(Color::NONE),
            Visibility::Hidden,
        ))
        .with_children(|root| {
            crate::font_overlay::spawn_overlay(root, OverheadNamesRaster);
            spawn_reticle(root);
            spawn_hud_elems_back(root);
            spawn_hud_elems(root);
        });
}

/// The scope root draws on the match world's camera when there is one (as
/// the LUI HUD does), otherwise on the default UI camera.
fn retarget_iris_root(
    mut commands: Commands,
    world_cam: Query<
        Entity,
        (
            With<Camera3d>,
            With<bevy::camera::CompositingSpace>,
            Without<frame::UiCamera>,
        ),
    >,
    root: Query<(Entity, Option<&UiTargetCamera>), With<HudIrisRoot>>,
) {
    let wanted = world_cam.iter().next();
    for (entity, current) in &root {
        if wanted == current.map(|target| target.0) {
            continue;
        }
        let mut root = commands.entity(entity);
        match wanted {
            Some(camera) => root.insert(UiTargetCamera(camera)),
            None => root.remove::<UiTargetCamera>(),
        };
    }
}

fn sync_hud_visibility(
    screen: Res<AppScreen>,
    ui_draw: Option<Res<UiDraw>>,
    world_dim: Option<Res<frame::WorldDim>>,
    menu_dims: Option<Res<frame::MenuDims>>,
    mut roots: Query<&mut Visibility, Or<(With<HudRoot>, With<HudIrisRoot>)>>,
    mut visible: ResMut<HudRootVisible>,
) {
    // Two or more of the black menu dims stacked (the pause menu, then a popup
    // over it) are applied in the world pass and not drawn as UI (`lui_hud`),
    // so nothing would darken the HUD under them: the hip-fire crosshair and
    // the overhead name skulls showed through the popup's 0.8 panel at full
    // white. The game darkens the HUD by both dims to about nothing and the
    // panel covers the rest, so the HUD stays down while both are up. With a
    // single dim (the pause menu) the dim is a UI node over the HUD root and
    // darkens it like the world.
    let stacked_dims = world_dim.is_some_and(|dim| dim.0 >= 2.0)
        || menu_dims.is_some_and(|dims| dims.0 >= 2);
    let show = hud_root_should_show(*screen, ui_draw.is_some_and(|draw| draw.0)) && !stacked_dims;
    visible.0 = Some(i32::from(show));

    let wanted = if show {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut roots {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Milliseconds from `since` to `now`, when there is a `since`.
fn millis_between(since: Option<Instant>, now: Instant) -> Option<f32> {
    since.map(|start| (now - start).as_secs_f32() * 1000.0)
}

fn hud_stamp_open(mut stamp: ResMut<HudPresentStamp>) {
    stamp.0 = Some(Instant::now());
}

fn hud_stamp_setup(mut stamp: ResMut<HudPresentStamp>, mut census: ResMut<UpdatePhaseCensus>) {
    let now = Instant::now();
    census.hud_root_setup_ms = millis_between(stamp.0, now);
    stamp.0 = Some(now);
}

fn hud_stamp_deferred(mut stamp: ResMut<HudPresentStamp>, mut census: ResMut<UpdatePhaseCensus>) {
    let now = Instant::now();
    census.present_apply_deferred_ms = millis_between(stamp.0, now);
    stamp.0 = Some(now);
}

fn hud_stamp_visibility(stamp: Res<HudPresentStamp>, mut census: ResMut<UpdatePhaseCensus>) {
    census.hud_visibility_ms = millis_between(stamp.0, Instant::now());
}

fn hud_surfaces_open(
    mut stamp: ResMut<HudPresentStamp>,
    mut stages: ResMut<HudStageStamp>,
    mut pass: ResMut<HudTessPass>,
) {
    let now = Instant::now();
    stamp.0 = Some(now);
    stages.0 = Some(now);
    *pass = HudTessPass::default();
}

fn hud_stage_close<const N: usize>(
    mut stages: ResMut<HudStageStamp>,
    mut census: ResMut<UpdatePhaseCensus>,
) {
    let now = Instant::now();
    if let Some(started) = stages.0 {
        census.hud_stage_ms[N] = millis_between(Some(started), now);
    }
    stages.0 = Some(now);
}

fn hud_surfaces_close(stamp: Res<HudPresentStamp>, mut census: ResMut<UpdatePhaseCensus>) {
    census.hud_surfaces_ms = millis_between(stamp.0, Instant::now());
    // Two different quantities, published side by side on purpose: the line
    // above is the gap between two systems, and the tess bodies below ran
    // inside part of it. Whatever else the executor put in that gap is the
    // difference, and naming it is the point.
    let (body_ms, jobs) = gpu_list::take_tess_body_cost();
    census.hud_tess_body_ms = Some(body_ms);
    census.hud_tess_jobs = Some(jobs);
}

fn flush_hud_elems_tess(
    surface: Res<Hud2dSurface>,
    mut pass: ResMut<HudTessPass>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<HudTessGpuFrame>,
    mut front: Query<
        (Entity, &mut Node, &mut GpuListLatch),
        (With<HudElemsRaster>, Without<HudElemsBackRaster>),
    >,
    mut back: Query<(Entity, &mut Node, &mut GpuListLatch), With<HudElemsBackRaster>>,
) {
    let _body = gpu_list::TessBody::open();
    if !surface.is_ready() {
        return;
    }
    let jobs = [
        (std::mem::take(&mut pass.hud_elems), front.single_mut()),
        (std::mem::take(&mut pass.hud_elems_back), back.single_mut()),
    ];
    for (job, layer) in jobs {
        let Ok((_, mut host, mut latch)) = layer else {
            continue;
        };
        gpu_list::apply_tess_job(
            job,
            &mut host,
            &mut latch,
            &mut hud_images,
            &mut images,
            &mut frame,
            surface.width(),
            surface.height(),
        );
    }
}

pub(crate) fn flush_overhead_names_tess(
    surface: Res<Hud2dSurface>,
    mut pass: ResMut<HudTessPass>,
    mut hud_images: ResMut<HudImages>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<HudTessGpuFrame>,
    mut layer: Query<(Entity, &mut Node, &mut GpuListLatch), With<OverheadNamesRaster>>,
) {
    let _body = gpu_list::TessBody::open();
    if !surface.is_ready() {
        return;
    }
    let job = std::mem::take(&mut pass.overhead_names);
    let Ok((_, mut host, mut latch)) = layer.single_mut() else {
        return;
    };
    gpu_list::apply_tess_job(
        job,
        &mut host,
        &mut latch,
        &mut hud_images,
        &mut images,
        &mut frame,
        surface.width(),
        surface.height(),
    );
}
