//! The server browser: a background thread that asks the lobby service for
//! its room list every two seconds, or straight away when the player asks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::prelude::Resource;
use master_protocol::{
    Advert, ContentFlags, ControlFrame, ControlHello, ControlRequest, ControlResponse,
    EndpointRole, RequestBody, ResponseBody,
};
use tokio_util::sync::CancellationToken;

use super::BoxError;
use super::intent::Dial;
use super::quic::{self, build_tag, next_control, within_deadline, worker_runtime, write_control};
use super::state::{MasterAdvert, MasterBrowserSnapshot};

/// The wait between two listings is this many steps...
const PAUSE_STEPS: u32 = 20;
/// ...of this long each; a refresh ends it at the next step.
const PAUSE_STEP: Duration = Duration::from_millis(100);
/// The browser's only request on each connection.
const LIST_REQUEST: u64 = 1;

type Listing = Arc<Mutex<MasterBrowserSnapshot>>;

fn shown(listing: &Listing) -> std::sync::MutexGuard<'_, MasterBrowserSnapshot> {
    listing.lock().expect("master browser state poisoned")
}

/// The game's handle on the browser thread.
#[derive(Resource)]
pub struct MasterBrowser {
    listing: Listing,
    /// Bumped by every refresh; the thread compares it across its pause.
    asked: Arc<AtomicU64>,
    stop: CancellationToken,
    worker: Option<JoinHandle<()>>,
}

impl MasterBrowser {
    /// Starts listing rooms from the service at `dial` for a PC that holds `have`.
    pub(super) fn open(dial: Dial, have: ContentFlags) -> Self {
        let listing: Listing = Arc::new(Mutex::new(MasterBrowserSnapshot {
            loading: true,
            have,
            ..Default::default()
        }));
        let asked = Arc::new(AtomicU64::new(0));
        let stop = CancellationToken::new();
        let worker = std::thread::Builder::new()
            .name("bo2mp-master-browser".into())
            .spawn({
                let (listing, asked, stop) = (listing.clone(), asked.clone(), stop.clone());
                move || browse(dial, listing, asked, stop)
            })
            .expect("spawn master browser thread");
        Self { listing, asked, stop, worker: Some(worker) }
    }

    pub fn snapshot(&self) -> MasterBrowserSnapshot {
        shown(&self.listing).clone()
    }

    pub fn refresh(&self) {
        self.asked.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for MasterBrowser {
    fn drop(&mut self) {
        self.stop.cancel();
        // Detach: the thread sees the stop and ends on its own.
        self.worker.take();
    }
}

fn browse(dial: Dial, listing: Listing, asked: Arc<AtomicU64>, stop: CancellationToken) {
    let runtime = match worker_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            shown(&listing).error = Some(error.to_string());
            return;
        }
    };
    runtime.block_on(async move {
        while !stop.is_cancelled() {
            shown(&listing).loading = true;
            let outcome = within_deadline(&stop, list_rooms(&dial, &stop)).await;
            record(&listing, outcome, &dial, &stop);
            if !pause(&asked, &stop).await {
                return;
            }
        }
    });
}

/// One short connection: greet, ask for the room list, read until it comes.
async fn list_rooms(dial: &Dial, stop: &CancellationToken) -> Result<(u64, Vec<Advert>), BoxError> {
    let (_endpoint, link) = quic::dial(dial, stop).await?;
    let (mut out, mut inbound) = within_deadline(stop, link.open_bi()).await?;
    out.set_priority(0)?;
    let hello = ControlHello {
        protocol_version: master_protocol::PROTOCOL_VERSION,
        game_protocol: crate::PROTOCOL_VERSION,
        role: EndpointRole::Cli,
        build: build_tag(),
        player_name: String::new(),
    };
    write_control(&mut out, &ControlFrame::Hello(hello)).await?;
    let ask = ControlRequest { request_id: LIST_REQUEST, body: RequestBody::ListRooms };
    write_control(&mut out, &ControlFrame::Request(ask)).await?;
    loop {
        let (stream, read) = next_control(inbound).await;
        inbound = stream;
        if let Some(rooms) = rooms_answer(read?)? {
            return Ok(rooms);
        }
    }
}

/// The room list when `frame` carries it; `None` for the room and peer news
/// the service may send first; an error for a refusal or a frame no browser expects.
fn rooms_answer(frame: ControlFrame) -> Result<Option<(u64, Vec<Advert>)>, BoxError> {
    match frame {
        ControlFrame::Response(ControlResponse {
            request_id: LIST_REQUEST,
            body: ResponseBody::RoomList { generation, adverts },
        }) => Ok(Some((generation, adverts))),
        ControlFrame::Response(ControlResponse { body: ResponseBody::Error(refusal), .. }) => {
            Err(refusal.into())
        }
        ControlFrame::RoomView(_)
        | ControlFrame::Closed { .. }
        | ControlFrame::PeerEvent(_)
        | ControlFrame::Hello(_)
        | ControlFrame::Relay(_) => Ok(None),
        other => Err(format!("unexpected browser frame {other:?}").into()),
    }
}

