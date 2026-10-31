//! The game-side half of the lobby link: Bevy systems that start the browser
//! and the link worker, carry menu choices to them, and fold what the worker
//! publishes back into the match (relay transports, members leaving, the
//! hosted match starting, the status file).

use std::path::Path;

use bevy::prelude::*;
use master_protocol::ListingId;

use super::bridge::{DirectoryLink, Order};
use super::browser::DirectoryBrowser;
use super::content_bits::ContentCatalog;
use super::intent::{self, DirectoryLaunchRequest, Plan, process_env};
use super::pipes::RELAY_DATAGRAM_LIMIT;
use super::room_view::{
    DirectoryLinkStatus, DirectoryLifecycleNote, DirectoryGameOffer, DirectoryMatchBegin, DirectoryMenuChoice,
    QueuedMenuChoice, LobbySessionLabel,
};
use super::worker::{self, Errand};
use crate::host_tick::tick_systems::HostSimulationWorld;
use crate::net_links::datagrams::{AssetDigestSet, JoinGreeting, GameSessionSummary};
use crate::net_links::relay::{HostUdpSwitchboard, PlayerUdpSession};

/// Set by the test harness: the link writes its state here on every change.
const STATUS_FILE: &str = "BO2MP_MASTER_STATUS_FILE";

pub(crate) fn register(game: &mut App) {
    let load = || crate::ClientStep::Load;
    game.init_resource::<QueuedMenuChoice>();
    game.add_systems(
        Update,
        (keep_browser_open, take_menu_choice).chain().in_set(load()),
    );
    game.add_systems(
        Update,
        (
            start_link_and_relay,
            start_hosted_match_once_loaded,
            fold_link_news,
        )
            .chain()
            .in_set(load()),
    );
    game.add_systems(
        Update,
        crate::join_walk::step_match_edge_phase
            .after(frame::MatchSwapDone)
            .in_set(load()),
    );
    game.add_systems(Update, watch_link.in_set(crate::ClientStep::Diag));
    game.add_systems(
        FixedUpdate,
        (
            follow_live_content.in_set(crate::ServerStage::Advance),
            report_ticking_authority.after(crate::ServerStage::Snapshot),
        ),
    );
}

/// The fingerprint a relayed peer must match: the summary's content ids,
/// plus the world's gameplay and model ids once its clip is loaded.
fn relay_hello(world: Option<&sim::SimWorld>, summary: &GameSessionSummary) -> JoinGreeting {
    let (gameplay, models) = world
        .filter(|loaded| loaded.has_static_collision())
        .map(AssetDigestSet::from_world)
        .map_or((0, 0), |live| (live.rules_digest, live.model_digest));
    JoinGreeting::current(AssetDigestSet {
        map: summary.map,
        weapon_digest: summary.weapons,
        class_digest: summary.classes,
        rules_digest: gameplay,
        model_digest: models,
    })
}

/// The browser thread runs exactly while the menu shows the browser.
fn keep_browser_open(
    mut commands: Commands,
    open: Option<Res<DirectoryBrowser>>,
    launch: Res<DirectoryLaunchRequest>,
) {
    match (open.is_some(), launch.browsing()) {
        (false, Some((dial, have))) => {
            commands.insert_resource(DirectoryBrowser::open(dial.clone(), have))
        }
        (true, None) => commands.remove_resource::<DirectoryBrowser>(),
        _ => {}
    }
}

/// An order for the live room, or a warning naming what the menu wanted.
fn order_room(bridge: Option<&DirectoryLink>, order: Order, wanted: &str) {
    let Some(room) = bridge else {
        diag::warn!(WireLink, "master lobby {wanted} requested without a live bridge");
        return;
    };
    room.give(order);
}

/// The plan for a menu "Host" click, or `None` (after saying why in the log).
fn menu_host_plan(
    launch: &DirectoryLaunchRequest,
    packs: Option<&ContentCatalog>,
    map: String,
    ruleset: String,
) -> Option<Plan> {
    let (dial, have) = launch.browsing()?;
    let Some(packs) = packs else {
        diag::warn!(WireLink, "master host content: no content catalog is installed");
        return None;
    };
    let requires = packs
        .mask_for_map(&map)
        .inspect_err(|error| diag::warn!(WireLink, "master host content: {error}"))
        .ok()?;
    intent::menu_host(&process_env, dial, have, map, ruleset, requires)
        .inspect_err(|error| diag::warn!(WireLink, "master host settings: {error}"))
        .ok()
}

