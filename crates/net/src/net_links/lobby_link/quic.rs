//! Reaching the lobby service over QUIC: the dial, the trust roots, the framed
//! control stream, and the deadline every network wait runs under.

use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use master_protocol::{
    ALPN, ControlEnvelope, FORWARD_ONEWAY_CHANNEL_CAP, LINK_IDLE_LIMIT, LINK_PING_EVERY,
    read_stream_body, write_stream_frame, stream_frame_len,
};
use quinn::crypto::rustls::QuicClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio_util::sync::CancellationToken;

use super::intent::Dial;

/// The longest any single network step may take before the link gives up.
pub(super) const PIPE_WAIT_LIMIT: Duration = Duration::from_secs(8);

/// Each link worker owns one small single-threaded runtime.
pub(super) fn worker_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_current_thread();
    builder.enable_all();
    builder.build()
}

/// Milliseconds since 1970; the host match clock. A clock before 1970 reads 0.
pub(super) fn wall_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_millis() as u64,
        Err(_) => 0,
    }
}

/// What this game build calls itself in its hello.
pub(super) fn build_tag() -> String {
    let game = crate::WIRE_REVISION_NUMBER;
    let lobby = master_protocol::WIRE_REV;
    format!(
        "bo2mp-net/{} game={game} master={lobby}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Runs `step` unless `cancel` fires first or `IO_DEADLINE` passes.
pub(super) async fn within_deadline<T, E>(
    stop: &CancellationToken,
    step: impl Future<Output = Result<T, E>>,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let timed = tokio::time::timeout(PIPE_WAIT_LIMIT, step);
    let stopped = stop.cancelled();
    tokio::select! {
        _ = stopped => Err("session cancelled".into()),
        outcome = timed => outcome
            .map_err(|_elapsed| <Box<dyn std::error::Error + Send + Sync>>::from("master I/O deadline"))?
            .map_err(Into::into),
    }
}

/// Reads one length-prefixed control frame.
async fn read_control(recv: &mut quinn::RecvStream) -> Result<ControlEnvelope, Box<dyn std::error::Error + Send + Sync>> {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix).await?;
    let mut frame_body = vec![0u8; stream_frame_len(prefix)?];
    recv.read_exact(&mut frame_body).await?;
    Ok(read_stream_body(&frame_body)?)
}

/// Reads one frame and hands the stream back, so the read can sit in a
/// `select!` across loop turns without borrowing the stream.
pub(super) async fn next_control(
    recv: quinn::RecvStream,
) -> (quinn::RecvStream, Result<ControlEnvelope, Box<dyn std::error::Error + Send + Sync>>) {
    let mut recv = recv;
    let outcome = read_control(&mut recv).await;
    (recv, outcome)
}

pub(super) async fn write_control(
    send: &mut quinn::SendStream,
    frame: &ControlEnvelope,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let wire = write_stream_frame(frame)?;
    send.write_all(&wire).await?;
    Ok(())
}

/// Every certificate in a PEM file; a file with none is an error.
fn pem_roots(path: &Path) -> Result<Vec<CertificateDer<'static>>, Box<dyn std::error::Error + Send + Sync>> {
    let mut pem = BufReader::new(File::open(path)?);
    let found = rustls_pemfile::certs(&mut pem).collect::<std::io::Result<Vec<_>>>()?;
    if !found.is_empty() {
        return Ok(found);
    }
    Err(format!("no certificates found in {}", path.display()).into())
}

/// TLS for the dial: the given roots when a CA file is set, else the system's.
fn tls_for(ca_file: Option<&Path>) -> Result<rustls::ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let mut client_tls = if let Some(path) = ca_file {
        let mut store = RootCertStore::empty();
        for root in pem_roots(path)? {
            store.add(root)?;
        }
        let builder = rustls::ClientConfig::builder();
        builder.with_root_certificates(store).with_no_client_auth()
    } else {
        <rustls::ClientConfig as ConfigVerifierExt>::with_platform_verifier()?
    };
    client_tls.alpn_protocols = vec![ALPN.to_vec()];
    Ok(client_tls)
}

fn quic_for(tls: rustls::ClientConfig) -> Result<quinn::ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let mut timing = quinn::TransportConfig::default();
    let idle = quinn::IdleTimeout::try_from(LINK_IDLE_LIMIT).expect("session idle fits QUIC VarInt");
    timing
        .max_idle_timeout(Some(idle))
        .keep_alive_interval(Some(LINK_PING_EVERY))
        .max_concurrent_uni_streams(FORWARD_ONEWAY_CHANNEL_CAP.into());
    let mut quic = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    quic.transport_config(Arc::new(timing));
    Ok(quic)
}

fn first_address(dial: &Dial) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
    let mut found = dial.address.to_socket_addrs()?;
    found
        .next()
        .ok_or_else(|| format!("no addresses resolved for {}", dial.address).into())
}

/// Where this process's own lobby service listens, once a LAN host opened it.
static LAN_SERVICE: Mutex<Option<SocketAddr>> = Mutex::new(None);

