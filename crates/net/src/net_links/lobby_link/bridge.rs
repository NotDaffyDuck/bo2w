//! The game's handle on a live lobby link, and the state both sides share.
//!
//! The game thread queues orders and reads the published state; the link
//! worker drains the orders, talks to the service and publishes what happened.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use bevy::prelude::Resource;
use master_protocol::{RoomOccupantId, LobbyOverview, RoomShutdownCause};
use tokio_util::sync::CancellationToken;

use super::fault::LinkFailureRecord;
use super::room_view::{DirectoryLinkStatus, DirectoryLifecycleNote, LobbySessionLabel};
use crate::net_links::match_handoff::TransferChannelState;
use crate::net_links::relay::ForwardingInbox;

/// What the game asks of the link worker.
#[derive(Debug)]
pub(super) enum Order {
    RefreshRoom {
        map: String,
        ruleset: String,
    },
    BeginGame {
        map: String,
        ruleset: String,
    },
    BallotForSkip,
    Kick {
        member_id: RoomOccupantId,
    },
    MapLoaded {
        epoch: u32,
        map: u64,
        weapon_digest: u64,
        class_digest: u64,
    },
    HostMapLoaded {
        epoch: u32,
        map: u64,
        weapon_digest: u64,
        class_digest: u64,
        decrypt_secret: ::frame::OwnLevelLoadTicket,
    },
    RoundOver {
        game_session_key: ::frame::GameSessionStamp,
    },
    HostAdvance,
    LetIntoMatch {
        member_id: RoomOccupantId,
        epoch: u32,
        handoff_no: u32,
        wire_conn: Option<u64>,
        seat_no: u32,
    },
    Shutdown,
}

/// Everything the game thread and the link worker both touch.
#[derive(Clone)]
pub(super) struct Shared {
    state: Arc<Mutex<DirectoryLinkStatus>>,
    orders: Arc<Mutex<Vec<Order>>>,
    installed_load: Arc<Mutex<Option<::frame::OwnLevelLoadTicket>>>,
    facts: Arc<Mutex<Vec<DirectoryLifecycleNote>>>,
    pub mailbox: ForwardingInbox,
    pub lane: Arc<TransferChannelState>,
}

/// The id a terminal state reports: the one already published, unless
/// nothing was published yet.
fn keep_known(current: &DirectoryLinkStatus, fallback: LobbySessionLabel) -> LobbySessionLabel {
    let known = current.identity();
    if known.join_try_number == 0 { fallback } else { known }
}

impl Shared {
    pub fn new(start: DirectoryLinkStatus, mailbox: ForwardingInbox, lane: Arc<TransferChannelState>) -> Self {
        Self {
            state: Arc::new(Mutex::new(start)),
            orders: Arc::default(),
            installed_load: Arc::default(),
            facts: Arc::default(),
            mailbox,
            lane,
        }
    }

