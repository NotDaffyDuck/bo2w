//! The server browser: a background thread that asks the lobby service for
//! its room list every two seconds, or straight away when the player asks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::prelude::Resource;
use master_protocol::{
    RoomListing, AssetCoverageBits, ControlEnvelope, ControlGreeting, ControlQuery, ControlReply,
    PeerKindTag, QueryPayload, ReplyPayload,
};
use tokio_util::sync::CancellationToken;

use super::intent::Dial;
use super::quic::{self, build_tag, next_control, within_deadline, worker_runtime, write_control};
use super::room_view::{DirectoryListing, DirectoryBrowserView};

/// The wait between two listings is this many steps...
const PAUSE_STEPS: u32 = 20;
/// ...of this long each; a refresh ends it at the next step.
const PAUSE_STEP: Duration = Duration::from_millis(100);
/// The browser's only request on each connection.
const LIST_REQUEST: u64 = 1;

fn shown(listing: &Arc<Mutex<DirectoryBrowserView>>) -> std::sync::MutexGuard<'_, DirectoryBrowserView> {
    listing.lock().expect("master browser state poisoned")
}

/// The game's handle on the browser thread.
#[derive(Resource)]
pub struct DirectoryBrowser {
    listing: Arc<Mutex<DirectoryBrowserView>>,
    /// Bumped by every refresh; the thread compares it across its pause.
    asked: Arc<AtomicU64>,
    stop: CancellationToken,
    /// The listing thread, dropped (detached) on close.
    thread: Option<JoinHandle<()>>,
}

impl DirectoryBrowser {
    /// Starts listing rooms from the service at `dial` for a PC that holds `have`.
    pub(super) fn open(dial: Dial, have: AssetCoverageBits) -> Self {
        // The first listing is on its way as soon as the thread starts.
        let first = DirectoryBrowserView { have, loading: true, ..Default::default() };
        let listing: Arc<Mutex<DirectoryBrowserView>> = Arc::new(Mutex::new(first));
        let (asked, stop) = (Arc::new(AtomicU64::new(0)), CancellationToken::new());
        let job = {
            let (listing, asked, stop) = (listing.clone(), asked.clone(), stop.clone());
            move || browse(dial, listing, asked, stop)
        };
        let thread = std::thread::Builder::new().name("bo2mp-master-browser".into()).spawn(job).expect("spawn master browser thread");
        Self { listing, asked, stop, thread: Some(thread) }
    }

    pub fn snapshot(&self) -> DirectoryBrowserView {
        shown(&self.listing).clone()
    }

    /// Cuts the current pause short so the next listing goes out now.
    pub fn refresh(&self) { self.asked.fetch_add(1, Ordering::Relaxed); }
}

impl Drop for DirectoryBrowser {
    fn drop(&mut self) {
        self.stop.cancel();
        // Detach: the thread sees the stop and ends on its own.
        drop(self.thread.take());
    }
}

fn browse(dial: Dial, listing: Arc<Mutex<DirectoryBrowserView>>, asked: Arc<AtomicU64>, stop: CancellationToken) {
    let made = worker_runtime();
    let Ok(tokio_rt) = made else {
        // No runtime, no listings: show why and end the thread.
        shown(&listing).error = made.err().map(|e| e.to_string());
        return;
    };
    let rounds = async move {
        // List, publish, pause; again until stopped.
        while !stop.is_cancelled() {
            shown(&listing).loading = true;
            let attempt = within_deadline(&stop, list_rooms(&dial, &stop)).await;
            record(&listing, attempt, &dial, &stop);
            if !pause(&asked, &stop).await {
                break;
            }
        }
    };
    tokio_rt.block_on(rounds);
}

