//! The link worker: one thread with its own small runtime, holding one
//! connection to the lobby service for as long as the game sits in one room.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use master_protocol::{
    ControlEnvelope, ControlGreeting, PeerKindTag, PARALLEL_TRANSFER_LIMIT, QueryPayload,
};
use tokio::sync::mpsc::channel;
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::bridge::{DirectoryLink, Shared};
use super::fault::LinkFailureRecord;
use super::intent::{Dial, HostPlan, JoinPlan};
use super::pipes::{self, BootstrapJob, Link};
use super::quic::{self, build_tag, next_control, within_deadline, worker_runtime, write_control};
use super::room::Room;
use super::room_view::{DirectoryLinkStatus, LobbySessionLabel};
use crate::match_flow::HostRoundOccurrence;
use crate::net_links::match_handoff::TransferChannelState;
use crate::net_links::relay::ForwardingInbox;

/// Relayed packets the mailbox holds each way before it refuses more.
const RELAY_QUEUE: usize = 64;
/// Control frames waiting for the writer.
const CONTROL_QUEUE: usize = 32;
/// How long a goodbye waits for the service to confirm it, and how long
/// the background tasks get to wind down afterwards.
const GOODBYE_WAIT: Duration = Duration::from_millis(500);
/// The loop wakes at least this often to move queued bytes.
const IDLE_WAKE: Duration = Duration::from_millis(5);
/// The host match's slow clock.
const CLOCK: Duration = Duration::from_secs(1);

static LAUNCH_COUNTER: AtomicU64 = AtomicU64::new(1);
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// The room this worker goes to the service for.
pub(super) enum Errand {
    Host(HostPlan),
    Join(JoinPlan),
}

impl Errand {
    fn side(&self) -> &'static str {
        match self {
            Self::Host(_) => "host",
            Self::Join(_) => "join",
        }
    }

    fn dial(&self) -> &Dial {
        match self {
            Self::Host(plan) => &plan.dial,
            Self::Join(plan) => &plan.dial,
        }
    }

    fn greeting_role(&self) -> PeerKindTag {
        match self {
            Self::Host(_) => PeerKindTag::Host,
            Self::Join(_) => PeerKindTag::Join,
        }
    }

    /// The first request on the control stream: open the room or take a seat.
    fn opening(&self) -> QueryPayload {
        match self {
            Self::Host(plan) => QueryPayload::OpenLobby {
                name: plan.room_name.clone(),
                map: plan.map.clone(),
                mode: plan.ruleset.clone(),
                seat_capacity: plan.room_size,
                requires: plan.requires,
            },
            Self::Join(plan) => QueryPayload::JoinRoom { lobby_number: plan.room, have: plan.have },
        }
    }
}

/// Starts a worker for `errand` and returns the game's handle on it.
pub(super) fn start(errand: Errand, player_name: String) -> DirectoryLink {
    let mut identity = LobbySessionLabel::unassigned(NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed));
    if let Errand::Join(plan) = &errand {
        identity.lobby_number = plan.room;
    }
    let shared = Shared::new(
        DirectoryLinkStatus::Connecting { identity },
        ForwardingInbox::new(RELAY_QUEUE),
        TransferChannelState::new(),
    );
    let close = CancellationToken::new();
    let launch_no = LAUNCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let thread = std::thread::Builder::new()
        .name("bo2mp-master-bridge".into())
        .spawn({
            let shared = shared.clone();
            let close = close.clone();
            move || {
                if let Err(fault) = run(errand, player_name, shared.clone(), close, identity) {
                    shared.abort_with(identity, fault);
                }
            }
        })
        .expect("could not start master bridge thread");
    DirectoryLink::running(shared, close, launch_no, thread)
}

fn run(
    errand: Errand,
    player_name: String,
    shared: Shared,
    close: CancellationToken,
    identity: LobbySessionLabel,
) -> Result<(), LinkFailureRecord> {
    let runtime = worker_runtime().map_err(|cause| LinkFailureRecord::new("runtime", errand.side(), cause.to_string()))?;
    runtime.block_on(serve(errand, player_name, shared, close, identity))
}

/// A task's own fault, or the join error if it panicked or was aborted.
fn settle(ended: Result<Result<(), LinkFailureRecord>, JoinError>, side: &'static str) -> Result<(), LinkFailureRecord> {
    ended.unwrap_or_else(|join| Err(LinkFailureRecord::new("child", side, join.to_string())))
}

/// Sleeps until `at`, or forever without one.
async fn until(at: Option<Instant>) {
    let Some(deadline) = at else {
        return std::future::pending().await;
    };
    tokio::time::sleep_until(deadline).await
}

/// A close or leave request on its way, and when to stop waiting for its answer.
struct Goodbye {
    request: u64,
    give_up_at: Instant,
}

