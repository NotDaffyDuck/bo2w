//! Mouse selection inside the console log.
//!
//! The log is drawn as a single text node. Pressing or dragging over it hits a
//! letter_shape, each letter_shape maps to a character of the newline-joined log, and the
//! selection covers the characters between the drag's start and its current
//! end.

use bevy::{picking::events::Drag, picking::events::Pointer, picking::events::Press};
use bevy::picking::events::Release;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::text::TextLayoutInfo;
use bevy::ui::ComputedUiRenderTargetInfo;
use bevy::ui::UiGlobalTransform;

use super::panel::PromptEntity;
use super::panel::PromptText;
use super::panel::ScrollbackEntity;
use super::panel::ScrollbackText;
use super::state::PromptPanelOptions;
use super::state::PromptPanelStatus;
use crate::key_input::{
    GlyphSpot, LogGrid, RowBounds, copy_prompt_echo, clip_prompt_highlight, map_glyphs_to_chars,
    sorted_glyph_span, cut_glyph_span, word_bounds,
};

/// A laid-out text reduced to what hit testing needs: which row and character
/// each letter_shape belongs to, and the rectangle of every row.
struct Glyphs(LogGrid);

/// The whole log as the text node shows it: one line per entry.
fn log_text(state: &PromptPanelStatus) -> String {
    state.lines.join("\n")
}

impl Glyphs {
    fn of(text: &str, layout: &TextLayoutInfo) -> Self {
        let count = layout.glyphs.len();
        let chars = map_glyphs_to_chars(text, count);
        let mut cells = Vec::with_capacity(count);
        for (letter_shape, ch) in layout.glyphs.iter().zip(chars) {
            cells.push(GlyphSpot { row_id: letter_shape.line_index, ch });
        }
        let mut runs = Vec::with_capacity(layout.run_geometry.len());
        for geometry in &layout.run_geometry {
            let Rect { min, max } = geometry.bounds;
            runs.push(RowBounds { x_min: min.x, y_min: min.y, x_max: max.x, y_max: max.y });
        }
        Self(LogGrid::new(&cells, runs))
    }

    /// Row and character at a point in the text node's own space.
    fn hit(&self, local: Vec2) -> Option<(usize, usize)> {
        let Vec2 { x, y } = local;
        let (row, col) = self.0.hit((x, y))?;
        let ch = self.0.char_at(row, col)?;
        Some((row, ch))
    }

    /// Where the widest row ends.
    fn text_right(&self) -> f32 {
        self.0.widest()
    }
}

/// Selection made by a press: a single click only places the insert_column, a double
/// click takes the word, and any further click takes the whole row.
fn pressed_range(text: &str, glyphs: &Glyphs, at: Vec2, count: u8) -> Option<(usize, usize)> {
    let (row, ch) = glyphs.hit(at)?;
    match count {
        0 | 1 => Some((ch, ch)),
        2 => Some(word_bounds(text, ch)),
        _ => glyphs.0.row_span(row),
    }
}

/// The log text between the selection ends, if any is selected.
fn selected_scrollback(state: &PromptPanelStatus) -> Option<String> {
    let anchor = state.scroll_anchor?;
    let text = log_text(state);
    let picked = cut_glyph_span(&text, anchor, state.backlog_anchor);
    (!picked.is_empty()).then(|| picked.to_owned())
}

fn selection_rects(glyphs: &Glyphs, lo: usize, hi: usize, text_box_right: f32) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for RowBounds { x_min: left, y_min: top, x_max: right, y_max: bottom } in glyphs.0.paint(lo, hi, text_box_right) {
        rects.push(Rect::new(left, top, right, bottom));
    }
    rects
}

/// How far, in layout units, the text node's content reaches across, given
/// the node's computed size and the window's scale.
fn layout_width(node: &ComputedNode, render: &ComputedUiRenderTargetInfo, ui_scale: f32) -> f32 {
    node.content_box().width() * render.scale_factor() / ui_scale
}

/// Where the text node sits on screen, enough to turn a window position into a
/// position inside it (in layout units).
struct NodeFrame<'a> {
    transform: &'a UiGlobalTransform,
    node: &'a ComputedNode,
    /// The window's scale factor, then the user's UI zoom it is divided by.
    window_scale: f32,
    user_scale: f32,
}

impl<'a> NodeFrame<'a> {
    fn new(
        transform: &'a UiGlobalTransform,
        node: &'a ComputedNode,
        render_target: &ComputedUiRenderTargetInfo,
        user_scale: f32,
    ) -> Self {
        Self { transform, node, window_scale: render_target.scale_factor(), user_scale }
    }

    fn local(&self, window_at: Vec2) -> Option<Vec2> {
        let to_node = self.transform.try_inverse()?;
        let at = to_node.transform_point2(window_at * self.window_scale / self.user_scale);
        Some(at - self.node.content_box().min)
    }
}

