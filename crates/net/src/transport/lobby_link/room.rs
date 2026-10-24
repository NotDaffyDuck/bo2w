//! One seat in one room, as the link worker sees it.
//!
//! The worker owns a `Room` and feeds it whatever arrives: the game's
//! orders, the service's frames, finished bootstrap sends and the clock.
//! The room keeps the session and (when hosting) the match admission state,
//! and answers by queueing control frames and bootstrap jobs.

use std::collections::HashSet;

use master_protocol::{
    AdvertId, ControlFrame, ControlRequest, MemberId, PeerEvent, RelayDatagram, RequestBody,
    ResponseBody, RoomView, decode_relay_stream, encode_relay_stream,
};
use tokio::sync::mpsc::Sender;

use super::bridge::{Order, Shared, map_ready};
use super::fault::{TransportFault, post};
use super::pipes::{BootstrapJob, addressed_to_us};
use super::quic::wall_ms;
use super::state::{MasterLifecycleFact, SessionIdentity};
use crate::session_core::{
    HostMatchApply, HostMatchCore, HostMatchEffect, HostMatchEvent, HostWorldReady, SessionCore,
    SessionEvent,
};
use crate::transport::bootstrap::{BootstrapAck, BootstrapMessage, decode_bootstrap, epoch_applies};
use ::frame::MatchKey;

/// The event word inside a host transition line (`... event=<word> stage_after=...`).
fn event_word(transition: &str) -> &str {
    let Some(after) = transition.split(" event=").nth(1) else {
        return "";
    };
    after.split(" stage_after=").next().unwrap_or("")
}

/// Heartbeats are logged only when they cause something.
fn worth_logging(applied: &HostMatchApply) -> bool {
    if !applied.effects.is_empty() {
        return true;
    }
    !matches!(event_word(&applied.transition), "tick" | "authority progress")
}

fn count(voters: usize) -> u8 {
    u8::try_from(voters).unwrap_or(0)
}

pub(super) struct Room {
    shared: Shared,
    role: &'static str,
    is_host: bool,
    pub identity: SessionIdentity,
    session: SessionCore,
    /// Admission of joiners into the running match; used only when hosting.
    host: HostMatchCore,
    skip_voters: HashSet<MemberId>,
    last_view: Option<RoomView>,
    /// The match this host last started; 0 before the first.
    started_epoch: u32,
    next_request: u64,
    control: Sender<ControlFrame>,
    jobs: Sender<BootstrapJob>,
}

impl Room {
    pub fn new(
        shared: Shared,
        role: &'static str,
        is_host: bool,
        identity: SessionIdentity,
        control: Sender<ControlFrame>,
        jobs: Sender<BootstrapJob>,
    ) -> Self {
        Self {
            session: SessionCore::for_connection(identity.room_id.0, identity.attempt_id),
            host: HostMatchCore::default(),
            skip_voters: HashSet::new(),
            last_view: None,
            started_epoch: 0,
            next_request: 1,
            shared,
            role,
            is_host,
            identity,
            control,
            jobs,
        }
    }

    fn fault(&self, step: &'static str, cause: impl ToString) -> TransportFault {
        TransportFault::new(step, self.role, cause.to_string())
    }

    /// Queues a request to the service and returns the id its answer will carry.
    pub fn request(&mut self, body: RequestBody) -> Result<u64, TransportFault> {
        let request_id = self.next_request;
        self.next_request = self.next_request.saturating_add(1);
        let frame = ControlFrame::Request(ControlRequest { request_id, body });
        post(&self.control, frame, "control_write", self.role)?;
        Ok(request_id)
    }

    fn host_event(&mut self, event: HostMatchEvent) -> HostMatchApply {
        let applied = self.host.apply(event);
        if worth_logging(&applied) {
            diag::info!(Net, "session-transition {}", applied.transition);
        }
        applied
    }

    /// Applies one host match event and carries out what it asks for.
    fn host_step(&mut self, event: HostMatchEvent) -> Result<(), TransportFault> {
        let applied = self.host_event(event);
        self.run_effects(applied.effects)
    }

