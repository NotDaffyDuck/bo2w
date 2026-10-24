//! The frame's list of things to draw: models, animated objects and brush
//! models, each with the slot limits the original scene had.

pub const SCENE_GFX_ENT_CAP: u32 = 0x80;
pub const SCENE_MODEL_CAP: u32 = 0x400;
pub const SCENE_DOBJ_CAP: u32 = 0x200;
pub const SCENE_BRUSH_CAP: u32 = 0x200;
/// The value an index table holds for an entity that has no scene slot.
pub const SCENE_INDEX_EMPTY: u16 = 0xffff;
/// Entity numbers of the first-person weapon models.
pub const SCENE_VIEWMODEL_ENTNUM: u32 = 0x7ff;
pub const SCENE_VIEWMODEL_LEFT_ENTNUM: u32 = 0x7fe;
pub const SCENE_VIEWMODEL_FX_FLAGS: u32 = 3;

/// Animated objects carrying any of these effect flags go in a small separate list.
const DOBJ_FLAG30_MASK: u32 = 0x30;
const DOBJ_FLAG30_CAP: u32 = 8;
/// An effect flag that needs its own per-entity effect slot.
const GFX_ENT_FLAG: u32 = 2;
/// A single-model object with none of these effect flags is drawn as a plain model.
const PLAIN_MODEL_FLAG_MASK: u32 = 0x34;

/// The entity number packed into a scene `info` word.
#[must_use]
pub fn scene_info_entnum(info: u32) -> u32 {
    (info >> 7) & 0xfff
}

/// `info` is the effect-slot index in the low 7 bits, then the entity number
/// (12 bits) with the effect flags above it.
pub fn pack_scene_info(entnum: u32, render_fx_flags: u32, gfx_ent: u32) -> u32 {
    (((entnum & 0xfff) | (render_fx_flags << 12)) << 7) | (gfx_ent & 0x7f)
}

