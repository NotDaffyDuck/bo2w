//! Effects a map script puts on the whole scene: fog that fades from one setting to another, and earthquakes that
//! shake the view.

mod earthquake;
mod fog;

pub use earthquake::{MAX_SCRIPT_EARTHQUAKES, ScriptEarthquake};
pub use fog::{ScriptFog, ScriptFogParams, ScriptSunFog};
