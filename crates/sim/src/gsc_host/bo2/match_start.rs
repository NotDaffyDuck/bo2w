//! What the start-up scripts' level notifies mean to the match.

use crate::step_view::StepWorldView;
use crate::match_ids::GameRoundStage;

/// The one notify the start-up scripts raise that the match acts on: the end of
/// the pre-match wait opens play.
pub(crate) fn apply_level_notify(frame: &mut StepWorldView, called: &str) {
    if called == "prematch_over" && frame.phase() == GameRoundStage::Warmup {
        crate::round_clock::finish_prematch(frame);
    }
}
