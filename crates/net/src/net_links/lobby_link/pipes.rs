//! The background tasks that move bytes over the lobby connection: control
//! frames out, gameplay datagrams both ways between the connection and the
//! relay mailbox, and the large bootstrap snapshots that travel as one-way
//! streams.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use master_protocol::{
    ControlEnvelope, TRANSFER_STREAM_CAP, RoomOccupantId, ForwardedPacket, read_relay,
    read_relay_stream, write_relay, write_relay_stream,
};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

use super::fault::{LinkFailureRecord, post, refused_text};
use super::quic::{PIPE_WAIT_LIMIT, within_deadline, write_control};
use crate::match_flow::HostRoundOccurrence;
use crate::net_links::match_handoff::{TransferChannelState, InitialStateTransferMsg, parse_setup_message};
use crate::net_links::datagram_split::{DatagramSplitter, FragmentStitcher};
use crate::net_links::relay::ForwardingInbox;

/// One relayed game packet is at most the game's own packet size.
pub(super) const RELAY_DATAGRAM_LIMIT: usize = crate::net_links::datagrams::DATAGRAM_BYTE_CAP as usize;
/// QUIC stream priority for bootstrap snapshots: below the default, so gameplay wins.
const SNAPSHOT_STREAM_PRIORITY: i32 = -32;
/// The largest payload one opaque relay message may carry.
const OPAQUE_LIMIT: usize = master_protocol::OPAQUE_BODY_BYTE_CAP;
/// How often queued outbound packets are flushed.
pub(super) const FLUSH_EVERY: Duration = Duration::from_millis(5);
/// The sender of every packet a joiner receives: the host, which has no member id.
const FROM_HOST: RoomOccupantId = RoomOccupantId([0; 16]);

/// A bootstrap snapshot the host owes one member for one match.
pub(super) struct BootstrapJob {
    pub game_session_key: ::frame::GameSessionStamp,
    pub member: RoomOccupantId,
    pub bytes: Vec<u8>,
}

/// What every task needs to know about the connection it serves.
#[derive(Clone)]
pub(super) struct Link {
    pub quic: quinn::Connection,
    pub cancel: CancellationToken,
    pub side: &'static str,
    pub hosting: bool,
}

impl Link {
    fn fault(&self, step: &'static str, cause: impl Display) -> LinkFailureRecord {
        LinkFailureRecord::on_link(step, self.side, cause, &self.quic)
    }
}

fn splitter() -> DatagramSplitter {
    DatagramSplitter::new(OPAQUE_LIMIT, RELAY_DATAGRAM_LIMIT)
}

fn joiner() -> FragmentStitcher {
    FragmentStitcher::new(OPAQUE_LIMIT, RELAY_DATAGRAM_LIMIT)
}

/// Logs a datagram or stream this side drops; `what` completes "ignored ...".
fn note_ignored(side: &str, what: impl Display) {
    diag::warn!(WireLink, "master {side} ignored {what}");
}

/// Logs a failure of one relay step as "master <side> <step>: <cause>".
fn note_failed(side: &str, step: &str, cause: impl Display) {
    diag::warn!(WireLink, "master {side} {step}: {cause}");
}

/// The sender and bytes of a datagram this side should receive, or the
/// datagram back when it travels the other way.
pub(super) fn addressed_to_us(
    datagram: ForwardedPacket<'_>,
    hosting: bool,
) -> Result<(RoomOccupantId, &[u8]), ForwardedPacket<'_>> {
    match datagram {
        ForwardedPacket::DirectoryToHost { member_id: from, payload: body } if hosting => Ok((from, body)),
        ForwardedPacket::DirectoryToParticipant(body) if !hosting => Ok((FROM_HOST, body)),
        elsewhere => Err(elsewhere),
    }
}

/// Control frames out, in the order the worker queued them.
pub(super) async fn send_control(
    link: Link,
    mut stream: quinn::SendStream,
    mut frames: Receiver<ControlEnvelope>,
) -> Result<(), LinkFailureRecord> {
    loop {
        let next = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            next = frames.recv() => next,
        };
        let Some(frame) = next else {
            return Ok(());
        };
        write_control(&mut stream, &frame)
            .await
            .map_err(|error| link.fault("control_write", error))?;
    }
}

