//! What the client holds between frames: the look angles, and every input the next command
//! is built from.

use std::collections::BTreeSet;

use bevy::prelude::Resource;
use entity_t6::event_ring::unwrap_previous;
use input_t6::{KEY_TURN_RATE, PlayerIntentFeed};
use playerstate_t6::button;

/// The view angles and the extras the camera queues for the next command.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ViewAimAccumulator {
    /// Pitch, yaw and roll as 16-bit angle units.
    pub angles: [i32; 3],
    /// bo2zm: view kick (pitch, yaw degrees) that moves the aim, queued by
    /// the camera for the next command (Black Ops II guns).
    pub kick_pending: [f32; 2],
    /// bo2mp: he looks through a Variable Zoom scope: melee changes the zoom
    /// ("While scoped in, press [{+melee}] to adjust between 2 different
    /// zoom levels") instead of knifing. Set by the camera each frame.
    pub melee_is_zoom: bool,
    /// bo2mp: melee presses taken as zoom changes, for the camera.
    pub zoom_presses: u32,
    /// bo2mp: aiming through a Hybrid Optic: Hold Breath switches optics.
    pub breath_is_switch: bool,
}

/// Keys, mouse and pad as read this frame, with the settings that scale them.
#[derive(Resource, Clone, Debug)]
pub struct PlayerControlSnapshot {
    pub client: PlayerIntentFeed,
    pub(crate) stance_life: Option<u32>,
    pub(crate) stance_event_sequence: i32,
    pub(crate) stance_events: [i32; 4],

    pub scenario_identifiers: BTreeSet<u32>,
    pub pointer_delta_horizontal: f32,
    pub pointer_delta_vertical: f32,
    pub sensitivity: f32,
    pub pointer_acceleration: f32,
    pub view_angle_multiplier: f32,

    pub stun_aim_dampening: f32,

    pub look_tilt_rate_cap: f32,

    pub look_turn_rate_cap: f32,
    pub yaw_sensitivity: f32,
    pub pitch_per_count: f32,

    pub key_yaw_speed: f32,
    pub present_tick_ms: i32,
    pub tick_span_ms: u32,

    /// Forward and right.
    pub pad_move: [f32; 2],
    /// Right and up.
    pub pad_look: [f32; 2],
    pub pad_deflection: f32,
    pub pad_sensitivity: f32,
    pub pad_ads_sensitivity: f32,
    pub pad_turn_rate: [f32; 2],
    /// Pitch and yaw, degrees.
    pub pad_look_delta: [f32; 2],
    pub pad_aim_assist: u8,
    pub pad_lockon: Option<u64>,
    pub pad_autoaim: Option<(u64, f32)>,
    pub pad_was_ads: bool,
    /// bo2mp: the player's last input was the pad (else the mouse or keyboard); it rides in
    /// the command for BO2's dual-wield triggers.
    pub last_input_pad: bool,
}

impl Default for PlayerControlSnapshot {
    fn default() -> Self {
        Self {
            client: PlayerIntentFeed::default(),
            stance_life: None,
            stance_event_sequence: 0,
            stance_events: [0; 4],
            scenario_identifiers: BTreeSet::new(),
            pointer_delta_horizontal: 0.0,
            pointer_delta_vertical: 0.0,
            sensitivity: 5.0,
            pointer_acceleration: 0.0,
            view_angle_multiplier: 1.0,
            stun_aim_dampening: 1.0,
            look_tilt_rate_cap: 0.0,
            look_turn_rate_cap: 0.0,
            yaw_sensitivity: 0.022,
            pitch_per_count: 0.022,
            key_yaw_speed: KEY_TURN_RATE,
            present_tick_ms: 16,
            tick_span_ms: 16,
            pad_move: [0.0; 2],
            pad_look: [0.0; 2],
            pad_deflection: 0.0,
            pad_sensitivity: 1.0,
            pad_ads_sensitivity: 1.0,
            pad_turn_rate: [0.0; 2],
            pad_look_delta: [0.0; 2],
            pad_aim_assist: 0,
            pad_lockon: None,
            pad_autoaim: None,
            pad_was_ads: false,
            last_input_pad: true,
        }
    }
}

/// Stance event numbers in the player state's event ring: BO2's 8, 9 and 10.
mod stance_event {
    use entity_t6::event;

    pub(super) const UPRIGHT: i32 = event::STANCE_FORCE_STAND as i32;
    pub(super) const DUCK_BUTTON_BIT: i32 = event::STANCE_FORCE_CROUCH as i32;
    pub(super) const LYING_FLAT: i32 = event::STANCE_FORCE_PRONE as i32;
}

impl PlayerControlSnapshot {
    /// Forget the keys' "was pressed" edges once this frame has used them.
    pub fn take_transitions(&mut self) {
        self.client.pressed_key_latch.forget_tap();
    }

