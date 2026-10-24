//! Depth bands. The world uses almost all of the depth buffer; things drawn in
//! the first person (the viewmodel and anything flagged with the depth hack)
//! are squeezed into a thin slice nearest the camera so they never poke into
//! walls.

use super::retained::RetainedDrawKind;
use super::tess::xmodel::XMODEL_OBJECT_ID_VIEWMODEL;

/// Normal scene depth.
pub const GFX_DEPTH_RANGE_SCENE: i32 = 0;
/// The thin near slice used by first-person models.
pub const GFX_DEPTH_RANGE_VIEWMODEL: i32 = 2;
/// The whole buffer, for full screen passes.
pub const GFX_DEPTH_RANGE_FULL: i32 = -1;

/// Width of the near slice (1/64 of the buffer).
pub const DEPTH_RANGE_BAND: f32 = 0.015_625;

/// Effect flag bit that asks for the near slice.
pub const RENDER_FX_DEPTH_HACK: u32 = 2;

/// Near and far depth for a band in conventional depth (0 near, 1 far).
/// The scene band starts where the near slice ends; every other band is the
/// near slice itself.
pub fn d3d_depth_range(depth_range_type: i32) -> (f32, f32) {
    match depth_range_type {
        GFX_DEPTH_RANGE_SCENE => (DEPTH_RANGE_BAND, 1.0),
        _ => (0.0, DEPTH_RANGE_BAND),
    }
}

/// The same band flipped for the reversed-z buffer, as an ordered
/// `(min, max)` viewport pair.
pub fn reverse_z_viewport_depth(depth_range_type: i32) -> (f32, f32) {
    let (near, far) = d3d_depth_range(depth_range_type);
    let (flipped_far, flipped_near) = (1.0 - far, 1.0 - near);
    (flipped_far.min(flipped_near), flipped_far.max(flipped_near))
}

/// Which band an entity's effect flags select.
pub fn depth_range_type_from_hack_flags(render_fx_flags: u32) -> i32 {
    match render_fx_flags & RENDER_FX_DEPTH_HACK {
        0 => GFX_DEPTH_RANGE_SCENE,
        _ => GFX_DEPTH_RANGE_VIEWMODEL,
    }
}

/// The effect flags the host gives a model, judged from its object id alone:
/// only the viewmodel carries the depth hack.
pub fn host_viewmodel_render_fx_flags(object_id: u16) -> u32 {
    if object_id == XMODEL_OBJECT_ID_VIEWMODEL {
        RENDER_FX_DEPTH_HACK
    } else {
        0
    }
}

/// Which band a retained draw belongs in, given its sort key.
pub fn depth_range_type_for_draw(kind: &RetainedDrawKind, key: u64) -> i32 {
    match kind {
        RetainedDrawKind::XModel { .. } => {
            let object_id = dpvs_t6::GfxDrawSurf { packed: key }.object_id();
            depth_range_type_from_hack_flags(host_viewmodel_render_fx_flags(object_id))
        }
        RetainedDrawKind::CodeMesh { viewmodel, .. } if *viewmodel => GFX_DEPTH_RANGE_VIEWMODEL,
        _ => GFX_DEPTH_RANGE_SCENE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_mesh(viewmodel: bool) -> RetainedDrawKind {
        RetainedDrawKind::CodeMesh {
            viewmodel,
            draw: 0,
            material: 0,
            arg_count: 0,
            args: [[0.0; 4]; 2],
        }
    }

    #[test]
    fn the_scene_band_starts_after_the_near_slice() {
        assert_eq!(
            d3d_depth_range(GFX_DEPTH_RANGE_SCENE),
            (DEPTH_RANGE_BAND, 1.0)
        );
    }

    #[test]
    fn every_other_band_is_the_near_slice() {
        for band in [GFX_DEPTH_RANGE_VIEWMODEL, GFX_DEPTH_RANGE_FULL, 7] {
            assert_eq!(d3d_depth_range(band), (0.0, DEPTH_RANGE_BAND));
        }
    }

    #[test]
    fn reversed_depth_is_ordered_and_mirrors_the_band() {
        let (lo, hi) = reverse_z_viewport_depth(GFX_DEPTH_RANGE_SCENE);
        assert_eq!(lo, 0.0);
        assert!((hi - (1.0 - DEPTH_RANGE_BAND)).abs() < 1e-6);
        let (lo, hi) = reverse_z_viewport_depth(GFX_DEPTH_RANGE_VIEWMODEL);
        assert!((lo - (1.0 - DEPTH_RANGE_BAND)).abs() < 1e-6);
        assert_eq!(hi, 1.0);
        assert!(lo < hi);
    }

    #[test]
    fn the_depth_hack_bit_picks_the_near_slice() {
        assert_eq!(depth_range_type_from_hack_flags(0), GFX_DEPTH_RANGE_SCENE);
        assert_eq!(depth_range_type_from_hack_flags(1), GFX_DEPTH_RANGE_SCENE);
        assert_eq!(
            depth_range_type_from_hack_flags(RENDER_FX_DEPTH_HACK),
            GFX_DEPTH_RANGE_VIEWMODEL
        );
        assert_eq!(
            depth_range_type_from_hack_flags(0xff),
            GFX_DEPTH_RANGE_VIEWMODEL
        );
    }

    #[test]
    fn only_the_viewmodel_object_carries_the_hack() {
        assert_eq!(
            host_viewmodel_render_fx_flags(XMODEL_OBJECT_ID_VIEWMODEL),
            RENDER_FX_DEPTH_HACK
        );
        assert_eq!(
            host_viewmodel_render_fx_flags(XMODEL_OBJECT_ID_VIEWMODEL.wrapping_add(1)),
            0
        );
    }

    #[test]
    fn code_meshes_follow_their_viewmodel_flag() {
        assert_eq!(
            depth_range_type_for_draw(&code_mesh(true), 0),
            GFX_DEPTH_RANGE_VIEWMODEL
        );
        assert_eq!(
            depth_range_type_for_draw(&code_mesh(false), 0),
            GFX_DEPTH_RANGE_SCENE
        );
    }
}
