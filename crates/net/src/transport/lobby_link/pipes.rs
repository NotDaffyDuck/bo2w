//! The background tasks that move bytes over the lobby connection: control
//! frames out, gameplay datagrams both ways between the connection and the
//! relay mailbox, and the large bootstrap snapshots that travel as one-way
//! streams.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use master_protocol::{
    ControlFrame, MAX_BOOTSTRAP_STREAM_BYTES, MemberId, RelayDatagram, decode_relay,
    decode_relay_stream, encode_relay, encode_relay_stream,
};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

use super::fault::{TransportFault, post, refused_text};
use super::quic::{IO_DEADLINE, within_deadline, write_control};
use crate::session_core::HostMatchEvent;
use crate::transport::bootstrap::{BootstrapLane, BootstrapMessage, decode_bootstrap};
use crate::transport::fragment::{Fragmenter, Reassembler};
use crate::transport::relay::RelayMailbox;

/// One relayed game packet is at most the game's own packet size.
pub(super) const RELAY_PACKET_BYTES: usize = crate::transport::protocol::MAX_PACKET_BYTES as usize;
/// Bootstrap streams yield to gameplay on the wire.
const BOOTSTRAP_PRIORITY: i32 = -32;
/// How often queued outbound packets are flushed.
pub(super) const FLUSH_EVERY: Duration = Duration::from_millis(5);
/// The sender of every packet a joiner receives: the host, which has no member id.
const FROM_HOST: MemberId = MemberId([0; 16]);

/// A bootstrap snapshot the host owes one member for one match.
pub(super) struct BootstrapJob {
    pub match_key: ::frame::MatchKey,
    pub member: MemberId,
    pub bytes: Vec<u8>,
}

/// What every task needs to know about the connection it serves.
#[derive(Clone)]
pub(super) struct Link {
    pub quic: quinn::Connection,
    pub cancel: CancellationToken,
    pub role: &'static str,
    pub is_host: bool,
}

impl Link {
    fn fault(&self, step: &'static str, cause: impl Display) -> TransportFault {
        TransportFault::on_link(step, self.role, cause, &self.quic)
    }
}

fn splitter() -> Fragmenter {
    Fragmenter::new(master_protocol::MAX_OPAQUE_PAYLOAD, RELAY_PACKET_BYTES)
}

fn joiner() -> Reassembler {
    Reassembler::new(master_protocol::MAX_OPAQUE_PAYLOAD, RELAY_PACKET_BYTES)
}

