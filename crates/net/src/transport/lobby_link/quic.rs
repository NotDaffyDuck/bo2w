//! Reaching the lobby service over QUIC: the dial, the trust roots, the framed
//! control stream, and the deadline every network wait runs under.

use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use master_protocol::{
    ALPN, ControlFrame, MAX_RELAY_UNI_STREAMS, SESSION_IDLE, SESSION_KEEP_ALIVE,
    decode_stream_payload, encode_stream_frame, stream_frame_len,
};
use quinn::crypto::rustls::QuicClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio_util::sync::CancellationToken;

use super::BoxError;
use super::intent::Dial;

/// The longest any single network step may take before the link gives up.
pub(super) const IO_DEADLINE: Duration = Duration::from_secs(8);

/// Each link worker owns one small single-threaded runtime.
pub(super) fn worker_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
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
    let game = crate::PROTOCOL_VERSION;
    let lobby = master_protocol::PROTOCOL_VERSION;
    format!("bo2mp-net/{} game={game} master={lobby}", env!("CARGO_PKG_VERSION"))
}

/// Runs `step` unless `cancel` fires first or `IO_DEADLINE` passes.
pub(super) async fn within_deadline<T, E: Into<BoxError>>(
    cancel: &CancellationToken,
    step: impl Future<Output = Result<T, E>>,
) -> Result<T, BoxError> {
    let timed = tokio::time::timeout(IO_DEADLINE, step);
    tokio::select! {
        _ = cancel.cancelled() => Err("session cancelled".into()),
        outcome = timed => match outcome {
            Err(_elapsed) => Err("master I/O deadline".into()),
            Ok(result) => result.map_err(Into::into),
        },
    }
}

/// Reads one length-prefixed control frame.
async fn read_control(recv: &mut quinn::RecvStream) -> Result<ControlFrame, BoxError> {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix).await?;
    let mut payload = vec![0u8; stream_frame_len(prefix)?];
    recv.read_exact(&mut payload).await?;
    Ok(decode_stream_payload(&payload)?)
}

/// Reads one frame and hands the stream back, so the read can sit in a
/// `select!` across loop turns without borrowing the stream.
pub(super) async fn next_control(
    mut recv: quinn::RecvStream,
) -> (quinn::RecvStream, Result<ControlFrame, BoxError>) {
    let read = read_control(&mut recv).await;
    (recv, read)
}

pub(super) async fn write_control(
    send: &mut quinn::SendStream,
    frame: &ControlFrame,
) -> Result<(), BoxError> {
    let wire = encode_stream_frame(frame)?;
    send.write_all(&wire).await?;
    Ok(())
}

/// Every certificate in a PEM file; a file with none is an error.
fn pem_roots(path: &Path) -> Result<Vec<CertificateDer<'static>>, BoxError> {
    let mut pem = BufReader::new(File::open(path)?);
    let found = rustls_pemfile::certs(&mut pem).collect::<std::io::Result<Vec<_>>>()?;
    if found.is_empty() {
        return Err(format!("{} contains no certificates", path.display()).into());
    }
    Ok(found)
}

/// TLS for the dial: the given roots when a CA file is set, else the system's.
fn tls_for(ca_file: Option<&Path>) -> Result<rustls::ClientConfig, BoxError> {
    let mut tls = match ca_file {
        None => rustls::ClientConfig::with_platform_verifier()?,
        Some(path) => {
            let mut store = RootCertStore::empty();
            for root in pem_roots(path)? {
                store.add(root)?;
            }
            rustls::ClientConfig::builder()
                .with_root_certificates(store)
                .with_no_client_auth()
        }
    };
    tls.alpn_protocols = vec![ALPN.to_vec()];
    Ok(tls)
}

