//! One client's keys, and what its bound commands change.

use alloc::vec::Vec;
use playerstate_t6::button;

use crate::bind_ids::{HOLD_COMMANDS, HOLD_ID_END, BIND_COMMAND_TABLE, release_action_number};
use crate::{KEYBOARD_SLOTS, HeldKeyLatch, HeldKeyLatchBank, KeyPressRecord};

pub(crate) const DUCK_BUTTON_BIT: i32 = button::DUCK_BUTTON_BIT as i32;
pub(crate) const LYING_FLAT: i32 = button::LYING_FLAT as i32;

/// Black Ops II's cl_stanceHoldTime: the stance button held this long goes prone (or up from
/// prone) instead of crouching.
const STANCE_HOLD_MS: i32 = 300;
/// cl_dtpHoldTime: while sprinting, the stance button held this long dives.
const DIVE_HOLD_MS: i32 = 200;
/// cl_weapNextHoldTime: the switch-weapon key held this long takes the inventory weapon.
const WEAPNEXT_HOLD_MS: i32 = 250;
/// cl_dblTapMaxDelayTime: a use press this long after the last one starts a new count.
const DOUBLE_TAP_GAP_MS: i32 = 500;
/// cl_dblTapMaxHoldTime: a use key let go within this long of its press is a tap.
const DOUBLE_TAP_HOLD_MS: i32 = 500;

/// Use, Reload and Use+Reload share one double tap (C4's second detonate). A press counts only
/// while none of the three is held; it starts the count over when it comes long after the
/// last counted press. Every release is judged against that press: quick is a tap, slow wipes
/// the count. The second tap goes out once, on the next command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UseTap {
    held: bool,
    pressed_at: i32,
    taps: u8,
}

impl UseTap {
    fn down(&mut self, now: i32) {
        if self.held {
            return;
        }
        if now.wrapping_sub(self.pressed_at) >= DOUBLE_TAP_GAP_MS {
            self.taps = 0;
        }
        self.held = true;
        self.pressed_at = now;
    }

    fn up(&mut self, now: i32) {
        self.held = false;
        self.taps = if now.wrapping_sub(self.pressed_at) < DOUBLE_TAP_HOLD_MS {
            self.taps.saturating_add(1)
        } else {
            0
        };
    }

    /// True once when the second tap is in; the count and clock start over.
    pub(crate) fn take_double(&mut self) -> bool {
        if self.taps != 2 {
            return false;
        }
        self.taps = 0;
        self.pressed_at = 0;
        true
    }
}

/// What the client's own holds wait on from frame to frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Holds {
    /// The stance button is down: the latch it went down from, and when.
    pub(crate) stance: Option<(i32, i32)>,
    /// `+weapnext_inventory` is down and can still be a tap: since when.
    pub(crate) weapnext: Option<i32>,
    /// The fire key went down and has not come up (Black Ops II's gAttackEdgeDetected).
    pub(crate) fire_down: bool,
    /// When the fire key last went down from up (lastFireTime).
    pub(crate) last_fire: i32,
    pub(crate) use_tap: UseTap,
}

#[derive(Clone, Debug)]
pub struct PlayerIntentFeed {
    pub keys: [KeyPressRecord; KEYBOARD_SLOTS],
    pub pressed_key_latch: HeldKeyLatchBank,
    /// The aim toggle; holding the speed key flips it.
    pub scoped_in: bool,
    /// The stance the client keeps asking for: `button::DUCK_BUTTON_BIT`, `button::LYING_FLAT`, or 0 to stand.
    pub posture_hold: i32,
    /// Switch-weapon presses waiting for the client: true is next, false previous.
    pub gun_swap_steps: Vec<bool>,
    /// Set and cleared by the client: the next command lets go of a grenade being held
    /// without throwing it.
    pub throw_grip_abort: bool,
    /// Set and cleared by the client: the next command sends Black Ops II's offhand-switch
    /// button (a switch-weapon press that cancels a held grenade).
    pub offhand_switch_t6: bool,
    /// Action slot presses (0 to 3) waiting for the client.
    pub action_slots: Vec<usize>,
    /// `centerview` was run; the client clears it.
    pub center_view: bool,
    /// Set each frame by the client: sprinting at standing height, so a stance press waits to
    /// see whether it is a dive.
    pub sprinting: bool,
    /// Set each frame by the client: the gun in hand forbids prone.
    pub gun_forbids_lying: bool,
    /// The client should switch to the inventory weapon (a held switch-weapon key, or
    /// `inventory`).
    pub inventory_request: bool,
    /// The switch-weapon key went down; it doubles as the vehicle seat button.
    pub change_seat_press: bool,
    /// Set each frame from the last snapshot: the jam window of the gun in hand, 0 for a gun
    /// that cannot jam, `None` before the first snapshot (no roll and no bookkeeping then).
    pub jam_fire_time: Option<i32>,
    pub holds: Holds,
}

