//! Inputs the console holds down on a script's behalf, and the mouse movement
//! it feeds the client.

use std::collections::BTreeMap;

use bevy::prelude::*;
use input_t6::{WEAPNEXT_INVENTORY_DOWN, action_number_for, action_label};
use input_t6::HOLD_ID_END;

/// How long `press` holds an input when a script gives no time.
pub const TAP_HOLD_SECONDS: f32 = 0.15;

/// The longest step `tick` counts, so a hitch cannot cut a press short.
pub const TAP_STEP_SECONDS_CAP: f32 = 0.05;

/// Whether `word` names an input a script can hold or bind.
pub(crate) fn names_control_order(word: &str) -> bool {
    action_number_for(word).is_some()
}

/// The id of the input `name` stands for. `-forward` is the release half of
/// `+forward` and means the same input; `inventory` stands alone.
fn input_id(name: &str) -> Option<u32> {
    let id = action_number_for(name)?;
    let release_half = (id < HOLD_ID_END && id % 2 == 0) || id == WEAPNEXT_INVENTORY_DOWN + 1;
    Some(if release_half { id - 1 } else { id })
}

/// The inputs a script is holding, and the mouse it is moving.
#[derive(Resource, Debug, Default, Clone)]
pub struct ScriptedControlsStatus {
    /// Each held input, with the seconds it has left if it was only pressed.
    held: BTreeMap<u32, Option<f32>>,
    /// Mouse pixels waiting for the client to take them.
    queued_pointer_move: Option<(f32, f32)>,
    /// Mouse pixels added every frame.
    pointer_velocity: Option<(f32, f32)>,
}

impl ScriptedControlsStatus {
    /// Holds `key_input` until it is released. False if it is not an input, or was
    /// held already.
    pub fn keep_down(&mut self, control: &str) -> bool {
        let id = match input_id(control) {
            Some(found) => found,
            _missing => return false,
        };
        let newly_held = !self.held.contains_key(&id);
        self.held.insert(id, None);
        newly_held
    }

    /// Holds `key_input` for `extra` more than it has left. False if it is not an
    /// input.
    pub fn tap(&mut self, control: &str, extra: f32) -> bool {
        let id = match input_id(control) {
            Some(found) => found,
            _missing => return false,
        };
        let left = match self.held.get(&id) {
            Some(Some(left)) => left.max(extra),
            _ => extra,
        };
        self.held.insert(id, Some(left));
        true
    }

    /// Counts `dt` off every pressed input, and lets go of those out of time.
    pub fn step_timers(&mut self, step: f32) {
        let dt = step.clamp(0.0, TAP_STEP_SECONDS_CAP);
        self.held.retain(|_, left| match left {
            Some(left) => {
                *left -= dt;
                *left > 0.0
            }
            _unlimited => true,
        });
    }

    /// Lets go of `key_input`. False if it is not an input, or was not held.
    pub fn let_go(&mut self, control: &str) -> bool {
        input_id(control).is_some_and(|id| self.held.remove(&id).is_some())
    }

    /// Lets go of everything and stops the mouse.
    pub fn release_all(&mut self) {
        *self = Self::default();
    }

    /// Adds mouse pixels for the client to take next.
    pub fn stage_pointer_move(&mut self, step_along_x: f32, dy: f32) {
        let (x, y) = self.queued_pointer_move.unwrap_or((0.0, 0.0));
        self.queued_pointer_move = Some((x + step_along_x, y + dy));
    }

    /// The mouse pixels queued since the last take.
    pub fn take_pointer_move(&mut self) -> (f32, f32) {
        self.queued_pointer_move.take().unwrap_or((0.0, 0.0))
    }

    pub fn pointer_velocity(&self) -> Option<(f32, f32)> {
        self.pointer_velocity
    }

    /// Moves the mouse this much every frame; zero stops it.
    pub fn assign_pointer_velocity(&mut self, step_along_x: f32, dy: f32) {
        self.pointer_velocity = (step_along_x != 0.0 || dy != 0.0).then_some((step_along_x, dy));
    }

    pub fn is_down(&self, control: &str) -> bool {
        input_id(control).is_some_and(|id| self.held.contains_key(&id))
    }

    /// The ids of the held inputs, lowest first.
    pub fn ids(&self) -> impl Iterator<Item = u32> {
        self.held.keys().copied()
    }

    /// The names of the held inputs, lowest id first.
    pub fn names(&self) -> impl Iterator<Item = &'static str> {
        self.ids().filter_map(action_label)
    }
}

/// Keys a script holds down: the key, the seconds left on it, and whether it
/// has been pressed yet. They go through the binds like a player's keyboard.
#[derive(Resource, Debug, Default, Clone)]
pub struct ScriptedKeys(pub Vec<(KeyCode, f32, bool)>);

#[cfg(test)]
mod tests {
    use crate::synthetic_input::*;

