//! Screenshots on request: wait for a settled frame, read the pixels back,
//! and write the file on an I/O worker.

mod drain;
mod facts;
mod queue;
mod request;
mod systems;

pub use drain::exit_is_user_quit;
pub use queue::ScreenshotBacklog;
pub use request::ScreenshotOrder;
pub(crate) use systems::{grab_screenshot_frame, complain_uncaptured_outputs};
