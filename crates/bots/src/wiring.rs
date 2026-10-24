//! The bots plugin. Each frame it passes the console's `bot add`/`bot hold` to the game, seats the test clients BO2's
//! bot script added, joins new ones to the match, and runs `bot tp`. Each authority tick it keeps one command queued
//! per living bot: BO2's bot code (sim's t6 module) rewrites that command inside the step, so a bot without one
//! would never move.

use bevy::prelude::*;
use frame::{AuthoritySet, ClientSet, HasWorld, MatchTornDown};
use net::{
    ActionRequestIds, AuthorityClock, AuthorityWorld, ClientActionInbox, ClientCommandInbox, LocalPresentClient,
    ReliableEventHub, authority_should_tick, look_angles_from_degrees,
};
use playerstate_t6::game::{PlayerState, UserCmd};
use sim::{ClientAction, ClientId, ClientLifecycle, Tick};

use crate::commands::{BotAddQueue, BotHold, BotTpQueue, BotTpTarget, BotTpWhere};
use crate::gamertags;
use crate::lineup::{BotRoster, class_for};

/// The steepest pitch, up or down, a teleported bot is turned to.
const STEEPEST_PITCH: f32 = 85.0;

/// Host-side bot plumbing; the module notes say what runs when.
pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BotRoster>();
        app.init_resource::<BotAddQueue>();
        app.init_resource::<BotHold>();
        app.init_resource::<BotTpQueue>();
        app.add_systems(
            Update,
            (forward_console, seat_new_bots, free_local_client, boot_new_bots, teleport_bots).chain(),
        );
        app.add_systems(Update, empty_roster_on_teardown.in_set(ClientSet::Load));
        app.add_systems(
            FixedUpdate,
            (free_local_client, keep_bots_commanded)
                .chain()
                .in_set(AuthoritySet::Ingress)
                .run_if(authority_should_tick),
        );
    }
}

/// A finished match takes its bots with it.
fn empty_roster_on_teardown(mut torn: MessageReader<MatchTornDown>, mut roster: ResMut<BotRoster>) {
    if torn.read().count() > 0 {
        *roster = BotRoster::default();
    }
}

/// Hands `bot add` and `bot hold` to the game. Asked-for bots wait here until a world exists to take them.
fn forward_console(mut adds: ResMut<BotAddQueue>, hold: Res<BotHold>, world: Option<ResMut<AuthorityWorld>>) {
    let Some(mut world) = world else {
        return;
    };
    let asked = adds.take();
    if asked > 0 {
        world.0.request_t6_bots(asked);
        diag::info!(Sim, "bots: asked the game's bot script for {asked}");
    }
    world.0.hold_t6_bots(hold.0);
}

/// Seats every test client the bot script added, and lets go of the bots the mod menu kicked.
fn seat_new_bots(mut roster: ResMut<BotRoster>, world: Option<ResMut<AuthorityWorld>>) {
    let Some(mut world) = world else {
        return;
    };
    for n in world.0.take_t6_test_clients() {
        if roster.is_bot(ClientId(n)) {
            continue;
        }
        let gamertag = gamertags::deal();
        diag::info!(Sim, "bots: BO2 test client {n} joins as {gamertag}");
        roster.seat(ClientId(n), gamertag);
    }
    // The script's own leave requests are only logged; the bot keeps its seat.
    for n in world.0.take_t6_bot_leaves() {
        diag::info!(Sim, "bots: BO2 bot {n} asked to leave, it stays");
    }
    // The menu's "Kick All Bots": the client leaves the match and its scripts see the disconnect.
    for n in sim::take_menu_kicks() {
        if roster.unseat(ClientId(n)) {
            world.0.retire_client(ClientId(n));
            diag::info!(Sim, "bots: the menu kicked bot {n}");
        }
    }
}

/// A seat on the local player's own client number would drive the player; give it up.
fn free_local_client(mut roster: ResMut<BotRoster>, local: Res<LocalPresentClient>) {
    if roster.unseat(local.0) {
        diag::warn!(Sim, "bots: client {} is the local player, its bot seat is gone", local.0.0);
    }
}

