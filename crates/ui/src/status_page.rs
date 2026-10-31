//! A plain full-screen text page shown when no real screen is up: the title
//! and a list of status lines, kept current from `InterimOverlay`.

use assets::LoadBackdrop;
use bevy::prelude::*;
use bevy::text::FontSize;
use frame::{ShellScreen, ClientStep};

use crate::tiers::{MenuTypeface, OverlayTier, OverlayStackShown, match_typeface};

/// Text of the status page.
#[derive(Resource, Clone, Debug)]
pub struct InterimOverlay {
    pub heading: String,
    pub rows: Vec<String>,
}

impl Default for InterimOverlay {
    fn default() -> Self {
        let rows = vec!["waiting for launch status…".to_owned()];
        Self {
            rows,
            heading: String::from("bo2mp"),
        }
    }
}

#[derive(Component)]
struct InterimOverlayAnchor;

/// Which of the page's two text lines an entity is.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum GapLine {
    Title,
    Body,
}

const TITLE_PX: f32 = 28.0;
const BODY_PX: f32 = 16.0;
const PAGE_BG: Color = Color::srgb(0.05, 0.06, 0.08);
const TITLE_RGB: Color = Color::srgb(0.9, 0.9, 0.85);
const BODY_RGB: Color = Color::srgb(0.75, 0.78, 0.7);

impl InterimOverlay {
    /// The status lines as one block of text, one line per entry.
    fn body_block(&self) -> String {
        self.rows.join(
            "
",
        )
    }
}

/// Whether the placeholder page should be on screen. Only the `Loading`
/// app screen can show it, and only while no loading-screen resource exists.
fn page_wanted(loading: bool, screen: ShellScreen) -> bool {
    match screen {
        ShellScreen::Loading => !loading,
        ShellScreen::MainMenu | ShellScreen::KitPicker | ShellScreen::InGame => false,
    }
}

/// Font for a line of the page: the game font when it is loaded, else the
/// default font at the same size.
fn text_style(font: Option<&MenuTypeface>, points: f32) -> TextFont {
    font.map_or_else(
        || TextFont {
            font_size: FontSize::Px(points),
            ..default()
        },
        |game| match_typeface(&game.0, points),
    )
}

/// Builds the page once: a full-screen column holding the title and the body.
fn build_interim_overlay(
    existing: Query<(), With<InterimOverlayAnchor>>,
    shown: Res<ShellScreen>,
    mut commands: Commands,
    font: Option<Res<MenuTypeface>>,
    page: Res<InterimOverlay>,
    loading: Option<Res<LoadBackdrop>>,
) {
    let wanted = page_wanted(loading.is_some(), *shown);
    if !wanted || !existing.is_empty() {
        return;
    }

    let font = font.as_deref();
    let mut column = Node {
        width: percent(100),
        height: percent(100),
        padding: UiRect::all(px(24)),
        row_gap: px(8),
        ..default()
    };
    column.flex_direction = FlexDirection::Column;
    let root = commands
        .spawn((
            InterimOverlayAnchor,
            OverlayTier::Debug,
            OverlayStackShown,
            column,
            BackgroundColor(PAGE_BG),
        ))
        .id();

    let line = |kind: GapLine, words: String, size: f32, tint: Color| {
        (kind, Text::new(words), text_style(font, size), TextColor(tint), ChildOf(root))
    };
    commands.spawn(line(GapLine::Title, page.heading.clone(), TITLE_PX, TITLE_RGB));
    commands.spawn(line(GapLine::Body, page.body_block(), BODY_PX, BODY_RGB));
}

/// Copies the current title and body into the page's text whenever `InterimOverlay`
/// changed this frame.
fn redo_interim_overlay(mut lines: Query<(&GapLine, &mut Text)>, page: Res<InterimOverlay>) {
    if !page.is_changed() {
        return;
    }
    let block = page.body_block();
    for (kind, mut text) in &mut lines {
        let source = match kind {
            GapLine::Title => &page.heading,
            GapLine::Body => &block,
        };
        text.0.clone_from(source);
    }
}

