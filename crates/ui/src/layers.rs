//! The interface is drawn in layers (in-game HUD, debug text, menus, the
//! loading screen, overlays). Exactly which layers are showing is one
//! resource; a system then pushes that onto every root that carries a
//! `UiLayer`.

use std::time::Instant;

use bevy::{camera::visibility::VisibilitySystems, prelude::*, text::FontSize, ui::UiSystems};

const OXANIUM_TTF: &[u8] = include_bytes!("../assets/Oxanium-Regular.ttf");

/// The font every game text is set in.
#[derive(Resource, Clone, Debug)]
pub struct GameUiFont(pub Handle<Font>);

pub(crate) struct GameUiFontPlugin;

impl Plugin for GameUiFontPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, install_game_font);
    }
}

fn install_game_font(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let handle = fonts.add(Font::from_bytes(OXANIUM_TTF.to_vec()));
    commands.insert_resource(GameUiFont(handle));
}

/// Text style in the game font at a pixel size.
pub fn game_text_font(font: &Handle<Font>, size_px: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size_px),
        ..default()
    }
}

/// Which layer a root node (or camera) belongs to.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiLayer {
    Hud,
    Debug,
    Shell,
    Loading,
    Overlay,
}

impl UiLayer {
    const COUNT: usize = 5;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Marks a layer root whose `Visibility` should follow the layer as well as
/// its `Node.display`.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct UiLayerVisibility;

pub use frame::UiDraw;
pub use frame::UiCamera;

/// Which layers are showing.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct UiLayers {
    visible: [bool; UiLayer::COUNT],
}

impl UiLayers {
    /// Shows these layers and hides all others. Does not touch the resource
    /// when the result would be the same.
    pub fn show_only(&mut self, layers: impl IntoIterator<Item = UiLayer>) {
        let mut wanted = [false; UiLayer::COUNT];
        for layer in layers {
            wanted[layer.index()] = true;
        }
        if self.visible != wanted {
            self.visible = wanted;
        }
    }

    pub fn is_visible(&self, layer: UiLayer) -> bool {
        self.visible[layer.index()]
    }
}

/// Runs the layer systems after the screen has decided what to show and
/// before layout and visibility are worked out.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApplyUiLayers;

/// How long the layer bookkeeping took last frame.
#[derive(Resource, Debug, Default, Clone)]
pub struct UiLayerCpu {
    pub ms: Option<f32>,
    started: Option<Instant>,
}

fn start_layer_clock(mut cpu: ResMut<UiLayerCpu>) {
    cpu.started = Some(Instant::now());
}

fn stop_layer_clock(mut cpu: ResMut<UiLayerCpu>) {
    if let Some(started) = cpu.started {
        cpu.ms = Some(started.elapsed().as_secs_f32() * 1000.0);
    }
}

pub(crate) struct UiLayersPlugin;

impl Plugin for UiLayersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayers>()
            .init_resource::<UiDraw>()
            .init_resource::<UiLayerCpu>()
            .configure_sets(
                PostUpdate,
                ApplyUiLayers
                    .before(UiSystems::Prepare)
                    .before(VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(
                PostUpdate,
                (
                    start_layer_clock.before(crate::screen::sync_ui_layers),
                    apply_ui_layers.in_set(ApplyUiLayers),
                    stop_layer_clock
                        .after(apply_ui_layers)
                        .before(UiSystems::Prepare),
                    pick_ui_camera
                        .after(ApplyUiLayers)
                        .before(UiSystems::Prepare),
                ),
            );
    }
}

/// Copies the layer state onto each layer root: node display, optional
/// visibility, and camera activity. Components are written only when they
/// would change.
#[allow(clippy::type_complexity)]
fn apply_ui_layers(
    layers: Res<UiLayers>,
    mut roots: Query<(
        &UiLayer,
        Option<&mut Node>,
        Option<&mut Visibility>,
        Has<UiLayerVisibility>,
        Option<&mut Camera>,
    )>,
) {
    for (layer, node, visibility, follows_visibility, camera) in &mut roots {
        let shown = layers.is_visible(*layer);

        if let Some(mut node) = node {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
        if follows_visibility && let Some(mut visibility) = visibility {
            let state = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != state {
                *visibility = state;
            }
        }
        if let Some(mut camera) = camera
            && camera.is_active != shown
        {
            camera.is_active = shown;
        }
    }
}

/// Of the active UI cameras, the one drawn last (highest order, newest on a
/// tie) is the one UI nodes attach to by default.
fn pick_ui_camera(
    mut commands: Commands,
    cameras: Query<(Entity, &Camera, Has<IsDefaultUiCamera>), With<UiCamera>>,
) {
    let winner = cameras
        .iter()
        .filter(|(_, camera, _)| camera.is_active)
        .max_by_key(|(entity, camera, _)| (camera.order, entity.to_bits()))
        .map(|(entity, ..)| entity);

    for (entity, _, is_default) in &cameras {
        let should_be = winner == Some(entity);
        if should_be && !is_default {
            commands.entity(entity).insert(IsDefaultUiCamera);
        } else if !should_be && is_default {
            commands.entity(entity).remove::<IsDefaultUiCamera>();
        }
    }
}