/// Gameplay datagrams in: rebuild each sender's packets and post them.
pub(super) async fn receive_gameplay(
    link: Link,
    mailbox: ForwardingInbox,
) -> Result<(), LinkFailureRecord> {
    let side = link.side;
    let mut partial: HashMap<RoomOccupantId, FragmentStitcher> = HashMap::new();
    loop {
        let raw = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            arrived = link.quic.read_datagram() => arrived,
        }
        .map_err(|error| link.fault("read_datagram", error))?;
        let datagram = match read_relay(&raw) {
            Err(error) => {
                note_ignored(side, format_args!("relay datagram: {error}"));
                continue;
            }
            Ok(datagram) => datagram,
        };
        let (sender, piece) = match addressed_to_us(datagram, link.hosting) {
            Err(other) => {
                note_ignored(side, format_args!("unexpected datagram {other:?}"));
                continue;
            }
            Ok(routed) => routed,
        };
        let rebuilt = match partial.entry(sender).or_insert_with(joiner).ingest(piece) {
            Err(error) => {
                note_failed(side, "fragment refused", error);
                continue;
            }
            Ok(rebuilt) => rebuilt,
        };
        let Some(packet) = rebuilt else { continue };
        if let Err(error) = mailbox.take_arrival(sender, packet) {
            note_failed(side, "inbound mailbox", error);
        }
    }
}

/// Gameplay datagrams out: split each queued packet to fit and send the pieces.
pub(super) async fn send_gameplay(link: Link, mailbox: ForwardingInbox) -> Result<(), LinkFailureRecord> {
    let side = link.side;
    let mut pieces = splitter();
    let mut flush = tokio::time::interval(FLUSH_EVERY);
    loop {
        tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            _ = flush.tick() => {}
        }
        for (dest, outgoing) in mailbox.take_outbound().into_iter() {
            let Ok(fragments) = pieces
                .split_up(&outgoing)
                .inspect_err(|error| note_failed(side, "packet not splittable", error))
            else {
                continue;
            };
            for piece in fragments.iter() {
                let datagram = match link.hosting {
                    true => ForwardedPacket::HostToParticipant {
                        member_id: dest,
                        payload: piece,
                    },
                    false => ForwardedPacket::ReceiverToHost(piece),
                };
                let wire =
                    write_relay(datagram).map_err(|error| link.fault("encode_relay", error))?;
                link.quic
                    .send_datagram(wire.into())
                    .map_err(|error| link.fault("send_datagram", error))?;
            }
        }
    }
}

/// Bootstrap streams in (joiner side): each finished stream goes to the lane.
pub(super) async fn receive_bootstraps(
    link: Link,
    lane: Arc<TransferChannelState>,
) -> Result<(), LinkFailureRecord> {
    const LANE_FULL: &str = "bootstrap from-worker queue is full";
    let side = link.side;
    loop {
        let mut stream = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            opened = link.quic.accept_uni() => opened,
        }
        .map_err(|error| link.fault("accept_uni", error))?;
        let read = stream.read_to_end(TRANSFER_STREAM_CAP);
        let bytes = tokio::time::timeout(PIPE_WAIT_LIMIT, read)
            .await
            .map_err(|_late| link.fault("bootstrap_uni_read", "deadline"))?
            .map_err(|error| link.fault("bootstrap_uni_read", error))?;
        let inbound = match read_relay_stream(&bytes) {
            Err(error) => {
                note_ignored(side, format_args!("unfinished bootstrap stream: {error}"));
                continue;
            }
            Ok(ForwardedPacket::DirectoryToParticipant(body)) => body,
            Ok(ForwardedPacket::HostToParticipant { payload: body, .. }) => body,
            Ok(unexpected) => {
                note_ignored(side, format_args!("unexpected uni {unexpected:?}"));
                continue;
            }
        };
        lane.enqueue_from_helper(None, inbound.to_vec())
            .map_err(|refusal| {
                let source = match refusal == LANE_FULL {
                    true => refused_text("bootstrap_from_worker", true),
                    false => refusal.to_owned(),
                };
                LinkFailureRecord::new("bootstrap_from_worker", side, source)
            })?;
    }
}

