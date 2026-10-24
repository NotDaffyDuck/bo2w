//! Picking text out of the scrollback with the mouse.
//!
//! The scrollback is one text node. A press or drag lands on a glyph, the
//! glyph stands for a character of the joined log, and the selection is the
//! stretch of characters between where the drag began and where it is now.

use bevy::{
    picking::{
        events::{Drag, Pointer, Press, Release},
        pointer::PointerButton,
    },
    prelude::*,
    text::{EditableText, TextLayoutInfo},
    ui::{ComputedUiRenderTargetInfo, UiGlobalTransform},
};

use super::{
    panel::{PromptEntity, PromptText, ScrollbackEntity, ScrollbackText},
    state::{ConsoleSettings, ConsoleState},
};
use crate::input::{
    RunBounds, ScrollCell, cell_char_index, cell_rects_for_char_range, copy_prompt_echo,
    copy_prompt_selection, hit_scroll_cell, line_char_range, map_glyphs_to_chars,
    ordered_char_range, slice_char_range, word_bounds,
};

/// Where each glyph of a laid-out text stands in the text, and the box each
/// row of it occupies.
struct Glyphs {
    /// For each glyph: its row, and the character of the text it draws.
    cells: Vec<ScrollCell>,
    runs: Vec<RunBounds>,
}

impl Glyphs {
    fn of(text: &str, layout: &TextLayoutInfo) -> Self {
        let chars = map_glyphs_to_chars(text, layout.glyphs.len());
        Self {
            cells: layout
                .glyphs
                .iter()
                .zip(chars)
                .map(|(glyph, ch)| (glyph.line_index, ch))
                .collect(),
            runs: layout
                .run_geometry
                .iter()
                .map(|run| {
                    let bounds = run.bounds;
                    (bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y)
                })
                .collect(),
        }
    }

    /// The row, and the character, under a point of the text node.
    fn hit(&self, local: Vec2) -> Option<(usize, usize)> {
        let (row, col) = hit_scroll_cell(&self.cells, &self.runs, (local.x, local.y))?;
        let ch = cell_char_index(&self.cells, &self.runs, row, col)?;
        Some((row, ch))
    }

    /// The right edge of the widest row.
    fn text_right(&self) -> f32 {
        self.runs.iter().map(|run| run.2).fold(0.0, f32::max)
    }
}

/// What a press selects: nothing yet for one click (the caret goes there), the
/// word for two, the whole row for three or more.
fn pressed_range(text: &str, glyphs: &Glyphs, local: Vec2, count: u8) -> Option<(usize, usize)> {
    let (row, ch) = glyphs.hit(local)?;
    match count {
        0 | 1 => Some((ch, ch)),
        2 => Some(word_bounds(text, ch)),
        _ => line_char_range(&glyphs.cells, &glyphs.runs, row),
    }
}

/// The text the scrollback selection covers.
fn selected_scrollback(state: &ConsoleState) -> Option<String> {
    let anchor = state.scroll_anchor?;
    let text = state.log.join("\n");
    let slice = slice_char_range(&text, anchor, state.scroll_focus);
    (!slice.is_empty()).then(|| slice.to_owned())
}

fn selection_rects(glyphs: &Glyphs, lo: usize, hi: usize, block_right: f32) -> Vec<Rect> {
    cell_rects_for_char_range(lo, hi, &glyphs.cells, &glyphs.runs, block_right)
        .into_iter()
        .map(|(x0, y0, x1, y1)| Rect::new(x0, y0, x1, y1))
        .collect()
}

/// A screen point as a place inside the text node, in layout units.
fn to_local(
    transform: &UiGlobalTransform,
    node: &ComputedNode,
    target: &ComputedUiRenderTargetInfo,
    ui_scale: f32,
    pointer: Vec2,
) -> Option<Vec2> {
    let inverse = transform.try_inverse()?;
    Some(
        inverse.transform_point2(pointer * target.scale_factor() / ui_scale)
            - node.content_box().min,
    )
}

