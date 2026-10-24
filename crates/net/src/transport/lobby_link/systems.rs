//! The game-side half of the lobby link: Bevy systems that start the browser
//! and the link worker, carry menu choices to them, and fold what the worker
//! publishes back into the match (relay transports, members leaving, the
//! hosted match starting, the status file).

use std::path::Path;

use bevy::prelude::*;
use master_protocol::AdvertId;

use super::browser::MasterBrowser;
use super::bridge::{MasterBridge, Order};
use super::content::ContentCatalog;
use super::intent::{self, MasterLaunchIntent, Plan, process_env};
use super::legacy_content;
use super::pipes::RELAY_PACKET_BYTES;
use super::state::{
    MasterBridgeState, MasterLifecycleFact, MasterMatchOffer, MasterMatchStart, MasterMenuAction,
    PendingMasterMenuAction,
};
use super::worker::{self, Errand};
use crate::authority::runtime::AuthorityWorld;
use crate::transport::protocol::{ContentFingerprint, HandshakeHello, MatchDescriptor};
use crate::transport::relay::{UdpAuthorityHub, UdpClientLink};

/// Set by the test harness: the link writes its state here on every change.
const STATUS_FILE: &str = "BO2MP_MASTER_STATUS_FILE";

pub(crate) fn register(app: &mut App) {
    app.init_resource::<PendingMasterMenuAction>()
        .add_systems(
            Update,
            (keep_browser_open, take_menu_choice).chain().in_set(crate::ClientSet::Load),
        )
        .add_systems(
            Update,
            (start_link_and_relay, start_hosted_match_once_loaded, fold_link_news)
                .chain()
                .in_set(crate::ClientSet::Load),
        )
        .add_systems(
            Update,
            crate::signon::drive_match_boundary
                .in_set(crate::ClientSet::Load)
                .after(frame::SessionSwapApplied),
        )
        .add_systems(Update, watch_link.in_set(crate::ClientSet::Diag))
        .add_systems(FixedUpdate, follow_live_content.in_set(crate::AuthoritySet::Advance))
        .add_systems(FixedUpdate, report_ticking_authority.after(crate::AuthoritySet::Snapshot));
}

/// The fingerprint a relayed peer must match: the descriptor's content ids,
/// plus the world's gameplay and model ids once its clip is loaded.
fn relay_hello(world: Option<&sim::SimWorld>, descriptor: &MatchDescriptor) -> HandshakeHello {
    let mut content = ContentFingerprint {
        map: descriptor.map,
        weapons: descriptor.weapons,
        classes: descriptor.classes,
        gameplay: 0,
        models: 0,
    };
    if let Some(world) = world.filter(|world| world.has_world_clip()) {
        let live = ContentFingerprint::from_world(world);
        (content.gameplay, content.models) = (live.gameplay, live.models);
    }
    HandshakeHello::current(content)
}

/// The browser thread runs exactly while the menu shows the browser.
fn keep_browser_open(
    intent: Res<MasterLaunchIntent>,
    browser: Option<Res<MasterBrowser>>,
    mut commands: Commands,
) {
    match (intent.browsing(), browser.is_some()) {
        (Some((dial, have)), false) => commands.insert_resource(MasterBrowser::open(dial.clone(), have)),
        (None, true) => commands.remove_resource::<MasterBrowser>(),
        _ => {}
    }
}

/// An order for the live room, or a warning naming what the menu wanted.
fn order_room(bridge: Option<&MasterBridge>, order: Order, wanted: &str) {
    match bridge {
        Some(bridge) => bridge.give(order),
        None => diag::warn!(Net, "master lobby {wanted} requested without a live bridge"),
    }
}

