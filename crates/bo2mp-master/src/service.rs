//! `serve`: the lobby service on one QUIC endpoint.
//!
//! Each accepted connection gets a link number. The [`Lobby`] decides what a
//! request does and who hears about it; this file only moves frames and packets
//! between connections, and cuts off a connection that cannot keep up.

use std::{
    collections::{hash_map::Entry, HashMap},
    fmt,
    io::Write,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use master_protocol::{
    decode_relay, decode_relay_stream, encode_relay, encode_relay_stream, ControlFrame,
    ControlRequest, ControlResponse, ProtocolError, RelayDatagram, ServiceError, ALPN,
    MAX_BOOTSTRAP_STREAM_BYTES, MAX_CONCURRENT_BOOTSTRAP, MAX_RELAY_UNI_STREAMS, PROTOCOL_VERSION,
    SESSION_IDLE, SESSION_KEEP_ALIVE,
};
use quinn::{crypto::rustls::QuicServerConfig, RecvStream, SendStream};
use tokio::{
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};

use crate::{
    lobby::{display_name, Link, Lobby, Outbox},
    pem, wire, Error,
};

/// Connections open at once, over all addresses.
const CONNECTION_CAP: usize = 256;
/// Connections open at once from one address: a household or a LAN party shares one.
const ADDRESS_CAP: usize = 32;
/// Control frames waiting for one connection's writer before it counts as stuck.
const CONTROL_QUEUE: usize = 32;
const CONTROL_PRIORITY: i32 = 0;
/// Bootstrap streams yield to the control stream.
const BOOTSTRAP_PRIORITY: i32 = -32;
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

/// One log line on stderr. Logging must never take the service down.
fn log(line: fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr(), "{line}");
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
    connection: quinn::Connection,
    /// Frames for this connection's control-stream writer.
    outbox: mpsc::Sender<ControlFrame>,
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
type Encoder = for<'p> fn(RelayDatagram<'p>) -> Result<Vec<u8>, ProtocolError>;

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
                line.connection
                    .close(CLOSE_OK.into(), b"control delivery failed");
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
        packet: &RelayDatagram<'_>,
        encode: Encoder,
    ) -> Result<(Line, Vec<u8>), Error> {
        let hop = self.lobby.route(from, packet)?;
        let Some(line) = self.lines.get(&hop.to) else {
            let gone = if hop.to_host {
                ServiceError::NotHost
            } else {
                ServiceError::NotMember
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
        if let Entry::Occupied(mut count) = open.entry(self.address) {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
    }
}

/// Runs the service until the endpoint closes.
pub async fn serve(bind: SocketAddr, cert: &Path, key: &Path) -> Result<(), Error> {
    let endpoint = quinn::Endpoint::server(server_config(cert, key)?, bind)?;
    let mut stderr = std::io::stderr();
    writeln!(stderr, "bo2mp-master listening on {}", endpoint.local_addr()?)?;
    writeln!(
        stderr,
        "bo2mp-master build={} wire={PROTOCOL_VERSION} alpn={ALPN:?}",
        env!("CARGO_PKG_VERSION"),
    )?;
    let hub: Shared = Arc::new(Mutex::new(Hub {
        lobby: Lobby::new(fresh_id),
        lines: HashMap::new(),
    }));
    let seats = Arc::new(Semaphore::new(CONNECTION_CAP));
    let per_address = Arc::new(Mutex::new(HashMap::new()));
    let mut next_link: Link = 1;
    while let Some(incoming) = endpoint.accept().await {
        // Prove the address before it holds anything: a forged first packet
        // would otherwise keep a seat until its handshake timed out.
        if !incoming.remote_address_validated() && incoming.may_retry() {
            let _ = incoming.retry();
            continue;
        }
        let remote = incoming.remote_address();
        let Some(quota) = AddressQuota::take(&per_address, remote.ip()) else {
            log(format_args!(
                "connection refused: {remote} at per-address cap {ADDRESS_CAP}"
            ));
            incoming.refuse();
            continue;
        };
        let Ok(seat) = Arc::clone(&seats).try_acquire_owned() else {
            log(format_args!(
                "connection refused: at connection cap {CONNECTION_CAP}"
            ));
            incoming.refuse();
            continue;
        };
        let link = next_link;
        next_link += 1;
        let started = Instant::now();
        log(format_args!("connection {link} incoming remote={remote}"));
        let hub = Arc::clone(&hub);
        tokio::spawn(async move {
            let _held = (seat, quota);
            match incoming.await {
                Ok(connection) => {
                    log(format_args!(
                        "connection {link} handshake ok remote={remote} elapsed_ms={}",
                        started.elapsed().as_millis()
                    ));
                    attend(&hub, link, connection).await;
                }
                Err(error) => log(format_args!(
                    "connection {link} handshake failed remote={remote} elapsed_ms={}: {error}",
                    started.elapsed().as_millis()
                )),
            }
        });
    }
    Ok(())
}

/// TLS from the PEM files, the lobby ALPN, and the session's stream and timer limits.
fn server_config(cert: &Path, key: &Path) -> Result<quinn::ServerConfig, Error> {
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(pem::certificates(cert)?, pem::private_key(key)?)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut transport = quinn::TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(1_u8.into())
        .max_concurrent_uni_streams(MAX_RELAY_UNI_STREAMS.into())
        .max_idle_timeout(Some(quinn::IdleTimeout::try_from(SESSION_IDLE)?))
        .keep_alive_interval(Some(SESSION_KEEP_ALIVE))
        .datagram_receive_buffer_size(Some(DATAGRAM_BUFFER))
        .datagram_send_buffer_size(DATAGRAM_BUFFER);
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    config.transport_config(Arc::new(transport));
    Ok(config)
}

/// One connection from handshake to goodbye, then its room is told it left.
async fn attend(hub: &Shared, link: Link, connection: quinn::Connection) {
    match converse(hub, link, &connection).await {
        Ok(()) => connection.close(CLOSE_OK.into(), b"session closed"),
        Err(error) => {
            let seat = match lock(hub).lobby.seat(link) {
                Ok((room, member)) => format!("room={room} member={member}"),
                Err(_) => "room=- member=-".to_owned(),
            };
            let reason = error.to_string();
            log(format_args!("connection {link} closed: {reason} ({seat})"));
            let shown = reason.len().min(CLOSE_REASON_BYTES);
            connection.close(CLOSE_FAULT.into(), &reason.as_bytes()[..shown]);
        }
    }
    let mut hub = lock(hub);
    let news = hub.hang_up(link);
    hub.deliver(news);
}

/// What a connection's helper task reports when it ends.
enum Finished {
    Writer(Result<(), Error>),
    Bootstrap,
}

/// The hello, then the connection's traffic until something ends it. Helper
/// tasks are stopped before this returns.
async fn converse(hub: &Shared, link: Link, connection: &quinn::Connection) -> Result<(), Error> {
    let (talk, listen) = tokio::time::timeout(HELLO_WAIT, connection.accept_bi())
        .await
        .map_err(|_| "hello deadline (accept_bi)")??;
    talk.set_priority(CONTROL_PRIORITY)?;
    let (listen, first) = tokio::time::timeout(HELLO_WAIT, wire::receive_owned(listen))
        .await
        .map_err(|_| "hello deadline (first frame)")?;
    let ControlFrame::Hello(hello) = first? else {
        return Err("first control frame must be Hello".into());
    };
    log(format_args!(
        "connection {link} hello role={:?} wire={} game={} build={:?}",
        hello.role, hello.protocol_version, hello.game_protocol, hello.build
    ));
    let (outbox, queued) = mpsc::channel(CONTROL_QUEUE);
    {
        let mut hub = lock(hub);
        hub.lobby.arrive(link, display_name(&hello.player_name));
        let line = Line {
            connection: connection.clone(),
            outbox,
        };
        hub.lines.insert(link, line);
    }
    let mut helpers = JoinSet::new();
    helpers.spawn(async move { Finished::Writer(write_queued(talk, queued).await) });
    let ending = exchange(hub, link, connection, listen, &mut helpers).await;
    helpers.abort_all();
    while helpers.join_next().await.is_some() {}
    ending
}

/// Writes queued control frames to the stream until the queue closes.
async fn write_queued(
    mut stream: SendStream,
    mut queued: mpsc::Receiver<ControlFrame>,
) -> Result<(), Error> {
    while let Some(frame) = queued.recv().await {
        wire::send(&mut stream, &frame).await?;
    }
    Ok(())
}

/// Serves requests, relayed packets and bootstrap streams from one connection.
/// Returns only when the connection must end.
async fn exchange(
    hub: &Shared,
    link: Link,
    connection: &quinn::Connection,
    listen: RecvStream,
    helpers: &mut JoinSet<Finished>,
) -> Result<(), Error> {
    let bootstrap_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_BOOTSTRAP as usize));
    // The read survives the other branches winning, so a frame is never torn.
    let mut next_frame = std::pin::pin!(wire::receive_owned(listen));
    loop {
        tokio::select! {
            (listen, frame) = &mut next_frame => {
                next_frame.set(wire::receive_owned(listen));
                match frame? {
                    ControlFrame::Request(ControlRequest { request_id, body }) => {
                        let mut hub = lock(hub);
                        let (reply, mut posts) = hub.lobby.answer(link, body);
                        let response = ControlResponse { request_id, body: reply };
                        posts.push((link, ControlFrame::Response(response)));
                        hub.deliver(posts);
                    }
                    ControlFrame::Relay(bytes) => {
                        let packet = decode_relay_stream(&bytes)?;
                        let (target, relayed) =
                            lock(hub).forward(link, &packet, encode_relay_stream)?;
                        if target.outbox.try_send(ControlFrame::Relay(relayed)).is_err() {
                            target
                                .connection
                                .close(CLOSE_OK.into(), b"control queue exhausted");
                            return Err("control queue exhausted".into());
                        }
                    }
                    ControlFrame::Hello(_) => return Err("duplicate Hello".into()),
                    _ => return Err("client sent a non-request control frame".into()),
                }
            }
            stream = connection.accept_uni() => {
                let mut stream = stream?;
                let Ok(slot) = Arc::clone(&bootstrap_slots).try_acquire_owned() else {
                    log(format_args!("connection {link} bootstrap refused: no slot"));
                    let _ = stream.stop(0_u8.into());
                    continue;
                };
                helpers.spawn(pass_bootstrap(Arc::clone(hub), link, stream, slot));
            }
            datagram = connection.read_datagram() => {
                if let Err(error) = pass_datagram(hub, link, &datagram?) {
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
fn pass_datagram(hub: &Shared, from: Link, bytes: &[u8]) -> Result<(), Error> {
    let packet = decode_relay(bytes)?;
    let (target, relayed) = lock(hub).forward(from, &packet, encode_relay)?;
    target.connection.send_datagram(relayed.into())?;
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

async fn copy_bootstrap(hub: &Shared, from: Link, stream: &mut RecvStream) -> Result<(), Error> {
    let bytes = stream.read_to_end(MAX_BOOTSTRAP_STREAM_BYTES).await?;
    let packet = decode_relay_stream(&bytes)?;
    let (target, relayed) = lock(hub).forward(from, &packet, encode_relay_stream)?;
    let mut out = target.connection.open_uni().await?;
    out.set_priority(BOOTSTRAP_PRIORITY)?;
    out.write_all(&relayed).await?;
    out.finish()?;
    Ok(())
}