/// Starts a selection on a press in the scrollback and stretches it as the
/// pointer drags. A press in the prompt drops the selection.
#[allow(clippy::type_complexity)]
pub(super) fn handle_scrollback_pointer(
    mut presses: MessageReader<Pointer<Press>>,
    mut drags: MessageReader<Pointer<Drag>>,
    prompt: Option<Res<PromptEntity>>,
    scrollback: Option<Res<ScrollbackEntity>>,
    mut state: ResMut<ConsoleState>,
    ui_scale: Res<UiScale>,
    texts: Query<
        (
            &ComputedNode,
            &ComputedUiRenderTargetInfo,
            &UiGlobalTransform,
            &TextLayoutInfo,
            &ChildOf,
        ),
        With<ScrollbackText>,
    >,
) {
    if !state.open || (presses.is_empty() && drags.is_empty()) {
        return;
    }
    let Some(scrollback) = scrollback.map(|entity| entity.0) else {
        return;
    };
    let prompt = prompt.map(|entity| entity.0);
    let Ok((node, target, transform, layout, parent)) = texts.get(scrollback) else {
        return;
    };
    let on_text = |entity: Entity| entity == scrollback || entity == parent.0;
    let local_of = |pointer: Vec2| to_local(transform, node, target, ui_scale.0, pointer);
    let text = state.log.join("\n");
    let glyphs = Glyphs::of(&text, layout);

    for press in presses.read() {
        if press.button != PointerButton::Primary {
            continue;
        }
        if Some(press.entity) == prompt {
            state.clear_scroll_selection();
            continue;
        }
        if !on_text(press.entity) {
            continue;
        }
        let range = local_of(press.pointer_location.position)
            .and_then(|local| pressed_range(&text, &glyphs, local, press.count));
        match range {
            Some((anchor, focus)) => {
                state.scroll_anchor = Some(anchor);
                state.scroll_focus = focus;
                state.scroll_gesture = true;
            }
            None => state.clear_scroll_selection(),
        }
    }
    if !state.scroll_gesture {
        return;
    }
    for drag in drags.read() {
        if drag.button != PointerButton::Primary || !on_text(drag.entity) {
            continue;
        }
        let hit = local_of(drag.pointer_location.position).and_then(|local| glyphs.hit(local));
        if let Some((_, ch)) = hit {
            state.scroll_focus = ch;
        }
    }
}

/// Copies the selection to the clipboard when the mouse button comes up: the
/// scrollback's when the press began there, the prompt's otherwise.
pub(super) fn copy_selection_on_release(
    mut releases: MessageReader<Pointer<Release>>,
    prompts: Query<&EditableText, With<PromptText>>,
    mut state: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut clipboard: ResMut<Clipboard>,
) {
    if !state.open {
        state.clear_scroll_selection();
        return;
    }
    if !releases
        .read()
        .any(|release| release.button == PointerButton::Primary)
    {
        return;
    }
    let from_scrollback = std::mem::take(&mut state.scroll_gesture);
    let selected = if from_scrollback {
        selected_scrollback(&state)
    } else {
        prompts
            .single()
            .ok()
            .and_then(|prompt| prompt.editor().selected_text().map(str::to_owned))
    };
    let result = copy_prompt_selection(selected.as_deref(), |text| {
        clipboard.set_text(text).map_err(|error| error.to_string())
    });
    if let Some(line) = copy_prompt_echo(&result) {
        state.echo(line, settings.log_capacity);
    }
}

