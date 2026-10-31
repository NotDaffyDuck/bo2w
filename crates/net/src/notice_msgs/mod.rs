//! Short text notices the server sends to clients: a player left, or a printed line.

mod codec;
mod pending;

use bevy::prelude::Message;
use sim::ClientId;

pub use codec::{parse_host_game_alerts, pack_host_game_alerts};
pub use pending::StagedAnnouncements;

/// Wire tag: a player disconnected (ASCII `e`).
pub const HOST_MSG_LEAVE_ALERT: u8 = 0x65;

/// Wire tag: a normal printed line (ASCII `f`).
pub const HOST_MSG_TEXT_LINE: u8 = 0x66;

/// Wire tag: a bold printed line (ASCII `g`).
pub const SVC_PRINT_BOLD: u8 = 0x67;

/// One notice: a numbered line made of a name and a message key.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct HostGameAnnouncement {
    pub id: u32,
    pub tag: u8,
    pub name: String,
    pub key: String,
}

/// A client's display name as the world has it packed, without the padding.
pub fn receiver_display_text(game: &sim::SimWorld, who: ClientId) -> String {
    entity_t6::client_info::c_str(&game.packed_client_name(who)).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tags_are_the_letters_e_f_and_g() {
        assert_eq!(HOST_MSG_LEAVE_ALERT, b'e');
        assert_eq!(HOST_MSG_TEXT_LINE, b'f');
        assert_eq!(SVC_PRINT_BOLD, b'g');
    }
}
