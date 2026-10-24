//! Where the sight-raise and sight-lower anims sit for how far the sights are up.

/// The sight anims scrubbed to the sights' height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdsOverlayScrub {
    /// How far (0..1) through the raise anim.
    pub ads_up_time_norm: f32,
    /// How far (0..1) through the lower anim.
    pub ads_down_time_norm: f32,
    /// The raise anim's weight.
    pub ads_up_weight: f32,
}

/// The scrub for sights `weapon_pos_frac` of the way up (held to 0..1).
pub fn ads_overlay_scrub(weapon_pos_frac: f32) -> AdsOverlayScrub {
    let up = weapon_pos_frac.clamp(0.0, 1.0);
    AdsOverlayScrub {
        ads_up_time_norm: up,
        ads_down_time_norm: 1.0 - up,
        ads_up_weight: up,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raise_runs_up_as_the_lower_runs_down() {
        let half = ads_overlay_scrub(0.25);
        assert_eq!(
            (
                half.ads_up_time_norm,
                half.ads_down_time_norm,
                half.ads_up_weight
            ),
            (0.25, 0.75, 0.25)
        );
        assert_eq!(ads_overlay_scrub(-1.0).ads_up_weight, 0.0);
        assert_eq!(ads_overlay_scrub(2.0).ads_down_time_norm, 0.0);
    }
}
