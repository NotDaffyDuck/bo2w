//! What a move asks of the world it moves through.

use trace_t6::Trace;

use crate::bridge::GroundTraceInput;
use crate::bridge::correct_solid::{CorrectSolidOutcome, correct_solid};

/// The collision queries a move makes.
pub trait CollisionBackend {
    /// Sweep a box from `input.start` to `input.end`.
    fn trace(&self, input: GroundTraceInput) -> Trace;

    /// Nudge a player that starts inside solid to the nearest free spot.
    /// The default searches the 26 neighbouring offsets with [`Self::trace`].
    fn correct_solid(
        &self,
        origin: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        tracemask: u32,
    ) -> Option<CorrectSolidOutcome> {
        correct_solid(origin, mins, maxs, tracemask, self)
    }

    /// bo2mp: the move struck, or stands inside, what `trace` hit. The server
    /// tells both sides "touch" once the move is over.
    fn touch_entity(&self, _trace: &Trace, _end_pos: [f32; 3]) {}
}
