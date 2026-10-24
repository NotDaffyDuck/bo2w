//! Static models into a sun-shadow partition (R_AddAllStaticModelSurfacesRangeSunShadow):
//! which models cast, and the inline cull-distance test.

/// GfxStaticModelDrawInst.flags bit 0: the model casts no shadow.
pub const SMODEL_FLAG_NO_CAST_SHADOW: u8 = 0x01;

/// flags bit 1: such a model also needs r_drawSModelsLV (default on). What else the bit
/// means is not read.
pub const SMODEL_FLAG_DRAW_LV: u8 = 0x02;

/// primaryLightIndex of the sun; only sun-lit models cast into the sun shadow.
pub const SUN_PRIMARY_LIGHT: u8 = 1;

/// The fields of one GfxStaticModelDrawInst the sun-shadow add reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SmodelShadowInst {
    /// cullDist, a float in BO2.
    pub cull_dist: f32,
    /// placement.origin.
    pub origin: [f32; 3],
    pub flags: u8,
    pub primary_light_index: u8,
}

/// rg.correctedLodParms[view]: the LOD origin and ramp[0]'s scale and bias.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunShadowLodParms {
    pub origin: [f32; 3],
    pub scale: f32,
    pub bias: f32,
}

impl SunShadowLodParms {
    /// |origin - lod origin| * scale + bias, the distance the cull test reads.
    #[must_use]
    pub fn adjusted_dist(&self, origin: [f32; 3]) -> f32 {
        let d = [
            origin[0] - self.origin[0],
            origin[1] - self.origin[1],
            origin[2] - self.origin[2],
        ];
        libm::sqrtf(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * self.scale + self.bias
    }
}

/// BO2 keeps a model while its LOD-ramped distance is below its cullDist. What a cullDist of
/// 0 means in BO2 is not read (no zero case was seen), so 0 keeps MW2's "never culled".
/// TODO(fact): cullDist 0.
#[must_use]
pub fn smodel_shadow_culled(cull_dist: f32, adjusted_dist: f32) -> bool {
    cull_dist != 0.0 && !(adjusted_dist < cull_dist)
}

/// The models of `insts` that cast into this sun-shadow partition, written to `out` by index;
/// returns how many. A model casts when its `vis` byte (smodelVisData[view + 1]) is set, flags
/// bit 0 is clear, its primary light is the sun, it has r_drawSModelsLV when flags bit 1 is
/// set (`draw_lv`), and, with `lod` given, its ramped distance is inside its cullDist. Ids are
/// u16 like the callers' id lists: models past 65535 are left out.
pub fn add_smodel_range_sun_shadow(
    vis: &[u8],
    insts: &[SmodelShadowInst],
    lod: Option<SunShadowLodParms>,
    draw_lv: bool,
    out: &mut [u16],
) -> usize {
    let mut written = 0usize;
    for (i, (&v, inst)) in vis.iter().zip(insts).enumerate() {
        if written == out.len() {
            break;
        }
        let Ok(id) = u16::try_from(i) else {
            break;
        };
        if v == 0
            || inst.flags & SMODEL_FLAG_NO_CAST_SHADOW != 0
            || inst.primary_light_index != SUN_PRIMARY_LIGHT
            || (inst.flags & SMODEL_FLAG_DRAW_LV != 0 && !draw_lv)
        {
            continue;
        }
        if let Some(lod) = lod
            && smodel_shadow_culled(inst.cull_dist, lod.adjusted_dist(inst.origin))
        {
            continue;
        }
        out[written] = id;
        written += 1;
    }
    written
}

/// MW2's static-model cull-distance rule (cull^2 <= dist^2, the distance scaled by the LOD
/// scale when one is given; 0 never culls), for the camera and spot-shadow passes. BO2's
/// rule for those passes is not read: BO2 unknown, TODO(fact).
#[must_use]
pub fn smodel_cull_dist_hides(cull_dist: u16, dist_sq: f32, lod_scale: Option<f32>) -> bool {
    if cull_dist == 0 {
        return false;
    }
    let cull = f32::from(cull_dist);
    let scaled_sq = match lod_scale {
        Some(s) => dist_sq * s * s,
        None => dist_sq,
    };
    cull * cull <= scaled_sq
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(x: f32, cull_dist: f32, flags: u8, light: u8) -> SmodelShadowInst {
        SmodelShadowInst {
            cull_dist,
            origin: [x, 0.0, 0.0],
            flags,
            primary_light_index: light,
        }
    }

    #[test]
    fn only_visible_sun_lit_casters_inside_their_cull_dist() {
        let insts = [
            inst(10.0, 100.0, 0, 1),
            inst(10.0, 100.0, SMODEL_FLAG_NO_CAST_SHADOW, 1),
            inst(10.0, 100.0, 0, 2),
            inst(10.0, 100.0, 0, 1),
            inst(200.0, 100.0, 0, 1),
            inst(200.0, 0.0, 0, 1),
            inst(10.0, 100.0, SMODEL_FLAG_DRAW_LV, 1),
            inst(10.0, 100.0, 0x10, 1),
        ];
        let vis = [1, 1, 1, 0, 1, 1, 1, 1];
        let lod = SunShadowLodParms {
            origin: [0.0; 3],
            scale: 1.0,
            bias: 0.0,
        };
        let mut out = [0u16; 8];
        let n = add_smodel_range_sun_shadow(&vis, &insts, Some(lod), true, &mut out);
        assert_eq!(&out[..n], [0, 5, 6, 7]);
        let n = add_smodel_range_sun_shadow(&vis, &insts, Some(lod), false, &mut out);
        assert_eq!(&out[..n], [0, 5, 7]);
        let n = add_smodel_range_sun_shadow(&vis, &insts, None, false, &mut out);
        assert_eq!(&out[..n], [0, 4, 5, 7]);
    }

    #[test]
    fn cull_compares_the_ramped_distance_linearly() {
        let lod = SunShadowLodParms {
            origin: [0.0, 3.0, 0.0],
            scale: 2.0,
            bias: 5.0,
        };
        // |(4, 0, 0) - (0, 3, 0)| = 5, ramped to 15.
        assert_eq!(lod.adjusted_dist([4.0, 0.0, 0.0]), 15.0);
        assert!(!smodel_shadow_culled(15.5, 15.0));
        assert!(smodel_shadow_culled(15.0, 15.0));
        assert!(!smodel_shadow_culled(0.0, 1.0e9));
    }

    #[test]
    fn mw2_rule_squares_and_scales() {
        assert!(!smodel_cull_dist_hides(0, 1.0e9, None));
        assert!(smodel_cull_dist_hides(10, 100.0, None));
        assert!(!smodel_cull_dist_hides(10, 99.0, None));
        assert!(smodel_cull_dist_hides(10, 25.0, Some(2.0)));
    }
}