/// Moves what queued up between two wake-ups: the host match's due work,
/// relayed bytes and the game's orders. A goodbye, once sent, is recorded.
fn catch_up(
    room: &mut Room,
    shared: &Shared,
    goodbye: &mut Option<Goodbye>,
    side: &'static str,
    hosting: bool,
) -> Result<(), LinkFailureRecord> {
    if goodbye.is_none() {
        room.pump()?;
    }
    room.relay_out()?;
    for order in shared.take_orders() {
        // Once leaving, the game's remaining orders are moot.
        if goodbye.is_some() {
            continue;
        }
        if let Some(request) = room.order(order)? {
            let farewell = if hosting { "CloseRoom" } else { "LeaveRoom" };
            diag::info!(WireLink, "master {side} closing: {farewell} request={request}");
            *goodbye = Some(Goodbye { request, give_up_at: Instant::now() + GOODBYE_WAIT });
        }
    }
    Ok(())
}

async fn serve(
    errand: Errand,
    player_name: String,
    shared: Shared,
    close: CancellationToken,
    identity: LobbySessionLabel,
) -> Result<(), LinkFailureRecord> {
    let side = errand.side();
    let hosting = matches!(errand, Errand::Host(_));
    let stop = CancellationToken::new();
    // Until the room has been asked for, closing the bridge abandons the dial.
    let abandon = tokio::spawn({
        let close = close.clone();
        let stop = stop.clone();
        async move {
            close.cancelled().await;
            stop.cancel();
        }
    });

    let dial = errand.dial();
    diag::info!(
        WireLink,
        "master {side} connect begin attempt={} target={} server_name={} build={}",
        identity.join_try_number,
        dial.address,
        dial.server_name,
        build_tag()
    );
    let (_kept_endpoint, quic) = quic::dial(dial, &stop)
        .await
        .map_err(|cause| LinkFailureRecord::new("connect", side, cause.to_string()))?;
    diag::info!(
        WireLink,
        "master {side} hello build={} wire={} game={} attempt={}",
        build_tag(),
        master_protocol::WIRE_REV,
        crate::WIRE_REVISION_NUMBER,
        identity.join_try_number
    );
    let link_fault = |step: &'static str, cause: &dyn std::fmt::Display| LinkFailureRecord::on_link(step, side, cause, &quic);
    let (mut control_out, control_in) = within_deadline(&stop, quic.open_bi())
        .await
        .map_err(|error| link_fault("open_bi", &error))?;
    control_out.set_priority(0).map_err(|error| link_fault("control_priority", &error))?;
    let hello = ControlEnvelope::Hello(ControlGreeting {
        wire_rev: master_protocol::WIRE_REV,
        match_wire_version: crate::WIRE_REVISION_NUMBER,
        role: errand.greeting_role(),
        build: build_tag(),
        display_name: player_name,
    });
    write_control(&mut control_out, &hello).await.map_err(|error| link_fault("hello", &error))?;

    let link = Link { quic: quic.clone(), cancel: stop.clone(), side, hosting };
    let (control, frames) = channel::<ControlEnvelope>(CONTROL_QUEUE);
    let (jobs, job_queue) = channel::<BootstrapJob>(PARALLEL_TRANSFER_LIMIT as usize);
    let (sent_tx, mut sent) = channel::<HostRoundOccurrence>(PARALLEL_TRANSFER_LIMIT as usize);
    let mut tasks = JoinSet::new();
    tasks.spawn(pipes::send_control(link.clone(), control_out, frames));
    tasks.spawn(pipes::receive_gameplay(link.clone(), shared.mailbox.clone()));
    tasks.spawn(pipes::receive_bootstraps(link.clone(), shared.lane.clone()));
    tasks.spawn(pipes::send_gameplay(link.clone(), shared.mailbox.clone()));
    tasks.spawn(pipes::send_bootstraps(link, job_queue, sent_tx));

    let mut room = Room::new(shared.clone(), side, hosting, identity, control, jobs);
    abandon.abort();
    if stop.is_cancelled() {
        return Ok(());
    }
    room.send_query(errand.opening())?;

    let mut goodbye: Option<Goodbye> = None;
    let mut clock = tokio::time::interval(CLOCK);
    clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut reading = Box::pin(next_control(control_in));
    let ended = loop {
        if stop.is_cancelled() {
            break Ok(());
        }
        if let Err(fault) = catch_up(&mut room, &shared, &mut goodbye, side, hosting) {
            break Err(fault);
        }
        tokio::select! {
            _ = stop.cancelled() => break Ok(()),
            _ = close.cancelled(), if goodbye.is_none() => {}
            _ = until(goodbye.as_ref().map(|bye| bye.give_up_at)) => {
                diag::warn!(
                    WireLink,
                    "master {side} closed: relay did not confirm within {}ms",
                    GOODBYE_WAIT.as_millis()
                );
                break Ok(());
            }
            finished = sent.recv() => {
                let Some(event) = finished else {
                    break Err(LinkFailureRecord::new("bootstrap_worker", side, "prepared mailbox closed"));
                };
                if let Err(fault) = room.prepared(event) {
                    break Err(fault);
                }
            }
            _ = clock.tick() => {
                if let Err(fault) = room.tick() {
                    break Err(fault);
                }
            }
            (stream, read) = async {
                shared.mailbox.wait_control_inbound_capacity().await;
                (&mut reading).await
            } => {
                reading.set(next_control(stream));
                let frame = match read {
                    Ok(frame) => frame,
                    Err(error) => break Err(link_fault("control_read", &error)),
                };
                if let ControlEnvelope::PassAlong(bytes) = &frame {
                    if let Err(fault) = room.relay_in(bytes) {
                        break Err(fault);
                    }
                    continue;
                }
                let confirms_goodbye = goodbye.as_ref().is_some_and(|bye| match &frame {
                    ControlEnvelope::Response(answer) => answer.call_id == bye.request,
                    ControlEnvelope::Closed { .. } => true,
                    _ => false,
                });
                if let Err(fault) = room.frame(frame) {
                    break Err(fault);
                }
                if confirms_goodbye {
                    diag::info!(WireLink, "master {side} closed: relay confirmed");
                    break Ok(());
                }
            }
            Some(ended) = tasks.join_next() => match settle(ended, side) {
                Ok(()) if stop.is_cancelled() || goodbye.is_some() => break Ok(()),
                Ok(()) => break Err(LinkFailureRecord::new("child", side, "worker finished early")),
                Err(fault) => break Err(fault),
            },
            _ = tokio::time::sleep(IDLE_WAKE) => {}
        }
    };

    stop.cancel();
    quic.close(0_u8.into(), b"link shut down");
    let seat = room.seat_label;
    // Dropping the room drops the job queue, which lets the bootstrap sender finish.
    drop(room);
    let _ = tokio::time::timeout(GOODBYE_WAIT, async { while tasks.join_next().await.is_some() {} }).await;
    if ended.is_ok() && goodbye.is_some() {
        shared.left(seat);
    }
    ended
}