    /// Carries out host match effects in order; a failure among them is
    /// reported only after the rest have run.
    fn run_effects(&mut self, effects: Vec<HostMatchEffect>) -> Result<(), TransportFault> {
        let mut failed = None;
        for effect in effects {
            match effect {
                HostMatchEffect::PublishReady { match_key, ready } => {
                    self.request(RequestBody::HostWorldReady {
                        room_id: AdvertId(match_key.session_id),
                        epoch: match_key.match_epoch,
                        map: ready.map,
                        weapons: ready.weapons,
                        classes: ready.classes,
                    })?;
                }
                HostMatchEffect::PublishEnd { match_key } => {
                    map_ready(&self.shared.lane).clear();
                    self.request(RequestBody::EndMatch {
                        room_id: AdvertId(match_key.session_id),
                        epoch: match_key.match_epoch,
                    })?;
                }
                HostMatchEffect::FlushBootstraps { member } => {
                    map_ready(&self.shared.lane).insert(member);
                }
                HostMatchEffect::Enter { .. } => {}
                HostMatchEffect::CancelPeer { member, reason, match_key } => {
                    map_ready(&self.shared.lane).remove(&member);
                    self.shared.note(MasterLifecycleFact::AdmissionFailed { member_id: member, reason });
                    self.request(RequestBody::AdmissionFailed {
                        member_id: member,
                        epoch: match_key.match_epoch,
                        reason,
                    })?;
                }
                HostMatchEffect::Fail { fail } => failed = Some(fail),
            }
        }
        match failed {
            Some(fail) => Err(self.fault("host_match", fail)),
            None => Ok(()),
        }
    }

    /// Acts on one order from the game. A shutdown returns the id of the
    /// close or leave request, whose answer ends the link.
    pub fn order(&mut self, order: Order) -> Result<Option<u64>, TransportFault> {
        match order {
            Order::UpdateLobby { map, mode } => {
                self.request(RequestBody::SetOptions { map, mode, joinable: true })?;
            }
            Order::StartMatch { map, mode } => {
                self.request(RequestBody::StartMatch { map, mode })?;
            }
            Order::VoteToSkip => {
                self.request(RequestBody::VoteToSkip)?;
            }
            Order::MapLoaded { epoch, map, weapons, classes } => {
                self.request(RequestBody::MapLoaded { epoch, map, weapons, classes })?;
                if self.is_host {
                    let loaded = HostWorldReady { epoch, map, weapons, classes };
                    let host_seat = MemberId([0; 16]);
                    self.host_step(HostMatchEvent::MapLoaded { member: host_seat, loaded, now_ms: wall_ms() })?;
                }
            }
            Order::HostWorldReady { epoch, map, weapons, classes, load_key } => {
                let current = self.is_host
                    && self.shared.installed() == Some(load_key)
                    && load_key.belongs_to(self.host.match_key())
                    && epoch == load_key.match_key.match_epoch;
                if !current {
                    return Ok(None);
                }
                self.host.set_incarnation(load_key.incarnation);
                self.host.set_local_load_id(Some(load_key.local_load_request_id));
                let ready = HostWorldReady { epoch, map, weapons, classes };
                let mut applied = self.host_event(HostMatchEvent::AuthorityReady { ready, now_ms: wall_ms() });
                if applied.accepted {
                    let publish = HostMatchEffect::PublishReady { match_key: load_key.match_key, ready };
                    applied.effects.insert(0, publish);
                }
                self.run_effects(applied.effects)?;
            }
            Order::MatchEnded { match_key } if self.is_host => {
                let mut applied = self.host_event(HostMatchEvent::MatchEnded { match_key });
                if applied.accepted {
                    applied.effects.push(HostMatchEffect::PublishEnd { match_key });
                }
                self.run_effects(applied.effects)?;
            }
            Order::AuthorityProgress if self.is_host => {
                self.host_step(HostMatchEvent::AuthorityProgress { now_ms: wall_ms() })?;
            }
            Order::AdmitEnter { member_id, epoch, bootstrap_id, connection_id, client_id } if self.is_host => {
                let match_key = MatchKey::new(self.host.match_key().session_id, epoch);
                self.host_step(HostMatchEvent::Applied {
                    match_key,
                    member: member_id,
                    bootstrap_id,
                    connection_id,
                })?;
                self.request(RequestBody::EnterMatch {
                    member_id,
                    epoch,
                    bootstrap_id,
                    connection_id: connection_id.unwrap_or(0),
                    client_id,
                })?;
            }
            Order::MatchEnded { .. } | Order::AuthorityProgress | Order::AdmitEnter { .. } => {}
            Order::Shutdown => {
                let goodbye = if self.is_host { RequestBody::CloseRoom } else { RequestBody::LeaveRoom };
                return self.request(goodbye).map(Some);
            }
        }
        Ok(None)
    }