/// Carries the menu's one pending choice to the browser, the room or the plan.
fn take_menu_choice(
    mut commands: Commands,
    content_catalog: Option<Res<ContentCatalog>>,
    mut role: ResMut<frame::ProcessDuty>,
    bridge: Option<Res<DirectoryLink>>,
    mut queued: ResMut<QueuedMenuChoice>,
    pane: Option<Res<DirectoryBrowser>>,
    mut slot: ResMut<DirectoryLaunchRequest>,
) {
    let Some(choice) = std::mem::take(&mut queued.0) else {
        return;
    };
    let room = bridge.as_deref();
    match choice {
        DirectoryMenuChoice::BallotForSkip => order_room(room, Order::BallotForSkip, "vote"),
        DirectoryMenuChoice::Kick { member_id } => order_room(room, Order::Kick { member_id }, "kick"),
        DirectoryMenuChoice::BeginGame { map: m, mode: gm } => {
            order_room(room, Order::BeginGame { map: m, ruleset: gm }, "start")
        }
        DirectoryMenuChoice::RefreshRoom { map: m, mode: gm } => {
            order_room(room, Order::RefreshRoom { map: m, ruleset: gm }, "update")
        }
        DirectoryMenuChoice::Refresh => {
            pane.as_deref().into_iter().for_each(|open| open.refresh());
        }
        DirectoryMenuChoice::ExitRoom => {
            if let Some(room) = room {
                room.leave();
                Commands::remove_resource::<DirectoryLink>(&mut commands);
            }
            Commands::remove_resource::<HostUdpSwitchboard>(&mut commands);
            Commands::remove_resource::<PlayerUdpSession>(&mut commands);
            slot.back_to_browser();
            *role = frame::ProcessDuty::Listen;
        }
        DirectoryMenuChoice::Host { map: m, mode: gm } => {
            let Some(plan) = menu_host_plan(&slot, content_catalog.as_deref(), m, gm) else {
                return;
            };
            slot.0 = plan;
            *role = frame::ProcessDuty::Listen;
        }
        DirectoryMenuChoice::Join {
            listing_token,
            map,
            mode: ruleset,
        } => {
            let joined = slot
                .browsing()
                .map(|(dial, have)| intent::menu_join(dial, have, listing_token, map, ruleset));
            if let Some(plan) = joined {
                slot.0 = plan;
                *role = frame::ProcessDuty::Client;
            }
        }
    }
}

