//! Inputs the console holds down on a script's behalf, and the mouse movement
//! it feeds the client.

use std::collections::BTreeMap;

use bevy::{input::keyboard::KeyCode, prelude::Resource};
use input_t6::{HOLD_PAIR_LIMIT, WEAPNEXT_INVENTORY_DOWN, command_id_lookup, command_name};

/// How long `press` holds an input when a script gives no time.
pub const PRESS_SECONDS: f32 = 0.15;

/// The longest step `tick` counts, so a hitch cannot cut a press short.
pub const PRESS_TICK_DT_MAX: f32 = 0.05;

/// Whether `word` names an input a script can hold or bind.
pub(crate) fn is_bind_command(word: &str) -> bool {
    command_id_lookup(word).is_some()
}

/// The id of the input `name` stands for. `-forward` is the release half of
/// `+forward` and means the same input; `inventory` stands alone.
fn input_id(name: &str) -> Option<u32> {
    let id = command_id_lookup(name)?;
    let release_half = (id < HOLD_PAIR_LIMIT && id % 2 == 0) || id == WEAPNEXT_INVENTORY_DOWN + 1;
    Some(if release_half { id - 1 } else { id })
}

/// The inputs a script is holding, and the mouse it is moving.
#[derive(Resource, Debug, Default, Clone)]
pub struct ConsoleInputState {
    /// Each held input, with the seconds it has left if it was only pressed.
    held: BTreeMap<u32, Option<f32>>,
    /// Mouse pixels waiting for the client to take them.
    pending_mouse: Option<(f32, f32)>,
    /// Mouse pixels added every frame.
    mouse_rate: Option<(f32, f32)>,
}

impl ConsoleInputState {
    /// Holds `input` until it is released. False if it is not an input, or was
    /// held already.
    pub fn hold(&mut self, input: &str) -> bool {
        let Some(id) = input_id(input) else {
            return false;
        };
        self.held.insert(id, None).is_none()
    }

    /// Holds `input` for `seconds` more than it has left. False if it is not an
    /// input.
    pub fn press(&mut self, input: &str, seconds: f32) -> bool {
        let Some(id) = input_id(input) else {
            return false;
        };
        let left = match self.held.get(&id) {
            Some(Some(left)) => left.max(seconds),
            _ => seconds,
        };
        self.held.insert(id, Some(left));
        true
    }

    /// Counts `dt` off every pressed input, and lets go of those out of time.
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, PRESS_TICK_DT_MAX);
        self.held.retain(|_, left| match left {
            Some(left) => {
                *left -= dt;
                *left > 0.0
            }
            None => true,
        });
    }

    /// Lets go of `input`. False if it is not an input, or was not held.
    pub fn release(&mut self, input: &str) -> bool {
        input_id(input).is_some_and(|id| self.held.remove(&id).is_some())
    }

    /// Lets go of everything and stops the mouse.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Adds mouse pixels for the client to take next.
    pub fn queue_mouse(&mut self, dx: f32, dy: f32) {
        let (x, y) = self.pending_mouse.unwrap_or((0.0, 0.0));
        self.pending_mouse = Some((x + dx, y + dy));
    }

    /// The mouse pixels queued since the last take.
    pub fn take_mouse(&mut self) -> (f32, f32) {
        self.pending_mouse.take().unwrap_or((0.0, 0.0))
    }

    pub fn mouse_rate(&self) -> Option<(f32, f32)> {
        self.mouse_rate
    }

    /// Moves the mouse this much every frame; zero stops it.
    pub fn set_mouse_rate(&mut self, dx: f32, dy: f32) {
        self.mouse_rate = (dx != 0.0 || dy != 0.0).then_some((dx, dy));
    }

    pub fn held(&self, input: &str) -> bool {
        input_id(input).is_some_and(|id| self.held.contains_key(&id))
    }

    /// The ids of the held inputs, lowest first.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.held.keys().copied()
    }

    /// The names of the held inputs, lowest id first.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.ids().filter_map(command_name)
    }
}

/// Keys a script holds down: the key, the seconds left on it, and whether it
/// has been pressed yet. They go through the binds like a player's keyboard.
#[derive(Resource, Debug, Default, Clone)]
pub struct ScriptedKeys(pub Vec<(KeyCode, f32, bool)>);

#[cfg(test)]
mod tests {
    use super::*;