impl Default for PlayerIntentFeed {
    fn default() -> Self {
        Self {
            keys: [KeyPressRecord::default(); KEYBOARD_SLOTS],
            pressed_key_latch: HeldKeyLatchBank::default(),
            scoped_in: false,
            posture_hold: 0,
            gun_swap_steps: Vec::new(),
            throw_grip_abort: false,
            offhand_switch_t6: false,
            action_slots: Vec::new(),
            center_view: false,
            sprinting: false,
            gun_forbids_lying: false,
            inventory_request: false,
            change_seat_press: false,
            jam_fire_time: None,
            holds: Holds::default(),
        }
    }
}

pub fn set_ads(client: &mut PlayerIntentFeed, ads: bool) {
    client.scoped_in = ads;
}

/// A hold command's key going down or up.
#[derive(Clone, Copy)]
struct Edge {
    down: bool,
    key: i32,
    now: i32,
    tick_span_ms: u32,
}

impl Edge {
    fn apply(self, held: &mut HeldKeyLatch) {
        if self.down {
            held.press(self.key, self.now);
        } else {
            held.release(self.key, self.now, self.tick_span_ms);
        }
    }
}

/// Runs bound command `cmd_id` for `key` at `present_tick_ms`.
pub fn input_cmd(client: &mut PlayerIntentFeed, binding_id: u32, key: i32, present_tick_ms: i32, tick_span_ms: u32) {
    if binding_id == 0 {
        return;
    }
    if binding_id < HOLD_ID_END {
        let edge = Edge {
            down: binding_id % 2 == 1,
            key,
            now: present_tick_ms,
            tick_span_ms,
        };
        hold_command(client, HOLD_COMMANDS[(binding_id as usize - 1) / 2], edge);
    } else if let Some(name) = BIND_COMMAND_TABLE.get(binding_id as usize) {
        once_command(client, name, present_tick_ms);
    }
}

fn hold_command(client: &mut PlayerIntentFeed, what: &str, edge: Edge) {
    let pressed_key_latch = &mut client.pressed_key_latch;
    match what {
        "attack" => {
            if edge.down {
                jam_roll(client, edge.key, edge.now);
            } else {
                client.holds.fire_down = false;
                edge.apply(&mut client.pressed_key_latch.jam);
            }
            edge.apply(&mut client.pressed_key_latch.attack);
        }
        "breath_sprint" => {
            edge.apply(&mut pressed_key_latch.holdbreath);
            edge.apply(&mut pressed_key_latch.sprint);
        }
        "usereload" | "activate" | "reload" => {
            if edge.down {
                client.holds.use_tap.down(edge.now);
            } else {
                client.holds.use_tap.up(edge.now);
            }
            edge.apply(match what {
                "usereload" => &mut pressed_key_latch.usereload,
                "activate" => &mut pressed_key_latch.activate,
                _ => &mut pressed_key_latch.reload,
            });
        }
        "speed_throw" => {
            if edge.down {
                client.scoped_in = false;
            }
            edge.apply(&mut pressed_key_latch.speed);
            edge.apply(&mut pressed_key_latch.throw_btn);
        }
        "toggleads_throw" => {
            if edge.down {
                client.scoped_in = !client.scoped_in;
            }
            edge.apply(&mut pressed_key_latch.throw_btn);
        }
        "stance" => {
            let previously_on = pressed_key_latch.stance.active;
            edge.apply(&mut pressed_key_latch.stance);
            let active = pressed_key_latch.stance.active;
            if active != previously_on {
                stance_button(client, active, edge.now);
            }
        }
        slot if slot.starts_with("actionslot ") => {
            if edge.down {
                let n = slot.as_bytes()[slot.len() - 1] - b'1';
                client.action_slots.push(usize::from(n));
            }
        }
        plain => {
            if let Some(held) = plain_button(pressed_key_latch, plain) {
                edge.apply(held);
            }
        }
    }
}

