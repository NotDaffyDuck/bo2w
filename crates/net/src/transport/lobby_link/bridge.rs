//! The game's handle on a live lobby link, and the state both sides share.
//!
//! The game thread queues orders and reads the published state; the link
//! worker drains the orders, talks to the service and publishes what happened.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use bevy::prelude::Resource;
use master_protocol::{MemberId, RoomView, SessionCloseReason};
use tokio_util::sync::CancellationToken;

use super::fault::TransportFault;
use super::state::{MasterBridgeState, MasterLifecycleFact, SessionIdentity};
use crate::transport::bootstrap::BootstrapLane;
use crate::transport::relay::RelayMailbox;

/// What the game asks of the link worker.
#[derive(Debug)]
pub(super) enum Order {
    UpdateLobby {
        map: String,
        mode: String,
    },
    StartMatch {
        map: String,
        mode: String,
    },
    VoteToSkip,
    MapLoaded {
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
    },
    HostWorldReady {
        epoch: u32,
        map: u64,
        weapons: u64,
        classes: u64,
        load_key: ::frame::LocalLoadKey,
    },
    MatchEnded {
        match_key: ::frame::MatchKey,
    },
    AuthorityProgress,
    AdmitEnter {
        member_id: MemberId,
        epoch: u32,
        bootstrap_id: u32,
        connection_id: Option<u64>,
        client_id: u32,
    },
    Shutdown,
}

/// Everything the game thread and the link worker both touch.
#[derive(Clone)]
pub(super) struct Shared {
    state: Arc<Mutex<MasterBridgeState>>,
    orders: Arc<Mutex<Vec<Order>>>,
    installed_load: Arc<Mutex<Option<::frame::LocalLoadKey>>>,
    facts: Arc<Mutex<Vec<MasterLifecycleFact>>>,
    pub mailbox: RelayMailbox,
    pub lane: Arc<BootstrapLane>,
}

/// The id a terminal state reports: the one already published, unless
/// nothing was published yet.
fn keep_known(current: &MasterBridgeState, fallback: SessionIdentity) -> SessionIdentity {
    let known = current.identity();
    if known.attempt_id == 0 { fallback } else { known }
}

impl Shared {
    pub fn new(start: MasterBridgeState, mailbox: RelayMailbox, lane: Arc<BootstrapLane>) -> Self {
        Self {
            state: Arc::new(Mutex::new(start)),
            orders: Arc::default(),
            installed_load: Arc::default(),
            facts: Arc::default(),
            mailbox,
            lane,
        }
    }

    fn state_slot(&self) -> std::sync::MutexGuard<'_, MasterBridgeState> {
        self.state.lock().expect("master state poisoned")
    }

    pub fn snapshot(&self) -> MasterBridgeState {
        self.state_slot().clone()
    }

    fn give(&self, order: Order) {
        self.orders
            .lock()
            .expect("master command queue poisoned")
            .push(order);
    }

    pub fn take_orders(&self) -> Vec<Order> {
        std::mem::take(&mut *self.orders.lock().expect("master command queue poisoned"))
    }

    pub fn installed(&self) -> Option<::frame::LocalLoadKey> {
        *self.installed_load.lock().expect("installed load poisoned")
    }

    pub fn note(&self, fact: MasterLifecycleFact) {
        self.facts.lock().expect("master facts poisoned").push(fact);
    }

    pub fn drain_facts(&self) -> Vec<MasterLifecycleFact> {
        std::mem::take(&mut *self.facts.lock().expect("master facts poisoned"))
    }

    /// Ends the link as failed, unless it already ended.
    pub fn fail(&self, fallback: SessionIdentity, error: TransportFault) {
        let mut slot = self.state_slot();
        if slot.is_terminal() {
            return;
        }
        let identity = keep_known(&slot, fallback);
        diag::warn!(Net, "master relay failed: {error}");
        *slot = MasterBridgeState::Failed { identity, error };
    }

    /// Ends the link because the service closed the room, unless it already ended.
    pub fn room_closed(&self, fallback: SessionIdentity, reason: SessionCloseReason) {
        let mut slot = self.state_slot();
        if !slot.is_terminal() {
            let identity = keep_known(&slot, fallback);
            *slot = MasterBridgeState::Closed { identity, reason };
        }
    }

    /// Ends the link after a leave the service confirmed, unless it already ended.
    pub fn left(&self, identity: SessionIdentity) {
        let mut slot = self.state_slot();
        if !slot.is_terminal() {
            *slot = MasterBridgeState::Left { identity };
        }
    }

    /// Publishes the room as this seat sees it; a joiner the room does not
    /// list yet is still joining.
    pub fn show_room(&self, identity: SessionIdentity, view: &RoomView, skip_votes: u8, is_host: bool) {
        let mut slot = self.state_slot();
        if slot.is_terminal() {
            return;
        }
        let seated = is_host || view.contains(identity.member_id);
        if !seated {
            *slot = MasterBridgeState::Joining { identity };
            return;
        }
        let name = view.name.clone();
        let map = view.map.clone();
        let mode = view.mode.clone();
        let members = view.members.clone();
        let member_names = view.member_names.clone();
        let max_players = view.max_players;
        let in_match = view.phase.in_match();
        *slot = if is_host {
            MasterBridgeState::Hosting {
                identity, name, map, mode, members, member_names, max_players, skip_votes, in_match,
            }
        } else {
            MasterBridgeState::Joined {
                identity, name, map, mode, members, member_names, max_players, skip_votes, in_match,
            }
        };
    }
}

