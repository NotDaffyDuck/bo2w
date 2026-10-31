use super::MENU_RESPONSE_BYTES;
use crate::PersonalClass;
use crate::match_ids::GameRoundStage;

/// A class slot: the position in the client's class list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadoutClass(pub u32);
/// Older spelling ClassId still calls; the rename sweep moves it over.
pub use self::LoadoutClass as ClassId;

/// Where a forced spawn puts the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpawnPick {
    /// A spot drawn from this seed.
    Seeded(u64),
    /// Exactly here, facing `yaw`.
    At { origin: [f32; 3], yaw: f32 },
}

/// A request to the match from a client, or from the host for one. Every variant carries its request id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerIntent {
    JoinMatch { request_id: u32 },
    LeaveMatch { request_id: u32 },
    /// Choose a class: its slot, the revision of the class list the choice came from, and what the class holds.
    PickLoadout { request_id: u32, class_id: LoadoutClass, revision: u32, loadout: PersonalClass },
    HandOutGun { request_id: u32, gun: u32 },
    /// Swap weapon `from` for the same weapon with different attachments, `to`.
    ChangeWeaponConfiguration { request_id: u32, from: u32, to: u32 },
    /// Kill the client's player where it stands.
    ForceDeath { request_id: u32 },
    /// Spawn the usual way.
    SpawnClient { request_id: u32 },
    ForceSpawn { request_id: u32, pick: SpawnPick },
    /// Send the client to the end-of-match view.
    SpawnIntermission { request_id: u32 },
    /// Move the whole match into a phase.
    StageMatch { request_id: u32, phase: GameRoundStage },
    /// Put the client's player down at a place, facing `angles`.
    Move { request_id: u32, origin: [f32; 3], angles: [f32; 3] },
    /// Spin the client's script mover at `speed`.
    SpinMover { request_id: u32, speed: f32 },
    /// Hurt the client's player by `amount`; a test aid.
    TestHarm { request_id: u32, amount: i32 },
    /// Rename the client. The name is padded with zero bytes.
    SetName { request_id: u32, name: [u8; 16] },
    /// Use whatever the Copycat perk picked up.
    UseCopycat { request_id: u32 },
    ActionSlot { request_id: u32, slot: u8 },
    /// Take the default class at `index`.
    ChooseDefaultClass { request_id: u32, index: u8 },
    /// Answer a script menu: the menu's name and the answer, both padded fields.
    MenuResponse { request_id: u32, menu: [u8; MENU_RESPONSE_BYTES], response: [u8; MENU_RESPONSE_BYTES] },
    /// Award the killstreak with this padded name.
    GiveKillstreak { request_id: u32, name: [u8; MENU_RESPONSE_BYTES] },
}

impl PlayerIntent {
    /// The request id, whichever kind of action this is.
    pub fn ticket(&self) -> u32 {
        use PlayerIntent::*;
        match *self {
            JoinMatch { request_id: ticket }
            | LeaveMatch { request_id: ticket }
            | ForceDeath { request_id: ticket }
            | SpawnClient { request_id: ticket }
            | SpawnIntermission { request_id: ticket }
            | UseCopycat { request_id: ticket }
            | PickLoadout { request_id: ticket, .. }
            | HandOutGun { request_id: ticket, .. }
            | ChangeWeaponConfiguration { request_id: ticket, .. }
            | ForceSpawn { request_id: ticket, .. }
            | StageMatch { request_id: ticket, .. }
            | Move { request_id: ticket, .. }
            | SpinMover { request_id: ticket, .. }
            | TestHarm { request_id: ticket, .. }
            | SetName { request_id: ticket, .. }
            | ActionSlot { request_id: ticket, .. }
            | ChooseDefaultClass { request_id: ticket, .. }
            | MenuResponse { request_id: ticket, .. }
            | GiveKillstreak { request_id: ticket, .. } => ticket,
        }
    }
}

/// The request id of `request`.
pub fn intent_serial(request: &PlayerIntent) -> u32 {
    request.ticket()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_of_each(ticket: u32) -> Vec<PlayerIntent> {
        let field = [0; MENU_RESPONSE_BYTES];
        vec![
            PlayerIntent::JoinMatch { request_id: ticket },
            PlayerIntent::LeaveMatch { request_id: ticket },
            PlayerIntent::PickLoadout { request_id: ticket, class_id: LoadoutClass(2), revision: 3, loadout: PersonalClass::default() },
            PlayerIntent::HandOutGun { request_id: ticket, gun: 5 },
            PlayerIntent::ChangeWeaponConfiguration { request_id: ticket, from: 5, to: 6 },
            PlayerIntent::ForceDeath { request_id: ticket },
            PlayerIntent::SpawnClient { request_id: ticket },
            PlayerIntent::ForceSpawn { request_id: ticket, pick: SpawnPick::Seeded(9) },
            PlayerIntent::SpawnIntermission { request_id: ticket },
            PlayerIntent::StageMatch { request_id: ticket, phase: GameRoundStage::Playing },
            PlayerIntent::Move { request_id: ticket, origin: [1.0; 3], angles: [2.0; 3] },
            PlayerIntent::SpinMover { request_id: ticket, speed: 4.0 },
            PlayerIntent::TestHarm { request_id: ticket, amount: 7 },
            PlayerIntent::SetName { request_id: ticket, name: [b'a'; 16] },
            PlayerIntent::UseCopycat { request_id: ticket },
            PlayerIntent::ActionSlot { request_id: ticket, slot: 1 },
            PlayerIntent::ChooseDefaultClass { request_id: ticket, index: 2 },
            PlayerIntent::MenuResponse { request_id: ticket, menu: field, response: field },
            PlayerIntent::GiveKillstreak { request_id: ticket, name: field },
        ]
    }

    #[test]
    fn every_kind_of_action_reports_its_id() {
        let all = one_of_each(41);
        assert_eq!(all.len(), 19);
        for action in &all {
            assert_eq!(action.ticket(), 41, "{action:?}");
            assert_eq!(intent_serial(action), 41);
        }
    }
}
