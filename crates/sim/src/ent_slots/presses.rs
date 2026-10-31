//! Turning a frame's worth of player commands into "who pressed use".

use std::collections::HashMap;

use playerstate_t6::game::buttons::{INTERACT_KEY, INTERACT_RELOAD_KEY};

/// One command that had use held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InteractTap {
    pub client: u32,
    pub holding: bool,
    /// The button was up in that client's previous command.
    pub rim: bool,
}

const USE_BITS: u32 = INTERACT_KEY | INTERACT_RELOAD_KEY;

/// `cmds` are `(client, buttons)` in the order they ran; `last_tick_buttons` is each client's buttons from before this
/// frame (when a client is listed twice the first entry counts). Commands with no use button down give no press.
pub fn gather_presses(orders: &[(u32, u32)], last_tick_buttons: &[(u32, u32)]) -> Vec<InteractTap> {
    let mut last: HashMap<u32, u32> = HashMap::new();
    for &(who, bits) in last_tick_buttons {
        last.entry(who).or_insert(bits);
    }

    let mut found: Vec<InteractTap> = vec![];
    for &(who, bits) in orders {
        let before = last.insert(who, bits).unwrap_or(0);
        if bits & USE_BITS != 0 {
            found.push(InteractTap { client: who, holding: true, rim: before & USE_BITS == 0 });
        }
    }
    found
}
