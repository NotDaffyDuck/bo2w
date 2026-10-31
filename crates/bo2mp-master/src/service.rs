//! `serve`: the lobby service on one QUIC endpoint.
//!
//! Each accepted connection gets a link number. The [`Lobby`] decides what a
//! request does and who hears about it; this file only moves frames and packets
//! between connections, and cuts off a connection that cannot keep up.

use std::{
    collections::hash_map,
    collections::HashMap,
    fmt,
    io::Write,
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use master_protocol as proto;
use quinn::{RecvStream, SendStream};
use tokio::{
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};
use tokio::select;

use crate::{
    lobby::{shown_label, Link, Lobby, Outbox},
    pem, wire, Error,
};

/// Connections open at once, over all addresses.
const CONNECTION_CAP: usize = 256;
/// Connections open at once from one address: a household or a LAN party shares one.
const ADDRESS_CAP: usize = 32;
/// Control frames waiting for one connection's writer before it counts as stuck.
const CONTROL_QUEUE: usize = 32;
const CONTROL_TRAFFIC_RANK: i32 = 0;
/// Bootstrap streams yield to the control stream.
const SETUP_TRAFFIC_RANK: i32 = -32;
/// How long a new connection has to open its control stream and say hello.
const HELLO_WAIT: Duration = Duration::from_secs(8);
/// How long one bootstrap stream may take to arrive and be passed on.
const FORWARD_WAIT: Duration = Duration::from_secs(8);
const DATAGRAM_BUFFER: usize = 128 * 1024;
/// Application close codes.
const CLOSE_OK: u32 = 0;
const CLOSE_FAULT: u32 = 1;
/// Close reasons longer than this are cut, so they fit in one packet.
const CLOSE_REASON_BYTES: usize = 120;
/// Close reasons for the ordinary endings.
const DELIVERY_FAILED: &[u8] = b"control delivery failed";
const SESSION_CLOSED: &[u8] = b"session closed";
const QUEUE_EXHAUSTED: &[u8] = b"control queue exhausted";
const NOT_HELLO: &str = "first control frame must be Hello";
/// This binary's version, as printed at start-up.
const BUILD: &str = env!("CARGO_PKG_VERSION");

type Outcome = Result<(), Error>;

/// One log line on stderr. Logging must never take the service down.
fn log(line: fmt::Arguments<'_>) {
    writeln!(std::io::stderr(), "{line}").ok();
}

/// A random advert or member id.
fn fresh_id() -> [u8; 16] {
    let mut id = [0; 16];
    getrandom::fill(&mut id).expect("OS randomness unavailable");
    id
}

/// A connection as the hub sees it.
#[derive(Clone)]
struct Line {
    quic: quinn::Connection,
    /// Frames for this connection's control-stream writer.
    outbox: mpsc::Sender<proto::ControlEnvelope>,
}

/// The lobby plus the connection behind every link.
struct Hub {
    lobby: Lobby,
    lines: HashMap<Link, Line>,
}

type Shared = Arc<Mutex<Hub>>;

/// The hub, even after a task panicked holding it: its maps stay consistent
/// between calls, so the service keeps going.
fn lock(hub: &Shared) -> MutexGuard<'_, Hub> {
    hub.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a relayed packet is framed again for the hop out.
type Encoder = for<'p> fn(proto::ForwardedPacket<'p>) -> Result<Vec<u8>, proto::ControlCodecFault>;

impl Hub {
    /// Queues each frame for its link, in order. A link whose queue is full or
    /// gone is cut off afterwards, and what its leaving changes for the others
    /// is delivered the same way.
    fn deliver(&mut self, posts: Outbox) {
        let mut stuck = Vec::new();
        for (link, frame) in posts {
            let Some(line) = self.lines.get(&link) else {
                continue;
            };
            if line.outbox.try_send(frame).is_err() {
                line.quic.close(CLOSE_OK.into(), DELIVERY_FAILED);
                stuck.push(link);
            }
        }
        for link in stuck {
            let news = self.hang_up(link);
            self.deliver(news);
        }
    }

    /// Forgets a link; returns what its leaving tells the rest of its room.
    fn hang_up(&mut self, link: Link) -> Outbox {
        self.lines.remove(&link);
        self.lobby.depart(link)
    }