    fn state_slot(&self) -> std::sync::MutexGuard<'_, DirectoryLinkStatus> {
        self.state.lock().expect("master state poisoned")
    }

    pub fn snapshot(&self) -> DirectoryLinkStatus {
        self.state_slot().clone()
    }

    fn give(&self, order: Order) {
        self.orders.lock().expect("master command queue poisoned").push(order);
    }

    pub fn take_orders(&self) -> Vec<Order> {
        std::mem::take(&mut *self.orders.lock().expect("master command queue poisoned"))
    }

    pub fn loaded_ticket(&self) -> Option<::frame::OwnLevelLoadTicket> {
        *self.installed_load.lock().expect("installed load poisoned")
    }

    pub fn log_fact(&self, entry: DirectoryLifecycleNote) {
        self.facts.lock().expect("master facts poisoned").push(entry);
    }

    pub fn drain_facts(&self) -> Vec<DirectoryLifecycleNote> {
        let mut facts = self.facts.lock().expect("master facts poisoned");
        std::mem::take(&mut *facts)
    }

    /// Ends the link as failed, unless it already ended.
    pub fn abort_with(&self, fallback: LobbySessionLabel, cause: LinkFailureRecord) {
        let mut slot = self.state_slot();
        if slot.is_terminal() {
            return;
        }
        let identity = keep_known(&slot, fallback);
        diag::warn!(WireLink, "master relay aborted: {cause}");
        *slot = DirectoryLinkStatus::Failed { identity, error: cause };
    }

    /// Ends the link because the service closed the room, unless it already ended.
    pub fn room_closed(&self, fallback: LobbySessionLabel, why: RoomShutdownCause) {
        let mut slot = self.state_slot();
        if !slot.is_terminal() {
            let identity = keep_known(&slot, fallback);
            *slot = DirectoryLinkStatus::Closed { identity, reason: why };
        }
    }

    /// Ends the link after a leave the service confirmed, unless it already ended.
    pub fn left(&self, identity: LobbySessionLabel) {
        let mut slot = self.state_slot();
        if !slot.is_terminal() {
            *slot = DirectoryLinkStatus::Left { identity };
        }
    }

    /// Publishes the room as this seat sees it; a joiner the room does not
    /// list yet is still joining.
    pub fn show_room(&self, identity: LobbySessionLabel, view: &LobbyOverview, pass_vote_count: u8, hosting: bool) {
        let mut slot = self.state_slot();
        if slot.is_terminal() {
            return;
        }
        let seated = hosting || view.contains(identity.member_id);
        if !seated {
            *slot = DirectoryLinkStatus::Joining { identity };
            return;
        }
        let name = view.name.clone();
        let map = view.map.clone();
        let mode = view.mode.clone();
        let members = view.members.clone();
        let member_names = view.member_names.clone();
        let seat_capacity = view.seat_capacity;
        let underway = view.phase.is_playing();
        *slot = if hosting {
            DirectoryLinkStatus::Hosting {
                identity, name, map, mode, members, member_names, seat_capacity, pass_vote_count, in_match: underway,
            }
        } else {
            DirectoryLinkStatus::Joined {
                identity, name, map, mode, members, member_names, seat_capacity, pass_vote_count, playing: underway,
            }
        };
    }
}

/// Which members finished loading the map; only they are offered a bootstrap.
pub(super) fn map_ready(lane: &TransferChannelState) -> std::sync::MutexGuard<'_, HashSet<RoomOccupantId>> {
    lane.host_map_ready.lock().expect("bootstrap readiness poisoned")
}

/// The game's handle on one hosted or joined room.
#[derive(Resource)]
pub struct DirectoryLink {
    shared: Shared,
    /// Fired to stop the worker without waiting for the service.
    close: CancellationToken,
    launch_no: u64,
    task: Option<JoinHandle<()>>,
}

impl DirectoryLink {
    pub(super) fn running(shared: Shared, close: CancellationToken, launch_no: u64, task: JoinHandle<()>) -> Self {
        Self { shared, close, launch_no, task: Some(task) }
    }

    pub fn state(&self) -> DirectoryLinkStatus {
        self.shared.snapshot()
    }

    /// The game hit a fault of its own: publish it and drop the link.
    pub fn mark_failed(&self, cause_note: &str) {
        let fault = LinkFailureRecord::new("gameplay", "local", cause_note);
        self.shared.abort_with(self.state().identity(), fault);
        self.hang_up();
    }

    fn hang_up(&self) {
        self.shared.give(Order::Shutdown);
        self.close.cancel();
    }

    pub fn mailbox(&self) -> ForwardingInbox {
        self.shared.mailbox.clone()
    }

    pub(super) fn lane(&self) -> Arc<TransferChannelState> {
        Arc::clone(&self.shared.lane)
    }

    pub(super) fn give(&self, order: Order) {
        self.shared.give(order);
    }

    pub fn assign_installed_loadout(&self, load: Option<::frame::OwnLevelLoadTicket>) {
        *self.shared.installed_load.lock().expect("installed load poisoned") = load;
    }

    pub fn send_map_loaded(&self, epoch: u32, summary: crate::GameSessionSummary) {
        let crate::GameSessionSummary { map, weapons, classes, .. } = summary;
        self.give(Order::MapLoaded { epoch, map, weapon_digest: weapons, class_digest: classes });
    }

    pub fn send_host_scene_loaded(
        &self,
        epoch: u32,
        summary: crate::GameSessionSummary,
        decrypt_secret: ::frame::OwnLevelLoadTicket,
    ) {
        let crate::GameSessionSummary { map, weapons, classes, .. } = summary;
        self.give(Order::HostMapLoaded { epoch, map, weapon_digest: weapons, class_digest: classes, decrypt_secret });
    }

    pub fn relay_host_advance(&self) {
        self.give(Order::HostAdvance);
    }

    pub fn send_match_ended(&self, game_session_key: ::frame::GameSessionStamp) {
        self.give(Order::RoundOver { game_session_key });
    }

    pub fn begin_hosted_session(&self, arena: String, ruleset: String) {
        self.give(Order::BeginGame { map: arena, ruleset });
    }

