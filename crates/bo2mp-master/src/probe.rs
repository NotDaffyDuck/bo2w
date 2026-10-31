//! `status` and `list`: one request to a running service, printed to stdout.

use std::{io::Write, sync::Arc, time::Duration};

use master_protocol as proto;

use crate::args::Target;
use crate::Error;

/// Connecting and the answer together must take less than this.
const PATIENCE: Duration = Duration::from_secs(8);
/// The probe asks one thing per connection, so one id will do.
const ONLY_REQUEST: u64 = 1;
/// Close reasons sent once the answer has been printed.
const STATUS_DONE: &[u8] = b"status complete";
const LIST_DONE: &[u8] = b"list complete";

/// The endpoint (kept so the connection lives on), the connection and the answer.
type Asked = (quinn::Endpoint, quinn::Connection, proto::ReplyPayload);

/// Prints `protocol=.. opaque=.. members=..` when the service runs this exact protocol.
pub async fn status(target: &Target) -> Result<(), Error> {
    tokio::time::timeout(PATIENCE, print_status(target))
        .await
        .map_err(|_elapsed| "master status deadline (connect + RPC)")?
}

/// Prints the generation and adverts count, then one tab-separated row per room.
pub async fn list(target: &Target) -> Result<(), Error> {
    tokio::time::timeout(PATIENCE, print_rooms(target))
        .await
        .map_err(|_elapsed| "master list deadline (connect + RPC)")?
}

async fn print_status(target: &Target) -> Result<(), Error> {
    let (_keep_alive, conn, reply) = ask(target, proto::QueryPayload::Status).await?;
    let health = match reply {
        proto::ReplyPayload::Status(health) if health == proto::HealthReply::CURRENT => health,
        other => return Err(format!("unexpected Status response: {other:?}").into()),
    };
    writeln!(
        std::io::stdout(),
        "protocol={0} opaque={1} members={2}",
        health.wire_rev,
        health.opaque_body_ceiling,
        health.member_count_ceiling
    )?;
    conn.close(0_u8.into(), STATUS_DONE);
    Ok(())
}

async fn print_rooms(target: &Target) -> Result<(), Error> {
    let (_keep_alive, conn, reply) = ask(target, proto::QueryPayload::ListRooms).await?;
    let (gen_no, rooms) = match reply {
        proto::ReplyPayload::RoomList {
            generation: gen_no,
            adverts,
        } => (gen_no, adverts),
        other => return Err(format!("unexpected List response: {other:?}").into()),
    };
    let mut lines = std::io::stdout().lock();
    writeln!(lines, "generation={gen_no} adverts={}", rooms.len())?;
    for room in &rooms {
        let fill = format!("{}/{}", room.players, room.seat_capacity);
        writeln!(
            lines,
            "{}\t{fill}\t{}\t{}\tlocked={}\tin_match={}\tgen={}\trequires=0x{:02x}\t{}",
            room.id,
            room.map,
            room.mode,
            u8::from(room.locked),
            u8::from(room.playing),
            room.generation,
            room.requires.0,
            room.name,
        )?;
    }
    conn.close(0_u8.into(), LIST_DONE);
    Ok(())
}

/// Connects, says hello as a command-line client and returns the answer to `body`.
/// Room pushes that arrive first are skipped. The endpoint is returned so the
/// connection outlives this call.
async fn ask(
    target: &Target,
    body: proto::QueryPayload,
) -> Result<Asked, Error> {
    let (endpoint, link) = dial(target).await?;
    let (mut out, mut back) = link.open_bi().await?;
    out.set_priority(0)?;
    let hello = proto::ControlGreeting {
        wire_rev: proto::WIRE_REV,
        match_wire_version: 0,
        role: proto::PeerKindTag::Cli,
        build: "bo2mp-master".to_owned(),
        display_name: String::new(),
    };
    crate::wire::send(&mut out, &proto::ControlEnvelope::Hello(hello)).await?;
    let request = proto::ControlQuery {
        call_id: ONLY_REQUEST,
        body,
    };
    crate::wire::send(&mut out, &proto::ControlEnvelope::Request(request)).await?;
    loop {
        match crate::wire::receive(&mut back).await? {
            proto::ControlEnvelope::Response(answer) if answer.call_id == ONLY_REQUEST => {
                return Ok((endpoint, link, answer.body));
            }
            proto::ControlEnvelope::LobbyOverview(_)
            | proto::ControlEnvelope::Closed { .. }
            | proto::ControlEnvelope::RoomMemberNotice(_)
            | proto::ControlEnvelope::PassAlong(_) => {}
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
            for authority in crate::pem::certificates(path)? {
                trusted.add(authority)?;
            }
            let roots = rustls::ClientConfig::builder().with_root_certificates(trusted);
            roots.with_no_client_auth()
        }
        None => <rustls::ClientConfig as rustls_platform_verifier::ConfigVerifierExt>::with_platform_verifier()?,
    };
    tls.alpn_protocols.push(proto::ALPN.to_vec());
    let local = if target.address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut client = quinn::Endpoint::client(local.parse()?)?;
    client.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)?,
    )));
    let conn = client.connect(target.address, &target.server_name)?.await?;
    Ok((client, conn))
}
