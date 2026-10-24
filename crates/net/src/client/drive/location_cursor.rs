//! The cursor on the map overlay while the player is choosing where a scorestreak should go.
//! The mouse moves it, and with the aim key held it turns instead into a direction arrow.

use bevy::prelude::*;
use playerstate_t6::game::{UserCmd, buttons};

/// How far a mouse count moves the cursor, before the player's own sensitivity is applied.
const MOUSE_REACH: f32 = 0.6;
const COUNTS_TO_MAP: f32 = 0.002;

/// Where the player has put the cursor on the map, and where he is pointing from there. `at`
/// and `aim` are fractions of the overlay, and `yaw` is counterclockwise from map up, in degrees.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LocationCursor {
    pub at: [f32; 2],
    pub aim: [f32; 2],
    pub yaw: f32,
    pub directing: bool,
}

impl Default for LocationCursor {
    fn default() -> Self {
        Self {
            at: [0.5, 0.5],
            aim: [0.5, 0.5],
            yaw: 0.0,
            directing: false,
        }
    }
}

fn nudged(point: [f32; 2], by: [f32; 2]) -> [f32; 2] {
    [(point[0] + by[0]).clamp(0.0, 1.0), (point[1] + by[1]).clamp(0.0, 1.0)]
}

/// A map fraction as the signed byte the command carries (-128 at the edge, 127 at the far edge).
fn axis_byte(fraction: f32) -> u8 {
    let signed = ((fraction * 255.0 - 128.0).round() as i32).clamp(-128, 127);
    signed as i8 as u8
}

fn yaw_byte(degrees: f32) -> u8 {
    (degrees * (256.0 / 360.0)).round() as i32 as u8
}

impl LocationCursor {
    /// Applies this frame's mouse movement and returns the three bytes the command carries:
    /// the cursor's x and y, and the direction. Without `directing` the mouse drags the cursor
    /// and the arrow follows it; with it the cursor stays and the arrow swings around it.
    pub(super) fn step(&mut self, sensitivity: f32, mouse: (f32, f32), directing: bool) -> [u8; 3] {
        let scale = MOUSE_REACH * sensitivity * COUNTS_TO_MAP;
        let moved = [mouse.0 * scale, mouse.1 * scale];
        self.directing = directing;
        if directing {
            self.aim = nudged(self.aim, moved);
            let away = [self.aim[0] - self.at[0], self.aim[1] - self.at[1]];
            if away != [0.0, 0.0] {
                self.yaw = (-away[0]).atan2(-away[1]).to_degrees().rem_euclid(360.0);
            }
        } else {
            self.at = nudged(self.at, moved);
            self.aim = self.at;
        }
        [axis_byte(self.at[0]), axis_byte(self.at[1]), yaw_byte(self.yaw)]
    }
}

