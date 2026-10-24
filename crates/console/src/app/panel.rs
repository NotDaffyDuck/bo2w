//! The console panel on screen: the scrollback, the prompt line, and the
//! completions drawn above it.

use bevy::{
    prelude::*,
    text::{EditableText, TextCursorStyle},
};
use ui::UiLayer;

use super::{
    prompt::offered,
    state::{ConsoleFont, ConsoleSettings, ConsoleState},
};
use crate::{
    ConsoleRegistry,
    suggest::{SuggestSpan, SuggestTone, suggestion_spans},
};

const FONT_BYTES: &[u8] = include_bytes!("../../assets/FreeMono.otf");

/// What is printed before the line being typed.
pub(super) const PROMPT: &str = "> ";

const FONT_SIZE: f32 = 15.0;
const MARGIN: f32 = 12.0;
/// The scrollback stops this far above the bottom edge, clear of the prompt.
const SCROLLBACK_BOTTOM: f32 = 36.0;

const COLOR_BODY: Color = Color::srgb(0.82, 0.92, 0.82);
const COLOR_MATCHED: Color = Color::srgb(0.40, 0.48, 0.42);
const COLOR_REST_SELECTED: Color = Color::srgb(0.95, 0.78, 0.35);
const COLOR_REST_MUTED: Color = Color::srgb(0.48, 0.58, 0.50);
const COLOR_PANEL: Color = Color::srgba(0.02, 0.03, 0.04, 0.96);
const COLOR_SUGGEST_BG: Color = Color::srgb(0.02, 0.03, 0.04);
const COLOR_SELECTION: Color = Color::srgba(0.35, 0.55, 0.85, 0.45);
const COLOR_SELECTION_UNFOCUSED: Color = Color::srgba(0.35, 0.55, 0.85, 0.2);

/// The whole panel, shown while the console is open.
#[derive(Component)]
pub(super) struct ConsolePanel;

/// The printed lines.
#[derive(Component)]
pub(super) struct ScrollbackText;

/// The line being typed.
#[derive(Component)]
pub(super) struct PromptText;

/// The list of completions, drawn over the scrollback's last line.
#[derive(Component)]
pub(super) struct CompletionText;

#[derive(Resource, Clone, Copy)]
pub(super) struct PromptEntity(pub Entity);

#[derive(Resource, Clone, Copy)]
pub(super) struct ScrollbackEntity(pub Entity);

fn text_font(font: &Handle<Font>) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(FONT_SIZE),
        ..default()
    }
}

