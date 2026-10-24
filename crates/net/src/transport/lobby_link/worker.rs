//! The link worker: one thread with its own small runtime, holding one
//! connection to the lobby service for as long as the game sits in one room.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use master_protocol::{
    ControlFrame, ControlHello, EndpointRole, MAX_CONCURRENT_BOOTSTRAP, RequestBody,
};
use tokio::sync::mpsc::channel;
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::bridge::{MasterBridge, Shared};
use super::fault::TransportFault;
use super::intent::{Dial, HostPlan, JoinPlan};
use super::pipes::{self, BootstrapJob, Link};
use super::quic::{self, build_tag, next_control, within_deadline, worker_runtime, write_control};
use super::room::Room;
use super::state::{MasterBridgeState, SessionIdentity};
use crate::session_core::HostMatchEvent;
use crate::transport::bootstrap::BootstrapLane;
use crate::transport::relay::RelayMailbox;

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

static NEXT_BRIDGE: AtomicU64 = AtomicU64::new(1);
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// The room this worker goes to the service for.
pub(super) enum Errand {
    Host(HostPlan),
    Join(JoinPlan),
}

impl Errand {
    fn role(&self) -> &'static str {
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

    fn greeting_role(&self) -> EndpointRole {
        match self {
            Self::Host(_) => EndpointRole::Host,
            Self::Join(_) => EndpointRole::Join,
        }
    }

    /// The first request on the control stream: open the room or take a seat.
    fn opening(&self) -> RequestBody {
        match self {
            Self::Host(plan) => RequestBody::CreateRoom {
                name: plan.room_name.clone(),
                map: plan.map.clone(),
                mode: plan.mode.clone(),
                max_players: plan.room_size,
                requires: plan.requires,
            },
            Self::Join(plan) => RequestBody::JoinRoom { room_id: plan.room, have: plan.have },
        }
    }
}

/// Starts a worker for `errand` and returns the game's handle on it.
pub(super) fn start(errand: Errand, player_name: String) -> MasterBridge {
    let mut identity = SessionIdentity::unassigned(NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed));
    if let Errand::Join(plan) = &errand {
        identity.room_id = plan.room;
    }
    let shared = Shared::new(
        MasterBridgeState::Connecting { identity },
        RelayMailbox::new(RELAY_QUEUE),
        BootstrapLane::new(),
    );
    let close = CancellationToken::new();
    let incarnation = NEXT_BRIDGE.fetch_add(1, Ordering::Relaxed);
    let worker = std::thread::Builder::new()
        .name("bo2mp-master-bridge".into())
        .spawn({
            let shared = shared.clone();
            let close = close.clone();
            move || {
                if let Err(fault) = run(errand, player_name, shared.clone(), close, identity) {
                    shared.fail(identity, fault);
                }
            }
        })
        .expect("spawn master bridge thread");
    MasterBridge::running(shared, close, incarnation, worker)
}

fn run(
    errand: Errand,
    player_name: String,
    shared: Shared,
    close: CancellationToken,
    identity: SessionIdentity,
) -> Result<(), TransportFault> {
    let runtime = worker_runtime().map_err(|error| TransportFault::new("runtime", errand.role(), error.to_string()))?;
    runtime.block_on(serve(errand, player_name, shared, close, identity))
}

/// A task's own fault, or the join error if it panicked or was aborted.
fn settle(ended: Result<Result<(), TransportFault>, JoinError>, role: &'static str) -> Result<(), TransportFault> {
    ended.unwrap_or_else(|join| Err(TransportFault::new("child", role, join.to_string())))
}

