//! A short match for the tests: one player walking along x, with a join
//! request on the first tick.

use playerstate_t6::game::{AvatarState, InputCommand};

pub const PLAYER: sim::ClientId = sim::ClientId(1);

pub fn snapshot(tick: u32) -> sim::Snapshot {
    let mut state = AvatarState::ZERO;
    state.last_cmd_ms = tick as i32 * 50;
    state.origin = [tick as f32, 2.0, 3.0];
    let mut snapshot = sim::Snapshot::unpublished(sim::Tick(tick));
    snapshot.players.push((PLAYER, state));
    snapshot
}

pub fn input(tick: u32) -> sim::StepInputs {
    let cmd = InputCommand {
        authority_clock_ms: tick as i32 * 50,
        thrust_ahead: 127,
        ..InputCommand::default()
    };
    let mut input = sim::StepInputs::from_cmds(vec![(PLAYER, cmd)]);
    if tick == 1 {
        input
            .actions
            .push((PLAYER, sim::PlayerIntent::JoinMatch { request_id: 7 }));
    }
    input
}

/// The frames ticks 1..=`count` go out as, in order, one encoder for all.
pub fn frames(count: u32) -> Vec<net::Frame> {
    let mut encoder = net::FrameDiffWriter::new();
    (1..=count)
        .map(|tick| net::frame_for_tick(&mut encoder, &input(tick), &snapshot(tick)))
        .collect()
}