/// Builds the panel, hidden, and the font it is set in.
pub(super) fn build_panel(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    settings: Res<ConsoleSettings>,
) {
    let font = fonts.add(Font::from_bytes(FONT_BYTES.to_vec()));
    commands.insert_resource(ConsoleFont(font.clone()));

    let scrollback = commands
        .spawn((
            ScrollbackText,
            Text::new(""),
            text_font(&font),
            TextColor(COLOR_BODY),
            TextCursorStyle {
                color: Color::NONE,
                selection_color: COLOR_SELECTION,
                unfocused_selection_color: COLOR_SELECTION,
                selected_text_color: None,
            },
        ))
        .id();
    let scrollback_clip = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(MARGIN),
            right: px(MARGIN),
            top: px(MARGIN),
            bottom: px(SCROLLBACK_BOTTOM),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::End,
            overflow: Overflow::clip(),
            ..default()
        })
        .add_child(scrollback)
        .id();

    let completions = commands
        .spawn((
            CompletionText,
            Node {
                position_type: PositionType::Absolute,
                left: px(MARGIN),
                bottom: px(SCROLLBACK_BOTTOM),
                padding: UiRect::axes(px(0.0), px(2.0)),
                ..default()
            },
            BackgroundColor(COLOR_SUGGEST_BG),
            Visibility::Hidden,
            Text::new(""),
            text_font(&font),
            TextColor(COLOR_REST_MUTED),
        ))
        .id();

    let label = commands
        .spawn((Text::new(PROMPT), text_font(&font), TextColor(COLOR_BODY)))
        .id();
    let prompt = commands
        .spawn((
            PromptText,
            EditableText {
                allow_newlines: false,
                visible_lines: Some(1.0),
                ..default()
            },
            TextLayout::no_wrap(),
            text_font(&font),
            TextColor(COLOR_BODY),
            TextCursorStyle {
                color: COLOR_BODY,
                selection_color: COLOR_SELECTION,
                unfocused_selection_color: COLOR_SELECTION_UNFOCUSED,
                selected_text_color: None,
            },
            Node {
                flex_grow: 1.0,
                min_width: px(8.0),
                height: px(20.0),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    let prompt_row = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(MARGIN),
            right: px(MARGIN),
            bottom: px(MARGIN),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(4.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .add_children(&[label, prompt])
        .id();

    commands
        .spawn((
            ConsolePanel,
            UiLayer::Overlay,
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                left: px(MARGIN),
                bottom: px(MARGIN),
                width: px(settings.width),
                height: px(settings.height),
                padding: UiRect::all(px(MARGIN)),
                ..default()
            },
            BackgroundColor(COLOR_PANEL),
            GlobalZIndex(20_000),
            ZIndex(100),
        ))
        .add_children(&[scrollback_clip, completions, prompt_row]);

    commands.insert_resource(PromptEntity(prompt));
    commands.insert_resource(ScrollbackEntity(scrollback));
    crate::showpos::spawn_hud(&mut commands, font);
}

fn tone_color(tone: SuggestTone) -> Color {
    match tone {
        SuggestTone::Indent | SuggestTone::Ellipsis | SuggestTone::RestMuted => COLOR_REST_MUTED,
        SuggestTone::Matched => COLOR_MATCHED,
        SuggestTone::RestSelected => COLOR_REST_SELECTED,
    }
}

fn completion_span(span: &SuggestSpan, font: &Handle<Font>) -> impl Bundle {
    (
        TextSpan::new(span.text.clone()),
        text_font(font),
        TextColor(tone_color(span.tone)),
        TextBackgroundColor(COLOR_SUGGEST_BG),
    )
}

fn visibility_for(shown: bool) -> Visibility {
    if shown {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}

/// Shows or hides the panel, and keeps the scrollback and the completions
/// matching the console's state.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub(super) fn update_console_ui(
    mut commands: Commands,
    state: Res<ConsoleState>,
    registry: Res<ConsoleRegistry>,
    font: Res<ConsoleFont>,
    mut drawn: Local<Vec<SuggestSpan>>,
    mut panels: Query<&mut Visibility, (With<ConsolePanel>, Without<CompletionText>)>,
    mut completions: Query<
        (Entity, &mut Visibility, &mut Text, Option<&Children>),
        (
            With<CompletionText>,
            Without<ConsolePanel>,
            Without<ScrollbackText>,
        ),
    >,
    mut scrollback: Query<&mut Text, (With<ScrollbackText>, Without<CompletionText>)>,
) {
    let want = visibility_for(state.open);
    for mut visibility in &mut panels {
        visibility.set_if_neq(want);
    }

    if !state.open {
        drawn.clear();
        for (entity, mut visibility, mut text, children) in &mut completions {
            visibility.set_if_neq(Visibility::Hidden);
            if !text.is_empty() {
                text.clear();
            }
            if children.is_some_and(|children| !children.is_empty()) {
                commands.entity(entity).despawn_related::<Children>();
            }
        }
        return;
    }

    let log = state.log.join("\n");
    for mut text in &mut scrollback {
        if **text != log {
            **text = log.clone();
        }
    }

    let (token_start, _, token) = state.editor.token_at_caret();
    let indent_cols = PROMPT.chars().count() + token_start;
    let spans = suggestion_spans(
        &offered(&state, &registry),
        state.editor.selected,
        &token,
        indent_cols,
    );
    if *drawn == spans {
        return;
    }
    for (entity, mut visibility, mut text, _) in &mut completions {
        text.clear();
        commands.entity(entity).despawn_related::<Children>();
        visibility.set_if_neq(visibility_for(!spans.is_empty()));
        commands.entity(entity).with_children(|list| {
            for span in &spans {
                list.spawn(completion_span(span, &font.0));
            }
        });
    }
    *drawn = spans;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandSpec;

    fn console_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Font>>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleRegistry>()
            .add_systems(Startup, build_panel)
            .add_systems(Update, update_console_ui);
        app.world_mut()
            .resource_mut::<ConsoleRegistry>()
            .register(CommandSpec::new("map").usage("map <name>"));
        app.update();
        app
    }

    fn visibility_of<M: Component>(app: &mut App) -> Visibility {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&Visibility, With<M>>();
        *query.single(world).expect("one such entity")
    }

    fn text_of<M: Component>(app: &mut App) -> String {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&Text, With<M>>();
        (**query.single(world).expect("one such entity")).clone()
    }

    fn completion_children(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&Children, With<CompletionText>>();
        query.iter(world).map(|children| children.len()).sum()
    }

    fn type_line(app: &mut App, line: &str) {
        let mut state = app.world_mut().resource_mut::<ConsoleState>();
        state.open = true;
        state.editor.line = line.to_owned();
        state.editor.caret = line.chars().count();
    }

    #[test]
    fn the_panel_starts_hidden_and_follows_open() {
        let mut app = console_app();
        assert_eq!(visibility_of::<ConsolePanel>(&mut app), Visibility::Hidden);
        type_line(&mut app, "");
        app.update();
        assert_eq!(visibility_of::<ConsolePanel>(&mut app), Visibility::Visible);
        app.world_mut().resource_mut::<ConsoleState>().open = false;
        app.update();
        assert_eq!(visibility_of::<ConsolePanel>(&mut app), Visibility::Hidden);
    }

    #[test]
    fn the_scrollback_shows_the_log_one_line_each() {
        let mut app = console_app();
        type_line(&mut app, "");
        let mut state = app.world_mut().resource_mut::<ConsoleState>();
        state.log = vec!["first".to_owned(), "second".to_owned()];
        app.update();
        assert_eq!(text_of::<ScrollbackText>(&mut app), "first\nsecond");
    }

    #[test]
    fn typing_a_command_prefix_lists_its_completions() {
        let mut app = console_app();
        type_line(&mut app, "ma");
        app.update();
        assert_eq!(
            visibility_of::<CompletionText>(&mut app),
            Visibility::Visible
        );
        assert!(completion_children(&mut app) > 0);
        type_line(&mut app, "zzzz");
        app.update();
        assert_eq!(
            visibility_of::<CompletionText>(&mut app),
            Visibility::Hidden
        );
        assert_eq!(completion_children(&mut app), 0);
    }

    #[test]
    fn closing_the_console_clears_the_completions() {
        let mut app = console_app();
        type_line(&mut app, "ma");
        app.update();
        app.world_mut().resource_mut::<ConsoleState>().open = false;
        app.update();
        assert_eq!(
            visibility_of::<CompletionText>(&mut app),
            Visibility::Hidden
        );
        assert_eq!(completion_children(&mut app), 0);
        type_line(&mut app, "ma");
        app.update();
        assert!(
            completion_children(&mut app) > 0,
            "reopening draws them again"
        );
    }

    #[test]
    fn the_prompt_and_scrollback_are_found_through_their_resources() {
        let app = console_app();
        let prompt = app.world().resource::<PromptEntity>().0;
        let scrollback = app.world().resource::<ScrollbackEntity>().0;
        assert!(app.world().get::<PromptText>(prompt).is_some());
        assert!(app.world().get::<EditableText>(prompt).is_some());
        assert!(app.world().get::<ScrollbackText>(scrollback).is_some());
    }

    #[test]
    fn completion_tones_have_their_own_colours() {
        assert_eq!(
            tone_color(SuggestTone::Indent),
            tone_color(SuggestTone::RestMuted)
        );
        assert_ne!(
            tone_color(SuggestTone::RestSelected),
            tone_color(SuggestTone::RestMuted)
        );
        assert_ne!(
            tone_color(SuggestTone::Matched),
            tone_color(SuggestTone::RestSelected)
        );
    }
}