#[cfg(test)]
mod tests {
    use master_protocol::{ListingId, AssetCoverageBits};

    use super::*;

    fn dial() -> Dial {
        Dial { address: "lobby.test:1".into(), server_name: "lobby".into(), ca_file: None, serve_here: None }
    }

    #[test]
    fn a_host_opens_the_room_it_planned() {
        let errand = Errand::Host(HostPlan {
            dial: dial(),
            start_on_load: true,
            room_name: "room".into(),
            map: "mp_x".into(),
            ruleset: "tdm".into(),
            room_size: 12,
            requires: AssetCoverageBits(8),
            have: AssetCoverageBits(12),
        });
        assert_eq!(errand.side(), "host");
        assert_eq!(errand.greeting_role(), PeerKindTag::Host);
        let QueryPayload::OpenLobby { name, map, mode, seat_capacity, requires } = errand.opening() else {
            panic!("a host opens a room");
        };
        assert_eq!((name.as_str(), map.as_str(), mode.as_str()), ("room", "mp_x", "tdm"));
        assert_eq!((seat_capacity, requires), (12, AssetCoverageBits(8)));
    }

    #[test]
    fn a_joiner_asks_for_its_seat_with_what_it_has() {
        let room = ListingId([6; 16]);
        let errand = Errand::Join(JoinPlan {
            dial: dial(),
            room,
            map: "mp_x".into(),
            ruleset: "dm".into(),
            have: AssetCoverageBits(4),
        });
        assert_eq!(errand.side(), "join");
        assert_eq!(errand.greeting_role(), PeerKindTag::Join);
        assert_eq!(errand.opening(), QueryPayload::JoinRoom { lobby_number: room, have: AssetCoverageBits(4) });
        assert_eq!(errand.dial().server_name, "lobby");
    }

    #[test]
    fn a_dial_that_cannot_resolve_fails_the_bridge_as_connect() {
        let errand = Errand::Join(JoinPlan {
            dial: Dial { address: "no-port-here".into(), ..dial() },
            room: ListingId([6; 16]),
            map: "mp_x".into(),
            ruleset: "dm".into(),
            have: AssetCoverageBits(4),
        });
        let mut bridge = start(errand, "player".into());
        let worker = bridge.take_worker().expect("worker thread");
        worker.join().unwrap();
        let DirectoryLinkStatus::Failed { identity: seat, error: fault } = bridge.state() else {
            panic!("an unreachable lobby fails the bridge");
        };
        assert_eq!((fault.step_name, fault.side), ("connect", "join"));
        assert_eq!(seat.lobby_number, ListingId([6; 16]));
        assert_ne!(seat.join_try_number, 0);
    }
}
