//! A plain full-screen text page shown when no real screen is up: the title
//! and a list of status lines, kept current from `GapHud`.

use assets::LoadingScreen;
use bevy::prelude::*;
use bevy::text::FontSize;
use frame::{AppScreen, ClientSet};

use crate::layers::{GameUiFont, UiLayer, UiLayerVisibility, game_text_font};

/// Text of the status page.
#[derive(Resource, Clone, Debug)]
pub struct GapHud {
    pub title: String,
    pub body: Vec<String>,
}

impl Default for GapHud {
    fn default() -> Self {
        Self {
            title: "bo2mp".to_owned(),
            body: vec!["waiting for launch status…".to_owned()],
        }
    }
}

#[derive(Component)]
struct GapHudRoot;

#[derive(Component)]
struct GapTitleText;

#[derive(Component)]
struct GapBodyText;

const TITLE_PX: f32 = 28.0;
const BODY_PX: f32 = 16.0;

/// The page yields to the loading screen and to every real screen.
fn page_wanted(loading: bool, screen: AppScreen) -> bool {
    !loading && !matches!(screen, AppScreen::MainMenu | AppScreen::ClassSelect | AppScreen::InGame)
}

fn text_style(font: Option<&GameUiFont>, size_px: f32) -> TextFont {
    match font {
        Some(font) => game_text_font(&font.0, size_px),
        None => TextFont {
            font_size: FontSize::Px(size_px),
            ..default()
        },
    }
}

fn spawn_gap_ui(
    mut commands: Commands,
    hud: Res<GapHud>,
    font: Option<Res<GameUiFont>>,
    loading: Option<Res<LoadingScreen>>,
    screen: Res<AppScreen>,
    existing: Query<(), With<GapHudRoot>>,
) {
    if !existing.is_empty() || !page_wanted(loading.is_some(), *screen) {
        return;
    }
    let font = font.as_deref();

    commands
        .spawn((
            GapHudRoot,
            UiLayer::Debug,
            UiLayerVisibility,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(24)),
                row_gap: px(8),
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.06, 0.08)),
        ))
        .with_children(|page| {
            page.spawn((
                GapTitleText,
                Text::new(hud.title.clone()),
                text_style(font, TITLE_PX),
                TextColor(Color::srgb(0.9, 0.9, 0.85)),
            ));
            page.spawn((
                GapBodyText,
                Text::new(hud.body.join("\n")),
                text_style(font, BODY_PX),
                TextColor(Color::srgb(0.75, 0.78, 0.7)),
            ));
        });
}

fn refresh_gap_ui(
    hud: Res<GapHud>,
    mut titles: Query<&mut Text, (With<GapTitleText>, Without<GapBodyText>)>,
    mut bodies: Query<&mut Text, (With<GapBodyText>, Without<GapTitleText>)>,
) {
    if !hud.is_changed() {
        return;
    }
    for mut text in &mut titles {
        text.0 = hud.title.clone();
    }
    for mut text in &mut bodies {
        text.0 = hud.body.join("\n");
    }
}

fn despawn_gap_when_hidden(
    mut commands: Commands,
    loading: Option<Res<LoadingScreen>>,
    screen: Res<AppScreen>,
    roots: Query<Entity, With<GapHudRoot>>,
) {
    if page_wanted(loading.is_some(), *screen) {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
}

pub(crate) fn register_gap_hud_systems(app: &mut App) {
    app.init_resource::<GapHud>().add_systems(
        Update,
        (
            spawn_gap_ui,
            refresh_gap_ui,
            despawn_gap_when_hidden.after(spawn_gap_ui),
        )
            .in_set(ClientSet::Ui),
    );
}
