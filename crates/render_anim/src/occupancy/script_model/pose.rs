//! Posing every shown script model once a frame: its DObj, the vehicle parts
//! that move by themselves, the skinned surfaces at the LOD in view, and a
//! row saying where to draw it.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use render_frame::RenderFocusFrame;
use render_scene::{
    DynAtPointLookup, FpvLens, GfxScene, HostGfxScene, LodRampArgs, LodRampSkinnedDvar,
    WorldModelLightingAtlas, WorldPresentFacts, WorldScriptModelInstance, XModelCasterBound,
};
use xmodel_runtime::DObjSemanticState;

use super::anim_clock::{advance_script_anim, anim_ahead_secs};
use super::focus::{FocusSelect, RenderFocus};
use super::lod::{DObjLodView, lod_culled, submodel_camera_lod};
use super::pose_dobj::pose_script_dobj_with_controller;
use super::product::{Compaction, ScriptModelPoseProduct, ScriptOwnerRow, ScriptPosedAsset};
use super::skels::presented_skels;
use super::state::ScriptModelDobjs;
use super::{HeliRotors, TurretGun, VehicleWheels, has_rotors, has_turret_gun, has_wheels};
use crate::anim::model_materials::PreparedModelMaterials;
use crate::occupancy::dyn_ent::{fpv_frustum_planes, sphere_behind_frustum};
use crate::occupancy::t6_body::SlowGuard;
use crate::script_model_lighting_box_half;

/// What the pose system keeps between frames, and its working space.
#[derive(Default)]
pub(super) struct PoseScratch {
    live_assets: Vec<usize>,
    live_ids: HashSet<u32>,
    compaction: Compaction,
}

#[derive(SystemParam)]
pub(super) struct PoseInputs<'w, 's> {
    catalog: Option<Res<'w, asset_world::MapXModelSceneCatalog>>,
    atlas: Option<Res<'w, WorldModelLightingAtlas>>,
    atpoint: Res<'w, DynAtPointLookup>,
    facts: Res<'w, WorldPresentFacts>,
    xanims: Option<Res<'w, assets::PreparedXAnims>>,
    prepared: Res<'w, PreparedModelMaterials>,
    lod_skinned: Res<'w, LodRampSkinnedDvar>,
    gfx_scene: Res<'w, HostGfxScene>,
    clock: Option<Res<'w, net::FrameClock>>,
    presented: Option<Res<'w, net::PresentedSnapshot>>,
    cameras: Query<'w, 's, (&'static GlobalTransform, &'static Projection, &'static Camera), With<FpvLens>>,
    owners: Query<
        'w,
        's,
        (Entity, &'static WorldScriptModelInstance, &'static Transform, &'static Visibility),
    >,
    scratch: Local<'s, PoseScratch>,
}

/// Which of an owner's moving parts are driven this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Drivers {
    wheels: bool,
    rotors: bool,
    turret: bool,
}

/// What scene entity `entnum` is subject to in the scene.
struct SceneSlot<'a> {
    hidden: bool,
    skin_entries: &'a [dpvs_t6::SceneEntSkinEntry],
}

fn scene_slot(scene: &GfxScene, entnum: u32) -> SceneSlot<'_> {
    SceneSlot {
        hidden: scene.scene_ent_skips_draw(entnum),
        skin_entries: scene
            .scene_ent_skinned_surfs(entnum)
            .map(|surfs| surfs.entries.as_slice())
            .unwrap_or(&[]),
    }
}

/// Moves the vehicle parts of owner `id` on by a frame and says how.
/// What drives is decided by the DObj held from the frame before; each
/// driver's key goes in the state's pose revision, so a changed pose is not
/// taken for the one already made.
fn drive_vehicles<'s>(
    persist: &mut ScriptModelDobjs,
    id: u32,
    transform: &Transform,
    state: &'s DObjSemanticState,
) -> (Cow<'s, DObjSemanticState>, Drivers) {
    let mut drivers = Drivers::default();
    let mut state = Cow::Borrowed(state);
    let Some(dobj) = persist.by_id.get(&id).map(|held| Arc::clone(&held.dobj)) else {
        return (state, drivers);
    };
    if has_rotors(&dobj) {
        let rotors = persist.rotors.entry(id).or_insert_with(HeliRotors::new);
        rotors.advance();
        let keyed = state.to_mut();
        keyed.pose_revision = rotors.key();
        rotors.hide(&dobj, &mut keyed.hide_part_bits);
        drivers.rotors = true;
    }
    if has_wheels(&dobj) {
        let wheels = persist.wheels.entry(id).or_insert_with(VehicleWheels::new);
        wheels.advance(transform.translation, transform.rotation, state.vehicle_steer);
        state.to_mut().pose_revision = wheels.key();
        drivers.wheels = true;
    }
    if let Some(aim) = state.turret_aim
        && has_turret_gun(&dobj)
    {
        let gun = persist.turrets.entry(id).or_insert_with(TurretGun::new);
        gun.advance(transform.rotation, aim);
        state.to_mut().pose_revision = gun.key();
        drivers.turret = true;
    }
    (state, drivers)
}