/// The hold commands that do nothing but hold their own button.
fn plain_button<'a>(pressed_key_latch: &'a mut HeldKeyLatchBank, what: &str) -> Option<&'a mut HeldKeyLatch> {
    Some(match what {
        "melee" => &mut pressed_key_latch.melee,
        "frag" => &mut pressed_key_latch.frag,
        "smoke" => &mut pressed_key_latch.smoke,
        "gostand" => &mut pressed_key_latch.gostand,
        "forward" => &mut pressed_key_latch.forward,
        "back" => &mut pressed_key_latch.back,
        "moveleft" => &mut pressed_key_latch.moveleft,
        "moveright" => &mut pressed_key_latch.moveright,
        "movedown" => &mut pressed_key_latch.movedown,
        "left" => &mut pressed_key_latch.left,
        "right" => &mut pressed_key_latch.right,
        "lookup" => &mut pressed_key_latch.lookup,
        "lookdown" => &mut pressed_key_latch.lookdown,
        "strafe" => &mut pressed_key_latch.strafe,
        "holdbreath" => &mut pressed_key_latch.holdbreath,
        "prone" => &mut pressed_key_latch.prone,
        "mlook" => &mut pressed_key_latch.mlook,
        "sprint" => &mut pressed_key_latch.sprint,
        "scores" => &mut pressed_key_latch.scores,
        "talk" => &mut pressed_key_latch.talk,
        _ => return None,
    })
}

fn once_command(client: &mut PlayerIntentFeed, name: &str, now: i32) {
    let toggle = |latch: i32, stance: i32| if latch == stance { 0 } else { stance };
    match name {
        "weapnext" | "weapprev" => {
            let next = name == "weapnext";
            client.gun_swap_steps.push(next);
            client.change_seat_press |= next;
        }
        "centerview" => client.center_view = true,
        "togglecrouch" => client.posture_hold = toggle(client.posture_hold, DUCK_BUTTON_BIT),
        "toggleprone" => client.posture_hold = toggle(client.posture_hold, LYING_FLAT),
        "goprone" => client.posture_hold = LYING_FLAT,
        "gocrouch" => client.posture_hold = DUCK_BUTTON_BIT,
        "toggleads" => client.scoped_in = !client.scoped_in,
        "leaveads" => client.scoped_in = false,
        // Down: a hold starts unless one is already running. Up: a hold not yet taken by
        // `weapnext_hold` was a tap, which is the next weapon.
        "+weapnext_inventory" => {
            if client.holds.weapnext.is_none() {
                client.holds.weapnext = Some(now);
                client.change_seat_press = true;
            }
        }
        "-weapnext_inventory" => {
            if client.holds.weapnext.take().is_some() {
                client.gun_swap_steps.push(true);
            }
        }
        "inventory" => client.inventory_request = true,
        // The menu, pause and chat commands belong to the console.
        _ => {}
    }
}

/// Black Ops II's semi-auto jam chance, in percent, for a fire press `dt` ms after the last
/// one with a gun whose jam window is `window` ms: 60000/dt - 60000/window, held to 0..=100.
/// A zero gap never jams.
pub(crate) fn jam_chance(dt: i32, window: i32) -> i32 {
    if dt <= 0 || window <= 0 {
        return 0;
    }
    let chance = 60000.0 / dt as f32 - 60000.0 / window as f32;
    (chance as i32).clamp(0, 100)
}

