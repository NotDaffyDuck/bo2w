//! Posing every shown script model once a frame: its DObj, the vehicle parts
//! that move by themselves, the skinned surfaces at the LOD in view, and a
//! row saying where to draw it.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use render_frame::FocusedFrameSnapshot;
use render_scene::{LooseAtSpotLookup, FirstPersonLens, VisualStage, HostVisualStage};
use render_scene::{DetailRampArgs, DetailRampSkinnedTunable, LevelModelIllumAtlas};
use render_scene::{LevelPresentFacts, LevelScriptModelInstance, XModelCasterBound};
use xmodel_runtime::RigPoseRecord;

use super::anim_clock::{advance_script_anim, anim_ahead_secs};
use super::focus::{FocusSelect, DrawFocus};
use super::limp;
use super::lod::{DetailLevelWindow, lod_culled, submodel_camera_lod};
use super::pose_dobj::{pose_script_dobj_with_controller, skin_script_dobj_with_materials};
use super::product::{Compaction, ScriptPropPoseOutput, ScriptOwnerRow, ScriptPosedAsset};
use super::skels::shown_skeletons;
use super::state::ScriptPropSkeletons;
use super::wheel_rig::turn_wheels;
use super::{HeliRotors, TurretGun, VehicleWheels, has_rotors, has_turret_gun, has_wheels};
use crate::motion::model_materials::PreparedModelMaterials;
use crate::occupancy::shard_piece::{fpv_cull_planes, sphere_outside_frustum};
use crate::occupancy::debris_phys::PropPhysicsShape;
use crate::occupancy::zm_ragdoll::Ragdolls;
use crate::occupancy::t6_body::SlowGuard;
use crate::{model_light_offset, script_prop_light_extent};

/// What the pose system keeps between frames, and its working space.
#[derive(Default)]
pub(super) struct PoseScratch {
    compact: Compaction,
    kept_ids: HashSet<u32>,
    kept_assets: Vec<usize>,
}

/// The world-side resources the pose reads.
#[derive(SystemParam)]
pub(super) struct PoseWorld<'w> {
    present: Res<'w, LevelPresentFacts>,
    host_scene: Res<'w, HostVisualStage>,
    at_point: Res<'w, LooseAtSpotLookup>,
    mat_list: Res<'w, PreparedModelMaterials>,
    light_atlas: Option<Res<'w, LevelModelIllumAtlas>>,
    map: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    /// bo2zm: the world a limp body falls onto.
    phys_clip: Option<Res<'w, PropPhysicsShape>>,
}

/// What changes from frame to frame: the clock, the clips, who looks, and
/// the models to pose.
#[derive(SystemParam)]
pub(super) struct PoseFeeds<'w, 's> {
    lod_dvar: Res<'w, DetailRampSkinnedTunable>,
    anims: Option<Res<'w, assets::StagedXAnims>>,
    frame_clock: Option<Res<'w, net::FrameClock>>,
    snapshot: Option<Res<'w, net::ShownWorldFrame>>,
    lenses: Query<'w, 's, (&'static GlobalTransform, &'static Projection, &'static Camera), With<FirstPersonLens>>,
    models: Query<'w, 's, (Entity, &'static LevelScriptModelInstance, &'static Transform, &'static Visibility)>,
    work: Local<'s, PoseScratch>,
}

/// Which of an owner's moving parts are driven this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Drivers {
    wheels_moved: bool,
    /// bo2zm: wheels read off the model's wheel tags (a bus), not the RC-XD's.
    bus_wheels: bool,
    rotors_spun: bool,
    turret_aimed: bool,
}

/// What scene entity `entnum` is subject to in the scene.
struct SceneSlot<'a> {
    skipped: bool,
    skin_rows: &'a [dpvs_t6::EntitySkinSlotRecord],
}

fn scene_slot(scene: &VisualStage, entnum: u32) -> SceneSlot<'_> {
    let skin_rows = scene
        .entity_bone_surface_set(entnum)
        .map(|surfs| surfs.entries.as_slice()).unwrap_or(&[][..]);
    SceneSlot { skipped: scene.entity_not_drawn(entnum), skin_rows }
}

