//! Killcams the match scripts ask for: each tick the host turns the scripts' spectator seats
//! into running killcam sessions, stepping the replay time to where the watched player was
//! actually recorded.

mod follow;
mod script;

pub(crate) use script::play_script_seats;

#[cfg(test)]
mod test_support {
    use playerstate_t6::game::AvatarState;
    use sim::{ClientId, Snapshot, Tick};

    use crate::net_links::snapshot_history::{StoredHistoryFrame, SnapshotVault};

    /// An archive with one frame per tick in `0..ticks`; `has_state(tick)` lists the clients
    /// that have a player state in that frame.
    pub(super) fn archive_with(ticks: u32, has_state: impl Fn(u32) -> Vec<u32>) -> SnapshotVault {
        let mut archive = SnapshotVault::new();
        for tick in 0..ticks {
            let mut snapshot = Snapshot::unpublished(Tick(tick));
            for client in has_state(tick) {
                snapshot.players.push((ClientId(client), AvatarState::ZERO));
            }
            archive.append(StoredHistoryFrame { at_tick: Tick(tick), state: snapshot, no_player_state: Vec::new() });
        }
        archive
    }
}