/// Which members finished loading the map; only they are offered a bootstrap.
pub(super) fn map_ready(lane: &BootstrapLane) -> std::sync::MutexGuard<'_, HashSet<MemberId>> {
    lane.host_map_ready.lock().expect("bootstrap readiness poisoned")
}

/// The game's handle on one hosted or joined room.
#[derive(Resource)]
pub struct MasterBridge {
    shared: Shared,
    /// Fired to stop the worker without waiting for the service.
    close: CancellationToken,
    incarnation: u64,
    worker: Option<JoinHandle<()>>,
}

impl MasterBridge {
    pub(super) fn running(
        shared: Shared,
        close: CancellationToken,
        incarnation: u64,
        worker: JoinHandle<()>,
    ) -> Self {
        Self {
            shared,
            close,
            incarnation,
            worker: Some(worker),
        }
    }

    pub fn state(&self) -> MasterBridgeState {
        self.shared.snapshot()
    }

    /// The game hit a fault of its own: publish it and drop the link.
    pub fn fail(&self, reason: &str) {
        let fault = TransportFault::new("gameplay", "local", reason);
        self.shared.fail(self.state().identity(), fault);
        self.hang_up();
    }

    fn hang_up(&self) {
        self.shared.give(Order::Shutdown);
        self.close.cancel();
    }

    pub fn mailbox(&self) -> RelayMailbox {
        self.shared.mailbox.clone()
    }

    pub(super) fn lane(&self) -> Arc<BootstrapLane> {
        Arc::clone(&self.shared.lane)
    }

    pub(super) fn give(&self, order: Order) {
        self.shared.give(order);
    }

    pub fn set_installed_load(&self, load: Option<::frame::LocalLoadKey>) {
        *self.shared.installed_load.lock().expect("installed load poisoned") = load;
    }

    pub fn report_map_loaded(&self, epoch: u32, descriptor: crate::MatchDescriptor) {
        let crate::MatchDescriptor { map, weapons, classes, .. } = descriptor;
        self.give(Order::MapLoaded { epoch, map, weapons, classes });
    }

    pub fn report_host_world_ready(
        &self,
        epoch: u32,
        descriptor: crate::MatchDescriptor,
        load_key: ::frame::LocalLoadKey,
    ) {
        let crate::MatchDescriptor { map, weapons, classes, .. } = descriptor;
        self.give(Order::HostWorldReady { epoch, map, weapons, classes, load_key });
    }

    pub fn report_authority_progress(&self) {
        self.give(Order::AuthorityProgress);
    }

    pub fn report_match_ended(&self, match_key: ::frame::MatchKey) {
        self.give(Order::MatchEnded { match_key });
    }