/// Removes the page as soon as it is no longer wanted.
fn drop_interim_when_unseen(
    anchors: Query<Entity, With<InterimOverlayAnchor>>,
    shown: Res<ShellScreen>,
    loading: Option<Res<LoadBackdrop>>,
    mut commands: Commands,
) {
    if page_wanted(loading.is_some(), *shown) {
        return;
    }
    for root in &anchors {
        commands.entity(root).despawn();
    }
}

pub(crate) fn install_interim_overlay_steps(host: &mut App) {
    let systems = (
        drop_interim_when_unseen.after(build_interim_overlay),
        redo_interim_overlay,
        build_interim_overlay,
    );
    host.init_resource::<InterimOverlay>();
    host.add_systems(Update, systems.in_set(ClientStep::Ui));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn page_only_on_the_loading_app_screen_without_loading_resource() {
        let every = [ShellScreen::Loading, ShellScreen::MainMenu, ShellScreen::KitPicker, ShellScreen::InGame];
        for screen in every {
            let want = screen == ShellScreen::Loading;
            assert_eq!(page_wanted(false, screen), want, "{screen:?}");
            assert!(
                !page_wanted(true, screen),
                "loading screen wins: {screen:?}"
            );
        }
    }

    #[test]
    fn default_text_is_a_single_waiting_line() {
        let page = InterimOverlay::default();
        assert_eq!(page.heading, "bo2mp");
        assert_eq!(page.rows, vec!["waiting for launch status…".to_owned()]);
    }

    fn world_with(screen: ShellScreen, body: &[&str]) -> World {
        let mut world = World::new();
        world.insert_resource(screen);
        world.insert_resource(InterimOverlay {
            heading: "T".into(),
            rows: body.iter().map(|s| s.to_string()).collect(),
        });
        world
    }

    fn texts(world: &mut World, which: GapLine) -> Vec<String> {
        let mut found = world.query::<(&GapLine, &Text)>();
        found
            .iter(world)
            .filter(|(kind, _)| **kind == which)
            .map(|(_, t)| t.0.clone())
            .collect()
    }

    #[test]
    fn spawn_once_joins_body_lines_with_newlines() {
        let mut world = world_with(ShellScreen::Loading, &["a", "b", "c"]);
        world.run_system_once(build_interim_overlay).unwrap();
        world.run_system_once(build_interim_overlay).unwrap();
        assert_eq!(world.query::<&InterimOverlayAnchor>().iter(&world).count(), 1);
        assert_eq!(texts(&mut world, GapLine::Title), ["T"]);
        assert_eq!(texts(&mut world, GapLine::Body), ["a\nb\nc"]);
    }

    #[test]
    fn no_spawn_on_a_real_screen_and_refresh_follows_resource() {
        let mut world = world_with(ShellScreen::InGame, &["x"]);
        world.run_system_once(build_interim_overlay).unwrap();
        assert_eq!(world.query::<&InterimOverlayAnchor>().iter(&world).count(), 0);

        world.insert_resource(ShellScreen::Loading);
        world.run_system_once(build_interim_overlay).unwrap();
        world.insert_resource(InterimOverlay {
            heading: "N".into(),
            rows: vec!["p".into(), "q".into()],
        });
        world.run_system_once(redo_interim_overlay).unwrap();
        assert_eq!(texts(&mut world, GapLine::Title), ["N"]);
        assert_eq!(texts(&mut world, GapLine::Body), ["p\nq"]);
    }

    #[test]
    fn despawns_when_a_real_screen_returns() {
        let mut world = world_with(ShellScreen::Loading, &["x"]);
        world.run_system_once(build_interim_overlay).unwrap();
        world.run_system_once(drop_interim_when_unseen).unwrap();
        assert_eq!(world.query::<&InterimOverlayAnchor>().iter(&world).count(), 1);
        world.insert_resource(ShellScreen::MainMenu);
        world.run_system_once(drop_interim_when_unseen).unwrap();
        assert_eq!(world.query::<&InterimOverlayAnchor>().iter(&world).count(), 0);
    }
}
