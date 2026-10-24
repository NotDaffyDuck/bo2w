//! A weapon file's fire type, and when a launcher's rocket shows on the viewmodel.

/// A fire type number with no fire type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownFireType(pub i32);

/// How the trigger fires: held down, once a pull, or a burst of 2 to 4 a pull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireType {
    FullAuto,
    SingleShot,
    BurstFire2,
    BurstFire3,
    BurstFire4,
}

impl FireType {
    const ALL: [Self; 5] = [
        Self::FullAuto,
        Self::SingleShot,
        Self::BurstFire2,
        Self::BurstFire3,
        Self::BurstFire4,
    ];

    /// The fire type a weapon file numbers `value` (0 full auto, 1 single, 2 to 4 burst).
    pub fn from_i32(value: i32) -> Result<Self, UnknownFireType> {
        usize::try_from(value)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .ok_or(UnknownFireType(value))
    }

    /// Fires a burst a pull.
    pub fn is_burst(self) -> bool {
        matches!(self, Self::BurstFire2 | Self::BurstFire3 | Self::BurstFire4)
    }
}

/// A launcher's rocket shows on the viewmodel while one is loaded, and during a reload
/// once more than `reload_show_rocket_time_ms` of it has run.
#[must_use]
pub fn viewmodel_rocket_should_be_attached(
    clip: i32,
    weaponstate: i32,
    weapon_time: i32,
    reload_time_ms: i32,
    reload_show_rocket_time_ms: i32,
) -> bool {
    let reloading = crate::state::is_any_reload(weaponstate);
    clip > 0 || reloading && reload_time_ms.wrapping_sub(weapon_time) > reload_show_rocket_time_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_types_number_from_full_auto() {
        assert_eq!(FireType::from_i32(0), Ok(FireType::FullAuto));
        assert_eq!(FireType::from_i32(3), Ok(FireType::BurstFire3));
        assert_eq!(FireType::from_i32(5), Err(UnknownFireType(5)));
        assert_eq!(FireType::from_i32(-1), Err(UnknownFireType(-1)));
        assert!(FireType::BurstFire2.is_burst());
        assert!(!FireType::SingleShot.is_burst());
    }

    #[test]
    fn the_rocket_shows_late_in_a_reload() {
        let reload = crate::state::RELOADING;
        assert!(viewmodel_rocket_should_be_attached(
            1,
            crate::state::READY,
            0,
            0,
            0
        ));
        assert!(!viewmodel_rocket_should_be_attached(
            0,
            crate::state::READY,
            0,
            3000,
            0
        ));
        assert!(!viewmodel_rocket_should_be_attached(
            0, reload, 2500, 3000, 1000
        ));
        assert!(viewmodel_rocket_should_be_attached(
            0, reload, 1500, 3000, 1000
        ));
    }
}
