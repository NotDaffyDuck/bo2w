//! The host's copy of the frame scene, the occupancy the spot-shadow pass reads
//! from it, and the per-object skin inputs that go with it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::prelude::*;
use lighting_t6::{
    SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS, SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ, SpotShadowSceneSlot,
    spot_shadow_entity_origin_track_index, spot_shadow_entity_should_relink,
};

use crate::{AddBModelArgs, AddBModelPose, AddDObjArgs, AddDObjPose, GfxScene, scene_info_entnum};

#[derive(Resource, Default)]
pub struct HostGfxScene {
    pub scene: GfxScene,
}

/// What the spot-shadow pass sees of the scene: one slot per object, with the box it was posed in.
#[derive(Resource, Default)]
pub struct SpotShadowSceneOccupancy {
    pub dobjs: Vec<SpotShadowSceneSlot>,
    pub models: Vec<SpotShadowSceneSlot>,
}

/// The origin each entity had when its spot shadows were last linked.
#[derive(Resource, Clone, Debug)]
pub struct SpotShadowEntityOriginTrack {
    pub last: Vec<[f32; 3]>,
}

impl Default for SpotShadowEntityOriginTrack {
    fn default() -> Self {
        Self { last: vec![[0.0; 3]; SPOT_SHADOW_ENTITY_ORIGIN_TRACK_ENTS as usize] }
    }
}

fn spot_slot(info: u32, bounds: Option<dpvs_t6::Bounds>) -> SpotShadowSceneSlot {
    SpotShadowSceneSlot {
        info,
        box_mid: bounds.map(|b| b.mid()),
        box_half: bounds.map(|b| b.half()),
    }
}

#[must_use]
pub fn gfx_scene_spot_shadow_dobj_slots(scene: &GfxScene) -> Vec<SpotShadowSceneSlot> {
    scene.scene_dobjs.iter().map(|d| spot_slot(d.info, d.posed_bounds)).collect()
}

#[must_use]
pub fn gfx_scene_spot_shadow_model_slots(scene: &GfxScene) -> Vec<SpotShadowSceneSlot> {
    scene.scene_models.iter().map(|m| spot_slot(m.info, m.posed_bounds)).collect()
}

