//! What a snapshot says about the local player and the thing it stands on.

use playerstate_t6::game::{PlayerState, other_flags};
use sim::{ClientId, Snapshot};

#[cfg(test)]
use sim::Tick;

/// The local player's state as the snapshot carries it.
pub(super) fn snapshot_local(snapshot: &Snapshot, local: ClientId) -> Option<PlayerState> {
    snapshot
        .players
        .iter()
        .find_map(|(id, state)| (*id == local).then_some(*state))
}

/// False while the player watches the kill camera: the move code does not run then.
pub(super) fn pmove_runs_for(state: &PlayerState) -> bool {
    state.other_flags & other_flags::DEAD_KILLCAM_TPV == 0
}

/// Entity numbers 1 to 2045 can be stood on.
///
/// This is the player state's own range today; change the two together, never one alone.
fn can_stand_on(entity_num: i32) -> bool {
    (1..=0x7fd).contains(&entity_num)
}

/// The entity type of what a player stands on, when the snapshot names it: another player, or a
/// corpse in the snapshot's corpse table.
pub fn snapshot_ground_e_type(snapshot: &Snapshot, mover_num: i32) -> Option<i16> {
    if !can_stand_on(mover_num) {
        return None;
    }
    let is_player = snapshot
        .players
        .iter()
        .any(|(id, _)| i32::try_from(id.0) == Ok(mover_num));
    if is_player {
        return Some(entity_t6::et::PLAYER);
    }
    let is_corpse = snapshot
        .meta
        .corpses
        .slots
        .iter()
        .any(|slot| slot.occupied && slot.entnum == mover_num);
    is_corpse.then_some(entity_t6::et::PLAYER_CORPSE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(flags: u32) -> PlayerState {
        PlayerState {
            other_flags: flags,
            ..PlayerState::ZERO
        }
    }

    #[test]
    fn only_the_killcam_stops_the_move_code() {
        assert!(pmove_runs_for(&PlayerState::ZERO));
        assert!(pmove_runs_for(&state_with(other_flags::PLAYER)));
        assert!(!pmove_runs_for(&state_with(other_flags::DEAD_KILLCAM_TPV)));
    }

    #[test]
    fn the_standing_range_runs_from_one_to_2045() {
        assert!(!can_stand_on(0));
        assert!(can_stand_on(1));
        assert!(can_stand_on(2045));
        assert!(!can_stand_on(2046));
        assert!(!can_stand_on(-1));
    }

    #[test]
    fn nothing_is_known_about_ground_outside_the_range() {
        let snapshot = Snapshot::unpublished(Tick(1));
        assert_eq!(snapshot_ground_e_type(&snapshot, 0), None);
        assert_eq!(snapshot_ground_e_type(&snapshot, 5000), None);
        assert_eq!(snapshot_ground_e_type(&snapshot, 7), None);
    }

    #[test]
    fn the_local_player_is_found_by_client_id() {
        let mut snapshot = Snapshot::unpublished(Tick(1));
        snapshot.players.push((ClientId(3), state_with(1)));
        snapshot.players.push((ClientId(4), state_with(2)));
        assert_eq!(
            snapshot_local(&snapshot, ClientId(4)).map(|s| s.other_flags),
            Some(2)
        );
        assert!(snapshot_local(&snapshot, ClientId(9)).is_none());
    }

    #[test]
    fn standing_on_another_player_is_reported_as_a_player() {
        let mut snapshot = Snapshot::unpublished(Tick(1));
        snapshot.players.push((ClientId(6), PlayerState::ZERO));
        assert_eq!(
            snapshot_ground_e_type(&snapshot, 6),
            Some(entity_t6::et::PLAYER)
        );
    }
}