    /// Acts on one control frame (relay frames go to `relay_in` instead).
    pub fn frame(&mut self, frame: ControlFrame) -> Result<(), TransportFault> {
        match frame {
            ControlFrame::Relay(_) => Err(self.fault("control_relay", "relay must be consumed before lifecycle")),
            ControlFrame::Hello(_) => Ok(()),
            ControlFrame::Request(_) => Err(self.fault("control_read", "service sent a request on the client stream")),
            ControlFrame::Response(answer) => match answer.body {
                ResponseBody::RoomCreated { member_id, view } | ResponseBody::RoomJoined { member_id, view } => {
                    self.room_view(view, member_id)
                }
                ResponseBody::RoomUpdated { view } => self.room_view(view, self.identity.member_id),
                ResponseBody::Error(refused) => Err(self.fault("control_rpc", refused)),
                ResponseBody::RoomLeft | ResponseBody::Ack | ResponseBody::Status(_) | ResponseBody::RoomList { .. } => {
                    Ok(())
                }
            },
            ControlFrame::RoomView(view) => self.room_view(view, self.identity.member_id),
            ControlFrame::Closed { room_id, epoch, reason } => {
                if room_id != self.identity.room_id || epoch != self.identity.epoch {
                    return Ok(());
                }
                self.session.apply(SessionEvent::CloseSession);
                self.shared.note(MasterLifecycleFact::SessionClosed { reason });
                self.shared.room_closed(self.identity, reason);
                Ok(())
            }
            ControlFrame::PeerEvent(event) => self.peer_event(event),
        }
    }

    /// Takes in the room as the service now sees it, seated as `local`.
    /// A view the session refuses changes nothing at all.
    fn room_view(&mut self, view: RoomView, local: MemberId) -> Result<(), TransportFault> {
        let present_voters = self.skip_voters.iter().filter(|voter| view.contains(**voter)).count();
        let applied = self.session.apply(SessionEvent::AuthoritativeState {
            session_id: view.room_id.0,
            member: local,
            revision: view.revision,
            match_epoch: view.epoch,
            in_match: view.phase.in_match(),
            start_nonce: view.epoch,
            map: view.map.clone(),
            mode: view.mode.clone(),
            members: view.members.clone(),
            skip_votes: count(present_voters),
        });
        if !applied.changed {
            return Ok(());
        }
        diag::info!(Net, "session-transition {}", applied.transition);
        self.identity.member_id = local;
        self.identity.room_id = view.room_id;
        self.identity.epoch = view.epoch;

        let departed: Vec<MemberId> = match &self.last_view {
            Some(before) => before.members.iter().copied().filter(|member| !view.contains(*member)).collect(),
            None => Vec::new(),
        };
        for member_id in departed {
            self.skip_voters.remove(&member_id);
            map_ready(&self.shared.lane).remove(&member_id);
            self.shared.note(MasterLifecycleFact::MemberLeft { member_id });
        }

        let in_match = view.phase.in_match();
        if self.is_host && in_match && view.epoch != 0 && self.started_epoch != view.epoch {
            let match_key = self.identity.match_key();
            self.host_step(HostMatchEvent::Start { match_key, now_ms: wall_ms() })?;
            self.started_epoch = view.epoch;
        }
        if self.is_host && !in_match && self.started_epoch != 0 {
            let match_key = MatchKey::new(view.room_id.0, self.started_epoch);
            self.host_step(HostMatchEvent::MatchEnded { match_key })?;
        }

        let live = self.session.view();
        self.shared.lane.set_epoch(if live.in_match { live.start_nonce } else { 0 });
        self.shared.show_room(self.identity, &view, count(self.skip_voters.len()), self.is_host);
        self.last_view = Some(view);
        Ok(())
    }

