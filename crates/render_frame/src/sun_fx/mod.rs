//! The sun's screen effects: a sprite and flare at the sun's position, and the
//! full-screen blind and glare when looking toward it.
//!
//! The definition is installed with the world. The per-frame inputs ride along
//! with the frame that is already published. When data is missing, only this
//! effect switches off.

mod definition;
mod frame;

pub use self::definition::SunEffectsDef;
pub use self::frame::{SunEffectsFrame, angular_lerp};