/// Starts the link worker for a host or join plan, then, once the room is
/// seated and the world is loaded, the relay transport for that side.
fn start_link_and_relay(
    mut commands: Commands,
    bridge: Option<Res<DirectoryLink>>,
    role: Res<frame::ProcessDuty>,
    link: Option<Res<PlayerUdpSession>>,
    prediction: Option<Res<crate::ClientGuessState>>,
    launch: Res<DirectoryLaunchRequest>,
    settings: Res<frame::PlayerPrefsBundle>,
    hub: Option<Res<HostUdpSwitchboard>>,
    match_info: Option<Res<GameSessionSummary>>,
    host_world: Option<Res<HostSimulationWorld>>,
) {
    let Some(lobby_bridge) = bridge else {
        let errand = match (&launch.0, &*role) {
            (Plan::Host(plan), frame::ProcessDuty::Listen) if hub.is_none() => {
                diag::info!(WireLink, "master: arming a public lobby at {}", plan.dial.address);
                Errand::Host(plan.clone())
            }
            (Plan::Join(plan), frame::ProcessDuty::Client) if link.is_none() => {
                let (room, address) = (&plan.room, &plan.dial.address);
                let (map, mode) = (&plan.map, &plan.ruleset);
                diag::info!(
                    WireLink,
                    "master joining lobby advert {room} through {address} ({map}/{mode})"
                );
                Errand::Join(plan.clone())
            }
            _ => return,
        };
        commands.insert_resource(worker::start(errand, settings.shown_name()));
        return;
    };
    let live_world = host_world
        .as_deref()
        .map(|auth| &auth.0)
        .or_else(|| prediction.as_deref().map(|pred| pred.0.world()))
        .filter(|candidate| candidate.has_static_collision());
    let (Some(match_info), Some(sim_world)) = (match_info.as_deref(), live_world) else {
        return;
    };
    let mut hello = relay_hello(Some(sim_world), match_info);
    hello.limits.datagram_size_ceiling =
        u32::try_from(RELAY_DATAGRAM_LIMIT).expect("relay packet size fits u32");
    let now_state = lobby_bridge.state();
    let hosting = matches!(now_state, DirectoryLinkStatus::Hosting { .. });
    let joining = matches!(now_state, DirectoryLinkStatus::Joining { .. } | DirectoryLinkStatus::Joined { .. });
    if hosting && hub.is_none() {
        let mut relay_hub = HostUdpSwitchboard::relay(hello, 1, lobby_bridge.mailbox());
        relay_hub.bind_offer_lane(lobby_bridge.lane());
        commands.insert_resource(relay_hub);
    }
    if joining && link.is_none() {
        let mut relay_link = PlayerUdpSession::relay(hello, lobby_bridge.mailbox());
        relay_link.bind_offer_lane(lobby_bridge.lane());
        let AssetDigestSet {
            map,
            weapon_digest: weapons,
            class_digest: classes,
            ..
        } = hello.content;
        let version = crate::WIRE_REVISION_NUMBER;
        diag::info!(
            WireLink,
            "master client relay mailbox ready, protocol={version} offering map={map:016x} weapons={weapons:016x} classes={classes:016x}"
        );
        commands.insert_resource(relay_link);
    }
}

/// A host launched straight into a map asks the room to start the match once
/// its world is live, once per link and loaded map.
fn start_hosted_match_once_loaded(
    mut asked: Local<Option<(u64, frame::OwnLevelLoadTicket)>>,
    load_gate: Option<Res<crate::HostLoadingPause>>,
    launched: Option<Res<frame::StartupPersona>>,
    admit: Res<crate::PlayerEntryTicket>,
    plan: Res<DirectoryLaunchRequest>,
    bridge: Option<Res<DirectoryLink>>,
    world_flag: Option<Res<frame::LevelPresent>>,
) {
    let (Some(lobby_bridge), Plan::Host(plan)) = (bridge, &plan.0) else {
        return;
    };
    let level_ready = world_flag.is_some_and(|world| world.0);
    let load_held = load_gate.is_some_and(|gate| gate.0);
    if load_held || !level_ready || !plan.start_on_load {
        return;
    }
    let fresh = admit
        .core
        .installed()
        .and_then(|held| held.game_session_key.is_none().then_some(held));
    let Some(load) = fresh else {
        return;
    };
    let this_load = Some((lobby_bridge.incarnation(), load));
    if *asked == this_load {
        return;
    }
    let idle_host = matches!(
        lobby_bridge.state(),
        DirectoryLinkStatus::Hosting {
            in_match: false,
            ..
        }
    );
    let (true, Some(launched)) = (idle_host, launched) else {
        return;
    };
    lobby_bridge.begin_hosted_session(launched.zone.clone(), plan.ruleset.clone());
    *asked = this_load;
}

/// The on-screen name of the client behind `member`, if the hub and world know it.
fn leaver_name(
    hub: Option<&HostUdpSwitchboard>,
    auth: Option<&HostSimulationWorld>,
    member: master_protocol::RoomOccupantId,
) -> Option<String> {
    let client = hub?.seat_for_occupant(member)?;
    Some(crate::receiver_display_text(&auth?.0, client))
}

