use bevy::prelude::Resource;
use entity_t6::event;
use input_t6::{
    ANGLE2SHORT, AdjustAnglesInput, CL_ANGLESPEEDKEY_DEFAULT, CL_PITCHSPEED_DEFAULT,
    CL_YAWSPEED_DEFAULT, ClientInput, CreateCmdInput, adjust_angles, axis_to_move, clamp_char,
    create_cmd, mouse_move_angles, pad_move_stretched, sample_move,
};
use playerstate_t6::game::UserCmd;
use playerstate_t6::game::buttons;
use playerstate_t6::button;
use std::collections::BTreeSet;

pub const KEY_FRAME_MSEC_MAX: u32 = 200;

pub fn frame_time_msec(elapsed_secs: f32) -> i32 {
    (elapsed_secs * 1000.0).max(1.0) as i32
}

pub fn key_frame_msec(delta_secs: f32) -> u32 {
    let ms = (delta_secs.max(0.0) * 1000.0).round() as i32;
    ms.clamp(1, KEY_FRAME_MSEC_MAX as i32) as u32
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LookState {
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

#[derive(Resource, Clone, Debug)]
pub struct ClientActionInput {
    pub client: ClientInput,
    pub(crate) stance_life: Option<u32>,
    pub(crate) stance_event_sequence: i32,
    pub(crate) stance_events: [i32; 4],

    pub scripted_ids: BTreeSet<u32>,
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub sensitivity: f32,
    pub mouse_accel: f32,
    pub fov_scale: f32,

    pub shellshock_look_scale: f32,

    pub cgame_max_pitch_speed: f32,

    pub cgame_max_yaw_speed: f32,
    pub m_yaw: f32,
    pub m_pitch: f32,

    pub cl_yawspeed: f32,
    pub now_msec: i32,
    pub frame_msec: u32,

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

impl Default for ClientActionInput {
    fn default() -> Self {
        Self {
            client: ClientInput::default(),
            stance_life: None,
            stance_event_sequence: 0,
            stance_events: [0; 4],
            scripted_ids: BTreeSet::new(),
            mouse_x: 0.0,
            mouse_y: 0.0,
            sensitivity: 5.0,
            mouse_accel: 0.0,
            fov_scale: 1.0,
            shellshock_look_scale: 1.0,
            cgame_max_pitch_speed: 0.0,
            cgame_max_yaw_speed: 0.0,
            m_yaw: 0.022,
            m_pitch: 0.022,
            cl_yawspeed: CL_YAWSPEED_DEFAULT,
            now_msec: 16,
            frame_msec: 16,
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

impl ClientActionInput {
    pub fn consume_edges(&mut self) {
        self.client.kb.clear_was_pressed();
    }

    /// BO2's CG_CheckPlayerstateEvents for the stance events (EV_STANCE_FORCE_STAND,
    /// _CROUCH, _PRONE: BO2's 8, 9, 10): the predicted state is held against last frame's,
    /// every frame. An event past last frame's sequence is new; one inside
    /// the four last frame held is new too when it changed (a correction
    /// replayed it at another number). The newest one sets the latch.
    pub(crate) fn read_stance_events(&mut self, sequence: i32, events: [i32; 4]) {
        let old = self.stance_event_sequence;
        for back in (1..=4).rev() {
            let seq = sequence.wrapping_sub(back);
            let slot = (seq & 3) as usize;
            let behind = old.wrapping_sub(seq);
            if behind > 0 && (behind > 4 || events[slot] == self.stance_events[slot]) {
                continue;
            }
            match u8::try_from(events[slot]) {
                Ok(event::STANCE_FORCE_STAND) => self.client.stance_latch = 0,
                Ok(event::STANCE_FORCE_CROUCH) => self.client.stance_latch = button::CROUCH as i32,
                Ok(event::STANCE_FORCE_PRONE) => self.client.stance_latch = button::PRONE as i32,
                _ => {}
            }
        }
        self.stance_event_sequence = sequence;
        self.stance_events = events;
    }
}

pub fn look_angles_from_degrees(viewangles: [f32; 3]) -> [i32; 3] {
    [
        (viewangles[0] * ANGLE2SHORT) as i32,
        (viewangles[1] * ANGLE2SHORT) as i32,
        (viewangles[2] * ANGLE2SHORT) as i32,
    ]
}

pub fn accumulate_look(
    dt: f32,
    now_msec: i32,
    frame_msec: u32,
    client: &mut ClientInput,
    look: &mut LookState,
    cl_yawspeed: f32,
    frozen: bool,
    cgame_max_pitch_speed: f32,
    cgame_max_yaw_speed: f32,
) {
    let (pitch_deg, yaw_deg) = adjust_angles(
        &mut client.kb,
        AdjustAnglesInput {
            dt,
            now_msec,
            frame_msec,
            cl_yawspeed,
            cl_pitchspeed: CL_PITCHSPEED_DEFAULT,
            cl_anglespeedkey: CL_ANGLESPEEDKEY_DEFAULT,
            cgame_max_yaw_speed,
            cgame_max_pitch_speed,
            frozen,
        },
    );
    look.angles[0] = look.angles[0].wrapping_add((pitch_deg * ANGLE2SHORT) as i32);
    look.angles[1] = look.angles[1].wrapping_add((yaw_deg * ANGLE2SHORT) as i32);
}

pub fn build_usercmd(input: &mut ClientActionInput, look: &LookState, server_time: i32) -> UserCmd {
    let now = input.now_msec.max(1);
    let frame = input.frame_msec.max(1);
    let (bo2_bits, axes) = sample_move(&mut input.client, now, frame);
    let mut extra = 0;
    if input.client.offhand_hold_cancel {
        input.client.offhand_hold_cancel = false;
        extra |= buttons::OFFHAND_HOLD_CANCEL;
    }
    // bo2mp gun brain: BO2's offhand-switch button (weapon_t6 cmd::button::OFFHAND_SWITCH).
    if input.client.offhand_switch_t6 {
        input.client.offhand_switch_t6 = false;
        extra |= 0x20000;
    }

    let mouse_counts = mouse_distance(input.mouse_x, input.mouse_y);
    let frame_f = frame as f32;
    let (mx, my) = input_t6::apply_mouse_sensitivity(
        input.mouse_x,
        input.mouse_y,
        mouse_counts,
        frame_f,
        input.sensitivity,
        input.mouse_accel,
        input.fov_scale,
    );
    let (mouse_pitch, mouse_yaw) = mouse_move_angles(mx, my, input.m_yaw, input.m_pitch);
    let mouse_look = input.mouse_x != 0.0 || input.mouse_y != 0.0;
    let pad_pitch = if mouse_look {
        0
    } else {
        (input.pad_look_delta[0] * ANGLE2SHORT) as i32
    };
    let pad_yaw = if mouse_look {
        0
    } else {
        (input.pad_look_delta[1] * ANGLE2SHORT) as i32
    };
    // Black Ops II: the key move, then the pad move stretched to the square
    // (CL_GamepadMove), added and held to a move byte.
    let (pad_forward, pad_right) = pad_move_stretched(input.pad_move[0], input.pad_move[1]);
    let forwardmove = clamp_char(i32::from(axis_to_move(axes.forward)) + pad_forward);
    let rightmove = clamp_char(i32::from(axis_to_move(axes.right)) + pad_right);

    let mut cmd = sim_cmd(&create_cmd(&CreateCmdInput {
        server_time,
        angles: look.angles,
        buttons: bo2_bits,
        forwardmove,
        rightmove,
        mouse_pitch_delta: mouse_pitch + pad_pitch,
        mouse_yaw_delta: mouse_yaw + pad_yaw,
        key_pitch_delta: 0,
        key_yaw_delta: 0,
        frozen: false,
    }));
    cmd.buttons |= extra;
    cmd
}

/// BO2's buttons and the MW2 command bit each one is in the sim.
const SIM_BUTTON_BITS: [(u32, u32); 17] = [
    (button::ATTACK, buttons::ATTACK),
    (button::SPRINT, buttons::SPRINT),
    (button::MELEE, buttons::MELEE_CHARGE),
    (button::USE, buttons::USE),
    (button::RELOAD, buttons::RELOAD),
    (button::USE_RELOAD, buttons::USE_RELOAD),
    (button::PRONE, buttons::PRONE),
    (button::CROUCH, buttons::CROUCH),
    (button::JUMP, buttons::JUMP),
    (button::ADS, buttons::ADS),
    (button::STANCE_HELD, buttons::STANCE_HELD),
    (button::BREATH, buttons::BREATH),
    (button::FRAG, buttons::FRAG),
    (button::SECONDARY_OFFHAND, buttons::SMOKE),
    (button::THROW, buttons::THROW),
    (button::JAM, buttons::JAM),
    (button::DOUBLE_TAP, buttons::DOUBLE_TAP),
];

/// The sim still steps MW2's command, so the client's BO2 command goes in
/// as one until the sim takes `playerstate_t6::UserCmd`.
fn sim_cmd(cmd: &playerstate_t6::UserCmd) -> UserCmd {
    let mut bits = 0;
    for (button, bit) in SIM_BUTTON_BITS {
        if cmd.held(button) {
            bits |= bit;
        }
    }
    UserCmd {
        server_time: cmd.server_time,
        buttons: bits,
        angles: cmd.angles,
        forwardmove: cmd.forwardmove,
        rightmove: cmd.rightmove,
        ..UserCmd::default()
    }
}

/// Black Ops II's mouse speed for cl_mouseAccel: the distance moved this
/// frame (sqrt of dx*dx + dy*dy), not the sum of both axes.
fn mouse_distance(dx: f32, dy: f32) -> f32 {
    (dx * dx + dy * dy).sqrt()
}

/// The missile he rides turns by the mouse, or, with the mouse still, by the
/// right stick (pushed up noses it up, right turns it right).
pub fn remote_control_axes(
    input: &ClientActionInput,
    mouse_x: f32,
    mouse_y: f32,
    pad_look: [f32; 2],
) -> [u8; 2] {
    let axis = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8;
    if mouse_x == 0.0 && mouse_y == 0.0 {
        return [axis(-pad_look[1]), axis(-pad_look[0])];
    }
    let (mx, my) = input_t6::apply_mouse_sensitivity(
        mouse_x,
        mouse_y,
        mouse_distance(mouse_x, mouse_y),
        input.frame_msec.max(1) as f32,
        input.sensitivity,
        input.mouse_accel,
        input.fov_scale,
    );
    [axis(input.m_pitch * my), axis(-input.m_yaw * mx)]
}

pub fn idle_usercmd(server_time: i32) -> UserCmd {
    UserCmd {
        server_time,
        ..UserCmd::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crouched(sequence: i32, events: [i32; 4]) -> ClientActionInput {
        let mut input = ClientActionInput::default();
        input.client.stance_latch = button::CROUCH as i32;
        input.stance_event_sequence = sequence;
        input.stance_events = events;
        input
    }

    #[test]
    fn a_new_stand_event_clears_the_crouch_latch() {
        let mut input = crouched(9, [0; 4]);
        input.read_stance_events(10, [0, event::STANCE_FORCE_STAND as i32, 0, 0]);
        assert_eq!(input.client.stance_latch, 0);
    }

    #[test]
    fn a_stand_event_replayed_lower_after_a_correction_still_counts() {
        // The client had run to 10; the server's state replayed lands the
        // sprint's stand event at 8.
        let mut input = crouched(10, [0; 4]);
        input.read_stance_events(9, [event::STANCE_FORCE_STAND as i32, 0, 0, 0]);
        assert_eq!(input.client.stance_latch, 0);
        assert_eq!(input.stance_event_sequence, 9);
    }

    #[test]
    fn the_stand_event_a_sprint_sends_never_goes_prone() {
        // Sprinting stands the player up every frame; the client must read that as
        // "stand" (latch off), not as the prone event, or Shift lies the player down.
        let mut input = ClientActionInput::default();
        input.stance_event_sequence = 0;
        let stand = event::STANCE_FORCE_STAND as i32;
        input.read_stance_events(1, [stand, 0, 0, 0]);
        assert_eq!(input.client.stance_latch, 0);
        input.read_stance_events(2, [stand, stand, 0, 0]);
        assert_eq!(input.client.stance_latch, 0);
    }

    #[test]
    fn the_crouch_and_prone_events_set_their_latch() {
        let mut input = ClientActionInput::default();
        input.read_stance_events(1, [event::STANCE_FORCE_CROUCH as i32, 0, 0, 0]);
        assert_eq!(input.client.stance_latch, button::CROUCH as i32);
        input.read_stance_events(2, [0, event::STANCE_FORCE_PRONE as i32, 0, 0]);
        assert_eq!(input.client.stance_latch, button::PRONE as i32);
    }

    #[test]
    fn events_already_seen_do_not_fire_again() {
        let stand = event::STANCE_FORCE_STAND as i32;
        let mut input = crouched(10, [0, stand, 0, 0]);
        input.read_stance_events(10, [0, stand, 0, 0]);
        assert_eq!(input.client.stance_latch, button::CROUCH as i32);
    }

    #[test]
    fn each_bo2_button_lands_on_its_sim_bit() {
        for (button, bit) in SIM_BUTTON_BITS {
            let mut cmd = playerstate_t6::UserCmd::default();
            cmd.button_bits.set(button);
            assert_eq!(sim_cmd(&cmd).buttons, bit, "button {button}");
        }
        let mut stance = playerstate_t6::UserCmd::default();
        stance.button_bits.set(button::PRONE);
        stance.button_bits.set(button::STANCE_HELD);
        assert_eq!(sim_cmd(&stance).buttons, buttons::PRONE | buttons::STANCE_HELD);
    }
}

#[cfg(test)]
mod remote_control_tests {
    use super::*;

    #[test]
    fn the_right_stick_steers_a_ridden_missile_when_the_mouse_is_still() {
        let input = ClientActionInput::default();
        // Stick full up: nose up (pitch down the scale); full right: turn right.
        assert_eq!(remote_control_axes(&input, 0.0, 0.0, [0.0, 1.0]), [(-127i8) as u8, 0]);
        assert_eq!(remote_control_axes(&input, 0.0, 0.0, [1.0, 0.0]), [0, (-127i8) as u8]);
        assert_eq!(remote_control_axes(&input, 0.0, 0.0, [-0.5, -0.5]), [64, 64]);
        // The mouse wins while it moves.
        let mouse = remote_control_axes(&input, 40.0, 0.0, [0.0, 1.0]);
        assert_eq!(mouse[0], 0);
        assert_ne!(mouse[1], 0);
    }
}