/// Carries the menu's one pending choice to the browser, the room or the plan.
fn take_menu_choice(
    mut pending: ResMut<PendingMasterMenuAction>,
    mut intent: ResMut<MasterLaunchIntent>,
    mut role: ResMut<frame::RuntimeRole>,
    browser: Option<Res<MasterBrowser>>,
    bridge: Option<Res<MasterBridge>>,
    catalog: Option<Res<ContentCatalog>>,
    mut commands: Commands,
) {
    let Some(choice) = pending.0.take() else {
        return;
    };
    let bridge = bridge.as_deref();
    match choice {
        MasterMenuAction::Refresh => {
            if let Some(browser) = browser {
                browser.refresh();
            }
        }
        MasterMenuAction::UpdateLobby { map, mode } => order_room(bridge, Order::UpdateLobby { map, mode }, "update"),
        MasterMenuAction::StartMatch { map, mode } => order_room(bridge, Order::StartMatch { map, mode }, "start"),
        MasterMenuAction::VoteToSkip => order_room(bridge, Order::VoteToSkip, "vote"),
        MasterMenuAction::LeaveLobby => {
            if let Some(bridge) = bridge {
                bridge.leave();
                commands.remove_resource::<MasterBridge>();
            }
            commands.remove_resource::<UdpAuthorityHub>();
            commands.remove_resource::<UdpClientLink>();
            intent.back_to_browser();
            *role = frame::RuntimeRole::Listen;
        }
        MasterMenuAction::Host { map, mode } => {
            let Some((dial, have)) = intent.browsing() else {
                return;
            };
            let fallback;
            let catalog = match catalog.as_deref() {
                Some(catalog) => catalog,
                None => {
                    fallback = legacy_content::catalog();
                    &fallback
                }
            };
            let requires = match catalog.mask_for_map(&map) {
                Ok(requires) => requires,
                Err(error) => {
                    diag::warn!(Net, "master host content: {error}");
                    return;
                }
            };
            let plan = match intent::menu_host(&process_env, dial, have, map, mode, requires) {
                Ok(plan) => plan,
                Err(error) => {
                    diag::warn!(Net, "master host settings: {error}");
                    return;
                }
            };
            intent.0 = plan;
            *role = frame::RuntimeRole::Listen;
        }
        MasterMenuAction::Join { advert_id, map, mode } => {
            let Some((dial, have)) = intent.browsing() else {
                return;
            };
            intent.0 = intent::menu_join(dial, have, advert_id, map, mode);
            *role = frame::RuntimeRole::Client;
        }
    }
}

/// Starts the link worker for a host or join plan, then, once the room is
/// seated and the world is loaded, the relay transport for that side.
fn start_link_and_relay(
    settings: Res<frame::GameSettings>,
    intent: Res<MasterLaunchIntent>,
    role: Res<frame::RuntimeRole>,
    authority: Option<Res<AuthorityWorld>>,
    prediction: Option<Res<crate::ClientPredictionState>>,
    descriptor: Option<Res<MatchDescriptor>>,
    bridge: Option<Res<MasterBridge>>,
    hub: Option<Res<UdpAuthorityHub>>,
    link: Option<Res<UdpClientLink>>,
    mut commands: Commands,
) {
    let Some(bridge) = bridge else {
        let player = || settings.player_name.clone();
        match &intent.0 {
            Plan::Host(plan) if *role == frame::RuntimeRole::Listen && hub.is_none() => {
                diag::info!(Net, "master public lobby arming for {}", plan.dial.address);
                commands.insert_resource(worker::start(Errand::Host(plan.clone()), player()));
            }
            Plan::Join(plan) if *role == frame::RuntimeRole::Client && link.is_none() => {
                diag::info!(
                    Net,
                    "master joining lobby advert {} through {} ({}/{})",
                    plan.room,
                    plan.dial.address,
                    plan.map,
                    plan.mode
                );
                commands.insert_resource(worker::start(Errand::Join(plan.clone()), player()));
            }
            _ => {}
        }
        return;
    };
    let Some(descriptor) = descriptor.as_deref() else {
        return;
    };
    let world = match (&authority, &prediction) {
        (Some(authority), _) => &authority.0,
        (None, Some(prediction)) => prediction.0.world(),
        (None, None) => return,
    };
    if !world.has_world_clip() {
        return;
    }
    let mut hello = relay_hello(Some(world), descriptor);
    hello.limits.max_packet_bytes = RELAY_PACKET_BYTES as u32;
    let state = bridge.state();
    if hub.is_none() && matches!(state, MasterBridgeState::Hosting { .. }) {
        let mut hub = UdpAuthorityHub::relay(hello, 1, bridge.mailbox());
        hub.attach_bootstrap(bridge.lane());
        commands.insert_resource(hub);
    }
    if link.is_none() && matches!(state, MasterBridgeState::Joining { .. } | MasterBridgeState::Joined { .. }) {
        let mut link = UdpClientLink::relay(hello, bridge.mailbox());
        link.attach_bootstrap(bridge.lane());
        diag::info!(
            Net,
            "master client relay mailbox — protocol={} offering map={:016x} weapons={:016x} classes={:016x}",
            crate::PROTOCOL_VERSION,
            hello.content.map,
            hello.content.weapons,
            hello.content.classes
        );
        commands.insert_resource(link);
    }
}

