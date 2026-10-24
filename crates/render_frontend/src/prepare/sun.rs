//! Sun shadow caster lists: which world surfaces each shadow partition draws.

use dpvs_t6::{GfxDrawSurf, add_bsp_draw_surfs_range_sun_shadow};
use render_frame::{SunShadowCasterLists, SunShadowPartitionLists};

/// Appends to `out` the surfaces in `begin..end` that are visible in the
/// partition and cast shadows.
pub fn add_bsp_sun_shadow_partition(
    begin: u32,
    end: u32,
    surface_vis: &[u8],
    caster_words: &[u32],
    surface_materials: &[GfxDrawSurf],
    out: &mut Vec<u16>,
) {
    add_bsp_draw_surfs_range_sun_shadow(begin, end, surface_vis, caster_words, surface_materials, out);
}

/// Pairs each of the two sun partitions' visibility with its list of world
/// surfaces to draw into the shadow map.
pub fn pack_sun_shadow_caster_lists(
    vis: [Vec<u8>; 2],
    smodel_vis: [Vec<u8>; 2],
    begin: u32,
    end: u32,
    caster_words: &[u32],
    surface_materials: &[GfxDrawSurf],
) -> SunShadowCasterLists {
    let pack = |surface_vis: Vec<u8>, smodel_vis: Vec<u8>| {
        let mut world_surfs = Vec::new();
        add_bsp_sun_shadow_partition(
            begin,
            end,
            &surface_vis,
            caster_words,
            surface_materials,
            &mut world_surfs,
        );
        SunShadowPartitionLists {
            surface_vis,
            smodel_vis,
            world_surfs,
        }
    };
    let ([near_vis, far_vis], [near_smodels, far_smodels]) = (vis, smodel_vis);
    SunShadowCasterLists {
        partitions: [pack(near_vis, near_smodels), pack(far_vis, far_smodels)],
    }
}
