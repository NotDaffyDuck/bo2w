//! The effect host: pools for effects, elements and trails, the ring of
//! effect handles with its garbage collection, bolt records, and the notes
//! the renderer leaves about decals and marks.

use fx_t6::runtime::{
    FX_BOLT_FREE_NONE, FX_BOLT_HANDLE_NONE, FX_BOLT_PARENT_IDENTITY_QUAT, FX_BOLT_RECORD_CAPACITY,
    FX_EFFECT_HANDLE_RING_SIZE, FX_EFFECT_POOL_CAPACITY, FX_EFFECT_SLOT_SIZE,
    FX_ELEM_POOL_CAPACITY, FX_SPAWN_BOLT_NONE, FX_SPOT_LIGHT_LIMIT,
    FX_STATUS_HAS_PENDING_LOOP_ELEMS, FX_STATUS_REF_COUNT_LOW_MASK, FX_TRAIL_ELEM_POOL_CAPACITY,
    FX_TRAIL_POOL_CAPACITY, FxOrientFrame, FxVisBlockerBuf, bolt_alloc, bolt_bone, bolt_dobj,
    bolt_handle_is_none, bolt_init_next_index, bolt_init_parent_orientation, effect_handle_for_slot,
    effect_random_seed_from_msec, status_is_unique_done,
};

use crate::elem::{FX_ELEM_HANDLE_NONE, FxElemSlot};
use crate::gaps::{FxGapCause, FxGaps};
use crate::glass::FxGlassSystemHost;
use crate::marks::{
    FxMarksSystemHost, GfxMarkMeshCensus, MarkImpactRequest, MarkImpactResult, MarkReceiverEnable,
    MarkTraceRecord,
};
use crate::spawn::{free_all_elems_for_effect, free_all_trails_for_effect, stop_effect_non_recursive};
use crate::trail::{FX_TRAIL_HANDLE_NONE, FxTrailElemSlot, FxTrailSlot};

pub const FX_CATALOG_INDEX_NONE: u16 = u16::MAX;

/// What an effect's `bolt` holds when no bolt record belongs to it.
pub(crate) const BOLT_NONE: u8 = 0xff;

/// A new effect holds one claim (its player's) and is marked freshly played.
const NEW_EFFECT_STATUS: u32 = 0x4000_0001;

/// How many mark traces are kept before the oldest is dropped.
const MARK_TRACE_LIMIT: usize = 512;

const IDENTITY_AXIS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxBoltOrientation {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxBoltTarget {
    pub dobj: u32,
    pub bone: u16,
    pub centity_teleport: bool,
    pub orientation: FxBoltOrientation,
}

/// Where a bolt's bone ended up this frame; no orientation when it can't be found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxResolvedBoltPose {
    pub centity_teleport: bool,
    pub orientation: Option<FxBoltOrientation>,
}

/// Where an effect's packed lighting colour came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FxPackedLightingSrc {
    White = 0,
    AtPointRgb = 1,
    Missing = 2,
}

#[derive(Clone, Debug)]
pub struct FxEffectSlot {
    pub def_name: String,
    pub catalog_index: u16,
    pub status: u32,
    pub first_elem_handle: [u16; 3],
    pub first_sorted_elem_handle: u16,
    pub first_trail_handle: u16,
    pub random_seed: u16,
    pub own_handle: u16,
    pub packed_lighting: [u8; 3],
    pub packed_lighting_src: FxPackedLightingSrc,
    pub bolt: u8,
    pub mark_entity: Option<u16>,
    pub bolt_packed: u32,
    pub bolt_centity_teleport: bool,
    pub bolt_parent_quat: [f32; 4],
    pub bolt_parent_origin: [f32; 3],
    pub bolt_bone_pose: Option<([f32; 3], [[f32; 3]; 3])>,
    pub frame_stamp: i32,
    pub msec_begin: i32,
    pub msec_last_update: i32,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub origin_when_played: [f32; 3],
    pub axis_when_played: [[f32; 3]; 3],
    pub origin_last: [f32; 3],
    pub axis_last: [[f32; 3]; 3],
    pub distance: f32,
    /// Whether this slot currently holds an effect named in the handle ring.
    pub ring_resident: bool,
}

