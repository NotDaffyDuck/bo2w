//! BO2's aim assist numbers and the scope overlay fade.

/// aim_aimAssistRangeScale default (allowed 0 to 2).
pub const AIM_ASSIST_RANGE_SCALE: f32 = 1.0;

/// The slowdown's pitch scale at the hip and fully aimed (lerped by posFrac).
/// TODO(fact): the yaw pair was not read.
pub const SLOWDOWN_PITCH_SCALE: [f32; 2] = [0.4, 0.5];

/// The slowdown's turn rates, hip and fully aimed: pitch, then yaw.
pub const TURN_RATE_PITCH: [f32; 2] = [90.0, 55.0];
pub const TURN_RATE_YAW: [f32; 2] = [260.0, 90.0];

/// aim_slowdown_region_width / height.
pub const SLOWDOWN_REGION: [f32; 2] = [90.0, 90.0];

/// Below this zoom nothing of the scope overlay is drawn.
pub const OVERLAY_MIN_ZOOM: f32 = 0.01;

#[inline]
fn lerp(pair: [f32; 2], amount: f32) -> f32 {
    pair[0] + (pair[1] - pair[0]) * amount
}

/// AimAssist_GetAimAssistRange: the hip range blended into the ADS range by posFrac.
#[must_use]
pub fn aim_assist_range(hip: f32, ads: f32, pos_frac: f32, range_scale: f32) -> f32 {
    range_scale * (pos_frac * ads + (1.0 - pos_frac) * hip)
}

/// The slowdown pitch scale at this posFrac.
#[must_use]
pub fn slowdown_pitch_scale(pos_frac: f32) -> f32 {
    lerp(SLOWDOWN_PITCH_SCALE, pos_frac)
}

/// The slowdown turn rates (pitch, yaw) at this posFrac.
#[must_use]
pub fn turn_rates(pos_frac: f32) -> (f32, f32) {
    (lerp(TURN_RATE_PITCH, pos_frac), lerp(TURN_RATE_YAW, pos_frac))
}

/// CG_GetWeapReticleZoom: how far the scope overlay has faded in, or None when nothing is drawn.
/// `zoom_frac` is the zoom-in fraction while moving toward ADS, else the zoom-out one.
#[must_use]
pub fn reticle_zoom(pos_frac: f32, zoom_frac: f32) -> Option<f32> {
    if pos_frac == 0.0 {
        return None;
    }
    let mut zoom = pos_frac - (1.0 - zoom_frac);
    if zoom > 0.0 {
        zoom /= zoom_frac;
    }
    (zoom > OVERLAY_MIN_ZOOM).then_some(zoom.min(1.0))
}

/// The scope overlay's alpha (white) and the crosshair's alpha for a zoom.
#[must_use]
pub fn overlay_alphas(zoom: f32, overlay_alpha_scale: f32) -> (f32, f32) {
    let alpha = zoom / overlay_alpha_scale.max(1.0);
    (alpha, 1.0 - alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_blends_hip_into_ads() {
        assert_eq!(aim_assist_range(1000.0, 2000.0, 0.0, 1.0), 1000.0);
        assert_eq!(aim_assist_range(1000.0, 2000.0, 0.5, 1.0), 1500.0);
        assert_eq!(aim_assist_range(1000.0, 2000.0, 1.0, 2.0), 4000.0);
        assert!((slowdown_pitch_scale(0.5) - 0.45).abs() < 1e-6);
        assert_eq!(turn_rates(1.0), (55.0, 90.0));
    }

    #[test]
    fn the_overlay_fades_in_over_the_last_part_of_the_aim() {
        assert_eq!(reticle_zoom(0.0, 0.2), None);
        // Zoom fraction 0.2: nothing until posFrac passes 0.8, full at 1.
        assert_eq!(reticle_zoom(0.8, 0.2), None);
        assert!((reticle_zoom(0.9, 0.2).unwrap() - 0.5).abs() < 1e-5);
        // f32 (1 - 0.2) / 0.2 lands a hair under 1.
        assert!((reticle_zoom(1.0, 0.2).unwrap() - 1.0).abs() < 1e-5);
        let (overlay, crosshair) = overlay_alphas(0.5, 2.0);
        assert_eq!((overlay, crosshair), (0.25, 0.75));
        assert_eq!(overlay_alphas(0.5, 0.0), (0.5, 0.5));
    }
}
