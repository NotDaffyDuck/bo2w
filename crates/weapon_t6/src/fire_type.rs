//! A weapon file's fire type, and when a launcher's rocket shows on the viewmodel.

/// A fire type number with no fire type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnrecognisedTriggerPattern(pub i32);

/// How the trigger fires: held down, once a pull, or a burst of 2 to 4 a pull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerPattern {
    FullAuto,
    SingleShot,
    TwoRoundVolley,
    BurstFire3,
    BurstFire4,
}

impl TriggerPattern {
    const ALL: [Self; 5] = [
        Self::FullAuto,
        Self::SingleShot,
        Self::TwoRoundVolley,
        Self::BurstFire3,
        Self::BurstFire4,
    ];

    /// The fire type a weapon file numbers `value` (0 full auto, 1 single, 2 to 4 burst).
    pub fn from_i32(value: i32) -> Result<Self, UnrecognisedTriggerPattern> {
        usize::try_from(value)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .ok_or(UnrecognisedTriggerPattern(value))
    }

    /// Fires a burst a pull.
    pub fn fires_volleys(self) -> bool {
        let volley = matches!(self, Self::TwoRoundVolley | Self::BurstFire3 | Self::BurstFire4);
        volley
    }
}

/// A launcher's rocket shows on the viewmodel while one is loaded, and during a reload
/// once more than `rocket_reappear_millis` of it has run.
#[must_use]
pub fn rocket_stays_on_launcher(
    clip: i32,
    state_now: i32,
    busy_ms: i32,
    reload_duration_millis: i32,
    rocket_reappear_millis: i32,
) -> bool {
    let reloading = crate::state::is_any_reload(state_now);
    clip > 0 || reloading && reload_duration_millis.wrapping_sub(busy_ms) > rocket_reappear_millis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_types_number_from_full_auto() {
        assert_eq!(TriggerPattern::from_i32(0), Ok(TriggerPattern::FullAuto));
        assert_eq!(TriggerPattern::from_i32(3), Ok(TriggerPattern::BurstFire3));
        assert_eq!(TriggerPattern::from_i32(5), Err(UnrecognisedTriggerPattern(5)));
        assert_eq!(TriggerPattern::from_i32(-1), Err(UnrecognisedTriggerPattern(-1)));
        assert!(TriggerPattern::TwoRoundVolley.fires_volleys());
        assert!(!TriggerPattern::SingleShot.fires_volleys());
    }

    #[test]
    fn the_rocket_shows_late_in_a_reload() {
        let reload = crate::state::RELOADING;
        assert!(rocket_stays_on_launcher(
            1,
            crate::state::READY,
            0,
            0,
            0
        ));
        assert!(!rocket_stays_on_launcher(
            0,
            crate::state::READY,
            0,
            3000,
            0
        ));
        assert!(!rocket_stays_on_launcher(
            0, reload, 2500, 3000, 1000
        ));
        assert!(rocket_stays_on_launcher(
            0, reload, 1500, 3000, 1000
        ));
    }
}