impl Default for FxEffectSlot {
    fn default() -> Self {
        Self {
            def_name: String::new(),
            catalog_index: FX_CATALOG_INDEX_NONE,
            status: 0,
            first_elem_handle: [FX_ELEM_HANDLE_NONE; 3],
            first_sorted_elem_handle: FX_ELEM_HANDLE_NONE,
            first_trail_handle: FX_ELEM_HANDLE_NONE,
            random_seed: 0,
            own_handle: 0,
            packed_lighting: [0xff; 3],
            packed_lighting_src: FxPackedLightingSrc::White,
            bolt: BOLT_NONE,
            mark_entity: None,
            bolt_packed: FX_BOLT_HANDLE_NONE,
            bolt_centity_teleport: false,
            bolt_parent_quat: FX_BOLT_PARENT_IDENTITY_QUAT,
            bolt_parent_origin: [0.0; 3],
            bolt_bone_pose: None,
            frame_stamp: -1,
            msec_begin: 0,
            msec_last_update: 0,
            origin: [0.0; 3],
            axis: IDENTITY_AXIS,
            origin_when_played: [0.0; 3],
            axis_when_played: IDENTITY_AXIS,
            origin_last: [0.0; 3],
            axis_last: IDENTITY_AXIS,
            distance: 0.0,
            ring_resident: false,
        }
    }
}

impl FxEffectSlot {
    pub fn has_refs(&self) -> bool {
        self.status & FX_STATUS_REF_COUNT_LOW_MASK != 0
    }

    pub fn frame_now(&self) -> FxOrientFrame {
        FxOrientFrame {
            origin: self.origin,
            axis: self.axis,
        }
    }

    pub fn frame_when_played(&self) -> FxOrientFrame {
        FxOrientFrame {
            origin: self.origin_when_played,
            axis: self.axis_when_played,
        }
    }

