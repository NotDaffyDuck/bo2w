//! One seat in one room, as the link worker sees it.
//!
//! The worker owns a `Room` and feeds it whatever arrives: the game's
//! orders, the service's frames, finished bootstrap sends and the clock.
//! The room keeps the session and (when hosting) the match admission state,
//! and answers by queueing control frames and bootstrap jobs.

use std::collections::HashSet;

use master_protocol::{
    ListingId, ControlEnvelope, ControlQuery, RoomOccupantId, RoomMemberNotice, ForwardedPacket, QueryPayload,
    ReplyPayload, LobbyOverview, read_relay_stream, write_relay_stream,
};
use tokio::sync::mpsc::Sender;

use super::bridge::{Order, Shared, map_ready};
use super::fault::{LinkFailureRecord, post};
use super::pipes::{BootstrapJob, addressed_to_us};
use super::quic::wall_ms;
use super::room_view::{DirectoryLifecycleNote, LobbySessionLabel};
use crate::match_flow::{HostRoundInstall, HostRoundCore, HostRoundConsequence, HostRoundOccurrence};
use crate::match_flow::{HostMapLoaded, SessionStateCore, GameSessionNotice};
use crate::net_links::match_handoff::{
    TransferReceiptMsg, InitialStateTransferMsg, parse_setup_message, same_generation,
};
use ::frame::GameSessionStamp;

/// The event word inside a host transition line (`... event=<word> stage_after=...`).
fn event_word(transition: &str) -> &str {
    let Some(after) = transition.split(" event=").nth(1) else {
        return "";
    };
    after.split(" stage_after=").next().unwrap_or("")
}

/// Heartbeats are logged only when they cause something.
fn worth_logging(applied: &HostRoundInstall) -> bool {
    let quiet = matches!(
        event_word(&applied.change_note),
        "tick" | "authority progress"
    );
    !quiet || !applied.effects.is_empty()
}

const WRITE_STEP: &str = "control_write";
const STILL_JOINABLE: bool = true;
const CLIENT_STREAM_REQUEST: &str = "service sent a request on the client stream";

/// A loaded-world report from its parts.
fn level_ready(epoch: u32, (map, weapons, classes): (u64, u64, u64)) -> HostMapLoaded {
    HostMapLoaded {
        round_era: epoch,
        map,
        weapon_digest: weapons,
        class_digest: classes,
    }
}

fn log_transition(transition: &str) {
    diag::info!(WireLink, "session-transition {transition}");
}

/// The room id and match epoch a match key stands for on the wire.
fn room_and_epoch(key: GameSessionStamp) -> (ListingId, u32) {
    (ListingId(key.stamp_bytes), key.game_generation)
}

fn count(voters: usize) -> u8 {
    u8::try_from(voters).unwrap_or(0)
}

pub(super) struct Room {
    shared: Shared,
    side: &'static str,
    hosting: bool,
    pub seat_label: LobbySessionLabel,
    session: SessionStateCore,
    /// Admission of joiners into the running match; used only when hosting.
    host: HostRoundCore,
    skip_voters: HashSet<RoomOccupantId>,
    last_view: Option<LobbyOverview>,
    /// The match this host last started; 0 before the first.
    started_epoch: u32,
    next_request: u64,
    control: Sender<ControlEnvelope>,
    jobs: Sender<BootstrapJob>,
}

impl Room {
    pub fn new(
        shared: Shared,
        side: &'static str,
        hosting: bool,
        seat_label: LobbySessionLabel,
        control: Sender<ControlEnvelope>,
        jobs: Sender<BootstrapJob>,
    ) -> Self {
        Self {
            session: SessionStateCore::for_connection(seat_label.lobby_number.0, seat_label.join_try_number),
            host: HostRoundCore::default(),
            skip_voters: HashSet::new(),
            last_view: None,
            started_epoch: 0,
            next_request: 1,
            shared,
            side,
            hosting,
            seat_label,
            control,
            jobs,
        }
    }

    fn fault(&self, step: &'static str, cause: impl ToString) -> LinkFailureRecord {
        LinkFailureRecord::new(step, self.side, cause.to_string())
    }

