//! The host's copy of the frame scene, the occupancy the spot-shadow pass reads
//! from it, and the per-object skin inputs that go with it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::prelude::*;
use lighting_t6::{
    SPOT_SHADOW_ORIGIN_TRACKED_LIMIT, SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED, ConeViewSlot,
    cone_origin_slot,
};

use crate::AnimatedObjectSpawn;
use crate::BrushModelPlacement;
use crate::DynObjectPlacementArgs;
use crate::InsertBrushModelArgs;
use crate::VisualStage;
use crate::stage_info_actor_num;

#[derive(Resource, Default)]
pub struct HostVisualStage {
    pub scene: VisualStage,
}

/// What the spot-shadow pass sees of the scene: one slot per object, with the box it was posed in.
#[derive(Resource, Default)]
pub struct ConeShadowStageOccupancy {
    pub dobjs: Vec<ConeViewSlot>,
    pub models: Vec<ConeViewSlot>,
}

/// The origin each entity had when its spot shadows were last linked.
#[derive(Resource, Clone, Debug)]
pub struct SpotShadowEntityOriginLog {
    pub last: Vec<[f32; 3]>,
}

impl Default for SpotShadowEntityOriginLog {
    fn default() -> Self {
        let slots = SPOT_SHADOW_ORIGIN_TRACKED_LIMIT as usize;
        let last = std::iter::repeat([0.0f32; 3]).take(slots).collect();
        Self { last }
    }
}

fn spot_slot(packed: u32, posed_box: Option<dpvs_t6::Bounds>) -> ConeViewSlot {
    let box_mid = posed_box.map(|b| b.mid());
    let box_half = posed_box.map(|b| b.half());
    ConeViewSlot { ent_bits: packed, mid_point: box_mid, half_ext: box_half }
}

#[must_use]
pub fn spot_shadow_figure_slots(scene: &VisualStage) -> Vec<ConeViewSlot> {
    let mut slots = Vec::with_capacity(scene.stage_puppets.len());
    for dobj in &scene.stage_puppets {
        slots.push(spot_slot(dobj.about, dobj.pose_bounds));
    }
    slots
}

#[must_use]
pub fn visual_stage_cone_shadow_model_slots(scene: &VisualStage) -> Vec<ConeViewSlot> {
    let mut slots = Vec::with_capacity(scene.stage_models.len());
    for model in &scene.stage_models {
        slots.push(spot_slot(model.about, model.pose_bounds));
    }
    slots
}