/// Keeps the relay hub's member list and admissions in step with the room,
/// and retires the clients of members who left or could not be admitted.
fn fold_link_news(
    mut auth_world: Option<ResMut<HostSimulationWorld>>,
    mut notice_board: Option<ResMut<crate::StagedAnnouncements>>,
    bridge: Option<Res<DirectoryLink>>,
    mut relay_hub: Option<ResMut<HostUdpSwitchboard>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    if let Some(relay) = relay_hub.as_deref_mut() {
        let snapshot = bridge.state();
        if let DirectoryLinkStatus::Hosting { .. } = snapshot {
            relay.sync_room_members(snapshot.members(), snapshot.identity().member_id);
        }
        for entered in relay.take_committed_admissions() {
            let client = relay
                .seat_for_occupant(entered.member_id)
                .map_or(0, |found| found.0);
            let wire_conn = Some(entered.link_no);
            let (who, epoch, boot) = (entered.member_id, entered.round_era, entered.transfer_ticket);
            bridge.let_into_match(who, epoch, boot, wire_conn, client);
        }
    }
    for fact in bridge.drain_facts().into_iter() {
        let retired = match fact {
            DirectoryLifecycleNote::SessionEnded { .. } => None,
            DirectoryLifecycleNote::EntryRefused {
                member_id: who,
                reason: why,
            } => {
                diag::warn!(WireLink, "admission failed for member {who}: {why}");
                let hub = relay_hub.as_deref_mut();
                hub.and_then(|relay| relay.refuse_occupant(who))
            }
            DirectoryLifecycleNote::ParticipantDeparted { member_id: who } => {
                let name = leaver_name(relay_hub.as_deref(), auth_world.as_deref(), who);
                if let (Some(leaver), Some(board)) = (name, notice_board.as_deref_mut()) {
                    board.enqueue_departure(leaver);
                }
                let hub = relay_hub.as_deref_mut();
                hub.and_then(|relay| relay.release_occupant(who))
            }
        };
        if let (Some(client), Some(sim_world)) = (retired, auth_world.as_deref_mut()) {
            sim_world.0.drop_player_record(client);
        }
    }
}

/// The match the room has just started, when `state` shows one not yet offered.
fn new_match(
    state: &DirectoryLinkStatus,
    offered: Option<(ListingId, u32)>,
) -> Option<(DirectoryGameOffer, (ListingId, u32))> {
    let seat = state.identity();
    let this_match = (seat.lobby_number, seat.epoch);
    let already = offered == Some(this_match);
    if already || seat.epoch == 0 || !state.underway() {
        return None;
    }
    let (map, mode) = (state.map()?, state.mode()?);
    let offer = DirectoryGameOffer {
        map: map.to_owned(),
        mode: String::from(mode),
        game_session_key: seat.game_session_key(),
    };
    Some((offer, this_match))
}

/// The status file's text for `state`.
fn status_report(state: &DirectoryLinkStatus) -> String {
    let room = state.identity().lobby_number;
    let (word, count, live) = (state.status_word(), state.members().len(), state.underway());
    format!("state={word}\nroom={room}\nmembers={count}\nin_match={live}\n")
}

/// Replaces `path` whole, so a reader never sees half a report.
fn write_status(path: &Path, report: &str) -> std::io::Result<()> {
    let staging = path.with_extension("tmp");
    std::fs::write(&staging, report)?;
    std::fs::rename(&staging, path)
}

/// Offers each newly started match once, and logs (and, for tests, writes
/// out) every change of link state.
fn watch_link(
    mut offers: ResMut<DirectoryMatchBegin>,
    mut offered: Local<Option<(ListingId, u32)>>,
    bridge: Option<Res<DirectoryLink>>,
    mut last: Local<Option<DirectoryLinkStatus>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    let now = bridge.state();
    if let Some((offer, this_match)) = new_match(&now, *offered) {
        offers.0 = Some(offer);
        *offered = Some(this_match);
    }
    if last.as_ref() == Some(&now) {
        return;
    }
    match &now {
        DirectoryLinkStatus::Failed {
            error,
            identity: seat,
        } => {
            let LobbySessionLabel {
                join_try_number,
                lobby_number,
                epoch,
                ..
            } = *seat;
            diag::warn!(
                WireLink,
                "master relay failed attempt={join_try_number} room={lobby_number} epoch={epoch}: {error}"
            );
        }
        other => diag::info!(WireLink, "master relay: {other:?}"),
    }
    if let Some(path) = process_env(STATUS_FILE)
        && let Err(error) = write_status(Path::new(&path), &status_report(&now))
    {
        diag::warn!(WireLink, "master status file: {error}");
    }
    *last = Some(now);
}