/// Publishes one listing attempt. A failure is logged once per new message.
fn record(
    listing: &Listing,
    outcome: Result<(u64, Vec<Advert>), BoxError>,
    dial: &Dial,
    stop: &CancellationToken,
) {
    let mut shown = shown(listing);
    shown.loading = false;
    match outcome {
        Ok((generation, adverts)) => {
            let have = shown.have;
            shown.generation = generation;
            shown.adverts = adverts.into_iter().map(|advert| MasterAdvert::seen_with(advert, have)).collect();
            shown.error = None;
        }
        Err(error) => {
            let message = error.to_string();
            if !stop.is_cancelled() && shown.error.as_deref() != Some(message.as_str()) {
                diag::warn!(
                    Net,
                    "master browser request failed target={} server_name={}: {}",
                    dial.address,
                    dial.server_name,
                    message
                );
            }
            shown.error = Some(message);
        }
    }
}

/// Waits out the pause between listings; false once the browser is stopped.
async fn pause(asked: &AtomicU64, stop: &CancellationToken) -> bool {
    let seen = asked.load(Ordering::Relaxed);
    for _ in 0..PAUSE_STEPS {
        tokio::select! {
            _ = stop.cancelled() => return false,
            _ = tokio::time::sleep(PAUSE_STEP) => {}
        }
        if asked.load(Ordering::Relaxed) != seen {
            break;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use master_protocol::{AdvertId, ServiceError, SessionCloseReason};

    use super::*;

    fn advert(requires: u8) -> Advert {
        Advert {
            id: AdvertId([2; 16]),
            name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            players: 3,
            max_players: 12,
            locked: false,
            in_match: true,
            requires: ContentFlags(requires),
            generation: 9,
        }
    }

    fn listing(have: u8) -> Listing {
        Arc::new(Mutex::new(MasterBrowserSnapshot { loading: true, have: ContentFlags(have), ..Default::default() }))
    }

    fn dial() -> Dial {
        Dial { address: "lobby.test:1".into(), server_name: "lobby".into(), ca_file: None }
    }

    fn answer(request_id: u64, body: ResponseBody) -> ControlFrame {
        ControlFrame::Response(ControlResponse { request_id, body })
    }

    #[test]
    fn only_the_answer_to_our_request_ends_the_wait() {
        let rooms = answer(LIST_REQUEST, ResponseBody::RoomList { generation: 4, adverts: vec![advert(0)] });
        assert_eq!(rooms_answer(rooms).unwrap(), Some((4, vec![advert(0)])));
        assert_eq!(rooms_answer(ControlFrame::Relay(vec![1])).unwrap(), None);
        let closed = ControlFrame::Closed { room_id: AdvertId([2; 16]), epoch: 1, reason: SessionCloseReason::HostLeft };
        assert_eq!(rooms_answer(closed).unwrap(), None);
        let stray = answer(LIST_REQUEST + 1, ResponseBody::RoomList { generation: 4, adverts: vec![] });
        assert!(rooms_answer(stray).unwrap_err().to_string().starts_with("unexpected browser frame "));
        assert!(rooms_answer(answer(LIST_REQUEST, ResponseBody::Ack)).is_err());
    }

    #[test]
    fn a_refusal_reads_as_the_service_wrote_it() {
        let refusal = ServiceError::Full;
        let shown = refusal.to_string();
        let error = rooms_answer(answer(7, ResponseBody::Error(refusal))).unwrap_err();
        assert_eq!(error.to_string(), shown);
    }

    #[test]
    fn a_listing_marks_the_packs_this_pc_lacks() {
        let rows = listing(0b0100);
        record(&rows, Ok((5, vec![advert(0b1100), advert(0b0100)])), &dial(), &CancellationToken::new());
        let shown = shown(&rows).clone();
        assert!(!shown.loading);
        assert_eq!(shown.generation, 5);
        assert_eq!(shown.adverts[0].missing, ContentFlags(0b1000));
        assert_eq!(shown.adverts[1].missing, ContentFlags(0));
    }

    #[test]
    fn a_failure_stays_shown_until_a_listing_works() {
        let rows = listing(0);
        record(&rows, Ok((2, vec![advert(0)])), &dial(), &CancellationToken::new());
        record(&rows, Err("lobby down".into()), &dial(), &CancellationToken::new());
        let failed = shown(&rows).clone();
        assert_eq!(failed.error.as_deref(), Some("lobby down"));
        assert_eq!((failed.generation, failed.adverts.len()), (2, 1));
        record(&rows, Ok((3, vec![])), &dial(), &CancellationToken::new());
        assert_eq!(shown(&rows).error, None);
    }

    #[test]
    fn a_refresh_or_a_stop_cuts_the_pause_short() {
        let runtime = worker_runtime().unwrap();
        let asked = Arc::new(AtomicU64::new(0));
        let stop = CancellationToken::new();
        let began = Instant::now();
        let bump = {
            let asked = asked.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                asked.fetch_add(1, Ordering::Relaxed);
            })
        };
        assert!(runtime.block_on(pause(&asked, &stop)));
        bump.join().unwrap();
        assert!(began.elapsed() < PAUSE_STEP * PAUSE_STEPS / 2);
        stop.cancel();
        assert!(!runtime.block_on(pause(&asked, &stop)));
    }
}