/// Lays the selection's rectangles over the scrollback text.
pub(super) fn paint_scrollback_selection(
    state: Res<ConsoleState>,
    ui_scale: Res<UiScale>,
    mut texts: Query<
        (
            &mut TextLayoutInfo,
            &ComputedNode,
            &ComputedUiRenderTargetInfo,
        ),
        With<ScrollbackText>,
    >,
) {
    let Ok((mut layout, node, target)) = texts.single_mut() else {
        return;
    };
    let range = state
        .scroll_anchor
        .map(|anchor| ordered_char_range(anchor, state.scroll_focus))
        .filter(|(lo, hi)| lo != hi);
    let rects = match range {
        None => Vec::new(),
        Some((lo, hi)) => {
            let text = state.log.join("\n");
            let glyphs = Glyphs::of(&text, &layout);
            let panel_right = node.content_box().width() * target.scale_factor() / ui_scale.0;
            selection_rects(&glyphs, lo, hi, glyphs.text_right().max(panel_right))
        }
    };
    if layout.selection_rects != rects {
        layout.selection_rects = rects;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "ab cd" over "ef", ten units to a character.
    fn two_rows() -> (&'static str, Glyphs) {
        let cells = vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (1, 6), (1, 7)];
        let runs = vec![(0.0, 0.0, 50.0, 10.0), (0.0, 10.0, 20.0, 20.0)];
        ("ab cd\nef", Glyphs { cells, runs })
    }

    fn state_with_log(anchor: Option<usize>, focus: usize) -> ConsoleState {
        ConsoleState {
            log: vec!["ab cd".to_owned(), "ef".to_owned()],
            scroll_anchor: anchor,
            scroll_focus: focus,
            ..Default::default()
        }
    }

    #[test]
    fn one_click_puts_the_caret_at_the_character() {
        let (text, glyphs) = two_rows();
        let at = pressed_range(text, &glyphs, Vec2::new(12.0, 5.0), 1);
        assert_eq!(at, Some((1, 1)));
    }

    #[test]
    fn two_clicks_take_the_word() {
        let (text, glyphs) = two_rows();
        let at = pressed_range(text, &glyphs, Vec2::new(32.0, 5.0), 2);
        assert_eq!(at, Some((3, 5)));
    }

    #[test]
    fn three_clicks_take_the_row() {
        let (text, glyphs) = two_rows();
        let first = pressed_range(text, &glyphs, Vec2::new(5.0, 5.0), 3);
        let second = pressed_range(text, &glyphs, Vec2::new(5.0, 15.0), 4);
        assert_eq!((first, second), (Some((0, 5)), Some((6, 8))));
    }

    #[test]
    fn a_press_past_the_end_of_a_row_lands_after_its_last_character() {
        let (text, glyphs) = two_rows();
        let at = pressed_range(text, &glyphs, Vec2::new(500.0, 5.0), 1);
        assert_eq!(at, Some((5, 5)));
    }

    #[test]
    fn a_press_with_no_text_laid_out_selects_nothing() {
        let empty = Glyphs {
            cells: Vec::new(),
            runs: Vec::new(),
        };
        assert_eq!(pressed_range("", &empty, Vec2::ZERO, 1), None);
    }

    #[test]
    fn a_selection_inside_a_row_paints_that_stretch() {
        let (_, glyphs) = two_rows();
        assert_eq!(
            selection_rects(&glyphs, 1, 3, 80.0),
            [Rect::new(10.0, 0.0, 30.0, 10.0)]
        );
    }

    #[test]
    fn a_selection_over_a_line_break_runs_to_the_block_edge() {
        let (_, glyphs) = two_rows();
        assert_eq!(
            selection_rects(&glyphs, 3, 7, 80.0),
            [
                Rect::new(30.0, 0.0, 80.0, 10.0),
                Rect::new(0.0, 10.0, 10.0, 20.0)
            ]
        );
    }

    #[test]
    fn the_widest_row_sets_the_text_edge() {
        let (_, glyphs) = two_rows();
        assert_eq!(glyphs.text_right(), 50.0);
    }

    #[test]
    fn the_selection_copies_the_joined_log_whichever_way_it_was_dragged() {
        for (anchor, focus) in [(3, 7), (7, 3)] {
            let state = state_with_log(Some(anchor), focus);
            assert_eq!(selected_scrollback(&state).as_deref(), Some("cd\ne"));
        }
    }

    #[test]
    fn nothing_is_copied_without_a_selection() {
        assert_eq!(selected_scrollback(&state_with_log(None, 0)), None);
        assert_eq!(selected_scrollback(&state_with_log(Some(2), 2)), None);
    }
}