    /// Queues a request to the service and returns the id its answer will carry.
    pub fn send_query(&mut self, payload: QueryPayload) -> Result<u64, LinkFailureRecord> {
        let issued = self.next_request;
        self.next_request = issued.saturating_add(1);
        let frame = ControlEnvelope::Request(ControlQuery {
            call_id: issued,
            body: payload,
        });
        post(&self.control, frame, WRITE_STEP, self.side).map(|()| issued)
    }

    fn host_event(&mut self, event: HostRoundOccurrence) -> HostRoundInstall {
        let applied = self.host.take_event(event);
        if worth_logging(&applied) {
            log_transition(&applied.change_note);
        }
        applied
    }

    /// Applies one host match event and carries out what it asks for.
    fn host_step(&mut self, event: HostRoundOccurrence) -> Result<(), LinkFailureRecord> {
        let applied = self.host_event(event);
        self.run_effects(applied.effects)
    }

    /// Carries out host match effects in order; a failure among them is
    /// reported only after the rest have run.
    fn run_effects(&mut self, effects: Vec<HostRoundConsequence>) -> Result<(), LinkFailureRecord> {
        let mut failure = None;
        for step in effects {
            if let HostRoundConsequence::Fail { fail } = step {
                failure = Some(fail);
                continue;
            }
            self.run_effect(step)?;
        }
        failure.map_or(Ok(()), |fail| Err(self.fault("host_match", fail)))
    }

    /// One host match effect other than a failure note.
    fn run_effect(&mut self, job: HostRoundConsequence) -> Result<(), LinkFailureRecord> {
        match job {
            HostRoundConsequence::Enter { .. } | HostRoundConsequence::Fail { .. } => Ok(()),
            HostRoundConsequence::FlushBootstraps { member } => {
                map_ready(&self.shared.lane).insert(member);
                Ok(())
            }
            HostRoundConsequence::ReadyToAnnounce { game_session_key, ready } => {
                let (lobby_number, epoch) = room_and_epoch(game_session_key);
                let HostMapLoaded {
                    map,
                    weapon_digest: weapons,
                    class_digest: classes,
                    ..
                } = ready;
                self.send_query(QueryPayload::HostMapLoaded {
                    lobby_number,
                    epoch,
                    map,
                    weapons,
                    classes,
                })
                .map(drop)
            }
            HostRoundConsequence::PublishEnd { game_session_key } => {
                map_ready(&self.shared.lane).clear();
                let (lobby_number, epoch) = room_and_epoch(game_session_key);
                self.send_query(QueryPayload::EndMatch { lobby_number, epoch })
                    .map(drop)
            }
            HostRoundConsequence::DropRemotePeer {
                member: peer,
                reason,
                game_session_key,
            } => {
                map_ready(&self.shared.lane).remove(&peer);
                self.shared.log_fact(DirectoryLifecycleNote::EntryRefused {
                    member_id: peer,
                    reason,
                });
                let body = QueryPayload::EntryRefused {
                    epoch: game_session_key.game_generation,
                    member_id: peer,
                    reason,
                };
                self.send_query(body).map(drop)
            }
        }
    }

