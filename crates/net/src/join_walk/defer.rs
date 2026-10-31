//! Test knobs that hold the host's reports back so a slow-load or slow-bootstrap peer can be
//! reproduced on demand.

use bevy::prelude::Resource;

/// `BO2MP_DEFER_HOST_WORLD_READY_MS=<n>` holds the host's world-ready report for `n` ms.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct HoldMapLoadedSignal {
    deadline_millis: Option<u64>,
}

impl HoldMapLoadedSignal {
    pub fn read_env_switch() -> Self {
        let delay_ms = std::env::var("BO2MP_DEFER_HOST_WORLD_READY_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok());
        let deadline_millis = delay_ms.map(|ms| wall_clock_ms().saturating_add(ms));
        if let Some(until) = deadline_millis {
            diag::warn!(WireLink, "host world-ready report held until {until}ms (BO2MP_DEFER_HOST_WORLD_READY_MS)");
        }
        Self { deadline_millis }
    }

    /// Is the report still held at `clock_millis`?
    pub fn holds_at(&self, clock_millis: u64) -> bool {
        world_ready_held_back(self.deadline_millis, clock_millis)
    }
}

/// `BO2MP_DEFER_BOOTSTRAP_APPLIED=1|true|TRUE` keeps the client from confirming the bootstrap.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct HoldTransferInstalled(pub bool);

impl HoldTransferInstalled {
    pub fn read_env_switch() -> Self {
        let on = matches!(
            std::env::var("BO2MP_DEFER_BOOTSTRAP_APPLIED").as_deref(),
            Ok("1" | "true" | "TRUE")
        );
        if on {
            diag::warn!(WireLink, "bootstrap-applied confirmation held back (BO2MP_DEFER_BOOTSTRAP_APPLIED)");
        }
        Self(on)
    }
}

pub fn world_ready_held_back(deadline_millis: Option<u64>, clock_millis: u64) -> bool {
    deadline_millis.is_some_and(|deadline_millis| clock_millis < deadline_millis)
}

pub(super) fn wall_clock_ms() -> u64 {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    elapsed.map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_deadline_never_defers() {
        assert!(!world_ready_held_back(None, 0));
        assert!(!world_ready_held_back(None, u64::MAX));
    }

    #[test]
    fn a_deadline_defers_until_it_is_reached() {
        assert!(world_ready_held_back(Some(100), 99));
        assert!(!world_ready_held_back(Some(100), 100));
        assert!(!world_ready_held_back(Some(100), 101));
    }

    #[test]
    fn the_default_knob_never_blocks() {
        assert!(!HoldMapLoadedSignal::default().holds_at(0));
        assert!(!HoldTransferInstalled::default().0);
    }

    #[test]
    fn a_knob_with_a_deadline_blocks_before_it() {
        let knob = HoldMapLoadedSignal { deadline_millis: Some(50) };
        assert!(knob.holds_at(10));
        assert!(!knob.holds_at(60));
    }

    #[test]
    fn the_wall_clock_is_after_2020() {
        assert!(wall_clock_ms() > 1_577_836_800_000);
    }
}
