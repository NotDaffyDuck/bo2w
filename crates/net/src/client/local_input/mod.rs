//! The local player's input as it is gathered each frame and turned into a command for the
//! host.

mod look;
mod remote;
mod held_input;
mod timing;
mod intent_pack;

pub use look::{fold_aim_input, aim_angles_from_degrees};
pub use remote::remote_control_axes;
pub use held_input::{PlayerControlSnapshot, ViewAimAccumulator};
pub use timing::{INPUT_SPAN_MILLIS_LIMIT, frame_time_msec, pressed_frame_millis};
pub use intent_pack::{assemble_input_record, resting_command};
