//! The edges of a match: the warm-up state before play, whether the first spawns use the start points, and the entity
//! class of the end-of-match camera spot. Facts gm-12, gm-14, gm-15 and gm-32.

/// Where warm-up stands: waiting for the first player, then the prematch countdown, then over. Both waiting states
/// carry their time so far for the snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrematchStep {
    Waiting { elapsed_ms: u32 },
    Starting { elapsed_ms: u32 },
    Done,
}

impl Default for PrematchStep {
    /// A new match is waiting, from zero.
    fn default() -> Self {
        PrematchStep::Waiting { elapsed_ms: 0 }
    }
}

/// The first spawns of a match use the start points (every mode except free-for-all, see [`crate::dm`]).
pub const USE_START_SPAWNS_AT_START: bool = true;

/// Class name of the spot the camera sits at for the end-of-match screen.
pub const MP_GLOBAL_INTERMISSION: &str = "mp_global_intermission";
