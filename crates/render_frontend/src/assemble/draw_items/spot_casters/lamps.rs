use lighting_t6::CONE_LAMP;
use lighting_t6::SM_SPOT_QUALITY_DEFAULT;
use lighting_t6::ConeShadowChooseTunables;
use lighting_t6::ConeCasterOffer;
use lighting_t6::mark_light_used;

use super::super::mesh_build::figure_mesh::FigureFacetDraw;
use crate::prepare::scene::world::LevelStage;

/// The light selection settings the host runs with.
pub(super) fn choose_dvars() -> ConeShadowChooseTunables {
    let fade_time = 1.0;
    ConeShadowChooseTunables {
        cone_enable: true,
        light_budget: 4,
        fade_secs: fade_time,
        viewer_projection_reach: 0.0,
        cone_project_frac: 0.0,
        spot_quality: SM_SPOT_QUALITY_DEFAULT,
    }
}

/// Every primary light of the world as a shadow candidate. Only spot lights
/// may take a shadow map. The distance cull and the shadow volume test have
/// no data behind them yet, so no light is culled by distance and every view
/// counts as inside a volume.
pub(super) fn candidates(world: &LevelStage) -> Vec<ConeCasterOffer> {
    let count = world.key_light_hide.len().max(world.key_light_pack.len());
    (0..count)
        .map(|at| {
            let cull = world.key_light_hide.get(at);
            let pack = world.key_light_pack.get(at);
            let lamp_type = match (cull, pack) {
                (Some(cull), _) => cull.kind,
                (None, Some(pack)) => pack.kind,
                (None, None) => 0,
            };
            ConeCasterOffer {
                supports_depth_shade: lamp_type == CONE_LAMP,
                color: pack.map_or([1.0; 3], |pack| pack.color),
                aim: cull.map_or([0.0, 0.0, 1.0], |cull| cull.direction),
                origin: cull.map_or([0.0; 3], |cull| cull.origin),
                radius: cull.map_or(0.0, |cull| cull.radius),
                kind: lamp_type,
                widened_half_view_cosine: cull.map_or(0.0, |cull| cull.widened_half_view_cosine),
                hide_dist: i16::MAX,
                view_outside_shadowmap_volume: false,
            }
        })
        .collect()
}

/// The bit set of lights that matter this frame: those lighting an animated
/// model surface, and those with geometry of their own to cast.
pub(super) fn lights_in_use(world: &LevelStage, animated: &[FigureFacetDraw]) -> [u32; 8] {
    let mut lit = [0u32; 8];
    for draw in animated {
        mark_light_used(&mut lit, u32::from(draw.stage_light_index));
    }
    for (at, geometry) in world.shadow_geometry.iter().enumerate() {
        if !(geometry.surface_total.is_empty() && geometry.fixed_props.is_empty()) {
            mark_light_used(&mut lit, at as u32);
        }
    }
    lit
}