    /// The connection a packet from `from` goes to, and its bytes for that hop.
    fn forward(
        &self,
        from: Link,
        packet: &proto::ForwardedPacket<'_>,
        encode: Encoder,
    ) -> Result<(Line, Vec<u8>), Error> {
        let hop = self.lobby.route(from, packet)?;
        let Some(line) = self.lines.get(&hop.to) else {
            let gone = if hop.to_host {
                proto::DirectoryServiceFault::NotRoomOwner
            } else {
                proto::DirectoryServiceFault::NotRoomOccupant
            };
            return Err(gone.into());
        };
        Ok((line.clone(), encode(hop.packet)?))
    }
}

/// One connection counted against its address until it is dropped.
struct AddressQuota {
    tally: Arc<Mutex<HashMap<IpAddr, usize>>>,
    address: IpAddr,
}

impl AddressQuota {
    fn take(tally: &Arc<Mutex<HashMap<IpAddr, usize>>>, address: IpAddr) -> Option<Self> {
        let mut open = tally.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(address).or_default();
        if *count >= ADDRESS_CAP {
            return None;
        }
        *count += 1;
        Some(Self {
            tally: Arc::clone(tally),
            address,
        })
    }
}

impl Drop for AddressQuota {
    fn drop(&mut self) {
        let mut open = self.tally.lock().unwrap_or_else(PoisonError::into_inner);
        if let hash_map::Entry::Occupied(mut count) = open.entry(self.address) {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
    }
}

/// Runs the service until the endpoint closes.
pub async fn serve(bind: std::net::SocketAddr, cert: &Path, key: &Path) -> Outcome {
    run(listen(bind, cert, key)?).await
}

/// Opens the service's UDP port. Must be called inside a Tokio runtime.
pub fn listen(bind: std::net::SocketAddr, cert: &Path, key: &Path) -> Result<quinn::Endpoint, Error> {
    Ok(quinn::Endpoint::server(server_config(cert, key)?, bind)?)
}

/// Serves on an endpoint from [`listen`] until it closes.
pub async fn run(endpoint: quinn::Endpoint) -> Outcome {
    let mut stderr = std::io::stderr();
    writeln!(stderr, "bo2mp-master listening on {}", endpoint.local_addr()?)?;
    writeln!(
        stderr,
        "bo2mp-master build={BUILD} wire={} alpn={:?}",
        proto::WIRE_REV,
        proto::ALPN,
    )?;
    let hub: Shared = Arc::new(Mutex::new(Hub {
        lobby: Lobby::new(fresh_id),
        lines: HashMap::new(),
    }));
    let seats = Arc::new(Semaphore::new(CONNECTION_CAP));
    let per_address = Arc::new(Mutex::new(HashMap::new()));
    let mut next_link: Link = 1;
    while let Some(pending) = endpoint.accept().await {
        // Prove the address before it holds anything: a forged first packet
        // would otherwise keep a seat until its handshake timed out.
        if !pending.remote_address_validated() && pending.may_retry() {
            let _ = pending.retry();
            continue;
        }
        let remote = pending.remote_address();
        let Some(quota) = AddressQuota::take(&per_address, remote.ip()) else {
            log(format_args!(
                "connection refused: {remote} at per-address cap {ADDRESS_CAP}"
            ));
            pending.refuse();
            continue;
        };
        let Ok(seat) = Arc::clone(&seats).try_acquire_owned() else {
            log(format_args!(
                "connection refused: at connection cap {CONNECTION_CAP}"
            ));
            pending.refuse();
            continue;
        };
        let link = next_link;
        next_link += 1;
        let accepted_at = Instant::now();
        log(format_args!("connection {link} incoming remote={remote}"));
        let hub = Arc::clone(&hub);
        tokio::spawn(async move {
            let _held = (seat, quota);
            match pending.await {
                Ok(conn) => {
                    log(format_args!(
                        "connection {link} handshake ok remote={remote} elapsed_ms={}",
                        accepted_at.elapsed().as_millis()
                    ));
                    attend(&hub, link, conn).await;
                }
                Err(error) => log(format_args!(
                    "connection {link} handshake failed remote={remote} elapsed_ms={}: {error}",
                    accepted_at.elapsed().as_millis()
                )),
            }
        });
    }
    Ok(())
}

/// TLS from the PEM files, the lobby ALPN, and the session's stream and timer limits.
fn server_config(cert: &Path, key: &Path) -> Result<quinn::ServerConfig, Error> {
    let builder = rustls::ServerConfig::builder().with_no_client_auth();
    let mut tls = builder.with_single_cert(pem::certificates(cert)?, pem::private_key(key)?)?;
    tls.alpn_protocols.push(proto::ALPN.to_vec());
    let mut limits = quinn::TransportConfig::default();
    limits
        .max_concurrent_bidi_streams(1_u8.into())
        .max_concurrent_uni_streams(proto::FORWARD_ONEWAY_CHANNEL_CAP.into())
        .max_idle_timeout(Some(quinn::IdleTimeout::try_from(proto::LINK_IDLE_LIMIT)?))
        .keep_alive_interval(Some(proto::LINK_PING_EVERY))
        .datagram_receive_buffer_size(Some(DATAGRAM_BUFFER))
        .datagram_send_buffer_size(DATAGRAM_BUFFER);
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quinn::crypto::rustls::QuicServerConfig::try_from(tls)?));
    config.transport_config(Arc::new(limits));
    Ok(config)
}

