use playerstate_t6::{UserCmd, button};

use crate::client::{CROUCH, PRONE, jam_chance};
use crate::*;

const FRAME: u32 = 16;

/// A client with each `(key, command)` bound.
fn bound(binds: &[(usize, &str)]) -> ClientInput {
    let mut client = ClientInput::default();
    for &(key, command) in binds {
        client.keys[key].binding = command_id_lookup(command).expect(command);
    }
    client
}

fn down(client: &mut ClientInput, key: usize, now: i32) {
    key_event(client, key, true, now, FRAME);
}

fn up(client: &mut ClientInput, key: usize, now: i32) {
    key_event(client, key, false, now, FRAME);
}

fn sampled_has(client: &mut ClientInput, now: i32, bit: u32) -> bool {
    sample_move(client, now, FRAME).0.test(bit)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

mod table {
    use super::*;

    #[test]
    fn ids_line_up_with_the_game() {
        assert_eq!(HOLD_PAIR_LIMIT, 0x41);
        assert_eq!(command_name(1), Some("+attack"));
        assert_eq!(command_name(2), Some("-attack"));
        assert_eq!(command_name(64), Some("-talk"));
        assert_eq!(
            command_name(WEAPNEXT_INVENTORY_DOWN),
            Some("+weapnext_inventory")
        );
        assert_eq!(command_name(79), Some("-weapnext_inventory"));
        assert_eq!(command_name(INVENTORY_COMMAND), Some("inventory"));
        assert_eq!(command_name(0), None);
        assert_eq!(command_name(81), None);
        assert_eq!(command_names().count(), 80);
    }

    #[test]
    fn lookup_takes_any_case_and_a_bare_hold_name() {
        assert_eq!(command_id_lookup("+attack"), Some(1));
        assert_eq!(command_id_lookup("  +ATTACK "), Some(1));
        assert_eq!(command_id_lookup("attack"), Some(1));
        assert_eq!(command_id_lookup("-attack"), Some(2));
        assert_eq!(command_id_lookup("TOGGLECROUCH"), Some(72));
        assert_eq!(
            command_id_lookup("actionslot 3"),
            command_id_lookup("+actionslot 3")
        );
        assert_eq!(command_id_lookup(""), None);
        assert_eq!(command_id_lookup("+weapnext"), None);
        assert_eq!(command_id_lookup("-nothing"), None);
    }

    #[test]
    fn only_the_down_half_of_a_pair_has_an_up_command() {
        assert_eq!(key_up_command_id(1), Some(2));
        assert_eq!(key_up_command_id(63), Some(64));
        assert_eq!(key_up_command_id(2), None);
        assert_eq!(key_up_command_id(0), None);
        assert_eq!(key_up_command_id(72), None);
        assert_eq!(key_up_command_id(WEAPNEXT_INVENTORY_DOWN), Some(79));
        assert_eq!(key_up_command_id(INVENTORY_COMMAND), None);
    }
}

mod buttons {
    use super::*;

    #[test]
    fn a_button_lets_go_with_its_last_key() {
        let mut c = bound(&[(1, "+forward"), (2, "+forward")]);
        down(&mut c, 1, 100);
        down(&mut c, 2, 105);
        up(&mut c, 1, 110);
        assert!(c.kb.forward.active);
        up(&mut c, 2, 120);
        assert!(!c.kb.forward.active);
        assert!(c.kb.forward.was_pressed);
        c.kb.clear_was_pressed();
        assert!(!c.kb.forward.was_pressed);
    }

    #[test]
    fn a_short_tap_moves_for_its_share_of_the_frame() {
        let mut c = bound(&[(1, "+forward")]);
        down(&mut c, 1, 100);
        up(&mut c, 1, 108);
        let (_, axes) = sample_move(&mut c, 116, FRAME);
        assert!(close(axes.forward, 0.5));
        let (_, axes) = sample_move(&mut c, 132, FRAME);
        assert!(close(axes.forward, 0.0));
    }

    #[test]
    fn held_keys_move_full_and_cancel_out() {
        let mut c = bound(&[(1, "+forward"), (2, "+back"), (3, "+moveright")]);
        down(&mut c, 1, 100);
        down(&mut c, 3, 100);
        let (_, axes) = sample_move(&mut c, 104, FRAME);
        assert!(close(axes.forward, 1.0) && close(axes.right, 1.0));
        down(&mut c, 2, 104);
        let (_, axes) = sample_move(&mut c, 120, FRAME);
        assert!(close(axes.forward, 0.0));
    }

    #[test]
    fn the_overlay_names_what_is_held() {
        let mut c = bound(&[(1, "+forward"), (2, "+strafe")]);
        down(&mut c, 1, 100);
        down(&mut c, 2, 100);
        let mut names = alloc::vec::Vec::new();
        c.kb.visit_active_names(|n| names.push(n));
        assert_eq!(names, ["+forward"]);
    }

    #[test]
    fn keys_off_the_board_are_ignored() {
        let mut c = ClientInput::default();
        down(&mut c, KEY_COUNT, 100);
        input_cmd(&mut c, 1, SCRIPT_KEYNUM, 100, FRAME);
        assert!(c.kb.attack.active);
        input_cmd(&mut c, 2, SCRIPT_KEYNUM, 110, FRAME);
        assert!(!c.kb.attack.active);
    }

    #[test]
    fn the_speed_key_flips_aiming() {
        let mut c = bound(&[(1, "+speed_throw"), (2, "toggleads")]);
        down(&mut c, 1, 100);
        assert!(sampled_has(&mut c, 110, button::ADS));
        up(&mut c, 1, 120);
        down(&mut c, 2, 130);
        assert!(c.using_ads);
        assert!(sampled_has(&mut c, 140, button::ADS));
        down(&mut c, 1, 150);
        assert!(!c.using_ads, "the speed key leaves the toggle");
        set_ads(&mut c, true);
        assert!(!sampled_has(&mut c, 160, button::ADS));
    }
}

mod double_tap {
    use super::*;

    fn tap(c: &mut ClientInput, key: usize, at: i32, held: i32) {
        down(c, key, at);
        up(c, key, at + held);
    }

    #[test]
    fn two_quick_taps_send_it_once() {
        let mut c = bound(&[(1, "+activate"), (2, "+reload")]);
        tap(&mut c, 1, 1000, 100);
        assert!(!sampled_has(&mut c, 1110, button::DOUBLE_TAP));
        tap(&mut c, 2, 1300, 100);
        assert!(sampled_has(&mut c, 1410, button::DOUBLE_TAP));
        assert!(!sampled_has(&mut c, 1420, button::DOUBLE_TAP));
    }

    #[test]
    fn a_late_second_press_starts_over() {
        let mut c = bound(&[(1, "+usereload")]);
        tap(&mut c, 1, 1000, 100);
        tap(&mut c, 1, 1500, 100);
        assert!(!sampled_has(&mut c, 1610, button::DOUBLE_TAP));
        tap(&mut c, 1, 1700, 50);
        assert!(sampled_has(&mut c, 1760, button::DOUBLE_TAP));
    }

    #[test]
    fn a_long_hold_is_not_a_tap() {
        let mut c = bound(&[(1, "+activate")]);
        tap(&mut c, 1, 1000, 100);
        tap(&mut c, 1, 1200, 500);
        assert!(!sampled_has(&mut c, 1710, button::DOUBLE_TAP));
    }
}

mod stance {
    use super::*;

    fn pressed(c: &mut ClientInput, at: i32, held: Option<i32>) {
        down(c, 1, at);
        if let Some(held) = held {
            up(c, 1, at + held);
        }
    }

    #[test]
    fn the_sprint_key_sprints_and_never_goes_prone() {
        for command in ["+breath_sprint", "+sprint"] {
            let mut c = bound(&[(1, command), (2, "toggleprone"), (3, "+stance")]);
            down(&mut c, 1, 1000);
            for now in [1016, 1300, 2000] {
                let (bits, _) = sample_move(&mut c, now, FRAME);
                assert!(bits.test(button::SPRINT), "{command} sprints at {now}");
                assert!(!bits.test(button::PRONE), "{command} is not prone at {now}");
                assert!(!bits.test(button::CROUCH), "{command} is not crouched at {now}");
            }
            assert_eq!(c.stance_latch, 0);
            up(&mut c, 1, 2100);
            c.kb.clear_was_pressed();
            assert!(!sampled_has(&mut c, 2200, button::SPRINT));
            // Ctrl (toggleprone) still toggles prone by itself.
            down(&mut c, 2, 2300);
            assert!(sampled_has(&mut c, 2316, button::PRONE));
        }
    }

    #[test]
    fn a_tap_crouches_and_another_stands() {
        let mut c = bound(&[(1, "+stance")]);
        pressed(&mut c, 1000, Some(100));
        assert_eq!(c.stance_latch, CROUCH);
        assert!(sampled_has(&mut c, 1110, button::CROUCH));
        pressed(&mut c, 1200, Some(100));
        assert_eq!(c.stance_latch, 0);
    }

    #[test]
    fn a_long_press_goes_prone_and_another_stands() {
        let mut c = bound(&[(1, "+stance")]);
        pressed(&mut c, 1000, None);
        assert_eq!(c.stance_latch, CROUCH);
        sample_move(&mut c, 1299, FRAME);
        assert_eq!(c.stance_latch, CROUCH);
        assert!(sampled_has(&mut c, 1300, button::PRONE));
        up(&mut c, 1, 1400);
        assert_eq!(c.stance_latch, PRONE);
        pressed(&mut c, 2000, None);
        sample_move(&mut c, 2300, FRAME);
        up(&mut c, 1, 2350);
        assert_eq!(c.stance_latch, 0);
    }

    #[test]
    fn sprinting_a_short_hold_dives_and_a_tap_crouches() {
        let mut c = bound(&[(1, "+stance")]);
        c.sprinting = true;
        pressed(&mut c, 1000, None);
        assert_eq!(c.stance_latch, 0, "a sprinting press waits");
        sample_move(&mut c, 1200, FRAME);
        assert_eq!(c.stance_latch, PRONE);
        up(&mut c, 1, 1250);

        let mut c = bound(&[(1, "+stance")]);
        c.sprinting = true;
        pressed(&mut c, 1000, Some(100));
        assert_eq!(c.stance_latch, CROUCH);
    }

    #[test]
    fn a_gun_that_forbids_prone_keeps_the_stance() {
        let mut c = bound(&[(1, "+stance")]);
        c.sprinting = true;
        c.weapon_blocks_prone = true;
        pressed(&mut c, 1000, Some(100));
        assert_eq!(c.stance_latch, 0);
    }

    #[test]
    fn a_held_crouch_key_wins_and_says_so() {
        let mut c = bound(&[(1, "+stance"), (2, "+movedown")]);
        down(&mut c, 2, 1000);
        pressed(&mut c, 1010, Some(50));
        assert_eq!(
            c.stance_latch, 0,
            "the stance button does nothing while crouch is held"
        );
        let (bits, _) = sample_move(&mut c, 1100, FRAME);
        assert!(bits.test(button::CROUCH) && bits.test(button::STANCE_HELD));
    }

    #[test]
    fn jump_stands() {
        let mut c = bound(&[(1, "+gostand"), (2, "gocrouch")]);
        down(&mut c, 2, 1000);
        assert_eq!(c.stance_latch, CROUCH);
        down(&mut c, 1, 1010);
        assert!(sampled_has(&mut c, 1020, button::JUMP));
        assert_eq!(c.stance_latch, 0);
    }

    #[test]
    fn lower_keys_are_the_crouch_and_prone_ones() {
        let mut c = bound(&[(1, "toggleprone"), (2, "+gostand")]);
        assert!(!lower_key_held(&c));
        down(&mut c, 2, 1000);
        assert!(!lower_key_held(&c));
        down(&mut c, 1, 1000);
        assert!(lower_key_held(&c));
        assert_eq!(c.stance_latch, PRONE);
        up(&mut c, 1, 1100);
        assert!(!lower_key_held(&c));
    }
}

mod weapnext {
    use super::*;

    #[test]
    fn a_tap_is_the_next_weapon_on_release() {
        let mut c = bound(&[(1, "+weapnext_inventory")]);
        down(&mut c, 1, 1000);
        assert!(c.change_seat_press);
        assert!(c.weapon_cycles.is_empty());
        sample_move(&mut c, 1200, FRAME);
        up(&mut c, 1, 1240);
        assert_eq!(c.weapon_cycles, [true]);
        assert!(!c.inventory_request);
    }

    #[test]
    fn a_hold_is_the_inventory_weapon() {
        let mut c = bound(&[(1, "+weapnext_inventory")]);
        down(&mut c, 1, 1000);
        down(&mut c, 1, 1200);
        sample_move(&mut c, 1249, FRAME);
        assert!(
            !c.inventory_request,
            "a repeat press keeps the first press time"
        );
        sample_move(&mut c, 1250, FRAME);
        assert!(c.inventory_request);
        up(&mut c, 1, 1300);
        assert!(c.weapon_cycles.is_empty());
    }

    #[test]
    fn inventory_asks_at_once_and_the_others_queue() {
        let mut c = bound(&[
            (1, "inventory"),
            (2, "weapprev"),
            (3, "weapnext"),
            (4, "+actionslot 3"),
        ]);
        down(&mut c, 1, 1000);
        assert!(c.inventory_request);
        down(&mut c, 2, 1000);
        assert!(!c.change_seat_press);
        down(&mut c, 3, 1000);
        assert!(c.change_seat_press);
        assert_eq!(c.weapon_cycles, [false, true]);
        down(&mut c, 4, 1000);
        up(&mut c, 4, 1010);
        assert_eq!(c.action_slots, [2]);
    }
}

mod jam {
    use super::*;

    #[test]
    fn the_chance_grows_as_the_gap_shrinks() {
        assert_eq!(jam_chance(50, 50), 0);
        assert_eq!(jam_chance(49, 50), 24);
        assert_eq!(jam_chance(45, 50), 100);
        assert_eq!(jam_chance(0, 50), 0);
        assert_eq!(jam_chance(10, 0), 0);
    }

    #[test]
    fn a_quick_second_press_jams_until_fire_comes_up() {
        let mut c = bound(&[(1, "+attack")]);
        c.jam_fire_time = Some(50);
        down(&mut c, 1, 1000);
        assert!(!c.kb.jam.active);
        up(&mut c, 1, 1010);
        down(&mut c, 1, 1045);
        assert!(c.kb.jam.active);
        assert!(sampled_has(&mut c, 1050, button::JAM));
        up(&mut c, 1, 1060);
        assert!(!c.kb.jam.active);
    }

    #[test]
    fn a_second_fire_key_does_not_roll() {
        let mut c = bound(&[(1, "+attack"), (2, "+attack")]);
        c.jam_fire_time = Some(50);
        down(&mut c, 1, 1000);
        down(&mut c, 2, 1020);
        assert!(!c.kb.jam.active);
        assert_eq!(c.holds.last_fire, 1000);
    }

    #[test]
    fn no_snapshot_no_roll_and_no_window_no_jam() {
        let mut c = bound(&[(1, "+attack")]);
        down(&mut c, 1, 1000);
        assert!(!c.holds.fire_down);
        up(&mut c, 1, 1010);

        c.jam_fire_time = Some(0);
        down(&mut c, 1, 1020);
        up(&mut c, 1, 1030);
        down(&mut c, 1, 1040);
        assert!(!c.kb.jam.active);
        assert_eq!(c.holds.last_fire, 1040);
    }
}

mod look_and_cmd {
    use super::*;

    fn look_input(now_msec: i32) -> AdjustAnglesInput {
        AdjustAnglesInput {
            dt: 0.1,
            now_msec,
            frame_msec: 100,
            cl_yawspeed: CL_YAWSPEED_DEFAULT,
            cl_pitchspeed: CL_PITCHSPEED_DEFAULT,
            cl_anglespeedkey: CL_ANGLESPEEDKEY_DEFAULT,
            cgame_max_yaw_speed: 0.0,
            cgame_max_pitch_speed: 0.0,
            frozen: false,
        }
    }

    #[test]
    fn turn_keys_turn_by_rate_and_speed_key() {
        let mut c = bound(&[(1, "+left"), (2, "+speed_throw"), (3, "+lookdown")]);
        down(&mut c, 1, 1000);
        let (pitch, yaw) = adjust_angles(&mut c.kb, look_input(1100));
        assert!(close(pitch, 0.0) && close(yaw, 14.0));
        down(&mut c, 2, 1100);
        down(&mut c, 3, 1100);
        let (pitch, yaw) = adjust_angles(&mut c.kb, look_input(1200));
        assert!(close(pitch, 21.0) && close(yaw, 21.0));
        let capped = AdjustAnglesInput {
            cgame_max_yaw_speed: 70.0,
            ..look_input(1300)
        };
        let (_, yaw) = adjust_angles(&mut c.kb, capped);
        assert!(close(yaw, 10.5));
    }

    #[test]
    fn strafe_and_frozen_stop_the_key_turn() {
        let mut c = bound(&[(1, "+right"), (2, "+strafe")]);
        down(&mut c, 1, 1000);
        down(&mut c, 2, 1000);
        assert_eq!(adjust_angles(&mut c.kb, look_input(1100)), (0.0, 0.0));
        let frozen = AdjustAnglesInput {
            frozen: true,
            ..look_input(1200)
        };
        assert_eq!(adjust_angles(&mut c.kb, frozen), (0.0, 0.0));
    }

    #[test]
    fn mouse_gain_and_turn() {
        assert_eq!(
            apply_mouse_sensitivity(2.0, 3.0, 10.0, 0.0, 5.0, 0.5, 1.0),
            (20.0, 30.0)
        );
        assert_eq!(
            apply_mouse_sensitivity(2.0, 3.0, 10.0, 10.0, 5.0, 0.5, 0.5),
            (5.5, 8.25)
        );
        assert_eq!(mouse_move_angles(10.0, -10.0, 1.0, 1.0), (-1820, -1820));
    }

    #[test]
    fn the_pad_reaches_the_corners() {
        assert_eq!(pad_move_stretched(1.0, 0.0), (127, 0));
        assert_eq!(pad_move_stretched(0.5, 0.0), (63, 0));
        assert_eq!(pad_move_stretched(-0.5, 0.0), (-63, 0));
        assert_eq!(pad_move_stretched(0.0, 0.0), (0, 0));
        let d = core::f32::consts::FRAC_1_SQRT_2;
        let (f, r) = pad_move_stretched(d, d);
        assert!(f >= 126 && r >= 126);
    }

    #[test]
    fn move_bytes_round_and_clamp() {
        assert_eq!(axis_to_move(1.0), 127);
        assert_eq!(axis_to_move(-1.0), -127);
        assert_eq!(axis_to_move(0.5), 64);
        assert_eq!(axis_to_move(-0.5), -64);
        assert_eq!(axis_to_move(2.0), 127);
        assert_eq!(clamp_char(254), 127);
        assert_eq!(clamp_char(-254), -128);
        assert_eq!(clamp_char(-5), -5);
    }

    fn turned(pitch: i32, yaw: i32) -> UserCmd {
        create_cmd(&CreateCmdInput {
            server_time: 500,
            angles: [1000, 0, 0],
            mouse_pitch_delta: pitch,
            mouse_yaw_delta: yaw,
            forwardmove: 127,
            ..CreateCmdInput::default()
        })
    }

    #[test]
    fn pitch_turns_at_most_a_quarter_a_command() {
        assert_eq!(turned(20000, 0).angles[0], 1000 + PITCH_STEP_MAX);
        assert_eq!(turned(-20000, 0).angles[0], 1000 - PITCH_STEP_MAX);
        assert_eq!(turned(65636, 0).angles[0], 1000 + 65636);
        assert_eq!(turned(0, 30000).angles[1], 30000);
        assert_eq!(turned(0, 0).forwardmove, 127);
    }

    #[test]
    fn a_frozen_command_carries_only_time_and_view() {
        let mut buttons = playerstate_t6::ButtonBits::default();
        buttons.set(button::ATTACK);
        let cmd = create_cmd(&CreateCmdInput {
            server_time: 500,
            angles: [1, 2, 3],
            buttons,
            forwardmove: 127,
            mouse_pitch_delta: 100,
            frozen: true,
            ..CreateCmdInput::default()
        });
        assert_eq!(cmd.server_time, 500);
        assert_eq!(cmd.angles, [1, 2, 3]);
        assert!(!cmd.held(button::ATTACK));
        assert_eq!(cmd.forwardmove, 0);
    }
}

mod weapon_select {
    use crate::weapon_select::{SelectFlags, cycle_weapon, weapon_cycle_allowed};

    const PLAYING: SelectFlags = SelectFlags {
        pm_flags: 0,
        weap_flags: 0,
        other_flags: 0x4,
        e_flags: 0,
        pm_type: 0,
    };

    #[test]
    fn a_change_waits_for_play_the_delay_and_the_override() {
        assert!(weapon_cycle_allowed(PLAYING, 1000, 900, 0, 100));
        assert!(!weapon_cycle_allowed(PLAYING, 1000, 950, 0, 100));
        assert!(!weapon_cycle_allowed(PLAYING, 1000, 0, 1001, 0));
        assert!(!weapon_cycle_allowed(SelectFlags::default(), 1000, 0, 0, 0));
        let blocked = [
            SelectFlags {
                pm_flags: 0x800,
                ..PLAYING
            },
            SelectFlags {
                weap_flags: 0x80,
                ..PLAYING
            },
            SelectFlags {
                other_flags: 0x6,
                ..PLAYING
            },
            SelectFlags {
                e_flags: 0x10_0000,
                ..PLAYING
            },
            SelectFlags {
                pm_type: 8,
                ..PLAYING
            },
        ];
        for ps in blocked {
            assert!(!weapon_cycle_allowed(ps, 1000, 0, 0, 0), "{ps:?}");
        }
    }

    /// Weapon `w`'s inventory kind in these tests: 30s are alternate modes, 40s cycle alone,
    /// 50s are out of the cycle.
    fn kind(w: u32) -> i32 {
        match w / 10 {
            3 => 3,
            4 => 4,
            5 => 1,
            _ => 0,
        }
    }

    fn inventory(list: &[i32]) -> [i32; 15] {
        let mut slots = [0; 15];
        slots[..list.len()].copy_from_slice(list);
        slots
    }

    #[test]
    fn the_walk_wraps_both_ways_and_skips_other_kinds() {
        let w = inventory(&[1, 41, 2, 51, 3]);
        assert_eq!(cycle_weapon(&w, 1, 0, 0, true, kind), Some(2));
        assert_eq!(cycle_weapon(&w, 3, 0, 0, true, kind), Some(1));
        assert_eq!(cycle_weapon(&w, 1, 0, 0, false, kind), Some(3));
        assert_eq!(cycle_weapon(&w, 0, 0, 0, true, kind), Some(1));
        assert_eq!(cycle_weapon(&inventory(&[1]), 1, 0, 0, true, kind), None);
    }

    #[test]
    fn alternate_modes_and_separate_kinds() {
        let w = inventory(&[1, 41, 2, 42, 51]);
        assert_eq!(
            cycle_weapon(&w, 31, 2, 0, true, kind),
            Some(1),
            "the launcher is its gun"
        );
        assert_eq!(cycle_weapon(&w, 41, 0, 1, true, kind), Some(42));
        assert_eq!(
            cycle_weapon(&w, 51, 0, 2, true, kind),
            Some(2),
            "back to the last primary"
        );
        assert_eq!(
            cycle_weapon(&w, 51, 0, 9, true, kind),
            Some(1),
            "a primary no longer held"
        );
    }
}