/// Rewrites a command that was built while the map overlay is up. The mouse moved the cursor,
/// so the command carries where the cursor is; the keys are cut down to the stance plus the one
/// button that says "pick here" or "never mind". `mouse` is `None` once the overlay is gone, and
/// the cursor then forgets where it was. `already_picking` is whether the previous command was
/// already a pick, so that only the first one of a held press is logged.
pub(super) fn steer_command(
    cursor: &mut LocationCursor,
    cmd: &mut UserCmd,
    sensitivity: f32,
    mouse: Option<(f32, f32)>,
    choose_direction: Option<bool>,
    already_picking: bool,
) {
    let Some(mouse) = mouse else {
        *cursor = LocationCursor::default();
        return;
    };
    let held = cmd.buttons;
    let picks_direction = choose_direction == Some(true);
    let directing = picks_direction && held & buttons::ADS != 0;
    let cancel_keys = if picks_direction {
        buttons::MELEE_CHARGE
    } else {
        buttons::ADS | buttons::MELEE_CHARGE
    };

    cmd.selected_location = cursor.step(sensitivity, mouse, directing);
    if !picks_direction {
        cmd.selected_location[2] = 0;
    }
    cmd.buttons &= buttons::CROUCH | buttons::PRONE;
    if held & buttons::ATTACK != 0 {
        if !already_picking {
            diag::info!(Net, "bo2mp location: pick at {:?}", cmd.selected_location);
        }
        cmd.buttons |= buttons::LOCATION_SELECT;
    } else if held & cancel_keys != 0 {
        cmd.buttons |= buttons::LOCATION_CANCEL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR: f32 = 1.0e6;

    #[test]
    fn the_cursor_starts_in_the_middle_of_the_map() {
        let cursor = LocationCursor::default();
        assert_eq!(cursor.at, [0.5, 0.5]);
        assert_eq!(cursor.aim, [0.5, 0.5]);
        assert_eq!(cursor.yaw, 0.0);
        assert!(!cursor.directing);
    }

    #[test]
    fn a_still_mouse_reports_the_middle_as_two_bytes() {
        let mut cursor = LocationCursor::default();
        assert_eq!(cursor.step(5.0, (0.0, 0.0), false), [255, 255, 0]);
    }

    #[test]
    fn dragging_the_cursor_to_the_corners_hits_the_byte_limits() {
        let mut cursor = LocationCursor::default();
        assert_eq!(cursor.step(5.0, (FAR, FAR), false)[..2], [127, 127]);
        assert_eq!(cursor.at, [1.0, 1.0]);
        assert_eq!(cursor.step(5.0, (-FAR, -FAR), false)[..2], [128, 128]);
        assert_eq!(cursor.at, [0.0, 0.0]);
    }

    #[test]
    fn a_higher_sensitivity_moves_the_cursor_further() {
        let (mut slow, mut quick) = (LocationCursor::default(), LocationCursor::default());
        slow.step(1.0, (100.0, 0.0), false);
        quick.step(4.0, (100.0, 0.0), false);
        assert!((slow.at[0] - 0.5 - 0.12).abs() < 1.0e-5);
        assert!((quick.at[0] - 0.5 - 0.48).abs() < 1.0e-5);
    }

    #[test]
    fn without_directing_the_arrow_follows_the_cursor() {
        let mut cursor = LocationCursor::default();
        cursor.aim = [0.9, 0.1];
        cursor.step(5.0, (50.0, 0.0), false);
        assert_eq!(cursor.aim, cursor.at);
        assert!(!cursor.directing);
    }

    #[test]
    fn directing_turns_the_arrow_and_leaves_the_cursor_where_it_is() {
        let mut cursor = LocationCursor::default();
        let bytes = cursor.step(5.0, (FAR, 0.0), true);
        assert!(cursor.directing);
        assert_eq!(cursor.at, [0.5, 0.5]);
        assert_eq!(cursor.aim, [1.0, 0.5]);
        assert!((cursor.yaw - 270.0).abs() < 1.0e-3, "yaw was {}", cursor.yaw);
        assert_eq!(bytes, [255, 255, 192]);
    }

    #[test]
    fn directing_straight_up_the_map_points_the_arrow_at_zero() {
        let mut cursor = LocationCursor::default();
        let bytes = cursor.step(5.0, (0.0, -FAR), true);
        assert_eq!(cursor.aim, [0.5, 0.0]);
        assert_eq!(bytes[2], 0);
    }

    #[test]
    fn the_arrow_keeps_its_last_direction_while_it_sits_on_the_cursor() {
        let mut cursor = LocationCursor::default();
        cursor.step(5.0, (FAR, 0.0), true);
        let turned = cursor.yaw;
        cursor.aim = cursor.at;
        cursor.step(5.0, (0.0, 0.0), true);
        assert_eq!(cursor.yaw, turned);
    }

    const FIRE: u32 = buttons::ATTACK;

    fn command_holding(keys: u32) -> UserCmd {
        UserCmd { buttons: keys, ..Default::default() }
    }

    fn steered(keys: u32, choose_direction: Option<bool>, already_picking: bool) -> (UserCmd, LocationCursor) {
        let mut cursor = LocationCursor { yaw: 90.0, ..Default::default() };
        let mut cmd = command_holding(keys);
        steer_command(&mut cursor, &mut cmd, 5.0, Some((0.0, 0.0)), choose_direction, already_picking);
        (cmd, cursor)
    }

    #[test]
    fn without_the_overlay_the_cursor_forgets_and_the_command_is_left_alone() {
        let mut cursor = LocationCursor::default();
        cursor.step(5.0, (300.0, 300.0), false);
        let mut cmd = command_holding(FIRE | buttons::RELOAD);
        steer_command(&mut cursor, &mut cmd, 5.0, None, Some(true), false);
        assert_eq!(cursor, LocationCursor::default());
        assert_eq!(cmd.buttons, FIRE | buttons::RELOAD);
        assert_eq!(cmd.selected_location, [0; 3]);
    }

    #[test]
    fn only_the_stance_and_the_pick_survive_on_the_overlay() {
        let keys = buttons::CROUCH | buttons::PRONE | buttons::RELOAD | buttons::ADS;
        let (cmd, _) = steered(keys, Some(false), false);
        // ADS cancels a fixed-direction pick, so the stance stays and the cancel is added.
        assert_eq!(cmd.buttons, buttons::CROUCH | buttons::PRONE | buttons::LOCATION_CANCEL);
        let (cmd, _) = steered(buttons::RELOAD, Some(false), false);
        assert_eq!(cmd.buttons, 0);
    }

    #[test]
    fn the_fire_key_picks_the_spot() {
        let (cmd, _) = steered(FIRE | buttons::CROUCH, Some(false), false);
        assert_eq!(cmd.buttons, buttons::CROUCH | buttons::LOCATION_SELECT);
    }

    #[test]
    fn a_pick_beats_a_cancel_held_at_the_same_time() {
        let (cmd, _) = steered(FIRE | buttons::MELEE_CHARGE, Some(false), true);
        assert_eq!(cmd.buttons, buttons::LOCATION_SELECT);
    }

    #[test]
    fn when_the_player_chooses_the_direction_aiming_turns_the_arrow_instead_of_cancelling() {
        let (cmd, cursor) = steered(buttons::ADS, Some(true), false);
        assert!(cursor.directing);
        assert_eq!(cmd.buttons, 0);
        let (cmd, _) = steered(buttons::MELEE_CHARGE, Some(true), false);
        assert_eq!(cmd.buttons, buttons::LOCATION_CANCEL);
    }

    #[test]
    fn the_direction_byte_is_only_sent_when_the_player_chooses_it() {
        let (fixed, _) = steered(0, Some(false), false);
        assert_eq!(fixed.selected_location[2], 0);
        let (chosen, _) = steered(0, Some(true), false);
        assert_eq!(chosen.selected_location[2], 64);
    }

    #[test]
    fn the_mouse_moves_the_cursor_that_the_command_reports() {
        let mut cursor = LocationCursor::default();
        let mut cmd = command_holding(0);
        steer_command(&mut cursor, &mut cmd, 5.0, Some((FAR, 0.0)), Some(false), false);
        assert_eq!(cursor.at, [1.0, 0.5]);
        assert_eq!(cmd.selected_location, [127, 255, 0]);
    }
}
