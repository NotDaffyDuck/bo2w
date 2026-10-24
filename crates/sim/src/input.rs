//! Input to the match for one tick: the movement command of each client, plus the single actions (joining, class
//! choice, spawning, menu answers) that clients and the host request.

use playerstate_t6::game::UserCmd;

use crate::identities::MatchPhase;
use crate::world::ClientId;

/// Every client sends its own running count with each action; the match acts on one number only once.
pub type ActionRequestId = u32;

/// Longest text, in bytes, that a menu name, a menu answer or a killstreak name can hold inside an action.
pub const MENU_RESPONSE_BYTES: usize = 48;

/// A class slot, numbered by position in the client's class list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClassId(pub u32);

/// How a forced spawn picks the spot: drawn from a seed, or exactly at the given place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpawnPick {
    Seeded(u64),
    At {
        origin: [f32; 3],
        yaw: f32,
    },
}

/// Something a client, or the host on its behalf, asks the match to do. All variants carry their request id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClientAction {
    /// Enter the match.
    JoinMatch {
        request_id: ActionRequestId,
    },
    /// Leave the match.
    LeaveMatch {
        request_id: ActionRequestId,
    },
    /// Pick a class: the slot, the revision of the class list the pick came from, and the class contents.
    SelectClass {
        request_id: ActionRequestId,
        class_id: ClassId,
        revision: u32,
        loadout: crate::PersonalClass,
    },
    /// Hand a weapon to the client.
    GiveWeapon {
        request_id: ActionRequestId,
        weapon: u32,
    },
    /// Trade the weapon `from` for the same weapon with other attachments, `to`.
    ChangeWeaponConfiguration {
        request_id: ActionRequestId,
        from: u32,
        to: u32,
    },
    /// Kill the client's player on the spot.
    ForceDeath {
        request_id: ActionRequestId,
    },
    /// Spawn the client in the normal way.
    SpawnClient {
        request_id: ActionRequestId,
    },
    /// Spawn the client where the pick says.
    ForceSpawn {
        request_id: ActionRequestId,
        pick: SpawnPick,
    },
    /// Send the client to the end-of-match view.
    SpawnIntermission {
        request_id: ActionRequestId,
    },
    /// Put the whole match into a phase.
    SetMatchPhase {
        request_id: ActionRequestId,
        phase: MatchPhase,
    },
    /// Set the client's player down at a place with the given angles.
    Move {
        request_id: ActionRequestId,
        origin: [f32; 3],
        angles: [f32; 3],
    },
    /// Make the client's script mover rotate at a speed.
    BeginScriptMoverRotateVelocity {
        request_id: ActionRequestId,
        speed: f32,
    },
    /// Damage the client's player by an amount, for testing.
    DebugDamage {
        request_id: ActionRequestId,
        amount: i32,
    },
    /// Rename the client; the name is padded with zeros.
    SetName {
        request_id: ActionRequestId,
        name: [u8; 16],
    },
    /// Use what the Copycat perk picked.
    UseCopycat {
        request_id: ActionRequestId,
    },
    /// Press an action slot.
    ActionSlot {
        request_id: ActionRequestId,
        slot: u8,
    },
    /// Take the default class with this index.
    ChooseDefaultClass {
        request_id: ActionRequestId,
        index: u8,
    },
    /// Answer a script menu with the menu name and the answer, both padded with zeros.
    MenuResponse {
        request_id: ActionRequestId,
        menu: [u8; MENU_RESPONSE_BYTES],
        response: [u8; MENU_RESPONSE_BYTES],
    },
    /// Award the client a killstreak by name.
    GiveKillstreak {
        request_id: ActionRequestId,
        name: [u8; MENU_RESPONSE_BYTES],
    },
}

impl ClientAction {
    /// The request id this action carries, whichever kind it is.
    pub fn request_id(&self) -> ActionRequestId {
        match *self {
            Self::JoinMatch { request_id }
            | Self::LeaveMatch { request_id }
            | Self::SelectClass { request_id, .. }
            | Self::GiveWeapon { request_id, .. }
            | Self::ChangeWeaponConfiguration { request_id, .. }
            | Self::ForceDeath { request_id }
            | Self::SpawnClient { request_id }
            | Self::ForceSpawn { request_id, .. }
            | Self::SpawnIntermission { request_id }
            | Self::SetMatchPhase { request_id, .. }
            | Self::Move { request_id, .. }
            | Self::BeginScriptMoverRotateVelocity { request_id, .. }
            | Self::DebugDamage { request_id, .. }
            | Self::SetName { request_id, .. }
            | Self::UseCopycat { request_id }
            | Self::ActionSlot { request_id, .. }
            | Self::ChooseDefaultClass { request_id, .. }
            | Self::MenuResponse { request_id, .. }
            | Self::GiveKillstreak { request_id, .. } => request_id,
        }
    }
}

/// The request id of `action`.
pub fn action_request_id(action: &ClientAction) -> ActionRequestId {
    action.request_id()
}

/// Packs `text` into a menu field, zero padded; `None` when it is longer than the field.
pub fn menu_response_field(text: &str) -> Option<[u8; MENU_RESPONSE_BYTES]> {
    let bytes = text.as_bytes();
    let mut field = [0u8; MENU_RESPONSE_BYTES];
    field.get_mut(..bytes.len())?.copy_from_slice(bytes);
    Some(field)
}

/// Reads the text back out of a menu field, up to the first zero byte. Invalid UTF-8 reads as empty.
pub fn menu_response_text(field: &[u8; MENU_RESPONSE_BYTES]) -> &str {
    let used = field.iter().position(|&byte| byte == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..used]).unwrap_or("")
}

