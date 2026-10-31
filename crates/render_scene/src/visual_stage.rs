//! The frame's list of things to draw: models, animated objects and brush
//! models, each with the slot limits the original scene had.

pub const RENDER_ENTITY_SLOTS_PER_FRAME: u32 = 0x80;
pub const MODEL_SLOTS_PER_FRAME: u32 = 0x400;
pub const ANIM_OBJECT_SLOTS_PER_FRAME: u32 = 0x200;
pub const BRUSH_SLOTS_PER_FRAME: u32 = 0x200;
/// The value an index table holds for an entity that has no scene slot.
pub const SLOT_INDEX_NONE: u16 = 0xffff;
/// Entity numbers of the first-person weapon models.
pub const FIRST_PERSON_WEAPON_SLOT: u32 = 0x7ff;
pub const FIRST_PERSON_OFFHAND_SLOT: u32 = 0x7fe;
pub const FIRST_PERSON_WEAPON_EFFECT_BITS: u32 = 3;

/// Animated objects carrying any of these effect flags go in a small separate list.
const DOBJ_FLAG30_MASK: u32 = 0x30;
const DOBJ_FLAG30_CAP: u32 = 8;
/// An effect flag that needs its own per-entity effect slot.
const GFX_ENT_FLAG: u32 = 2;
/// A single-model object with none of these effect flags is drawn as a plain model.
const PLAIN_MODEL_FLAG_MASK: u32 = 0x34;

/// Bit layout of a scene `info` word, low to high: effect slot, entity number,
/// effect flags.
const INFO_SLOT_BITS: u32 = 7;
const INFO_ENT_BITS: u32 = 12;
const INFO_SLOT_MASK: u32 = (1 << INFO_SLOT_BITS) - 1;
const INFO_ENT_MASK: u32 = (1 << INFO_ENT_BITS) - 1;
const INFO_FLAGS_SHIFT: u32 = INFO_SLOT_BITS + INFO_ENT_BITS;

/// The entity number packed into a scene `info` word.
#[must_use]
pub fn stage_info_actor_num(info: u32) -> u32 {
    (info >> INFO_SLOT_BITS) & INFO_ENT_MASK
}

/// `info` is the effect-slot index in the low 7 bits, then the entity number
/// (12 bits) with the effect flags above it.
pub fn pack_stage_word(who: u32, render_effect_flags: u32, fx_row: u32) -> u32 {
    let slot_part = fx_row & INFO_SLOT_MASK;
    let ent_part = (who & INFO_ENT_MASK) << INFO_SLOT_BITS;
    let flag_part = render_effect_flags << INFO_FLAGS_SHIFT;
    flag_part | ent_part | slot_part
}

