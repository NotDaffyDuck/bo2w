//! Per-map sun-shadow tuning.

/// What a map's client script leaves `sm_sunSampleSizeNear` at when it sets
/// none of its own.
const DEFAULT_SAMPLE_SIZE_NEAR: f32 = 0.5;

/// The world units per texel of the near sun-shadow partition for the maps
/// whose client scripts set their own, grouped by value.
const SAMPLE_SIZE_NEAR: &[(f32, &[&str])] = &[
    (0.2, &["mp_overflow"]),
    (0.25, &["mp_bridge", "mp_dig", "mp_frostbite", "mp_pod"]),
    (0.26, &["mp_castaway"]),
    (0.3, &["mp_studio", "mp_takeoff"]),
    (0.35, &["mp_hydro", "mp_paintball"]),
    (
        0.39,
        &[
            "mp_carrier",
            "mp_drone",
            "mp_express",
            "mp_la",
            "mp_mirage",
            "mp_nightclub",
            "mp_socotra",
            "mp_turbine",
        ],
    ),
    (0.45, &["mp_magma"]),
];

/// The near sun-shadow sample size for a zone, given by its map's name or by
/// a `source:map` pair.
#[must_use]
pub fn sun_sample_size_near(zone: &str) -> f32 {
    let map = zone.rsplit_once(':').map_or(zone, |(_, map)| map);
    SAMPLE_SIZE_NEAR
        .iter()
        .find(|(_, maps)| maps.contains(&map))
        .map_or(DEFAULT_SAMPLE_SIZE_NEAR, |&(size, _)| size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_with_its_own_value_gets_it() {
        assert_eq!(sun_sample_size_near("mp_overflow"), 0.2);
        assert_eq!(sun_sample_size_near("mp_nightclub"), 0.39);
    }

    #[test]
    fn the_map_name_is_what_follows_the_last_colon() {
        assert_eq!(sun_sample_size_near("dlc:mp_magma"), 0.45);
        assert_eq!(sun_sample_size_near("a:b:mp_castaway"), 0.26);
    }

    #[test]
    fn any_other_map_gets_the_default() {
        assert_eq!(sun_sample_size_near("mp_unlisted"), 0.5);
        assert_eq!(sun_sample_size_near(""), 0.5);
    }

    #[test]
    fn no_map_is_listed_twice() {
        let mut met_names = std::collections::HashSet::new();
        for (_, maps) in SAMPLE_SIZE_NEAR {
            for map in *maps {
                assert!(met_names.insert(*map), "{map} is listed twice");
            }
        }
    }
}