    pub fn start_hosted_match(&self, map: String, mode: String) {
        self.give(Order::StartMatch { map, mode });
    }

    pub fn leave(&self) {
        self.hang_up();
    }

    pub fn admit_enter(
        &self,
        member_id: MemberId,
        epoch: u32,
        bootstrap_id: u32,
        connection_id: Option<u64>,
        client_id: u32,
    ) {
        self.give(Order::AdmitEnter { member_id, epoch, bootstrap_id, connection_id, client_id });
    }

    /// Unique per bridge; tells one hosted room's loads from the next.
    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    pub fn drain_facts(&self) -> Vec<MasterLifecycleFact> {
        self.shared.drain_facts()
    }

    /// Lets a test wait for the worker to finish.
    #[cfg(test)]
    pub(super) fn take_worker(&mut self) -> Option<JoinHandle<()>> {
        self.worker.take()
    }
}

impl Drop for MasterBridge {
    fn drop(&mut self) {
        // A poisoned queue means the worker is gone already; just cancel.
        if let Ok(mut orders) = self.shared.orders.lock() {
            orders.push(Order::Shutdown);
        }
        self.close.cancel();
        // Detach: the worker finishes its goodbye on its own.
        self.worker.take();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use master_protocol::{AdvertId, RoomPhase};

    use super::*;

    fn shared() -> Shared {
        let start = MasterBridgeState::Connecting { identity: SessionIdentity::unassigned(0) };
        Shared::new(start, RelayMailbox::new(4), BootstrapLane::new())
    }

    fn view(members: Vec<MemberId>) -> RoomView {
        RoomView {
            room_id: AdvertId([1; 16]),
            revision: 1,
            epoch: 0,
            phase: RoomPhase::Gathering,
            name: "room".into(),
            map: "mp_x".into(),
            mode: "dm".into(),
            host: members[0],
            members,
            member_names: HashMap::new(),
            joinable: true,
            max_players: 6,
            requires: Default::default(),
        }
    }

    fn seated(member: u8) -> SessionIdentity {
        SessionIdentity { member_id: MemberId([member; 16]), ..SessionIdentity::unassigned(3) }
    }

    #[test]
    fn a_joiner_is_joined_only_once_the_room_lists_it() {
        let link = shared();
        let room = view(vec![MemberId([1; 16])]);
        link.show_room(seated(2), &room, 0, false);
        assert!(matches!(link.snapshot(), MasterBridgeState::Joining { .. }));
        let room = view(vec![MemberId([1; 16]), MemberId([2; 16])]);
        link.show_room(seated(2), &room, 1, false);
        assert_eq!(link.snapshot().skip_votes(), 1);
        assert!(matches!(link.snapshot(), MasterBridgeState::Joined { .. }));
        link.show_room(seated(1), &room, 0, true);
        assert!(matches!(link.snapshot(), MasterBridgeState::Hosting { max_players: 6, .. }));
    }

    #[test]
    fn the_first_ending_sticks() {
        let link = shared();
        link.fail(seated(9), TransportFault::new("connect", "join", "refused"));
        assert_eq!(link.snapshot().identity(), seated(9));
        link.left(seated(1));
        link.room_closed(seated(1), SessionCloseReason::HostLeft);
        link.show_room(seated(1), &view(vec![MemberId([1; 16])]), 0, true);
        assert!(matches!(link.snapshot(), MasterBridgeState::Failed { .. }));
    }

    #[test]
    fn an_ending_keeps_the_published_identity() {
        let link = shared();
        link.show_room(seated(1), &view(vec![MemberId([1; 16])]), 0, true);
        link.room_closed(seated(7), SessionCloseReason::LeaseExpired);
        assert_eq!(link.snapshot().identity(), seated(1));
    }

    #[test]
    fn facts_and_orders_drain_once() {
        let link = shared();
        link.give(Order::VoteToSkip);
        link.note(MasterLifecycleFact::MemberLeft { member_id: MemberId([4; 16]) });
        assert_eq!(link.take_orders().len(), 1);
        assert!(link.take_orders().is_empty());
        assert_eq!(link.drain_facts().len(), 1);
        assert!(link.drain_facts().is_empty());
    }
}