    /// Acts on one order from the game. A shutdown returns the id of the
    /// close or leave request, whose answer ends the link.
    pub fn order(&mut self, order: Order) -> Result<Option<u64>, LinkFailureRecord> {
        match order {
            Order::RefreshRoom { map, ruleset } => {
                self.send_query(QueryPayload::ConfigureLobby {
                    map,
                    mode: ruleset,
                    open_to_join: STILL_JOINABLE,
                })?;
            }
            Order::BeginGame { map, ruleset } => {
                self.send_query(QueryPayload::BeginGame { map, mode: ruleset })?;
            }
            Order::BallotForSkip => {
                self.send_query(QueryPayload::BallotForSkip)?;
            }
            Order::Kick { member_id } => {
                self.send_query(QueryPayload::KickMember { member_id })?;
            }
            Order::MapLoaded {
                epoch,
                map,
                weapon_digest,
                class_digest,
            } => {
                self.send_query(QueryPayload::MapLoaded {
                    epoch,
                    map,
                    weapons: weapon_digest,
                    classes: class_digest,
                })?;
                if self.hosting {
                    let loaded = level_ready(epoch, (map, weapon_digest, class_digest));
                    let host_seat = RoomOccupantId([0; 16]);
                    self.host_step(HostRoundOccurrence::MapLoaded {
                        member: host_seat,
                        loaded,
                        clock_millis: wall_ms(),
                    })?;
                }
            }
            Order::HostMapLoaded {
                epoch,
                map,
                weapon_digest,
                class_digest,
                decrypt_secret,
            } => {
                let current = self.hosting
                    && self.shared.loaded_ticket() == Some(decrypt_secret)
                    && decrypt_secret.issued_for(self.host.game_session_key())
                    && epoch == decrypt_secret.game_session_key.game_generation;
                if !current {
                    return Ok(None);
                }
                self.host.remember_incarnation(decrypt_secret.incarnation);
                self.host
                    .remember_load_ticket(Some(decrypt_secret.own_level_ticket_no));
                let ready = level_ready(epoch, (map, weapon_digest, class_digest));
                let applied = self.host_event(HostRoundOccurrence::WorldUp {
                    ready,
                    clock_millis: wall_ms(),
                });
                let publish = HostRoundConsequence::ReadyToAnnounce {
                    game_session_key: decrypt_secret.game_session_key,
                    ready,
                };
                let ordered = applied
                    .was_taken
                    .then_some(publish)
                    .into_iter()
                    .chain(applied.effects);
                self.run_effects(ordered.collect())?;
            }
            Order::RoundOver { game_session_key } if self.hosting => {
                let applied = self.host_event(HostRoundOccurrence::MatchEnded { game_session_key });
                let closing = applied
                    .was_taken
                    .then_some(HostRoundConsequence::PublishEnd { game_session_key });
                self.run_effects(applied.effects.into_iter().chain(closing).collect())?;
            }
            Order::HostAdvance if self.hosting => {
                self.host_step(HostRoundOccurrence::HostAdvance { clock_millis: wall_ms() })?;
            }
            Order::LetIntoMatch {
                member_id: who,
                epoch,
                handoff_no,
                wire_conn,
                seat_no,
            } if self.hosting => {
                let game_session_key = GameSessionStamp::new(self.host.game_session_key().stamp_bytes, epoch);
                let applied = HostRoundOccurrence::TakenIntoEffect {
                    game_session_key,
                    member: who,
                    transfer_ticket: handoff_no,
                    link_no: wire_conn,
                };
                self.host_step(applied)?;
                let entered = QueryPayload::JoinGame {
                    player_slot_no: seat_no,
                    connection_id: wire_conn.unwrap_or_default(),
                    bootstrap_id: handoff_no,
                    epoch,
                    member_id: who,
                };
                self.send_query(entered)?;
            }
            Order::RoundOver { .. } | Order::HostAdvance | Order::LetIntoMatch { .. } => {}
            Order::Shutdown => {
                let goodbye = if self.hosting {
                    QueryPayload::CloseRoom
                } else {
                    QueryPayload::LeaveRoom
                };
                return self.send_query(goodbye).map(Some);
            }
        }
        Ok(None)
    }

    /// Acts on one control frame (relay frames go to `relay_in` instead).
    pub fn frame(&mut self, incoming: ControlEnvelope) -> Result<(), LinkFailureRecord> {
        let local = self.seat_label.member_id;
        match incoming {
            ControlEnvelope::RoomMemberNotice(event) => self.peer_event(event),
            ControlEnvelope::LobbyOverview(view) => self.room_view(view, local),
            ControlEnvelope::Response(answer) => self.answered(answer.body),
            ControlEnvelope::Closed {
                lobby_number,
                epoch,
                reason,
            } => {
                let ours = (lobby_number, epoch) == (self.seat_label.lobby_number, self.seat_label.epoch);
                if ours {
                    self.session.take_event(GameSessionNotice::EndSession);
                    self.shared
                        .log_fact(DirectoryLifecycleNote::SessionEnded { reason });
                    self.shared.room_closed(self.seat_label, reason);
                }
                Ok(())
            }
            ControlEnvelope::Hello(..) => Ok(()),
            ControlEnvelope::Request(_) => Err(self.fault("control_read", CLIENT_STREAM_REQUEST)),
            ControlEnvelope::PassAlong(_) => {
                Err(self.fault("control_relay", "relay must be consumed before lifecycle"))
            }
        }
    }

