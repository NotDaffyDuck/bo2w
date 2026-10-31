//! Where the sight-raise and sight-lower anims sit for how far the sights are up.

/// The sight anims scrubbed to the sights' height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeOverlayTimeline {
    /// How far (0..1) through the raise anim.
    pub scope_raise_progress: f32,
    /// How far (0..1) through the lower anim.
    pub scope_lower_progress: f32,
    /// The raise anim's weight.
    pub scope_raise_blend: f32,
}

/// The scrub for sights `gun_raise_share` of the way up (held to 0..1).
pub fn ads_overlay_scrub(gun_raise_share: f32) -> ScopeOverlayTimeline {
    let up = gun_raise_share.clamp(0.0, 1.0);
    ScopeOverlayTimeline {
        scope_raise_progress: up,
        scope_lower_progress: 1.0 - up,
        scope_raise_blend: up,
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
                half.scope_raise_progress,
                half.scope_lower_progress,
                half.scope_raise_blend
            ),
            (0.25, 0.75, 0.25)
        );
        assert_eq!(ads_overlay_scrub(-1.0).scope_raise_blend, 0.0);
        assert_eq!(ads_overlay_scrub(2.0).scope_lower_progress, 0.0);
    }
}
