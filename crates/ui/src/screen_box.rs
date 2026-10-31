// A box in screen pixels, measured from the top-left corner, and the node that covers it.

use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;
use frame::ClientStep;

pub(crate) struct ScreenBox {
    pub left_px: f32,
    pub top_px: f32,
    pub width_px: f32,
    pub height_px: f32,
}

impl ScreenBox {
    /// An absolutely placed node over the box; every other layout field keeps its default.
    pub(crate) fn node(self) -> Node {
        let mut placed = absolute_node();
        [placed.left, placed.top, placed.width, placed.height] =
            [self.left_px, self.top_px, self.width_px, self.height_px].map(Val::Px);
        placed
    }
}

/// An absolutely placed node; every other layout field keeps its default.
pub(crate) fn absolute_node() -> Node {
    let mut placed = Node::DEFAULT;
    placed.position_type = PositionType::Absolute;
    placed
}

/// An absolutely placed node over the whole parent.
pub(crate) fn covering_node() -> Node {
    let mut cover = absolute_node();
    (cover.width, cover.height) = (Val::Percent(100.0), Val::Percent(100.0));
    cover
}

/// An absolutely placed node whose top-left corner sits at this pixel; its size follows its content.
pub(crate) fn pinned_node(left_px: f32, top_px: f32) -> Node {
    let mut pinned = absolute_node();
    (pinned.left, pinned.top) = (Val::Px(left_px), Val::Px(top_px));
    pinned
}

/// The game's main window, read by the HUD systems that size themselves to the screen.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MainWindow<'w, 's> {
    shown: Query<'w, 's, (&'static Window, &'static bevy::window::PrimaryWindow)>,
}

impl MainWindow<'_, '_> {
    /// The main window, while there is exactly one.
    pub(crate) fn shown(&self) -> Option<&Window> {
        self.shown.single().ok().map(|(shown, _)| shown)
    }
}

/// The game's main window, for the systems that change it.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MainWindowMut<'w, 's> {
    shown: Query<'w, 's, (&'static mut Window, &'static bevy::window::PrimaryWindow)>,
}

impl MainWindowMut<'_, '_> {
    /// The main window, while there is exactly one.
    pub(crate) fn shown_mut(&mut self) -> Option<Mut<'_, Window>> {
        self.shown.single_mut().ok().map(|(shown, _)| shown)
    }
}

/// The world cameras the HUD projects its markers through.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct SceneCameras<'w, 's> {
    views: Query<'w, 's, (&'static Camera, &'static GlobalTransform, &'static Projection, &'static Camera3d)>,
}

/// The world cameras the HUD is composited over (not the interface's own camera).
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CompositedWorldCameras<'w, 's> {
    under_hud: Query<'w, 's, Entity, (With<Camera3d>, With<bevy::camera::CompositingSpace>, Without<crate::UiCamera>)>,
}

impl CompositedWorldCameras<'_, '_> {
    /// The first of them, while there is one.
    pub(crate) fn first(&self) -> Option<Entity> {
        self.under_hud.iter().next()
    }
}

/// The world camera drawing this frame: what it is, where it sits and how it projects.
pub(crate) struct ActiveView<'a> {
    pub camera: &'a Camera,
    pub at: &'a GlobalTransform,
    pub lens: &'a Projection,
}

impl SceneCameras<'_, '_> {
    /// The first world camera that is drawing.
    pub(crate) fn active(&self) -> Option<ActiveView<'_>> {
        let (camera, at, lens, _) = self.views.iter().find(|(camera, ..)| camera.is_active)?;
        Some(ActiveView { camera, at, lens })
    }
}

/// Runs these systems every frame, with the rest of the interface.
pub(crate) fn every_ui_frame<M>(app: &mut App, systems: impl IntoScheduleConfigs<ScheduleSystem, M>) {
    app.add_systems(Update, systems.in_set(ClientStep::Ui));
}