    /// Follow the stance events in the predicted player state (8 stand, 9 crouch, 10 prone) to
    /// keep the stance latch in step, as BO2 does every frame.
    ///
    /// The state keeps the last four events in a ring keyed by sequence number. Of the four
    /// slots behind `sequence`, one counts as new when its number is past last frame's
    /// sequence, or when it is within four of it and holds a different event than last frame
    /// saw there (a correction replayed it at another number). New events are applied
    /// oldest first, so the newest one decides the latch.
    pub(crate) fn read_stance_events(&mut self, sequence: i32, events: [i32; 4]) {
        // The ring's count wraps; a last count far ahead of this one is from before the wrap.
        let last_sequence = i32::from(unwrap_previous(sequence as i16, self.stance_event_sequence as i16));
        for back in (1..=4).rev() {
            let seq = sequence.wrapping_sub(back);
            let slot = (seq & 3) as usize;
            let behind = last_sequence.wrapping_sub(seq);
            let already_seen = behind > 0 && (behind > 4 || events[slot] == self.stance_events[slot]);
            if already_seen {
                continue;
            }
            match events[slot] {
                stance_event::UPRIGHT => self.client.posture_hold = 0,
                stance_event::DUCK_BUTTON_BIT => self.client.posture_hold = button::DUCK_BUTTON_BIT as i32,
                stance_event::LYING_FLAT => self.client.posture_hold = button::LYING_FLAT as i32,
                _ => {}
            }
        }
        self.stance_event_sequence = sequence;
        self.stance_events = events;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latched(latch: u32, sequence: i32, events: [i32; 4]) -> PlayerControlSnapshot {
        let mut input = PlayerControlSnapshot::default();
        input.client.posture_hold = latch as i32;
        input.stance_event_sequence = sequence;
        input.stance_events = events;
        input
    }

    #[test]
    fn the_defaults_are_a_mouse_player_with_the_pad_as_last_input() {
        let input = PlayerControlSnapshot::default();
        assert_eq!((input.sensitivity, input.view_angle_multiplier), (5.0, 1.0));
        assert_eq!((input.present_tick_ms, input.tick_span_ms), (16, 16));
        assert!(input.last_input_pad);
        assert!(input.scenario_identifiers.is_empty());
        assert_eq!(input.stance_events, [0; 4]);
        assert_eq!(ViewAimAccumulator::default().angles, [0; 3]);
    }

    #[test]
    fn a_new_stand_event_clears_the_latch() {
        let mut input = latched(button::DUCK_BUTTON_BIT, 9, [0; 4]);
        input.read_stance_events(10, [0, stance_event::UPRIGHT, 0, 0]);
        assert_eq!(input.client.posture_hold, 0);
        assert_eq!(input.stance_event_sequence, 10);
        assert_eq!(input.stance_events, [0, stance_event::UPRIGHT, 0, 0]);
    }

    #[test]
    fn new_crouch_and_prone_events_set_the_latch() {
        let mut input = latched(0, 9, [0; 4]);
        input.read_stance_events(10, [0, stance_event::DUCK_BUTTON_BIT, 0, 0]);
        assert_eq!(input.client.posture_hold, button::DUCK_BUTTON_BIT as i32);
        input.read_stance_events(11, [0, stance_event::DUCK_BUTTON_BIT, stance_event::LYING_FLAT, 0]);
        assert_eq!(input.client.posture_hold, button::LYING_FLAT as i32);
    }

    #[test]
    fn events_that_are_not_stance_events_leave_the_latch_alone() {
        let mut input = latched(button::LYING_FLAT, 9, [0; 4]);
        input.read_stance_events(10, [0, 3, 0, 0]);
        assert_eq!(input.client.posture_hold, button::LYING_FLAT as i32);
    }

    #[test]
    fn when_several_events_are_new_the_newest_wins() {
        let mut input = latched(0, 10, [0; 4]);
        // Sequence 14 reads slots for 10, 11, 12, 13: slot 0 holds crouch at 12,
        // slot 1 holds stand at 13 -- both past what was seen, the later one is last.
        input.read_stance_events(14, [stance_event::DUCK_BUTTON_BIT, stance_event::UPRIGHT, 0, 0]);
        assert_eq!(input.client.posture_hold, 0);
        let mut input = latched(0, 10, [0; 4]);
        input.read_stance_events(14, [stance_event::UPRIGHT, stance_event::DUCK_BUTTON_BIT, 0, 0]);
        assert_eq!(input.client.posture_hold, button::DUCK_BUTTON_BIT as i32);
    }

    #[test]
    fn a_changed_event_inside_the_last_four_frames_counts_again() {
        // Last frame ran to 10; a correction replays the stand event at number 8.
        let mut input = latched(button::DUCK_BUTTON_BIT, 10, [0; 4]);
        input.read_stance_events(9, [stance_event::UPRIGHT, 0, 0, 0]);
        assert_eq!(input.client.posture_hold, 0);
        assert_eq!(input.stance_event_sequence, 9);
    }

    #[test]
    fn the_stand_event_a_sprint_sends_never_goes_prone() {
        // Sprinting stands the player up every frame; the client must read that as
        // "stand" (latch off), not as the prone event, or Shift lies the player down.
        let mut input = PlayerControlSnapshot::default();
        input.stance_event_sequence = 0;
        let stand = stance_event::UPRIGHT;
        input.read_stance_events(1, [stand, 0, 0, 0]);
        assert_eq!(input.client.posture_hold, 0);
        input.read_stance_events(2, [stand, stand, 0, 0]);
        assert_eq!(input.client.posture_hold, 0);
    }

    #[test]
    fn a_stance_event_added_as_the_count_wraps_is_still_read() {
        // The ring's count runs 254, 255, then back to 0: the crouch event sits in slot 3 (number 255).
        let mut input = latched(0, 255, [0; 4]);
        input.read_stance_events(0, [0, 0, 0, stance_event::DUCK_BUTTON_BIT]);
        assert_eq!(input.client.posture_hold, button::DUCK_BUTTON_BIT as i32);
        assert_eq!(input.stance_event_sequence, 0);
    }

    #[test]
    fn events_already_seen_do_not_fire_again() {
        let mut input = latched(button::DUCK_BUTTON_BIT, 10, [0, stance_event::UPRIGHT, 0, 0]);
        input.read_stance_events(10, [0, stance_event::UPRIGHT, 0, 0]);
        assert_eq!(input.client.posture_hold, button::DUCK_BUTTON_BIT as i32);
    }
}
