//! Menu requests the scripts send a client, and the pick-a-spot prompt a streak can raise.

/// How many of the newest menu commands a client's record keeps.
pub const MENU_COMMAND_TAIL: usize = 8;

/// A numbered request; the numbers climb so a client can tell a new request from one it already acted on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOrder {
    pub serial: u32,
    pub kind: MenuCommandKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCommandKind {
    Show(String),
    ClosePopup,
    CloseInGame,
}

/// The "choose where" map prompt of a call-in streak.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationSelection {
    pub material: String,
    pub choose_direction: bool,
    pub reach: f32,
}