/// A host launched straight into a map asks the room to start the match once
/// its world is live, once per link and loaded map.
fn start_hosted_match_once_loaded(
    bridge: Option<Res<MasterBridge>>,
    intent: Res<MasterLaunchIntent>,
    admission: Res<crate::ClientAdmission>,
    has_world: Option<Res<frame::HasWorld>>,
    launch: Option<Res<frame::LaunchIdentity>>,
    hold: Option<Res<crate::AuthorityLoadHold>>,
    mut asked: Local<Option<(u64, frame::LocalLoadKey)>>,
) {
    let (Some(bridge), Plan::Host(plan)) = (bridge, &intent.0) else {
        return;
    };
    let world_live = has_world.is_some_and(|world| world.0);
    let held = hold.is_some_and(|hold| hold.0);
    if !plan.start_on_load || !world_live || held {
        return;
    }
    let Some(load) = admission.core.installed().filter(|load| load.match_key.is_none()) else {
        return;
    };
    let this_load = Some((bridge.incarnation(), load));
    if *asked == this_load {
        return;
    }
    let (MasterBridgeState::Hosting { in_match: false, .. }, Some(launch)) = (bridge.state(), launch) else {
        return;
    };
    bridge.start_hosted_match(launch.zone.clone(), plan.mode.clone());
    *asked = this_load;
}

/// Keeps the relay hub's member list and admissions in step with the room,
/// and retires the clients of members who left or could not be admitted.
fn fold_link_news(
    bridge: Option<Res<MasterBridge>>,
    mut hub: Option<ResMut<UdpAuthorityHub>>,
    mut authority: Option<ResMut<AuthorityWorld>>,
    mut notices: Option<ResMut<crate::PendingGameNotify>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    if let Some(hub) = hub.as_deref_mut() {
        let state = bridge.state();
        if matches!(state, MasterBridgeState::Hosting { .. }) {
            hub.reconcile_relay_membership(state.members(), state.identity().member_id);
        }
        for entered in hub.take_committed_admissions() {
            let client = hub.client_of_member(entered.member_id).map_or(0, |client| client.0);
            bridge.admit_enter(
                entered.member_id,
                entered.epoch,
                entered.bootstrap_id,
                Some(entered.connection_id),
                client,
            );
        }
    }
    for fact in bridge.drain_facts() {
        let retired = match fact {
            MasterLifecycleFact::MemberLeft { member_id } => {
                let leaver = hub.as_deref().zip(authority.as_deref()).and_then(|(hub, authority)| {
                    let client = hub.client_of_member(member_id)?;
                    Some(crate::client_name_string(&authority.0, client))
                });
                if let (Some(name), Some(notices)) = (leaver, notices.as_deref_mut()) {
                    notices.push_left(name);
                }
                hub.as_deref_mut().and_then(|hub| hub.retire_member(member_id))
            }
            MasterLifecycleFact::AdmissionFailed { member_id, reason } => {
                diag::warn!(Net, "admission failed for member {member_id}: {reason}");
                hub.as_deref_mut().and_then(|hub| hub.deny_member(member_id))
            }
            MasterLifecycleFact::SessionClosed { .. } => None,
        };
        if let (Some(client), Some(authority)) = (retired, authority.as_deref_mut()) {
            authority.0.retire_client(client);
        }
    }
}

/// The match the room has just started, when `state` shows one not yet offered.
fn new_match(state: &MasterBridgeState, offered: Option<(AdvertId, u32)>) -> Option<(MasterMatchOffer, (AdvertId, u32))> {
    let seat = state.identity();
    let this_match = (seat.room_id, seat.epoch);
    if !state.in_match() || seat.epoch == 0 || offered == Some(this_match) {
        return None;
    }
    let offer = MasterMatchOffer {
        map: state.map()?.to_owned(),
        mode: state.mode()?.to_owned(),
        match_key: seat.match_key(),
    };
    Some((offer, this_match))
}

