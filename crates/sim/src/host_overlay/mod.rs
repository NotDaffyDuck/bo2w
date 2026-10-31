//! "Adopting" a snapshot: laying the host's authoritative state over a predicting client's world.
//!
//! A snapshot carries what a client must agree with the host on, not every field the world holds. What an adoption
//! leaves alone, and why that is safe:
//!
//! - **Random streams** (spawn, combat, bot). Snapshots carry only how many numbers each stream has drawn, not where it
//!   is, so a client cannot resume one. It never needs to: every draw feeds a decision the host already made and
//!   replicated (a spawn place, a spread). A client that re-runs a step draws something else and has it overwritten at
//!   the next adoption, which is why shots that depend on spread are settled by the host rather than predicted.
//! - **Shot and projectile id counters.** A predicted shot gets an id the host never issued; the real one arrives with
//!   the snapshot that is adopted over it. Nothing presented may be keyed by a predicted id, and events come from the
//!   adopted snapshot, not from predicted output. A projectile's flight is predicted; only its id waits.
//! - **Lag compensation state** (the pose history, the script-model collision history, the per-shooter sample record).
//!   Only the host resolves damage, so only the host rewinds. A client keeps a history from its own poses and never
//!   reads it.
//! - **The event journal and its counter.** The journal is emptied at the start of every step and republished from the
//!   adopted state; adopting it too would show one-shot events twice, once from the snapshot and once from the re-run.
//! - **The previous command's buttons and angles.** They are not in the snapshot but are not lost: they are on the
//!   previous user command, and the client puts them back on every adoption, a retired or missing acknowledgement
//!   included. Skipping the retired case once made a held sprint button look like a new press on a snapshot that
//!   already had the player sprinting.
//! - **Content** (collision brushes, weapon tables, equipment tables). The same on both sides by construction and
//!   checked by the content digest; adopting it would mean sending the map and the weapon data every tick.
//! - **Match identity** (the root seed and the started flag), fixed when the match is set up.
mod report;
pub use report::TakeoverReport;
