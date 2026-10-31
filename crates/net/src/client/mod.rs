//! The client half: input, prediction, interpolation of other players, and what gets drawn.

pub mod drive; // the per-frame client systems and their state
pub mod frame_tally; // per-frame measurements of the client schedule
pub mod frame_clock; // the client's game clock
pub mod local_input; // keys, mouse and pad into one command
pub mod pad_aim; // the pad stick's look and aim assist
pub mod run_ahead; // move history and reconciliation
pub mod correction_fade; // the view offset left after a correction
pub mod presentation; // entities, events and what the snapshot shows
pub mod remote_buffer; // interpolation of other players
pub mod realtime; // the client's millisecond clock
pub mod view_z_smooth; // smoothing the view height over steps (bo2zm)