/// One connection from handshake to goodbye, then its room is told it left.
async fn attend(hub: &Shared, link: Link, conn: quinn::Connection) {
    match converse(hub, link, &conn).await {
        Ok(()) => conn.close(CLOSE_OK.into(), SESSION_CLOSED),
        Err(error) => {
            let seat = match lock(hub).lobby.seat(link) {
                Ok((room, member)) => format!("room={room} member={member}"),
                Err(_) => "room=- member=-".to_owned(),
            };
            let why = error.to_string();
            log(format_args!("connection {link} closed: {why} ({seat})"));
            let shown = why.len().min(CLOSE_REASON_BYTES);
            conn.close(CLOSE_FAULT.into(), &why.as_bytes()[..shown]);
        }
    }
    let mut hub = lock(hub);
    let news = hub.hang_up(link);
    hub.deliver(news);
}

/// What a connection's helper task reports when it ends.
enum Finished {
    Writer(Outcome),
    Bootstrap,
}

/// The hello, then the connection's traffic until something ends it. Helper
/// tasks are stopped before this returns.
async fn converse(hub: &Shared, link: Link, conn: &quinn::Connection) -> Outcome {
    let (talk, listen) = tokio::time::timeout(HELLO_WAIT, conn.accept_bi())
        .await
        .map_err(|_elapsed| "hello deadline (accept_bi)")??;
    talk.set_priority(CONTROL_TRAFFIC_RANK)?;
    let (listen, first) = tokio::time::timeout(HELLO_WAIT, wire::receive_owned(listen))
        .await
        .map_err(|_elapsed| "hello deadline (first frame)")?;
    let proto::ControlEnvelope::Hello(greeting) = first? else {
        return Err(NOT_HELLO.into());
    };
    log(format_args!(
        "connection {link} hello role={:?} wire={} game={} build={:?}",
        greeting.role, greeting.wire_rev, greeting.match_wire_version, greeting.build
    ));
    let (outbox, queued) = mpsc::channel(CONTROL_QUEUE);
    {
        let mut hub = lock(hub);
        hub.lobby.arrive(link, shown_label(&greeting.display_name));
        let line = Line {
            quic: conn.clone(),
            outbox,
        };
        hub.lines.insert(link, line);
    }
    let mut helpers = JoinSet::new();
    helpers.spawn(async move { Finished::Writer(write_queued(talk, queued).await) });
    let ending = exchange(hub, link, conn, listen, &mut helpers).await;
    helpers.abort_all();
    while helpers.join_next().await.is_some() {}
    ending
}

/// Writes queued control frames to the stream until the queue closes.
async fn write_queued(
    mut writer: SendStream,
    mut queued: mpsc::Receiver<proto::ControlEnvelope>,
) -> Outcome {
    while let Some(envelope) = queued.recv().await {
        wire::send(&mut writer, &envelope).await?;
    }
    Ok(())
}