/// Where an added object went, and the slot number it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneAddKind {
    Model(u32),
    Dobj(u32),
    /// Numbered after the ordinary animated objects.
    DobjFlag30(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneAddError {
    GfxEntOverflow,
    ModelOverflow,
    DobjOverflow,
    DobjFlag30Overflow,
    BrushOverflow,
    BrushNoSurfs,
}

#[derive(Clone, Copy, Debug)]
pub struct AddDObjArgs {
    pub render_fx_flags: u32,
    pub material_time: f32,
    pub has_tree: bool,
    pub num_models: u8,
    pub pose: Option<AddDObjPose>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddDObjPose {
    pub origin: [f32; 3],
    pub lighting_origin: [f32; 3],
    pub radius: Option<f32>,
    pub entnum: u32,
    pub quat: Option<[f32; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxSceneModel {
    pub origin: [f32; 3],
    pub lighting_origin: [f32; 3],
    pub radius: Option<f32>,
    pub info: u32,
    pub scale: f32,
    pub quat: Option<[f32; 4]>,
    pub posed_bounds: Option<dpvs_t6::Bounds>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxSceneDobj {
    pub origin: [f32; 3],
    pub lighting_origin: [f32; 3],
    pub radius: Option<f32>,
    pub info: u32,
    pub quat: Option<[f32; 4]>,
    /// The skin pass's claim on this object; see `dpvs_t6::scene_dobj_skin_begin`.
    pub cull_gate: u32,
    pub posed_bounds: Option<dpvs_t6::Bounds>,
    pub lods: Vec<i8>,
    pub skinned_surfs: Option<SceneEntSkinnedSurfs>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneEntSkinnedSurfs {
    pub entries: Vec<dpvs_t6::SceneEntSkinEntry>,
    pub summary: dpvs_t6::PreSkinSummary,
}

#[derive(Clone, Copy, Debug)]
pub struct AddBModelArgs {
    /// The brush model's surface count; zero means there is nothing to draw.
    pub surf_id: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddBModelPose {
    pub model_index: u32,
    pub origin: [f32; 3],
    pub quat: Option<[f32; 4]>,
    pub param_4: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxSceneBrush {
    pub model_index: u32,
    pub origin: [f32; 3],
    pub quat: Option<[f32; 4]>,
    pub param_4: Option<u16>,
}

/// One frame's scene. `scene_model_index` and `scene_dobj_index` map an entity
/// number to its position in `scene_models` / `scene_dobjs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GfxScene {
    gfx_ent_count: u32,
    scene_model_count: u32,
    scene_dobj_count: u32,
    scene_dobj_flag30_count: u32,
    scene_brush_count: u32,
    pub scene_models: Vec<GfxSceneModel>,
    pub scene_dobjs: Vec<GfxSceneDobj>,
    pub scene_brushes: Vec<GfxSceneBrush>,
    pub scene_model_index: Vec<u16>,
    pub scene_dobj_index: Vec<u16>,
    pub scene_ent_visible: Vec<u8>,
    /// The visibility walk has run, so an entity it did not mark really is hidden.
    pub scene_ent_walked: bool,
}

/// Takes the next slot of a counted list with `cap` places. The count keeps
/// rising with each ask but is pinned at `cap` once the list is full, so a full
/// list stays full.
fn take_slot(count: &mut u32, cap: u32) -> Option<u32> {
    let slot = *count;
    *count = if slot < cap { slot + 1 } else { cap };
    (slot < cap).then_some(slot)
}

/// The list position an entity's index-table entry points at, if it has one.
fn live_slot(table: &[u16], entnum: u32) -> Option<usize> {
    table
        .get(entnum as usize)
        .copied()
        .filter(|&slot| slot != SCENE_INDEX_EMPTY)
        .map(usize::from)
}

/// Points `entnum` at `slot`, growing the table to cover every entity first.
/// Entities past the configured count get no slot.
fn set_slot(table: &mut Vec<u16>, entnum: u32, slot: usize) {
    let entities = dpvs_t6::GFX_CFG_ENT_COUNT as usize;
    if entnum as usize >= entities {
        return;
    }
    if table.len() != entities {
        table.resize(entities, SCENE_INDEX_EMPTY);
    }
    table[entnum as usize] = slot as u16;
}

fn seed_bounds(origin: [f32; 3], radius: Option<f32>) -> Option<dpvs_t6::Bounds> {
    radius.map(|r| dpvs_t6::scene_dobj_initial_bounds(origin, r))
}

impl GfxScene {
    /// Empties the scene for a new frame, with every entity unslotted and invisible.
    pub fn clear(&mut self) {
        let entities = dpvs_t6::GFX_CFG_ENT_COUNT as usize;
        *self = Self {
            scene_model_index: vec![SCENE_INDEX_EMPTY; entities],
            scene_dobj_index: vec![SCENE_INDEX_EMPTY; entities],
            scene_ent_visible: vec![0; entities],
            ..Self::default()
        };
    }

    pub fn mark_scene_ent_visible(&mut self, entnum: u32) {
        dpvs_t6::mark_scene_ent_visible(&mut self.scene_ent_visible, entnum);
    }

    #[must_use]
    pub fn scene_ent_visible(&self, entnum: u32) -> bool {
        dpvs_t6::scene_ent_is_visible(&self.scene_ent_visible, entnum)
    }

    #[must_use]
    pub fn scene_ent_hidden(&self, entnum: u32) -> bool {
        self.scene_ent_walked && self.scene_ent_live(entnum) && !self.scene_ent_visible(entnum)
    }

    /// Hidden by the visibility walk, or skinned and found to have nothing to draw.
    #[must_use]
    pub fn scene_ent_skips_draw(&self, entnum: u32) -> bool {
        self.scene_ent_hidden(entnum) || self.scene_ent_surface_count(entnum) == Some(0)
    }

    #[must_use]
    pub fn scene_dobj_live(&self, entnum: u32) -> bool {
        live_slot(&self.scene_dobj_index, entnum).is_some()
    }

    #[must_use]
    pub fn scene_model_live(&self, entnum: u32) -> bool {
        live_slot(&self.scene_model_index, entnum).is_some()
    }

    #[must_use]
    pub fn scene_ent_live(&self, entnum: u32) -> bool {
        self.scene_dobj_live(entnum) || self.scene_model_live(entnum)
    }

    /// Adds a brush model; returns its slot number.
    pub fn add_bmodel(&mut self, args: AddBModelArgs, pose: AddBModelPose) -> Result<u32, SceneAddError> {
        if args.surf_id == 0 {
            return Err(SceneAddError::BrushNoSurfs);
        }
        let slot = take_slot(&mut self.scene_brush_count, SCENE_BRUSH_CAP)
            .ok_or(SceneAddError::BrushOverflow)?;
        self.scene_brushes.push(GfxSceneBrush {
            model_index: pose.model_index,
            origin: pose.origin,
            quat: pose.quat,
            param_4: pose.param_4,
        });
        Ok(slot)
    }

    /// Adds an animated object, as a plain model when it is a single bare model.
    /// The object is only recorded (and findable by entity number) if `args.pose` is given.
    pub fn add_dobj(&mut self, args: AddDObjArgs) -> Result<SceneAddKind, SceneAddError> {
        let flags = args.render_fx_flags;
        let needs_gfx_ent = args.material_time != 0.0 || flags & GFX_ENT_FLAG != 0;
        let gfx_ent = if needs_gfx_ent {
            take_slot(&mut self.gfx_ent_count, SCENE_GFX_ENT_CAP).ok_or(SceneAddError::GfxEntOverflow)?
        } else {
            0
        };

        let plain_model = flags & PLAIN_MODEL_FLAG_MASK == 0 && !args.has_tree && args.num_models == 1;
        let kind = if plain_model {
            let slot = take_slot(&mut self.scene_model_count, SCENE_MODEL_CAP)
                .ok_or(SceneAddError::ModelOverflow)?;
            SceneAddKind::Model(slot)
        } else if flags & DOBJ_FLAG30_MASK == 0 {
            let slot = take_slot(&mut self.scene_dobj_count, SCENE_DOBJ_CAP)
                .ok_or(SceneAddError::DobjOverflow)?;
            SceneAddKind::Dobj(slot)
        } else {
            let slot = take_slot(&mut self.scene_dobj_flag30_count, DOBJ_FLAG30_CAP)
                .ok_or(SceneAddError::DobjFlag30Overflow)?;
            SceneAddKind::DobjFlag30(SCENE_DOBJ_CAP + slot)
        };
        if let Some(pose) = args.pose {
            self.record(kind, flags, gfx_ent, pose);
        }
        Ok(kind)
    }

    /// Stores a posed object and points its entity number at it. The flag-30
    /// objects are counted but not stored: nothing draws from that list.
    fn record(&mut self, kind: SceneAddKind, flags: u32, gfx_ent: u32, pose: AddDObjPose) {
        let info = pack_scene_info(pose.entnum, flags, gfx_ent);
        let posed_bounds = seed_bounds(pose.origin, pose.radius);
        match kind {
            SceneAddKind::Model(_) => {
                set_slot(&mut self.scene_model_index, pose.entnum, self.scene_models.len());
                self.scene_models.push(GfxSceneModel {
                    origin: pose.origin,
                    lighting_origin: pose.lighting_origin,
                    radius: pose.radius,
                    info,
                    scale: 1.0,
                    quat: pose.quat,
                    posed_bounds,
                });
            }
            SceneAddKind::Dobj(_) => {
                set_slot(&mut self.scene_dobj_index, pose.entnum, self.scene_dobjs.len());
                self.scene_dobjs.push(GfxSceneDobj {
                    origin: pose.origin,
                    lighting_origin: pose.lighting_origin,
                    radius: pose.radius,
                    info,
                    quat: pose.quat,
                    cull_gate: dpvs_t6::SCENE_DOBJ_CULL_IDLE,
                    posed_bounds,
                    lods: Vec::new(),
                    skinned_surfs: None,
                });
            }
            SceneAddKind::DobjFlag30(_) => {}
        }
    }

    /// Moves an entity's object (its animated object, else its model) to a new pose.
    pub fn store_pose_origin_quat(&mut self, entnum: u32, origin: [f32; 3], quat: Option<[f32; 4]>) {
        if let Some(dobj) = self.scene_dobj_mut(entnum) {
            dobj.origin = origin;
            dobj.quat = quat;
        } else if let Some(model) = live_slot(&self.scene_model_index, entnum)
            .and_then(|slot| self.scene_models.get_mut(slot))
        {
            model.origin = origin;
            model.quat = quat;
        }
    }

    /// Records the LOD chosen for each of an object's models, up to the number
    /// the cull record has room for.
    pub fn store_scene_ent_lods(&mut self, entnum: u32, lods: &[i8]) {
        if let Some(dobj) = self.scene_dobj_mut(entnum) {
            let kept = lods.len().min(dpvs_t6::SCENE_DOBJ_CULL_LOD_COUNT);
            dobj.lods = lods[..kept].to_vec();
        }
    }

    /// Stores the skin pass's result and marks the object as skinned.
    pub fn store_scene_ent_skin(&mut self, entnum: u32, surfs: SceneEntSkinnedSurfs) {
        if let Some(dobj) = self.scene_dobj_mut(entnum) {
            // The gate was claimed for this skin; completing it records the surface count.
            let _ = dpvs_t6::scene_dobj_skin_begin(&mut dobj.cull_gate);
            dpvs_t6::scene_dobj_flag_skinned(&mut dobj.cull_gate, surfs.summary.surface_count);
            dobj.skinned_surfs = Some(surfs);
        }
    }

    /// The surface count recorded by the skin pass, once there is one.
    #[must_use]
    pub fn scene_ent_surface_count(&self, entnum: u32) -> Option<u32> {
        self.scene_dobj(entnum)
            .and_then(|dobj| dpvs_t6::scene_dobj_skinned_surface_count(dobj.cull_gate))
    }

    #[must_use]
    pub fn scene_ent_skinned_surfs(&self, entnum: u32) -> Option<&SceneEntSkinnedSurfs> {
        self.scene_dobj(entnum)?.skinned_surfs.as_ref()
    }

    #[must_use]
    pub fn scene_dobj(&self, entnum: u32) -> Option<&GfxSceneDobj> {
        self.scene_dobjs.get(live_slot(&self.scene_dobj_index, entnum)?)
    }

    #[must_use]
    pub fn scene_model(&self, entnum: u32) -> Option<&GfxSceneModel> {
        self.scene_models.get(live_slot(&self.scene_model_index, entnum)?)
    }

    fn scene_dobj_mut(&mut self, entnum: u32) -> Option<&mut GfxSceneDobj> {
        self.scene_dobjs.get_mut(live_slot(&self.scene_dobj_index, entnum)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> GfxScene {
        let mut scene = GfxScene::default();
        scene.clear();
        scene
    }

    fn pose(entnum: u32) -> AddDObjPose {
        AddDObjPose {
            origin: [1.0, 2.0, 3.0],
            lighting_origin: [1.0, 2.0, 4.0],
            radius: Some(10.0),
            entnum,
            quat: None,
        }
    }

    fn args(flags: u32, has_tree: bool, num_models: u8, pose: Option<AddDObjPose>) -> AddDObjArgs {
        AddDObjArgs { render_fx_flags: flags, material_time: 0.0, has_tree, num_models, pose }
    }

    fn bmodel(model_index: u32) -> AddBModelPose {
        AddBModelPose { model_index, origin: [0.0; 3], quat: None, param_4: None }
    }

    #[test]
    fn info_packs_and_unpacks_the_entity_number() {
        let info = pack_scene_info(0x123, 0b11, 0x45);
        assert_eq!(scene_info_entnum(info), 0x123);
        assert_eq!(info & 0x7f, 0x45);
        assert_eq!(info >> (7 + 12), 0b11);
        assert_eq!(scene_info_entnum(pack_scene_info(0x1fff, 0, 0)), 0xfff);
    }

    #[test]
    fn a_single_bare_model_is_a_scene_model() {
        let mut scene = scene();
        assert_eq!(scene.add_dobj(args(0, false, 1, Some(pose(5)))), Ok(SceneAddKind::Model(0)));
        let model = scene.scene_model(5).expect("model by entity number");
        assert_eq!(model.origin, [1.0, 2.0, 3.0]);
        assert_eq!(model.scale, 1.0);
        assert!(model.posed_bounds.is_some());
        assert!(scene.scene_model_live(5) && !scene.scene_dobj_live(5));
        assert!(scene.scene_dobj(5).is_none());
    }

    #[test]
    fn trees_many_models_and_special_flags_make_animated_objects() {
        let mut scene = scene();
        assert_eq!(scene.add_dobj(args(0, true, 1, Some(pose(1)))), Ok(SceneAddKind::Dobj(0)));
        assert_eq!(scene.add_dobj(args(0, false, 2, Some(pose(2)))), Ok(SceneAddKind::Dobj(1)));
        assert_eq!(scene.add_dobj(args(0x4, false, 1, Some(pose(3)))), Ok(SceneAddKind::Dobj(2)));
        assert_eq!(scene.scene_dobjs.len(), 3);
        assert_eq!(scene.scene_dobj(3).expect("dobj").cull_gate, dpvs_t6::SCENE_DOBJ_CULL_IDLE);
        assert_eq!(scene.scene_dobj_index[2], 1);
    }

    #[test]
    fn flag_30_objects_are_numbered_after_the_ordinary_ones_and_not_stored() {
        let mut scene = scene();
        assert_eq!(
            scene.add_dobj(args(0x10, false, 1, Some(pose(9)))),
            Ok(SceneAddKind::DobjFlag30(SCENE_DOBJ_CAP))
        );
        assert!(scene.scene_dobjs.is_empty() && !scene.scene_ent_live(9));
        for _ in 1..DOBJ_FLAG30_CAP {
            scene.add_dobj(args(0x20, true, 1, None)).unwrap();
        }
        assert_eq!(scene.add_dobj(args(0x20, true, 1, None)), Err(SceneAddError::DobjFlag30Overflow));
    }

    #[test]
    fn an_object_without_a_pose_takes_a_slot_but_is_not_recorded() {
        let mut scene = scene();
        assert_eq!(scene.add_dobj(args(0, false, 1, None)), Ok(SceneAddKind::Model(0)));
        assert_eq!(scene.add_dobj(args(0, false, 1, Some(pose(7)))), Ok(SceneAddKind::Model(1)));
        // The entity points at the stored model, not at the counter value.
        assert_eq!(scene.scene_models.len(), 1);
        assert_eq!(scene.scene_model(7).expect("model").origin, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn effect_slots_are_handed_out_and_packed_into_info() {
        let mut scene = scene();
        let timed = AddDObjArgs { material_time: 1.5, ..args(0, true, 1, Some(pose(1))) };
        scene.add_dobj(timed).unwrap();
        scene
            .add_dobj(AddDObjArgs { render_fx_flags: GFX_ENT_FLAG, pose: Some(pose(2)), ..timed })
            .unwrap();
        scene.add_dobj(AddDObjArgs { pose: Some(pose(3)), material_time: 0.0, ..timed }).unwrap();
        let slot = |ent| scene.scene_dobj(ent).expect("dobj").info & 0x7f;
        assert_eq!((slot(1), slot(2), slot(3)), (0, 1, 0));
        assert_eq!(scene.gfx_ent_count, 2);
    }

    #[test]
    fn the_effect_slot_list_overflows_after_its_cap() {
        let mut scene = scene();
        let timed = AddDObjArgs { material_time: 1.0, ..args(0, true, 1, None) };
        for _ in 0..SCENE_GFX_ENT_CAP {
            scene.add_dobj(timed).unwrap();
        }
        assert_eq!(scene.add_dobj(timed), Err(SceneAddError::GfxEntOverflow));
        assert_eq!(scene.gfx_ent_count, SCENE_GFX_ENT_CAP);
    }

    #[test]
    fn models_and_animated_objects_overflow_after_their_caps() {
        let mut scene = scene();
        for _ in 0..SCENE_MODEL_CAP {
            scene.add_dobj(args(0, false, 1, None)).unwrap();
        }
        assert_eq!(scene.add_dobj(args(0, false, 1, None)), Err(SceneAddError::ModelOverflow));
        for _ in 0..SCENE_DOBJ_CAP {
            scene.add_dobj(args(0, true, 1, None)).unwrap();
        }
        assert_eq!(scene.add_dobj(args(0, true, 1, None)), Err(SceneAddError::DobjOverflow));
    }

    #[test]
    fn entities_past_the_configured_count_get_no_slot() {
        let mut scene = scene();
        let beyond = dpvs_t6::GFX_CFG_ENT_COUNT;
        scene.add_dobj(args(0, false, 1, Some(pose(beyond)))).unwrap();
        assert_eq!(scene.scene_models.len(), 1);
        assert!(!scene.scene_ent_live(beyond));
    }

    #[test]
    fn index_tables_are_sized_on_first_use_when_the_scene_was_never_cleared() {
        let mut scene = GfxScene::default();
        scene.add_dobj(args(0, true, 1, Some(pose(2)))).unwrap();
        assert_eq!(scene.scene_dobj_index.len(), dpvs_t6::GFX_CFG_ENT_COUNT as usize);
        assert!(scene.scene_dobj_live(2) && !scene.scene_dobj_live(3));
    }

    #[test]
    fn brush_models_need_surfaces_and_stop_at_their_cap() {
        let mut scene = scene();
        assert_eq!(
            scene.add_bmodel(AddBModelArgs { surf_id: 0 }, bmodel(1)),
            Err(SceneAddError::BrushNoSurfs)
        );
        let args = AddBModelArgs { surf_id: 3 };
        assert_eq!(scene.add_bmodel(args, bmodel(4)), Ok(0), "a refusal takes no slot");
        for _ in 1..SCENE_BRUSH_CAP {
            scene.add_bmodel(args, bmodel(4)).unwrap();
        }
        assert_eq!(scene.add_bmodel(args, bmodel(4)), Err(SceneAddError::BrushOverflow));
        assert_eq!(scene.scene_brushes.len(), SCENE_BRUSH_CAP as usize);
        assert_eq!(scene.scene_brushes[0].model_index, 4);
    }

    #[test]
    fn clearing_resets_everything_but_keeps_the_tables_sized() {
        let mut scene = scene();
        scene.add_dobj(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.add_bmodel(AddBModelArgs { surf_id: 1 }, bmodel(1)).unwrap();
        scene.mark_scene_ent_visible(1);
        scene.scene_ent_walked = true;
        scene.clear();
        assert_eq!(scene, super::tests::scene());
        assert!(!scene.scene_ent_live(1) && !scene.scene_ent_visible(1) && !scene.scene_ent_walked);
    }

    #[test]
    fn an_entity_is_hidden_only_after_a_walk_that_did_not_mark_it() {
        let mut scene = scene();
        scene.add_dobj(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.add_dobj(args(0, true, 1, Some(pose(2)))).unwrap();
        assert!(!scene.scene_ent_hidden(1), "no walk yet");
        scene.scene_ent_walked = true;
        scene.mark_scene_ent_visible(2);
        assert!(scene.scene_ent_hidden(1) && scene.scene_ent_skips_draw(1));
        assert!(!scene.scene_ent_hidden(2) && !scene.scene_ent_skips_draw(2));
        assert!(!scene.scene_ent_hidden(3), "an entity not in the scene is not hidden");
    }

    #[test]
    fn a_skinned_object_with_no_surfaces_skips_drawing() {
        let mut scene = scene();
        scene.add_dobj(args(0, true, 1, Some(pose(1)))).unwrap();
        assert_eq!(scene.scene_ent_surface_count(1), None);
        let skinned = |surface_count| SceneEntSkinnedSurfs {
            summary: dpvs_t6::PreSkinSummary { surface_count, ..Default::default() },
            ..Default::default()
        };
        // The skin pass stores into an object whose bounds are built.
        let bound = |scene: &mut GfxScene| scene.scene_dobjs[0].cull_gate = dpvs_t6::SCENE_DOBJ_CULL_BOUNDED;
        bound(&mut scene);
        scene.store_scene_ent_skin(1, skinned(3));
        assert_eq!(scene.scene_ent_surface_count(1), Some(3));
        assert!(scene.scene_ent_skinned_surfs(1).is_some() && !scene.scene_ent_skips_draw(1));
        scene.store_scene_ent_skin(1, skinned(0));
        assert_eq!(scene.scene_ent_surface_count(1), Some(3), "a finished skin is not redone");
        bound(&mut scene);
        scene.store_scene_ent_skin(1, skinned(0));
        assert!(scene.scene_ent_skips_draw(1));
        scene.store_scene_ent_skin(99, skinned(1));
        assert_eq!(scene.scene_ent_surface_count(99), None);
    }

    #[test]
    fn stored_lods_are_cut_to_what_the_cull_record_holds() {
        let mut scene = scene();
        scene.add_dobj(args(0, true, 1, Some(pose(1)))).unwrap();
        let count = dpvs_t6::SCENE_DOBJ_CULL_LOD_COUNT;
        scene.store_scene_ent_lods(1, &vec![2; count + 3]);
        assert_eq!(scene.scene_dobj(1).expect("dobj").lods, vec![2; count]);
        scene.store_scene_ent_lods(1, &[1]);
        assert_eq!(scene.scene_dobj(1).expect("dobj").lods, vec![1]);
    }

    #[test]
    fn a_new_pose_updates_the_animated_object_else_the_model() {
        let mut scene = scene();
        scene.add_dobj(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.add_dobj(args(0, false, 1, Some(pose(2)))).unwrap();
        let quat = Some([0.0, 0.0, 1.0, 0.0]);
        scene.store_pose_origin_quat(1, [9.0; 3], quat);
        scene.store_pose_origin_quat(2, [8.0; 3], None);
        scene.store_pose_origin_quat(3, [7.0; 3], None);
        let dobj = scene.scene_dobj(1).expect("dobj");
        assert_eq!((dobj.origin, dobj.quat), ([9.0; 3], quat));
        assert_eq!(scene.scene_model(2).expect("model").origin, [8.0; 3]);
    }
}
