//! BO2's model LOD pick for a distance (XModelGetLodForDist) and the distances it is
//! given: the base distance from the LOD origin, the ramped one (R_GetAdjustedLodDist) and
//! a static model's divide by its scale.

/// XModel lodInfo entries.
pub const XMODEL_MAX_LODS: usize = 4;

/// The first LOD whose distance is 0 (no limit) or above `dist`.
#[must_use]
fn first_lod_for_dist(num_lods: i16, lod_dist: [f32; XMODEL_MAX_LODS], dist: f32) -> Option<u8> {
    let n = i32::from(num_lods).clamp(0, XMODEL_MAX_LODS as i32) as usize;
    (0..n)
        .find(|&lod| lod_dist[lod] == 0.0 || dist < lod_dist[lod])
        .map(|lod| lod as u8)
}

/// XModelGetLodForDist: the ramped distance first, then the raw one. With nothing found,
/// a model that may not cull out by distance takes numLods - 2 (clamped 0..3); otherwise
/// it is culled (None).
#[must_use]
pub fn xmodel_get_lod_for_dist(
    num_lods: i16,
    lod_dist: [f32; XMODEL_MAX_LODS],
    adjusted_dist: f32,
    raw_dist: f32,
    no_cull_out: bool,
) -> Option<u8> {
    let mut lod = first_lod_for_dist(num_lods, lod_dist, adjusted_dist);
    if lod.is_none() && raw_dist != adjusted_dist {
        lod = first_lod_for_dist(num_lods, lod_dist, raw_dist);
    }
    match lod {
        Some(lod) => Some(lod),
        None if no_cull_out => Some((i32::from(num_lods) - 2).clamp(0, 3) as u8),
        None => None,
    }
}

/// R_GetBaseLodDist for the main camera: |lodOrigin - point|.
#[must_use]
pub fn lod_base_dist(lod_origin: [f32; 3], point: [f32; 3]) -> f32 {
    let d = [lod_origin[0] - point[0], lod_origin[1] - point[1], lod_origin[2] - point[2]];
    libm::sqrtf(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
}

/// R_GetAdjustedLodDist: ramp scale * dist + ramp bias.
#[must_use]
pub fn lod_adjusted_dist(dist: f32, scale: f32, bias: f32) -> f32 {
    scale * dist + bias
}

/// A static model's distance is divided by max(instance scale, 1).
#[must_use]
pub fn smodel_lod_dist(dist: f32, instance_scale: f32) -> f32 {
    dist / instance_scale.max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_dist_is_no_limit() {
        let dists = [100.0, 0.0, 500.0, 0.0];
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 50.0, 50.0, false), Some(0));
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 900.0, 900.0, false), Some(1));
    }

    #[test]
    fn raw_dist_then_cull_out() {
        let dists = [100.0, 200.0, 300.0, 0.0];
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 250.0, 250.0, false), Some(2));
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 400.0, 150.0, false), Some(1));
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 400.0, 400.0, false), None);
        assert_eq!(xmodel_get_lod_for_dist(3, dists, 400.0, 400.0, true), Some(1));
        assert_eq!(xmodel_get_lod_for_dist(1, dists, 400.0, 400.0, true), Some(0));
    }

    #[test]
    fn distances() {
        assert_eq!(lod_base_dist([3.0, 4.0, 0.0], [0.0; 3]), 5.0);
        assert_eq!(lod_adjusted_dist(10.0, 2.0, -1.0), 19.0);
        assert_eq!(smodel_lod_dist(10.0, 0.5), 10.0);
        assert_eq!(smodel_lod_dist(10.0, 2.0), 5.0);
    }
}