    pub fn leave(&self) {
        self.hang_up();
    }

    pub fn let_into_match(
        &self,
        member_id: RoomOccupantId,
        epoch: u32,
        handoff_no: u32,
        wire_conn: Option<u64>,
        seat_no: u32,
    ) {
        self.give(Order::LetIntoMatch { member_id, epoch, handoff_no, wire_conn, seat_no });
    }

    /// Unique per bridge; tells one hosted room's loads from the next.
    pub fn incarnation(&self) -> u64 {
        self.launch_no
    }

    pub fn drain_facts(&self) -> Vec<DirectoryLifecycleNote> {
        self.shared.drain_facts()
    }

    /// Lets a test wait for the worker to finish.
    #[cfg(test)]
    pub(super) fn take_worker(&mut self) -> Option<JoinHandle<()>> {
        self.task.take()
    }
}

impl Drop for DirectoryLink {
    fn drop(&mut self) {
        // A poisoned queue means the worker is gone already; just cancel.
        if let Ok(mut orders) = self.shared.orders.lock() {
            orders.push(Order::Shutdown);
        }
        self.close.cancel();
        // Detach: the worker finishes its goodbye on its own.
        self.task.take();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use master_protocol::{ListingId, LobbyStage};

    use super::*;

    fn shared() -> Shared {
        let start = DirectoryLinkStatus::Connecting { identity: LobbySessionLabel::unassigned(0) };
        Shared::new(start, ForwardingInbox::new(4), TransferChannelState::new())
    }

    fn view(members: Vec<RoomOccupantId>) -> LobbyOverview {
        let first_rev = 1;
        LobbyOverview {
            lobby_number: ListingId([1; 16]),
            revision: first_rev,
            epoch: 0,
            phase: LobbyStage::Gathering,
            name: "room".into(),
            map: "mp_x".into(),
            mode: "dm".into(),
            host: members[0],
            members,
            member_names: HashMap::new(),
            open_to_join: true,
            seat_capacity: 6,
            requires: Default::default(),
        }
    }

    fn seated(member: u8) -> LobbySessionLabel {
        LobbySessionLabel { member_id: RoomOccupantId([member; 16]), ..LobbySessionLabel::unassigned(3) }
    }

    #[test]
    fn a_joiner_is_joined_only_once_the_room_lists_it() {
        let link = shared();
        let room = view(vec![RoomOccupantId([1; 16])]);
        link.show_room(seated(2), &room, 0, false);
        assert!(matches!(link.snapshot(), DirectoryLinkStatus::Joining { .. }));
        let room = view(vec![RoomOccupantId([1; 16]), RoomOccupantId([2; 16])]);
        link.show_room(seated(2), &room, 1, false);
        assert_eq!(link.snapshot().pass_vote_count(), 1);
        assert!(matches!(link.snapshot(), DirectoryLinkStatus::Joined { .. }));
        link.show_room(seated(1), &room, 0, true);
        assert!(matches!(link.snapshot(), DirectoryLinkStatus::Hosting { seat_capacity: 6, .. }));
    }

    #[test]
    fn the_first_ending_sticks() {
        let link = shared();
        link.abort_with(seated(9), LinkFailureRecord::new("connect", "join", "refused"));
        assert_eq!(link.snapshot().identity(), seated(9));
        link.left(seated(1));
        link.room_closed(seated(1), RoomShutdownCause::HostLeft);
        link.show_room(seated(1), &view(vec![RoomOccupantId([1; 16])]), 0, true);
        assert!(matches!(link.snapshot(), DirectoryLinkStatus::Failed { .. }));
    }

    #[test]
    fn an_ending_keeps_the_published_identity() {
        let link = shared();
        link.show_room(seated(1), &view(vec![RoomOccupantId([1; 16])]), 0, true);
        link.room_closed(seated(7), RoomShutdownCause::TenancyLapsed);
        assert_eq!(link.snapshot().identity(), seated(1));
    }

    #[test]
    fn facts_and_orders_drain_once() {
        let link = shared();
        link.give(Order::BallotForSkip);
        link.log_fact(DirectoryLifecycleNote::ParticipantDeparted { member_id: RoomOccupantId([4; 16]) });
        assert_eq!(link.take_orders().len(), 1);
        assert!(link.take_orders().is_empty());
        assert_eq!(link.drain_facts().len(), 1);
        assert!(link.drain_facts().is_empty());
    }
}