/// Moves the vehicle parts of owner `owner_id` on by a frame and says how.
/// What drives is decided by the DObj held from the frame before; each
/// driver's key goes in the state's pose revision, so a changed pose is not
/// taken for the one already made.
fn drive_vehicles<'s>(
    held: &mut ScriptPropSkeletons,
    owner_id_in: u32,
    placed: &Transform,
    base: &'s RigPoseRecord,
) -> (Cow<'s, RigPoseRecord>, Drivers) {
    let idle = (Cow::Borrowed(base), Drivers::default());
    let Some(dobj) = held.by_id.get(&owner_id_in).map(|kept| Arc::clone(&kept.dobj)) else {
        return idle;
    };
    let wheels_moved = has_wheels(&dobj);
    let bus_wheels = !wheels_moved && held.by_id.get(&owner_id_in).is_some_and(|kept| kept.rig.is_some());
    let drivers = Drivers {
        rotors_spun: has_rotors(&dobj),
        wheels_moved,
        bus_wheels,
        turret_aimed: base.turret_aim.is_some() && has_turret_gun(&dobj),
    };
    if drivers == Drivers::default() {
        return idle;
    }

    let mut edited = base.clone();
    if drivers.rotors_spun {
        let spin = held.rotors.entry(owner_id_in).or_insert_with(HeliRotors::new);
        spin.advance();
        edited.pose_epoch = spin.key();
        spin.hide(&dobj, &mut edited.hidden_mask);
        super::rotor_log(&dobj, spin, "roster");
    }
    if drivers.wheels_moved {
        let rolling = held.wheels.entry(owner_id_in).or_insert_with(VehicleWheels::new);
        rolling.advance(placed.translation, placed.rotation, edited.vehicle_steer);
        edited.pose_epoch = rolling.key();
    }
    if drivers.bus_wheels
        && let Some(kept) = held.by_id.get_mut(&owner_id_in)
        && let Some(rig) = kept.rig.as_ref()
    {
        kept.track.roll(rig, placed.translation, placed.rotation);
        edited.pose_epoch = kept.track.keyed_revision(edited.pose_epoch);
    }
    if let (true, Some(aim)) = (drivers.turret_aimed, edited.turret_aim) {
        let barrel = held.turrets.entry(owner_id_in).or_insert_with(TurretGun::new);
        barrel.advance(placed.rotation, aim);
        edited.pose_epoch = barrel.key();
    }
    (Cow::Owned(edited), drivers)
}

/// The same for every owner in a frame: where the eye is and what it sees.
struct FrameView {
    view_eye: Option<[f32; 3]>,
    ramp: DetailRampArgs,
    frustum: Vec<[f32; 4]>,
    lead_secs: f32,
    /// bo2zm: the client clock, which a limp body's fall runs on.
    clock_ms: Option<i32>,
}

impl FrameView {
    fn lod_view(&self, pos: [f32; 3]) -> DetailLevelWindow {
        DetailLevelWindow { spot: pos, eye_pos: self.view_eye.map(Vec3::from_array), ramp: self.ramp }
    }
}

/// Where a frame's results go.
struct Sinks<'a> {
    dobjs: &'a mut ScriptPropSkeletons,
    product: &'a mut ScriptPropPoseOutput,
    log: &'a mut DrawFocus,
    kept_assets: &'a mut Vec<usize>,
    kept_ids: &'a mut HashSet<u32>,
}

/// Why an owner is left out of this frame.
enum Skip {
    /// Worth telling the focus log.
    Because(&'static str),
    /// Nothing to say; the log keeps its default reason.
    Quietly,
}

/// What the posing of one owner needs to know about it.
struct OwnerCase<'c> {
    script_id: u32,
    instance: &'c LevelScriptModelInstance,
    state: &'c RigPoseRecord,
    drivers: Drivers,
    skels: &'c [&'c asset_model::MeshRig],
    viewer_detail_levels: &'c [Option<u8>],
    lod_view: DetailLevelWindow,
    skin_rows: &'c [dpvs_t6::EntitySkinSlotRecord],
    scene_from_local: Mat4,
}

/// Everything one frame's posing reads, and what it writes.
struct Poser<'a> {
    model_total: &'a asset_world::LevelModelPlacementRoster,
    mat_list: &'a PreparedModelMaterials,
    anims: Option<&'a assets::StagedXAnims>,
    gfx: &'a VisualStage,
    at_point: &'a LooseAtSpotLookup,
    view: FrameView,
    watched: Option<u32>,
    sinks: Sinks<'a>,
    /// bo2zm: the world limp bodies fall onto, and their falls.
    solids: Option<&'a asset_world::CollisionClipData>,
    ragdolls: &'a mut Ragdolls,
}

