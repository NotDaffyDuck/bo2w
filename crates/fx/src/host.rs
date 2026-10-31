//! The effect host: pools for effects, elements and trails, the ring of
//! effect handles with its garbage collection, bolt records, and the notes
//! the renderer leaves about decals and marks.

use fx_t6::runtime::{
    NO_SPARE_ATTACH, NO_ATTACH_TICKET, ATTACH_ROOT_NO_TURN, ATTACH_RECORD_LIMIT,
    PARTICLE_BANK_LIMIT, BIRTH_UNATTACHED, CONE_GLOW_LIMIT,
    EMITTER_REPEAT_QUEUED_BIT, FX_STATUS_REF_COUNT_LOW_MASK, STREAK_JOINT_BANK_LIMIT,
    STREAK_BANK_LIMIT,
};
use fx_t6::runtime::{
    EMITTER_TICKET_WHEEL_LEN, EMITTER_BANK_LIMIT, EMITTER_SLOT_BYTES, EffectFacingBasis,
    SightOccluderStore, bolt_alloc, bolt_bone, bolt_dobj, bolt_handle_is_none, bolt_init_next_index,
    bolt_init_parent_orientation, effect_handle_for_slot, effect_random_seed_from_msec,
    status_is_unique_done,
};

use crate::particle_bin::{NO_PARTICLE_TICKET, ParticleBinSlot};
use crate::holes::{EffectHoleReason, GapBook};
use crate::pane::PaneShardOwner;
use crate::decal::{
    DecalOwner, DecalMeshTally, DecalContactAsk, MarkOutcome, DecalAcceptToggle,
    DecalRayLog,
};
use crate::birth::{
    free_all_elems_for_effect, free_all_trails_for_effect, halt_effect_alone,
};
use crate::ribbon::{NO_STREAK_TICKET, RibbonPointCell, RibbonCell};

pub const NO_RECIPE_LISTING: u16 = u16::MAX;

/// What an effect's `bolt` holds when no bolt record belongs to it.
pub(crate) const BOLT_NONE: u8 = 0xff;

/// What `frame_tick_no` holds for an effect no frame has visited yet.
const NEVER_STAMPED: i32 = -1;

/// A new effect holds one claim (its player's) and is marked freshly played.
const NEW_EFFECT_STATUS: u32 = 0x4000_0001;

/// How many mark traces are kept before the oldest is dropped.
const MARK_TRACE_LIMIT: usize = 512;

/// The claims an effect holds on itself, which ending it at once drops.
const ALL_CLAIMS: u32 = FX_STATUS_REF_COUNT_LOW_MASK | EMITTER_REPEAT_QUEUED_BIT;

const WORLD_FORWARD: [f32; 3] = [1.0, 0.0, 0.0];
const WORLD_LEFT: [f32; 3] = [0.0, 1.0, 0.0];
const WORLD_UP: [f32; 3] = [0.0, 0.0, 1.0];

/// The world's own forward, left and up axes: the basis of an effect started without a turn.
pub const UNTURNED_BASIS: [[f32; 3]; 3] = [WORLD_FORWARD, WORLD_LEFT, WORLD_UP];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttachPointFrame {
    pub axis: [[f32; 3]; 3],
    pub origin: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectBoltTarget {
    pub orientation: AttachPointFrame,
    pub client_actor_teleport: bool,
    pub bone: u16,
    pub dobj: u32,
}

/// Where a bolt's bone ended up this frame; no orientation when it can't be found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettledAttachFrame {
    pub orientation: Option<AttachPointFrame>,
    pub client_actor_teleport: bool,
}

/// Where an effect's packed lighting colour came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BakedShadeSource {
    #[default]
    White = 0x00,
    AtPointRgb = 0x01,
    Missing = 0x02,
}

#[derive(Clone, Debug)]
pub struct LiveEffect {
    pub definition_name: String,
    pub ledger_index: u16,
    pub state_bits: u32,
    /// Whether this slot currently holds an effect named in the handle ring.
    pub in_ring: bool,
    pub self_ticket: u16,
    pub stain_owner: Option<u16>,
    pub lead_particle_ticket: [u16; 3],
    pub first_part_ticket: u16,
    pub first_ribbon_ticket: u16,
    pub dice_start: u16,
    pub compact_illum: [u8; 3],
    pub packed_lighting_src: BakedShadeSource,
    pub tether: u8,
    pub tether_bits: u32,
    pub bolt_teleported: bool,
    pub bolt_parent_rot: [f32; 4],
    pub tether_spot: [f32; 3],
    pub tether_pose: Option<([f32; 3], [[f32; 3]; 3])>,
    pub frame_tick_no: i32,
    pub window_open_ms: i32,
    pub ms_last_tick: i32,
    pub where_at: [f32; 3],
    pub orient_rows: [[f32; 3]; 3],
    pub spot_at_launch: [f32; 3],
    pub frame_at_launch: [[f32; 3]; 3],
    pub spot_before: [f32; 3],
    pub frame_before: [[f32; 3]; 3],
    pub range_to_eye: f32,
}

impl Default for LiveEffect {
    fn default() -> Self {
        let unlinked = NO_PARTICLE_TICKET;
        let no_catalog = NO_RECIPE_LISTING;
        let (rest_origin, rest_axis) = ([0.0; 3], UNTURNED_BASIS);
        Self {
            definition_name: String::default(),
            ledger_index: no_catalog,
            state_bits: u32::default(),
            in_ring: bool::default(),
            self_ticket: u16::default(),
            stain_owner: Option::default(),
            lead_particle_ticket: [unlinked; 3],
            first_part_ticket: unlinked,
            first_ribbon_ticket: unlinked,
            dice_start: u16::default(),
            compact_illum: [u8::MAX; 3],
            packed_lighting_src: BakedShadeSource::default(),
            tether: BOLT_NONE,
            tether_bits: NO_ATTACH_TICKET,
            bolt_teleported: bool::default(),
            bolt_parent_rot: ATTACH_ROOT_NO_TURN,
            tether_spot: rest_origin,
            tether_pose: Option::default(),
            frame_tick_no: NEVER_STAMPED,
            window_open_ms: i32::default(),
            ms_last_tick: i32::default(),
            where_at: rest_origin,
            orient_rows: rest_axis,
            spot_at_launch: rest_origin,
            frame_at_launch: rest_axis,
            spot_before: rest_origin,
            frame_before: rest_axis,
            range_to_eye: f32::default(),
        }
    }
}

impl LiveEffect {
    /// A just-played effect: one claim, the given pose as now, as played and
    /// as last frame, and a bolt parent that has not been bolted yet.
    fn begin(start: EffectStart<'_>) -> Self {
        let mut effect = Self::default();
        effect.definition_name.push_str(start.name);
        effect.ledger_index = start.catalog;
        effect.self_ticket = start.handle;
        effect.state_bits = NEW_EFFECT_STATUS;
        effect.tether = start.bolt_record;
        effect.stain_owner = start.owner;
        effect.dice_start = effect_random_seed_from_msec(start.msec);
        effect.window_open_ms = start.msec;
        effect.ms_last_tick = start.msec;
        effect.in_ring = true;
        (effect.bolt_parent_rot, effect.tether_spot) = bolt_init_parent_orientation();
        effect.place(start.spot, start.turn_rows);
        effect
    }

    /// Puts the effect at `origin`/`axis`: as now, as when played, as last frame.
    fn place(&mut self, pos: [f32; 3], basis: [[f32; 3]; 3]) {
        self.spot_before = pos;
        self.spot_at_launch = pos;
        self.where_at = pos;
        self.frame_before = basis;
        self.frame_at_launch = basis;
        self.orient_rows = basis;
    }

    pub fn holds_refs(&self) -> bool {
        (self.state_bits & FX_STATUS_REF_COUNT_LOW_MASK) > 0
    }

    pub fn current_frame(&self) -> EffectFacingBasis {
        let (pos, axis) = (self.where_at, self.orient_rows);
        EffectFacingBasis { origin: pos, axis }
    }

    pub fn spawn_frame(&self) -> EffectFacingBasis {
        let (pos, axis) = (self.spot_at_launch, self.frame_at_launch);
        EffectFacingBasis { origin: pos, axis }
    }

    pub fn carry_frame_forward(&mut self) {
        let latest = (self.where_at, self.orient_rows);
        self.spot_before = latest.0;
        self.frame_before = latest.1;
    }
}