/// Where an added object went, and the slot number it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneInsertCategory {
    Mesh(u32),
    Dobj(u32),
    /// Numbered after the ordinary animated objects.
    AnimObjectFlagBit30(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneInsertFault {
    RenderEntityTableFull,
    ModelTableFull,
    AnimObjectTableFull,
    AnimObjectFlagBit30Full,
    BrushTableFull,
    BrushLacksFaces,
}

#[derive(Clone, Copy, Debug)]
pub struct AnimatedObjectSpawn {
    pub render_effect_flags: u32,
    pub surface_clock: f32,
    pub carries_anim_graph: bool,
    pub model_slot_total: u8,
    pub pose: Option<DynObjectPlacementArgs>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynObjectPlacementArgs {
    pub origin: [f32; 3],
    pub illum_origin: [f32; 3],
    pub reach: Option<f32>,
    pub ent_no: u32,
    pub spin: Option<[f32; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneModelInstance {
    pub origin: [f32; 3],
    pub illum_origin: [f32; 3],
    pub reach: Option<f32>,
    pub about: u32,
    pub size: f32,
    pub spin: Option<[f32; 4]>,
    pub pose_bounds: Option<dpvs_t6::Bounds>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualStagePuppet {
    pub origin: [f32; 3],
    pub illum_origin: [f32; 3],
    pub reach: Option<f32>,
    pub about: u32,
    pub spin: Option<[f32; 4]>,
    /// The skin pass's claim on this object; see `dpvs_t6::scene_dobj_skin_begin`.
    pub visibility_gate_bits: u32,
    pub pose_bounds: Option<dpvs_t6::Bounds>,
    pub detail_tiers: Vec<i8>,
    pub skinned_facets: Option<StageActorSkinnedFacets>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageActorSkinnedFacets {
    pub entries: Vec<dpvs_t6::EntitySkinSlotRecord>,
    pub summary: dpvs_t6::PreSkinningDigest,
}

#[derive(Clone, Copy, Debug)]
pub struct InsertBrushModelArgs {
    /// The brush model's surface count; zero means there is nothing to draw.
    pub face_no: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushModelPlacement {
    pub mesh_slot_number: u32,
    pub spot: [f32; 3],
    pub spin: Option<[f32; 4]>,
    pub extra_slot_param: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneBrushInstance {
    pub mesh_slot_number: u32,
    pub origin: [f32; 3],
    pub spin: Option<[f32; 4]>,
    pub extra_slot_param: Option<u16>,
}

/// How many slots of each capped list this frame has asked for so far.
#[derive(Clone, Debug, Default, PartialEq)]
struct SlotCounters {
    effects: u32,
    models: u32,
    dobjs: u32,
    flag30_dobjs: u32,
    brushes: u32,
}

/// One frame's scene. `static_model_slot_table` and `animated_object_slot_table` map an entity
/// number to its position in `stage_models` / `stage_puppets`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualStage {
    counters: SlotCounters,
    pub stage_models: Vec<SceneModelInstance>,
    pub stage_puppets: Vec<VisualStagePuppet>,
    pub scene_brushes: Vec<SceneBrushInstance>,
    pub static_model_slot_table: Vec<u16>,
    pub animated_object_slot_table: Vec<u16>,
    pub stage_actor_visible: Vec<u8>,
    /// The visibility walk has run, so an entity it did not mark really is hidden.
    pub stage_actor_walked: bool,
}

/// Asks a counted list with `cap` places for its next slot. Every ask moves the
/// counter on, but it never passes `cap` (only this function moves it, so it
/// is never above `cap`), so a full list stays full.
fn take_slot(count: &mut u32, cap: u32) -> Option<u32> {
    let asked = *count;
    *count = (asked + 1).min(cap);
    if asked >= cap { None } else { Some(asked) }
}

/// The list position an entity's index-table entry points at, if it has one.
fn live_slot(table: &[u16], who: u32) -> Option<usize> {
    match table.get(who as usize) {
        Some(&SLOT_INDEX_NONE) | None => None,
        Some(&slot) => Some(slot as usize),
    }
}

/// The list entry an entity's index-table entry points at.
fn entry_mut<'a, T>(list: &'a mut [T], slots: &[u16], who: u32) -> Option<&'a mut T> {
    live_slot(slots, who).and_then(move |slot| list.get_mut(slot))
}

/// Points `who` at `row`, growing the table to cover every entity first.
/// Entities past the configured count get no slot.
fn set_slot(table: &mut Vec<u16>, who: u32, row: usize) {
    let entities = dpvs_t6::VISUAL_CFG_ACTOR_TALLY as usize;
    if who as usize >= entities {
        return;
    }
    if table.len() != entities {
        table.resize(entities, SLOT_INDEX_NONE);
    }
    table[who as usize] = row as u16;
}

impl VisualStage {
    /// Empties the scene for a new frame, with every entity unslotted and invisible.
    pub fn clear(&mut self) {
        let entities = dpvs_t6::VISUAL_CFG_ACTOR_TALLY as usize;
        let mut fresh = Self::default();
        fresh.static_model_slot_table.resize(entities, SLOT_INDEX_NONE);
        fresh.animated_object_slot_table.resize(entities, SLOT_INDEX_NONE);
        fresh.stage_actor_visible.resize(entities, 0);
        std::mem::swap(self, &mut fresh);
    }

    pub fn stain_stage_actor_visible(&mut self, who: u32) {
        dpvs_t6::stain_stage_actor_visible(&mut self.stage_actor_visible, who);
    }

    #[must_use]
    pub fn stage_actor_visible(&self, who: u32) -> bool {
        dpvs_t6::entity_flagged_visible(&self.stage_actor_visible, who)
    }

    /// In the scene, after a walk, and not marked by that walk.
    #[must_use]
    pub fn entity_concealed(&self, who: u32) -> bool {
        matches!(
            (self.stage_actor_walked, self.entity_present(who), self.stage_actor_visible(who)),
            (true, true, false)
        )
    }

    /// Hidden by the visibility walk, or skinned and found to have nothing to draw.
    #[must_use]
    pub fn entity_not_drawn(&self, who: u32) -> bool {
        let skinned_empty = self.entity_surface_total(who).is_some_and(|n| n == 0);
        skinned_empty || self.entity_concealed(who)
    }

    #[must_use]
    pub fn animated_object_present(&self, who: u32) -> bool {
        live_slot(&self.animated_object_slot_table, who).is_some()
    }

    #[must_use]
    pub fn static_model_present(&self, who: u32) -> bool {
        live_slot(&self.static_model_slot_table, who).is_some()
    }

    #[must_use]
    pub fn entity_present(&self, who: u32) -> bool {
        [&self.animated_object_slot_table, &self.static_model_slot_table]
            .into_iter()
            .any(|table| live_slot(table, who).is_some())
    }

    /// Adds a brush model; returns its slot number.
    pub fn add_bmodel(&mut self, args: InsertBrushModelArgs, pose: BrushModelPlacement) -> Result<u32, SceneInsertFault> {
        let slot = match args.face_no {
            0 => Err(SceneInsertFault::BrushLacksFaces),
            _ => take_slot(&mut self.counters.brushes, BRUSH_SLOTS_PER_FRAME).ok_or(SceneInsertFault::BrushTableFull),
        }?;
        let BrushModelPlacement { mesh_slot_number, spot: origin, spin: quat, extra_slot_param } = pose;
        self.scene_brushes.push(SceneBrushInstance { mesh_slot_number, origin, spin: quat, extra_slot_param });
        Ok(slot)
    }

    /// Adds an animated object, as a plain model when it is a single bare model.
    /// The object is only recorded (and findable by entity number) if `args.pose` is given.
    pub fn place_object(&mut self, args: AnimatedObjectSpawn) -> Result<SceneInsertCategory, SceneInsertFault> {
        // The effect slot is claimed first and stays claimed even if the list is full.
        let wants_effect_slot = args.render_effect_flags & GFX_ENT_FLAG != 0 || args.surface_clock != 0.0;
        let fx_row = match wants_effect_slot {
            true => take_slot(&mut self.counters.effects, RENDER_ENTITY_SLOTS_PER_FRAME).ok_or(SceneInsertFault::RenderEntityTableFull)?,
            false => 0,
        };
        let kind = self.claim_list_slot(&args)?;
        args.pose.into_iter().for_each(|pose| self.record(kind, args.render_effect_flags, fx_row, pose));
        Ok(kind)
    }

    /// Picks the list an object belongs in and takes the next slot there.
    fn claim_list_slot(&mut self, args: &AnimatedObjectSpawn) -> Result<SceneInsertCategory, SceneInsertFault> {
        let flags = args.render_effect_flags;
        let bare_single_model = args.model_slot_total == 1 && !args.carries_anim_graph && flags & PLAIN_MODEL_FLAG_MASK == 0;
        let (counter, cap, overflow, kind): (&mut u32, u32, SceneInsertFault, fn(u32) -> SceneInsertCategory) =
            if bare_single_model {
                (&mut self.counters.models, MODEL_SLOTS_PER_FRAME, SceneInsertFault::ModelTableFull, SceneInsertCategory::Mesh)
            } else if flags & DOBJ_FLAG30_MASK != 0 {
                (
                    &mut self.counters.flag30_dobjs,
                    DOBJ_FLAG30_CAP,
                    SceneInsertFault::AnimObjectFlagBit30Full,
                    |n| SceneInsertCategory::AnimObjectFlagBit30(ANIM_OBJECT_SLOTS_PER_FRAME + n),
                )
            } else {
                (&mut self.counters.dobjs, ANIM_OBJECT_SLOTS_PER_FRAME, SceneInsertFault::AnimObjectTableFull, SceneInsertCategory::Dobj)
            };
        take_slot(counter, cap).map(kind).ok_or(overflow)
    }

    /// Stores a posed object and points its entity number at it. The flag-30
    /// objects are counted but not stored: nothing draws from that list.
    fn record(&mut self, kind: SceneInsertCategory, flags: u32, fx_row: u32, pose: DynObjectPlacementArgs) {
        let DynObjectPlacementArgs { origin, illum_origin, reach: radius, ent_no: who, spin: quat } = pose;
        let info = pack_stage_word(who, flags, fx_row);
        let pose_bounds = radius.map(|r| dpvs_t6::animated_object_starting_box(origin, r));
        let (table, stored_at) = match kind {
            SceneInsertCategory::AnimObjectFlagBit30(_) => return,
            SceneInsertCategory::Mesh(_) => {
                let model = SceneModelInstance { origin, illum_origin, reach: radius, about: info, size: 1.0, spin: quat, pose_bounds };
                self.stage_models.push(model);
                (&mut self.static_model_slot_table, self.stage_models.len() - 1)
            }
            SceneInsertCategory::Dobj(_) => {
                self.stage_puppets.push(VisualStagePuppet {
                    visibility_gate_bits: dpvs_t6::SCENE_DOBJ_CULL_IDLE,
                    detail_tiers: Vec::new(),
                    skinned_facets: None,
                    origin,
                    illum_origin,
                    reach: radius,
                    about: info,
                    spin: quat,
                    pose_bounds,
                });
                (&mut self.animated_object_slot_table, self.stage_puppets.len() - 1)
            }
        };
        set_slot(table, who, stored_at);
    }

    /// Moves an entity's object (its animated object, else its model) to a new pose.
    pub fn store_pose_origin_quat(&mut self, who: u32, at: [f32; 3], turn: Option<[f32; 4]>) {
        let target = match entry_mut(&mut self.stage_puppets, &self.animated_object_slot_table, who) {
            Some(dobj) => Some((&mut dobj.origin, &mut dobj.spin)),
            None => entry_mut(&mut self.stage_models, &self.static_model_slot_table, who)
                .map(|model| (&mut model.origin, &mut model.spin)),
        };
        if let Some((place, rotation)) = target {
            *place = at;
            *rotation = turn;
        }
    }

    /// Records the LOD chosen for each of an object's models, up to the number
    /// the cull record has room for.
    pub fn store_lods_for(&mut self, who: u32, detail_tiers: &[i8]) {
        let Some(target) = entry_mut(&mut self.stage_puppets, &self.animated_object_slot_table, who) else {
            return;
        };
        target.detail_tiers.clear();
        target.detail_tiers.extend(detail_tiers.iter().copied().take(dpvs_t6::SCENE_DOBJ_CULL_LOD_COUNT));
    }

    /// Stores the skin pass's result and marks the object as skinned.
    pub fn store_stage_actor_skin(&mut self, who: u32, surfs: StageActorSkinnedFacets) {
        let Some(target) = entry_mut(&mut self.stage_puppets, &self.animated_object_slot_table, who) else {
            return;
        };
        // The gate was claimed for this skin; completing it records the surface count.
        let gate = &mut target.visibility_gate_bits;
        let _ = dpvs_t6::scene_dobj_skin_begin(gate);
        dpvs_t6::scene_dobj_flag_skinned(gate, surfs.summary.facet_tally);
        target.skinned_facets.replace(surfs);
    }

    /// The surface count recorded by the skin pass, once there is one.
    #[must_use]
    pub fn entity_surface_total(&self, who: u32) -> Option<u32> {
        let gate = self.stage_puppet(who).map(|d| d.visibility_gate_bits)?;
        dpvs_t6::scene_dobj_skinned_surface_count(gate)
    }

    #[must_use]
    pub fn entity_bone_surface_set(&self, who: u32) -> Option<&StageActorSkinnedFacets> {
        self.stage_puppet(who).and_then(|d| d.skinned_facets.as_ref())
    }

    #[must_use]
    pub fn stage_puppet(&self, who: u32) -> Option<&VisualStagePuppet> {
        live_slot(&self.animated_object_slot_table, who).and_then(|slot| self.stage_puppets.get(slot))
    }

    #[must_use]
    pub fn model_for(&self, who: u32) -> Option<&SceneModelInstance> {
        live_slot(&self.static_model_slot_table, who).and_then(|slot| self.stage_models.get(slot))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> VisualStage {
        let mut scene = VisualStage::default();
        scene.clear();
        scene
    }

    fn pose(entnum: u32) -> DynObjectPlacementArgs {
        DynObjectPlacementArgs {
            origin: [1.0, 2.0, 3.0],
            illum_origin: [1.0, 2.0, 4.0],
            reach: Some(10.0),
            ent_no: entnum,
            spin: None,
        }
    }

    fn args(flags: u32, carries_anim_graph: bool, model_slot_total: u8, pose: Option<DynObjectPlacementArgs>) -> AnimatedObjectSpawn {
        AnimatedObjectSpawn { render_effect_flags: flags, surface_clock: 0.0, carries_anim_graph, model_slot_total, pose }
    }

    fn bmodel(mesh_slot_number: u32) -> BrushModelPlacement {
        BrushModelPlacement { mesh_slot_number, spot: <[f32; 3]>::default(), spin: None, extra_slot_param: None }
    }

    #[test]
    fn info_packs_and_unpacks_the_entity_number() {
        let info = pack_stage_word(0x123, 0b11, 0x45);
        assert_eq!(stage_info_actor_num(info), 0x123);
        assert_eq!(info & 0x7f, 0x45);
        assert_eq!(info >> (7 + 12), 0b11);
        assert_eq!(stage_info_actor_num(pack_stage_word(0x1fff, 0, 0)), 0xfff);
    }

    #[test]
    fn a_single_bare_model_is_a_scene_model() {
        let mut scene = scene();
        assert_eq!(scene.place_object(args(0, false, 1, Some(pose(5)))), Ok(SceneInsertCategory::Mesh(0)));
        let model = scene.model_for(5).expect("model by entity number");
        assert_eq!(model.origin, [1.0, 2.0, 3.0]);
        assert_eq!(model.size, 1.0);
        assert!(model.pose_bounds.is_some());
        assert!(scene.static_model_present(5) && !scene.animated_object_present(5));
        assert!(scene.stage_puppet(5).is_none());
    }

    #[test]
    fn trees_many_models_and_special_flags_make_animated_objects() {
        let mut scene = scene();
        assert_eq!(scene.place_object(args(0, true, 1, Some(pose(1)))), Ok(SceneInsertCategory::Dobj(0)));
        assert_eq!(scene.place_object(args(0, false, 2, Some(pose(2)))), Ok(SceneInsertCategory::Dobj(1)));
        assert_eq!(scene.place_object(args(0x4, false, 1, Some(pose(3)))), Ok(SceneInsertCategory::Dobj(2)));
        assert_eq!(scene.stage_puppets.len(), 3);
        assert_eq!(scene.stage_puppet(3).expect("dobj").visibility_gate_bits, dpvs_t6::SCENE_DOBJ_CULL_IDLE);
        assert_eq!(scene.animated_object_slot_table[2], 1);
    }

    #[test]
    fn flag_30_objects_are_numbered_after_the_ordinary_ones_and_not_stored() {
        let mut scene = scene();
        assert_eq!(
            scene.place_object(args(0x10, false, 1, Some(pose(9)))),
            Ok(SceneInsertCategory::AnimObjectFlagBit30(ANIM_OBJECT_SLOTS_PER_FRAME))
        );
        assert!(scene.stage_puppets.is_empty() && !scene.entity_present(9));
        for _ in 1..DOBJ_FLAG30_CAP {
            scene.place_object(args(0x20, true, 1, None)).unwrap();
        }
        assert_eq!(scene.place_object(args(0x20, true, 1, None)), Err(SceneInsertFault::AnimObjectFlagBit30Full));
    }

    #[test]
    fn an_object_without_a_pose_takes_a_slot_but_is_not_recorded() {
        let mut scene = scene();
        assert_eq!(scene.place_object(args(0, false, 1, None)), Ok(SceneInsertCategory::Mesh(0)));
        assert_eq!(scene.place_object(args(0, false, 1, Some(pose(7)))), Ok(SceneInsertCategory::Mesh(1)));
        // The entity points at the stored model, not at the counter value.
        assert_eq!(scene.stage_models.len(), 1);
        assert_eq!(scene.model_for(7).expect("model").origin, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn effect_slots_are_handed_out_and_packed_into_info() {
        let mut scene = scene();
        let timed = AnimatedObjectSpawn { surface_clock: 1.5, ..args(0, true, 1, Some(pose(1))) };
        scene.place_object(timed).unwrap();
        scene
            .place_object(AnimatedObjectSpawn { render_effect_flags: GFX_ENT_FLAG, pose: Some(pose(2)), ..timed })
            .unwrap();
        scene.place_object(AnimatedObjectSpawn { pose: Some(pose(3)), surface_clock: 0.0, ..timed }).unwrap();
        let slot = |ent| scene.stage_puppet(ent).expect("dobj").about & 0x7f;
        assert_eq!((slot(1), slot(2), slot(3)), (0, 1, 0));
        assert_eq!(scene.counters.effects, 2);
    }

    #[test]
    fn the_effect_slot_list_overflows_after_its_cap() {
        let mut scene = scene();
        let timed = AnimatedObjectSpawn { surface_clock: 1.0, ..args(0, true, 1, None) };
        for _ in 0..RENDER_ENTITY_SLOTS_PER_FRAME {
            scene.place_object(timed).unwrap();
        }
        assert_eq!(scene.place_object(timed), Err(SceneInsertFault::RenderEntityTableFull));
        assert_eq!(scene.counters.effects, RENDER_ENTITY_SLOTS_PER_FRAME);
    }

    #[test]
    fn models_and_animated_objects_overflow_after_their_caps() {
        let mut scene = scene();
        for _ in 0..MODEL_SLOTS_PER_FRAME {
            scene.place_object(args(0, false, 1, None)).unwrap();
        }
        assert_eq!(scene.place_object(args(0, false, 1, None)), Err(SceneInsertFault::ModelTableFull));
        for _ in 0..ANIM_OBJECT_SLOTS_PER_FRAME {
            scene.place_object(args(0, true, 1, None)).unwrap();
        }
        assert_eq!(scene.place_object(args(0, true, 1, None)), Err(SceneInsertFault::AnimObjectTableFull));
    }

    #[test]
    fn entities_past_the_configured_count_get_no_slot() {
        let mut scene = scene();
        let beyond = dpvs_t6::VISUAL_CFG_ACTOR_TALLY;
        scene.place_object(args(0, false, 1, Some(pose(beyond)))).unwrap();
        assert_eq!(scene.stage_models.len(), 1);
        assert!(!scene.entity_present(beyond));
    }

    #[test]
    fn index_tables_are_sized_on_first_use_when_the_scene_was_never_cleared() {
        let mut scene = VisualStage::default();
        scene.place_object(args(0, true, 1, Some(pose(2)))).unwrap();
        assert_eq!(scene.animated_object_slot_table.len(), dpvs_t6::VISUAL_CFG_ACTOR_TALLY as usize);
        assert!(scene.animated_object_present(2) && !scene.animated_object_present(3));
    }

    #[test]
    fn brush_models_need_surfaces_and_stop_at_their_cap() {
        let mut scene = scene();
        assert_eq!(
            scene.add_bmodel(InsertBrushModelArgs { face_no: 0 }, bmodel(1)),
            Err(SceneInsertFault::BrushLacksFaces)
        );
        let args = InsertBrushModelArgs { face_no: 3 };
        assert_eq!(scene.add_bmodel(args, bmodel(4)), Ok(0), "a refusal takes no slot");
        for _ in 1..BRUSH_SLOTS_PER_FRAME {
            scene.add_bmodel(args, bmodel(4)).unwrap();
        }
        assert_eq!(scene.add_bmodel(args, bmodel(4)), Err(SceneInsertFault::BrushTableFull));
        assert_eq!(scene.scene_brushes.len(), BRUSH_SLOTS_PER_FRAME as usize);
        assert_eq!(scene.scene_brushes[0].mesh_slot_number, 4);
    }

    #[test]
    fn clearing_resets_everything_but_keeps_the_tables_sized() {
        let mut scene = scene();
        scene.place_object(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.add_bmodel(InsertBrushModelArgs { face_no: 1 }, bmodel(1)).unwrap();
        scene.stain_stage_actor_visible(1);
        scene.stage_actor_walked = true;
        scene.clear();
        assert_eq!(scene, super::tests::scene());
        assert!(!scene.entity_present(1) && !scene.stage_actor_visible(1) && !scene.stage_actor_walked);
    }

    #[test]
    fn an_entity_is_hidden_only_after_a_walk_that_did_not_mark_it() {
        let mut scene = scene();
        scene.place_object(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.place_object(args(0, true, 1, Some(pose(2)))).unwrap();
        assert!(!scene.entity_concealed(1), "no walk yet");
        scene.stage_actor_walked = true;
        scene.stain_stage_actor_visible(2);
        assert!(scene.entity_concealed(1) && scene.entity_not_drawn(1));
        assert!(!scene.entity_concealed(2) && !scene.entity_not_drawn(2));
        assert!(!scene.entity_concealed(3), "an entity not in the scene is not hidden");
    }

    #[test]
    fn a_skinned_object_with_no_surfaces_skips_drawing() {
        let mut scene = scene();
        scene.place_object(args(0, true, 1, Some(pose(1)))).unwrap();
        assert_eq!(scene.entity_surface_total(1), None);
        let skinned = |facet_tally| StageActorSkinnedFacets {
            summary: dpvs_t6::PreSkinningDigest { facet_tally, ..dpvs_t6::PreSkinningDigest::default() },
            ..Default::default()
        };
        // The skin pass stores into an object whose bounds are built.
        let bound = |scene: &mut VisualStage| scene.stage_puppets[0].visibility_gate_bits = dpvs_t6::SCENE_DOBJ_CULL_BOUNDED;
        bound(&mut scene);
        scene.store_stage_actor_skin(1, skinned(3));
        assert_eq!(scene.entity_surface_total(1), Some(3));
        assert!(scene.entity_bone_surface_set(1).is_some() && !scene.entity_not_drawn(1));
        scene.store_stage_actor_skin(1, skinned(0));
        assert_eq!(scene.entity_surface_total(1), Some(3), "a finished skin is not redone");
        bound(&mut scene);
        scene.store_stage_actor_skin(1, skinned(0));
        assert!(scene.entity_not_drawn(1));
        scene.store_stage_actor_skin(99, skinned(1));
        assert_eq!(scene.entity_surface_total(99), None);
    }

    #[test]
    fn stored_lods_are_cut_to_what_the_cull_record_holds() {
        let mut scene = scene();
        scene.place_object(args(0, true, 1, Some(pose(1)))).unwrap();
        let count = dpvs_t6::SCENE_DOBJ_CULL_LOD_COUNT;
        scene.store_lods_for(1, &vec![2; count + 3]);
        assert_eq!(scene.stage_puppet(1).expect("dobj").detail_tiers, vec![2; count]);
        scene.store_lods_for(1, &[1]);
        assert_eq!(scene.stage_puppet(1).expect("dobj").detail_tiers, vec![1]);
    }

    #[test]
    fn a_new_pose_updates_the_animated_object_else_the_model() {
        let mut scene = scene();
        scene.place_object(args(0, true, 1, Some(pose(1)))).unwrap();
        scene.place_object(args(0, false, 1, Some(pose(2)))).unwrap();
        let quat = Some([0.0, 0.0, 1.0, 0.0]);
        scene.store_pose_origin_quat(1, [9.0; 3], quat);
        scene.store_pose_origin_quat(2, [8.0; 3], None);
        scene.store_pose_origin_quat(3, [7.0; 3], None);
        let dobj = scene.stage_puppet(1).expect("dobj");
        assert_eq!((dobj.origin, dobj.spin), ([9.0; 3], quat));
        assert_eq!(scene.model_for(2).expect("model").origin, [8.0; 3]);
    }
}