/// The anti-macro roll on a fresh fire press, before the fire button itself goes down: inside
/// the gun's jam window the jam button goes down when the clock's last two digits fall under
/// the chance. A fresh press is then marked, jam or not.
fn jam_roll(client: &mut PlayerIntentFeed, key: i32, now: i32) {
    let Some(window) = client.jam_fire_time else {
        return;
    };
    if client.holds.fire_down {
        return;
    }
    let dt = now.wrapping_sub(client.holds.last_fire);
    if window > 0 && dt < window && now % 100 < jam_chance(dt, window) {
        client.pressed_key_latch.jam.press(key, now);
    }
    client.holds.fire_down = true;
    client.holds.last_fire = now;
}

/// The stance button going down or up. Nothing happens while a crouch or prone key is held.
/// Down remembers the stance it left and crouches at once, unless sprinting: then the press
/// waits to be a dive. Up after a waiting press crouches (a gun that forbids prone keeps the
/// stance); up after a press made crouched stands.
pub(crate) fn stance_button(client: &mut PlayerIntentFeed, down: bool, now: i32) {
    if client.pressed_key_latch.prone.active || client.pressed_key_latch.movedown.active {
        return;
    }
    if down {
        client.holds.stance = Some((client.posture_hold, now));
        if !client.sprinting {
            client.posture_hold = DUCK_BUTTON_BIT;
        }
        return;
    }
    let Some((from, _)) = client.holds.stance.take() else {
        return;
    };
    if from == DUCK_BUTTON_BIT {
        client.posture_hold = 0;
    } else if client.posture_hold == from && !client.gun_forbids_lying {
        client.posture_hold = DUCK_BUTTON_BIT;
    }
}

/// A stance press held long enough: while sprinting it dives; otherwise it goes prone, or
/// stands when it started prone.
pub(crate) fn stance_hold(client: &mut PlayerIntentFeed, now: i32) {
    let Some((from, since)) = client.holds.stance else {
        return;
    };
    let held = now.wrapping_sub(since);
    let latch = if client.sprinting {
        (held >= DIVE_HOLD_MS).then_some(LYING_FLAT)
    } else {
        (held >= STANCE_HOLD_MS).then_some(if from == LYING_FLAT { 0 } else { LYING_FLAT })
    };
    if let Some(latch) = latch {
        client.posture_hold = latch;
        client.holds.stance = None;
    }
}

/// The switch-weapon key held long enough takes the inventory weapon; its release then does
/// nothing.
pub(crate) fn weapnext_hold(client: &mut PlayerIntentFeed, now: i32) {
    let Some(since) = client.holds.weapnext else {
        return;
    };
    if now.wrapping_sub(since) >= WEAPNEXT_HOLD_MS {
        client.holds.weapnext = None;
        client.inventory_request = true;
    }
}

/// Key `digit_for_key` goes down or up: its binding runs, or the up half of it.
pub fn key_event(
    client: &mut PlayerIntentFeed,
    digit_for_key: usize,
    down: bool,
    present_tick_ms: i32,
    tick_span_ms: u32,
) {
    let Some(state) = client.keys.get_mut(digit_for_key) else {
        return;
    };
    state.down = u8::from(down);
    let binding = state.binding;
    let run = if down {
        Some(binding).filter(|&id| id != 0)
    } else {
        release_action_number(binding)
    };
    if let Some(id) = run {
        input_cmd(client, id, digit_for_key as i32, present_tick_ms, tick_span_ms);
    }
}

/// A key bound to going down a stance (crouch, prone, or a toggle or go command for either)
/// is held now.
pub fn lower_key_held(client: &PlayerIntentFeed) -> bool {
    const LOWER: [&str; 7] = [
        "+stance",
        "+movedown",
        "+prone",
        "togglecrouch",
        "toggleprone",
        "goprone",
        "gocrouch",
    ];
    client.keys.iter().any(|key| {
        key.down != 0
            && BIND_COMMAND_TABLE
                .get(key.binding as usize)
                .is_some_and(|name| LOWER.contains(name))
    })
}