/// Copies the scene's occupancy for the spot-shadow pass, and moves each
/// entity's tracked origin along when it has travelled far enough to relink.
pub fn snapshot_spot_shadow_occupancy(
    gfx: Res<HostGfxScene>,
    mut occupancy: ResMut<SpotShadowSceneOccupancy>,
    mut track: ResMut<SpotShadowEntityOriginTrack>,
) {
    let scene = &gfx.scene;
    occupancy.dobjs = gfx_scene_spot_shadow_dobj_slots(scene);
    occupancy.models = gfx_scene_spot_shadow_model_slots(scene);

    let dobjs = scene.scene_dobjs.iter().map(|d| (d.origin, d.info));
    let models = scene.scene_models.iter().map(|m| (m.origin, m.info));
    for (origin, info) in dobjs.chain(models) {
        let Some(tracked) = spot_shadow_entity_origin_track_index(0, scene_info_entnum(info))
            .and_then(|index| track.last.get_mut(index))
        else {
            continue;
        };
        if spot_shadow_entity_should_relink(*tracked, origin, SPOT_SHADOW_ENTITY_RELINK_THRESH_SQ) {
            *tracked = origin;
        }
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GfxSceneClear;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GfxSceneAdd;

/// Starts a frame: empties the scene, the skin inputs and the spot-shadow occupancy.
pub fn clear_host_gfx_scene(
    mut scene: ResMut<HostGfxScene>,
    mut skin_inputs: ResMut<SceneEntSkinInputs>,
    mut occupancy: ResMut<SpotShadowSceneOccupancy>,
) {
    scene.scene.clear();
    skin_inputs.by_ent.clear();
    skin_inputs.pending.clear();
    skin_inputs.frame_bytes_used = 0;
    *occupancy = SpotShadowSceneOccupancy::default();
}

#[must_use]
pub fn scene_quat_from_angles(angles: [f32; 3]) -> [f32; 4] {
    fx_t6::runtime::axis_to_quat(game_math::angles_to_axis(angles))
}

/// The first-person weapon turns with the view: its own angles first, then the view's.
#[must_use]
pub fn scene_quat_from_viewmodel_axes(gun_angles: [f32; 3], view_angles: [f32; 3]) -> [f32; 4] {
    fx_t6::runtime::axis_to_quat(fx_t6::runtime::mat3_mul(
        game_math::angles_to_axis(gun_angles),
        game_math::angles_to_axis(view_angles),
    ))
}

pub fn occupy_add_dobj(scene: &mut GfxScene, model_n: usize, has_tree: bool, pose: AddDObjPose) {
    occupy_add_dobj_fx(scene, model_n, has_tree, 0, pose);
}

/// Adds an object to the scene; an object the scene has no room for is dropped.
pub fn occupy_add_dobj_fx(
    scene: &mut GfxScene,
    model_n: usize,
    has_tree: bool,
    render_fx_flags: u32,
    pose: AddDObjPose,
) {
    let args = AddDObjArgs {
        render_fx_flags,
        material_time: 0.0,
        has_tree,
        num_models: u8::try_from(model_n).unwrap_or(u8::MAX),
        pose: Some(pose),
    };
    let _ = scene.add_dobj(args);
}

/// A script-moved brush model: where it is now and which entity moves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptMoverBmodelClaim {
    pub model_index: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub entnum: u32,
    pub hidden: bool,
}

/// Adds a brush model; one the scene has no room for is dropped.
pub fn occupy_add_bmodel(scene: &mut GfxScene, surf_id: i16, pose: AddBModelPose) {
    let _ = scene.add_bmodel(AddBModelArgs { surf_id }, pose);
}

fn surf_id_for_model(models: &[asset_world::GfxBrushModelSurfs], model_index: u32) -> i16 {
    models
        .get(model_index as usize)
        .and_then(|model| i16::try_from(model.surface_count).ok())
        .unwrap_or(0)
}

/// Fills the scene with the map's script brush models. Where a script mover
/// holds a brush model, the mover's pose wins and a hidden mover removes it;
/// movers with no authored brush are added after the authored ones.
pub fn occupy_script_brushes(
    scene: &mut GfxScene,
    models: &[asset_world::GfxBrushModelSurfs],
    authored: &[asset_world::ScriptBrushModelPlacement],
    live: &[ScriptMoverBmodelClaim],
) {
    let mut claims: BTreeMap<u32, ScriptMoverBmodelClaim> = BTreeMap::new();
    for claim in live.iter().filter(|claim| claim.model_index != 0) {
        claims.entry(claim.model_index).or_insert(*claim);
    }
    let claim_pose = |claim: &ScriptMoverBmodelClaim| AddBModelPose {
        model_index: claim.model_index,
        origin: claim.origin,
        quat: Some(scene_quat_from_angles(claim.angles)),
        param_4: u16::try_from(claim.entnum).ok(),
    };

    for brush in authored.iter().filter(|brush| brush.cmodel_handle != 0) {
        let surf_id = surf_id_for_model(models, brush.cmodel_handle);
        match claims.remove(&brush.cmodel_handle) {
            Some(claim) if claim.hidden => {}
            Some(claim) => occupy_add_bmodel(scene, surf_id, claim_pose(&claim)),
            None => {
                let pose = AddBModelPose {
                    model_index: brush.cmodel_handle,
                    origin: brush.origin,
                    quat: Some(scene_quat_from_angles(brush.angles)),
                    param_4: None,
                };
                occupy_add_bmodel(scene, surf_id, pose);
            }
        }
    }
    for claim in claims.values().filter(|claim| !claim.hidden) {
        occupy_add_bmodel(scene, surf_id_for_model(models, claim.model_index), claim_pose(claim));
    }
}

/// The skin inputs of this frame's objects, by entity number.
#[derive(Resource, Default)]
pub struct SceneEntSkinInputs {
    pub by_ent: HashMap<u32, SceneEntSkinInput>,
    /// Objects whose surfaces are still to be worked out; see [`expand_scene_ent_pending`].
    pub pending: HashMap<u32, SceneEntSkinPending>,
    pub frame_bytes_used: u32,
}

#[derive(Clone, Debug, Default)]
pub struct SceneEntSkinInput {
    pub models: Vec<SceneEntSkinModel>,
    pub hide_part_bits: [u32; 5],
}

#[derive(Clone, Debug)]
pub struct SceneEntSkinModel {
    pub lod: i8,
    pub bone_count: u8,
    pub surfaces: Arc<[dpvs_t6::PreSkinSurface]>,
}

#[derive(Clone, Debug)]
pub struct SceneEntSkinPendingModel {
    pub lod: i8,
    pub bone_count: u8,
    pub skel: Arc<asset_model::ModelSkel>,
}

#[derive(Clone, Debug, Default)]
pub struct SceneEntSkinPending {
    pub models: Vec<SceneEntSkinPendingModel>,
    pub hide_part_bits: [u32; 5],
}

/// Pre-skin surface lists, built once per model and detail level.
#[derive(Resource, Default)]
pub struct SceneEntSurfaceCache {
    by_name: HashMap<String, HashMap<u8, Arc<[dpvs_t6::PreSkinSurface]>>>,
}

impl SceneEntSurfaceCache {
    pub fn surfaces(
        &mut self,
        name: &str,
        skel: &asset_model::ModelSkel,
        lod: u8,
    ) -> Arc<[dpvs_t6::PreSkinSurface]> {
        let lods = self.by_name.entry(name.to_owned()).or_default();
        Arc::clone(lods.entry(lod).or_insert_with(|| {
            skel.surfaces_for_lod(lod).map(|surface| pre_skin_surface(skel, surface)).collect()
        }))
    }
}

/// A surface as the skin pass wants it. Whatever the model does not say is
/// assumed the expensive way: deformed, and no vertex lists.
fn pre_skin_surface(skel: &asset_model::ModelSkel, surface: usize) -> dpvs_t6::PreSkinSurface {
    dpvs_t6::PreSkinSurface {
        part_bits: skel.surface_part_bits.get(surface).copied().unwrap_or([0; 5]),
        vert_count: skel
            .surface_vertex_ranges
            .get(surface)
            .map_or(0, |&(_, count)| u16::try_from(count).unwrap_or(u16::MAX)),
        deformed: skel.surface_deformed.get(surface).copied().flatten().unwrap_or(true),
        vert_list_count: skel.surface_vert_list_count.get(surface).copied().flatten().unwrap_or(0),
    }
}

/// Queues an object's models for surface expansion, and records their LODs in the scene.
pub fn store_scene_ent_pending(
    scene: &mut GfxScene,
    skin_inputs: &mut SceneEntSkinInputs,
    entnum: u32,
    models: Vec<SceneEntSkinPendingModel>,
    hide_part_bits: [u32; 5],
) {
    let lods: Vec<i8> = models.iter().map(|model| model.lod).collect();
    scene.store_scene_ent_lods(entnum, &lods);
    skin_inputs.pending.insert(entnum, SceneEntSkinPending { models, hide_part_bits });
}

/// A model with a negative LOD is not drawn and has no surfaces.
pub fn expand_scene_ent_pending(
    pending: &SceneEntSkinPending,
    cache: &mut SceneEntSurfaceCache,
) -> SceneEntSkinInput {
    let models = pending
        .models
        .iter()
        .map(|model| SceneEntSkinModel {
            lod: model.lod,
            bone_count: model.bone_count,
            surfaces: match u8::try_from(model.lod) {
                Ok(lod) => cache.surfaces(&model.skel.name, &model.skel, lod),
                Err(_) => Arc::from([]),
            },
        })
        .collect();
    SceneEntSkinInput { models, hide_part_bits: pending.hide_part_bits }
}

/// Sets, at `base + bone`, the part bit of every bone that is named in
/// `hide_tags` or has an ancestor that is.
pub fn hide_part_bits_from_tags(
    bits: &mut [u32; 5],
    skel: &asset_model::ModelSkel,
    base: usize,
    hide_tags: &[String],
) {
    if hide_tags.is_empty() {
        return;
    }
    for bone in 0..skel.bone_names.len() {
        if asset_game::bone_has_hidden_ancestor(&skel.bone_names, |b| skel.parent_of(b), bone, hide_tags) {
            dpvs_t6::msb_set(bits, base + bone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush_models() -> Vec<asset_world::GfxBrushModelSurfs> {
        (0..6)
            .map(|i| asset_world::GfxBrushModelSurfs { surface_count: i, ..Default::default() })
            .collect()
    }

    fn authored(handle: u32, x: f32) -> asset_world::ScriptBrushModelPlacement {
        asset_world::ScriptBrushModelPlacement {
            source_ordinal: 0,
            cmodel_handle: handle,
            origin: [x, 0.0, 0.0],
            angles: [0.0; 3],
            script_exploder: String::new(),
            gameobject: String::new(),
            targetname: String::new(),
        }
    }

    fn claim(model_index: u32, x: f32, entnum: u32, hidden: bool) -> ScriptMoverBmodelClaim {
        ScriptMoverBmodelClaim { model_index, origin: [x, 0.0, 0.0], angles: [0.0; 3], entnum, hidden }
    }

    fn fresh_scene() -> GfxScene {
        let mut scene = GfxScene::default();
        scene.clear();
        scene
    }

    /// The (model, x, entity) of each brush the scene ended up with.
    fn brushes(scene: &GfxScene) -> Vec<(u32, f32, Option<u16>)> {
        scene.scene_brushes.iter().map(|b| (b.model_index, b.origin[0], b.param_4)).collect()
    }

    #[test]
    fn authored_brushes_are_placed_where_the_map_put_them() {
        let mut scene = fresh_scene();
        let placements = [authored(0, 9.0), authored(2, 5.0), authored(3, 6.0)];
        occupy_script_brushes(&mut scene, &brush_models(), &placements, &[]);
        assert_eq!(brushes(&scene), vec![(2, 5.0, None), (3, 6.0, None)]);
    }

    #[test]
    fn a_brush_with_no_surfaces_is_left_out() {
        let mut scene = fresh_scene();
        // Model 0 has no surfaces in the table, and model 9 is not in it.
        occupy_script_brushes(&mut scene, &brush_models(), &[authored(9, 1.0)], &[claim(9, 2.0, 1, false)]);
        assert!(scene.scene_brushes.is_empty());
    }

    #[test]
    fn a_mover_replaces_the_authored_pose_and_a_hidden_one_removes_the_brush() {
        let mut scene = fresh_scene();
        let placements = [authored(1, 0.0), authored(2, 0.0), authored(3, 0.0)];
        let movers = [claim(1, 50.0, 7, false), claim(2, 60.0, 8, true)];
        occupy_script_brushes(&mut scene, &brush_models(), &placements, &movers);
        assert_eq!(brushes(&scene), vec![(1, 50.0, Some(7)), (3, 0.0, None)]);
    }

    #[test]
    fn the_first_mover_for_a_model_wins_and_model_zero_is_ignored() {
        let mut scene = fresh_scene();
        let movers = [claim(0, 1.0, 1, false), claim(4, 10.0, 2, false), claim(4, 20.0, 3, false)];
        occupy_script_brushes(&mut scene, &brush_models(), &[], &movers);
        assert_eq!(brushes(&scene), vec![(4, 10.0, Some(2))]);
    }

    #[test]
    fn movers_without_an_authored_brush_come_after_the_authored_ones() {
        let mut scene = fresh_scene();
        let movers = [claim(5, 1.0, 1, false), claim(2, 2.0, 2, false), claim(3, 3.0, 3, true)];
        occupy_script_brushes(&mut scene, &brush_models(), &[authored(4, 0.0)], &movers);
        let models: Vec<u32> = brushes(&scene).iter().map(|b| b.0).collect();
        assert_eq!(models, vec![4, 2, 5], "in model order, hidden ones dropped");
    }

    #[test]
    fn a_mover_entity_number_too_big_for_the_scene_is_left_off_the_brush() {
        let mut scene = fresh_scene();
        occupy_script_brushes(&mut scene, &brush_models(), &[], &[claim(1, 0.0, 70_000, false)]);
        assert_eq!(brushes(&scene), vec![(1, 0.0, None)]);
    }

    #[test]
    fn view_model_and_plain_angles_agree_when_the_view_is_level() {
        let angles = [10.0, 80.0, 0.0];
        let plain = scene_quat_from_angles(angles);
        let viewmodel = scene_quat_from_viewmodel_axes(angles, [0.0; 3]);
        for (a, b) in plain.iter().zip(viewmodel) {
            assert!((a - b).abs() < 1e-5, "{plain:?} vs {viewmodel:?}");
        }
    }

    #[test]
    fn objects_added_with_a_big_model_count_are_clamped_not_dropped() {
        let mut scene = fresh_scene();
        let pose = AddDObjPose {
            origin: [0.0; 3],
            lighting_origin: [0.0; 3],
            radius: None,
            entnum: 4,
            quat: None,
        };
        occupy_add_dobj(&mut scene, 1_000, false, pose);
        occupy_add_dobj_fx(&mut scene, 1, false, 0, AddDObjPose { entnum: 5, ..pose });
        assert!(scene.scene_dobj_live(4), "many models make an animated object");
        assert!(scene.scene_model_live(5), "one bare model makes a scene model");
    }

    #[test]
    fn the_snapshot_copies_slots_and_moves_the_tracked_origin_of_far_movers() {
        let mut host = HostGfxScene::default();
        host.scene.clear();
        let pose = AddDObjPose {
            origin: [500.0, 0.0, 0.0],
            lighting_origin: [0.0; 3],
            radius: Some(8.0),
            entnum: 3,
            quat: None,
        };
        occupy_add_dobj(&mut host.scene, 2, false, pose);
        let Some(index) = spot_shadow_entity_origin_track_index(0, 3) else {
            panic!("entity 3 has no track slot");
        };

        let mut app = App::new();
        app.insert_resource(host)
            .init_resource::<SpotShadowSceneOccupancy>()
            .init_resource::<SpotShadowEntityOriginTrack>()
            .add_systems(Update, snapshot_spot_shadow_occupancy);
        app.update();

        let occupancy = app.world().resource::<SpotShadowSceneOccupancy>();
        assert_eq!((occupancy.dobjs.len(), occupancy.models.len()), (1, 0));
        assert_eq!(occupancy.dobjs[0].box_mid, Some([500.0, 0.0, 0.0]));
        assert_eq!(app.world().resource::<SpotShadowEntityOriginTrack>().last[index], [500.0, 0.0, 0.0]);
    }

    #[test]
    fn a_slot_without_a_posed_box_has_no_box() {
        let slot = spot_slot(12, None);
        assert_eq!((slot.info, slot.box_mid, slot.box_half), (12, None, None));
    }

    #[test]
    fn clearing_the_host_scene_empties_everything_it_owns() {
        let mut app = App::new();
        app.init_resource::<HostGfxScene>()
            .init_resource::<SceneEntSkinInputs>()
            .init_resource::<SpotShadowSceneOccupancy>()
            .add_systems(Update, clear_host_gfx_scene);
        {
            let mut inputs = app.world_mut().resource_mut::<SceneEntSkinInputs>();
            inputs.by_ent.insert(1, SceneEntSkinInput::default());
            inputs.pending.insert(1, SceneEntSkinPending::default());
            inputs.frame_bytes_used = 99;
            app.world_mut()
                .resource_mut::<SpotShadowSceneOccupancy>()
                .dobjs
                .push(spot_slot(1, None));
        }
        app.update();
        let inputs = app.world().resource::<SceneEntSkinInputs>();
        assert!(inputs.by_ent.is_empty() && inputs.pending.is_empty() && inputs.frame_bytes_used == 0);
        assert!(app.world().resource::<SpotShadowSceneOccupancy>().dobjs.is_empty());
        assert_eq!(
            app.world().resource::<HostGfxScene>().scene.scene_dobj_index.len(),
            dpvs_t6::GFX_CFG_ENT_COUNT as usize
        );
    }

    fn skel(name: &str, surfaces: usize) -> asset_model::ModelSkel {
        asset_model::ModelSkel {
            name: name.to_owned(),
            surface_vertex_ranges: (0..surfaces).map(|i| (i * 10, 10 + i)).collect(),
            surface_part_bits: vec![[1, 0, 0, 0, 0]; surfaces],
            surface_deformed: vec![Some(false); surfaces],
            surface_vert_list_count: vec![Some(2); surfaces],
            ..Default::default()
        }
    }

    #[test]
    fn surfaces_describe_what_the_model_says_and_assume_the_worst_for_the_rest() {
        let mut model = skel("m", 2);
        model.surface_deformed.truncate(1);
        model.surface_vert_list_count.truncate(1);
        model.surface_part_bits.truncate(1);
        let mut cache = SceneEntSurfaceCache::default();
        let surfaces = cache.surfaces("m", &model, 0);
        assert_eq!(surfaces.len(), 2);
        assert_eq!(
            (surfaces[0].part_bits, surfaces[0].vert_count, surfaces[0].deformed, surfaces[0].vert_list_count),
            ([1, 0, 0, 0, 0], 10, false, 2)
        );
        assert_eq!(
            (surfaces[1].part_bits, surfaces[1].vert_count, surfaces[1].deformed, surfaces[1].vert_list_count),
            ([0; 5], 11, true, 0)
        );
    }

    #[test]
    fn a_huge_surface_saturates_its_vertex_count() {
        let mut model = skel("big", 1);
        model.surface_vertex_ranges[0] = (0, 1 << 20);
        let surfaces = SceneEntSurfaceCache::default().surfaces("big", &model, 0);
        assert_eq!(surfaces[0].vert_count, u16::MAX);
    }

    #[test]
    fn the_cache_builds_a_model_level_once() {
        let mut cache = SceneEntSurfaceCache::default();
        let first = cache.surfaces("m", &skel("m", 2), 0);
        let again = cache.surfaces("m", &skel("m", 5), 0);
        assert!(Arc::ptr_eq(&first, &again));
        assert!(cache.surfaces("m", &skel("m", 2), 1).is_empty(), "a plain model has only level 0");
    }

    #[test]
    fn expanding_gives_a_hidden_level_no_surfaces() {
        let pending = SceneEntSkinPending {
            models: vec![
                SceneEntSkinPendingModel { lod: 0, bone_count: 3, skel: Arc::new(skel("a", 2)) },
                SceneEntSkinPendingModel { lod: -1, bone_count: 4, skel: Arc::new(skel("b", 2)) },
            ],
            hide_part_bits: [0, 0, 7, 0, 0],
        };
        let input = expand_scene_ent_pending(&pending, &mut SceneEntSurfaceCache::default());
        assert_eq!(input.hide_part_bits, [0, 0, 7, 0, 0]);
        assert_eq!(input.models.iter().map(|m| (m.lod, m.bone_count, m.surfaces.len())).collect::<Vec<_>>(), vec![
            (0, 3, 2),
            (-1, 4, 0)
        ]);
    }

    #[test]
    fn storing_pending_models_records_their_levels_in_the_scene() {
        let mut scene = fresh_scene();
        let pose = AddDObjPose {
            origin: [0.0; 3],
            lighting_origin: [0.0; 3],
            radius: None,
            entnum: 6,
            quat: None,
        };
        occupy_add_dobj(&mut scene, 2, false, pose);
        let mut inputs = SceneEntSkinInputs::default();
        let model = |lod| SceneEntSkinPendingModel { lod, bone_count: 1, skel: Arc::new(skel("m", 1)) };
        store_scene_ent_pending(&mut scene, &mut inputs, 6, vec![model(1), model(-1)], [9; 5]);
        assert_eq!(scene.scene_dobj(6).expect("dobj").lods, vec![1, -1]);
        assert_eq!(inputs.pending[&6].hide_part_bits, [9; 5]);
    }

    #[test]
    fn hidden_tags_set_the_part_bit_of_each_hidden_bone() {
        let model = asset_model::ModelSkel {
            bone_names: ["root", "Arm", "hand"].map(String::from).to_vec(),
            ..Default::default()
        };
        let mut bits = [0; 5];
        hide_part_bits_from_tags(&mut bits, &model, 2, &[]);
        assert_eq!(bits, [0; 5], "no tags hide nothing");
        hide_part_bits_from_tags(&mut bits, &model, 2, &["arm".to_owned()]);
        let mut expected = [0; 5];
        dpvs_t6::msb_set(&mut expected, 3);
        assert_eq!(bits, expected, "the match ignores case and the bit sits at base + bone");
    }
}