/// Sleeps until `at`, or forever without one.
async fn until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
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
    role: &'static str,
    is_host: bool,
) -> Result<(), TransportFault> {
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
            let farewell = if is_host { "CloseRoom" } else { "LeaveRoom" };
            diag::info!(Net, "master {role} closing: {farewell} request={request}");
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
    identity: SessionIdentity,
) -> Result<(), TransportFault> {
    let role = errand.role();
    let is_host = matches!(errand, Errand::Host(_));
    let cancel = CancellationToken::new();
    // Until the room has been asked for, closing the bridge abandons the dial.
    let abandon = tokio::spawn({
        let close = close.clone();
        let cancel = cancel.clone();
        async move {
            close.cancelled().await;
            cancel.cancel();
        }
    });

    let dial = errand.dial();
    diag::info!(
        Net,
        "master {role} connect begin attempt={} target={} server_name={} build={}",
        identity.attempt_id,
        dial.address,
        dial.server_name,
        build_tag()
    );
    let (_endpoint, quic) = quic::dial(dial, &cancel)
        .await
        .map_err(|error| TransportFault::new("connect", role, error.to_string()))?;
    diag::info!(
        Net,
        "master {role} hello build={} wire={} game={} attempt={}",
        build_tag(),
        master_protocol::PROTOCOL_VERSION,
        crate::PROTOCOL_VERSION,
        identity.attempt_id
    );
    let link_fault = |step: &'static str, cause: &dyn std::fmt::Display| TransportFault::on_link(step, role, cause, &quic);
    let (mut control_out, control_in) = within_deadline(&cancel, quic.open_bi())
        .await
        .map_err(|error| link_fault("open_bi", &error))?;
    control_out.set_priority(0).map_err(|error| link_fault("control_priority", &error))?;
    let hello = ControlFrame::Hello(ControlHello {
        protocol_version: master_protocol::PROTOCOL_VERSION,
        game_protocol: crate::PROTOCOL_VERSION,
        role: errand.greeting_role(),
        build: build_tag(),
        player_name,
    });
    write_control(&mut control_out, &hello).await.map_err(|error| link_fault("hello", &error))?;

    let link = Link { quic: quic.clone(), cancel: cancel.clone(), role, is_host };
    let (control, frames) = channel::<ControlFrame>(CONTROL_QUEUE);
    let (jobs, job_queue) = channel::<BootstrapJob>(MAX_CONCURRENT_BOOTSTRAP as usize);
    let (sent_tx, mut sent) = channel::<HostMatchEvent>(MAX_CONCURRENT_BOOTSTRAP as usize);
    let mut tasks = JoinSet::new();
    tasks.spawn(pipes::send_control(link.clone(), control_out, frames));
    tasks.spawn(pipes::receive_gameplay(link.clone(), shared.mailbox.clone()));
    tasks.spawn(pipes::receive_bootstraps(link.clone(), shared.lane.clone()));
    tasks.spawn(pipes::send_gameplay(link.clone(), shared.mailbox.clone()));
    tasks.spawn(pipes::send_bootstraps(link, job_queue, sent_tx));

    let mut room = Room::new(shared.clone(), role, is_host, identity, control, jobs);
    abandon.abort();
    if cancel.is_cancelled() {
        return Ok(());
    }
    room.request(errand.opening())?;

    let mut goodbye: Option<Goodbye> = None;
    let mut clock = tokio::time::interval(CLOCK);
    clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut reading = Box::pin(next_control(control_in));
    let outcome = loop {
        if cancel.is_cancelled() {
            break Ok(());
        }
        if let Err(fault) = catch_up(&mut room, &shared, &mut goodbye, role, is_host) {
            break Err(fault);
        }
        tokio::select! {
            _ = cancel.cancelled() => break Ok(()),
            _ = close.cancelled(), if goodbye.is_none() => {}
            _ = until(goodbye.as_ref().map(|bye| bye.give_up_at)) => {
                diag::warn!(
                    Net,
                    "master {role} closed: relay did not confirm within {}ms",
                    GOODBYE_WAIT.as_millis()
                );
                break Ok(());
            }
            finished = sent.recv() => {
                let Some(event) = finished else {
                    break Err(TransportFault::new("bootstrap_worker", role, "prepared mailbox closed"));
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
                if let ControlFrame::Relay(bytes) = &frame {
                    if let Err(fault) = room.relay_in(bytes) {
                        break Err(fault);
                    }
                    continue;
                }
                let confirms_goodbye = goodbye.as_ref().is_some_and(|bye| match &frame {
                    ControlFrame::Response(answer) => answer.request_id == bye.request,
                    ControlFrame::Closed { .. } => true,
                    _ => false,
                });
                if let Err(fault) = room.frame(frame) {
                    break Err(fault);
                }
                if confirms_goodbye {
                    diag::info!(Net, "master {role} closed: relay confirmed");
                    break Ok(());
                }
            }
            Some(ended) = tasks.join_next() => match settle(ended, role) {
                Ok(()) if cancel.is_cancelled() || goodbye.is_some() => break Ok(()),
                Ok(()) => break Err(TransportFault::new("child", role, "task ended")),
                Err(fault) => break Err(fault),
            },
            _ = tokio::time::sleep(IDLE_WAKE) => {}
        }
    };

    cancel.cancel();
    quic.close(0_u8.into(), b"session closed");
    let seat = room.identity;
    // Dropping the room drops the job queue, which lets the bootstrap sender finish.
    drop(room);
    let _ = tokio::time::timeout(GOODBYE_WAIT, async { while tasks.join_next().await.is_some() {} }).await;
    if outcome.is_ok() && goodbye.is_some() {
        shared.left(seat);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use master_protocol::{AdvertId, ContentFlags};

    use super::*;

    fn dial() -> Dial {
        Dial { address: "lobby.test:1".into(), server_name: "lobby".into(), ca_file: None }
    }

    #[test]
    fn a_host_opens_the_room_it_planned() {
        let errand = Errand::Host(HostPlan {
            dial: dial(),
            start_on_load: true,
            room_name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            room_size: 12,
            requires: ContentFlags(8),
            have: ContentFlags(12),
        });
        assert_eq!(errand.role(), "host");
        assert_eq!(errand.greeting_role(), EndpointRole::Host);
        let RequestBody::CreateRoom { name, map, mode, max_players, requires } = errand.opening() else {
            panic!("a host opens a room");
        };
        assert_eq!((name.as_str(), map.as_str(), mode.as_str()), ("room", "mp_x", "tdm"));
        assert_eq!((max_players, requires), (12, ContentFlags(8)));
    }

    #[test]
    fn a_joiner_asks_for_its_seat_with_what_it_has() {
        let room = AdvertId([6; 16]);
        let errand = Errand::Join(JoinPlan {
            dial: dial(),
            room,
            map: "mp_x".into(),
            mode: "dm".into(),
            have: ContentFlags(4),
        });
        assert_eq!(errand.role(), "join");
        assert_eq!(errand.greeting_role(), EndpointRole::Join);
        assert_eq!(errand.opening(), RequestBody::JoinRoom { room_id: room, have: ContentFlags(4) });
        assert_eq!(errand.dial().server_name, "lobby");
    }

    #[test]
    fn a_dial_that_cannot_resolve_fails_the_bridge_as_connect() {
        let errand = Errand::Join(JoinPlan {
            dial: Dial { address: "no-port-here".into(), ..dial() },
            room: AdvertId([6; 16]),
            map: "mp_x".into(),
            mode: "dm".into(),
            have: ContentFlags(4),
        });
        let mut bridge = start(errand, "player".into());
        let worker = bridge.take_worker().expect("worker thread");
        worker.join().unwrap();
        let MasterBridgeState::Failed { identity, error } = bridge.state() else {
            panic!("an unreachable lobby fails the bridge");
        };
        assert_eq!((error.operation, error.role), ("connect", "join"));
        assert_eq!(identity.room_id, AdvertId([6; 16]));
        assert_ne!(identity.attempt_id, 0);
    }
}