/// Everything one frame's posing reads, and what it writes.
struct Poser<'a> {
    catalog: &'a asset_world::MapXModelSceneCatalog,
    prepared: &'a PreparedModelMaterials,
    xanims: Option<&'a assets::PreparedXAnims>,
    scene: &'a GfxScene,
    atpoint: &'a DynAtPointLookup,
    planes: Vec<[f32; 4]>,
    eye: Option<[f32; 3]>,
    ramp: LodRampArgs,
    ahead_s: f32,
    focus_id: Option<u32>,
    persist: &'a mut ScriptModelDobjs,
    product: &'a mut ScriptModelPoseProduct,
    focus: &'a mut RenderFocus,
    live_assets: &'a mut Vec<usize>,
    live_ids: &'a mut HashSet<u32>,
}

impl Poser<'_> {
    /// Records why the focused owner (if this is it) is not drawn, and gives
    /// up on it.
    fn refuse<T>(
        &mut self,
        focused: Option<u32>,
        model: &asset_world::MapXModelAssetKey,
        why: &'static str,
    ) -> Option<T> {
        if let Some(id) = focused {
            self.focus.refuse(id, &model.0, why);
        }
        None
    }

    /// Poses one owner and adds its row. `None` when it is not drawn.
    fn pose_owner(
        &mut self,
        (entity, owner, transform, visibility): (Entity, &WorldScriptModelInstance, &Transform, &Visibility),
    ) -> Option<()> {
        let catalog = self.catalog;
        let owner_id = owner
            .authority_owner
            .and_then(|owner| owner.script_model())
            .map(|id| id.to_wire());
        let focused = self.focus_id.filter(|wanted| owner_id == Some(*wanted));
        if let Some(id) = focused {
            self.focus.refuse(id, &owner.current_model.0, "unclassified_branch");
        }
        if *visibility == Visibility::Hidden {
            return self.refuse(focused, &owner.current_model, "hidden");
        }
        let entnum = owner.gentity_number.map(u32::from);
        let scene = self.scene;
        let slot = entnum.map(|entnum| scene_slot(scene, entnum));
        if slot.as_ref().is_some_and(|slot| slot.hidden) {
            return self.refuse(focused, &owner.current_model, "scene_ent_skip");
        }

        let xanims = self.xanims;
        let advanced = advance_script_anim(&owner.dobj_state, self.ahead_s, |clip| {
            let clip = xanims?.0.clip(asset_core::AssetNamespace::T6, clip)?;
            Some(clip.looping)
        });
        let state = advanced.as_ref().unwrap_or(&owner.dobj_state);
        let (state, drivers) = match owner_id {
            Some(id) => drive_vehicles(self.persist, id, transform, state),
            None => (Cow::Borrowed(state), Drivers::default()),
        };
        let state: &DObjSemanticState = &state;

        let Some(skels) = presented_skels(catalog, state) else {
            return self.refuse(focused, &owner.current_model, "skeleton_unavailable");
        };
        let origin = transform.translation.to_array();
        if lod_culled(&skels, origin, self.eye, self.ramp) {
            return self.refuse(focused, &owner.current_model, "lod_culled");
        }
        let lod_view = Some(DObjLodView {
            origin,
            eye: self.eye.map(Vec3::from_array),
            ramp: self.ramp,
        });
        let camera_lods: Vec<Option<u8>> = skels
            .iter()
            .map(|skel| submodel_camera_lod(skel, lod_view))
            .collect();

        let asset_index = match owner_id {
            Some(id) => {
                let dobj = self.persist.dobj_for(id, catalog, state, self.prepared);
                let Some(dobj) = dobj else {
                    self.persist.by_id.remove(&id);
                    return None;
                };
                self.live_ids.insert(id);
                let index = match self.product.asset_index(&owner.current_model, state, &camera_lods) {
                    Some(index) => index,
                    None => {
                        let request = state
                            .resolve_request(|name| xanims?.0.clip(asset_core::AssetNamespace::T6, name))
                            .ok()?;
                        let persist = &*self.persist;
                        let (wheels, rotors, turret) = (
                            persist.wheels.get(&id).filter(|_| drivers.wheels),
                            persist.rotors.get(&id).filter(|_| drivers.rotors),
                            persist.turrets.get(&id).filter(|_| drivers.turret),
                        );
                        let (surfaces, authored) = pose_script_dobj_with_controller(
                            Some(catalog),
                            &skels,
                            &dobj,
                            &request,
                            lod_view,
                            slot.as_ref().map_or(&[], |slot| slot.skin_entries),
                            |dobj, _, locals| {
                                if let Some(wheels) = wheels {
                                    wheels.apply(dobj, locals);
                                }
                                if let Some(rotors) = rotors {
                                    rotors.apply(dobj, locals);
                                }
                                if let Some(turret) = turret {
                                    turret.apply(dobj, locals);
                                }
                            },
                        )?;
                        self.product.note_unbound(&owner.current_model.0, &surfaces, &authored);
                        self.product.store(
                            self.live_assets,
                            ScriptPosedAsset {
                                key: owner.current_model.clone(),
                                dobj_state: state.clone(),
                                camera_lods: camera_lods.clone(),
                                surfaces,
                                authored,
                            },
                        )
                    }
                };
                index
            }
            // A model with no owner draws what an owner of the same model
            // posed this frame, or last.
            None => {
                let Some(index) = self.product.asset_index(&owner.current_model, state, &camera_lods)
                else {
                    return self.refuse(focused, &owner.current_model, "posed_asset_unavailable");
                };
                index
            }
        };
        self.live_assets.push(asset_index);

        let box_half = script_model_lighting_box_half(catalog, &owner.dobj_state.composition.models);
        let radius = skels
            .first()
            .and_then(|skel| skel.radius)
            .unwrap_or(1.0)
            .max(1.0);
        self.product.rows.push(ScriptOwnerRow {
            entity,
            asset_index,
            focused_owner_id: focused,
            model: owner.current_model.clone(),
            world_from_local: transform.to_matrix(),
            entnum,
            camera_hidden: sphere_behind_frustum(origin, radius, &self.planes),
            lighting_origin: owner.lighting_origin,
            lookup_fallback: self.atpoint.fallback(owner.lighting_origin, box_half),
            caster_bound: XModelCasterBound { origin, radius },
        });
        Some(())
    }
}