    /// Acts on the body of one service answer.
    fn answered(&mut self, reply: ReplyPayload) -> Result<(), LinkFailureRecord> {
        match reply {
            ReplyPayload::Error(refused) => Err(self.fault("control_rpc", refused)),
            ReplyPayload::RoomUpdated { view } => self.room_view(view, self.seat_label.member_id),
            ReplyPayload::LobbyOpened { member_id: who, view } | ReplyPayload::LobbyEntered { member_id: who, view } => {
                self.room_view(view, who)
            }
            ReplyPayload::RoomLeft | ReplyPayload::Ack | ReplyPayload::Status(_) | ReplyPayload::RoomList { .. } => Ok(()),
        }
    }

    /// Takes in the room as the service now sees it, seated as `local`.
    /// A view the session refuses changes nothing at all.
    fn room_view(&mut self, view: LobbyOverview, local: RoomOccupantId) -> Result<(), LinkFailureRecord> {
        let present_voters = self
            .skip_voters
            .iter()
            .filter(|voter| view.contains(**voter))
            .count();
        let (epoch, in_match) = (view.epoch, view.phase.is_playing());
        let event = GameSessionNotice::HostDecidedState {
            members: view.members.to_vec(),
            game_mode: view.mode.to_owned(),
            map: view.map.to_owned(),
            launch_token: epoch,
            game_generation: epoch,
            pass_vote_count: count(present_voters),
            version_no: view.revision,
            session_no: view.lobby_number.0,
            member: local,
            in_play: in_match,
        };
        let applied = self.session.take_event(event);
        if !applied.did_change {
            return Ok(());
        }
        log_transition(&applied.change_note);
        self.seat_label.member_id = local;
        self.seat_label.lobby_number = view.lobby_number;
        self.seat_label.epoch = view.epoch;

        let departed: Vec<RoomOccupantId> = (self.last_view.iter())
            .flat_map(|before| before.members.iter().copied())
            .filter(|member| !view.contains(*member))
            .collect();
        for member_id in departed {
            self.skip_voters.remove(&member_id);
            map_ready(&self.shared.lane).remove(&member_id);
            self.shared
                .log_fact(DirectoryLifecycleNote::ParticipantDeparted { member_id });
        }

        if self.hosting && in_match && view.epoch != 0 && self.started_epoch != view.epoch {
            let game_session_key = self.seat_label.game_session_key();
            self.host_step(HostRoundOccurrence::Start {
                game_session_key,
                clock_millis: wall_ms(),
            })?;
            self.started_epoch = view.epoch;
        }
        if self.hosting && !in_match && self.started_epoch != 0 {
            let game_session_key = GameSessionStamp::new(view.lobby_number.0, self.started_epoch);
            self.host_step(HostRoundOccurrence::MatchEnded { game_session_key })?;
        }

        let live = self.session.view();
        self.shared
            .lane
            .assign_generation(if live.in_play { live.launch_token } else { 0 });
        self.shared.show_room(
            self.seat_label,
            &view,
            count(self.skip_voters.len()),
            self.hosting,
        );
        self.last_view = Some(view);
        Ok(())
    }