    pub fn commit_frame_last_from_now(&mut self) {
        self.origin_last = self.origin;
        self.axis_last = self.axis;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnFail {
    RingFull,
    TooManySpotlights,
    EffectLimit,
}

/// A runner element asking the renderer to start a child effect.
#[derive(Clone, Debug)]
pub struct PendingRunnerSpawn {
    pub mark_entity: Option<u16>,
    pub parent_name: String,
    pub catalog_index: u16,
    pub def_index: u8,
    pub msec_begin: i32,
    pub random_seed: u32,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub rot_deg: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct PendingSoundSpawn {
    pub parent_name: String,
    pub catalog_index: u16,
    pub def_index: u8,
    pub msec_begin: i32,
    pub random_seed: u32,
    pub origin: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct PendingDecalSpawn {
    pub parent_name: String,
    pub catalog_index: u16,
    pub def_index: u8,
    pub msec_begin: i32,
    pub random_seed: u32,
    pub origin: [f32; 3],
    pub bolt: u8,
    pub mark_entity: Option<u16>,
    pub axis: [[f32; 3]; 3],
}

#[derive(Clone, Debug)]
pub struct PendingTrailImpact {
    pub parent_def_name: String,
    pub catalog_index: u16,
    pub def_index: u8,
    pub origin: [f32; 3],
    pub pre_vel: [f32; 3],
    pub msec: i32,
}

pub struct FxSystemHost {
    /// The entity that effects spawned from now on belong to, for marks.
    pub spawn_mark_entity: Option<u16>,
    pub last_decal_mark_entity: Option<u16>,

    effects: Vec<FxEffectSlot>,
    /// The handle ring: live effects between `first_active_effect` and `first_free_effect`.
    handles: Vec<u16>,
    pub first_active_effect: i32,
    pub first_new_effect: i32,
    pub first_free_effect: i32,
    pub spotlight_count: i32,

    pub(crate) elems: Vec<FxElemSlot>,
    pub(crate) sort_distances: Vec<(u32, f32)>,
    pub(crate) sort_epoch: u32,
    pub(crate) elem_first_free: Option<usize>,
    pub elem_live_count: u32,
    pub elem_alloc_failures: u32,

    pub(crate) trails: Vec<FxTrailSlot>,
    pub(crate) trail_first_free: Option<usize>,
    pub(crate) trail_elems: Vec<FxTrailElemSlot>,
    pub(crate) trail_elem_first_free: Option<usize>,

    pub vis_blocker_write: FxVisBlockerBuf,
    pub vis_blocker_read: FxVisBlockerBuf,
    pub gaps: FxGaps,
    pub glass: FxGlassSystemHost,

    pub bolt_first_free: i32,
    bolt_next: Vec<i32>,

    pub msec_now: i32,
    pub frame_stamp: i32,
    pub iterator_count: i32,
    pub needs_garbage_collection: bool,

    /// The camera at the last update, for each part's spawn range (its
    /// distance tier: a fire's near flames within 600 units, its far ones beyond).
    pub camera_origin: Option<[f32; 3]>,

    pub last_runner_parent: Option<String>,
    pub last_runner_elem: Option<u8>,
    pub last_runner_child: Option<String>,
    pub last_runner_msec: Option<i32>,
    pub last_runner_rot_deg: Option<f32>,

    pub last_decal_parent: Option<String>,
    pub last_decal_elem: Option<u8>,
    pub last_decal_msec: Option<i32>,
    pub last_decal_vis: Option<u8>,
    pub last_decal_size0: Option<f32>,
    pub last_decal_rotation: Option<f32>,
    pub last_decal_mat0_edge: Option<String>,
    pub last_decal_mat1_edge: Option<String>,
    pub last_decal_mat0: Option<String>,
    pub last_decal_mat1: Option<String>,
    pub last_decal_origin: Option<[f32; 3]>,
    pub last_decal_axis: Option<[[f32; 3]; 3]>,
    pub last_decal_bolt: Option<u8>,
    pub last_decal_against_world: Option<bool>,
    pub last_decal_against_models: Option<bool>,
    pub last_decal_color: Option<u32>,

    pub gfx_mark_surf_n: Option<u32>,
    pub gfx_mark_vert_n: Option<u32>,
    pub gfx_mark_index_n: Option<u32>,
    pub gfx_mark_packed_n: Option<u32>,
    pub gfx_mark_draw_n: Option<u32>,
    pub gfx_mark_draw_tri: Option<u32>,
    pub gfx_mark_draw_skip_why: Option<String>,
    pub last_mark_lmap: Option<u8>,
    pub last_mark_primary_light: Option<u8>,
    pub last_mark_probe: Option<u8>,
    pub last_mark_lmap_none_n: Option<u32>,
    pub last_mark_lmap_page_n: Option<u32>,
    pub last_mark_mesh: Option<GfxMarkMeshCensus>,

    pub marks: FxMarksSystemHost,
    pub mark_receivers: MarkReceiverEnable,
    mark_trace_seq: u32,
    pub mark_traces: Vec<MarkTraceRecord>,
    pub last_mark_alloc_slot: Option<u16>,

    pub pending_runners: Vec<PendingRunnerSpawn>,
    pub pending_sounds: Vec<PendingSoundSpawn>,
    pub pending_decals: Vec<PendingDecalSpawn>,
    pub pending_trail_impacts: Vec<PendingTrailImpact>,
}

impl Default for FxSystemHost {
    fn default() -> Self {
        Self::new()
    }
}

/// `count` default slots.
fn pool<T: Default>(count: u32) -> Vec<T> {
    (0..count).map(|_| T::default()).collect()
}

/// Strings the slots together front to back so each names its successor,
/// the last one naming `end`.
fn string_together<T>(slots: &mut [T], end: u16, set_next: impl Fn(&mut T, u16)) {
    let last = slots.len().saturating_sub(1);
    for (i, slot) in slots.iter_mut().enumerate() {
        set_next(slot, if i < last { (i + 1) as u16 } else { end });
    }
}

/// The pool index of the effect an effect handle names; None for a handle
/// that does not land on a slot boundary or is past the pool.
fn slot_of_handle(handle: u16) -> Option<usize> {
    let byte_offset = u32::from(handle) << 2;
    let slot_size = FX_EFFECT_SLOT_SIZE as u32;
    if byte_offset % slot_size != 0 {
        return None;
    }
    let slot = (byte_offset / slot_size) as usize;
    (slot < FX_EFFECT_POOL_CAPACITY as usize).then_some(slot)
}

fn count_or_null(count: Option<u32>) -> String {
    count.map_or_else(|| "null".to_owned(), |n| n.to_string())
}

impl FxSystemHost {
    pub fn new() -> Self {
        let mut elems = pool::<FxElemSlot>(FX_ELEM_POOL_CAPACITY);
        string_together(&mut elems, FX_ELEM_HANDLE_NONE, |slot, next| {
            slot.next_elem_handle = next;
        });
        let mut trails = pool::<FxTrailSlot>(FX_TRAIL_POOL_CAPACITY);
        string_together(&mut trails, FX_TRAIL_HANDLE_NONE, |slot, next| {
            slot.next_trail_handle = next;
        });
        let mut trail_elems = pool::<FxTrailElemSlot>(FX_TRAIL_ELEM_POOL_CAPACITY);
        string_together(&mut trail_elems, FX_TRAIL_HANDLE_NONE, |slot, next| {
            slot.next_trail_elem_handle = next;
        });

        Self {
            spawn_mark_entity: None,
            last_decal_mark_entity: None,
            effects: pool(FX_EFFECT_POOL_CAPACITY),
            handles: (0..FX_EFFECT_POOL_CAPACITY)
                .map(effect_handle_for_slot)
                .collect(),
            first_active_effect: 0,
            first_new_effect: 0,
            first_free_effect: 0,
            spotlight_count: 0,
            sort_distances: vec![(0, 0.0); elems.len()],
            elems,
            sort_epoch: 0,
            elem_first_free: Some(0),
            elem_live_count: 0,
            elem_alloc_failures: 0,
            trails,
            trail_first_free: Some(0),
            trail_elems,
            trail_elem_first_free: Some(0),
            vis_blocker_write: FxVisBlockerBuf::default(),
            vis_blocker_read: FxVisBlockerBuf::default(),
            gaps: FxGaps::default(),
            glass: FxGlassSystemHost::default(),
            bolt_first_free: 0,
            bolt_next: (0..FX_BOLT_RECORD_CAPACITY as i32)
                .map(bolt_init_next_index)
                .collect(),
            msec_now: 0,
            frame_stamp: 0,
            iterator_count: 0,
            needs_garbage_collection: false,
            camera_origin: None,
            last_runner_parent: None,
            last_runner_elem: None,
            last_runner_child: None,
            last_runner_msec: None,
            last_runner_rot_deg: None,
            last_decal_parent: None,
            last_decal_elem: None,
            last_decal_msec: None,
            last_decal_vis: None,
            last_decal_size0: None,
            last_decal_rotation: None,
            last_decal_mat0_edge: None,
            last_decal_mat1_edge: None,
            last_decal_mat0: None,
            last_decal_mat1: None,
            last_decal_origin: None,
            last_decal_axis: None,
            last_decal_bolt: None,
            last_decal_against_world: None,
            last_decal_against_models: None,
            last_decal_color: None,
            gfx_mark_surf_n: None,
            gfx_mark_vert_n: None,
            gfx_mark_index_n: None,
            gfx_mark_packed_n: None,
            gfx_mark_draw_n: None,
            gfx_mark_draw_tri: None,
            gfx_mark_draw_skip_why: None,
            last_mark_lmap: None,
            last_mark_primary_light: None,
            last_mark_probe: None,
            last_mark_lmap_none_n: None,
            last_mark_lmap_page_n: None,
            last_mark_mesh: None,
            marks: FxMarksSystemHost::init(),
            mark_receivers: MarkReceiverEnable::default(),
            mark_trace_seq: 0,
            mark_traces: Vec::new(),
            last_mark_alloc_slot: None,
            pending_runners: Vec::new(),
            pending_sounds: Vec::new(),
            pending_decals: Vec::new(),
            pending_trail_impacts: Vec::new(),
        }
    }

    // -- looking effects up ------------------------------------------------

    pub fn live_effects(&self) -> impl Iterator<Item = &FxEffectSlot> {
        self.effects
            .iter()
            .filter(|effect| effect.ring_resident && effect.has_refs())
    }

    pub fn live_elems(&self) -> impl Iterator<Item = &FxElemSlot> {
        self.elems.iter().filter(|elem| elem.occupied)
    }

    pub fn slot_for_handle(&self, handle: u16) -> Option<&FxEffectSlot> {
        let slot = self.slot_index_for_handle(handle)?;
        self.effects.get(slot)
    }

    pub fn slot_for_handle_mut(&mut self, handle: u16) -> Option<&mut FxEffectSlot> {
        let slot = self.slot_index_for_handle(handle)?;
        self.effects.get_mut(slot)
    }

    /// The pool index of the effect `handle` names, if one lives there now.
    pub(crate) fn slot_index_for_handle(&self, handle: u16) -> Option<usize> {
        let slot = slot_of_handle(handle)?;
        self.effects.get(slot)?.ring_resident.then_some(slot)
    }

    pub fn effect_at(&self, slot: usize) -> Option<&FxEffectSlot> {
        self.effects.get(slot)
    }

    pub(crate) fn effect_at_mut(&mut self, slot: usize) -> Option<&mut FxEffectSlot> {
        self.effects.get_mut(slot)
    }

    // -- the handle ring ---------------------------------------------------

    pub(crate) fn handle_at_ring(&self, ring_index: u32) -> u16 {
        self.handles[(ring_index & (FX_EFFECT_HANDLE_RING_SIZE - 1)) as usize]
    }

    fn set_handle_at_ring(&mut self, ring_index: u32, handle: u16) {
        self.handles[(ring_index & (FX_EFFECT_HANDLE_RING_SIZE - 1)) as usize] = handle;
    }

    /// How many ring entries are in use.
    pub fn ring_occupancy(&self) -> i32 {
        self.first_free_effect
            .wrapping_sub(self.first_active_effect)
    }

    fn alloc_bolt_record(&mut self) -> Option<u8> {
        let first = self.bolt_first_free;
        let next = self
            .bolt_next
            .get(first as usize)
            .copied()
            .unwrap_or(FX_BOLT_FREE_NONE);
        let (index, new_first) = bolt_alloc(first, next)?;
        self.bolt_first_free = new_first;
        Some(index as u8)
    }

    /// Starts an effect in the next free ring entry and returns its handle.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_effect(
        &mut self,
        def_name: &str,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
        msec: i32,
        bolt: u32,
        wants_spotlight: bool,
        catalog_index: u16,
    ) -> Result<u16, SpawnFail> {
        if wants_spotlight && self.spotlight_count >= FX_SPOT_LIGHT_LIMIT {
            return Err(SpawnFail::TooManySpotlights);
        }
        let bolt_record = if bolt == FX_SPAWN_BOLT_NONE {
            BOLT_NONE
        } else {
            self.alloc_bolt_record().ok_or(SpawnFail::EffectLimit)?
        };

        if self.ring_occupancy() >= FX_EFFECT_HANDLE_RING_SIZE as i32 {
            return Err(SpawnFail::RingFull);
        }
        let ring_position = self.first_free_effect;
        self.first_free_effect = ring_position.wrapping_add(1);

        let handle = self.handle_at_ring(ring_position as u32);
        let slot = slot_of_handle(handle).expect("the handle ring only holds valid slot handles");
        let (bolt_parent_quat, bolt_parent_origin) = bolt_init_parent_orientation();
        self.effects[slot] = FxEffectSlot {
            def_name: def_name.to_owned(),
            catalog_index,
            status: NEW_EFFECT_STATUS,
            random_seed: effect_random_seed_from_msec(msec),
            own_handle: handle,
            bolt: bolt_record,
            mark_entity: self.spawn_mark_entity,
            bolt_parent_quat,
            bolt_parent_origin,
            msec_begin: msec,
            msec_last_update: msec,
            origin,
            axis,
            origin_when_played: origin,
            axis_when_played: axis,
            origin_last: origin,
            axis_last: axis,
            ring_resident: true,
            ..FxEffectSlot::default()
        };

        if self.first_new_effect <= ring_position {
            self.first_new_effect = ring_position.wrapping_add(1);
        }
        if wants_spotlight {
            self.spotlight_count = self.spotlight_count.saturating_add(1);
        }
        Ok(handle)
    }

    /// An effect lost a claim; the next collection looks for effects left with none.
    pub fn del_ref_to_effect(&mut self, _handle: u16) {
        self.needs_garbage_collection = true;
    }

    /// The player lets go of its claim on an effect.
    pub fn play_release_ownership(&mut self, handle: u16) {
        let last_claim_gone = self
            .slot_for_handle(handle)
            .is_some_and(|effect| status_is_unique_done(effect.status));
        if last_claim_gone {
            self.del_ref_to_effect(handle);
        }
        if let Some(effect) = self.slot_for_handle_mut(handle) {
            effect.status = effect.status.wrapping_sub(1);
        }
    }

    /// Reclaims every effect with no claims left: its elements and trails
    /// are freed, its slot is reset, and the live handles close ranks in the
    /// ring behind the freed ones.
    pub fn run_garbage_collection(&mut self) {
        if !self.needs_garbage_collection {
            return;
        }
        let oldest = self.first_active_effect;
        let mut cursor = self.first_new_effect;
        let mut dead: Vec<u16> = Vec::new();
        while cursor != oldest {
            cursor = cursor.wrapping_sub(1);
            let handle = self.handle_at_ring(cursor as u32);
            let resident = self.slot_index_for_handle(handle);
            let alive = resident.is_some_and(|slot| self.effects[slot].has_refs());
            if alive {
                let shifted = (cursor as u32).wrapping_add(dead.len() as u32);
                self.set_handle_at_ring(shifted, handle);
                continue;
            }
            if let Some(slot) = resident {
                free_all_elems_for_effect(self, slot);
                free_all_trails_for_effect(self, slot);
            }
            dead.push(handle);
        }
        for handle in dead.into_iter().rev() {
            self.set_handle_at_ring(cursor as u32, handle);
            cursor = cursor.wrapping_add(1);
            if let Some(slot) = slot_of_handle(handle) {
                self.effects[slot] = FxEffectSlot::default();
            }
        }
        self.first_active_effect = cursor;
        self.needs_garbage_collection = false;
    }

    // -- stopping effects --------------------------------------------------

    fn slots_bolted_to(&self, def_name: &str, dobj: u32, bone: u16) -> Vec<usize> {
        self.effects
            .iter()
            .enumerate()
            .filter(|(_, effect)| {
                effect.ring_resident
                    && effect.def_name == def_name
                    && bolt_dobj(effect.bolt_packed) == dobj
                    && bolt_bone(effect.bolt_packed) == u32::from(bone)
            })
            .map(|(slot, _)| slot)
            .collect()
    }

    /// Frees what the effects show and every claim on them, then collects them.
    fn end_effects_at_once(&mut self, slots: &[usize]) {
        for &slot in slots {
            free_all_elems_for_effect(self, slot);
            free_all_trails_for_effect(self, slot);
            if let Some(effect) = self.effect_at_mut(slot) {
                effect.status &= !(FX_STATUS_REF_COUNT_LOW_MASK | FX_STATUS_HAS_PENDING_LOOP_ELEMS);
            }
        }
        self.needs_garbage_collection = true;
        self.run_garbage_collection();
    }

    pub fn stop_bolted(&mut self, def_name: &str, dobj: u32, bone: u16) {
        for slot in self.slots_bolted_to(def_name, dobj, bone) {
            stop_effect_non_recursive(self, slot);
        }
    }

    pub fn stop_owned(&mut self, handle: u16) {
        let Some(slot) = self.slot_index_for_handle(handle) else {
            return;
        };
        stop_effect_non_recursive(self, slot);
        self.play_release_ownership(handle);
    }

    /// Like `stop_bolted`, but whatever the effects still show goes at once as
    /// well, including a one-shot whose spawning is already done. Returns how
    /// many effects went.
    pub fn kill_bolted(&mut self, def_name: &str, dobj: u32, bone: u16) -> usize {
        let slots = self.slots_bolted_to(def_name, dobj, bone);
        if !slots.is_empty() {
            self.end_effects_at_once(&slots);
        }
        slots.len()
    }

    /// Ends every effect of `def_name` that began at or after `msec_begin`.
    pub fn kill_def_newer_than(&mut self, def_name: &str, msec_begin: i32) {
        let slots: Vec<usize> = self
            .effects
            .iter()
            .enumerate()
            .filter(|(_, effect)| {
                effect.ring_resident
                    && effect.has_refs()
                    && effect.def_name == def_name
                    && effect.msec_begin >= msec_begin
            })
            .map(|(slot, _)| slot)
            .collect();
        if !slots.is_empty() {
            self.end_effects_at_once(&slots);
        }
    }

    /// Asks `resolve` for the current pose of each bolted effect's bone.
    pub fn refresh_bolt_poses(
        &mut self,
        mut resolve: impl FnMut(u32, u16) -> Option<FxResolvedBoltPose>,
    ) {
        for effect in &mut self.effects {
            if !effect.ring_resident || bolt_handle_is_none(effect.bolt_packed) {
                continue;
            }
            let pose = resolve(
                bolt_dobj(effect.bolt_packed),
                bolt_bone(effect.bolt_packed) as u16,
            );
            effect.bolt_centity_teleport = pose.is_some_and(|p| p.centity_teleport);
            effect.bolt_bone_pose = pose
                .and_then(|p| p.orientation)
                .map(|o| (o.origin, o.axis));
        }
    }

    // -- the clock ---------------------------------------------------------

    pub fn set_msec_now(&mut self, msec: i32) {
        self.msec_now = msec;
        self.marks.update_grid(msec);
    }

    pub fn advance_frame_stamp(&mut self) {
        self.frame_stamp = self.frame_stamp.wrapping_add(1);
    }

    // -- marks and decals --------------------------------------------------

    /// Asks the mark host whether the decal noted in `last_decal_*` may
    /// leave a mark, and records which receivers it would land on.
    pub fn record_impact_mark_from_decal(&mut self, bolt: u8) -> MarkImpactResult {
        let result = self.marks.impact_mark(MarkImpactRequest {
            skip_world: marks_t6::impact_mark_skip_world_from_stored_bolt(bolt),
            receivers: self.mark_receivers,
            camera: self.camera_origin,
            origin: self.last_decal_origin,
        });
        self.last_decal_against_world = result.entered.then_some(result.against_world);
        self.last_decal_against_models = result.entered.then_some(result.against_models);
        result
    }

    pub fn note_world_go_skip(&mut self, def_index: u8) {
        self.gaps.raise(FxGapCause::MarkFragmentsSkipped { def_index });
    }

    /// The mark host keeps its own counts, so a successful world generate
    /// leaves nothing more to note.
    pub fn note_world_go_fire(&mut self) {}

    pub fn note_models_go_skip(&mut self, def_index: u8) {
        self.gaps.raise(FxGapCause::MarkFragmentsSkipped { def_index });
    }

    /// As `note_world_go_fire`.
    pub fn note_models_go_fire(&mut self) {}

    pub fn note_impact_mark_outer_skip(&mut self, def_index: u8) {
        self.gaps
            .raise(FxGapCause::ElemDecalSpawnSkipped { def_index });
    }

    /// Rebuilds the world-mark mesh, empty while the world receivers are off.
    pub fn generate_world_mark_verts(&mut self) {
        let enabled = self.mark_receivers.fx_marks;
        let mesh = if enabled {
            self.marks.generate_world_mark_verts()
        } else {
            GfxMarkMeshCensus::default()
        };
        let counted = |n: u32| Some(if enabled { n } else { 0 });
        self.gfx_mark_surf_n = counted(mesh.budget.surf_n);
        self.gfx_mark_vert_n = counted(mesh.budget.vert_n);
        self.gfx_mark_index_n = counted(mesh.budget.index_n);
        self.gfx_mark_packed_n = Some(mesh.packed.len() as u32);
        self.last_mark_mesh = Some(mesh);
    }

    fn trace_skip_reason(&self, result: &MarkImpactResult, allocated: bool) -> Option<&'static str> {
        if !result.entered {
            Some(if self.marks.no_marks {
                "no_marks"
            } else if !self.mark_receivers.fx_marks {
                "fx_marks=0"
            } else {
                "outer_gate"
            })
        } else if !allocated {
            Some("no_alloc")
        } else {
            None
        }
    }

    /// Files a trace of the latest decal attempt: what was asked, what the
    /// mark host said, and the mark that came of it.
    pub fn push_mark_trace(&mut self, result: &MarkImpactResult) {
        self.mark_trace_seq = match self.mark_trace_seq.wrapping_add(1) {
            0 => 1,
            seq => seq,
        };
        let slot = self.last_mark_alloc_slot.take();
        let mark = slot.and_then(|s| self.marks.constructed(s).copied());
        let normal = self.last_decal_axis.map(|axis| axis[2]);
        let record = MarkTraceRecord {
            id: self.mark_trace_seq,
            bolt: self.last_decal_bolt.unwrap_or(BOLT_NONE),
            entered: result.entered,
            against_world: result.against_world,
            against_models: result.against_models,
            parent: self.last_decal_parent.clone(),
            elem: self.last_decal_elem,
            msec: self.last_decal_msec,
            origin: self.last_decal_origin,
            size0: self.last_decal_size0,
            color: self.last_decal_color,
            color_kind: if self.last_decal_color.is_some() {
                "sampled"
            } else {
                "none"
            },
            mat0: self.last_decal_mat0.clone(),
            mat1: self.last_decal_mat1.clone(),
            bound: slot.and_then(|s| self.marks.material_name(s).map(str::to_owned)),
            slot,
            tri_n: mark.map(|m| m.tri_count),
            point_n: mark.map(|m| m.point_count),
            native_color: mark.map(|m| m.native_color),
            context: mark.map(|m| m.context),
            skip_why: self.trace_skip_reason(result, slot.is_some()),
            vis: self.last_decal_vis,
            mat0_edge: self.last_decal_mat0_edge.clone(),
            mat1_edge: self.last_decal_mat1_edge.clone(),
            nx: normal.map(|n| n[0]),
            ny: normal.map(|n| n[1]),
            nz: normal.map(|n| n[2]),
        };
        if self.mark_traces.len() >= MARK_TRACE_LIMIT {
            self.mark_traces.remove(0);
        }
        self.mark_traces.push(record);
    }

    /// One line of mark pool numbers for the console.
    pub fn mark_profile_line(&self) -> String {
        format!(
            "fx_mark_profile live={}/{} alloced={} tri_free={} point_free={} gfx_surf={} gfx_draw={} unlinked={} ents_AddEntity=typed_gap draw_gate={}",
            self.marks.live_count(),
            marks_t6::FX_MARKS_LIMIT,
            self.marks.alloced_count(),
            self.marks.tri_first_free(),
            self.marks.point_first_free(),
            count_or_null(self.gfx_mark_surf_n),
            count_or_null(self.gfx_mark_draw_n),
            self.marks.live_count(),
            if self.mark_receivers.fx_marks {
                "on"
            } else {
                "fx_marks=0"
            },
        )
    }
}