/// Bootstrap streams out (host side): sends each job, then tells the host
/// match which offer went to whom.
pub(super) async fn send_bootstraps(
    link: Link,
    mut jobs: Receiver<BootstrapJob>,
    prepared: Sender<HostRoundOccurrence>,
) -> Result<(), LinkFailureRecord> {
    loop {
        let job = tokio::select! {
            _ = link.cancel.cancelled() => None,
            next = jobs.recv() => next,
        };
        let Some(job) = job else { return Ok(()) };
        stream_bootstrap(&link, job.member, &job.bytes).await?;
        let Ok(Some(InitialStateTransferMsg::Offer {
            handoff_id: handoff_no,
            link: connection,
            ..
        })) = parse_setup_message(&job.bytes)
        else {
            continue;
        };
        let sent = HostRoundOccurrence::SetupReady {
            game_session_key: job.game_session_key,
            member: job.member,
            transfer_ticket: handoff_no,
            link_no: Some(connection.0),
        };
        post(&prepared, sent, "setup_ready", link.side)?;
    }
}

/// One bootstrap snapshot as one low-priority one-way stream.
async fn stream_bootstrap(
    link: &Link,
    target: RoomOccupantId,
    snapshot: &[u8],
) -> Result<(), LinkFailureRecord> {
    let envelope = write_relay_stream(ForwardedPacket::HostToParticipant { member_id: target, payload: snapshot })
        .map_err(|error| link.fault("encode_bootstrap", error))?;
    if envelope.len() > TRANSFER_STREAM_CAP {
        let size = envelope.len();
        let budget = TRANSFER_STREAM_CAP;
        let why = format!("relay bootstrap envelope {size} bytes exceeds stream budget {budget}");
        return Err(link.fault("encode_bootstrap", why));
    }
    let mut stream = within_deadline(&link.cancel, link.quic.open_uni())
        .await
        .map_err(|error| link.fault("open_bootstrap_uni", error))?;
    stream
        .set_priority(SNAPSHOT_STREAM_PRIORITY)
        .map_err(|error| link.fault("bootstrap_priority", error))?;
    within_deadline(&link.cancel, stream.write_all(&envelope))
        .await
        .map_err(|error| link.fault("bootstrap_uni_write", error))?;
    stream
        .finish()
        .map_err(|error| link.fault("bootstrap_uni_finish", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_takes_only_its_own_direction() {
        let guest = RoomOccupantId([5; 16]);
        let to_host = ForwardedPacket::DirectoryToHost {
            member_id: guest,
            payload: b"a",
        };
        assert_eq!(
            addressed_to_us(to_host.clone(), true),
            Ok((guest, &b"a"[..]))
        );
        assert!(addressed_to_us(to_host, false).is_err());
        let to_member = ForwardedPacket::DirectoryToParticipant(b"b");
        assert_eq!(
            addressed_to_us(to_member.clone(), false),
            Ok((FROM_HOST, &b"b"[..]))
        );
        assert!(addressed_to_us(to_member, true).is_err());
        assert!(addressed_to_us(ForwardedPacket::ReceiverToHost(b"c"), true).is_err());
    }

    #[test]
    fn a_packet_split_for_the_relay_comes_back_whole() {
        let packet: Vec<u8> = (0..RELAY_DATAGRAM_LIMIT).map(|at| at as u8).collect();
        let mut rebuilt = None;
        let mut join = joiner();
        for piece in splitter().split_up(&packet).unwrap() {
            assert!(piece.len() <= master_protocol::OPAQUE_BODY_BYTE_CAP);
            if let Some(done) = join.ingest(&piece).unwrap() {
                rebuilt = Some(done);
            }
        }
        assert_eq!(rebuilt.as_deref(), Some(packet.as_slice()));
    }
}
