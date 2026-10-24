//! Turning a frame's worth of player commands into "who pressed use".

use std::collections::HashMap;

use playerstate_t6::game::buttons::{USE, USE_RELOAD};

/// One command that had use held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsePress {
    pub client: u32,
    pub held: bool,
    /// The button was up in that client's previous command.
    pub edge: bool,
}

const USE_BITS: u32 = USE | USE_RELOAD;

/// `cmds` are `(client, buttons)` in the order they ran; `old_buttons` is each client's buttons from before this
/// frame (when a client is listed twice the first entry counts). Commands with no use button down give no press.
pub fn collect_use_presses(cmds: &[(u32, u32)], old_buttons: &[(u32, u32)]) -> Vec<UsePress> {
    let mut last: HashMap<u32, u32> = HashMap::new();
    for &(client, bits) in old_buttons {
        last.entry(client).or_insert(bits);
    }

    let mut presses = Vec::new();
    for &(client, bits) in cmds {
        let before = last.insert(client, bits).unwrap_or(0);
        if bits & USE_BITS != 0 {
            presses.push(UsePress { client, held: true, edge: before & USE_BITS == 0 });
        }
    }
    presses
}