/// Everything `LiveEffect::begin` needs to fill a fresh effect in.
struct EffectStart<'a> {
    name: &'a str,
    catalog: u16,
    handle: u16,
    msec: i32,
    bolt_record: u8,
    owner: Option<u16>,
    spot: [f32; 3],
    turn_rows: [[f32; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnRefusal {
    RingFull,
    SpotlightCap,
    PoolFull,
}

/// A runner element asking the renderer to start a child effect.
#[derive(Clone, Debug)]
pub struct QueuedSpawnerBirth {
    pub source_name: String,
    pub ledger_index: u16,
    pub definition_slot: u8,
    pub stain_ent: Option<u16>,
    pub turn_deg: Option<f32>,
    pub dice_start: u32,
    pub window_open_ms: i32,
    pub orient_rows: [[f32; 3]; 3],
    pub spot: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct QueuedSound {
    pub anchor: [f32; 3],
    pub dice_start: u32,
    pub window_open_ms: i32,
    pub definition_slot: u8,
    pub ledger_index: u16,
    pub owner_name: String,
}

#[derive(Clone, Debug)]
pub struct QueuedDecalBirth {
    pub turn_rows: [[f32; 3]; 3],
    pub where_at: [f32; 3],
    pub stain_on_ent: Option<u16>,
    pub tether_ix: u8,
    pub dice_start: u32,
    pub window_open_ms: i32,
    pub definition_slot: u8,
    pub ledger_index: u16,
    pub src_name: String,
}

#[derive(Clone, Debug)]
pub struct QueuedTrailHit {
    pub ms_now: i32,
    pub prior_vel: [f32; 3],
    pub place_at: [f32; 3],
    pub definition_slot: u8,
    pub ledger_index: u16,
    pub source_def_name: String,
}

pub struct EffectSystemHost {
    /// The entity that effects spawned from now on belong to, for marks.
    pub stain_target_entity: Option<u16>,
    pub last_stain_entity: Option<u16>,

    /// Every effect slot in the pool, indexed by pool position.
    cell_list: Vec<LiveEffect>,
    /// The handle ring: live effects between `first_active_effect` and `first_free_effect`.
    ring: Vec<u16>,
    pub(crate) first_active_effect: i32,
    pub(crate) first_new_effect: i32,
    pub(crate) first_free_effect: i32,
    pub(crate) spotlight_count: i32,
    pub(crate) gc_pending: bool,

    pub(crate) elems: Vec<ParticleBinSlot>,
    pub(crate) first_spare_particle: Option<usize>,
    pub(crate) sort_distances: Vec<(u32, f32)>,
    pub(crate) sort_epoch: u32,
    pub live_parts: u32,
    pub part_take_failures: u32,

    pub(crate) trails: Vec<RibbonCell>,
    pub(crate) ribbon_free_head: Option<usize>,
    pub(crate) trail_elems: Vec<RibbonPointCell>,
    pub(crate) ribbon_head_free: Option<usize>,

    pub vis_blocker_write: SightOccluderStore,
    pub vis_blocker_read: SightOccluderStore,
    pub gaps: GapBook,
    pub glass: PaneShardOwner,

    /// The bolt record chain: each entry names the record after it.
    bolt_links: Vec<i32>,
    pub(crate) bolt_first_free: i32,

    pub clock_ms_now: i32,
    pub frame_tick_no: i32,
    pub(crate) walker_total: i32,

    /// The camera at the last update, for each part's spawn range (its
    /// distance tier: a fire's near flames within 600 units, its far ones beyond).
    pub camera_origin: Option<[f32; 3]>,

    pub last_emitter_source: Option<String>,
    pub last_emitter_part: Option<u8>,
    pub last_emitter_child: Option<String>,
    pub last_emitter_ms: Option<i32>,
    pub last_emitter_turn_deg: Option<f32>,

    pub last_stain_source: Option<String>,
    pub last_stain_part: Option<u8>,
    pub last_stain_ms: Option<i32>,
    pub prior_decal_seen: Option<u8>,
    pub last_stain_size0: Option<f32>,
    pub last_stain_rotation: Option<f32>,
    pub prior_decal_first_border: Option<String>,
    pub prior_decal_second_border: Option<String>,
    pub last_stain_mat0: Option<String>,
    pub prior_decal_second_surface: Option<String>,
    pub last_stain_origin: Option<[f32; 3]>,
    pub last_stain_axis: Option<[[f32; 3]; 3]>,
    pub last_stain_tether: Option<u8>,
    pub last_stain_on_world: Option<bool>,
    pub last_stain_on_models: Option<bool>,
    pub last_stain_color: Option<u32>,

    pub stain_surfs: Option<u32>,
    pub stain_verts: Option<u32>,
    pub stain_indices: Option<u32>,
    pub visual_stain_compact_tally: Option<u32>,
    pub stain_draws: Option<u32>,
    pub stain_draw_tris: Option<u32>,
    pub stain_draw_skip: Option<String>,
    pub last_stain_lightmap: Option<u8>,
    pub last_stain_light: Option<u8>,
    pub last_stain_probe: Option<u8>,
    pub stain_no_lightmap_count: Option<u32>,
    pub stain_lightmap_page_count: Option<u32>,
    pub last_stain_mesh: Option<DecalMeshTally>,

    pub marks: DecalOwner,
    pub stain_receivers: DecalAcceptToggle,
    trace_serial: u32,
    pub(crate) mark_traces: Vec<DecalRayLog>,
    pub last_stain_alloc_slot: Option<u16>,

    pub pending_runners: Vec<QueuedSpawnerBirth>,
    pub pending_sounds: Vec<QueuedSound>,
    pub pending_decals: Vec<QueuedDecalBirth>,
    pub pending_trail_impacts: Vec<QueuedTrailHit>,
}

impl Default for EffectSystemHost {
    fn default() -> Self {
        Self::new()
    }
}

/// `count` default slots.
fn blank_pool<T: Default>(count: u32) -> Vec<T> {
    (0..count).map(|_| T::default()).collect()
}

/// `count` default slots chained together: each names the slot after it by
/// index, and the last one names `end`.
fn chained_pool<T: Default>(count: u32, end: u16, set_next: fn(&mut T, u16)) -> Vec<T> {
    let mut slots = blank_pool::<T>(count);
    for (index, slot) in slots.iter_mut().enumerate() {
        let after = index + 1;
        let target = u16::try_from(after)
            .ok()
            .filter(|_| after < count as usize)
            .unwrap_or(end);
        set_next(slot, target);
    }
    slots
}

/// The pool index of the effect an effect handle names; None for a handle
/// that does not land on a slot boundary or is past the pool.
fn slot_of_handle(handle: u16) -> Option<usize> {
    // A handle counts words from the start of the pool.
    let byte_offset = usize::from(handle) * 4;
    if byte_offset % EMITTER_SLOT_BYTES != 0 {
        return None;
    }
    let slot = byte_offset / EMITTER_SLOT_BYTES;
    Some(slot).filter(|&n| n < EMITTER_BANK_LIMIT as usize)
}

/// Where ring entry `index` sits in the handle table: the ring wraps.
fn ring_cell(index: u32) -> usize {
    let wrapped = index.rem_euclid(EMITTER_TICKET_WHEEL_LEN);
    wrapped as usize
}

fn count_or_null(count: Option<u32>) -> String {
    count.map_or_else(|| "null".to_owned(), |n| format!("{n}"))
}

/// Leaves a reading in a note.
fn record<T>(note: &mut Option<T>, reading: T) {
    *note = Some(reading);
}

impl EffectSystemHost {
    pub fn new() -> Self {
        let elem_pool = chained_pool::<ParticleBinSlot>(
            PARTICLE_BANK_LIMIT,
            NO_PARTICLE_TICKET,
            |slot, next| slot.following_particle_ticket = next,
        );
        let trail_pool = chained_pool::<RibbonCell>(
            STREAK_BANK_LIMIT,
            NO_STREAK_TICKET,
            |slot, next| slot.next_trail_handle = next,
        );
        let trail_elem_pool = chained_pool::<RibbonPointCell>(
            STREAK_JOINT_BANK_LIMIT,
            NO_STREAK_TICKET,
            |slot, next| slot.next_trail_elem_handle = next,
        );
        let ring = (0..EMITTER_BANK_LIMIT)
            .map(effect_handle_for_slot)
            .collect();
        let bolt_links = (0..ATTACH_RECORD_LIMIT as i32)
            .map(bolt_init_next_index)
            .collect();
        let whole_pool = Some(0);

        Self {
            cell_list: blank_pool(EMITTER_BANK_LIMIT),
            ring,
            sort_distances: vec![(0, 0.0); elem_pool.len()],
            elems: elem_pool,
            trails: trail_pool,
            trail_elems: trail_elem_pool,
            bolt_links,
            first_spare_particle: whole_pool,
            ribbon_free_head: whole_pool,
            ribbon_head_free: whole_pool,
            marks: DecalOwner::init(),
            vis_blocker_write: Default::default(),
            vis_blocker_read: Default::default(),
            gaps: Default::default(),
            glass: Default::default(),
            stain_receivers: Default::default(),
            first_active_effect: Default::default(),
            first_new_effect: Default::default(),
            first_free_effect: Default::default(),
            spotlight_count: Default::default(),
            bolt_first_free: Default::default(),
            gc_pending: Default::default(),
            sort_epoch: Default::default(),
            live_parts: Default::default(),
            part_take_failures: Default::default(),
            clock_ms_now: Default::default(),
            frame_tick_no: Default::default(),
            walker_total: Default::default(),
            trace_serial: Default::default(),
            mark_traces: Default::default(),
            pending_runners: Default::default(),
            pending_sounds: Default::default(),
            pending_decals: Default::default(),
            pending_trail_impacts: Default::default(),
            stain_target_entity: Default::default(),
            last_stain_entity: Default::default(),
            camera_origin: Default::default(),
            last_emitter_source: Default::default(),
            last_emitter_part: Default::default(),
            last_emitter_child: Default::default(),
            last_emitter_ms: Default::default(),
            last_emitter_turn_deg: Default::default(),
            last_stain_source: Default::default(),
            last_stain_part: Default::default(),
            last_stain_ms: Default::default(),
            prior_decal_seen: Default::default(),
            last_stain_size0: Default::default(),
            last_stain_rotation: Default::default(),
            prior_decal_first_border: Default::default(),
            prior_decal_second_border: Default::default(),
            last_stain_mat0: Default::default(),
            prior_decal_second_surface: Default::default(),
            last_stain_origin: Default::default(),
            last_stain_axis: Default::default(),
            last_stain_tether: Default::default(),
            last_stain_on_world: Default::default(),
            last_stain_on_models: Default::default(),
            last_stain_color: Default::default(),
            stain_surfs: Default::default(),
            stain_verts: Default::default(),
            stain_indices: Default::default(),
            visual_stain_compact_tally: Default::default(),
            stain_draws: Default::default(),
            stain_draw_tris: Default::default(),
            stain_draw_skip: Default::default(),
            last_stain_lightmap: Default::default(),
            last_stain_light: Default::default(),
            last_stain_probe: Default::default(),
            stain_no_lightmap_count: Default::default(),
            stain_lightmap_page_count: Default::default(),
            last_stain_mesh: Default::default(),
            last_stain_alloc_slot: Default::default(),
        }
    }

    // -- looking effects up ------------------------------------------------

    pub fn running_emitters(&self) -> impl Iterator<Item = &LiveEffect> {
        self.cell_list
            .iter()
            .filter(|slot| slot.in_ring && slot.holds_refs())
    }

    pub fn live_elems(&self) -> impl Iterator<Item = &ParticleBinSlot> {
        self.elems.iter().filter(|slot| slot.is_taken)
    }

    pub fn bin_of_ticket(&self, named: u16) -> Option<&LiveEffect> {
        self.ring_index_of(named)
            .and_then(|index| self.cell_list.get(index))
    }

    pub fn bin_of_ticket_editable(&mut self, named: u16) -> Option<&mut LiveEffect> {
        let index = self.ring_index_of(named)?;
        self.cell_list.get_mut(index)
    }

    /// The pool index of the effect `named` names, if one lives there now.
    pub(crate) fn ring_index_of(&self, named: u16) -> Option<usize> {
        let index = slot_of_handle(named)?;
        match self.cell_list.get(index) {
            Some(slot) if slot.in_ring => Some(index),
            _ => None,
        }
    }

    pub fn emitter_in_slot(&self, k: usize) -> Option<&LiveEffect> {
        self.cell_list.get(k)
    }

    pub(crate) fn live_effect_mut(&mut self, index: usize) -> Option<&mut LiveEffect> {
        self.cell_list.get_mut(index)
    }

    // -- the handle ring ---------------------------------------------------

    pub(crate) fn ring_entry_handle(&self, entry: u32) -> u16 {
        self.ring[ring_cell(entry)]
    }

    fn store_ring_entry(&mut self, entry: u32, named: u16) {
        let cell = ring_cell(entry);
        self.ring[cell] = named;
    }

    /// How many ring entries are in use.
    pub fn ring_fill(&self) -> i32 {
        let (newest, oldest) = (self.first_free_effect, self.first_active_effect);
        newest.wrapping_sub(oldest)
    }

    /// Asks for the next collection to look for effects left with no claims.
    fn request_collection(&mut self) {
        self.gc_pending = true;
    }

    /// Takes the bolt record at the head of the free list, if any is left.
    fn take_tether_record(&mut self) -> Option<u8> {
        let head = self.bolt_first_free;
        let after_head = usize::try_from(head)
            .ok()
            .and_then(|record| self.bolt_links.get(record).copied());
        let (taken, rest) = bolt_alloc(head, after_head.unwrap_or(NO_SPARE_ATTACH))?;
        self.bolt_first_free = rest;
        Some(taken as u8)
    }

    /// Reserves the next free ring entry and says where it was.
    fn take_ring_entry(&mut self) -> Option<i32> {
        let full = self.ring_fill() >= EMITTER_TICKET_WHEEL_LEN as i32;
        if full {
            return None;
        }
        let claimed = self.first_free_effect;
        self.first_free_effect = claimed.wrapping_add(1);
        Some(claimed)
    }

    /// Whether another spotlight effect may start.
    fn spotlight_allowed(&self, wanted: bool) -> Result<(), SpawnRefusal> {
        let at_limit = self.spotlight_count >= CONE_GLOW_LIMIT;
        if wanted && at_limit {
            return Err(SpawnRefusal::SpotlightCap);
        }
        Ok(())
    }

    /// bo2zm: puts a bolt record back at the head of the free list.
    fn free_bolt_record(&mut self, record: u8) {
        if let Some(next) = self.bolt_links.get_mut(usize::from(record)) {
            *next = self.bolt_first_free;
            self.bolt_first_free = i32::from(record);
        }
    }

    /// Starts an effect in the next free ring entry and returns its handle.
    #[allow(clippy::too_many_arguments)]
    pub fn start_effect(
        &mut self,
        definition_name: &str,
        at: [f32; 3],
        facing: [[f32; 3]; 3],
        now: i32,
        bolt: u32,
        spotlight: bool,
        catalog: u16,
    ) -> Result<u16, SpawnRefusal> {
        self.spotlight_allowed(spotlight)?;
        let bolt_record = if bolt == BIRTH_UNATTACHED {
            BOLT_NONE
        } else {
            self.take_tether_record().ok_or(SpawnRefusal::PoolFull)?
        };
        let entry = self.take_ring_entry().ok_or(SpawnRefusal::RingFull)?;

        let handle = self.ring_entry_handle(entry as u32);
        let index = slot_of_handle(handle).expect("the handle ring only holds valid slot handles");
        self.cell_list[index] = LiveEffect::begin(EffectStart {
            name: definition_name,
            catalog,
            handle,
            msec: now,
            bolt_record,
            owner: self.stain_target_entity,
            spot: at,
            turn_rows: facing,
        });

        self.first_new_effect = self.first_new_effect.max(entry.wrapping_add(1));
        self.spotlight_count = self.spotlight_count.saturating_add(spotlight as i32);
        Ok(handle)
    }

    /// An effect lost a claim; the next collection looks for effects left with none.
    pub fn drop_effect_ref(&mut self, _ticket: u16) {
        self.request_collection();
    }

    /// The player lets go of its claim on an effect.
    pub fn release_play_ownership(&mut self, named: u16) {
        let released_last = self.bin_of_ticket_editable(named).map(|fx| {
            let last = status_is_unique_done(fx.state_bits);
            fx.state_bits = fx.state_bits.wrapping_sub(1);
            last
        });
        if released_last == Some(true) {
            self.drop_effect_ref(named);
        }
    }

    /// Reclaims every effect with no claims left: its elements and trails
    /// are freed, its slot is reset, and the live handles close ranks in the
    /// ring behind the freed ones.
    pub fn collect_garbage(&mut self) {
        if !self.gc_pending {
            return;
        }
        let oldest = self.first_active_effect;
        let span = self.first_new_effect.wrapping_sub(oldest) as u32;

        // The window of handles in the ring, oldest first, and whether each
        // still has a claim on a resident effect.
        let window: Vec<(u16, bool)> = (0..span)
            .map(|age| {
                let named = self.ring_entry_handle(oldest.wrapping_add(age as i32) as u32);
                let claimed = self
                    .ring_index_of(named)
                    .is_some_and(|index| self.cell_list[index].holds_refs());
                (named, claimed)
            })
            .collect();

        // Free what the unclaimed ones show, newest first.
        for &(named, _) in window.iter().rev().filter(|(_, claimed)| !claimed) {
            if let Some(index) = self.ring_index_of(named) {
                free_all_elems_for_effect(self, index);
                free_all_trails_for_effect(self, index);
            }
        }

        // The freed handles go first, then the kept ones, each in age order.
        let (kept, gone): (Vec<_>, Vec<_>) = window.iter().partition(|(_, claimed)| *claimed);
        for (age, &&(named, _)) in (0u32..).zip(gone.iter().chain(kept.iter())) {
            self.store_ring_entry(oldest.wrapping_add(age as i32) as u32, named);
        }
        for &&(named, _) in &gone {
            if let Some(index) = slot_of_handle(named) {
                // bo2zm: hand its bolt record back; kept, every bolted effect (muzzle flashes,
                // Mob's ghost wrists) used one up for good and after 255 none could spawn.
                let bolt = self.cell_list[index].tether;
                if bolt != BOLT_NONE {
                    self.free_bolt_record(bolt);
                }
                self.cell_list[index] = LiveEffect::default();
            }
        }
        self.first_active_effect = oldest.wrapping_add(gone.len() as i32);
        self.gc_pending = bool::default();
    }

    // -- stopping effects --------------------------------------------------

    /// Pool indices of the resident effects `wanted` accepts.
    fn slots_where(&self, wanted: impl Fn(&LiveEffect) -> bool) -> Vec<usize> {
        self.cell_list
            .iter()
            .enumerate()
            .filter_map(|(k, slot)| (slot.in_ring && wanted(slot)).then_some(k))
            .collect()
    }

    fn slots_bolted_to(&self, definition_name: &str, dobj: u32, bone: u16) -> Vec<usize> {
        let wanted_bolt = (dobj, u32::from(bone));
        self.slots_where(|slot| {
            slot.definition_name == definition_name
                && (bolt_dobj(slot.tether_bits), bolt_bone(slot.tether_bits)) == wanted_bolt
        })
    }

    /// Frees what the effects show and every claim on them, then collects them.
    fn end_effects_at_once(&mut self, slots: &[usize]) {
        for &index in slots {
            free_all_elems_for_effect(self, index);
            free_all_trails_for_effect(self, index);
            if let Some(effect) = self.live_effect_mut(index) {
                effect.state_bits &= !ALL_CLAIMS;
            }
        }
        let [_, ..] = slots else {
            return;
        };
        self.request_collection();
        self.collect_garbage();
    }

    pub fn stop_bolted(&mut self, definition_name: &str, dobj: u32, bone: u16) {
        for index in self.slots_bolted_to(definition_name, dobj, bone) {
            halt_effect_alone(self, index);
        }
    }

    pub fn stop_owned(&mut self, fx_ref: u16) {
        let Some(index) = self.ring_index_of(fx_ref) else {
            return;
        };
        halt_effect_alone(self, index);
        self.release_play_ownership(fx_ref);
    }

    /// Like `stop_bolted`, but whatever the effects still show goes at once as
    /// well, including a one-shot whose spawning is already done. Returns how
    /// many effects went.
    pub fn kill_bolted(&mut self, definition_name: &str, dobj: u32, bone: u16) -> usize {
        let victims = self.slots_bolted_to(definition_name, dobj, bone);
        self.end_effects_at_once(&victims);
        victims.len()
    }

    /// Ends every effect of `definition_name` that began at or after `window_open_ms`.
    pub fn kill_def_newer_than(&mut self, definition_name: &str, window_open_ms: i32) {
        let victims = self.slots_where(|slot| {
            slot.holds_refs() && slot.window_open_ms >= window_open_ms && slot.definition_name == definition_name
        });
        self.end_effects_at_once(&victims);
    }

    /// Asks `lookup` for the current pose of each bolted effect's bone.
    pub fn redo_attach_frames(
        &mut self,
        mut lookup: impl FnMut(u32, u16) -> Option<SettledAttachFrame>,
    ) {
        for effect in &mut self.cell_list {
            let packed = effect.tether_bits;
            if !effect.in_ring || bolt_handle_is_none(packed) {
                continue;
            }
            let pose = lookup(bolt_dobj(packed), bolt_bone(packed) as u16);
            effect.bolt_teleported = pose.map_or(false, |p| p.client_actor_teleport);
            effect.tether_pose = pose
                .and_then(|p| p.orientation)
                .map(|found| (found.origin, found.axis));
        }
    }

    // -- the clock ---------------------------------------------------------

    pub fn store_effect_clock(&mut self, now: i32) {
        self.marks.update_grid(now);
        self.clock_ms_now = now;
    }

    pub fn bump_tick_marker(&mut self) {
        let (next, _wrapped) = self.frame_tick_no.overflowing_add(1);
        self.frame_tick_no = next;
    }

    // -- marks and decals --------------------------------------------------

    /// Asks the mark host whether the decal noted in `last_decal_*` may
    /// leave a mark, and records which receivers it would land on.
    pub fn log_stain_from_decal(&mut self, bolt: u8) -> MarkOutcome {
        let (pos, receivers) = (self.last_stain_origin, self.stain_receivers);
        let verdict = self.marks.hit_decal(DecalContactAsk {
            anchor: pos,
            receivers,
            camera: self.camera_origin,
            no_world: marks_t6::impact_mark_skip_world_from_stored_bolt(bolt),
        });
        self.last_stain_on_world = verdict.went_inside.then_some(verdict.against_level);
        self.last_stain_on_models = verdict.went_inside.then_some(verdict.versus_models);
        verdict
    }

    /// Logs a gap: definition `def` asked for fragment marks, which are not drawn.
    pub fn note_fragments_skipped(&mut self, def: u8) {
        let cause = EffectHoleReason::FragmentsUnsupported { definition_slot: def };
        self.gaps.log_hole(cause);
    }

    /// The mark host keeps its own counts, so a successful world generate
    /// leaves nothing more to note.
    pub fn noted_world_fire(&mut self) {}

    /// As `noted_world_fire`.
    pub fn noted_models_fire(&mut self) {}

    pub fn count_stain_outer_skip(&mut self, def: u8) {
        let cause = EffectHoleReason::DecalNeverStarted { definition_slot: def };
        self.gaps.log_hole(cause);
    }

    /// Rebuilds the world-mark mesh, empty while the world receivers are off.
    pub fn generate_level_stain_vertices(&mut self) {
        let mesh = if self.stain_receivers.stains {
            self.marks.generate_level_stain_vertices()
        } else {
            DecalMeshTally::default()
        };
        // A default mesh counts zero everywhere, so the switch needs no second look.
        let packed_len = mesh.vert_blob.len() as u32;
        record(&mut self.stain_surfs, mesh.budget.facet_sum);
        record(&mut self.stain_verts, mesh.budget.vertex_amount);
        record(&mut self.stain_indices, mesh.budget.index_tally);
        record(&mut self.visual_stain_compact_tally, packed_len);
        record(&mut self.last_stain_mesh, mesh);
    }

    /// Why the mark gate turned an attempt away before it was entered.
    fn gate_refusal(&self) -> &'static str {
        let closed_by = [
            (self.marks.skip_decals, "no_marks"),
            (!self.stain_receivers.stains, "fx_marks=0"),
        ];
        closed_by
            .into_iter()
            .find_map(|(closed, why)| closed.then_some(why))
            .unwrap_or("outer_gate")
    }

    fn trace_skip_reason(
        &self,
        verdict: &MarkOutcome,
        allocated: bool,
    ) -> Option<&'static str> {
        if !verdict.went_inside {
            return Some(self.gate_refusal());
        }
        (!allocated).then_some("no_alloc")
    }

    /// Files a trace of the latest decal attempt: what was asked, what the
    /// mark host said, and the mark that came of it.
    pub fn log_stain_trace(&mut self, verdict: &MarkOutcome) {
        self.trace_serial = self.trace_serial.checked_add(1).unwrap_or(1).max(1);
        let slot = self.last_stain_alloc_slot.take();

        // What became of the mark, if one was made.
        let made = slot.and_then(|named| self.marks.built_record(named).copied());
        let bound = slot
            .and_then(|named| self.marks.surface_title(named))
            .map(str::to_owned);
        // The decal's up axis stands in for its surface normal.
        let normal = self.last_stain_axis.map(|axis| axis[2]);

        // The decal the attempt was for, as last noted.
        let (parent, elem, msec) = (
            self.last_stain_source.clone(),
            self.last_stain_part,
            self.last_stain_ms,
        );
        let (vis, origin, start_extent) = (self.prior_decal_seen, self.last_stain_origin, self.last_stain_size0);
        let (mat0, mat1) = (self.last_stain_mat0.clone(), self.prior_decal_second_surface.clone());
        let (mat0_edge, mat1_edge) = (
            self.prior_decal_first_border.clone(),
            self.prior_decal_second_border.clone(),
        );
        let color = self.last_stain_color;
        let tint_category = color.map_or("none", |_| "sampled");

        let trace = DecalRayLog {
            id: self.trace_serial,
            bypass_cause: self.trace_skip_reason(verdict, slot.is_some()),
            went_in: verdict.went_inside,
            against_level: verdict.against_level,
            versus_models: verdict.versus_models,
            cell: slot,
            bound_to: bound,
            triangle_tally: made.map(|mark| mark.tri_tally),
            point_tally: made.map(|mark| mark.vertex_total_in_mark),
            authored_tint: made.map(|mark| mark.authored_tint),
            context: made.map(|mark| mark.context),
            bolt_tag: self.last_stain_tether.unwrap_or(BOLT_NONE),
            source: parent,
            part: elem,
            ms_now: msec,
            seen: vis,
            spot: origin,
            start_extent,
            tint: color,
            tint_category,
            tex_a: mat0,
            tex_b: mat1,
            tex_a_edge: mat0_edge,
            tex_b_edge: mat1_edge,
            facing_x: normal.map(|n| n[0]),
            facing_y: normal.map(|n| n[1]),
            facing_z: normal.map(|n| n[2]),
        };

        // Make room for the newcomer by dropping the oldest.
        let surplus = self.mark_traces.len().saturating_sub(MARK_TRACE_LIMIT - 1);
        self.mark_traces.drain(..surplus);
        self.mark_traces.push(trace);
    }

    /// One line of mark pool numbers for the console.
    pub fn tag_timing_row(&self) -> String {
        let live = self.marks.active_total();
        let draw_gate = match self.stain_receivers.stains {
            true => "on",
            false => "fx_marks=0",
        };
        let readings = [
            ("live", format!("{live}/{}", marks_t6::DECAL_POOL_CAPACITY)),
            ("alloced", self.marks.taken_count().to_string()),
            ("tri_free", self.marks.tri_free_head().to_string()),
            ("point_free", self.marks.point_free_head().to_string()),
            ("gfx_surf", count_or_null(self.stain_surfs)),
            ("gfx_draw", count_or_null(self.stain_draws)),
            ("unlinked", live.to_string()),
            ("ents_AddEntity", String::from("typed_gap")),
            ("draw_gate", draw_gate.to_string()),
        ];
        let mut line = String::from("fx_mark_profile");
        for (label, reading) in readings {
            line.push(' ');
            line.push_str(label);
            line.push('=');
            line.push_str(&reading);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_t6::runtime::bolt_pack;

    const A: [f32; 3] = [1.0, 2.0, 3.0];

    fn start(host: &mut EffectSystemHost, name: &str, msec: i32) -> u16 {
        host.start_effect(name, A, UNTURNED_BASIS, msec, BIRTH_UNATTACHED, false, 7)
            .expect("room for one more effect")
    }

    fn hit(entered: bool, hits_world: bool, models: bool) -> MarkOutcome {
        MarkOutcome {
            went_inside: entered,
            against_level: hits_world,
            versus_models: models,
        }
    }

    fn bolted(host: &mut EffectSystemHost, name: &str, dobj: u32, bone: u32) -> u16 {
        let handle = host
            .start_effect(name, A, UNTURNED_BASIS, 100, 5, false, 0)
            .unwrap();
        host.bin_of_ticket_editable(handle).unwrap().tether_bits = bolt_pack(dobj, false, bone);
        handle
    }

    #[test]
    fn pools_start_strung_together() {
        let host = EffectSystemHost::new();
        assert_eq!(host.elems.len(), PARTICLE_BANK_LIMIT as usize);
        assert_eq!(host.elems[0].following_particle_ticket, 1);
        assert_eq!(host.elems[5].following_particle_ticket, 6);
        let last_elem = host.elems.last().unwrap().following_particle_ticket;
        assert_eq!(last_elem, NO_PARTICLE_TICKET);
        assert_eq!(host.sort_distances.len(), host.elems.len());
        assert_eq!(host.trails.len(), STREAK_BANK_LIMIT as usize);
        assert_eq!(host.trails[2].next_trail_handle, 3);
        let last_trail = host.trails.last().unwrap().next_trail_handle;
        let last_trail_elem = host.trail_elems.last().unwrap().next_trail_elem_handle;
        assert_eq!([last_trail, last_trail_elem], [NO_STREAK_TICKET; 2]);
        assert_eq!(host.trail_elems[9].next_trail_elem_handle, 10);
        assert_eq!(host.first_spare_particle, Some(0));
        assert_eq!(host.bolt_first_free, 0);
        assert_eq!(host.bolt_links[0], 1);
        assert_eq!(host.bolt_links[0xfe], NO_SPARE_ATTACH);
        assert_eq!(host.ring_fill(), 0);
        assert_eq!(host.running_emitters().count(), 0);
        for ring in 0..4u32 {
            assert_eq!(host.ring_entry_handle(ring), effect_handle_for_slot(ring));
        }
    }

    #[test]
    fn handles_land_only_on_slot_boundaries() {
        assert_eq!(slot_of_handle(0), Some(0));
        assert_eq!(slot_of_handle(effect_handle_for_slot(1)), Some(1));
        assert_eq!(slot_of_handle(effect_handle_for_slot(1023)), Some(1023));
        assert_eq!(slot_of_handle(1), None);
        assert_eq!(slot_of_handle(effect_handle_for_slot(1) + 1), None);
        assert_eq!(slot_of_handle(effect_handle_for_slot(1024)), None);
        assert_eq!(slot_of_handle(u16::MAX), None);
    }

    #[test]
    fn counts_print_as_numbers_or_null() {
        assert_eq!(count_or_null(None), "null");
        assert_eq!(count_or_null(Some(0)), "0");
        assert_eq!(count_or_null(Some(41)), "41");
    }

    #[test]
    fn a_new_effect_is_filled_in_and_found_by_handle() {
        let mut host = EffectSystemHost::new();
        host.stain_target_entity = Some(9);
        let handle = start(&mut host, "fx/a", 1234);
        assert_eq!(handle, effect_handle_for_slot(0));
        assert_eq!(host.ring_fill(), 1);
        assert_eq!(host.first_new_effect, 1);
        let effect = host.bin_of_ticket(handle).unwrap();
        assert_eq!(effect.definition_name, "fx/a");
        assert_eq!(effect.ledger_index, 7);
        assert_eq!(effect.state_bits, NEW_EFFECT_STATUS);
        assert_eq!(effect.self_ticket, handle);
        assert_eq!(effect.stain_owner, Some(9));
        assert_eq!(effect.window_open_ms, 1234);
        assert_eq!(effect.ms_last_tick, 1234);
        assert_eq!(effect.dice_start, effect_random_seed_from_msec(1234));
        assert_eq!(effect.tether, BOLT_NONE);
        assert_eq!(effect.where_at, A);
        assert_eq!(effect.spot_at_launch, A);
        assert_eq!(effect.spot_before, A);
        assert!(effect.in_ring && effect.holds_refs());
        assert_eq!(effect.frame_tick_no, -1);
        assert_eq!(host.running_emitters().count(), 1);
        assert!(host.bin_of_ticket(effect_handle_for_slot(1)).is_none());
    }

    #[test]
    fn spawning_hands_out_ring_entries_in_order() {
        let mut host = EffectSystemHost::new();
        let first = start(&mut host, "a", 0);
        let second = start(&mut host, "b", 0);
        assert_eq!(second, effect_handle_for_slot(1));
        assert_eq!(host.bin_of_ticket(first).unwrap().definition_name, "a");
        assert_eq!(host.bin_of_ticket(second).unwrap().definition_name, "b");
        assert_eq!(host.first_free_effect, 2);
        assert_eq!(host.first_new_effect, 2);
    }

    #[test]
    fn only_one_spotlight_at_a_time() {
        let mut host = EffectSystemHost::new();
        let lit = host.start_effect("a", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, true, 0);
        assert!(lit.is_ok());
        assert_eq!(host.spotlight_count, 1);
        let second = host.start_effect("b", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, true, 0);
        assert_eq!(second, Err(SpawnRefusal::SpotlightCap));
        assert_eq!(host.ring_fill(), 1);
        assert!(
            host.start_effect("c", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, false, 0)
                .is_ok()
        );
    }

    #[test]
    fn the_ring_fills_up() {
        let mut host = EffectSystemHost::new();
        for _ in 0..EMITTER_TICKET_WHEEL_LEN {
            start(&mut host, "x", 0);
        }
        assert_eq!(host.ring_fill(), EMITTER_TICKET_WHEEL_LEN as i32);
        let over = host.start_effect("x", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, false, 0);
        assert_eq!(over, Err(SpawnRefusal::RingFull));
        assert_eq!(host.ring_fill(), EMITTER_TICKET_WHEEL_LEN as i32);
    }

    #[test]
    fn bolted_effects_take_bolt_records_until_they_run_out() {
        let mut host = EffectSystemHost::new();
        let handle = host
            .start_effect("a", A, UNTURNED_BASIS, 0, 5, false, 0)
            .unwrap();
        assert_eq!(host.bin_of_ticket(handle).unwrap().tether, 0);
        assert_eq!(host.bolt_first_free, 1);
        let second = host
            .start_effect("b", A, UNTURNED_BASIS, 0, 5, false, 0)
            .unwrap();
        assert_eq!(host.bin_of_ticket(second).unwrap().tether, 1);
        for _ in 2..ATTACH_RECORD_LIMIT {
            host.start_effect("c", A, UNTURNED_BASIS, 0, 5, false, 0)
                .unwrap();
        }
        assert_eq!(host.bolt_first_free, NO_SPARE_ATTACH);
        let none_left = host.start_effect("d", A, UNTURNED_BASIS, 0, 5, false, 0);
        assert_eq!(none_left, Err(SpawnRefusal::PoolFull));
        assert!(
            host.start_effect("e", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, false, 0)
                .is_ok()
        );
    }

    #[test]
    fn releasing_the_last_claim_asks_for_a_collection() {
        let mut host = EffectSystemHost::new();
        let handle = start(&mut host, "a", 0);
        host.release_play_ownership(handle);
        assert!(host.gc_pending);
        assert!(!host.bin_of_ticket(handle).unwrap().holds_refs());

        let mut host = EffectSystemHost::new();
        let handle = start(&mut host, "a", 0);
        host.bin_of_ticket_editable(handle).unwrap().state_bits += 1;
        host.release_play_ownership(handle);
        assert!(!host.gc_pending);
        assert_eq!(host.bin_of_ticket(handle).unwrap().state_bits & 0x1fff, 1);

        host.release_play_ownership(effect_handle_for_slot(500));
        assert!(!host.gc_pending);
    }

    #[test]
    fn collection_does_nothing_unless_asked() {
        let mut host = EffectSystemHost::new();
        let handle = start(&mut host, "a", 0);
        host.bin_of_ticket_editable(handle).unwrap().state_bits = NEW_EFFECT_STATUS - 1;
        host.collect_garbage();
        assert!(host.bin_of_ticket(handle).is_some());
        assert_eq!(host.first_active_effect, 0);
    }

    #[test]
    fn collection_closes_ranks_in_the_ring() {
        let mut host = EffectSystemHost::new();
        let handles: Vec<u16> = (0..4).map(|n| start(&mut host, "e", n)).collect();
        host.release_play_ownership(handles[1]);
        host.release_play_ownership(handles[3]);
        host.collect_garbage();
        assert!(!host.gc_pending);
        assert_eq!(host.first_active_effect, 2);
        assert_eq!(host.first_new_effect, 4);
        let ring: Vec<u16> = (0..4).map(|i| host.ring_entry_handle(i)).collect();
        assert_eq!(ring, vec![handles[1], handles[3], handles[0], handles[2]]);
        assert_eq!(host.running_emitters().count(), 2);
        assert!(host.bin_of_ticket(handles[1]).is_none());
        assert!(host.bin_of_ticket(handles[3]).is_none());
        assert_eq!(host.bin_of_ticket(handles[0]).unwrap().window_open_ms, 0);
        assert_eq!(host.bin_of_ticket(handles[2]).unwrap().window_open_ms, 2);
        let reset = slot_of_handle(handles[1]).unwrap();
        assert_eq!(host.emitter_in_slot(reset).unwrap().state_bits, 0);
        assert_eq!(host.ring_fill(), 2);
    }

    #[test]
    fn collection_keeps_ring_order_when_only_the_oldest_died() {
        let mut host = EffectSystemHost::new();
        let handles: Vec<u16> = (0..3).map(|n| start(&mut host, "e", n)).collect();
        host.release_play_ownership(handles[0]);
        host.collect_garbage();
        assert_eq!(host.first_active_effect, 1);
        let ring: Vec<u16> = (0..3).map(|i| host.ring_entry_handle(i)).collect();
        assert_eq!(ring, vec![handles[0], handles[1], handles[2]]);
    }

    #[test]
    fn collection_with_everything_dead_empties_the_ring() {
        let mut host = EffectSystemHost::new();
        let handles: Vec<u16> = (0..3).map(|n| start(&mut host, "e", n)).collect();
        for &h in &handles {
            host.release_play_ownership(h);
        }
        host.collect_garbage();
        assert_eq!(host.first_active_effect, 3);
        assert_eq!(host.ring_fill(), 0);
        assert_eq!(host.running_emitters().count(), 0);
    }

    #[test]
    fn kill_bolted_ends_matching_effects_at_once() {
        let mut host = EffectSystemHost::new();
        let hit_a = bolted(&mut host, "fx/x", 3, 2);
        let hit_b = bolted(&mut host, "fx/x", 3, 2);
        let other_bone = bolted(&mut host, "fx/x", 3, 4);
        let other_def = bolted(&mut host, "fx/y", 3, 2);
        assert_eq!(host.kill_bolted("fx/x", 3, 2), 2);
        assert!(host.bin_of_ticket(hit_a).is_none());
        assert!(host.bin_of_ticket(hit_b).is_none());
        assert!(host.bin_of_ticket(other_bone).is_some());
        assert!(host.bin_of_ticket(other_def).is_some());
        assert_eq!(host.running_emitters().count(), 2);
        assert_eq!(host.kill_bolted("fx/x", 3, 2), 0);
        assert!(!host.gc_pending);
    }

    #[test]
    fn stop_bolted_lets_the_effect_wind_down() {
        let mut host = EffectSystemHost::new();
        let target = bolted(&mut host, "fx/x", 3, 2);
        let other = bolted(&mut host, "fx/x", 4, 2);
        host.stop_bolted("fx/x", 3, 2);
        assert!(host.gc_pending);
        assert!(host.bin_of_ticket(target).is_some());
        assert!(host.bin_of_ticket(other).is_some());
    }

    #[test]
    fn stop_owned_gives_up_the_players_claim() {
        let mut host = EffectSystemHost::new();
        host.stop_owned(effect_handle_for_slot(3));
        assert!(!host.gc_pending);
        let handle = start(&mut host, "a", 0);
        host.stop_owned(handle);
        assert!(host.gc_pending);
        assert!(!host.bin_of_ticket(handle).unwrap().holds_refs());
    }

    #[test]
    fn kill_def_newer_than_spares_older_and_other_defs() {
        let mut host = EffectSystemHost::new();
        let old = start(&mut host, "fx/x", 100);
        let edge = start(&mut host, "fx/x", 200);
        let late = start(&mut host, "fx/x", 300);
        let other = start(&mut host, "fx/y", 300);
        host.kill_def_newer_than("fx/x", 200);
        assert!(host.bin_of_ticket(old).is_some());
        assert!(host.bin_of_ticket(edge).is_none());
        assert!(host.bin_of_ticket(late).is_none());
        assert!(host.bin_of_ticket(other).is_some());
        host.kill_def_newer_than("fx/none", 0);
        assert_eq!(host.running_emitters().count(), 2);
    }

    #[test]
    fn bolt_poses_come_from_the_resolver() {
        let mut host = EffectSystemHost::new();
        let bolt = bolted(&mut host, "fx/x", 6, 9);
        let free = start(&mut host, "fx/free", 0);
        let orientation = AttachPointFrame {
            origin: A,
            axis: UNTURNED_BASIS,
        };
        let mut asked = Vec::new();
        host.redo_attach_frames(|dobj, bone| {
            asked.push((dobj, bone));
            Some(SettledAttachFrame {
                client_actor_teleport: true,
                orientation: Some(orientation),
            })
        });
        assert_eq!(asked, vec![(6, 9)]);
        let effect = host.bin_of_ticket(bolt).unwrap();
        assert!(effect.bolt_teleported);
        assert_eq!(effect.tether_pose, Some((A, UNTURNED_BASIS)));
        assert!(host.bin_of_ticket(free).unwrap().tether_pose.is_none());

        host.redo_attach_frames(|_, _| None);
        let effect = host.bin_of_ticket(bolt).unwrap();
        assert!(!effect.bolt_teleported);
        assert_eq!(effect.tether_pose, None);

        host.redo_attach_frames(|_, _| {
            Some(SettledAttachFrame {
                client_actor_teleport: true,
                orientation: None,
            })
        });
        let effect = host.bin_of_ticket(bolt).unwrap();
        assert!(effect.bolt_teleported);
        assert_eq!(effect.tether_pose, None);
    }

    #[test]
    fn frames_follow_the_slot() {
        let mut host = EffectSystemHost::new();
        let handle = start(&mut host, "a", 0);
        let effect = host.bin_of_ticket_editable(handle).unwrap();
        effect.where_at = [9.0, 9.0, 9.0];
        effect.carry_frame_forward();
        assert_eq!(effect.spot_before, [9.0, 9.0, 9.0]);
        assert_eq!(effect.current_frame().origin, [9.0, 9.0, 9.0]);
        assert_eq!(effect.spawn_frame().origin, A);
    }

    #[test]
    fn the_clock_and_the_frame_stamp_move() {
        let mut host = EffectSystemHost::new();
        host.store_effect_clock(500);
        assert_eq!(host.clock_ms_now, 500);
        host.frame_tick_no = i32::MAX;
        host.bump_tick_marker();
        assert_eq!(host.frame_tick_no, i32::MIN);
    }

    #[test]
    fn the_world_mesh_note_follows_the_receiver_switch() {
        let mut host = EffectSystemHost::new();
        host.generate_level_stain_vertices();
        assert_eq!(host.stain_surfs, Some(0));
        assert_eq!(host.stain_verts, Some(0));
        assert_eq!(host.stain_indices, Some(0));
        assert_eq!(host.visual_stain_compact_tally, Some(0));
        assert!(host.last_stain_mesh.is_some());
        host.stain_receivers.stains = false;
        host.generate_level_stain_vertices();
        assert_eq!(host.stain_surfs, Some(0));
        assert_eq!(host.visual_stain_compact_tally, Some(0));
    }

    #[test]
    fn impact_marks_are_noted_only_when_entered() {
        let mut host = EffectSystemHost::new();
        let result = host.log_stain_from_decal(BOLT_NONE);
        assert_eq!(host.last_stain_on_world.is_some(), result.went_inside);
        assert_eq!(host.last_stain_on_models.is_some(), result.went_inside);
        host.stain_receivers.stains = false;
        let result = host.log_stain_from_decal(BOLT_NONE);
        assert!(!result.went_inside);
        assert_eq!(host.last_stain_on_world, None);
        assert_eq!(host.last_stain_on_models, None);
    }

    #[test]
    fn a_trace_says_why_a_mark_was_skipped() {
        let mut host = EffectSystemHost::new();
        assert_eq!(host.trace_skip_reason(&hit(true, true, false), true), None);
        let no_alloc = host.trace_skip_reason(&hit(true, true, false), false);
        assert_eq!(no_alloc, Some("no_alloc"));
        let gated = host.trace_skip_reason(&hit(false, false, false), true);
        assert_eq!(gated, Some("outer_gate"));
        host.stain_receivers.stains = false;
        let marks_off = host.trace_skip_reason(&hit(false, false, false), true);
        assert_eq!(marks_off, Some("fx_marks=0"));
        host.marks.skip_decals = true;
        let none_left = host.trace_skip_reason(&hit(false, false, false), true);
        assert_eq!(none_left, Some("no_marks"));
    }

    #[test]
    fn a_trace_records_the_latest_decal() {
        let mut host = EffectSystemHost::new();
        host.last_stain_source = Some("fx/p".to_owned());
        host.last_stain_part = Some(2);
        host.last_stain_ms = Some(77);
        host.last_stain_origin = Some(A);
        host.last_stain_size0 = Some(4.5);
        host.last_stain_color = Some(0x1122_3344);
        host.prior_decal_seen = Some(1);
        host.last_stain_mat0 = Some("m0".to_owned());
        host.prior_decal_second_border = Some("m1e".to_owned());
        host.last_stain_axis = Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.25, 0.125]]);
        host.last_stain_tether = Some(8);
        host.log_stain_trace(&hit(true, true, false));
        let trace = host.mark_traces.last().unwrap();
        assert_eq!(trace.id, 1);
        assert_eq!(trace.bolt_tag, 8);
        assert!(trace.went_in && trace.against_level && !trace.versus_models);
        assert_eq!(trace.source.as_deref(), Some("fx/p"));
        assert_eq!(trace.part, Some(2));
        assert_eq!(trace.ms_now, Some(77));
        assert_eq!(trace.spot, Some(A));
        assert_eq!(trace.start_extent, Some(4.5));
        assert_eq!(trace.tint, Some(0x1122_3344));
        assert_eq!(trace.tint_category, "sampled");
        assert_eq!(trace.tex_a.as_deref(), Some("m0"));
        assert_eq!(trace.tex_b, None);
        assert_eq!(trace.tex_b_edge.as_deref(), Some("m1e"));
        assert_eq!(trace.cell, None);
        assert_eq!(trace.bound_to, None);
        assert_eq!(trace.triangle_tally, None);
        assert_eq!(trace.bypass_cause, Some("no_alloc"));
        assert_eq!(trace.seen, Some(1));
        assert_eq!(
            (trace.facing_x, trace.facing_y, trace.facing_z),
            (Some(0.5), Some(0.25), Some(0.125))
        );
    }

    #[test]
    fn a_bare_trace_has_nothing_filled_in() {
        let mut host = EffectSystemHost::new();
        host.log_stain_trace(&hit(false, false, false));
        let trace = &host.mark_traces[0];
        assert_eq!(trace.bolt_tag, BOLT_NONE);
        assert_eq!(trace.tint_category, "none");
        assert_eq!(trace.bypass_cause, Some("outer_gate"));
        assert_eq!((trace.facing_x, trace.facing_y, trace.facing_z), (None, None, None));
    }

    #[test]
    fn trace_ids_count_up_and_skip_zero_on_wrap() {
        let mut host = EffectSystemHost::new();
        host.log_stain_trace(&hit(true, false, false));
        host.log_stain_trace(&hit(true, false, false));
        assert_eq!(host.mark_traces[1].id, 2);
        host.trace_serial = u32::MAX;
        host.log_stain_trace(&hit(true, false, false));
        assert_eq!(host.mark_traces[2].id, 1);
    }

    #[test]
    fn only_the_newest_traces_are_kept() {
        let mut host = EffectSystemHost::new();
        for _ in 0..MARK_TRACE_LIMIT + 3 {
            host.log_stain_trace(&hit(true, false, false));
        }
        assert_eq!(host.mark_traces.len(), MARK_TRACE_LIMIT);
        assert_eq!(host.mark_traces[0].id, 4);
        assert_eq!(
            host.mark_traces.last().unwrap().id,
            MARK_TRACE_LIMIT as u32 + 3
        );
    }

    #[test]
    fn the_profile_line_reads_the_mark_numbers() {
        let mut host = EffectSystemHost::new();
        let line = host.tag_timing_row();
        assert!(line.starts_with("fx_mark_profile live=0/"), "{line}");
        assert!(line.contains(" gfx_surf=null gfx_draw=null "), "{line}");
        assert!(line.ends_with("draw_gate=on"), "{line}");
        host.stain_surfs = Some(3);
        host.stain_draws = Some(2);
        host.stain_receivers.stains = false;
        let line = host.tag_timing_row();
        assert!(line.contains(" gfx_surf=3 gfx_draw=2 "), "{line}");
        assert!(line.ends_with("draw_gate=fx_marks=0"), "{line}");
    }

    #[test]
    fn a_default_host_is_a_new_host() {
        let (fresh, default) = (EffectSystemHost::new(), EffectSystemHost::default());
        assert_eq!(fresh.elems.len(), default.elems.len());
        assert_eq!(fresh.ring_entry_handle(7), default.ring_entry_handle(7));
        assert_eq!(fresh.bolt_first_free, default.bolt_first_free);
        assert_eq!(default.ribbon_free_head, Some(0));
        assert_eq!(default.ribbon_head_free, Some(0));
        assert!(default.camera_origin.is_none() && default.pending_runners.is_empty());
        assert!(default.last_stain_axis.is_none() && default.last_stain_mesh.is_none());
        assert!(default.mark_traces.is_empty() && !default.gc_pending);
    }

    #[test]
    fn a_blank_effect_slot_has_no_name_and_no_claims() {
        let blank = LiveEffect::default();
        assert_eq!(blank.ledger_index, NO_RECIPE_LISTING);
        assert_eq!(blank.lead_particle_ticket, [NO_PARTICLE_TICKET; 3]);
        assert_eq!(blank.first_part_ticket, NO_PARTICLE_TICKET);
        assert_eq!(blank.first_ribbon_ticket, NO_PARTICLE_TICKET);
        assert_eq!(blank.compact_illum, [0xff; 3]);
        assert_eq!(blank.packed_lighting_src, BakedShadeSource::White);
        assert_eq!(blank.tether, BOLT_NONE);
        assert_eq!(blank.tether_bits, NO_ATTACH_TICKET);
        assert_eq!(blank.bolt_parent_rot, ATTACH_ROOT_NO_TURN);
        assert_eq!(blank.frame_tick_no, -1);
        assert_eq!(blank.orient_rows, UNTURNED_BASIS);
        assert!(!blank.holds_refs() && !blank.in_ring && blank.tether_pose.is_none());
    }

    #[test]
    fn a_new_effect_starts_in_its_spawn_pose_with_an_unbolted_parent() {
        let mut host = EffectSystemHost::new();
        let axis = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let handle = host
            .start_effect("fx/a", A, axis, 40, BIRTH_UNATTACHED, false, 3)
            .unwrap();
        let effect = host.bin_of_ticket(handle).unwrap();
        assert_eq!(effect.frame_at_launch, axis);
        assert_eq!(effect.frame_before, axis);
        assert_eq!(effect.spawn_frame().axis, axis);
        assert_eq!(effect.current_frame().origin, A);
        let (quat, origin) = bolt_init_parent_orientation();
        assert_eq!(effect.bolt_parent_rot, quat);
        assert_eq!(effect.tether_spot, origin);
        assert_eq!(effect.tether_bits, NO_ATTACH_TICKET);
    }

    #[test]
    fn a_failed_spawn_leaves_the_spotlight_count_alone() {
        let mut host = EffectSystemHost::new();
        host.start_effect("a", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, true, 0)
            .unwrap();
        let _ = host.start_effect("b", A, UNTURNED_BASIS, 0, BIRTH_UNATTACHED, true, 0);
        assert_eq!(host.spotlight_count, 1);
        assert_eq!(host.first_free_effect, 1);
    }

    #[test]
    fn ring_entries_wrap_around_the_handle_table() {
        assert_eq!(ring_cell(0), 0);
        assert_eq!(ring_cell(EMITTER_TICKET_WHEEL_LEN - 1), 1023);
        assert_eq!(ring_cell(EMITTER_TICKET_WHEEL_LEN + 2), 2);
        let mut host = EffectSystemHost::new();
        host.store_ring_entry(EMITTER_TICKET_WHEEL_LEN + 5, 99);
        assert_eq!(host.ring_entry_handle(5), 99);
    }

    #[test]
    fn a_second_collection_works_from_where_the_first_stopped() {
        let mut host = EffectSystemHost::new();
        let first: Vec<u16> = (0..4).map(|n| start(&mut host, "e", n)).collect();
        host.release_play_ownership(first[0]);
        host.release_play_ownership(first[2]);
        host.collect_garbage();
        assert_eq!(host.first_active_effect, 2);
        let late = start(&mut host, "late", 50);
        assert_eq!(host.first_free_effect, 5);
        assert_eq!(host.ring_fill(), 3);
        host.release_play_ownership(first[3]);
        host.collect_garbage();
        assert_eq!(host.first_active_effect, 3);
        let ring: Vec<u16> = (3..5).map(|i| host.ring_entry_handle(i)).collect();
        assert_eq!(ring, vec![first[1], late]);
        assert_eq!(host.running_emitters().count(), 2);
    }

    #[test]
    fn collecting_frees_the_dead_effects_slots() {
        let mut host = EffectSystemHost::new();
        let a = start(&mut host, "a", 0);
        let b = start(&mut host, "b", 0);
        host.release_play_ownership(a);
        host.collect_garbage();
        let slot_a = slot_of_handle(a).unwrap();
        assert!(host.emitter_in_slot(slot_a).unwrap().definition_name.is_empty());
        assert!(!host.emitter_in_slot(slot_a).unwrap().in_ring);
        assert_eq!(host.bin_of_ticket(b).unwrap().definition_name, "b");
    }

    #[test]
    fn refreshing_poses_skips_free_slots_and_unbolted_effects() {
        let mut host = EffectSystemHost::new();
        let loose = start(&mut host, "loose", 0);
        let mut asked = 0;
        host.redo_attach_frames(|_, _| {
            asked += 1;
            None
        });
        assert_eq!(asked, 0);
        assert!(!host.bin_of_ticket(loose).unwrap().bolt_teleported);
    }

    #[test]
    fn a_stale_handle_names_nothing() {
        let mut host = EffectSystemHost::new();
        let handle = start(&mut host, "a", 0);
        host.release_play_ownership(handle);
        host.collect_garbage();
        assert!(host.bin_of_ticket(handle).is_none());
        assert!(host.bin_of_ticket_editable(handle).is_none());
        assert!(host.ring_index_of(handle).is_none());
        assert!(host.emitter_in_slot(slot_of_handle(handle).unwrap()).is_some());
        assert!(host.emitter_in_slot(EMITTER_BANK_LIMIT as usize).is_none());
    }

    #[test]
    fn skipped_decals_raise_their_gaps() {
        use crate::holes::EffectHole;
        let mut host = EffectSystemHost::new();
        host.note_fragments_skipped(4);
        host.note_fragments_skipped(4);
        host.noted_world_fire();
        host.noted_models_fire();
        assert_eq!(host.gaps.hit_total(EffectHole::FragmentMarks), 2);
        assert_eq!(host.gaps.hit_total(EffectHole::DecalChild), 0);
        host.count_stain_outer_skip(4);
        assert_eq!(host.gaps.hit_total(EffectHole::DecalChild), 1);
    }

    #[test]
    fn the_profile_line_lists_every_number_in_order() {
        let mut host = EffectSystemHost::new();
        host.stain_surfs = Some(5);
        let line = host.tag_timing_row();
        let live = host.marks.active_total();
        let want = format!(
            "fx_mark_profile live={live}/{} alloced={} tri_free={} point_free={} gfx_surf=5 \
             gfx_draw=null unlinked={live} ents_AddEntity=typed_gap draw_gate=on",
            marks_t6::DECAL_POOL_CAPACITY,
            host.marks.taken_count(),
            host.marks.tri_free_head(),
            host.marks.point_free_head(),
        );
        assert_eq!(line, want);
    }

    #[test]
    fn a_trace_keeps_the_receivers_it_was_given() {
        let mut host = EffectSystemHost::new();
        host.log_stain_trace(&hit(true, false, true));
        let trace = host.mark_traces.last().unwrap();
        assert!(trace.went_in && !trace.against_level && trace.versus_models);
        assert_eq!(trace.bypass_cause, Some("no_alloc"));
        assert_eq!(trace.authored_tint, None);
        assert_eq!(trace.context, None);
        assert_eq!(trace.point_tally, None);
        host.last_stain_alloc_slot = Some(3);
        host.log_stain_trace(&hit(true, true, true));
        assert_eq!(host.mark_traces.last().unwrap().cell, Some(3));
        assert_eq!(host.last_stain_alloc_slot, None);
    }
}

#[cfg(test)]
mod bolt_record_tests {
    use super::*;

    #[test]
    fn dead_bolted_effects_hand_their_bolt_record_back() {
        let mut host = EffectSystemHost::new();
        let axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for i in 0..1000 {
            let handle = host
                .start_effect("wrist", [0.0; 3], axis, i, 0x800, false, 0)
                .unwrap_or_else(|e| panic!("bolted spawn {i} failed: {e:?}"));
            host.release_play_ownership(handle);
            if let Some(effect) = host.bin_of_ticket_editable(handle) {
                effect.state_bits &= !FX_STATUS_REF_COUNT_LOW_MASK;
            }
            host.request_collection();
            host.collect_garbage();
        }
    }
}
