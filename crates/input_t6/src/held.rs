//! A game button and the keys holding it.

use crate::{KEY_COUNT, SCRIPT_KEYNUM};

/// One bit per key number, plus one more for presses a script makes.
const HOLDER_WORDS: usize = (KEY_COUNT + 1).div_ceil(64);

/// A game button such as `+attack` or `+forward`. Any number of keys can hold it: it goes
/// active when the first of them goes down and lets go when the last comes up. Between two
/// samples it banks how long it was held, so a tap shorter than a frame still counts for its
/// share of that frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kbutton {
    holders: [u64; HOLDER_WORDS],
    /// When it went active, or when it was last sampled while active; 0 is unknown.
    since: i32,
    banked_ms: u32,
    pub active: bool,
    /// Went active since the client last cleared it: a tap between two commands still sends.
    pub was_pressed: bool,
}

/// The word and bit for `key`; keys outside the board (and not the script's) hold nothing.
fn holder_bit(key: i32) -> Option<(usize, u64)> {
    let n = if key == SCRIPT_KEYNUM {
        KEY_COUNT
    } else {
        usize::try_from(key).ok().filter(|&n| n < KEY_COUNT)?
    };
    Some((n / 64, 1 << (n % 64)))
}

fn elapsed(from: i32, to: i32) -> u32 {
    to.wrapping_sub(from).max(0) as u32
}

impl Kbutton {
    /// `key` goes down at `now`; a key already holding the button changes nothing.
    pub(crate) fn press(&mut self, key: i32, now: i32) {
        let Some((word, bit)) = holder_bit(key) else {
            return;
        };
        if self.holders[word] & bit != 0 {
            return;
        }
        self.holders[word] |= bit;
        if !self.active {
            self.active = true;
            self.was_pressed = true;
            self.since = now;
        }
    }

    /// `key` comes up at `now`. When it was the last key holding the button, the button lets
    /// go and banks the time it was held: half a frame when it never knew when it went down.
    pub(crate) fn release(&mut self, key: i32, now: i32, frame_msec: u32) {
        let Some((word, bit)) = holder_bit(key) else {
            return;
        };
        if self.holders[word] & bit == 0 {
            return;
        }
        self.holders[word] &= !bit;
        if !self.active || self.holders.iter().any(|w| *w != 0) {
            return;
        }
        let held = if self.since == 0 {
            frame_msec / 2
        } else {
            elapsed(self.since, now)
        };
        self.banked_ms = self.banked_ms.saturating_add(held);
        self.active = false;
        self.since = 0;
    }

    /// The share of a `frame_msec` frame the button was held, 0 to 1, ending at `now`. The bank
    /// empties; a button still held is counted again from `now` on.
    pub(crate) fn fraction(&mut self, now: i32, frame_msec: u32) -> f32 {
        let mut held = core::mem::take(&mut self.banked_ms);
        if self.active {
            if self.since != 0 {
                held = held.saturating_add(elapsed(self.since, now));
            }
            self.since = now;
        }
        if held == 0 || frame_msec == 0 {
            0.0
        } else if held >= frame_msec {
            1.0
        } else {
            held as f32 / frame_msec as f32
        }
    }

    /// Held now, or pressed since the last command.
    pub(crate) fn down_or_tapped(&self) -> bool {
        self.active || self.was_pressed
    }
}

/// One key: whether it is down, and the command bound to it (0 when unbound).
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyState {
    pub down: u8,
    pub binding: u32,
}
