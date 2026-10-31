//! Sun shadow caster lists: which world surfaces each shadow partition draws.

use dpvs_t6::SortedDraw;
use dpvs_t6::add_bsp_draw_surfs_range_sun_shadow;
use render_frame::{DaylightShadowCasterLists, SunCascadeSliceLists};

/// Appends to `out` the surfaces in `begin..end` that are visible in the
/// partition and cast shadows.
pub fn add_level_tree_daylight_shadow_partition(
    begin: u32,
    end: u32,
    facet_vis: &[u8],
    caster_bitwords: &[u32],
    facet_materials: &[SortedDraw],
    out: &mut Vec<u16>,
) {
    add_bsp_draw_surfs_range_sun_shadow(begin, end, facet_vis, caster_bitwords, facet_materials, out);
}

/// Pairs each of the two sun partitions' visibility with its list of world
/// surfaces to draw into the shadow map.
pub fn pack_daylight_shadow_caster_lists(
    vis_bits: [Vec<u8>; 2],
    prop_vis: [Vec<u8>; 2],
    begin: u32,
    end: u32,
    caster_bitwords: &[u32],
    facet_materials: &[SortedDraw],
) -> DaylightShadowCasterLists {
    let pack = |facet_vis: Vec<u8>, prop_vis: Vec<u8>| {
        let mut world_surfs = Vec::new();
        add_level_tree_daylight_shadow_partition(
            begin,
            end,
            &facet_vis,
            caster_bitwords,
            facet_materials,
            &mut world_surfs,
        );
        SunCascadeSliceLists {
            facet_vis,
            prop_vis,
            level_faces: world_surfs,
        }
    };
    let ([near_vis, far_vis], [near_smodels, far_smodels]) = (vis_bits, prop_vis);
    DaylightShadowCasterLists {
        partitions: [pack(near_vis, near_smodels), pack(far_vis, far_smodels)],
    }
}
