//! On-screen console panel: the log area, the input line, and the
//! suggestion strip that floats over the log's final row.

use bevy::prelude::*;
use ui::OverlayTier;

use super::prompt::offered;
use super::state::PromptTypeface;
use super::state::PromptPanelOptions;
use super::state::PromptPanelStatus;
use crate::OrderCatalog;
use crate::completion::HintRange;
use crate::completion::HintShade;
use crate::completion::suggestion_spans;
use bevy::ecs::system::Commands;
use bevy::text::Font;
use bevy::text::EditableText;
use bevy::text::TextCursorStyle;

const FONT_BYTES: &[u8] = include_bytes!("../../assets/FreeMono.otf");

/// Text drawn ahead of whatever the user is typing.
pub(super) const INPUT_LEAD_GLYPH: &str = "> ";

/// Palette used by the panel; every tint lives here.
mod palette {
    use bevy::color::Color;

    pub(super) const TEXT: Color = Color::srgb(0.82, 0.92, 0.82);
    pub(super) const HIT: Color = Color::srgb(0.40, 0.48, 0.42);
    pub(super) const TAIL_PICKED: Color = Color::srgb(0.95, 0.78, 0.35);
    pub(super) const TAIL_DIM: Color = Color::srgb(0.48, 0.58, 0.50);
    pub(super) const BACKDROP: Color = Color::srgba(0.02, 0.03, 0.04, 0.96);
    pub(super) const STRIP: Color = Color::srgb(0.02, 0.03, 0.04);
    pub(super) const HIGHLIGHT: Color = Color::srgba(0.35, 0.55, 0.85, 0.45);
    pub(super) const HIGHLIGHT_IDLE: Color = Color::srgba(0.35, 0.55, 0.85, 0.2);
}

const GLYPH_PX: f32 = 15.0;
const PANEL_MARGIN: f32 = 12.0;
/// Gap kept between the log's lower edge and the panel bottom, for the input row.
const LOG_FLOOR: f32 = 36.0;

/// Root of the panel; visible only while the console is open.
#[derive(Component)]
pub(super) struct PromptPanelRoot;

/// Marks the printed log text.
#[derive(Component)]
pub(super) struct ScrollbackText;

/// Marks the editable input line.
#[derive(Component)]
pub(super) struct PromptText;

/// Marks the suggestion strip, which overlaps the last log row.
#[derive(Component)]
pub(super) struct CompletionText;

#[derive(Resource, Clone, Copy)]
pub(super) struct PromptEntity(pub Entity);

#[derive(Resource, Clone, Copy)]
pub(super) struct ScrollbackEntity(pub Entity);

/// The console's monospace face at its one size.
fn mono(handle: &Handle<Font>) -> TextFont {
    let mut face = TextFont::default();
    face.font = handle.clone().into();
    face.font_size = FontSize::Px(GLYPH_PX);
    face
}

/// Cursor/selection look shared by both text areas; only the insert_column and the
/// unfocused highlight differ.
fn cursor_look(insert_column: Color, unfocused: Color) -> TextCursorStyle {
    let mut look = TextCursorStyle::default();
    look.color = insert_column;
    look.selection_color = palette::HIGHLIGHT;
    look.unfocused_selection_color = unfocused;
    look.selected_text_color = None;
    look
}

/// An absolutely placed node, offset from whichever panel edges are given.
fn pinned(left: Option<f32>, right: Option<f32>, top: Option<f32>, bottom: Option<f32>) -> Node {
    let edge = |offset: Option<f32>| offset.map_or(Val::Auto, px);
    let mut node = Node::default();
    node.position_type = PositionType::Absolute;
    node.left = edge(left);
    node.right = edge(right);
    node.top = edge(top);
    node.bottom = edge(bottom);
    node
}

/// The log text inside a clipping frame that keeps the newest line at its foot.
fn spawn_log(commands: &mut Commands, face: &TextFont) -> (Entity, Entity) {
    let text_bits = (TextColor(palette::TEXT), cursor_look(Color::NONE, palette::HIGHLIGHT));
    let text = commands.spawn((ScrollbackText, Text(String::new()), face.clone(), text_bits)).id();
    let mut frame = pinned(Some(PANEL_MARGIN), Some(PANEL_MARGIN), Some(PANEL_MARGIN), Some(LOG_FLOOR));
    frame.flex_direction = FlexDirection::Column;
    frame.justify_content = JustifyContent::End;
    frame.overflow = Overflow::clip();
    let frame = commands.spawn(frame).add_child(text).id();
    (frame, text)
}

/// The suggestion strip, hidden until there is something to offer.
fn spawn_strip(commands: &mut Commands, face: &TextFont) -> Entity {
    let mut place = pinned(Some(PANEL_MARGIN), None, None, Some(LOG_FLOOR));
    place.padding = UiRect::vertical(px(2.0));
    let look = (TextColor(palette::TAIL_DIM), BackgroundColor(palette::STRIP));
    let hidden = Visibility::Hidden;
    commands.spawn((CompletionText, Text(String::new()), face.clone(), look, hidden, place)).id()
}

