use lighting_t6::{
    GFX_LIGHT_TYPE_SPOT, SM_SPOT_QUALITY_DEFAULT, SpotShadowChooseDvars, SpotShadowableLight,
    set_used_bit,
};

use super::super::tess::xmodel::XModelSurfaceDraw;
use crate::prepare::scene::world::WorldScene;

/// The light selection settings the host runs with.
pub(super) fn choose_dvars() -> SpotShadowChooseDvars {
    SpotShadowChooseDvars {
        spot_enable: true,
        max_lights: 4,
        fade_time: 1.0,
        eye_project_dist: 0.0,
        spot_project_frac: 0.0,
        spot_quality: SM_SPOT_QUALITY_DEFAULT,
    }
}

/// Every primary light of the world as a shadow candidate. Only spot lights
/// may take a shadow map. The distance cull and the shadow volume test have
/// no data behind them yet, so no light is culled by distance and every view
/// counts as inside a volume.
pub(super) fn candidates(world: &WorldScene) -> Vec<SpotShadowableLight> {
    let count = world.primary_light_cull.len().max(world.primary_light_pack.len());
    (0..count)
        .map(|at| {
            let cull = world.primary_light_cull.get(at);
            let pack = world.primary_light_pack.get(at);
            let light_type = match (cull, pack) {
                (Some(cull), _) => cull.light_type,
                (None, Some(pack)) => pack.light_type,
                (None, None) => 0,
            };
            SpotShadowableLight {
                can_use_shadow_map: light_type == GFX_LIGHT_TYPE_SPOT,
                color: pack.map_or([1.0; 3], |pack| pack.color),
                dir: cull.map_or([0.0, 0.0, 1.0], |cull| cull.direction),
                origin: cull.map_or([0.0; 3], |cull| cull.origin),
                radius: cull.map_or(0.0, |cull| cull.radius),
                light_type,
                cos_half_fov_expanded: cull.map_or(0.0, |cull| cull.cos_half_fov_expanded),
                cull_dist: i16::MAX,
                view_outside_shadowmap_volume: false,
            }
        })
        .collect()
}

/// The bit set of lights that matter this frame: those lighting an animated
/// model surface, and those with geometry of their own to cast.
pub(super) fn lights_in_use(world: &WorldScene, animated: &[XModelSurfaceDraw]) -> [u32; 8] {
    let mut used = [0u32; 8];
    for draw in animated {
        set_used_bit(&mut used, u32::from(draw.scene_light_index));
    }
    for (at, geometry) in world.shadow_geometry.iter().enumerate() {
        if !(geometry.surfaces.is_empty() && geometry.smodels.is_empty()) {
            set_used_bit(&mut used, at as u32);
        }
    }
    used
}
