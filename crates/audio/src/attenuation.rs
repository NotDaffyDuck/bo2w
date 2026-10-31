//! Distance falloff for sounds. A curve is a short list of `[position, gain]`
//! knots, position running 0..1 across the span from a sound's near distance
//! to its far distance.

/// A BO2 falloff curve holds eight `[x, y]` points and no count.
pub(crate) const CURVE_KNOT_CAP: usize = 8;

/// Gain read off the curve at `pos`; -1 once `pos` runs past the last knot or
/// when the curve has fewer than two knots. (The way BO2 walks its curve is
/// not known; this is a plain piecewise-linear read.)
pub(crate) fn curve_eval(knots: &[[f32; 2]], pos: f32) -> f32 {
    // The first span whose far end is not short of `pos` is the one read.
    knots
        .windows(2)
        .find(|span| !(span[1][0] < pos))
        .map_or(-1.0, |span| {
            let t = (pos - span[0][0]) / (span[1][0] - span[0][0]);
            span[0][1] + (span[1][1] - span[0][1]) * t
        })
}

/// Gain at `dist` for a sound that is full inside `near`, silent beyond
/// `far`, and follows the curve between.
pub(crate) fn attenuate(knots: &[[f32; 2]], dist: f32, near: f32, far: f32) -> f32 {
    let past_near = dist - near;
    if past_near <= 0.0 {
        return 1.0;
    }
    let pos = past_near / (far - near);
    if pos >= 1.0 {
        return 0.0;
    }
    curve_eval(knots, pos)
}

/// Gain for a sound that may carry a separate curve for the close-in part:
/// `near_curve` covers 0..`min`, `knots` covers `min`..`max`. Without a
/// close-in curve this is `attenuate`.
pub(crate) fn distance_attenuation(
    knots: &[[f32; 2]],
    near_curve: Option<&[[f32; 2]]>,
    dist: f32,
    min: f32,
    max: f32,
) -> f32 {
    let Some(near_curve) = near_curve else {
        return attenuate(knots, dist, min, max);
    };
    let (curve, pos) = if dist < min {
        (near_curve, dist / min)
    } else if max > min {
        (knots, (dist - min) / (max - min))
    } else {
        (knots, 1.0)
    };
    let pos = pos.clamp(0.0, 1.0);

    let (Some(first), Some(last)) = (curve.first(), curve.last()) else {
        return -1.0;
    };
    if pos <= first[0] {
        return first[1];
    }
    curve
        .windows(2)
        .find(|span| pos <= span[1][0] && span[1][0] > span[0][0])
        .map_or(last[1], |span| {
            let t = (pos - span[0][0]) / (span[1][0] - span[0][0]);
            span[0][1] + (span[1][1] - span[0][1]) * t
        })
}