pub(super) fn pose_script_models(
    inputs: PoseInputs,
    mut persist: ResMut<ScriptModelDobjs>,
    mut product: ResMut<ScriptModelPoseProduct>,
    select: Res<FocusSelect>,
    mut focus: ResMut<RenderFocus>,
) {
    let _slow = SlowGuard::new("pose_script_models");
    let PoseInputs {
        catalog,
        atlas,
        atpoint,
        facts,
        xanims,
        prepared,
        lod_skinned,
        gfx_scene,
        clock,
        presented,
        cameras,
        owners,
        mut scratch,
    } = inputs;
    let focus_id = select.script_model_id();
    if let Some(id) = focus_id {
        focus.begin_frame(id);
    }
    product.clear_frame();
    let PoseScratch {
        live_assets,
        live_ids,
        compaction,
    } = &mut *scratch;
    live_assets.clear();
    live_ids.clear();

    let Some(catalog) = catalog.filter(|_| atlas.is_some() && facts.spawned) else {
        if let Some(id) = focus_id {
            focus.frame = Some(RenderFocusFrame::unresolved(id, "producer_unavailable"));
        }
        product.producer_unavailable = true;
        product.assets.clear();
        persist.by_id.clear();
        return;
    };
    if catalog.is_changed() {
        product.assets.clear();
    }

    let mut poser = Poser {
        catalog: &catalog,
        prepared: &prepared,
        xanims: xanims.as_deref(),
        scene: &gfx_scene.scene,
        atpoint: &atpoint,
        planes: fpv_frustum_planes(&cameras),
        eye: cameras
            .single()
            .ok()
            .map(|(transform, _, _)| transform.translation().to_array()),
        ramp: lod_skinned.args(),
        ahead_s: anim_ahead_secs(clock.as_deref(), presented.as_deref()),
        focus_id,
        persist: &mut persist,
        product: &mut product,
        focus: &mut focus,
        live_assets,
        live_ids,
    };
    for owner in &owners {
        poser.pose_owner(owner);
    }

    persist.keep_only(live_ids);
    compaction.plan(product.assets.len(), live_assets);
    product.assets = compaction.apply(std::mem::take(&mut product.assets));
    for row in &mut product.rows {
        if let Some(index) = compaction.new_index(row.asset_index) {
            row.asset_index = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_not_yet_held_has_no_vehicle_parts_to_drive() {
        let mut persist = ScriptModelDobjs::default();
        let state = DObjSemanticState::bind_pose("rcbomb".to_owned(), 1, 1);
        let (driven, drivers) = drive_vehicles(&mut persist, 7, &Transform::default(), &state);
        assert!(matches!(driven, Cow::Borrowed(_)));
        assert_eq!(drivers, Drivers::default());
        assert!(persist.wheels.is_empty() && persist.rotors.is_empty() && persist.turrets.is_empty());
    }
}