/// Joins each new bot to the match with its first class and its gamertag. All three requests are sent even when one
/// is refused; a refused boot is tried again next frame.
fn boot_new_bots(
    mut roster: ResMut<BotRoster>,
    mut actions: ResMut<ClientActionInbox>,
    mut ids: ResMut<ActionRequestIds>,
    has_world: Option<Res<HasWorld>>,
) {
    if !has_world.is_some_and(|flag| flag.0) {
        return;
    }
    let seed = roster.seed;
    for seat in roster.seats.iter_mut().filter(|seat| !seat.booted) {
        let class = class_for(seed, seat.client, seat.picks);
        let requests = [
            ClientAction::JoinMatch { request_id: ids.allocate() },
            ClientAction::ChooseDefaultClass { request_id: ids.allocate(), index: class },
            ClientAction::SetName {
                request_id: ids.allocate(),
                name: entity_t6::client_state::strncpyz::<16>(&seat.gamertag),
            },
        ];
        let outcomes = requests.map(|request| actions.push(seat.client, request));
        if let Some(error) = outcomes.into_iter().find_map(Result::err) {
            diag::warn!(Sim, "bots: client {} not joined yet: {error}", seat.client.0);
            continue;
        }
        seat.booted = true;
        seat.picks += 1;
        diag::info!(Sim, "bots: client {} joined with class pick {class}", seat.client.0);
    }
}

/// Runs the `bot tp` requests. Without a world they are dropped.
fn teleport_bots(
    mut queue: ResMut<BotTpQueue>,
    roster: Res<BotRoster>,
    mut actions: ResMut<ClientActionInbox>,
    mut ids: ResMut<ActionRequestIds>,
    world: Option<Res<AuthorityWorld>>,
    local: Res<LocalPresentClient>,
) {
    let requests = queue.take();
    let Some(world) = world.filter(|_| !requests.is_empty()) else {
        return;
    };
    let now = world.0.snapshot(Tick(0));
    let state_of = |client: ClientId| now.players.iter().find(|(id, _)| *id == client).map(|(_, ps)| ps);
    let local_alive = now.meta.for_client(local.0).is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
    let player_at = local_alive.then(|| state_of(local.0).map(|ps| ps.origin)).flatten();
    for request in requests {
        let movers = match request.target {
            BotTpTarget::All => roster.seats.iter().map(|seat| seat.client).collect(),
            BotTpTarget::Id(client) if roster.is_bot(client) => vec![client],
            BotTpTarget::Id(client) => {
                diag::warn!(Sim, "bots: tp skipped, client {} is not a bot", client.0);
                Vec::new()
            }
        };
        for client in movers {
            let facing = state_of(client).map_or([0.0; 3], |ps| ps.viewangles);
            let Some((origin, angles)) = destination(request.where_, facing, player_at) else {
                diag::warn!(Sim, "bots: tp above skipped, the local player is not alive");
                continue;
            };
            let request_id = ids.allocate();
            match actions.push(client, ClientAction::Move { request_id, origin, angles }) {
                Ok(()) => diag::info!(
                    Sim,
                    "bots: tp client={} ({:.1} {:.1} {:.1}) request_id={request_id}",
                    client.0,
                    origin[0],
                    origin[1],
                    origin[2]
                ),
                Err(error) => diag::warn!(Sim, "bots: tp client={} not queued: {error}", client.0),
            }
        }
    }
}

/// Where a teleport lands and which way the bot then faces. `facing` is the bot's view now; `player` is the local
/// player's origin while alive. None for `above` with no live player.
fn destination(place: BotTpWhere, facing: [f32; 3], player: Option<[f32; 3]>) -> Option<([f32; 3], [f32; 3])> {
    match place {
        BotTpWhere::Absolute { origin, yaw, pitch } => {
            let angles = [pitch.unwrap_or(facing[0]), yaw.unwrap_or(facing[1]), facing[2]];
            Some((origin, angles))
        }
        BotTpWhere::Above { height } => {
            let below = player?;
            let origin = [below[0], below[1], below[2] + height];
            Some((origin, look_from(origin, below)))
        }
    }
}

/// View angles (pitch, yaw, roll in degrees) for an eye at `eye` looking at `spot`. Positive pitch looks down; it is
/// kept within the steepest pitch, and a spot straight above or below gets that pitch with yaw 0.
fn look_from(eye: [f32; 3], spot: [f32; 3]) -> [f32; 3] {
    let gap: [f32; 3] = std::array::from_fn(|axis| spot[axis] - eye[axis]);
    if gap[0] == 0.0 && gap[1] == 0.0 {
        let pitch = if gap[2] < 0.0 { STEEPEST_PITCH } else { -STEEPEST_PITCH };
        return [pitch, 0.0, 0.0];
    }
    let [pitch, yaw, roll] = game_math::vect_to_angles(gap);
    let pitch = match game_math::angle_normalize_360(pitch) {
        over if over > 180.0 => over - 360.0,
        within => within,
    };
    [pitch.clamp(-STEEPEST_PITCH, STEEPEST_PITCH), game_math::angle_normalize_360(yaw), roll]
}

/// A command that keeps the bot's view and weapon as they are and does nothing else.
fn standing_cmd(server_time: i32, ps: &PlayerState) -> UserCmd {
    let view: [f32; 3] = std::array::from_fn(|axis| ps.viewangles[axis] - ps.delta_angles[axis]);
    let weapon = ps.weapon as u16;
    UserCmd {
        server_time,
        angles: look_angles_from_degrees(view),
        weapon,
        weapon_mapped: weapon,
        ..UserCmd::default()
    }
}