/// A LAN host opens its lobby service once, before its first dial. The
/// certificate lives in the folder of the file the dial trusts. When the port
/// cannot be opened (another game on this PC may already serve it) the dial
/// goes ahead and says why if it fails.
fn serve_here_once(bind: SocketAddr, trusted: Option<&Path>) {
    let mut open = LAN_SERVICE.lock().unwrap_or_else(PoisonError::into_inner);
    if open.is_some() {
        return;
    }
    let folder = trusted.and_then(Path::parent).unwrap_or(Path::new("."));
    match bo2mp_master::lan::start(bind, folder) {
        Ok(listening) => {
            diag::info!(
                WireLink,
                "LAN host: lobby service listening on UDP {listening}; other PCs set BO2MP_MASTER_ADDR=<this PC's address>:{} BO2MP_MASTER_SERVER_NAME={} BO2MP_MASTER_CA_CERT=<a copy of {}>",
                listening.port(),
                bo2mp_master::lan::SERVER_NAME,
                folder.join(bo2mp_master::lan::CERT_FILE).display(),
            );
            *open = Some(listening);
        }
        Err(error) => diag::warn!(WireLink, "LAN host: cannot open UDP {bind}: {error}"),
    }
}

/// Opens a QUIC connection to the lobby service; the endpoint must outlive it.
pub(super) async fn dial(
    dial: &Dial,
    stop: &CancellationToken,
) -> Result<(quinn::Endpoint, quinn::Connection), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(bind) = dial.serve_here {
        serve_here_once(bind, dial.ca_file.as_deref());
    }
    let remote = first_address(dial)?;
    let quic = quic_for(tls_for(dial.ca_file.as_deref())?)?;
    let any_port = if remote.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut endpoint = quinn::Endpoint::client(any_port.parse()?)?;
    endpoint.set_default_client_config(quic);
    let local = endpoint.local_addr().map_err(<Box<dyn std::error::Error + Send + Sync>>::from)?;
    let trust = dial.ca_file.as_ref().map_or("platform", |_| "custom-ca");
    diag::info!(
        WireLink,
        "master quic handshake begin local={local} remote={remote} server_name={} trust={trust} alpn={ALPN:?}",
        dial.server_name,
    );
    let began = Instant::now();
    let handshake = endpoint.connect(remote, &dial.server_name)?;
    let session = match within_deadline(stop, handshake).await {
        Ok(session) => session,
        Err(error) => {
            let waited = began.elapsed().as_millis();
            let server_name = &dial.server_name;
            return Err(format!(
                "quic handshake failed ({error}); local={local} remote={remote} server_name={server_name} elapsed_ms={waited}"
            )
            .into());
        }
    };
    let peer = session.remote_address();
    let elapsed_ms = began.elapsed().as_millis();
    diag::info!(
        WireLink,
        "master quic handshake ok local={local} remote={peer} elapsed_ms={elapsed_ms}"
    );
    Ok((endpoint, session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tag_names_both_protocols() {
        let tag = build_tag();
        assert!(tag.starts_with("bo2mp-net/"));
        assert!(tag.contains(&format!(" game={} ", crate::WIRE_REVISION_NUMBER)));
        assert!(tag.ends_with(&format!(" master={}", master_protocol::WIRE_REV)));
    }

    #[test]
    fn deadline_reports_cancel_and_passes_results_through() {
        let runtime = worker_runtime().unwrap();
        let token = CancellationToken::new();
        let cancel = &token;
        let done = runtime.block_on(within_deadline(cancel, async { Ok::<_, Box<dyn std::error::Error + Send + Sync>>(5) }));
        assert_eq!(done.unwrap(), 5);
        let failed = runtime.block_on(within_deadline(cancel, async { Err::<u8, _>("broke") }));
        assert_eq!(failed.unwrap_err().to_string(), "broke");
        token.cancel();
        let never = std::future::pending::<Result<u8, Box<dyn std::error::Error + Send + Sync>>>();
        let stopped = runtime.block_on(within_deadline(cancel, never));
        assert_eq!(stopped.unwrap_err().to_string(), "session cancelled");
    }

    #[test]
    fn a_pem_without_certificates_is_refused() {
        let roots_file = std::env::temp_dir().join(format!("lobby-roots-{}.pem", std::process::id()));
        std::fs::write(&roots_file, b"not a certificate\n").unwrap();
        let error = pem_roots(&roots_file).unwrap_err().to_string();
        std::fs::remove_file(&roots_file).ok();
        assert!(error.starts_with("no certificates found in"), "{error}");
    }

    #[test]
    fn an_unresolvable_address_is_named() {
        let unresolved = Dial {
            address: "no-port-here".into(),
            server_name: "lobby".into(),
            ca_file: None,
            serve_here: None,
        };
        assert!(first_address(&unresolved).is_err());
    }

    /// A LAN host's first dial opens the lobby service's UDP port, and the
    /// game's own client trusts it with the certificate kept beside it. (On
    /// loopback, so the test never asks the firewall for a port.)
    #[test]
    fn a_lan_host_opens_its_port_and_reaches_it() {
        let folder = std::env::temp_dir().join(format!("bo2mp-lan-dial-{}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let lan = Dial {
            address: address.to_string(),
            server_name: bo2mp_master::lan::SERVER_NAME.into(),
            ca_file: Some(folder.join(bo2mp_master::lan::CERT_FILE)),
            serve_here: Some(address),
        };
        let runtime = worker_runtime().unwrap();
        let (_endpoint, link) = runtime.block_on(dial(&lan, &CancellationToken::new())).expect("dial the LAN host");
        assert_eq!(link.remote_address(), address);
        assert_eq!(*LAN_SERVICE.lock().unwrap(), Some(address));
        let taken = std::net::UdpSocket::bind(address).is_err();
        println!("LAN host listening on UDP {address}, handshake ok, port held: {taken}");
        assert!(taken, "the lobby service must hold UDP {address}");
        std::fs::remove_dir_all(&folder).ok();
    }
}
