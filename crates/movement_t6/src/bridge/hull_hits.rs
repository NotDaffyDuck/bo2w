//! What a move asks of the world it moves through.

use trace_t6::Trace;

use crate::bridge::SweepAsk;
use crate::bridge::solid_escape::{UnstickVerdict, correct_solid};

/// The collision queries a move makes.
pub trait SolidProbe {
    /// Sweep a box from `input.start` to `input.end`.
    fn trace(&self, input: SweepAsk) -> Trace;

    /// Nudge a player that starts inside solid to the nearest free spot.
    /// The default searches the 26 neighbouring offsets with [`Self::trace`].
    #[must_use] fn correct_solid(
        &self,
        at_pos: [f32; 3],
        box_lo: [f32; 3],
        box_hi: [f32; 3],
        collide_filter: u32,
    ) -> Option<UnstickVerdict> {
        correct_solid(at_pos, box_lo, box_hi, collide_filter, self)
    }

    /// bo2mp: the move struck, or stands inside, what `trace` hit. The server
    /// tells both sides "touch" once the move is over.
    fn brush_against(&self, _trace: &Trace, _end_pos: [f32; 3]) {}
}
