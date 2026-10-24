//! What the start-up scripts' level notifies mean to the match.

use crate::frame::FrameWorld;
use crate::identities::MatchPhase;

/// The one notify the start-up scripts raise that the match acts on: the end of
/// the pre-match wait opens play.
pub(crate) fn apply_level_notify(frame: &mut FrameWorld, name: &str) {
    if name == "prematch_over" && frame.phase() == MatchPhase::Warmup {
        crate::score::finish_prematch(frame);
    }
}