    fn peer_event(&mut self, event: RoomMemberNotice) -> Result<(), LinkFailureRecord> {
        match event {
            RoomMemberNotice::HostMapLoaded {
                epoch,
                map,
                weapon_digest: weapons,
                class_digest: classes,
            } if self.hosting => {
                let ready = level_ready(epoch, (map, weapons, classes));
                self.host_step(HostRoundOccurrence::WorldUp {
                    ready,
                    clock_millis: wall_ms(),
                })
            }
            RoomMemberNotice::MapLoaded {
                member_id: sender,
                epoch,
                map,
                weapon_digest: weapons,
                class_digest: classes,
            } if self.hosting => {
                let loaded = level_ready(epoch, (map, weapons, classes));
                let applied = self.host_event(HostRoundOccurrence::MapLoaded {
                    member: sender,
                    loaded,
                    clock_millis: wall_ms(),
                });
                // A loaded peer that draws no effect sits in the loading
                // screen all match; say which gate owes it an offer.
                if applied.effects.is_empty() {
                    diag::warn!(
                        WireLink,
                        "peer map loaded admits nothing: epoch={epoch} host_phase={:?} host_match={:?} host_world={:?} peer={:?} — no bootstrap offer will follow",
                        self.host.phase(),
                        self.host.game_session_key(),
                        self.host.host_world(),
                        self.host.gate_for(sender).map(|peer| peer.phase),
                    );
                }
                self.run_effects(applied.effects)
            }
            RoomMemberNotice::TransferInstalledMsg {
                member_id,
                epoch,
                transfer_id: bootstrap_id,
                frame_number_on_wire,
                connection,
            } if self.hosting => {
                let confirm_receipt = TransferReceiptMsg {
                    member_id,
                    era: epoch,
                    handoff_id: bootstrap_id,
                    frame_number_on_wire,
                    link: crate::net_links::datagrams::LinkNumber(connection),
                };
                self.shared
                    .lane
                    .queue_receipt(confirm_receipt)
                    .map_err(|refusal| self.fault("bootstrap_ack", refusal))
            }
            RoomMemberNotice::EntryRefused {
                member_id,
                epoch,
                reason,
            } if !self.hosting => {
                let ours = member_id == self.seat_label.member_id
                    && same_generation(epoch, self.seat_label.epoch);
                if ours {
                    Err(self.fault("admission", reason))
                } else {
                    Ok(())
                }
            }
            RoomMemberNotice::JoinGame {
                member_id,
                epoch,
                transfer_id: bootstrap_id,
                player_slot_no,
                ..
            } => {
                if member_id == self.seat_label.member_id {
                    self.shared.lane.record_entry(bootstrap_id, player_slot_no);
                    let applied = self.session.take_event(GameSessionNotice::JoinGame {
                        epoch,
                        bootstrap_id,
                    });
                    match applied.did_change {
                        true => log_transition(&applied.change_note),
                        false => {}
                    }
                }
                Ok(())
            }
            RoomMemberNotice::BallotForSkip { member_id: voter } => {
                self.skip_voters.insert(voter);
                if let Some(view) = &self.last_view {
                    self.shared.show_room(
                        self.seat_label,
                        view,
                        count(self.skip_voters.len()),
                        self.hosting,
                    );
                }
                Ok(())
            }
            _unhandled => Ok(()),
        }
    }

    /// Moves what the game's bootstrap side queued for the link: the host's
    /// offers become stream jobs, a joiner's "applied" note becomes a request.
    pub fn pump(&mut self) -> Result<(), LinkFailureRecord> {
        let game_session_key = self.seat_label.game_session_key();
        for (peer, bytes) in self.shared.lane.take_to_worker() {
            if self.hosting {
                let Some(member) = peer else { continue };
                if !map_ready(&self.shared.lane).contains(&member) {
                    // Capture already refuses peers that have not reported
                    // their map, so readiness was dropped in between.
                    diag::warn!(
                        WireLink,
                        "bootstrap offer dropped for a peer that is not map-ready ({} bytes)",
                        bytes.len()
                    );
                    continue;
                }
                let for_this_match = matches!(
                    parse_setup_message(&bytes),
                    Ok(Some(InitialStateTransferMsg::Offer { era, .. }))
                        if !game_session_key.is_none() && era == game_session_key.game_generation
                );
                if for_this_match {
                    post(
                        &self.jobs,
                        BootstrapJob {
                            game_session_key,
                            member,
                            bytes,
                        },
                        "bootstrap_job",
                        self.side,
                    )?;
                }
                continue;
            }
            let decoded = parse_setup_message(&bytes);
            match decoded {
                Ok(Some(InitialStateTransferMsg::TakenIntoEffect {
                    era,
                    handoff_id,
                    frame_number_on_wire,
                    link,
                    ..
                })) => {
                    self.send_query(QueryPayload::TransferInstalledMsg {
                        epoch: era,
                        bootstrap_id: handoff_id,
                        frame_number_on_wire,
                        connection: link.0,
                    })?;
                }
                Ok(_) => {}
                Err(error) => diag::warn!(WireLink, "master join: bootstrap job not queued: {error}"),
            }
        }
        Ok(())
    }