    fn names(state: &ScriptedControlsStatus) -> Vec<&'static str> {
        state.names().collect()
    }

    #[test]
    fn a_held_input_stays_held_until_released() {
        let mut state = ScriptedControlsStatus::default();
        assert!(state.keep_down("+attack"));
        assert!(!state.keep_down("+attack"), "already held");
        state.step_timers(10.0);
        assert!(state.is_down("+attack"));
        assert!(state.let_go("+attack"));
        assert!(!state.let_go("+attack"));
        assert!(!state.is_down("+attack"));
    }

    #[test]
    fn a_name_without_its_plus_is_the_same_input() {
        let mut state = ScriptedControlsStatus::default();
        state.keep_down("attack");
        assert_eq!(names(&state), ["+attack"]);
        assert!(state.is_down("+attack") && state.is_down("ATTACK"));
    }

    #[test]
    fn the_release_half_is_the_same_input_as_the_press_half() {
        let mut state = ScriptedControlsStatus::default();
        state.keep_down("+attack");
        assert!(state.is_down("-attack"));
        assert!(state.let_go("-attack"));
        assert!(!state.is_down("+attack"));
        state.keep_down("+weapnext_inventory");
        assert!(state.is_down("-weapnext_inventory"));
        assert_eq!(names(&state), ["+weapnext_inventory"]);
    }

    #[test]
    fn an_input_with_no_pair_stands_alone() {
        let mut state = ScriptedControlsStatus::default();
        state.keep_down("inventory");
        assert_eq!(names(&state), ["inventory"]);
    }

    #[test]
    fn what_is_not_an_input_is_refused() {
        let mut state = ScriptedControlsStatus::default();
        assert!(!state.keep_down("nonsense") && !state.tap("nonsense", 1.0));
        assert!(!state.let_go("nonsense") && !state.is_down("nonsense"));
        assert!(state.ids().next().is_none());
        assert!(!names_control_order("nonsense") && names_control_order("attack"));
    }

    #[test]
    fn a_press_lets_go_when_its_time_is_up() {
        let mut state = ScriptedControlsStatus::default();
        assert!(state.tap("+attack", 0.1));
        state.step_timers(0.05);
        assert!(state.is_down("+attack"));
        state.step_timers(0.05);
        assert!(!state.is_down("+attack"));
    }

    #[test]
    fn a_long_frame_counts_for_no_more_than_the_longest_step() {
        let mut state = ScriptedControlsStatus::default();
        state.tap("+attack", 0.12);
        state.step_timers(5.0);
        assert!(state.is_down("+attack"), "5 s counted as 0.05 s");
        state.step_timers(0.0);
        state.step_timers(-1.0);
        assert!(state.is_down("+attack"), "time does not run backwards");
    }

    #[test]
    fn pressing_again_keeps_the_longer_time() {
        let mut state = ScriptedControlsStatus::default();
        state.tap("+attack", 0.3);
        state.tap("+attack", 0.1);
        for _ in 0..5 {
            state.step_timers(TAP_STEP_SECONDS_CAP);
        }
        assert!(state.is_down("+attack"), "0.25 s of 0.3 s");
        state.tap("+attack", 1.0);
        for _ in 0..5 {
            state.step_timers(TAP_STEP_SECONDS_CAP);
        }
        assert!(state.is_down("+attack"));
    }

    #[test]
    fn holding_cancels_a_press_timer_and_pressing_times_a_hold() {
        let mut state = ScriptedControlsStatus::default();
        state.tap("+attack", 0.05);
        state.keep_down("+attack");
        state.step_timers(TAP_STEP_SECONDS_CAP);
        assert!(state.is_down("+attack"));
        state.tap("+attack", 0.05);
        state.step_timers(TAP_STEP_SECONDS_CAP);
        assert!(!state.is_down("+attack"));
    }

    #[test]
    fn held_inputs_list_lowest_id_first() {
        let mut state = ScriptedControlsStatus::default();
        state.keep_down("+melee");
        state.keep_down("+attack");
        assert_eq!(names(&state), ["+attack", "+melee"]);
        let ids: Vec<u32> = state.ids().collect();
        assert!(ids[0] < ids[1]);
    }

    #[test]
    fn mouse_pixels_add_up_until_taken() {
        let mut state = ScriptedControlsStatus::default();
        assert_eq!(state.take_pointer_move(), (0.0, 0.0));
        state.stage_pointer_move(3.0, -1.0);
        state.stage_pointer_move(2.0, 4.0);
        assert_eq!(state.take_pointer_move(), (5.0, 3.0));
        assert_eq!(state.take_pointer_move(), (0.0, 0.0));
    }

    #[test]
    fn a_mouse_rate_runs_until_set_to_zero() {
        let mut state = ScriptedControlsStatus::default();
        state.assign_pointer_velocity(2.0, 0.0);
        assert_eq!(state.pointer_velocity(), Some((2.0, 0.0)));
        state.assign_pointer_velocity(0.0, 0.0);
        assert_eq!(state.pointer_velocity(), None);
    }

    #[test]
    fn clearing_lets_go_of_everything() {
        let mut state = ScriptedControlsStatus::default();
        state.keep_down("+attack");
        state.tap("+melee", 1.0);
        state.stage_pointer_move(1.0, 1.0);
        state.assign_pointer_velocity(1.0, 1.0);
        state.release_all();
        assert!(state.ids().next().is_none());
        assert_eq!((state.take_pointer_move(), state.pointer_velocity()), ((0.0, 0.0), None));
    }
}
