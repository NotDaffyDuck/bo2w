//! The listen host's loopback to its own client: the frame queue, the delta coder on each end of it, and the
//! counters the debug overlay reads.

use super::ArrivedHostStep;
use crate::net_links::snapshot_diff::{FrameDiffReader, FrameDiffWriter};
use crate::net_links::tick_frames::{InProcessLink, Transport, LinkIoFailure};

#[derive(bevy::prelude::Resource, Debug, Default)]
pub struct LocalHostTickPipe {
    pub(super) queue: InProcessLink,
    pub(super) encode: FrameDiffWriter,
    decode: FrameDiffReader,
    /// Frames put on the queue since the last reset.
    pub dispatched: u64,
    /// Frames taken off the queue since the last reset.
    pub taken_in: u64,
    /// The queue depth seen by the most recent drain.
    pub latest_queue_depth: usize,
    /// The largest `latest_queue_depth` seen.
    pub deepest_queue: usize,
}

impl LocalHostTickPipe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Frames sitting in the queue right now.
    pub fn queued_len(&self) -> usize {
        self.queue.queued_len()
    }

    /// Start over: empty queue, fresh coders, zeroed counters.
    pub fn empty_out(&mut self) {
        *self = Self::default();
    }

    /// Take the oldest frame and decode its snapshot against the previous one; `None` on an empty queue.
    pub fn recv_tick(&mut self) -> Result<Option<ArrivedHostStep>, LinkIoFailure> {
        let Some(raw) = self.queue.take_frame()? else {
            return Ok(None);
        };
        let mut snapshot = self.decode.decode(&raw.frame_diff_section)?;
        snapshot.meta.clone_from(&raw.meta_rows);
        self.taken_in += 1;
        Ok(Some(ArrivedHostStep { snapshot, frame: raw }))
    }

    /// Drain the queue into `out`, oldest first, after recording how deep it was.
    pub fn drain_into(&mut self, out: &mut Vec<ArrivedHostStep>) -> Result<(), LinkIoFailure> {
        self.latest_queue_depth = self.queued_len();
        self.deepest_queue = self.deepest_queue.max(self.latest_queue_depth);
        while let Some(tick) = self.recv_tick()? {
            out.push(tick);
        }
        Ok(())
    }
}