/// The `> ` label and the editable line beside it.
fn spawn_input(commands: &mut Commands, face: &TextFont) -> (Entity, Entity) {
    let label = commands.spawn((Text::new(INPUT_LEAD_GLYPH), face.clone(), TextColor(palette::TEXT))).id();
    let mut editing = EditableText::default();
    editing.visible_lines = Some(1.0);
    editing.allow_newlines = false;
    let mut field = Node::default();
    field.min_width = px(8.0);
    field.height = px(20.0);
    field.flex_grow = 1.0;
    field.overflow = Overflow::clip();
    let look = (TextColor(palette::TEXT), cursor_look(palette::TEXT, palette::HIGHLIGHT_IDLE));
    let one_row = TextLayout::no_wrap();
    let line = commands
        .spawn((PromptText, editing, one_row, face.clone(), look, field))
        .id();
    let mut row = pinned(Some(PANEL_MARGIN), Some(PANEL_MARGIN), None, Some(PANEL_MARGIN));
    row.flex_direction = FlexDirection::Row;
    row.align_items = AlignItems::Center;
    row.column_gap = px(4.0);
    row.overflow = Overflow::clip();
    let row = commands.spawn(row).add_children(&[label, line]).id();
    (row, line)
}

/// Spawns the panel (initially hidden) and loads its font.
pub(super) fn build_panel(
    mut commands: Commands,
    mut font_store: ResMut<Assets<Font>>,
    settings: Res<PromptPanelOptions>,
) {
    let handle = font_store.add(Font::from_bytes(FONT_BYTES.to_vec()));
    let face = mono(&handle);
    let (log_frame, log_text) = spawn_log(&mut commands, &face);
    let strip = spawn_strip(&mut commands, &face);
    let (input_row, input) = spawn_input(&mut commands, &face);

    let mut body = pinned(Some(PANEL_MARGIN), None, None, Some(PANEL_MARGIN));
    body.padding = UiRect::all(px(PANEL_MARGIN));
    body.width = px(settings.panel_width);
    body.height = px(settings.panel_height);
    // Drawn above every other overlay.
    let stacking = (GlobalZIndex(20_000), ZIndex(100));
    let backdrop = BackgroundColor(palette::BACKDROP);
    let hidden = Visibility::Hidden;
    commands
        .spawn((PromptPanelRoot, OverlayTier::Overlay, hidden, backdrop, stacking, body))
        .add_children(&[log_frame, strip, input_row]);

    commands.insert_resource(PromptTypeface(handle.clone()));
    commands.insert_resource(ScrollbackEntity(log_text));
    commands.insert_resource(PromptEntity(input));
    crate::showpos::spawn_hud(&mut commands, handle);
}

fn tint_of(tone: HintShade) -> Color {
    if tone == HintShade::Matched {
        palette::HIT
    } else if tone == HintShade::RemainderPicked {
        palette::TAIL_PICKED
    } else {
        palette::TAIL_DIM
    }
}

fn shown_if(flag: bool) -> Visibility {
    [Visibility::Hidden, Visibility::Visible][usize::from(flag)]
}

type StripQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Visibility, &'static mut Text, Option<&'static Children>),
    (With<CompletionText>, Without<PromptPanelRoot>, Without<ScrollbackText>),
>;

/// Empties and hides every strip (the console just closed).
fn wipe_strips(commands: &mut Commands, strips: &mut StripQuery) {
    for (strip, mut shown, mut body, pieces) in strips.iter_mut() {
        shown.set_if_neq(Visibility::Hidden);
        if !body.is_empty() {
            body.clear();
        }
        if pieces.is_some_and(|list| !list.is_empty()) {
            commands.entity(strip).despawn_related::<Children>();
        }
    }
}

/// Swaps every strip's pieces for `spans`, shown only when there are any.
fn redraw_strips(commands: &mut Commands, strips: &mut StripQuery, spans: &[HintRange], handle: &Handle<Font>) {
    let face = mono(handle);
    for (strip, mut shown, mut body, _) in strips.iter_mut() {
        body.clear();
        shown.set_if_neq(shown_if(!spans.is_empty()));
        let mut node = commands.entity(strip);
        node.despawn_related::<Children>();
        node.with_children(|strip_children| {
            for span in spans {
                let colours = (TextColor(tint_of(span.tone)), TextBackgroundColor(palette::STRIP));
                strip_children.spawn((TextSpan::new(span.fill.clone()), face.clone(), colours));
            }
        });
    }
}

