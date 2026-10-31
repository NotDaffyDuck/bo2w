use std::collections::VecDeque;

use super::payload::Frame;
use crate::net_links::snapshot_wire::WorldPropStreamDecoder;
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader};

/// A way to move frames between the server and a client.
pub trait Transport {
    fn send(&mut self, frame: &Frame) -> Result<(), LinkIoFailure>;

    /// The next frame that has arrived, or `None` if none has.
    fn take_frame(&mut self) -> Result<Option<Frame>, LinkIoFailure>;
}

#[derive(Debug)]
pub enum LinkIoFailure {
    /// The other end is gone.
    Ended,
    /// Bytes arrived that are not a frame.
    Wire(PacketDecodeFault),
    Io(std::io::Error),
}

impl core::fmt::Display for LinkIoFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ended => write!(f, "frame feed closed"),
            Self::Wire(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LinkIoFailure {}

impl From<PacketDecodeFault> for LinkIoFailure {
    fn from(error: PacketDecodeFault) -> Self {
        Self::Wire(error)
    }
}

impl From<std::io::Error> for LinkIoFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// A transport that hands every frame sent into it back out of `recv`, as bytes, in order.
#[derive(Debug, Default)]
pub struct InProcessLink {
    backlog: VecDeque<Vec<u8>>,
    prop_reader: WorldPropStreamDecoder,
}

impl InProcessLink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Frames sent and not yet received.
    pub fn queued_len(&self) -> usize {
        self.backlog.len()
    }
}

impl Transport for InProcessLink {
    fn send(&mut self, frame: &Frame) -> Result<(), LinkIoFailure> {
        self.backlog.push_back(frame.wire_bytes());
        Ok(())
    }

    fn take_frame(&mut self) -> Result<Option<Frame>, LinkIoFailure> {
        let Some(raw) = self.backlog.pop_front() else {
            return Ok(None);
        };
        let got_frame = Frame::decode(&mut ByteCursorReader::new(&raw), &mut self.prop_reader)?;
        Ok(Some(got_frame))
    }
}

#[cfg(test)]
mod tests {
    use sim::{Snapshot, Tick, StepInputs};

    use super::*;
    use crate::FrameDiffWriter;
    use crate::net_links::tick_frames::frame_for_tick;

    fn frame_at(tick: u32) -> Frame {
        frame_for_tick(
            &mut FrameDiffWriter::new(),
            &StepInputs::default(),
            &Snapshot::unpublished(Tick(tick)),
        )
    }

    #[test]
    fn an_empty_loopback_has_nothing_to_receive() {
        let mut link = InProcessLink::new();
        assert_eq!(link.queued_len(), 0);
        assert!(link.take_frame().unwrap().is_none());
    }

    #[test]
    fn frames_come_out_in_the_order_they_went_in() {
        let mut link = InProcessLink::new();
        link.send(&frame_at(1)).unwrap();
        link.send(&frame_at(2)).unwrap();
        assert_eq!(link.queued_len(), 2);
        assert_eq!(link.take_frame().unwrap().unwrap().tick, Tick(1));
        assert_eq!(link.take_frame().unwrap().unwrap().tick, Tick(2));
        assert_eq!(link.queued_len(), 0);
        assert!(link.take_frame().unwrap().is_none());
    }

    #[test]
    fn errors_say_what_went_wrong() {
        assert_eq!(LinkIoFailure::Ended.to_string(), "frame feed closed");
        let io = LinkIoFailure::from(std::io::Error::other("cable out"));
        assert_eq!(io.to_string(), "cable out");
        assert!(matches!(
            LinkIoFailure::from(PacketDecodeFault::GarbledRequest("bad")),
            LinkIoFailure::Wire(_)
        ));
    }
}