/// One short connection: greet, ask for the room list, read until it comes.
async fn list_rooms(dial: &Dial, stop: &CancellationToken) -> Result<(u64, Vec<RoomListing>), Box<dyn std::error::Error + Send + Sync>> {
    let (_ep, link) = quic::dial(dial, stop).await?;
    let (mut out, mut inbound) = within_deadline(stop, link.open_bi()).await?;
    out.set_priority(0)?;
    let hello = ControlGreeting {
        wire_rev: master_protocol::WIRE_REV,
        match_wire_version: crate::WIRE_REVISION_NUMBER,
        role: PeerKindTag::Cli,
        build: build_tag(),
        display_name: String::new(),
    };
    write_control(&mut out, &ControlEnvelope::Hello(hello)).await?;
    let ask = ControlQuery { call_id: LIST_REQUEST, body: QueryPayload::ListRooms };
    write_control(&mut out, &ControlEnvelope::Request(ask)).await?;
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
fn rooms_answer(envelope: ControlEnvelope) -> Result<Option<(u64, Vec<RoomListing>)>, Box<dyn std::error::Error + Send + Sync>> {
    match envelope {
        ControlEnvelope::Response(ControlReply {
            call_id: LIST_REQUEST,
            body: ReplyPayload::RoomList { generation: list_gen, adverts: rooms },
        }) => Ok(Some((list_gen, rooms))),
        ControlEnvelope::Response(ControlReply { body: ReplyPayload::Error(refusal), .. }) => {
            Err(refusal.into())
        }
        ControlEnvelope::LobbyOverview(_)
        | ControlEnvelope::Closed { .. }
        | ControlEnvelope::RoomMemberNotice(_)
        | ControlEnvelope::Hello(_)
        | ControlEnvelope::PassAlong(_) => Ok(None),
        other => Err(format!("unexpected browser frame {other:?}").into()),
    }
}

/// Publishes one listing attempt. A failure is logged once per new message.
fn record(
    listing: &Arc<Mutex<DirectoryBrowserView>>,
    attempt: Result<(u64, Vec<RoomListing>), Box<dyn std::error::Error + Send + Sync>>,
    dial: &Dial,
    stop: &CancellationToken,
) {
    let mut shown = shown(listing);
    shown.loading = false;
    match attempt {
        Ok((list_generation, rooms)) => {
            let have = shown.have;
            shown.adverts = rooms.into_iter().map(|room| DirectoryListing::seen_with(room, have)).collect();
            (shown.generation, shown.error) = (list_generation, None);
        }
        Err(error) => {
            let text = error.to_string();
            let repeat = shown.error.as_deref() == Some(text.as_str());
            if !repeat && !stop.is_cancelled() {
                diag::warn!(
                    WireLink,
                    "master room-list fetch failed target={} server_name={}: {}",
                    dial.address,
                    dial.server_name,
                    text
                );
            }
            shown.error = Some(text);
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

    use master_protocol::{ListingId, DirectoryServiceFault, RoomShutdownCause};

    use super::*;

    fn advert(requires: u8) -> RoomListing {
        RoomListing {
            id: ListingId([2; 16]),
            name: "room".into(),
            map: "mp_x".into(),
            mode: "tdm".into(),
            players: 3,
            seat_capacity: 12,
            playing: true, locked: false,
            requires: AssetCoverageBits(requires),
            generation: 9,
        }
    }

    fn listing(have: u8) -> Arc<Mutex<DirectoryBrowserView>> {
        Arc::new(Mutex::new(DirectoryBrowserView { loading: true, have: AssetCoverageBits(have), ..Default::default() }))
    }

    fn dial() -> Dial {
        Dial { address: "lobby.test:1".into(), server_name: "lobby".into(), ca_file: None, serve_here: None }
    }

    fn answer(ask_id: u64, payload: ReplyPayload) -> ControlEnvelope {
        ControlEnvelope::Response(ControlReply { call_id: ask_id, body: payload })
    }

    #[test]
    fn only_the_answer_to_our_request_ends_the_wait() {
        let rooms = answer(LIST_REQUEST, ReplyPayload::RoomList { generation: 4, adverts: vec![advert(0)] });
        assert_eq!(rooms_answer(rooms).unwrap(), Some((4, vec![advert(0)])));
        assert_eq!(rooms_answer(ControlEnvelope::PassAlong(vec![1])).unwrap(), None);
        let closed = ControlEnvelope::Closed { lobby_number: ListingId([2; 16]), epoch: 1, reason: RoomShutdownCause::HostLeft };
        assert_eq!(rooms_answer(closed).unwrap(), None);
        let stray = answer(LIST_REQUEST + 1, ReplyPayload::RoomList { generation: 4, adverts: vec![] });
        assert!(rooms_answer(stray).unwrap_err().to_string().starts_with("unexpected browser frame "));
        assert!(rooms_answer(answer(LIST_REQUEST, ReplyPayload::Ack)).is_err());
    }

    #[test]
    fn a_refusal_reads_as_the_service_wrote_it() {
        let refusal = DirectoryServiceFault::Full;
        let shown = refusal.to_string();
        let error = rooms_answer(answer(7, ReplyPayload::Error(refusal))).unwrap_err();
        assert_eq!(error.to_string(), shown);
    }

    #[test]
    fn a_listing_marks_the_packs_this_pc_lacks() {
        let rows = listing(0b0100);
        record(&rows, Ok((5, vec![advert(0b1100), advert(0b0100)])), &dial(), &CancellationToken::new());
        let shown = shown(&rows).clone();
        assert!(!shown.loading);
        assert_eq!(shown.generation, 5);
        assert_eq!(shown.adverts[0].missing, AssetCoverageBits(0b1000));
        assert_eq!(shown.adverts[1].missing, AssetCoverageBits(0));
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
