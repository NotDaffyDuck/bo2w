//! Which draws a dynamic light can touch, and the light-type table the lit
//! technique choice reads.

use lighting_t6::brief_lamp_hits_box;
use render_frame::{HeldDrawItem, HeldDrawKind};
use render_scene::{VisualStage, FIRST_PERSON_WEAPON_SLOT};

use super::FrameBuildFeed;
use crate::prepare::scene::world::LevelStage;

/// Light type for every primary light index: the map's own lights first
/// (padded with "none" if the map table is short), then any lights added at
/// run time.
pub(crate) fn combined_light_types(map_kinds: &[u8], inputs: &FrameBuildFeed) -> Vec<u8> {
    let mut types = map_kinds.to_vec();
    types.resize(types.len().max(inputs.map_light_tally), 0);
    let extra = inputs.key_lights.iter().skip(inputs.map_light_tally);
    types.extend(extra.map(|light| light.kind));
    types
}

/// Bounds of a posed model or animated object in the scene, whichever the
/// entity is.
fn pose_bounds(gfx: &VisualStage, entnum: u32) -> Option<dpvs_t6::Bounds> {
    let from_dobj = gfx.stage_puppet(entnum).and_then(|dobj| dobj.pose_bounds);
    from_dobj.or_else(|| gfx.model_for(entnum).and_then(|m| m.pose_bounds))
}

/// Does a sphere of light reach this box?
fn light_reaches(origin: [f32; 3], rad: f32, bbox: &dpvs_t6::Bounds) -> bool {
    brief_lamp_hits_box(origin, rad, bbox.mid(), bbox.half())
}

/// Whether a draw is lit by the dynamic light at `origin` with `radius`:
/// true when the box around what it draws is within the light's reach. World
/// runs are tested member by member, static models by their placement box,
/// and scene models by their posed box (the viewmodel is never lit this
/// way). Everything else is not a receiver.
pub(crate) fn dlight_receivers_kept(
    draw_item: &HeldDrawItem,
    pos: [f32; 3],
    rad: f32,
    scene: Option<&LevelStage>,
    gfx: Option<&VisualStage>,
    level_run_facets: &[u16],
) -> bool {
    let dpvs = || {
        scene
            .and_then(|scene| scene.culling.as_ref())
            .map(|cull| &cull.dpvs)
    };
    match draw_item.kind {
        HeldDrawKind::World {
            surf, run, run_start_offset, ..
        } => {
            let Some(dpvs) = dpvs() else {
                return false;
            };
            let member_at = |step: u16| {
                if run <= 1 {
                    Some(surf)
                } else {
                    level_run_facets
                        .get(run_start_offset as usize + usize::from(step))
                        .copied()
                }
            };
            (0..run.max(1)).any(|step| {
                member_at(step)
                    .and_then(|member| dpvs.facet_bounds.get(usize::from(member)))
                    .is_some_and(|bounds| light_reaches(pos, rad, bounds))
            })
        }
        HeldDrawKind::Smodel {
            placement,
            stream: Some(_),
            ..
        } => dpvs()
            .and_then(|dpvs| dpvs.prop_bounds.get(placement as usize))
            .is_some_and(|bounds| light_reaches(pos, rad, bounds)),
        HeldDrawKind::XModel {
            stage_actor_num: Some(entnum),
            ..
        } if entnum != FIRST_PERSON_WEAPON_SLOT => gfx
            .and_then(|gfx| pose_bounds(gfx, entnum))
            .is_some_and(|bounds| light_reaches(pos, rad, &bounds)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Vec3};
    use render_frame::LevelViewLane;

    fn pack(lamp_type: u8) -> lighting_t6::LampRecord {
        let lookup_first = 0;
        let exponent = 0;
        lighting_t6::LampRecord {
            kind: lamp_type,
            color: Vec3::ONE.to_array(),
            direction: Vec3::NEG_Z.to_array(),
            origin: Vec3::ZERO.to_array(),
            radius: 100.0,
            outer_cone_cosine: 0.0,
            inner_cone_cosine: 0.0,
            falloff: exponent,
            falloff_map_width: None,
            lightmap_first: lookup_first,
        }
    }

    fn draw(kind: HeldDrawKind) -> HeldDrawItem {
        let material_id = None;
        HeldDrawItem {
            material_key: material_id,
            key: 0,
            material_order_no: 0,
            kind,
            facet_samplers: Default::default(),
            camera_zone: None,
        }
    }

    fn xmodel(stage_actor_num: Option<u32>) -> HeldDrawItem {
        let material = 0;
        draw(HeldDrawKind::XModel {
            surface: 0,
            material,
            object_tag: 0,
            model_to_world: Mat4::default(),
            illum_handle: 0,
            compact_illum: None,
            is_scoped_view: false,
            stage_actor_num,
        })
    }

    #[test]
    fn a_short_map_table_is_padded_with_no_light() {
        let inputs = FrameBuildFeed {
            map_light_tally: 4,
            ..Default::default()
        };
        assert_eq!(combined_light_types(&[1, 2], &inputs), vec![1, 2, 0, 0]);
    }

    #[test]
    fn run_time_lights_follow_the_map_lights() {
        let inputs = FrameBuildFeed {
            map_light_tally: 2,
            key_lights: vec![pack(9), pack(9), pack(4), pack(5)],
            ..Default::default()
        };
        assert_eq!(combined_light_types(&[1, 2], &inputs), vec![1, 2, 4, 5]);
    }

    #[test]
    fn a_long_map_table_is_left_whole() {
        let inputs = FrameBuildFeed::default();
        assert_eq!(combined_light_types(&[3, 3, 3], &inputs), vec![3, 3, 3]);
    }

    #[test]
    fn world_draws_need_a_culled_scene() {
        let item = draw(HeldDrawKind::World {
            surf: 0,
            run: 1,
            run_start_offset: 0,
            world_tree_camera_kind: Some(LevelViewLane::LitOpaque),
            level_tree_run_first: None,
            model_to_world: Mat4::default(),
        });
        assert!(!dlight_receivers_kept(
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
        let gfx = VisualStage::default();
        for item in [xmodel(None), xmodel(Some(FIRST_PERSON_WEAPON_SLOT))] {
            assert!(!dlight_receivers_kept(
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
        let gfx = VisualStage::default();
        assert!(!dlight_receivers_kept(
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
        let arg_count = 0;
        let material = 0;
        let item = draw(HeldDrawKind::CodeMesh {
            gun_view: false,
            draw: 0,
            material,
            n_args: arg_count,
            arg_rows: [[0.0; 4]; 2],
        });
        assert!(!dlight_receivers_kept(
            &item,
            [0.0; 3],
            50.0,
            None,
            None,
            &[]
        ));
    }
}