/// The status file's text for `state`.
fn status_report(state: &MasterBridgeState) -> String {
    let room = state.identity().room_id;
    let members = state.members().len();
    format!("state={}\nroom={room}\nmembers={members}\nin_match={}\n", state.status_word(), state.in_match())
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
    bridge: Option<Res<MasterBridge>>,
    mut last: Local<Option<MasterBridgeState>>,
    mut offered: Local<Option<(AdvertId, u32)>>,
    mut offers: ResMut<MasterMatchStart>,
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
    if let MasterBridgeState::Failed { error, identity } = &now {
        diag::warn!(
            Net,
            "master relay failed attempt={} room={} epoch={}: {error}",
            identity.attempt_id,
            identity.room_id,
            identity.epoch
        );
    } else {
        diag::info!(Net, "master relay: {now:?}");
    }
    if let Some(path) = process_env(STATUS_FILE)
        && let Err(error) = write_status(Path::new(&path), &status_report(&now))
    {
        diag::warn!(Net, "master status file: {error}");
    }
    *last = Some(now);
}

/// Mid-match content changes (a world clip finishing its load) reach the
/// hello the relay hub checks joiners against.
fn follow_live_content(
    hub: Option<ResMut<UdpAuthorityHub>>,
    authority: Option<Res<AuthorityWorld>>,
    descriptor: Option<Res<MatchDescriptor>>,
) {
    let (Some(mut hub), Some(authority), Some(descriptor)) = (hub, authority, descriptor) else {
        return;
    };
    let now = relay_hello(Some(&authority.0), &descriptor).content;
    let was = hub.hello.content;
    if was == now {
        return;
    }
    if (was.map, was.weapons, was.classes) != (now.map, now.weapons, now.classes) {
        diag::info!(
            Net,
            "udp authority content now map={:016x} weapons={:016x} classes={:016x} (was {:016x} / {:016x} / {:016x})",
            now.map,
            now.weapons,
            now.classes,
            was.map,
            was.weapons,
            was.classes
        );
    }
    hub.hello.content = now;
}

/// Tells the room the host's authority is ticking, unless a load holds it.
fn report_ticking_authority(
    bridge: Option<Res<MasterBridge>>,
    server_tick: Option<Res<crate::authority::runtime::ServerTick>>,
    hold: Option<Res<crate::AuthorityLoadHold>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    let held = hold.is_some_and(|hold| hold.0);
    if !held && server_tick.is_some_and(|tick| tick.0.is_some()) {
        bridge.report_authority_progress();
    }
}

#[cfg(test)]
mod tests {
    use master_protocol::MemberId;

    use super::super::state::SessionIdentity;
    use super::*;

    fn seat(epoch: u32) -> SessionIdentity {
        SessionIdentity { room_id: AdvertId([3; 16]), member_id: MemberId([4; 16]), epoch, ..SessionIdentity::unassigned(7) }
    }

    fn hosting(epoch: u32, in_match: bool) -> MasterBridgeState {
        MasterBridgeState::Hosting {
            identity: seat(epoch),
            name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            members: vec![MemberId([4; 16]), MemberId([5; 16])],
            member_names: Default::default(),
            max_players: 12,
            skip_votes: 0,
            in_match,
        }
    }

    #[test]
    fn a_started_match_is_offered_once_per_room_and_epoch() {
        let live = hosting(2, true);
        let (offer, this_match) = new_match(&live, None).unwrap();
        assert_eq!((offer.map.as_str(), offer.mode.as_str()), ("mp_x", "tdm"));
        assert_eq!(offer.match_key, seat(2).match_key());
        assert_eq!(new_match(&live, Some(this_match)), None);
        assert!(new_match(&hosting(3, true), Some(this_match)).is_some());
    }

    #[test]
    fn no_offer_before_the_match_or_before_a_seat() {
        assert_eq!(new_match(&hosting(2, false), None), None);
        assert_eq!(new_match(&hosting(0, true), None), None);
        assert_eq!(new_match(&MasterBridgeState::Joining { identity: seat(2) }, None), None);
    }

    #[test]
    fn the_status_report_names_state_room_and_members() {
        let report = status_report(&hosting(2, true));
        let room = AdvertId([3; 16]);
        assert_eq!(report, format!("state=hosting\nroom={room}\nmembers=2\nin_match=true\n"));
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