/// Per-frame sync: panel visibility, the log text, and the suggestion strip.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub(super) fn update_console_ui(
    mut commands: Commands,
    state: Res<PromptPanelStatus>,
    book: Res<OrderCatalog>,
    font: Res<PromptTypeface>,
    mut drawn: Local<Vec<HintRange>>,
    mut panels: Query<&mut Visibility, (With<PromptPanelRoot>, Without<CompletionText>)>,
    mut strips: StripQuery,
    mut logs: Query<&mut Text, (With<ScrollbackText>, Without<CompletionText>)>,
) {
    for mut shown in &mut panels {
        shown.set_if_neq(shown_if(state.shown));
    }
    if !state.shown {
        drawn.clear();
        wipe_strips(&mut commands, &mut strips);
        return;
    }

    let whole_log = state.lines.join("\n");
    for mut text in logs.iter_mut().filter(|text| text.as_str() != whole_log) {
        text.0.clone_from(&whole_log);
    }

    // The strip lines up under the word being typed, after the prompt.
    let (word_start, _, word) = state.editor.word_at_caret();
    let column = word_start + INPUT_LEAD_GLYPH.chars().count();
    let choices = offered(&state, &book);
    let spans = suggestion_spans(&choices, state.editor.highlight, &word, column);
    if spans != *drawn {
        redraw_strips(&mut commands, &mut strips, &spans, &font.0);
        *drawn = spans;
    }
}

#[cfg(test)]
mod tests {
    use crate::app::panel::*;
    use crate::OrderBlueprint;
    use bevy::app::Update;

    fn console_app() -> App {
        let mut app = App::default();
        app.init_resource::<Assets<Font>>();
        app.init_resource::<PromptPanelOptions>();
        app.init_resource::<PromptPanelStatus>();
        let mut book = OrderCatalog::default();
        book.enlist(OrderBlueprint::new("map").help_line("map <name>"));
        app.insert_resource(book);
        app.add_systems(Startup, build_panel);
        app.add_systems(Update, update_console_ui);
        app.update();
        app
    }

    fn visibility_of<M: Component>(app: &mut App) -> Visibility {
        let world = app.world_mut();
        let found = world.query_filtered::<&Visibility, With<M>>().single(world).copied();
        found.expect("one such entity")
    }

    fn text_of<M: Component>(app: &mut App) -> String {
        let world = app.world_mut();
        let found = world.query_filtered::<&Text, With<M>>().single(world).map(|text| text.0.clone());
        found.expect("one such entity")
    }

    fn completion_children(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut lists = world.query_filtered::<&Children, With<CompletionText>>();
        lists.iter(world).fold(0, |total, list| total + list.len())
    }

    fn type_line(app: &mut App, typed: &str) {
        let mut state = app.world_mut().resource_mut::<PromptPanelStatus>();
        state.shown = true;
        state.editor.insert_column = typed.chars().count();
        state.editor.line = typed.to_owned();
    }

    fn set_open(app: &mut App, open: bool) {
        app.world_mut().resource_mut::<PromptPanelStatus>().shown = open;
    }

    #[test]
    fn the_panel_starts_hidden_and_follows_open() {
        let mut app = console_app();
        assert_eq!(visibility_of::<PromptPanelRoot>(&mut app), Visibility::Hidden);
        type_line(&mut app, "");
        app.update();
        assert_eq!(visibility_of::<PromptPanelRoot>(&mut app), Visibility::Visible);
        set_open(&mut app, false);
        app.update();
        assert_eq!(visibility_of::<PromptPanelRoot>(&mut app), Visibility::Hidden);
    }

    #[test]
    fn the_scrollback_shows_the_log_one_line_each() {
        let mut app = console_app();
        type_line(&mut app, "");
        app.world_mut().resource_mut::<PromptPanelStatus>().lines = ["first", "second"].map(String::from).to_vec();
        app.update();
        assert_eq!(text_of::<ScrollbackText>(&mut app), "first\nsecond");
    }

    #[test]
    fn typing_a_command_prefix_lists_its_completions() {
        let mut app = console_app();
        type_line(&mut app, "ma");
        app.update();
        assert_eq!(visibility_of::<CompletionText>(&mut app), Visibility::Visible);
        assert_ne!(completion_children(&mut app), 0);
        type_line(&mut app, "zzzz");
        app.update();
        assert_eq!(visibility_of::<CompletionText>(&mut app), Visibility::Hidden);
        assert_eq!(completion_children(&mut app), 0);
    }

    #[test]
    fn closing_the_console_clears_the_completions() {
        let mut app = console_app();
        type_line(&mut app, "ma");
        app.update();
        set_open(&mut app, false);
        app.update();
        assert_eq!(visibility_of::<CompletionText>(&mut app), Visibility::Hidden);
        assert_eq!(completion_children(&mut app), 0);
        type_line(&mut app, "ma");
        app.update();
        assert_ne!(completion_children(&mut app), 0, "reopening draws them again");
    }

    #[test]
    fn the_prompt_and_scrollback_are_found_through_their_resources() {
        let app = console_app();
        let world = app.world();
        let PromptEntity(input) = *world.resource::<PromptEntity>();
        let ScrollbackEntity(log) = *world.resource::<ScrollbackEntity>();
        assert!(world.entity(input).contains::<PromptText>());
        assert!(world.entity(input).contains::<EditableText>());
        assert!(world.entity(log).contains::<ScrollbackText>());
    }

    #[test]
    fn completion_tones_have_their_own_colours() {
        let [lead_spaces, muted, picked, matched] =
            [HintShade::Indent, HintShade::RestMuted, HintShade::RemainderPicked, HintShade::Matched]
                .map(tint_of);
        assert_eq!(lead_spaces, muted);
        assert_ne!(picked, muted);
        assert_ne!(matched, picked);
    }
}