/// Copies the scene's occupancy for the spot-shadow pass, and moves each
/// entity's tracked origin along when it has travelled far enough to relink.
pub fn capture_cone_shadow_fill(
    gfx: Res<HostVisualStage>,
    mut occupancy: ResMut<ConeShadowStageOccupancy>,
    mut track: ResMut<SpotShadowEntityOriginLog>,
) {
    let scene = &gfx.scene;
    *occupancy = ConeShadowStageOccupancy {
        dobjs: spot_shadow_figure_slots(scene),
        models: visual_stage_cone_shadow_model_slots(scene),
    };

    let placed = scene
        .stage_puppets
        .iter()
        .map(|dobj| (dobj.about, dobj.origin))
        .chain(scene.stage_models.iter().map(|model| (model.about, model.origin)));
    for (info, now) in placed {
        let slot = cone_origin_slot(0, stage_info_actor_num(info));
        if let Some(last) = slot.and_then(|slot| track.last.get_mut(slot)) {
            if lighting_t6::cone_relink_due(*last, now, SPOT_SHADOW_RELINK_MOVE_DIST_SQUARED) {
                *last = now;
            }
        }
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderSceneWipe;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderSceneInsert;

/// Starts a frame: empties the scene, the skin inputs and the spot-shadow occupancy.
pub fn reset_host_render_scene(
    mut scene: ResMut<HostVisualStage>,
    mut skinning_feed: ResMut<EntitySkinningInputSet>,
    mut occupancy: ResMut<ConeShadowStageOccupancy>,
) {
    VisualStage::clear(&mut scene.scene);
    let EntitySkinningInputSet { per_entity: drawn, pending: queued, frame_bytes_consumed: used } = &mut *skinning_feed;
    drawn.clear();
    queued.clear();
    *used = 0;
    *occupancy = ConeShadowStageOccupancy::default();
}

#[must_use]
pub fn orientation_quat_from_euler(angles: [f32; 3]) -> [f32; 4] {
    let axis = game_math::euler_to_rows(angles);
    fx_t6::runtime::axis_to_quat(axis)
}

/// The first-person weapon turns with the view: its own angles first, then the view's.
#[must_use]
pub fn quat_from_viewmodel_basis(weapon_aim_angles: [f32; 3], look_rotation: [f32; 3]) -> [f32; 4] {
    let gun = game_math::euler_to_rows(weapon_aim_angles);
    let view = game_math::euler_to_rows(look_rotation);
    fx_t6::runtime::axis_to_quat(fx_t6::runtime::mat3_mul(gun, view))
}

/// The scene's arguments for an object posed at `pose`; more models than the
/// count can hold clamp to its top.
fn dobj_args(model_tally: usize, carries_anim_graph: bool, fx: u32, placement: DynObjectPlacementArgs) -> AnimatedObjectSpawn {
    let model_slot_total = u8::try_from(model_tally).unwrap_or(u8::MAX);
    AnimatedObjectSpawn {
        model_slot_total,
        carries_anim_graph,
        pose: Some(placement),
        render_effect_flags: fx,
        surface_clock: 0.0,
    }
}

pub fn stage_figure(scene: &mut VisualStage, model_tally: usize, carries_anim_graph: bool, pose: DynObjectPlacementArgs) {
    // An object the scene has no room for is dropped.
    scene.place_object(dobj_args(model_tally, carries_anim_graph, 0, pose)).ok();
}

/// Adds an object to the scene; an object the scene has no room for is dropped.
pub fn occupy_add_puppet_effect(
    scene: &mut VisualStage,
    model_tally: usize,
    carries_anim_graph: bool,
    render_effect_flags: u32,
    pose: DynObjectPlacementArgs,
) {
    scene.place_object(dobj_args(model_tally, carries_anim_graph, render_effect_flags, pose)).ok();
}

/// A script-moved brush model: where it is now and which entity moves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptedBrushMoverClaim {
    pub mesh_slot_number: u32,
    pub origin: [f32; 3],
    pub turn: [f32; 3],
    pub ent_no: u32,
    pub hidden: bool,
}

/// Adds a brush model; one the scene has no room for is dropped.
pub fn stage_brush_model(scene: &mut VisualStage, faces: i16, pose: BrushModelPlacement) {
    scene.add_bmodel(InsertBrushModelArgs { face_no: faces }, pose).ok();
}

/// The surface id the scene files a brush model under: its surface count, or 0
/// when the model is unknown or has too many surfaces to say.
fn brush_face_tag(models: &[asset_world::SolidModelFaces], row: u32) -> i16 {
    let Some(model) = models.get(row as usize) else {
        return 0;
    };
    i16::try_from(model.facet_tally).unwrap_or(0)
}

/// A brush pose turned by `angles`; the moving entity is kept only when the
/// scene's 16-bit field can hold its number.
fn brush_pose(row: u32, at: [f32; 3], turn: [f32; 3], mover: Option<u32>) -> BrushModelPlacement {
    let quat = Some(orientation_quat_from_euler(turn));
    let extra_slot_param = mover.and_then(|who| u16::try_from(who).ok());
    BrushModelPlacement { mesh_slot_number: row, spot: at, spin: quat, extra_slot_param }
}

fn mover_pose(claim: &ScriptedBrushMoverClaim) -> BrushModelPlacement {
    brush_pose(claim.mesh_slot_number, claim.origin, claim.turn, Some(claim.ent_no))
}

/// Fills the scene with the map's script brush models. Where a script mover
/// holds a brush model, the mover's pose wins and a hidden mover removes it;
/// movers with no authored brush are added after the authored ones.
pub fn occupy_script_solids(
    scene: &mut VisualStage,
    models: &[asset_world::SolidModelFaces],
    authored: &[asset_world::ScriptSolidPlacementInfo],
    live: &[ScriptedBrushMoverClaim],
) {
    // The first live mover of each model holds it; model 0 is the world.
    let mut held = BTreeMap::<u32, ScriptedBrushMoverClaim>::new();
    for mover in live {
        if mover.mesh_slot_number != 0 && !held.contains_key(&mover.mesh_slot_number) {
            held.insert(mover.mesh_slot_number, *mover);
        }
    }

    for placed in authored {
        let model_ref = placed.collision_model_ref;
        if model_ref == 0 {
            continue;
        }
        let pose = match held.remove(&model_ref) {
            None => brush_pose(model_ref, placed.origin, placed.angles, None),
            Some(mover) if !mover.hidden => mover_pose(&mover),
            Some(_) => continue,
        };
        stage_brush_model(scene, brush_face_tag(models, model_ref), pose);
    }
    held.into_values()
        .filter(|mover| !mover.hidden)
        .for_each(|mover| stage_brush_model(scene, brush_face_tag(models, mover.mesh_slot_number), mover_pose(&mover)));
}

/// The skin inputs of this frame's objects, by entity number.
#[derive(Resource, Default)]
pub struct EntitySkinningInputSet {
    pub per_entity: HashMap<u32, EntitySkinningInput>,
    /// Objects whose surfaces are still to be worked out; see [`grow_pending_scene_entities`].
    pub pending: HashMap<u32, EntitySkinAwaitingWork>,
    pub frame_bytes_consumed: u32,
}

#[derive(Clone, Debug, Default)]
pub struct EntitySkinningInput {
    pub models: Vec<EntitySkinModelHandle>,
    pub hidden_parts: [u32; 5],
}

#[derive(Clone, Debug)]
pub struct EntitySkinModelHandle {
    pub lod: i8,
    pub joint_total: u8,
    pub surfaces: Arc<[dpvs_t6::PreposedSurfaceInput]>,
}

#[derive(Clone, Debug)]
pub struct StageActorSkinQueuedModel {
    pub lod: i8,
    pub joint_total: u8,
    pub bone_rig: Arc<asset_model::MeshRig>,
}

#[derive(Clone, Debug, Default)]
pub struct EntitySkinAwaitingWork {
    pub models: Vec<StageActorSkinQueuedModel>,
    pub hidden_parts: [u32; 5],
}

/// Pre-skin surface lists, built once per model and detail level.
#[derive(Resource, Default)]
pub struct EntitySurfaceMemo {
    by_model: HashMap<(String, u8), Arc<[dpvs_t6::PreposedSurfaceInput]>>,
}

impl EntitySurfaceMemo {
    pub fn surfaces(&mut self, name: &str, bone_rig: &asset_model::MeshRig, lod: u8) -> Arc<[dpvs_t6::PreposedSurfaceInput]> {
        Arc::clone(self.by_model.entry((name.to_owned(), lod)).or_insert_with(|| {
            bone_rig.facets_at_tier(lod).map(|surface| pre_skin_surface(bone_rig, surface)).collect()
        }))
    }
}

/// A surface as the skin pass wants it. Whatever the model does not say is
/// assumed the expensive way: deformed, and no vertex lists.
fn pre_skin_surface(bone_rig: &asset_model::MeshRig, surface: usize) -> dpvs_t6::PreposedSurfaceInput {
    let vertex_tally = match bone_rig.facet_vertex_ranges.get(surface) {
        Some(&(_, count)) => u16::try_from(count).unwrap_or(u16::MAX),
        None => 0,
    };
    dpvs_t6::PreposedSurfaceInput {
        hide_bits: bone_rig.face_parts.get(surface).copied().unwrap_or_default(),
        vertex_tally,
        deformed: bone_rig.bent.get(surface).copied().flatten().unwrap_or(true),
        vert_lists: bone_rig.vert_lists.get(surface).copied().flatten().unwrap_or(0),
    }
}

/// Queues an object's models for surface expansion, and records their LODs in the scene.
pub fn store_stage_actor_queued(
    scene: &mut VisualStage,
    skinning_feed: &mut EntitySkinningInputSet,
    who: u32,
    models: Vec<StageActorSkinQueuedModel>,
    hidden: [u32; 5],
) {
    let detail_tiers: Vec<i8> = models.iter().map(|model| model.lod).collect();
    scene.store_lods_for(who, &detail_tiers);
    skinning_feed.pending.insert(who, EntitySkinAwaitingWork { models, hidden_parts: hidden });
}

/// One pending model with its surfaces filled in from the cache.
fn expand_model(model: &StageActorSkinQueuedModel, cache: &mut EntitySurfaceMemo) -> EntitySkinModelHandle {
    let surfaces = if model.lod < 0 {
        Arc::from([])
    } else {
        cache.surfaces(&model.bone_rig.name, &model.bone_rig, model.lod as u8)
    };
    let StageActorSkinQueuedModel { lod, joint_total, .. } = *model;
    EntitySkinModelHandle { lod, joint_total, surfaces }
}

/// A model with a negative LOD is not drawn and has no surfaces.
pub fn grow_pending_scene_entities(
    queued: &EntitySkinAwaitingWork,
    cache: &mut EntitySurfaceMemo,
) -> EntitySkinningInput {
    let mut expanded = EntitySkinningInput {
        models: Vec::with_capacity(queued.models.len()),
        hidden_parts: queued.hidden_parts,
    };
    for model in &queued.models {
        expanded.models.push(expand_model(model, cache));
    }
    expanded
}

/// Sets, at `first_bit + bone`, the part bit of every bone that is named in
/// `concealed_bones` or has an ancestor that is.
pub fn hidden_part_mask_for_tags(
    mask: &mut [u32; 5],
    bone_rig: &asset_model::MeshRig,
    first_bit: usize,
    concealed_bones: &[String],
) {
    // No tags hide nothing, so skip the ancestry walk entirely.
    let bones = match concealed_bones {
        [] => 0,
        _ => bone_rig.joint_labels.len(),
    };
    let hidden = |bone: &usize| {
        asset_game::joint_under_concealed_parent(&bone_rig.joint_labels, |b| bone_rig.hanging_from(b), *bone, concealed_bones)
    };
    (0..bones).filter(hidden).for_each(|bone| dpvs_t6::top_bit_set(mask, first_bit + bone));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn solid_models() -> Vec<asset_world::SolidModelFaces> {
        (0..6)
            .map(|i| asset_world::SolidModelFaces { facet_tally: i, ..asset_world::SolidModelFaces::default() })
            .collect()
    }

    /// A map placement of brush model `collision_model_ref`, unturned, at (x, 0, 0).
    fn authored(collision_model_ref: u32, x: f32) -> asset_world::ScriptSolidPlacementInfo {
        let mut origin = [0.0; 3];
        origin[0] = x;
        let blank = String::new();
        asset_world::ScriptSolidPlacementInfo {
            origin_sequence_number: 0, angles: <[f32; 3]>::default(),
            collision_model_ref,
            origin,
            scripted_detonator: blank.clone(),
            script_entity: blank.clone(),
            lookup_key: blank,
        }
    }

    /// A world holding the host-scene resources, all defaulted.
    fn host_world() -> World {
        let mut world = World::new();
        world.init_resource::<HostVisualStage>();
        world.init_resource::<EntitySkinningInputSet>();
        world.init_resource::<ConeShadowStageOccupancy>();
        world.init_resource::<SpotShadowEntityOriginLog>();
        world
    }

    fn claim(row: u32, x: f32, who: u32, hidden: bool) -> ScriptedBrushMoverClaim {
        ScriptedBrushMoverClaim { mesh_slot_number: row, origin: [x, 0.0, 0.0], turn: <[f32; 3]>::default(), ent_no: who, hidden }
    }

    fn fresh_scene() -> VisualStage {
        let mut scene = VisualStage::default();
        scene.clear();
        scene
    }

    /// The (model, x, entity) of each brush the scene ended up with.
    fn brushes(scene: &VisualStage) -> Vec<(u32, f32, Option<u16>)> {
        scene.scene_brushes.iter().map(|b| (b.mesh_slot_number, b.origin[0], b.extra_slot_param)).collect()
    }

    #[test]
    fn authored_brushes_are_placed_where_the_map_put_them() {
        let mut scene = fresh_scene();
        let placements = [authored(0, 9.0), authored(2, 5.0), authored(3, 6.0)];
        occupy_script_solids(&mut scene, &solid_models(), &placements, &[]);
        assert_eq!(brushes(&scene), vec![(2, 5.0, None), (3, 6.0, None)]);
    }

    #[test]
    fn a_brush_with_no_surfaces_is_left_out() {
        let mut scene = fresh_scene();
        // Model 0 has no surfaces in the table, and model 9 is not in it.
        occupy_script_solids(&mut scene, &solid_models(), &[authored(9, 1.0)], &[claim(9, 2.0, 1, false)]);
        assert!(scene.scene_brushes.is_empty());
    }

    #[test]
    fn a_mover_replaces_the_authored_pose_and_a_hidden_one_removes_the_brush() {
        let mut scene = fresh_scene();
        let placements = [authored(1, 0.0), authored(2, 0.0), authored(3, 0.0)];
        let movers = [claim(1, 50.0, 7, false), claim(2, 60.0, 8, true)];
        occupy_script_solids(&mut scene, &solid_models(), &placements, &movers);
        assert_eq!(brushes(&scene), vec![(1, 50.0, Some(7)), (3, 0.0, None)]);
    }

    #[test]
    fn the_first_mover_for_a_model_wins_and_model_zero_is_ignored() {
        let mut scene = fresh_scene();
        let movers = [claim(0, 1.0, 1, false), claim(4, 10.0, 2, false), claim(4, 20.0, 3, false)];
        occupy_script_solids(&mut scene, &solid_models(), &[], &movers);
        assert_eq!(brushes(&scene), vec![(4, 10.0, Some(2))]);
    }

    #[test]
    fn movers_without_an_authored_brush_come_after_the_authored_ones() {
        let mut scene = fresh_scene();
        let movers = [claim(5, 1.0, 1, false), claim(2, 2.0, 2, false), claim(3, 3.0, 3, true)];
        occupy_script_solids(&mut scene, &solid_models(), &[authored(4, 0.0)], &movers);
        let models: Vec<u32> = brushes(&scene).iter().map(|b| b.0).collect();
        assert_eq!(models, vec![4, 2, 5], "in model order, hidden ones dropped");
    }

    #[test]
    fn a_mover_entity_number_too_big_for_the_scene_is_left_off_the_brush() {
        let mut scene = fresh_scene();
        occupy_script_solids(&mut scene, &solid_models(), &[], &[claim(1, 0.0, 70_000, false)]);
        assert_eq!(brushes(&scene), vec![(1, 0.0, None)]);
    }

    #[test]
    fn view_model_and_plain_angles_agree_when_the_view_is_level() {
        let angles = [10.0, 80.0, 0.0];
        let plain = orientation_quat_from_euler(angles);
        let viewmodel = quat_from_viewmodel_basis(angles, [0.0; 3]);
        for (a, b) in plain.iter().zip(viewmodel) {
            assert!((a - b).abs() < 1e-5, "{plain:?} vs {viewmodel:?}");
        }
    }

    #[test]
    fn objects_added_with_a_big_model_count_are_clamped_not_dropped() {
        let mut scene = fresh_scene();
        let pose = DynObjectPlacementArgs {
            origin: <[f32; 3]>::default(),
            illum_origin: [0.0; 3],
            reach: None,
            ent_no: 4,
            spin: None,
        };
        stage_figure(&mut scene, 1_000, false, pose);
        occupy_add_puppet_effect(&mut scene, 1, false, 0, DynObjectPlacementArgs { ent_no: 5, ..pose });
        assert!(scene.animated_object_present(4), "many models make an animated object");
        assert!(scene.static_model_present(5), "one bare model makes a scene model");
    }

    #[test]
    fn the_snapshot_copies_slots_and_moves_the_tracked_origin_of_far_movers() {
        let mut host = HostVisualStage::default();
        host.scene.clear();
        let pose = DynObjectPlacementArgs {
            origin: [500.0, 0.0, 0.0],
            illum_origin: [0.0; 3],
            reach: Some(8.0),
            ent_no: 3,
            spin: None,
        };
        stage_figure(&mut host.scene, 2, false, pose);
        let Some(index) = cone_origin_slot(0, 3) else {
            panic!("entity 3 has no track slot");
        };

        let mut world = host_world();
        world.insert_resource(host);
        world.run_system_once(capture_cone_shadow_fill).expect("snapshot runs");

        let occupancy = world.resource::<ConeShadowStageOccupancy>();
        assert_eq!((occupancy.dobjs.len(), occupancy.models.len()), (1, 0));
        assert_eq!(occupancy.dobjs[0].mid_point, Some([500.0, 0.0, 0.0]));
        assert_eq!(world.resource::<SpotShadowEntityOriginLog>().last[index], [500.0, 0.0, 0.0]);
    }

    #[test]
    fn a_slot_without_a_posed_box_has_no_box() {
        let slot = spot_slot(12, None);
        assert_eq!((slot.ent_bits, slot.mid_point, slot.half_ext), (12, None, None));
    }

    #[test]
    fn clearing_the_host_scene_empties_everything_it_owns() {
        let mut world = host_world();
        let mut inputs = world.resource_mut::<EntitySkinningInputSet>();
        inputs.per_entity.insert(1, EntitySkinningInput::default());
        inputs.pending.insert(1, EntitySkinAwaitingWork::default());
        inputs.frame_bytes_consumed = 99;
        world.resource_mut::<ConeShadowStageOccupancy>().dobjs.push(spot_slot(1, None));
        world.run_system_once(reset_host_render_scene).expect("reset runs");
        let inputs = world.resource::<EntitySkinningInputSet>();
        assert!(inputs.per_entity.is_empty() && inputs.pending.is_empty() && inputs.frame_bytes_consumed == 0);
        assert!(world.resource::<ConeShadowStageOccupancy>().dobjs.is_empty());
        assert_eq!(
            world.resource::<HostVisualStage>().scene.animated_object_slot_table.len(),
            dpvs_t6::VISUAL_CFG_ACTOR_TALLY as usize
        );
    }

    fn bone_rig(name: &str, surfaces: usize) -> asset_model::MeshRig {
        asset_model::MeshRig {
            name: name.to_owned(),
            facet_vertex_ranges: (0..surfaces).map(|i| (i * 10, 10 + i)).collect(),
            face_parts: vec![[1, 0, 0, 0, 0]; surfaces],
            bent: vec![Some(false); surfaces],
            vert_lists: vec![Some(2); surfaces],
            ..Default::default()
        }
    }

    #[test]
    fn surfaces_describe_what_the_model_says_and_assume_the_worst_for_the_rest() {
        let mut model = bone_rig("m", 2);
        model.bent.truncate(1);
        model.vert_lists.truncate(1);
        model.face_parts.truncate(1);
        let mut cache = EntitySurfaceMemo::default();
        let surfaces = cache.surfaces("m", &model, 0);
        assert_eq!(surfaces.len(), 2);
        assert_eq!(
            (surfaces[0].hide_bits, surfaces[0].vertex_tally, surfaces[0].deformed, surfaces[0].vert_lists),
            ([1, 0, 0, 0, 0], 10, false, 2)
        );
        assert_eq!(
            (surfaces[1].hide_bits, surfaces[1].vertex_tally, surfaces[1].deformed, surfaces[1].vert_lists),
            ([0; 5], 11, true, 0)
        );
    }

    #[test]
    fn a_huge_surface_saturates_its_vertex_count() {
        let mut model = bone_rig("big", 1);
        model.facet_vertex_ranges[0] = (0, 1 << 20);
        let surfaces = EntitySurfaceMemo::default().surfaces("big", &model, 0);
        assert_eq!(surfaces[0].vertex_tally, u16::MAX);
    }

    #[test]
    fn the_cache_builds_a_model_level_once() {
        let mut cache = EntitySurfaceMemo::default();
        let first = cache.surfaces("m", &bone_rig("m", 2), 0);
        let again = cache.surfaces("m", &bone_rig("m", 5), 0);
        assert!(Arc::ptr_eq(&first, &again));
        assert!(cache.surfaces("m", &bone_rig("m", 2), 1).is_empty(), "a plain model has only level 0");
    }

    #[test]
    fn expanding_gives_a_hidden_level_no_surfaces() {
        let mask = [0, 0, 7, 0, 0];
        let pending = EntitySkinAwaitingWork {
            models: vec![
                StageActorSkinQueuedModel { lod: 0, joint_total: 3, bone_rig: Arc::new(bone_rig("a", 2)) },
                StageActorSkinQueuedModel { lod: -1, joint_total: 4, bone_rig: Arc::new(bone_rig("b", 2)) },
            ],
            hidden_parts: mask,
        };
        let input = grow_pending_scene_entities(&pending, &mut EntitySurfaceMemo::default());
        assert_eq!(input.hidden_parts, mask);
        assert_eq!(input.models.iter().map(|m| (m.lod, m.joint_total, m.surfaces.len())).collect::<Vec<_>>(), vec![
            (0, 3, 2),
            (-1, 4, 0)
        ]);
    }

    #[test]
    fn storing_pending_models_records_their_levels_in_the_scene() {
        let mut scene = fresh_scene();
        let pose = DynObjectPlacementArgs {
            origin: <[f32; 3]>::default(),
            illum_origin: [0.0; 3],
            reach: None,
            ent_no: 6,
            spin: None,
        };
        stage_figure(&mut scene, 2, false, pose);
        let mut inputs = EntitySkinningInputSet::default();
        let model = |lod| StageActorSkinQueuedModel { lod, joint_total: 1, bone_rig: Arc::new(bone_rig("m", 1)) };
        store_stage_actor_queued(&mut scene, &mut inputs, 6, vec![model(1), model(-1)], [9; 5]);
        assert_eq!(scene.stage_puppet(6).expect("dobj").detail_tiers, vec![1, -1]);
        assert_eq!(inputs.pending[&6].hidden_parts, [9; 5]);
    }

    #[test]
    fn hidden_tags_set_the_part_bit_of_each_hidden_bone() {
        let model = asset_model::MeshRig {
            joint_labels: ["root", "Arm", "hand"].map(String::from).to_vec(),
            ..Default::default()
        };
        let mut bits = [0; 5];
        hidden_part_mask_for_tags(&mut bits, &model, 2, &[]);
        assert_eq!(bits, [0; 5], "no tags hide nothing");
        hidden_part_mask_for_tags(&mut bits, &model, 2, &["arm".to_owned()]);
        let mut expected = [0; 5];
        dpvs_t6::top_bit_set(&mut expected, 3);
        assert_eq!(bits, expected, "the match ignores case and the bit sits at base + bone");
    }
}
