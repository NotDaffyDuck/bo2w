//! The in-process link a listen server uses to feed its own client: frames go through the
//! same encode and decode as the network path, minus the sockets.

mod tick_queue;
mod tick_send;
mod received;

pub use tick_queue::LocalHostTickPipe;
pub use received::ArrivedHostStep;

#[cfg(test)]
mod tests {
    use sim::{ClientId, Snapshot, Tick, StepInputs};

    use super::*;
    use crate::client::run_ahead::InputCommandNumber;
    use crate::{GuaranteedEnvelope, HostSoundCue};

    pub(super) fn snapshot_at(at: u32) -> Snapshot {
        let mut snap = Snapshot::unpublished(Tick(0));
        snap.tick = Tick(at);
        snap
    }

    pub(super) fn nothing_reliable() -> GuaranteedEnvelope {
        GuaranteedEnvelope { confirmed_up_to: 0, entries: Vec::new(), evicted_stale_count: 0 }
    }

    pub(super) fn send(
        link: &mut LocalHostTickPipe,
        tick: u32,
        receipts_batch: Vec<(ClientId, InputCommandNumber)>,
        sounds: Vec<HostSoundCue>,
        scores: Option<String>,
    ) {
        link.transmit_step(
            &StepInputs::default(),
            &snapshot_at(tick),
            receipts_batch,
            sounds,
            scores,
            Vec::default(),
            Vec::default(),
            Vec::default(),
            Vec::default(),
            nothing_reliable(),
        )
        .expect("the loopback takes a tick");
    }

    #[test]
    fn ticks_come_out_in_the_order_they_went_in() {
        let mut link = LocalHostTickPipe::new();
        for tick in [5, 6, 7] {
            send(&mut link, tick, Vec::default(), Vec::default(), None);
        }
        assert_eq!(link.queued_len(), 3);
        let mut arrived = Vec::default();
        link.drain_into(&mut arrived).unwrap();
        let ticks: Vec<u32> = arrived.iter().map(|got| got.snapshot.tick.0).collect();
        assert_eq!(ticks, [5, 6, 7]);
        assert_eq!(link.queued_len(), 0);
        assert_eq!((link.dispatched, link.taken_in), (3, 3));
    }

    #[test]
    fn nothing_waiting_gives_none() {
        let mut link = LocalHostTickPipe::new();
        assert!(link.recv_tick().unwrap().is_none());
        assert_eq!(link.taken_in, 0);
    }

    #[test]
    fn the_backlog_is_noted_when_draining_and_its_peak_is_kept() {
        let mut link = LocalHostTickPipe::new();
        for tick in 1..=4 {
            send(&mut link, tick, Vec::default(), Vec::default(), None);
        }
        let mut arrived = Vec::default();
        link.drain_into(&mut arrived).unwrap();
        assert_eq!((link.latest_queue_depth, link.deepest_queue), (4, 4));

        send(&mut link, 5, Vec::default(), Vec::default(), None);
        link.drain_into(&mut arrived).unwrap();
        assert_eq!((link.latest_queue_depth, link.deepest_queue), (1, 4));
    }

    #[test]
    fn what_rode_along_with_the_snapshot_arrives_with_it() {
        let mut link = LocalHostTickPipe::new();
        send(
            &mut link,
            9,
            vec![(ClientId(2), InputCommandNumber(77))],
            Vec::default(),
            Some("b 0 0 0 0".to_owned()),
        );
        let got = link.recv_tick().unwrap().expect("one tick waiting");
        assert_eq!(got.receipt_for(ClientId(2)), Some(InputCommandNumber(77)));
        assert_eq!(got.receipt_for(ClientId(3)), None);
        assert_eq!(got.frame.board_text.as_deref(), Some("b 0 0 0 0"));
    }

    #[test]
    fn a_reset_forgets_queued_ticks_and_counters() {
        let mut link = LocalHostTickPipe::new();
        send(&mut link, 1, Vec::default(), Vec::default(), None);
        let mut arrived = Vec::default();
        link.drain_into(&mut arrived).unwrap();
        send(&mut link, 2, Vec::default(), Vec::default(), None);
        link.empty_out();
        assert_eq!(link.queued_len(), 0);
        assert_eq!((link.dispatched, link.taken_in, link.latest_queue_depth, link.deepest_queue), (0, 0, 0, 0));
    }
}