/// Mid-match content changes (a world clip finishing its load) reach the
/// hello the relay hub checks joiners against.
fn follow_live_content(
    info: Option<Res<GameSessionSummary>>,
    relay_hub: Option<ResMut<HostUdpSwitchboard>>,
    auth_world: Option<Res<HostSimulationWorld>>,
) {
    let (Some(mut relay_hub), Some(auth_world), Some(info)) = (relay_hub, auth_world, info) else {
        return;
    };
    let now = relay_hello(Some(&auth_world.0), &info).content;
    let before = relay_hub.hello.content;
    if now == before {
        return;
    }
    if (now.map, now.weapon_digest, now.class_digest) != (before.map, before.weapon_digest, before.class_digest) {
        let (map, weapons, classes) = (now.map, now.weapon_digest, now.class_digest);
        let (old_map, old_weapons, old_classes) = (before.map, before.weapon_digest, before.class_digest);
        diag::info!(
            WireLink,
            "udp auth_world content now map={map:016x} weapons={weapons:016x} classes={classes:016x} (before {old_map:016x} / {old_weapons:016x} / {old_classes:016x})"
        );
    }
    relay_hub.hello.content = now;
}

/// Tells the room the host's authority is ticking, unless a load holds it.
fn report_ticking_authority(
    gate: Option<Res<crate::HostLoadingPause>>,
    tick_res: Option<Res<crate::host_tick::tick_systems::HostStepNumber>>,
    room: Option<Res<DirectoryLink>>,
) {
    let Some(room) = room else {
        return;
    };
    let ticking = tick_res.is_some_and(|tick| tick.0.is_some());
    if ticking && !gate.is_some_and(|gate| gate.0) {
        room.relay_host_advance();
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::RoomOccupantId;

    use super::*;

    fn seat(epoch: u32) -> LobbySessionLabel {
        let base = LobbySessionLabel::unassigned(7);
        LobbySessionLabel {
            epoch,
            member_id: RoomOccupantId([4; 16]),
            lobby_number: ListingId([3; 16]),
            ..base
        }
    }

    fn hosting(epoch: u32, in_match: bool) -> DirectoryLinkStatus {
        let members = [4u8, 5].map(|tag| RoomOccupantId([tag; 16])).to_vec();
        DirectoryLinkStatus::Hosting {
            in_match,
            members,
            pass_vote_count: 0,
            seat_capacity: 12,
            member_names: Default::default(),
            identity: seat(epoch),
            name: String::from("room"),
            map: String::from("mp_x"),
            mode: String::from("tdm"),
        }
    }

    #[test]
    fn a_started_match_is_offered_once_per_room_and_epoch() {
        let live = hosting(2, true);
        let (offer, this_match) = new_match(&live, None).unwrap();
        assert_eq!((offer.map.as_str(), offer.mode.as_str()), ("mp_x", "tdm"));
        assert_eq!(offer.game_session_key, seat(2).game_session_key());
        assert_eq!(new_match(&live, Some(this_match)), None);
        assert!(new_match(&hosting(3, true), Some(this_match)).is_some());
    }

    #[test]
    fn no_offer_before_the_match_or_before_a_seat() {
        assert_eq!(new_match(&hosting(2, false), None), None);
        assert_eq!(new_match(&hosting(0, true), None), None);
        assert_eq!(
            new_match(&DirectoryLinkStatus::Joining { identity: seat(2) }, None),
            None
        );
    }

    #[test]
    fn the_status_report_names_state_room_and_members() {
        let report = status_report(&hosting(2, true));
        let room = ListingId([3; 16]);
        assert_eq!(
            report,
            format!("state=hosting\nroom={room}\nmembers=2\nin_match=true\n")
        );
    }

    #[test]
    fn the_status_file_is_replaced_whole() {
        let folder = std::env::temp_dir().join(format!("lobby-status-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("status.txt");
        write_status(&path, "state=connecting\n").unwrap();
        write_status(&path, "state=joined\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "state=joined\n");
        assert!(!path.with_extension("tmp").exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