fn quic_for(tls: rustls::ClientConfig) -> Result<quinn::ClientConfig, BoxError> {
    let mut timing = quinn::TransportConfig::default();
    let idle = quinn::IdleTimeout::try_from(SESSION_IDLE).expect("session idle fits QUIC VarInt");
    timing
        .max_idle_timeout(Some(idle))
        .keep_alive_interval(Some(SESSION_KEEP_ALIVE))
        .max_concurrent_uni_streams(MAX_RELAY_UNI_STREAMS.into());
    let mut quic = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    quic.transport_config(Arc::new(timing));
    Ok(quic)
}

fn first_address(dial: &Dial) -> Result<SocketAddr, BoxError> {
    let mut found = dial.address.to_socket_addrs()?;
    found
        .next()
        .ok_or_else(|| format!("{} resolved no addresses", dial.address).into())
}

/// Opens a QUIC connection to the lobby service; the endpoint must outlive it.
pub(super) async fn dial(
    dial: &Dial,
    cancel: &CancellationToken,
) -> Result<(quinn::Endpoint, quinn::Connection), BoxError> {
    let remote = first_address(dial)?;
    let quic = quic_for(tls_for(dial.ca_file.as_deref())?)?;
    let any_port = if remote.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let mut endpoint = quinn::Endpoint::client(any_port.parse()?)?;
    endpoint.set_default_client_config(quic);
    let local = endpoint.local_addr()?;
    let trust = match dial.ca_file {
        Some(_) => "custom-ca",
        None => "platform",
    };
    diag::info!(
        Net,
        "master quic handshake begin local={} remote={} server_name={} trust={} alpn={:?}",
        local,
        remote,
        dial.server_name,
        trust,
        ALPN
    );
    let began = Instant::now();
    let handshake = endpoint.connect(remote, &dial.server_name)?;
    let link = match within_deadline(cancel, handshake).await {
        Ok(link) => link,
        Err(error) => {
            let waited = began.elapsed().as_millis();
            let server_name = &dial.server_name;
            return Err(format!(
                "quic handshake: {error}; local={local} remote={remote} server_name={server_name} elapsed_ms={waited}"
            )
            .into());
        }
    };
    diag::info!(
        Net,
        "master quic handshake ok local={} remote={} elapsed_ms={}",
        local,
        link.remote_address(),
        began.elapsed().as_millis()
    );
    Ok((endpoint, link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tag_names_both_protocols() {
        let tag = build_tag();
        assert!(tag.starts_with("bo2mp-net/"));
        assert!(tag.contains(&format!(" game={} ", crate::PROTOCOL_VERSION)));
        assert!(tag.ends_with(&format!(" master={}", master_protocol::PROTOCOL_VERSION)));
    }

    #[test]
    fn deadline_reports_cancel_and_passes_results_through() {
        let runtime = worker_runtime().unwrap();
        let cancel = CancellationToken::new();
        let done = runtime.block_on(within_deadline(&cancel, async { Ok::<_, BoxError>(5) }));
        assert_eq!(done.unwrap(), 5);
        let failed =
            runtime.block_on(within_deadline(&cancel, async { Err::<u8, _>("broke") }));
        assert_eq!(failed.unwrap_err().to_string(), "broke");
        cancel.cancel();
        let never = std::future::pending::<Result<u8, BoxError>>();
        let stopped = runtime.block_on(within_deadline(&cancel, never));
        assert_eq!(stopped.unwrap_err().to_string(), "session cancelled");
    }

    #[test]
    fn a_pem_without_certificates_is_refused() {
        let path = std::env::temp_dir().join(format!("lobby-roots-{}.pem", std::process::id()));
        std::fs::write(&path, b"not a certificate\n").unwrap();
        let error = pem_roots(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).ok();
        assert!(error.ends_with("contains no certificates"), "{error}");
    }

    #[test]
    fn an_unresolvable_address_is_named() {
        let unresolved = Dial {
            address: "no-port-here".into(),
            server_name: "lobby".into(),
            ca_file: None,
        };
        assert!(first_address(&unresolved).is_err());
    }
}