/// Every authority tick: acknowledge each bot's reliable events (no remote reader will, and an unread queue
/// overflows until the bot is dropped as disconnected), choose a class once per life, and queue a standing command
/// for each living bot from the last tick's state.
fn keep_bots_commanded(
    clock: Res<AuthorityClock>,
    world: Res<AuthorityWorld>,
    mut roster: ResMut<BotRoster>,
    mut cmds: ResMut<ClientCommandInbox>,
    mut actions: ResMut<ClientActionInbox>,
    mut ids: ResMut<ActionRequestIds>,
    mut reliable: ResMut<ReliableEventHub>,
) {
    for seat in &roster.seats {
        reliable.ack_all(seat.client);
    }
    if roster.seats.is_empty() {
        return;
    }
    let last = world.0.snapshot(Tick(clock.tick.saturating_sub(1)));
    let seed = roster.seed;
    for seat in &mut roster.seats {
        let Some(meta) = last.meta.for_client(seat.client) else {
            continue;
        };
        if meta.lifecycle != ClientLifecycle::Alive {
            continue;
        }
        if seat.chose_for != Some(meta.life_sequence) {
            seat.chose_for = Some(meta.life_sequence);
            let index = class_for(seed, seat.client, seat.picks);
            match actions.push(seat.client, ClientAction::ChooseDefaultClass { request_id: ids.allocate(), index }) {
                Ok(()) => seat.picks += 1,
                Err(error) => diag::warn!(Sim, "bots: client {} class pick not queued: {error}", seat.client.0),
            }
        }
        if let Some((_, ps)) = last.players.iter().find(|(id, _)| *id == seat.client) {
            cmds.push(seat.client, None, standing_cmd(clock.time_ms, ps), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(got: [f32; 3], want: [f32; 3]) -> bool {
        got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3)
    }

    #[test]
    fn a_spot_straight_below_tips_the_view_all_the_way_down() {
        assert_eq!(look_from([5.0, 5.0, 300.0], [5.0, 5.0, 0.0]), [STEEPEST_PITCH, 0.0, 0.0]);
        assert_eq!(look_from([5.0, 5.0, 0.0], [5.0, 5.0, 300.0]), [-STEEPEST_PITCH, 0.0, 0.0]);
    }

    #[test]
    fn level_and_slanted_looks() {
        assert!(near(look_from([0.0; 3], [100.0, 0.0, 0.0]), [0.0, 0.0, 0.0]));
        assert!(near(look_from([0.0, 0.0, 100.0], [100.0, 0.0, 0.0]), [45.0, 0.0, 0.0]));
        assert!(near(look_from([0.0; 3], [0.0, 100.0, 0.0]), [0.0, 90.0, 0.0]));
    }

    #[test]
    fn absolute_teleports_keep_the_bots_own_angles_unless_given() {
        let facing = [10.0, 20.0, 0.0];
        let kept = BotTpWhere::Absolute { origin: [1.0, 2.0, 3.0], yaw: None, pitch: None };
        assert_eq!(destination(kept, facing, None), Some(([1.0, 2.0, 3.0], facing)));
        let turned = BotTpWhere::Absolute { origin: [0.0; 3], yaw: Some(90.0), pitch: Some(-5.0) };
        assert_eq!(destination(turned, facing, None), Some(([0.0; 3], [-5.0, 90.0, 0.0])));
    }

    #[test]
    fn above_needs_a_live_player_and_looks_down_at_them() {
        let over = BotTpWhere::Above { height: 300.0 };
        assert_eq!(destination(over, [0.0; 3], None), None);
        assert_eq!(
            destination(over, [0.0; 3], Some([10.0, 20.0, 30.0])),
            Some(([10.0, 20.0, 330.0], [STEEPEST_PITCH, 0.0, 0.0]))
        );
    }

    #[test]
    fn a_standing_command_keeps_view_and_weapon_and_nothing_else() {
        let ps = PlayerState {
            viewangles: [10.0, 20.0, 0.0],
            delta_angles: [1.0, 2.0, 0.0],
            weapon: 7,
            ..PlayerState::ZERO
        };
        let cmd = standing_cmd(5000, &ps);
        assert_eq!(cmd.server_time, 5000);
        assert_eq!(cmd.angles, look_angles_from_degrees([9.0, 18.0, 0.0]));
        assert_eq!((cmd.weapon, cmd.weapon_mapped), (7, 7));
        assert_eq!((cmd.buttons, cmd.forwardmove, cmd.rightmove), (0, 0, 0));
    }
}