/// All the input of one tick: movement commands and actions, each tagged with the client it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickInput {
    pub cmds: Vec<(ClientId, UserCmd)>,
    pub actions: Vec<(ClientId, ClientAction)>,
}

impl TickInput {
    /// Input with movement commands and no actions.
    pub fn from_cmds(cmds: Vec<(ClientId, UserCmd)>) -> Self {
        Self {
            cmds,
            actions: Vec::new(),
        }
    }

    /// Orders the input the same way on every machine: commands by client, actions by client and then request id.
    /// The sort is stable, so equal keys stay in arrival order.
    pub fn canonicalize(&mut self) {
        self.cmds.sort_by_key(|(client, _)| client.0);
        self.actions
            .sort_by_key(|(client, action)| (client.0, action.request_id()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_actions() -> Vec<ClientAction> {
        let request_id = 41;
        vec![
            ClientAction::JoinMatch { request_id },
            ClientAction::LeaveMatch { request_id },
            ClientAction::SelectClass { request_id, class_id: ClassId(2), revision: 3, loadout: Default::default() },
            ClientAction::GiveWeapon { request_id, weapon: 5 },
            ClientAction::ChangeWeaponConfiguration { request_id, from: 5, to: 6 },
            ClientAction::ForceDeath { request_id },
            ClientAction::SpawnClient { request_id },
            ClientAction::ForceSpawn { request_id, pick: SpawnPick::Seeded(9) },
            ClientAction::SpawnIntermission { request_id },
            ClientAction::SetMatchPhase { request_id, phase: MatchPhase::Playing },
            ClientAction::Move { request_id, origin: [1.0; 3], angles: [2.0; 3] },
            ClientAction::BeginScriptMoverRotateVelocity { request_id, speed: 4.0 },
            ClientAction::DebugDamage { request_id, amount: 7 },
            ClientAction::SetName { request_id, name: [b'a'; 16] },
            ClientAction::UseCopycat { request_id },
            ClientAction::ActionSlot { request_id, slot: 1 },
            ClientAction::ChooseDefaultClass { request_id, index: 2 },
            ClientAction::MenuResponse { request_id, menu: [0; MENU_RESPONSE_BYTES], response: [0; MENU_RESPONSE_BYTES] },
            ClientAction::GiveKillstreak { request_id, name: [0; MENU_RESPONSE_BYTES] },
        ]
    }

    fn cmd(server_time: i32) -> UserCmd {
        UserCmd { server_time, ..UserCmd::default() }
    }

    #[test]
    fn every_kind_of_action_reports_the_id_it_carries() {
        let actions = all_actions();
        assert_eq!(actions.len(), 19);
        for action in &actions {
            assert_eq!(action.request_id(), 41, "{action:?}");
            assert_eq!(action_request_id(action), 41);
        }
    }

    #[test]
    fn text_fits_a_menu_field_up_to_its_size_and_reads_back() {
        let field = menu_response_field("class_select").unwrap();
        assert_eq!(&field[..12], b"class_select");
        assert!(field[12..].iter().all(|&byte| byte == 0));
        assert_eq!(menu_response_text(&field), "class_select");
    }

    #[test]
    fn text_exactly_the_size_of_the_field_fits_and_one_more_byte_does_not() {
        let full = "x".repeat(MENU_RESPONSE_BYTES);
        let field = menu_response_field(&full).unwrap();
        assert_eq!(menu_response_text(&field), full);
        assert_eq!(menu_response_field(&format!("{full}x")), None);
    }

    #[test]
    fn an_empty_field_reads_as_empty_text_and_bad_text_reads_as_empty() {
        assert_eq!(menu_response_text(&menu_response_field("").unwrap()), "");
        let mut field = [0u8; MENU_RESPONSE_BYTES];
        field[0] = 0xff;
        field[1] = 0xfe;
        assert_eq!(menu_response_text(&field), "");
    }

    #[test]
    fn reading_a_field_stops_at_the_first_zero_byte() {
        let mut field = menu_response_field("ab").unwrap();
        field[5] = b'z';
        assert_eq!(menu_response_text(&field), "ab");
    }

    #[test]
    fn canonical_order_sorts_commands_by_client() {
        let mut input = TickInput::from_cmds(vec![(ClientId(3), cmd(30)), (ClientId(1), cmd(10)), (ClientId(2), cmd(20))]);
        input.canonicalize();
        let clients: Vec<u32> = input.cmds.iter().map(|(client, _)| client.0).collect();
        assert_eq!(clients, [1, 2, 3]);
        assert!(input.actions.is_empty());
    }

    #[test]
    fn canonical_order_sorts_actions_by_client_then_request_and_keeps_ties_in_arrival_order() {
        let mut input = TickInput::default();
        input.actions = vec![
            (ClientId(2), ClientAction::ForceDeath { request_id: 1 }),
            (ClientId(1), ClientAction::SpawnClient { request_id: 9 }),
            (ClientId(1), ClientAction::GiveWeapon { request_id: 4, weapon: 11 }),
            (ClientId(1), ClientAction::GiveWeapon { request_id: 4, weapon: 12 }),
        ];
        input.canonicalize();
        let order: Vec<(u32, u32)> = input.actions.iter().map(|(client, action)| (client.0, action.request_id())).collect();
        assert_eq!(order, [(1, 4), (1, 4), (1, 9), (2, 1)]);
        assert_eq!(input.actions[0].1, ClientAction::GiveWeapon { request_id: 4, weapon: 11 });
        assert_eq!(input.actions[1].1, ClientAction::GiveWeapon { request_id: 4, weapon: 12 });
    }
}