    fn peer_event(&mut self, event: PeerEvent) -> Result<(), TransportFault> {
        match event {
            PeerEvent::HostWorldReady { epoch, map, weapons, classes } if self.is_host => {
                let ready = HostWorldReady { epoch, map, weapons, classes };
                self.host_step(HostMatchEvent::AuthorityReady { ready, now_ms: wall_ms() })
            }
            PeerEvent::MapLoaded { member_id, epoch, map, weapons, classes } if self.is_host => {
                let loaded = HostWorldReady { epoch, map, weapons, classes };
                let applied = self.host_event(HostMatchEvent::MapLoaded { member: member_id, loaded, now_ms: wall_ms() });
                // A loaded peer that draws no effect sits in the loading
                // screen all match; say which gate owes it an offer.
                if applied.effects.is_empty() {
                    diag::warn!(
                        Net,
                        "peer map loaded admits nothing: epoch={epoch} host_phase={:?} host_match={:?} host_world={:?} peer={:?} — no bootstrap offer will follow",
                        self.host.phase(),
                        self.host.match_key(),
                        self.host.host_world(),
                        self.host.peer(member_id).map(|peer| peer.phase),
                    );
                }
                self.run_effects(applied.effects)
            }
            PeerEvent::BootstrapApplied { member_id, epoch, bootstrap_id, snapshot_seq, connection } if self.is_host => {
                let ack = BootstrapAck {
                    member_id,
                    epoch,
                    bootstrap_id,
                    snapshot_seq,
                    connection: crate::transport::protocol::ConnectionId(connection),
                };
                self.shared.lane.push_ack(ack).map_err(|refusal| self.fault("bootstrap_ack", refusal))
            }
            PeerEvent::AdmissionFailed { member_id, epoch, reason } if !self.is_host => {
                let ours = member_id == self.identity.member_id && epoch_applies(epoch, self.identity.epoch);
                if ours { Err(self.fault("admission", reason)) } else { Ok(()) }
            }
            PeerEvent::EnterMatch { member_id, epoch, bootstrap_id, client_id, .. } => {
                if member_id == self.identity.member_id {
                    self.shared.lane.note_entered(bootstrap_id, client_id);
                    let applied = self.session.apply(SessionEvent::EnterMatch { epoch, bootstrap_id });
                    if applied.changed {
                        diag::info!(Net, "session-transition {}", applied.transition);
                    }
                }
                Ok(())
            }
            PeerEvent::VoteToSkip { member_id } => {
                self.skip_voters.insert(member_id);
                if let Some(view) = &self.last_view {
                    self.shared.show_room(self.identity, view, count(self.skip_voters.len()), self.is_host);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Moves what the game's bootstrap side queued for the link: the host's
    /// offers become stream jobs, a joiner's "applied" note becomes a request.
    pub fn pump(&mut self) -> Result<(), TransportFault> {
        let match_key = self.identity.match_key();
        for (peer, bytes) in self.shared.lane.take_to_worker() {
            if self.is_host {
                let Some(member) = peer else {
                    continue;
                };
                if !map_ready(&self.shared.lane).contains(&member) {
                    // Capture already refuses peers that have not reported
                    // their map, so readiness was dropped in between.
                    diag::warn!(Net, "bootstrap offer dropped for a peer that is not map-ready ({} bytes)", bytes.len());
                    continue;
                }
                let for_this_match = matches!(
                    decode_bootstrap(&bytes),
                    Ok(Some(BootstrapMessage::Offer { epoch, .. }))
                        if !match_key.is_none() && epoch == match_key.match_epoch
                );
                if for_this_match {
                    post(&self.jobs, BootstrapJob { match_key, member, bytes }, "bootstrap_job", self.role)?;
                }
                continue;
            }
            match decode_bootstrap(&bytes) {
                Ok(Some(BootstrapMessage::Applied { epoch, bootstrap_id, snapshot_seq, connection, .. })) => {
                    self.request(RequestBody::BootstrapApplied {
                        epoch,
                        bootstrap_id,
                        snapshot_seq,
                        connection: connection.0,
                    })?;
                }
                Ok(_) => {}
                Err(error) => diag::warn!(Net, "master join ignored bootstrap to-worker: {error}"),
            }
        }
        Ok(())
    }

    /// A bootstrap stream finished sending.
    pub fn prepared(&mut self, sent: HostMatchEvent) -> Result<(), TransportFault> {
        self.host_step(sent)
    }

    /// The once-a-second clock: lets the host match time out slow joiners.
    pub fn tick(&mut self) -> Result<(), TransportFault> {
        if !self.is_host {
            return Ok(());
        }
        self.host_step(HostMatchEvent::Tick { now_ms: wall_ms() })
    }

    /// Game control bytes out over the control stream.
    pub fn relay_out(&self) -> Result<(), TransportFault> {
        for (member_id, payload) in self.shared.mailbox.take_control_outbound() {
            let routed = if self.is_host {
                RelayDatagram::HostToMember { member_id, payload: &payload }
            } else {
                RelayDatagram::ClientToHost(&payload)
            };
            let wire = encode_relay_stream(routed).map_err(|error| self.fault("control_encode", error))?;
            post(&self.control, ControlFrame::Relay(wire), "control_write", self.role)?;
        }
        Ok(())
    }

    /// Game control bytes in, from a relay frame on the control stream.
    pub fn relay_in(&self, bytes: &[u8]) -> Result<(), TransportFault> {
        let routed = decode_relay_stream(bytes).ok().and_then(|datagram| addressed_to_us(datagram, self.is_host).ok());
        let Some((sender, payload)) = routed else {
            return Err(self.fault("control_relay", "wrong relay direction"));
        };
        self.shared
            .mailbox
            .push_control_inbound(sender, payload.to_vec())
            .map_err(|refusal| self.fault("control_relay", refusal))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use master_protocol::{ControlResponse, RoomPhase};
    use tokio::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::transport::bootstrap::BootstrapLane;
    use crate::transport::relay::RelayMailbox;
    use super::super::state::MasterBridgeState;

    const ROOM: AdvertId = AdvertId([7; 16]);
    const HOST: MemberId = MemberId([1; 16]);
    const GUEST: MemberId = MemberId([2; 16]);

    struct Seat {
        room: Room,
        shared: Shared,
        control: Receiver<ControlFrame>,
        _jobs: Receiver<BootstrapJob>,
    }

    fn seat(is_host: bool) -> Seat {
        let identity = SessionIdentity::unassigned(4);
        let start = MasterBridgeState::Connecting { identity };
        let shared = Shared::new(start, RelayMailbox::new(4), BootstrapLane::new());
        let (control_tx, control) = channel(16);
        let (jobs_tx, jobs) = channel(4);
        let role = if is_host { "host" } else { "join" };
        let room = Room::new(shared.clone(), role, is_host, identity, control_tx, jobs_tx);
        Seat { room, shared, control, _jobs: jobs }
    }

    fn view(revision: u64, epoch: u32, phase: RoomPhase, members: &[MemberId]) -> RoomView {
        RoomView {
            room_id: ROOM,
            name: "room".into(),
            host: HOST,
            members: members.to_vec(),
            member_names: HashMap::new(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            joinable: true,
            revision,
            epoch,
            phase,
            max_players: 6,
            requires: Default::default(),
        }
    }

    fn created(member_id: MemberId, view: RoomView) -> ControlFrame {
        let body = ResponseBody::RoomCreated { member_id, view };
        ControlFrame::Response(ControlResponse { request_id: 1, body })
    }

    fn sent(control: &mut Receiver<ControlFrame>) -> Vec<(u64, RequestBody)> {
        let mut out = Vec::new();
        while let Ok(frame) = control.try_recv() {
            if let ControlFrame::Request(request) = frame {
                out.push((request.request_id, request.body));
            }
        }
        out
    }

    #[test]
    fn request_ids_count_up_and_a_shutdown_says_goodbye() {
        let mut host = seat(true);
        host.room.order(Order::VoteToSkip).unwrap();
        let close = host.room.order(Order::Shutdown).unwrap();
        assert_eq!(close, Some(2));
        let asked = sent(&mut host.control);
        assert_eq!(asked, vec![(1, RequestBody::VoteToSkip), (2, RequestBody::CloseRoom)]);

        let mut guest = seat(false);
        assert_eq!(guest.room.order(Order::Shutdown).unwrap(), Some(1));
        assert_eq!(sent(&mut guest.control), vec![(1, RequestBody::LeaveRoom)]);
    }

    #[test]
    fn a_member_who_leaves_loses_its_vote_and_readiness() {
        let mut host = seat(true);
        host.room.frame(created(HOST, view(1, 0, RoomPhase::Gathering, &[HOST]))).unwrap();
        assert!(matches!(host.shared.snapshot(), MasterBridgeState::Hosting { .. }));
        assert_eq!(host.room.identity.member_id, HOST);
        host.room.frame(ControlFrame::RoomView(view(2, 0, RoomPhase::Gathering, &[HOST, GUEST]))).unwrap();
        host.room.frame(ControlFrame::PeerEvent(PeerEvent::VoteToSkip { member_id: GUEST })).unwrap();
        map_ready(&host.shared.lane).insert(GUEST);
        assert_eq!(host.shared.snapshot().skip_votes(), 1);

        host.room.frame(ControlFrame::RoomView(view(3, 0, RoomPhase::Gathering, &[HOST]))).unwrap();
        assert_eq!(host.shared.snapshot().skip_votes(), 0);
        assert!(!map_ready(&host.shared.lane).contains(&GUEST));
        assert_eq!(host.shared.drain_facts(), vec![MasterLifecycleFact::MemberLeft { member_id: GUEST }]);
    }

    #[test]
    fn an_old_view_changes_nothing() {
        let mut host = seat(true);
        host.room.frame(created(HOST, view(5, 0, RoomPhase::Gathering, &[HOST]))).unwrap();
        host.room.frame(ControlFrame::RoomView(view(5, 0, RoomPhase::Gathering, &[HOST, GUEST]))).unwrap();
        assert_eq!(host.shared.snapshot().members(), &[HOST]);
    }

    #[test]
    fn a_hosted_match_starts_with_its_room_and_ends_once() {
        let mut host = seat(true);
        host.room.frame(created(HOST, view(1, 0, RoomPhase::Gathering, &[HOST]))).unwrap();
        host.room.frame(ControlFrame::RoomView(view(2, 1, RoomPhase::Loading, &[HOST]))).unwrap();
        let match_key = MatchKey::new(ROOM.0, 1);
        assert_eq!(host.room.host.match_key(), match_key);
        assert!(host.shared.snapshot().in_match());

        host.room.order(Order::MatchEnded { match_key }).unwrap();
        let asked = sent(&mut host.control);
        assert_eq!(asked, vec![(1, RequestBody::EndMatch { room_id: ROOM, epoch: 1 })]);
        host.room.order(Order::MatchEnded { match_key }).unwrap();
        assert!(sent(&mut host.control).is_empty());
    }

    #[test]
    fn a_closed_room_ends_the_link_only_if_it_is_ours() {
        let mut guest = seat(false);
        let body = ResponseBody::RoomJoined { member_id: GUEST, view: view(1, 0, RoomPhase::Gathering, &[HOST, GUEST]) };
        guest.room.frame(ControlFrame::Response(ControlResponse { request_id: 1, body })).unwrap();
        let reason = master_protocol::SessionCloseReason::HostLeft;
        let elsewhere = ControlFrame::Closed { room_id: AdvertId([9; 16]), epoch: 0, reason };
        guest.room.frame(elsewhere).unwrap();
        assert!(!guest.shared.snapshot().is_terminal());
        guest.room.frame(ControlFrame::Closed { room_id: ROOM, epoch: 0, reason }).unwrap();
        assert!(matches!(guest.shared.snapshot(), MasterBridgeState::Closed { .. }));
        assert_eq!(guest.shared.drain_facts(), vec![MasterLifecycleFact::SessionClosed { reason }]);
    }

    #[test]
    fn a_refused_entry_fails_only_the_member_it_names() {
        let mut guest = seat(false);
        guest.room.identity = SessionIdentity { member_id: GUEST, room_id: ROOM, epoch: 3, attempt_id: 4 };
        let reason = master_protocol::AdmissionFailure::Deadline;
        let other = PeerEvent::AdmissionFailed { member_id: HOST, epoch: 3, reason };
        assert!(guest.room.frame(ControlFrame::PeerEvent(other)).is_ok());
        let stale = PeerEvent::AdmissionFailed { member_id: GUEST, epoch: 2, reason };
        assert!(guest.room.frame(ControlFrame::PeerEvent(stale)).is_ok());
        let ours = PeerEvent::AdmissionFailed { member_id: GUEST, epoch: 3, reason };
        let fault = guest.room.frame(ControlFrame::PeerEvent(ours)).unwrap_err();
        assert_eq!(fault.operation, "admission");
        assert_eq!(fault.source, "peer admission deadline");
    }

    #[test]
    fn relayed_control_bytes_go_the_right_way() {
        let host = seat(true);
        let inbound = encode_relay_stream(RelayDatagram::ServiceToHost { member_id: GUEST, payload: b"hi" }).unwrap();
        host.room.relay_in(&inbound).unwrap();
        assert_eq!(host.shared.mailbox.take_control_inbound(), vec![(GUEST, b"hi".to_vec())]);

        let guest = seat(false);
        let fault = guest.room.relay_in(&inbound).unwrap_err();
        assert_eq!((fault.operation, fault.source.as_str()), ("control_relay", "wrong relay direction"));
        assert!(guest.room.relay_in(b"junk").is_err());
    }

    #[test]
    fn transition_lines_name_their_event() {
        assert_eq!(event_word("stage_before=a event=tick stage_after=b"), "tick");
        assert_eq!(event_word("event=authority progress"), "");
        assert_eq!(event_word("x event=authority progress stage_after=y"), "authority progress");
        assert_eq!(event_word("nothing here"), "");
    }
}
