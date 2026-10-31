//! A frame taken off the loopback, with its snapshot decoded.

use sim::{ClientId, Snapshot};

use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::tick_frames::Frame;

#[derive(Clone, Debug)]
pub struct ArrivedHostStep {
    pub snapshot: Snapshot,
    pub frame: Frame,
}

impl ArrivedHostStep {
    /// The newest command the host confirms for `client`, if this frame says.
    pub fn receipt_for(&self, client: ClientId) -> Option<InputCommandNumber> {
        self.frame.receipt_for(client)
    }
}
