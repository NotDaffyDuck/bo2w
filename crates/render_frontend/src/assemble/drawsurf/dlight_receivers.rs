//! Which draws a dynamic light can touch, and the light-type table the lit
//! technique choice reads.

use lighting_t6::dlight_hits_aabb;
use render_frame::{RetainedDrawItem, RetainedDrawKind};
use render_scene::{GfxScene, SCENE_VIEWMODEL_ENTNUM};

use super::FrameAssemblyInputs;
use crate::prepare::scene::world::WorldScene;

/// Light type for every primary light index: the map's own lights first
/// (padded with "none" if the map table is short), then any lights added at
/// run time.
pub(crate) fn combined_light_types(map_types: &[u8], inputs: &FrameAssemblyInputs) -> Vec<u8> {
    let mut types = map_types.to_vec();
    types.resize(types.len().max(inputs.map_light_n), 0);
    let extra = inputs.primary_lights.iter().skip(inputs.map_light_n);
    types.extend(extra.map(|light| light.light_type));
    types
}

/// Bounds of a posed model or animated object in the scene, whichever the
/// entity is.
fn posed_bounds(gfx: &GfxScene, entnum: u32) -> Option<dpvs_t6::Bounds> {
    let from_dobj = gfx.scene_dobj(entnum).and_then(|dobj| dobj.posed_bounds);
    from_dobj.or_else(|| gfx.scene_model(entnum).and_then(|m| m.posed_bounds))
}

/// Does a sphere of light reach this box?
fn light_reaches(origin: [f32; 3], radius: f32, bounds: &dpvs_t6::Bounds) -> bool {
    dlight_hits_aabb(origin, radius, bounds.mid(), bounds.half())
}

/// Whether a draw is lit by the dynamic light at `origin` with `radius`:
/// true when the box around what it draws is within the light's reach. World
/// runs are tested member by member, static models by their placement box,
/// and scene models by their posed box (the viewmodel is never lit this
/// way). Everything else is not a receiver.
pub(crate) fn dlight_receiver_keep(
    draw: &RetainedDrawItem,
    origin: [f32; 3],
    radius: f32,
    scene: Option<&WorldScene>,
    gfx: Option<&GfxScene>,
    world_run_surfs: &[u16],
) -> bool {
    let dpvs = || {
        scene
            .and_then(|scene| scene.cull.as_ref())
            .map(|cull| &cull.dpvs)
    };
    match draw.kind {
        RetainedDrawKind::World {
            surf, run, run_off, ..
        } => {
            let Some(dpvs) = dpvs() else {
                return false;
            };
            let member_at = |step: u16| {
                if run <= 1 {
                    Some(surf)
                } else {
                    world_run_surfs
                        .get(run_off as usize + usize::from(step))
                        .copied()
                }
            };
            (0..run.max(1)).any(|step| {
                member_at(step)
                    .and_then(|member| dpvs.surface_bounds.get(usize::from(member)))
                    .is_some_and(|bounds| light_reaches(origin, radius, bounds))
            })
        }
        RetainedDrawKind::Smodel {
            placement,
            stream: Some(_),
            ..
        } => dpvs()
            .and_then(|dpvs| dpvs.smodel_bounds.get(placement as usize))
            .is_some_and(|bounds| light_reaches(origin, radius, bounds)),
        RetainedDrawKind::XModel {
            scene_entnum: Some(entnum),
            ..
        } if entnum != SCENE_VIEWMODEL_ENTNUM => gfx
            .and_then(|gfx| posed_bounds(gfx, entnum))
            .is_some_and(|bounds| light_reaches(origin, radius, &bounds)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Mat4;
    use render_frame::BspCameraLane;

    fn pack(light_type: u8) -> lighting_t6::GfxLightPack {
        lighting_t6::GfxLightPack {
            light_type,
            color: [1.0; 3],
            direction: [0.0, 0.0, -1.0],
            origin: [0.0; 3],
            radius: 100.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            exponent: 0,
            falloff_image_width: None,
            lmap_lookup_start: 0,
        }
    }

    fn draw(kind: RetainedDrawKind) -> RetainedDrawItem {
        RetainedDrawItem {
            material_id: None,
            key: 0,
            material_rank: 0,
            kind,
            surface_samplers: Default::default(),
            camera_region: None,
        }
    }

    fn xmodel(scene_entnum: Option<u32>) -> RetainedDrawItem {
        draw(RetainedDrawKind::XModel {
            surface: 0,
            material: 0,
            object_id: 0,
            world_from_local: Mat4::IDENTITY,
            lighting_handle: 0,
            packed_lighting: None,
            is_scope: false,
            scene_entnum,
        })
    }

    #[test]
    fn a_short_map_table_is_padded_with_no_light() {
        let inputs = FrameAssemblyInputs {
            map_light_n: 4,
            ..Default::default()
        };
        assert_eq!(combined_light_types(&[1, 2], &inputs), vec![1, 2, 0, 0]);
    }

    #[test]
    fn run_time_lights_follow_the_map_lights() {
        let inputs = FrameAssemblyInputs {
            map_light_n: 2,
            primary_lights: vec![pack(9), pack(9), pack(4), pack(5)],
            ..Default::default()
        };
        assert_eq!(combined_light_types(&[1, 2], &inputs), vec![1, 2, 4, 5]);
    }

    #[test]
    fn a_long_map_table_is_left_whole() {
        let inputs = FrameAssemblyInputs::default();
        assert_eq!(combined_light_types(&[3, 3, 3], &inputs), vec![3, 3, 3]);
    }

    #[test]
    fn world_draws_need_a_culled_scene() {
        let item = draw(RetainedDrawKind::World {
            surf: 0,
            run: 1,
            run_off: 0,
            bsp_kind: Some(BspCameraLane::LitOpaque),
            bsp_run_first: None,
            world_from_local: Mat4::IDENTITY,
        });
        assert!(!dlight_receiver_keep(
            &item,
            [0.0; 3],
            50.0,
            None,
            None,
            &[]
        ));
    }

    #[test]
    fn the_viewmodel_and_unplaced_models_are_never_receivers() {
        let gfx = GfxScene::default();
        for item in [xmodel(None), xmodel(Some(SCENE_VIEWMODEL_ENTNUM))] {
            assert!(!dlight_receiver_keep(
                &item,
                [0.0; 3],
                50.0,
                None,
                Some(&gfx),
                &[]
            ));
        }
    }

    #[test]
    fn a_model_the_scene_does_not_know_is_not_a_receiver() {
        let gfx = GfxScene::default();
        assert!(!dlight_receiver_keep(
            &xmodel(Some(12)),
            [0.0; 3],
            50.0,
            None,
            Some(&gfx),
            &[]
        ));
    }

    #[test]
    fn code_meshes_are_not_receivers() {
        let item = draw(RetainedDrawKind::CodeMesh {
            viewmodel: false,
            draw: 0,
            material: 0,
            arg_count: 0,
            args: [[0.0; 4]; 2],
        });
        assert!(!dlight_receiver_keep(
            &item,
            [0.0; 3],
            50.0,
            None,
            None,
            &[]
        ));
    }
}