impl Poser<'_> {
    /// Poses one owner and adds its row; an owner that is not drawn leaves
    /// the reason with the focus log when it is the one being watched.
    fn pose_owner(&mut self, item: (Entity, &LevelScriptModelInstance, &Transform, &Visibility)) {
        let instance = item.1;
        let script_id = instance
            .server_owned_model_owner
            .and_then(|authority| authority.scripted_model())
            .map(|id| id.encode_for_net());
        let watched = self.watched.filter(|wanted| script_id == Some(*wanted));
        let model_name = &instance.active_mesh.0;
        if let Some(id) = watched {
            let why = "unclassified_branch";
            self.sinks.log.decline(id, model_name, why);
        }
        let verdict = self.try_pose(item, script_id);
        if let (Err(Skip::Because(reason)), Some(id)) = (verdict, watched) {
            self.sinks.log.decline(id, model_name, reason);
        }
    }

    fn try_pose(&mut self, (piece, instance, placement, shown): (Entity, &LevelScriptModelInstance, &Transform, &Visibility), script_id: Option<u32>) -> Result<(), Skip> {
        if matches!(shown, Visibility::Hidden) {
            return Err(Skip::Because("hidden"));
        }
        let entnum = instance.game_actor_number.map(u32::from);
        let slot = entnum.map(|num| scene_slot(self.gfx, num));
        if slot.as_ref().is_some_and(|found| found.skipped) {
            return Err(Skip::Because("scene_ent_skip"));
        }

        let (state, drivers) = self.settle_state(instance, placement, script_id);
        let st: &RigPoseRecord = &state;
        let catalog = self.model_total;
        let Some(skels) = shown_skeletons(catalog, st) else {
            let why = "skeleton_unavailable";
            limp::raglog_skip(instance, why);
            return Err(Skip::Because(why));
        };
        // bo2zm: a limp body is drawn where its fall has taken it.
        let shift = limp::shift(self.ragdolls, instance);
        let origin = (placement.translation + shift).to_array();
        if lod_culled(&skels, origin, self.view.view_eye, self.view.ramp) {
            let why = "lod_culled";
            limp::raglog_skip(instance, why);
            return Err(Skip::Because(why));
        }
        let lod_view = self.view.lod_view(origin);
        let viewer_detail_levels: Vec<Option<u8>> = skels
            .iter()
            .map(|bone_rig| submodel_camera_lod(bone_rig, Some(lod_view)))
            .collect::<_>();

        let draw_asset_slot = match script_id {
            // A model with no owner draws what an owner of the same model
            // posed this frame, or last.
            None => self
                .sinks
                .product
                .draw_asset_slot(&instance.active_mesh, st, &viewer_detail_levels)
                .ok_or(Skip::Because("posed_asset_unavailable"))?,
            Some(script_id) => {
                let case = OwnerCase {
                    script_id,
                    instance,
                    state: st,
                    drivers,
                    skels: &skels,
                    viewer_detail_levels: &viewer_detail_levels,
                    lod_view,
                    skin_rows: slot.as_ref().map_or(&[], |found| found.skin_rows),
                    scene_from_local: placement.to_matrix(),
                };
                self.owner_asset(&case)?
            }
        };
        self.sinks.kept_assets.push(draw_asset_slot);

        let radius = skels.first().and_then(|bone_rig| bone_rig.radius).map_or(1.0, |reach| reach.max(1.0));
        let light_half = script_prop_light_extent(catalog, &instance.puppet_state.composition.models);
        // BO2 reads a model's light at its lighting origin, not at its feet.
        let light_at = (placement.transform_point(skels.first().map_or(Vec3::ZERO, |&model0| Vec3::from_array(model_light_offset(model0)))) + shift).to_array();
        let row = ScriptOwnerRow {
            ent: piece,
            draw_asset_slot,
            spectated_entity_id: self.watched.filter(|wanted| script_id == Some(*wanted)),
            mdl: instance.active_mesh.clone(),
            world_from_local: placement.to_matrix(),
            entnum,
            hidden_from_viewer: sphere_outside_frustum(origin, radius, &self.view.frustum),
            illum_origin: light_at,
            lookup_backup: self.at_point.fallback(light_at, light_half),
            caster_bound: XModelCasterBound { origin, reach: radius },
        };
        self.sinks.product.rows.push(row);
        Ok(())
    }

    /// The owner's state this frame: animations stepped ahead, then the
    /// vehicle parts driven on top.
    fn settle_state<'i>(
        &mut self,
        instance: &'i LevelScriptModelInstance,
        placement: &Transform,
        script_id: Option<u32>,
    ) -> (Cow<'i, RigPoseRecord>, Drivers) {
        let anims = self.anims;
        let loops = |name: &str| {
            let clip = anims?.0.clip(asset_core::AssetCategoryScope::T6, name)?;
            Some(clip.looping)
        };
        let stepped = advance_script_anim(&instance.puppet_state, self.view.lead_secs, loops);
        let Some(id) = script_id else {
            let plain = match stepped {
                Some(held_row) => Cow::Owned(held_row),
                None => Cow::Borrowed(&instance.puppet_state),
            };
            return (plain, Drivers::default());
        };
        match stepped {
            None => drive_vehicles(self.sinks.dobjs, id, placement, &instance.puppet_state),
            Some(held_row) => {
                let (driven, drivers) = drive_vehicles(self.sinks.dobjs, id, placement, &held_row);
                (Cow::Owned(driven.into_owned()), drivers)
            }
        }
    }

    /// The posed asset an owned model draws: the one already posed for the
    /// same state and LODs, or a new one.
    fn owner_asset(&mut self, case: &OwnerCase<'_>) -> Result<usize, Skip> {
        let id = case.script_id;
        let current = &case.instance.active_mesh;
        let Some(dobj) = self.sinks.dobjs.dobj_for(id, self.model_total, case.state, self.mat_list) else {
            self.sinks.dobjs.by_id.remove(&id);
            limp::raglog_skip(case.instance, "compose");
            return Err(Skip::Quietly);
        };
        self.sinks.kept_ids.insert(id);
        if let Some(posed) = self.limp_asset(case, &dobj) {
            return posed;
        }
        if let Some(known) = self.sinks.product.draw_asset_slot(current, case.state, case.viewer_detail_levels) {
            return Ok(known);
        }

        let anims = self.anims;
        let request = case
            .state
            .look_up_pose_query(|name| anims?.0.clip(asset_core::AssetCategoryScope::T6, name))
            .map_err(|_| Skip::Quietly)?;
        let held = &*self.sinks.dobjs;
        let on = case.drivers;
        let wheels = held.wheels.get(&id).filter(|_| on.wheels_moved);
        let rotors = held.rotors.get(&id).filter(|_| on.rotors_spun);
        let turret = held.turrets.get(&id).filter(|_| on.turret_aimed);
        let bus = held
            .by_id
            .get(&id)
            .filter(|_| on.bus_wheels)
            .and_then(|kept| kept.rig.as_ref().map(|rig| (rig, &kept.track)));
        let posed = pose_script_dobj_with_controller(
            Some(self.model_total),
            case.skels,
            &dobj,
            &request,
            Some(case.lod_view),
            case.skin_rows,
            |dobj, _, locals| {
                if let Some(part) = wheels {
                    part.apply(dobj, locals);
                }
                if let Some((rig, track)) = bus {
                    turn_wheels(dobj, locals, rig, track);
                }
                if let Some(part) = rotors {
                    part.apply(dobj, locals);
                }
                if let Some(part) = turret {
                    part.apply(dobj, locals);
                }
            },
        );
        let (surfaces, authored) = posed.ok_or(Skip::Quietly)?;
        self.sinks.product.note_unbound(&current.0, &surfaces, &authored);
        let fresh = ScriptPosedAsset {
            key: current.clone(),
            puppet_state: case.state.clone(),
            viewer_detail_levels: case.viewer_detail_levels.to_vec(),
            surfaces,
            authored,
        };
        Ok(self.sinks.product.store(self.sinks.kept_assets, fresh))
    }

    /// bo2zm: a limp body is posed by its fall, from the pose it had when it went limp. `None`
    /// when the owner is not limp (or its fall cannot be posed: it keeps its animation).
    fn limp_asset(&mut self, case: &OwnerCase<'_>, dobj: &xmodel_runtime::PosedSkeleton) -> Option<Result<usize, Skip>> {
        let clock_millis = self.view.clock_ms?;
        let (ragdoll, pose) = limp::pose_limp(
            self.ragdolls,
            case.script_id,
            case.instance,
            case.state,
            dobj,
            case.scene_from_local,
            clock_millis,
            self.solids,
            self.view.lead_secs,
            self.anims,
        )?;
        let current = &case.instance.active_mesh;
        let key = limp::limp_key(case.instance, case.script_id, &ragdoll, &pose);
        if let Some(known) = self.sinks.product.draw_asset_slot(current, &key, case.viewer_detail_levels) {
            return Some(Ok(known));
        }
        let anims = self.anims;
        let request_in = case.state.look_up_pose_query(|name| anims?.0.clip(asset_core::AssetCategoryScope::T6, name));
        let Ok(request_in) = request_in else {
            return Some(Err(Skip::Quietly));
        };
        let skinned = skin_script_dobj_with_materials(
            Some(self.model_total),
            case.skels,
            dobj,
            &pose.bone_mats,
            &request_in.hidden_mask,
            Some(case.lod_view),
            case.skin_rows,
        );
        let Some((surf_list, authored)) = skinned else {
            return Some(Err(Skip::Quietly));
        };
        self.sinks.product.note_unbound(&current.0, &surf_list, &authored);
        let fresh = ScriptPosedAsset {
            key: current.clone(),
            puppet_state: key,
            viewer_detail_levels: case.viewer_detail_levels.to_vec(),
            surfaces: surf_list,
            authored,
        };
        Some(Ok(self.sinks.product.store(self.sinks.kept_assets, fresh)))
    }
}