/// What one primary press does to the log's selection.
enum PressEffect {
    /// Not about the log at all.
    Ignore,
    /// Drops whatever was selected.
    Clear,
    /// Starts a selection from `anchor` to `focus`.
    Select(usize, usize),
}

impl PressEffect {
    fn apply(self, state: &mut PromptPanelStatus) {
        if let PressEffect::Select(anchor, focus) = self {
            state.scroll_anchor = Some(anchor);
            state.backlog_anchor = focus;
            state.wheel_dragging = true;
        } else if let PressEffect::Clear = self {
            state.clear_scroll_selection();
        }
    }
}

/// The log's text node, as hit testing reads it.
type LogNodeView<'w, 's> = Query<
    'w,
    's,
    (
        &'static ComputedNode,
        &'static ComputedUiRenderTargetInfo,
        &'static UiGlobalTransform,
        &'static TextLayoutInfo,
        &'static ChildOf,
    ),
    With<ScrollbackText>,
>;

/// The log's text node, as selection painting writes it.
type LogNodePaint<'w, 's> = Query<
    'w,
    's,
    (&'static mut TextLayoutInfo, &'static ComputedNode, &'static ComputedUiRenderTargetInfo),
    With<ScrollbackText>,
>;

/// Pressing in the log starts a selection there and dragging moves its far
/// end; pressing in the prompt drops the log's selection.
pub(super) fn handle_scrollback_pointer(
    mut press_events: MessageReader<Pointer<Press>>,
    mut drag_events: MessageReader<Pointer<Drag>>,
    prompt: Option<Res<PromptEntity>>,
    scrollback: Option<Res<ScrollbackEntity>>,
    mut state: ResMut<PromptPanelStatus>,
    zoom: Res<UiScale>,
    log_nodes: LogNodeView,
) {
    if !state.shown || (press_events.is_empty() && drag_events.is_empty()) {
        return;
    }
    let Some(log_entity) = scrollback.as_deref().map(|holder| holder.0) else {
        return;
    };
    let Ok(found) = log_nodes.get(log_entity) else {
        return;
    };
    let (node, target, transform, layout, holder) = found;
    let prompt_entity = prompt.as_deref().map(|holder| holder.0);
    let frame = NodeFrame::new(transform, node, target, zoom.0);
    let in_log = |entity: Entity| entity == log_entity || entity == holder.0;
    let joined = log_text(&state);
    let glyphs = Glyphs::of(&joined, layout);

    let effects: Vec<PressEffect> = press_events
        .read()
        .filter(|press| press.button == PointerButton::Primary)
        .map(|press| {
            if prompt_entity == Some(press.entity) {
                PressEffect::Clear
            } else if in_log(press.entity) {
                frame
                    .local(press.pointer_location.position)
                    .and_then(|local| pressed_range(&joined, &glyphs, local, press.count))
                    .map_or(PressEffect::Clear, |(anchor, focus)| PressEffect::Select(anchor, focus))
            } else {
                PressEffect::Ignore
            }
        })
        .collect::<Vec<PressEffect>>();
    effects.into_iter().for_each(|effect| effect.apply(&mut state));

    // A drag counts only while a gesture that started in the log is held.
    if !state.wheel_dragging {
        return;
    }
    let last_char = drag_events
        .read()
        .filter(|drag| drag.button == PointerButton::Primary && in_log(drag.entity))
        .filter_map(|drag| frame.local(drag.pointer_location.position))
        .filter_map(|local| glyphs.hit(local))
        .map(|(_, ch)| ch)
        .last();
    if let Some(ch) = last_char {
        state.backlog_anchor = ch;
    }
}

/// When the primary button comes up, copies what is selected: the log's
/// selection if the gesture started in the log, else the prompt's.
pub(super) fn copy_selection_on_release(
    mut release_events: MessageReader<Pointer<Release>>,
    prompts: Query<&EditableText, With<PromptText>>,
    mut state: ResMut<PromptPanelStatus>,
    settings: Res<PromptPanelOptions>,
    mut system_clipboard: ResMut<Clipboard>,
) {
    let open = state.shown;
    if !open {
        state.clear_scroll_selection();
        return;
    }
    let primary_released = release_events
        .read()
        .any(|released| matches!(released.button, PointerButton::Primary));
    if !primary_released {
        return;
    }
    let started_in_log = std::mem::take(&mut state.wheel_dragging);
    let picked = if started_in_log {
        selected_scrollback(&state)
    } else {
        prompts
            .single()
            .ok()
            .and_then(|prompt| prompt.editor().selected_text().map(String::from))
    };
    let outcome = clip_prompt_highlight(picked.as_deref(), |chosen| {
        system_clipboard.set_text(chosen).map_err(|why| why.to_string())
    });
    if let Some(note) = copy_prompt_echo(&outcome) {
        state.post_line(note, settings.backlog_limit);
    }
}

/// Keeps the selection rectangles in the log's text layout in step with the
/// selection, writing them only when they change.
pub(super) fn paint_scrollback_selection(
    state: Res<PromptPanelStatus>,
    zoom: Res<UiScale>,
    mut log_nodes: LogNodePaint,
) {
    let Ok(found) = log_nodes.single_mut() else {
        return;
    };
    let (mut layout, node, target) = found;
    let range = if let Some(anchor) = state.scroll_anchor {
        Some(sorted_glyph_span(anchor, state.backlog_anchor))
    } else {
        None
    };
    let wanted = match range {
        Some((lo, hi)) if lo != hi => {
            let joined = log_text(&state);
            let glyphs = Glyphs::of(&joined, &layout);
            let right_edge = layout_width(node, target, zoom.0).max(glyphs.text_right());
            selection_rects(&glyphs, lo, hi, right_edge)
        }
        _ => vec![],
    };
    if layout.selection_rects != wanted {
        layout.selection_rects = wanted;
    }
}

#[cfg(test)]
mod tests {
    use crate::app::scrollback::*;

    /// "ab cd" over "ef", ten units to a character.
    fn two_rows() -> (&'static str, Glyphs) {
        let spots = [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (1, 6), (1, 7)];
        let cells = spots.map(|(row_id, ch)| GlyphSpot { row_id, ch });
        let runs = vec![
            RowBounds { x_min: 0.0, y_min: 0.0, x_max: 50.0, y_max: 10.0 },
            RowBounds { x_min: 0.0, y_min: 10.0, x_max: 20.0, y_max: 20.0 },
        ];
        ("ab cd\nef", Glyphs(LogGrid::new(&cells, runs)))
    }

    fn state_with_log(anchor: Option<usize>, focus: usize) -> PromptPanelStatus {
        let mut state = PromptPanelStatus::default();
        state.lines = ["ab cd", "ef"].map(String::from).to_vec();
        state.scroll_anchor = anchor;
        state.backlog_anchor = focus;
        state
    }

    fn press_two_rows(x: f32, y: f32, count: u8) -> Option<(usize, usize)> {
        let (text, glyphs) = two_rows();
        pressed_range(text, &glyphs, Vec2::new(x, y), count)
    }

    #[test]
    fn one_click_puts_the_caret_at_the_character() {
        assert_eq!(press_two_rows(12.0, 5.0, 1), Some((1, 1)));
    }

    #[test]
    fn two_clicks_take_the_word() {
        assert_eq!(press_two_rows(32.0, 5.0, 2), Some((3, 5)));
    }

    #[test]
    fn three_clicks_take_the_row() {
        assert_eq!(press_two_rows(5.0, 5.0, 3), Some((0, 5)));
        assert_eq!(press_two_rows(5.0, 15.0, 4), Some((6, 8)));
    }

    #[test]
    fn a_press_past_the_end_of_a_row_lands_after_its_last_character() {
        assert_eq!(press_two_rows(500.0, 5.0, 1), Some((5, 5)));
    }

    #[test]
    fn a_press_with_no_text_laid_out_selects_nothing() {
        let bare = Glyphs(LogGrid::new(&[], vec![]));
        assert!(pressed_range("", &bare, Vec2::ZERO, 1).is_none());
    }

    #[test]
    fn a_selection_inside_a_row_paints_that_stretch() {
        let (_, glyphs) = two_rows();
        let painted = selection_rects(&glyphs, 1, 3, 80.0);
        assert_eq!(painted, vec![Rect::new(10.0, 0.0, 30.0, 10.0)]);
    }

    #[test]
    fn a_selection_over_a_line_break_runs_to_the_block_edge() {
        let (_, glyphs) = two_rows();
        let painted = selection_rects(&glyphs, 3, 7, 80.0);
        let first_row = Rect::new(30.0, 0.0, 80.0, 10.0);
        let second_row = Rect::new(0.0, 10.0, 10.0, 20.0);
        assert_eq!(painted, vec![first_row, second_row]);
    }

    #[test]
    fn the_widest_row_sets_the_text_edge() {
        assert_eq!(two_rows().1.text_right(), 50.0);
    }

    #[test]
    fn the_selection_copies_the_joined_log_whichever_way_it_was_dragged() {
        let forward = state_with_log(Some(3), 7);
        let backward = state_with_log(Some(7), 3);
        for state in [forward, backward] {
            assert_eq!(selected_scrollback(&state).as_deref(), Some("cd\ne"));
        }
    }

    #[test]
    fn nothing_is_copied_without_a_selection() {
        assert!(selected_scrollback(&state_with_log(None, 0)).is_none());
        assert!(selected_scrollback(&state_with_log(Some(2), 2)).is_none());
    }
}