/// Serves requests, relayed packets and bootstrap streams from one connection.
/// Returns only when the connection must end.
async fn exchange(
    hub: &Shared,
    link: Link,
    conn: &quinn::Connection,
    listen: RecvStream,
    helpers: &mut JoinSet<Finished>,
) -> Outcome {
    let bootstrap_slots = Arc::new(Semaphore::new(proto::PARALLEL_TRANSFER_LIMIT as usize));
    // The read survives the other branches winning, so a frame is never torn.
    let mut next_frame = std::pin::pin!(wire::receive_owned(listen));
    loop {
        select! {
            (listen, frame) = &mut next_frame => {
                next_frame.set(wire::receive_owned(listen));
                match frame? {
                    proto::ControlEnvelope::Request(query) => {
                        let mut hub = lock(hub);
                        let (reply, mut posts) = hub.lobby.answer(link, query.body);
                        let response = proto::ControlReply { body: reply, call_id: query.call_id };
                        posts.push((link, proto::ControlEnvelope::Response(response)));
                        hub.deliver(posts);
                    }
                    proto::ControlEnvelope::PassAlong(bytes) => {
                        let packet = proto::read_relay_stream(&bytes)?;
                        let (dest, relayed) =
                            lock(hub).forward(link, &packet, proto::write_relay_stream)?;
                        if dest.outbox.try_send(proto::ControlEnvelope::PassAlong(relayed)).is_err() {
                            dest.quic.close(CLOSE_OK.into(), QUEUE_EXHAUSTED);
                            return Err("control queue exhausted".into());
                        }
                    }
                    proto::ControlEnvelope::Hello(_) => return Err("duplicate Hello".into()),
                    _ => return Err("client sent a non-request control frame".into()),
                }
            }
            uni = conn.accept_uni() => {
                let mut stream = uni?;
                let Ok(slot) = Arc::clone(&bootstrap_slots).try_acquire_owned() else {
                    log(format_args!("connection {link} bootstrap refused: no slot"));
                    let _ = stream.stop(0_u8.into());
                    continue;
                };
                helpers.spawn(pass_bootstrap(Arc::clone(hub), link, stream, slot));
            }
            dgram = conn.read_datagram() => {
                if let Err(error) = pass_datagram(hub, link, &dgram?) {
                    log(format_args!("relay datagram: {error}"));
                }
            }
            finished = helpers.join_next() => match finished {
                Some(Ok(Finished::Writer(Err(error)))) => return Err(error),
                Some(Ok(Finished::Writer(Ok(())))) => return Err("control writer ended".into()),
                Some(Ok(Finished::Bootstrap)) | None => {}
                Some(Err(failure)) => return Err(failure.into()),
            },
        }
    }
}

/// One relayed game packet, passed on as a datagram.
fn pass_datagram(hub: &Shared, from: Link, raw: &[u8]) -> Outcome {
    let packet = proto::read_relay(raw)?;
    let (dest, relayed) = lock(hub).forward(from, &packet, proto::write_relay)?;
    dest.quic.send_datagram(relayed.into())?;
    Ok(())
}

/// Passes one bootstrap stream on to its target within the deadline; failures
/// are logged and never end the sender's connection.
async fn pass_bootstrap(
    hub: Shared,
    from: Link,
    mut stream: RecvStream,
    _slot: OwnedSemaphorePermit,
) -> Finished {
    match tokio::time::timeout(FORWARD_WAIT, copy_bootstrap(&hub, from, &mut stream)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log(format_args!("bootstrap forward: {error}")),
        Err(_) => log(format_args!("bootstrap forward: transfer deadline")),
    }
    Finished::Bootstrap
}

async fn copy_bootstrap(hub: &Shared, from: Link, stream: &mut RecvStream) -> Outcome {
    let bytes = stream.read_to_end(proto::TRANSFER_STREAM_CAP).await?;
    let packet = proto::read_relay_stream(&bytes)?;
    let (dest, relayed) = lock(hub).forward(from, &packet, proto::write_relay_stream)?;
    let mut out = dest.quic.open_uni().await?;
    out.set_priority(SETUP_TRAFFIC_RANK)?;
    out.write_all(&relayed).await?;
    out.finish()?;
    Ok(())
}