pub(super) fn pose_script_models(
    world: PoseWorld,
    feeds: PoseFeeds,
    mut dobjs: ResMut<ScriptPropSkeletons>,
    mut product_in: ResMut<ScriptPropPoseOutput>,
    mut log: ResMut<DrawFocus>,
    pick: Res<FocusSelect>,
    mut ragdolls: ResMut<Ragdolls>,
) {
    let _slow = SlowGuard::new("pose_script_models");
    let watched = pick.script_model_id();
    if let Some(id) = watched {
        log.begin_frm(id);
    }
    product_in.wipe_frame();
    let mut work = feeds.work;
    let PoseScratch { compact, kept_ids, kept_assets } = &mut *work;
    kept_assets.truncate(0);
    kept_ids.clear();

    let map = match world.map {
        Some(map) if world.light_atlas.is_some() && world.present.spawned => map,
        _ => {
            if let Some(id) = watched {
                let why = "producer_unavailable";
                log.frame = Some(FocusedFrameSnapshot::never_found(id, why));
            }
            product_in.unavailable_producer = true;
            product_in.assets.truncate(0);
            dobjs.by_id.clear();
            return;
        }
    };
    if map.is_changed() {
        product_in.assets.truncate(0);
    }

    let lenses = &feeds.lenses;
    let view = FrameView {
        view_eye: lenses.single().ok().map(|(at, _, _)| at.translation().to_array()),
        ramp: feeds.lod_dvar.args(),
        frustum: fpv_cull_planes(lenses),
        lead_secs: anim_ahead_secs(feeds.frame_clock.as_deref(), feeds.snapshot.as_deref()),
        clock_ms: feeds.frame_clock.as_deref().filter(|clock| clock.started()).map(|clock| clock.time()),
    };
    let clock_ms = view.clock_ms;
    let mut poser = Poser {
        model_total: &map,
        mat_list: &world.mat_list,
        anims: feeds.anims.as_deref(),
        gfx: &world.host_scene.scene,
        at_point: &world.at_point,
        view,
        watched,
        sinks: Sinks {
            dobjs: &mut dobjs,
            product: &mut product_in,
            log: &mut log,
            kept_assets,
            kept_ids,
        },
        solids: world.phys_clip.as_deref().and_then(|clip| clip.0.as_deref()),
        ragdolls: &mut ragdolls,
    };
    for owner in feeds.models.iter() {
        poser.pose_owner(owner);
    }
    if let Some(clock_millis) = clock_ms {
        ragdolls.forget_unseen(clock_millis);
    }

    dobjs.keep_only(kept_ids);
    compact.plan(product_in.assets.len(), kept_assets);
    let before = std::mem::take(&mut product_in.assets);
    product_in.assets = compact.apply(before);
    for row in product_in.rows.iter_mut() {
        row.draw_asset_slot = compact.new_index(row.draw_asset_slot).unwrap_or(row.draw_asset_slot);
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn an_owner_not_yet_held_has_no_vehicle_parts_to_drive() {
        let mut held = ScriptPropSkeletons::default();
        let bind = RigPoseRecord::neutral_pose("rcbomb".to_owned(), 1, 1);
        let (driven_state, drivers) = drive_vehicles(&mut held, 7, &Transform::default(), &bind);
        assert!(matches!(driven_state, Cow::Borrowed(_)));
        assert_eq!(drivers, Drivers::default());
        let parts = [held.wheels.len(), held.rotors.len(), held.turrets.len()];
        assert_eq!(parts, [0, 0, 0]);
    }
}
