//! A short match for the tests: one player walking along x, with a join
//! request on the first tick.

use net::{Frame, SnapshotEncoder, frame_from_tick};
use playerstate_t6::game::{PlayerState, UserCmd};
use sim::{ClientAction, ClientId, Snapshot, Tick, TickInput};

pub const PLAYER: ClientId = ClientId(1);

pub fn snapshot(tick: u32) -> Snapshot {
    let mut state = PlayerState::ZERO;
    state.command_time = tick as i32 * 50;
    state.origin = [tick as f32, 2.0, 3.0];
    let mut snapshot = Snapshot::unpublished(Tick(tick));
    snapshot.players.push((PLAYER, state));
    snapshot
}

pub fn input(tick: u32) -> TickInput {
    let cmd = UserCmd {
        server_time: tick as i32 * 50,
        forwardmove: 127,
        ..UserCmd::default()
    };
    let mut input = TickInput::from_cmds(vec![(PLAYER, cmd)]);
    if tick == 1 {
        input
            .actions
            .push((PLAYER, ClientAction::JoinMatch { request_id: 7 }));
    }
    input
}

/// The frames ticks 1..=`count` go out as, in order, one encoder for all.
pub fn frames(count: u32) -> Vec<Frame> {
    let mut encoder = SnapshotEncoder::new();
    (1..=count)
        .map(|tick| frame_from_tick(&mut encoder, &input(tick), &snapshot(tick)))
        .collect()
}
