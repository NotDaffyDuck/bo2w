//! `status` and `list`: one request to a running service, printed to stdout.

use std::{io::Write, sync::Arc, time::Duration};

use master_protocol::{
    ControlFrame, ControlHello, ControlRequest, EndpointRole, RequestBody, ResponseBody,
    StatusResponse, ALPN, PROTOCOL_VERSION,
};
use quinn::crypto::rustls::QuicClientConfig;
use rustls_platform_verifier::ConfigVerifierExt;

use crate::{args::Target, pem, wire, Error};

/// Connecting and the answer together must take less than this.
const PATIENCE: Duration = Duration::from_secs(8);
/// The probe asks one thing per connection, so one id will do.
const ONLY_REQUEST: u64 = 1;

/// Prints `protocol=.. opaque=.. members=..` when the service runs this exact protocol.
pub async fn status(target: &Target) -> Result<(), Error> {
    tokio::time::timeout(PATIENCE, print_status(target))
        .await
        .map_err(|_| "master status deadline (connect + RPC)")?
}

/// Prints the generation and adverts count, then one tab-separated row per room.
pub async fn list(target: &Target) -> Result<(), Error> {
    tokio::time::timeout(PATIENCE, print_rooms(target))
        .await
        .map_err(|_| "master list deadline (connect + RPC)")?
}

async fn print_status(target: &Target) -> Result<(), Error> {
    let (_endpoint, connection, reply) = ask(target, RequestBody::Status).await?;
    let status = match reply {
        ResponseBody::Status(status) if status == StatusResponse::CURRENT => status,
        other => return Err(format!("unexpected Status response: {other:?}").into()),
    };
    writeln!(
        std::io::stdout(),
        "protocol={} opaque={} members={}",
        status.protocol_version,
        status.max_opaque_payload,
        status.max_session_members
    )?;
    connection.close(0_u8.into(), b"status complete");
    Ok(())
}

async fn print_rooms(target: &Target) -> Result<(), Error> {
    let (_endpoint, connection, reply) = ask(target, RequestBody::ListRooms).await?;
    let (generation, adverts) = match reply {
        ResponseBody::RoomList {
            generation,
            adverts,
        } => (generation, adverts),
        other => return Err(format!("unexpected List response: {other:?}").into()),
    };
    let mut out = std::io::stdout().lock();
    writeln!(out, "generation={generation} adverts={}", adverts.len())?;
    for room in &adverts {
        let fill = format!("{}/{}", room.players, room.max_players);
        writeln!(
            out,
            "{}\t{fill}\t{}\t{}\tlocked={}\tin_match={}\tgen={}\trequires=0x{:02x}\t{}",
            room.id,
            room.map,
            room.mode,
            u8::from(room.locked),
            u8::from(room.in_match),
            room.generation,
            room.requires.0,
            room.name,
        )?;
    }
    connection.close(0_u8.into(), b"list complete");
    Ok(())
}

/// Connects, says hello as a command-line client and returns the answer to `body`.
/// Room pushes that arrive first are skipped. The endpoint is returned so the
/// connection outlives this call.
async fn ask(
    target: &Target,
    body: RequestBody,
) -> Result<(quinn::Endpoint, quinn::Connection, ResponseBody), Error> {
    let (endpoint, connection) = dial(target).await?;
    let (mut out, mut back) = connection.open_bi().await?;
    out.set_priority(0)?;
    let hello = ControlHello {
        protocol_version: PROTOCOL_VERSION,
        game_protocol: 0,
        role: EndpointRole::Cli,
        build: "bo2mp-master".to_owned(),
        player_name: String::new(),
    };
    wire::send(&mut out, &ControlFrame::Hello(hello)).await?;
    let request = ControlRequest {
        request_id: ONLY_REQUEST,
        body,
    };
    wire::send(&mut out, &ControlFrame::Request(request)).await?;
    loop {
        match wire::receive(&mut back).await? {
            ControlFrame::Response(answer) if answer.request_id == ONLY_REQUEST => {
                return Ok((endpoint, connection, answer.body));
            }
            ControlFrame::RoomView(_)
            | ControlFrame::Closed { .. }
            | ControlFrame::PeerEvent(_)
            | ControlFrame::Relay(_) => {}
            other => {
                return Err(
                    format!("unexpected control frame while waiting for response: {other:?}")
                        .into(),
                );
            }
        }
    }
}

/// A QUIC connection to the target, checked against its CA file or the system store.
async fn dial(target: &Target) -> Result<(quinn::Endpoint, quinn::Connection), Error> {
    let mut tls = match &target.ca_file {
        Some(path) => {
            let mut trusted = rustls::RootCertStore::empty();
            for authority in pem::certificates(path)? {
                trusted.add(authority)?;
            }
            rustls::ClientConfig::builder()
                .with_root_certificates(trusted)
                .with_no_client_auth()
        }
        None => rustls::ClientConfig::with_platform_verifier()?,
    };
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let local = if target.address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut endpoint = quinn::Endpoint::client(local.parse()?)?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(tls)?,
    )));
    let connection = endpoint.connect(target.address, &target.server_name)?.await?;
    Ok((endpoint, connection))
}