    /// A bootstrap stream finished sending.
    pub fn prepared(&mut self, sent: HostRoundOccurrence) -> Result<(), LinkFailureRecord> {
        self.host_step(sent)
    }

    /// The once-a-second clock: lets the host match time out slow joiners.
    pub fn tick(&mut self) -> Result<(), LinkFailureRecord> {
        if !self.hosting {
            return Ok(());
        }
        self.host_step(HostRoundOccurrence::Tick { clock_millis: wall_ms() })
    }

    /// Game control bytes out over the control stream.
    pub fn relay_out(&self) -> Result<(), LinkFailureRecord> {
        for (target, bytes_out) in self.shared.mailbox.take_control_outbound() {
            let routed = if self.hosting {
                ForwardedPacket::HostToParticipant {
                    member_id: target,
                    payload: &bytes_out,
                }
            } else {
                ForwardedPacket::ReceiverToHost(bytes_out.as_slice())
            };
            let wire =
                write_relay_stream(routed).map_err(|error| self.fault("control_encode", error))?;
            post(
                &self.control,
                ControlEnvelope::PassAlong(wire),
                WRITE_STEP,
                self.side,
            )?;
        }
        Ok(())
    }

    /// Game control bytes in, from a relay frame on the control stream.
    pub fn relay_in(&self, frame: &[u8]) -> Result<(), LinkFailureRecord> {
        let routed = read_relay_stream(frame)
            .ok()
            .and_then(|datagram| addressed_to_us(datagram, self.hosting).ok());
        let Some((sender, data)) = routed else {
            return Err(self.fault("control_relay", "relay frame points the wrong way"));
        };
        self.shared
            .mailbox
            .enqueue_control_incoming(sender, data.to_vec())
            .map_err(|refusal| self.fault("control_relay", refusal))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use master_protocol::{ControlReply, LobbyStage};
    use tokio::sync::mpsc::{Receiver, channel};

    use super::super::room_view::DirectoryLinkStatus;
    use super::*;
    use crate::net_links::match_handoff::TransferChannelState;
    use crate::net_links::relay::ForwardingInbox;

    const ROOM: ListingId = ListingId([7; 16]);
    const HOST: RoomOccupantId = RoomOccupantId([1; 16]);
    const GUEST: RoomOccupantId = RoomOccupantId([2; 16]);
    const OPEN: bool = true;
    const FIRST_ID: u64 = 1;

    struct ChairSlot {
        room: Room,
        shared: Shared,
        control: Receiver<ControlEnvelope>,
        _jobs: Receiver<BootstrapJob>,
    }

    fn seat(hosting: bool) -> ChairSlot {
        let identity = LobbySessionLabel::unassigned(4);
        let start = DirectoryLinkStatus::Connecting { identity };
        let shared = Shared::new(start, ForwardingInbox::new(4), TransferChannelState::new());
        let (ctl_tx, control) = channel(16);
        let (jobs_tx, jobs) = channel(4);
        let side = if hosting { "host" } else { "join" };
        let room = Room::new(shared.clone(), side, hosting, identity, ctl_tx, jobs_tx);
        ChairSlot {
            room,
            shared,
            control,
            _jobs: jobs,
        }
    }

    fn view(revision: u64, epoch: u32, phase: LobbyStage, members: &[RoomOccupantId]) -> LobbyOverview {
        LobbyOverview {
            lobby_number: ROOM,
            name: "room".into(),
            host: HOST,
            members: members.to_vec(),
            member_names: HashMap::new(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            open_to_join: OPEN,
            revision,
            epoch,
            phase,
            seat_capacity: 6,
            requires: Default::default(),
        }
    }

    fn created(who: RoomOccupantId, view: LobbyOverview) -> ControlEnvelope {
        let body = ReplyPayload::LobbyOpened { member_id: who, view };
        ControlEnvelope::Response(ControlReply {
            call_id: FIRST_ID,
            body,
        })
    }

    fn sent(control: &mut Receiver<ControlEnvelope>) -> Vec<(u64, QueryPayload)> {
        let mut out = Vec::new();
        while let Ok(frame) = control.try_recv() {
            if let ControlEnvelope::Request(request) = frame {
                out.push((request.call_id, request.body));
            }
        }
        out
    }

    #[test]
    fn request_ids_count_up_and_a_shutdown_says_goodbye() {
        let mut host = seat(true);
        host.room.order(Order::BallotForSkip).unwrap();
        let close = host.room.order(Order::Shutdown).unwrap();
        assert_eq!(close, Some(2));
        let asked = sent(&mut host.control);
        assert_eq!(
            asked,
            vec![(1, QueryPayload::BallotForSkip), (2, QueryPayload::CloseRoom)]
        );

        let mut guest = seat(false);
        assert_eq!(guest.room.order(Order::Shutdown).unwrap(), Some(1));
        assert_eq!(sent(&mut guest.control), vec![(1, QueryPayload::LeaveRoom)]);
    }

    #[test]
    fn a_member_who_leaves_loses_its_vote_and_readiness() {
        let mut host = seat(true);
        host.room
            .frame(created(HOST, view(1, 0, LobbyStage::Gathering, &[HOST])))
            .unwrap();
        assert!(matches!(
            host.shared.snapshot(),
            DirectoryLinkStatus::Hosting { .. }
        ));
        assert_eq!(host.room.seat_label.member_id, HOST);
        host.room
            .frame(ControlEnvelope::LobbyOverview(view(
                2,
                0,
                LobbyStage::Gathering,
                &[HOST, GUEST],
            )))
            .unwrap();
        host.room
            .frame(ControlEnvelope::RoomMemberNotice(RoomMemberNotice::BallotForSkip {
                member_id: GUEST,
            }))
            .unwrap();
        map_ready(&host.shared.lane).insert(GUEST);
        assert_eq!(host.shared.snapshot().pass_vote_count(), 1);

        host.room
            .frame(ControlEnvelope::LobbyOverview(view(
                3,
                0,
                LobbyStage::Gathering,
                &[HOST],
            )))
            .unwrap();
        assert_eq!(host.shared.snapshot().pass_vote_count(), 0);
        assert!(!map_ready(&host.shared.lane).contains(&GUEST));
        assert_eq!(
            host.shared.drain_facts(),
            vec![DirectoryLifecycleNote::ParticipantDeparted { member_id: GUEST }]
        );
    }

    #[test]
    fn an_old_view_changes_nothing() {
        let mut host = seat(true);
        host.room
            .frame(created(HOST, view(5, 0, LobbyStage::Gathering, &[HOST])))
            .unwrap();
        host.room
            .frame(ControlEnvelope::LobbyOverview(view(
                5,
                0,
                LobbyStage::Gathering,
                &[HOST, GUEST],
            )))
            .unwrap();
        assert_eq!(host.shared.snapshot().members(), &[HOST]);
    }

    #[test]
    fn a_hosted_match_starts_with_its_room_and_ends_once() {
        let mut host = seat(true);
        host.room
            .frame(created(HOST, view(1, 0, LobbyStage::Gathering, &[HOST])))
            .unwrap();
        host.room
            .frame(ControlEnvelope::LobbyOverview(view(
                2,
                1,
                LobbyStage::Loading,
                &[HOST],
            )))
            .unwrap();
        let game_session_key = GameSessionStamp::new(ROOM.0, 1);
        assert_eq!(host.room.host.game_session_key(), game_session_key);
        assert!(host.shared.snapshot().underway());

        host.room.order(Order::RoundOver { game_session_key }).unwrap();
        let asked = sent(&mut host.control);
        assert_eq!(
            asked,
            vec![(
                1,
                QueryPayload::EndMatch {
                    lobby_number: ROOM,
                    epoch: 1
                }
            )]
        );
        host.room.order(Order::RoundOver { game_session_key }).unwrap();
        assert!(sent(&mut host.control).is_empty());
    }

    #[test]
    fn a_closed_room_ends_the_link_only_if_it_is_ours() {
        let mut guest = seat(false);
        let body = ReplyPayload::LobbyEntered {
            member_id: GUEST,
            view: view(1, 0, LobbyStage::Gathering, &[HOST, GUEST]),
        };
        guest
            .room
            .frame(ControlEnvelope::Response(ControlReply {
                call_id: FIRST_ID,
                body,
            }))
            .unwrap();
        let reason = master_protocol::RoomShutdownCause::HostLeft;
        let elsewhere = ControlEnvelope::Closed {
            lobby_number: ListingId([9; 16]),
            epoch: 0,
            reason,
        };
        guest.room.frame(elsewhere).unwrap();
        assert!(!guest.shared.snapshot().is_terminal());
        guest
            .room
            .frame(ControlEnvelope::Closed {
                lobby_number: ROOM,
                epoch: 0,
                reason,
            })
            .unwrap();
        assert!(matches!(
            guest.shared.snapshot(),
            DirectoryLinkStatus::Closed { .. }
        ));
        assert_eq!(
            guest.shared.drain_facts(),
            vec![DirectoryLifecycleNote::SessionEnded { reason }]
        );
    }

    #[test]
    fn a_refused_entry_fails_only_the_member_it_names() {
        let mut guest = seat(false);
        guest.room.seat_label = LobbySessionLabel {
            member_id: GUEST,
            lobby_number: ROOM,
            epoch: 3,
            join_try_number: 4,
        };
        let reason = master_protocol::EntryRefusalReason::CutoffInstant;
        let other = RoomMemberNotice::EntryRefused {
            member_id: HOST,
            epoch: 3,
            reason,
        };
        assert!(guest.room.frame(ControlEnvelope::RoomMemberNotice(other)).is_ok());
        let stale = RoomMemberNotice::EntryRefused {
            member_id: GUEST,
            epoch: 2,
            reason,
        };
        assert!(guest.room.frame(ControlEnvelope::RoomMemberNotice(stale)).is_ok());
        let ours = RoomMemberNotice::EntryRefused {
            member_id: GUEST,
            epoch: 3,
            reason,
        };
        let fault = guest.room.frame(ControlEnvelope::RoomMemberNotice(ours)).unwrap_err();
        assert_eq!(fault.step_name, "admission");
        assert_eq!(fault.source, "peer admission deadline");
    }

    #[test]
    fn relayed_control_bytes_go_the_right_way() {
        let host = seat(true);
        let inbound = write_relay_stream(ForwardedPacket::DirectoryToHost {
            member_id: GUEST,
            payload: b"hi",
        })
        .unwrap();
        host.room.relay_in(&inbound).unwrap();
        assert_eq!(
            host.shared.mailbox.take_control_inbound(),
            vec![(GUEST, b"hi".to_vec())]
        );

        let guest = seat(false);
        let fault = guest.room.relay_in(&inbound).unwrap_err();
        assert_eq!(
            (fault.step_name, fault.source.as_str()),
            ("control_relay", "relay frame points the wrong way")
        );
        assert!(guest.room.relay_in(b"junk").is_err());
    }

    #[test]
    fn transition_lines_name_their_event() {
        assert_eq!(
            event_word("stage_before=a event=tick stage_after=b"),
            "tick"
        );
        assert_eq!(event_word("event=authority progress"), "");
        assert_eq!(
            event_word("x event=authority progress stage_after=y"),
            "authority progress"
        );
        assert_eq!(event_word("nothing here"), "");
    }
}