    fn names(state: &ConsoleInputState) -> Vec<&'static str> {
        state.names().collect()
    }

    #[test]
    fn a_held_input_stays_held_until_released() {
        let mut state = ConsoleInputState::default();
        assert!(state.hold("+attack"));
        assert!(!state.hold("+attack"), "already held");
        state.tick(10.0);
        assert!(state.held("+attack"));
        assert!(state.release("+attack"));
        assert!(!state.release("+attack"));
        assert!(!state.held("+attack"));
    }

    #[test]
    fn a_name_without_its_plus_is_the_same_input() {
        let mut state = ConsoleInputState::default();
        state.hold("attack");
        assert_eq!(names(&state), ["+attack"]);
        assert!(state.held("+attack") && state.held("ATTACK"));
    }

    #[test]
    fn the_release_half_is_the_same_input_as_the_press_half() {
        let mut state = ConsoleInputState::default();
        state.hold("+attack");
        assert!(state.held("-attack"));
        assert!(state.release("-attack"));
        assert!(!state.held("+attack"));
        state.hold("+weapnext_inventory");
        assert!(state.held("-weapnext_inventory"));
        assert_eq!(names(&state), ["+weapnext_inventory"]);
    }

    #[test]
    fn an_input_with_no_pair_stands_alone() {
        let mut state = ConsoleInputState::default();
        state.hold("inventory");
        assert_eq!(names(&state), ["inventory"]);
    }

    #[test]
    fn what_is_not_an_input_is_refused() {
        let mut state = ConsoleInputState::default();
        assert!(!state.hold("nonsense") && !state.press("nonsense", 1.0));
        assert!(!state.release("nonsense") && !state.held("nonsense"));
        assert!(state.ids().next().is_none());
        assert!(!is_bind_command("nonsense") && is_bind_command("attack"));
    }

    #[test]
    fn a_press_lets_go_when_its_time_is_up() {
        let mut state = ConsoleInputState::default();
        assert!(state.press("+attack", 0.1));
        state.tick(0.05);
        assert!(state.held("+attack"));
        state.tick(0.05);
        assert!(!state.held("+attack"));
    }

    #[test]
    fn a_long_frame_counts_for_no_more_than_the_longest_step() {
        let mut state = ConsoleInputState::default();
        state.press("+attack", 0.12);
        state.tick(5.0);
        assert!(state.held("+attack"), "5 s counted as 0.05 s");
        state.tick(0.0);
        state.tick(-1.0);
        assert!(state.held("+attack"), "time does not run backwards");
    }

    #[test]
    fn pressing_again_keeps_the_longer_time() {
        let mut state = ConsoleInputState::default();
        state.press("+attack", 0.3);
        state.press("+attack", 0.1);
        for _ in 0..5 {
            state.tick(PRESS_TICK_DT_MAX);
        }
        assert!(state.held("+attack"), "0.25 s of 0.3 s");
        state.press("+attack", 1.0);
        for _ in 0..5 {
            state.tick(PRESS_TICK_DT_MAX);
        }
        assert!(state.held("+attack"));
    }

    #[test]
    fn holding_cancels_a_press_timer_and_pressing_times_a_hold() {
        let mut state = ConsoleInputState::default();
        state.press("+attack", 0.05);
        state.hold("+attack");
        state.tick(PRESS_TICK_DT_MAX);
        assert!(state.held("+attack"));
        state.press("+attack", 0.05);
        state.tick(PRESS_TICK_DT_MAX);
        assert!(!state.held("+attack"));
    }

    #[test]
    fn held_inputs_list_lowest_id_first() {
        let mut state = ConsoleInputState::default();
        state.hold("+melee");
        state.hold("+attack");
        assert_eq!(names(&state), ["+attack", "+melee"]);
        let ids: Vec<u32> = state.ids().collect();
        assert!(ids[0] < ids[1]);
    }

    #[test]
    fn mouse_pixels_add_up_until_taken() {
        let mut state = ConsoleInputState::default();
        assert_eq!(state.take_mouse(), (0.0, 0.0));
        state.queue_mouse(3.0, -1.0);
        state.queue_mouse(2.0, 4.0);
        assert_eq!(state.take_mouse(), (5.0, 3.0));
        assert_eq!(state.take_mouse(), (0.0, 0.0));
    }

    #[test]
    fn a_mouse_rate_runs_until_set_to_zero() {
        let mut state = ConsoleInputState::default();
        state.set_mouse_rate(2.0, 0.0);
        assert_eq!(state.mouse_rate(), Some((2.0, 0.0)));
        state.set_mouse_rate(0.0, 0.0);
        assert_eq!(state.mouse_rate(), None);
    }

    #[test]
    fn clearing_lets_go_of_everything() {
        let mut state = ConsoleInputState::default();
        state.hold("+attack");
        state.press("+melee", 1.0);
        state.queue_mouse(1.0, 1.0);
        state.set_mouse_rate(1.0, 1.0);
        state.clear();
        assert!(state.ids().next().is_none());
        assert_eq!((state.take_mouse(), state.mouse_rate()), ((0.0, 0.0), None));
    }
}