/// The sender and bytes of a datagram this side should receive, or the
/// datagram back when it travels the other way.
pub(super) fn addressed_to_us(datagram: RelayDatagram<'_>, is_host: bool) -> Result<(MemberId, &[u8]), RelayDatagram<'_>> {
    match datagram {
        RelayDatagram::ServiceToHost { member_id, payload } if is_host => Ok((member_id, payload)),
        RelayDatagram::ServiceToMember(payload) if !is_host => Ok((FROM_HOST, payload)),
        elsewhere => Err(elsewhere),
    }
}

/// Control frames out, in the order the worker queued them.
pub(super) async fn send_control(
    link: Link,
    mut stream: quinn::SendStream,
    mut frames: Receiver<ControlFrame>,
) -> Result<(), TransportFault> {
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
pub(super) async fn receive_gameplay(link: Link, mailbox: RelayMailbox) -> Result<(), TransportFault> {
    let role = link.role;
    let mut partial: HashMap<MemberId, Reassembler> = HashMap::new();
    loop {
        let arrived = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            arrived = link.quic.read_datagram() => arrived,
        };
        let raw = arrived.map_err(|error| link.fault("read_datagram", error))?;
        let datagram = match decode_relay(&raw) {
            Ok(datagram) => datagram,
            Err(error) => {
                diag::warn!(Net, "master {role} ignored relay datagram: {error}");
                continue;
            }
        };
        let (sender, piece) = match addressed_to_us(datagram, link.is_host) {
            Ok(routed) => routed,
            Err(other) => {
                diag::warn!(Net, "master {role} ignored unexpected datagram {other:?}");
                continue;
            }
        };
        match partial.entry(sender).or_insert_with(joiner).push(piece) {
            Ok(None) => {}
            Ok(Some(packet)) => {
                if let Err(error) = mailbox.push_inbound(sender, packet) {
                    diag::warn!(Net, "master {role} inbound mailbox: {error}");
                }
            }
            Err(error) => diag::warn!(Net, "master {role} fragment refused: {error}"),
        }
    }
}

/// Gameplay datagrams out: split each queued packet to fit and send the pieces.
pub(super) async fn send_gameplay(link: Link, mailbox: RelayMailbox) -> Result<(), TransportFault> {
    let role = link.role;
    let mut pieces = splitter();
    let mut flush = tokio::time::interval(FLUSH_EVERY);
    loop {
        tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            _ = flush.tick() => {}
        }
        for (member, packet) in mailbox.take_outbound() {
            let fragments = match pieces.split(&packet) {
                Ok(fragments) => fragments,
                Err(error) => {
                    diag::warn!(Net, "master {role} packet not splittable: {error}");
                    continue;
                }
            };
            for fragment in &fragments {
                let datagram = if link.is_host {
                    RelayDatagram::HostToMember { member_id: member, payload: fragment }
                } else {
                    RelayDatagram::ClientToHost(fragment)
                };
                let wire = encode_relay(datagram).map_err(|error| link.fault("encode_relay", error))?;
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
    lane: Arc<BootstrapLane>,
) -> Result<(), TransportFault> {
    let role = link.role;
    loop {
        let opened = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            opened = link.quic.accept_uni() => opened,
        };
        let mut stream = opened.map_err(|error| link.fault("accept_uni", error))?;
        let whole = tokio::time::timeout(IO_DEADLINE, stream.read_to_end(MAX_BOOTSTRAP_STREAM_BYTES));
        let bytes = match whole.await {
            Err(_late) => return Err(link.fault("bootstrap_uni_read", "deadline")),
            Ok(read) => read.map_err(|error| link.fault("bootstrap_uni_read", error))?,
        };
        let payload = match decode_relay_stream(&bytes) {
            Ok(RelayDatagram::ServiceToMember(payload) | RelayDatagram::HostToMember { payload, .. }) => {
                payload
            }
            Ok(other) => {
                diag::warn!(Net, "master {role} ignored unexpected uni {other:?}");
                continue;
            }
            Err(error) => {
                diag::warn!(Net, "master {role} ignored unfinished bootstrap stream: {error}");
                continue;
            }
        };
        if let Err(refusal) = lane.push_from_worker(None, payload.to_vec()) {
            let source = if refusal == "bootstrap from-worker queue is full" {
                refused_text("bootstrap_from_worker", true)
            } else {
                refusal.to_owned()
            };
            return Err(TransportFault::new("bootstrap_from_worker", role, source));
        }
    }
}

/// Bootstrap streams out (host side): sends each job, then tells the host
/// match which offer went to whom.
pub(super) async fn send_bootstraps(
    link: Link,
    mut jobs: Receiver<BootstrapJob>,
    prepared: Sender<HostMatchEvent>,
) -> Result<(), TransportFault> {
    loop {
        let next = tokio::select! {
            _ = link.cancel.cancelled() => return Ok(()),
            next = jobs.recv() => next,
        };
        let Some(job) = next else {
            return Ok(());
        };
        stream_bootstrap(&link, job.member, &job.bytes).await?;
        let Ok(Some(BootstrapMessage::Offer { bootstrap_id, connection, .. })) =
            decode_bootstrap(&job.bytes)
        else {
            continue;
        };
        let sent = HostMatchEvent::BootstrapPrepared {
            match_key: job.match_key,
            member: job.member,
            bootstrap_id,
            connection_id: Some(connection.0),
        };
        post(&prepared, sent, "bootstrap_prepared", link.role)?;
    }
}

/// One bootstrap snapshot as one low-priority one-way stream.
async fn stream_bootstrap(link: &Link, member_id: MemberId, payload: &[u8]) -> Result<(), TransportFault> {
    let envelope = encode_relay_stream(RelayDatagram::HostToMember { member_id, payload })
        .map_err(|error| link.fault("encode_bootstrap", error))?;
    if envelope.len() > MAX_BOOTSTRAP_STREAM_BYTES {
        let size = envelope.len();
        let budget = MAX_BOOTSTRAP_STREAM_BYTES;
        let why = format!("relay bootstrap envelope {size} bytes exceeds stream budget {budget}");
        return Err(link.fault("encode_bootstrap", why));
    }
    let mut stream = within_deadline(&link.cancel, link.quic.open_uni())
        .await
        .map_err(|error| link.fault("open_bootstrap_uni", error))?;
    stream
        .set_priority(BOOTSTRAP_PRIORITY)
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
        let member = MemberId([5; 16]);
        let to_host = RelayDatagram::ServiceToHost { member_id: member, payload: b"a" };
        assert_eq!(addressed_to_us(to_host.clone(), true), Ok((member, &b"a"[..])));
        assert!(addressed_to_us(to_host, false).is_err());
        let to_member = RelayDatagram::ServiceToMember(b"b");
        assert_eq!(addressed_to_us(to_member.clone(), false), Ok((FROM_HOST, &b"b"[..])));
        assert!(addressed_to_us(to_member, true).is_err());
        assert!(addressed_to_us(RelayDatagram::ClientToHost(b"c"), true).is_err());
    }

    #[test]
    fn a_packet_split_for_the_relay_comes_back_whole() {
        let packet: Vec<u8> = (0..RELAY_PACKET_BYTES).map(|at| at as u8).collect();
        let mut rebuilt = None;
        let mut join = joiner();
        for piece in splitter().split(&packet).unwrap() {
            assert!(piece.len() <= master_protocol::MAX_OPAQUE_PAYLOAD);
            if let Some(done) = join.push(&piece).unwrap() {
                rebuilt = Some(done);
            }
        }
        assert_eq!(rebuilt.as_deref(), Some(packet.as_slice()));
    }
}
