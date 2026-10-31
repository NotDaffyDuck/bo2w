//! Player-card messages from the server: which card slot a player shows, a menu to open, and
//! HUD splash text. The server queues them as they happen (`StagedCardUpdate`) and hands each
//! client its own when a frame is built; the wire half is in `codec`.

mod codec;
mod pending;

use bevy::prelude::*;

pub use codec::{
    decode_svc_card_slots, decode_svc_hud_splashes, decode_svc_open_menus, pack_host_nameplates,
    pack_host_overlay_flashes, pack_host_menu_openings,
};
pub use pending::StagedCardUpdate;

/// The tag of a card-slot message in a reliable block.
pub const HOST_MSG_NAMEPLATE: u8 = b'K';

/// The tag of an open-menu message in a reliable block.
pub const HOST_MSG_MENU_OPEN: u8 = b'u';

/// Show the card of `client` in `slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCardSlotMsg {
    pub client: u32,
    pub card_slot: i32,
}

/// Open the menu the config string at `config_slot_number` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMenuOpenMsg {
    pub config_slot_number: i32,
}

/// Show `key` as a splash in `slot` (0 to 4).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct HostScreenFlash {
    pub key: String,
    pub card_slot: i32,
    pub optional: i32,
}

/// The client-side message a received `HostCardSlotMsg` becomes.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardSlotCommand {
    pub client: u32,
    pub card_slot: i32,
}

/// The client-side message a received `HostMenuOpenMsg` becomes.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuOpenCommand {
    pub config_slot_number: i32,
}
