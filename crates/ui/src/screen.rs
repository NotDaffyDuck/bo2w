//! Which interface layers each application screen shows.

use assets::LoadingScreen;
use bevy::prelude::*;
use frame::AppScreen;

use crate::layers::{ApplyUiLayers, UiDraw, UiLayer, UiLayers};

/// The layers to show for a screen. A loading screen in progress wins over
/// whatever the screen says; `ui_draw` turns off the game-facing layers.
pub fn layers_for_screen(screen: AppScreen, ui_draw: bool, loading: bool) -> &'static [UiLayer] {
    use UiLayer::*;
    if loading {
        return &[Loading, Overlay];
    }
    match screen {
        AppScreen::MainMenu => &[Shell, Overlay],
        AppScreen::Loading => &[Loading, Overlay],
        AppScreen::ClassSelect if ui_draw => &[Hud, Overlay],
        AppScreen::InGame if ui_draw => &[Hud, Debug, Overlay],
        AppScreen::ClassSelect | AppScreen::InGame => &[Overlay],
    }
}

pub fn sync_ui_layers(
    screen: Res<AppScreen>,
    draw: Res<UiDraw>,
    loading: Option<Res<LoadingScreen>>,
    mut layers: ResMut<UiLayers>,
) {
    let wanted = layers_for_screen(*screen, draw.0, loading.is_some());
    layers.show_only(wanted.iter().copied());
}

pub(crate) fn register_screen_systems(app: &mut App) {
    app.init_resource::<AppScreen>()
        .add_systems(Startup, sync_ui_layers)
        .add_systems(PostUpdate, sync_ui_layers.before(ApplyUiLayers));
}
