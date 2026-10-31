//! One draw plan for every animated or script-placed model. Eight producers
//! (remote bodies, the viewmodel, script models, missiles, items, fx models,
//! dynamic entities, the sky model) each build their own geometry and draws;
//! the merge here puts them one after another in fixed slot order and, from
//! the revisions each producer publishes, redoes only what moved.

use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasher, BuildHasherDefault};
use std::sync::Arc;

use asset_world::PackedVertexPayload;
use bevy::prelude::*;
use render_anim::geometry::install_held_compact;
use render_frame::{CompactSegment, CompactSegments, InputChangeCounters};

use super::sky::SkyDomeDrawPlan;
use super::prop_mesh::{PackedVertexRefusal, PropPassMaterial, PropVertex};
use crate::assemble::draw_items::mesh_build::{PORTION_BANKS, PortionBanks};

pub use render_anim::{
    BODY_VERTS_DROPPED_NOTE, PropMeshPaint, DynEntPaintPlan, DebrisOwnerDraw, FPV_PACKED_BLANK_PLAN,
    FPV_PACKED_MISSING, FirstPersonPaintPlan, GunSurfaceBatch, PickupMeshPaint, PickupPaintPlan,
    ItemOwnerDraw, ThrownPaintPlan, MissileOwnerDraw, OtherBodyPaintPlan, OtherBodyFacetRun,
    ScriptPropMeshPaint, ScriptedModelPaintPlan, ScriptPropOwnerRun, DRAW_ID_START_BODIES,
    PAINT_ID_FIRST_PROP, PAINT_ID_FIRST_PICKUP, DRAW_ID_START_ROCKETS, RemoteBodyVertexStore,
    append_debris_asset, push_loose_prop_surfaces, push_pickup_surfaces, push_projectile_surfaces,
    add_other_body_software_mesh, push_script_prop_mesh, designer_lit_mesh_material,
    body_lighting_material, bound_lit_figure_pass_material, seal_other_body_batch,
    begin_body_packed_session, swap_script_prop_mesh, queue_other_body_cpu_paint,
    keep_script_model_assets, end_body_packed_session,
};
pub use render_fx::push_effect_model_asset;

pub use render_frame::MODEL_OBJECT_ID_FIRST_PERSON_WEAPON;
pub use render_fx::{FxModelSurfaces, FxModelDrawList, FIGURE_OBJECT_ID_EFFECT_BASE};
pub use render_scene::{ModelTintRejection, FigureFacetDraw};


pub const FIGURE_COMPACT_UNAVAILABLE: &str =
    "xmodel merge: GfxPackedVertex absent or its count mismatched; decoded floats are not a packed VB";
pub const FIGURE_COMPACT_EMPTY_PLAN: &str = "xmodel plan has no vertices";

/// The payload a plan holds while its rows live in the shared bank (or
/// nowhere).
const fn unavailable_payload() -> PackedVertexPayload {
    PackedVertexPayload::Unavailable { origin_layout: FIGURE_COMPACT_UNAVAILABLE }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct FigureDrawPlan {
    /// How many merged vertices there are.
    pub decoded_tally: usize,
    pub index_list: Vec<u32>,
    pub facet_ranges: Vec<(u32, u32)>,
    pub materials: Vec<PropPassMaterial>,
    pub draws: Vec<FigureFacetDraw>,

    /// Fx model draws left out for want of an object id.
    pub effect_object_id_exhausted: u32,
    /// Moves on every change to the shared packed rows.
    pub revision: u64,
    /// Moves on every change to the shared indices or ranges.
    pub topology_edition: u64,

    pub compact_vertices: asset_world::PackedVertexPayload,
    pub compact_portion: Option<Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>>,
    /// Each slot's span inside the shared packed rows.
    pub compact_segments: render_frame::CompactSegments,
    pub index_portion: Option<Arc<Vec<u32>>>,
    pub range_portion: Option<Arc<Vec<(u32, u32)>>>,
    /// Never filled by the merge (it keeps no float rows); read by the extract.
    pub decoded_share: Option<Arc<Vec<PropVertex>>>,

    banks: Banks,
    memo: Memo,
}

/// The rotating buffers behind the shares. They outlive [`FigureDrawPlan::clear`].
#[derive(Clone, Debug, Default)]
struct Banks {
    packed: PortionBanks<[u8; asset_t6::T6_PACKED_VERTEX]>,
    index: PortionBanks<u32>,
    range: PortionBanks<(u32, u32)>,
    /// What the shared packed bank holds, when it is known.
    packed_from: Option<ContentKey>,
}

/// What the previous merge saw and laid out. Wiped by a clear.
#[derive(Clone, Debug, Default)]
struct Memo {
    input: Option<Stamp>,
    layout: Option<LayoutKey>,
    /// Every slot's geometry went in uncut, so one slot's fresh rows can be
    /// copied straight over its span.
    whole: bool,
    /// Slots that had draws at the last layout.
    mask: u8,
    /// Per packed bank: the vertex revision of each slot when its rows were
    /// last copied into that bank.
    bank_revs: [[u64; OWNERS]; PORTION_BANKS],
}

const OWNERS: usize = render_frame::COMPACT_SEGMENT_CLAIMANTS;

/// The producers in the order their rows are laid out. A slot's bit in an
/// admitted mask is `1 << slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Slot {
    Body,
    ViewModelPerspective,
    Script,
    Missile,
    Item,
    Fx,
    DynamicProp,
    Sky,
}

/// Every slot, in order (its length pins `OWNERS` to eight).
const SLOTS: [Slot; OWNERS] = [
    Slot::Body,
    Slot::ViewModelPerspective,
    Slot::Script,
    Slot::Missile,
    Slot::Item,
    Slot::Fx,
    Slot::DynamicProp,
    Slot::Sky,
];

impl Slot {
    const fn bit(self) -> u8 {
        1 << self as u8
    }

    const fn at(self) -> usize {
        self.bit().trailing_zeros() as usize
    }
}

/// One slot's published geometry and revisions, as the merge reads it.
#[derive(Clone, Copy)]
struct Source<'a> {
    gen_no: u64,
    revisions: InputChangeCounters,
    decoded_tally: usize,
    /// How many packed rows the producer has, matching or not.
    packed_len: usize,
    /// The packed rows, only when there is one per vertex.
    exact: Option<&'a [[u8; asset_t6::T6_PACKED_VERTEX]]>,
    indices: &'a [u32],
    ranges: &'a [(u32, u32)],
    materials: &'a [PropPassMaterial],
}

impl<'a> Source<'a> {
    fn new(
        gen_no: u64,
        revisions: InputChangeCounters,
        decoded_tally: usize,
        packed: &'a PackedVertexPayload,
        indices: &'a [u32],
        ranges: &'a [(u32, u32)],
        materials: &'a [PropPassMaterial],
    ) -> Self {
        let rows = match packed {
            PackedVertexPayload::T6(rows) => Some(rows.as_slice()),
            PackedVertexPayload::Unavailable { .. } => None,
        };
        Self {
            gen_no,
            revisions,
            decoded_tally,
            packed_len: rows.map_or(0, <[[u8; asset_t6::T6_PACKED_VERTEX]]>::len),
            exact: rows.filter(|r| r.len() == decoded_tally),
            indices,
            ranges,
            materials,
        }
    }

    /// A slot with nothing published (the sky when there is none).
    fn absent() -> Self {
        static NOTHING: PackedVertexPayload = unavailable_payload();
        Source::new(0, InputChangeCounters::default(), 0, &NOTHING, &[], &[], &[])
    }
}

/// Where a draw sits and how it is lit.
#[derive(Clone, Copy)]
struct Placement {
    world: Mat4,
    lighting: u32,
    light: u8,
    probe: u8,
    scope: bool,
    entity: Option<u32>,
}

impl Placement {
    /// A merged draw placed here, with nothing pending, no refusal, no
    /// packed lighting and no caster bound (unbounded: it casts into every
    /// shadow partition).
    fn draw(self, facet: u32, shade: u32, object_tag: u16) -> FigureFacetDraw {
        let colour_refusal = None;
        FigureFacetDraw {
            surface: facet,
            material: shade,
            object_tag,
            is_scoped_view: self.scope,
            stage_actor_num: self.entity,
            map_from_self: self.world,
            illum_handle: self.lighting,
            stage_light_index: self.light,
            env_probe_index: self.probe,
            queued_illum: None,
            colour_refusal,
            compact_illum: None,
            caster_bound: None,
        }
    }
}

/// Everything one merge reads. The plans are in slot order.
struct Producers<'a> {
    plans: (
        &'a OtherBodyPaintPlan,
        &'a FirstPersonPaintPlan,
        &'a ScriptedModelPaintPlan,
        &'a ThrownPaintPlan,
        &'a PickupPaintPlan,
        &'a FxModelDrawList,
        &'a DynEntPaintPlan,
    ),
    scene: Option<&'a crate::VisualStage>,
    sky: Option<(&'a SkyDomeDrawPlan, Vec3)>,
}

impl<'a> Producers<'a> {
    fn sources(&self) -> [Source<'a>; OWNERS] {
        let (body, fpv, script, missile, item, fx, debris_bit) = self.plans;
        macro_rules! source {
            ($plan:expr, $vertices:expr) => {
                Source::new(
                    $plan.generation,
                    $plan.revisions,
                    $vertices,
                    $plan.compact_vertices(),
                    $plan.indices(),
                    $plan.facet_ranges(),
                    $plan.materials(),
                )
            };
        }
        let sky = self.sky.map_or_else(Source::absent, |(sky, _)| {
            let geometry = &sky.geometry;
            Source::new(
                sky.era,
                sky.revisions,
                geometry.decoded_vertices().len(),
                &geometry.compact_vertices,
                geometry.indices(),
                geometry.facet_ranges(),
                &geometry.materials,
            )
        });
        [
            source!(body, body.decoded_tally()),
            source!(fpv, fpv.decoded_tally()),
            source!(script, script.vertices().len()),
            source!(missile, missile.vertices().len()),
            source!(item, item.vertices().len()),
            source!(fx, fx.vertices().len()),
            source!(debris_bit, debris_bit.vertices().len()),
            sky,
        ]
    }

    /// Whether the scene lets this entity's draws out this frame. Draws tied
    /// to no entity always go out.
    fn admits(&self, ent: Option<u32>) -> bool {
        ent.is_none_or(|n| {
            self.scene
                .is_some_and(|scene| scene.entity_present(n) && !scene.entity_not_drawn(n))
        })
    }

    fn fpv_colour(&self) -> bool {
        let live = |scene: &crate::VisualStage| scene.animated_object_present(crate::FIRST_PERSON_WEAPON_SLOT);
        self.plans.1.allows_tint() && self.scene.is_some_and(live)
    }

    /// The scene's verdicts on which entities draw, hashed together.
    fn occupancy(&self) -> u64 {
        let (body, _, script, missile, item, _, _) = self.plans;
        let others = [script.draws(), missile.draws(), item.draws()]
            .into_iter()
            .flatten()
            .map(|d| d.stage_actor_num);
        let entnums = body.draws().iter().map(|d| d.stage_actor_num).chain(others);
        let verdicts: Vec<bool> = std::iter::once(self.fpv_colour())
            .chain(entnums.map(|e| self.admits(e)))
            .collect();
        BuildHasherDefault::<DefaultHasher>::default().hash_one(&verdicts)
    }

    fn stamp(&self, sources: &[Source<'_>; OWNERS]) -> Stamp {
        let revs = sources.each_ref().map(|s| s.revisions);
        let fpv = self.plans.1;
        Stamp {
            layout: LayoutKey {
                per_slot: std::array::from_fn(|i| {
                    (sources[i].gen_no, revs[i].topology, revs[i].admission)
                }),
                scene: self.occupancy(),
            },
            vertex_revs: revs.map(|r| r.vertices),
            rows: revs.map(|r| (r.materials, r.draws)),
            fpv: FpvStamp {
                world: fpv.model_to_world.to_cols_array().map(f32::to_bits),
                lighting: (fpv.illum_handle, fpv.stage_light_index, fpv.env_probe_index),
                shown: (fpv.visible, fpv.viewmodel_pass_flag, self.fpv_colour()),
            },
            sky_view_bits: self.sky.map(|(_, eye)| eye.to_array().map(f32::to_bits)),
        }
    }

    /// The draws each slot sends out this frame, in slot order, plus how
    /// many fx draws found no object id.
    fn draws(&self) -> ([Vec<FigureFacetDraw>; OWNERS], u32) {
        let (body, fpv, script, missile, item, fx, debris_bit) = self.plans;
        let admitted = |list: &[FigureFacetDraw]| -> Vec<FigureFacetDraw> {
            list.iter().filter(|d| self.admits(d.stage_actor_num)).copied().collect()
        };

        // Remote bodies carry no radius here; unbounded keeps every player
        // caster in both partitions.
        let body_ids =
            std::iter::successors(Some(DRAW_ID_START_BODIES), |id| Some(id.saturating_add(1)));
        let bodies = body
            .draws()
            .iter()
            .filter(|d| self.admits(d.stage_actor_num))
            .zip(body_ids)
            .map(|(d, object_tag)| {
                let at = Placement {
                    world: d.model_to_world,
                    lighting: d.illum_handle,
                    light: d.stage_light_index,
                    probe: d.env_probe_index,
                    scope: false,
                    entity: d.stage_actor_num,
                };
                at.draw(d.surf, d.material, object_tag)
            })
            .collect();

        let mut viewmodel = Vec::new();
        if self.fpv_colour() {
            let at = Placement {
                world: fpv.model_to_world,
                lighting: fpv.illum_handle,
                light: fpv.stage_light_index,
                probe: fpv.env_probe_index,
                scope: false,
                entity: Some(crate::FIRST_PERSON_WEAPON_SLOT),
            };
            viewmodel.extend(fpv.draws().iter().map(|d| {
                let at = Placement { scope: d.is_scoped_view, ..at };
                at.draw(d.surf, d.material, MODEL_OBJECT_ID_FIRST_PERSON_WEAPON)
            }));
        }

        let highest_dynent = debris_bit.draws().iter().map(|d| d.object_tag).max();
        let mut fx_draws = Vec::new();
        let spent = dense_effect_object_ids(fx.draws(), highest_dynent, &mut fx_draws);

        let sky = self.sky.map_or_else(Vec::new, |(sky, eye)| {
            let local_to_world = Mat4::from_translation(eye);
            sky.draws.iter().map(|d| FigureFacetDraw { map_from_self: local_to_world, ..*d }).collect()
        });

        let lists = [
            bodies,
            viewmodel,
            admitted(script.draws()),
            admitted(missile.draws()),
            admitted(item.draws()),
            fx_draws,
            debris_bit.draws().to_vec(),
            sky,
        ];
        (lists, spent)
    }
}

/// The slots that have draws, as a mask.
fn admitted_mask(lists: &[Vec<FigureFacetDraw>; OWNERS]) -> u8 {
    SLOTS
        .iter()
        .zip(lists)
        .filter_map(|(seat, list)| (!list.is_empty()).then_some(seat.bit()))
        .fold(0, |mask, bit| mask | bit)
}

/// What fixes where the merged rows go. While it holds, a merge only needs
/// to refresh contents in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayoutKey {
    /// Per slot: generation, topology revision, admission revision.
    per_slot: [(u64, u64, u64); OWNERS],
    /// The scene's admission verdicts, hashed.
    scene: u64,
}

/// The viewmodel inputs the merged draws carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FpvStamp {
    world: [u32; 16],
    /// Lighting handle, scene light, reflection probe.
    lighting: (u32, u8, u8),
    /// Visible, drawn gun, colour admitted.
    shown: (bool, Option<i32>, bool),
}

/// The sky's eye position as bits, when there is a sky.

/// Every input one merge's output depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    layout: LayoutKey,
    vertex_revs: [u64; OWNERS],
    /// Per slot: material revision, draw revision.
    rows: [(u64, u64); OWNERS],
    /// The viewmodel inputs.
    fpv: FpvStamp,
    /// The sky's eye position, as bits.
    sky_view_bits: Option<[u32; 3]>,
}

impl Stamp {
    /// True when the draw or material rows built from `then` are stale.
    fn draws_differ(&self, then: &Stamp) -> bool {
        self.rows != then.rows || self.fpv != then.fpv || self.sky_view_bits != then.sky_view_bits
    }

    fn content(&self, mask: u8, rows: usize) -> ContentKey {
        ContentKey(self.layout, self.vertex_revs, mask, rows)
    }
}

/// What a set of shared packed rows was built from: layout, vertex
/// revisions, admitted mask and row count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContentKey(LayoutKey, [u64; OWNERS], u8, usize);

#[allow(clippy::too_many_arguments)]
pub fn merge_figure_draw_plan(
    merged: &mut FigureDrawPlan,
    fpv: &FirstPersonPaintPlan,
    body_list: &OtherBodyPaintPlan,
    script_list: &ScriptedModelPaintPlan,
    rockets: &ThrownPaintPlan,
    item_list: &PickupPaintPlan,
    effect_meshes: &FxModelDrawList,
    dyn_ents: &DynEntPaintPlan,
    scene: Option<&crate::VisualStage>,
    sky: Option<(&SkyDomeDrawPlan, Vec3)>,
) {
    let producers = Producers {
        plans: (body_list, fpv, script_list, rockets, item_list, effect_meshes, dyn_ents),
        scene,
        sky,
    };
    let sources = producers.sources();
    let stamp = producers.stamp(&sources);
    let seen = merged.memo.input;
    if seen == Some(stamp) {
        return;
    }
    let in_place = merged.memo.whole && merged.memo.layout == Some(stamp.layout);
    if !in_place {
        merged.relayout(&producers, &sources, stamp);
        return;
    }
    if seen.is_none_or(|then| then.vertex_revs != stamp.vertex_revs) {
        merged.recopy_packed(&sources, &stamp);
    }
    if seen.is_none_or(|then| stamp.draws_differ(&then)) {
        merged.refresh_draws(&producers, &sources);
    }
    merged.memo.input = Some(stamp);
}

impl FigureDrawPlan {
    pub fn clear(&mut self) {
        let Self {
            decoded_tally,
            index_list: indices,
            facet_ranges,
            materials,
            draws,
            effect_object_id_exhausted,
            compact_vertices,
            compact_portion,
            compact_segments,
            index_portion,
            range_portion,
            decoded_share,
            memo,
            ..
        } = self;
        *decoded_tally = 0;
        *effect_object_id_exhausted = 0;
        indices.clear();
        facet_ranges.clear();
        materials.clear();
        draws.clear();
        *compact_portion = None;
        *index_portion = None;
        *range_portion = None;
        *decoded_share = None;
        compact_segments.forget();
        *compact_vertices = unavailable_payload();
        *memo = Memo::default();
    }

    /// The rows held in the plan itself, or why it holds none.
    fn local_rows(&self) -> Result<&[[u8; asset_t6::T6_PACKED_VERTEX]], PackedVertexRefusal> {
        match self.compact_vertices {
            PackedVertexPayload::T6(ref rows) => Ok(rows.as_slice()),
            PackedVertexPayload::Unavailable { origin_layout } => {
                Err(PackedVertexRefusal::ForeignLayout { origin_layout })
            }
        }
    }

    pub fn compact_rows(&self) -> Option<&[[u8; asset_t6::T6_PACKED_VERTEX]]> {
        let shared = self.compact_portion.as_deref().map(Vec::as_slice);
        shared.or_else(|| self.local_rows().ok())
    }

    pub fn idx_rows(&self) -> &[u32] {
        super::published_rows(&self.index_portion)
    }

    pub fn span_rows(&self) -> &[(u32, u32)] {
        super::published_rows(&self.range_portion)
    }

    pub fn precise_compact_vertices(&self) -> Result<&[[u8; asset_t6::T6_PACKED_VERTEX]], PackedVertexRefusal> {
        use render_material::decl_table;
        let wanted = asset_t6::T6_PACKED_VERTEX as u16;
        let table_stride = decl_table::stream_extent(decl_table::COMPRESSED_VERTEX_KIND, 0);
        if table_stride.is_none_or(|stride| stride != wanted) {
            return Err(PackedVertexRefusal::StrideMismatch { table_stride });
        }
        let rows = match self.compact_portion.as_deref() {
            Some(shared) => shared.as_slice(),
            None => self.local_rows()?,
        };
        match rows.len() {
            n if n == self.decoded_tally => Ok(rows),
            packed => Err(PackedVertexRefusal::CountsDiffer {
                packed,
                decoded: self.decoded_tally,
            }),
        }
    }

    /// Builds every row again from nothing.
    fn relayout(&mut self, producers: &Producers<'_>, sources: &[Source<'_>; OWNERS], stamp: Stamp) {
        let old_packed = self.compact_portion.take();
        let old_index = self.index_portion.take();
        let old_range = self.range_portion.take();
        let mut out = PackedOut::new(self.banks.packed.claim_write());
        let bank = self.banks.packed.write_at();
        let new_indices = self.banks.index.claim_write();
        let new_ranges = self.banks.range.claim_write();
        self.clear();
        self.index_list = new_indices;
        self.facet_ranges = new_ranges;
        self.memo.whole = true;

        // A hint only: the sky's rows were never part of it.
        let hint: usize = sources[..Slot::Sky.at()].iter().map(|s| s.packed_len).sum();
        out.row_list.reserve(hint);
        let (lists, exhausted) = producers.draws();
        let mask = admitted_mask(&lists);
        self.effect_object_id_exhausted = exhausted;
        self.memo.mask = mask;

        let laid = sources.iter().zip(lists).filter(|(_, list)| !list.is_empty());
        for (slot_src, list) in laid {
            self.append_owner(&mut out, slot_src, list);
        }
        if out.ok {
            self.memo.bank_revs[bank] = stamp.vertex_revs;
        }
        self.install_packed(old_packed, out, &stamp, mask);
        self.publish_topology(old_index, old_range);
        self.memo.input = Some(stamp);
        self.memo.layout = Some(stamp.layout);
    }

    /// Copies the slots' moved packed rows over their spans.
    fn recopy_packed(&mut self, sources: &[Source<'_>; OWNERS], stamp: &Stamp) {
        let old_packed = self.compact_portion.take();
        let (rows, new_bank) = self.banks.packed.claim_write_keep();
        let mask = self.memo.mask;
        let revs = &mut self.memo.bank_revs[self.banks.packed.write_at()];
        if new_bank {
            *revs = [0; OWNERS];
        }
        let mut out = PackedOut::new(rows);
        let patch = Patch { sources, mask, revs: stamp.vertex_revs, new_bank };
        patch.apply(&mut out, revs, &mut self.compact_segments);
        self.install_packed(old_packed, out, &stamp, mask);
    }

    /// Hands the gathered rows to the shared bank, keyed by what they were
    /// built from when every row made it.
    fn install_packed(
        &mut self,
        old: Option<Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>>,
        out: PackedOut,
        stamp: &Stamp,
        mask: u8,
    ) {
        let PackedOut { row_list: rows, ok } = out;
        let key = ok.then(|| stamp.content(mask, rows.len()));
        let (empty, unavailable) = (FIGURE_COMPACT_EMPTY_PLAN, FIGURE_COMPACT_UNAVAILABLE);
        let n = self.decoded_tally;
        self.publish_packed(old, install_held_compact(ok, rows, n, empty, unavailable), key);
    }

    /// Builds the draw and material rows again over the layout already in
    /// place.
    fn refresh_draws(&mut self, producers: &Producers<'_>, sources: &[Source<'_>; OWNERS]) {
        self.draws.truncate(0);
        self.materials.truncate(0);
        let (lists, exhausted) = producers.draws();
        self.effect_object_id_exhausted = exhausted;
        let mut first_range = 0u32;
        let laid = sources.iter().zip(lists).filter(|(_, list)| !list.is_empty());
        for (slot_src, list) in laid {
            self.push_draws(first_range, slot_src, list);
            first_range = first_range.saturating_add(slot_src.ranges.len() as u32);
        }
    }

    /// Appends a slot's materials, then its draws pointed at the merged
    /// ranges and materials.
    fn push_draws(&mut self, first_range: u32, source: &Source<'_>, list: Vec<FigureFacetDraw>) {
        let first_material = self.materials.len() as u32;
        self.materials.extend_from_slice(source.materials);
        let moved = |mut d: FigureFacetDraw| {
            d.surface = d.surface.saturating_add(first_range);
            d.material = d.material.saturating_add(first_material);
            d
        };
        self.draws.extend(list.into_iter().map(moved));
    }

    /// Lays one slot out: uncut when its draws touch every surface, else cut
    /// down to the surfaces (and vertices) the draws touch.
    fn append_owner(&mut self, out: &mut PackedOut, source: &Source<'_>, list: Vec<FigureFacetDraw>) {
        let touched = marks(source.ranges.len(), list.iter().map(|d| d.surface));
        if !touched.is_empty() && !touched.contains(&false) {
            self.append_uncut(source, list);
            out.take_whole(source);
        } else {
            self.append_cut(out, source, &touched, list);
        }
    }

    fn append_uncut(&mut self, source: &Source<'_>, list: Vec<FigureFacetDraw>) {
        let first_vertex = self.decoded_tally as u32;
        let first_index = self.index_list.len() as u32;
        let first_range = self.facet_ranges.len() as u32;
        self.decoded_tally = self.decoded_tally.saturating_add(source.decoded_tally);
        let indices = source.indices.iter().map(|&i| first_vertex.saturating_add(i));
        self.index_list.extend(indices);
        let ranges = source.ranges.iter().map(|&(at, n)| (first_index.saturating_add(at), n));
        self.facet_ranges.extend(ranges);
        self.push_draws(first_range, source, list);
    }

    /// Lays out only the `touched` surfaces, numbering their vertices afresh
    /// in first-use order and keeping only the materials the draws name. The
    /// layout stops being uncut, so later merges rebuild in full.
    fn append_cut(
        &mut self,
        out: &mut PackedOut,
        source: &Source<'_>,
        touched: &[bool],
        mut list: Vec<FigureFacetDraw>,
    ) {
        if source.exact.is_none() && source.decoded_tally != 0 {
            out.give_up();
        }
        let copy_from = source.exact.filter(|_| out.ok);
        let mut next_vertex = self.decoded_tally as u32;
        let mut vertex_to: Vec<Option<u32>> = vec![None; source.decoded_tally];
        let mut surface_to: Vec<Option<u32>> = vec![None; source.ranges.len()];

        let kept = source.ranges.iter().enumerate().filter(|&(s, _)| touched[s]);
        for (facet, &(at, count)) in kept {
            let begin = at as usize;
            let Some(span) = source.indices.get(begin..begin.saturating_add(count as usize)) else {
                continue;
            };
            surface_to[facet] = Some(self.facet_ranges.len() as u32);
            self.facet_ranges.push((self.index_list.len() as u32, count));
            for old in span.iter().map(|&i| i as usize) {
                let Some(to) = vertex_to.get_mut(old) else {
                    continue;
                };
                let renumbered = *to.get_or_insert_with(|| {
                    if let Some(rows) = copy_from {
                        out.row_list.push(rows[old]);
                    }
                    let id = next_vertex;
                    next_vertex = next_vertex.saturating_add(1);
                    id
                });
                self.index_list.push(renumbered);
            }
        }

        let named = marks(source.materials.len(), list.iter().map(|d| d.material));
        let material_to: Vec<Option<u32>> = named
            .iter()
            .zip(source.materials)
            .map(|(&keep, mat)| {
                keep.then(|| {
                    self.materials.push(mat.clone());
                    self.materials.len() as u32 - 1
                })
            })
            .collect();

        let renumbered = vertex_to.iter().flatten().count();
        self.decoded_tally = self.decoded_tally.saturating_add(renumbered);
        self.memo.whole = false;
        for d in &mut list {
            if let Some(&Some(to)) = surface_to.get(d.surface as usize) {
                d.surface = to;
            }
            if let Some(&Some(to)) = material_to.get(d.material as usize) {
                d.material = to;
            }
        }
        self.draws.append(&mut list);
    }

    /// Shares the packed rows. Rows with the same content as the ones already
    /// shared keep the old share (and the revision).
    fn publish_packed(
        &mut self,
        old: Option<Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>>,
        payload: PackedVertexPayload,
        key: Option<ContentKey>,
    ) {
        let share = match payload {
            PackedVertexPayload::T6(rows) => {
                let same_content = key.is_some() && key == self.banks.packed_from;
                self.banks.packed.store_write(rows);
                self.compact_vertices = unavailable_payload();
                if same_content && old.is_some() {
                    self.compact_portion = old;
                    return;
                }
                self.banks.packed_from = key;
                Some(self.banks.packed.posted())
            }
            refused => {
                self.banks.packed_from = None;
                self.compact_vertices = refused;
                None
            }
        };
        if share.is_some() || old.is_some() {
            let next = self.revision.wrapping_add(1);
            self.revision = next;
        }
        self.compact_portion = share;
    }

    /// Shares the merged indices and ranges. When both match the old shares,
    /// those stay and the topology revision holds.
    fn publish_topology(
        &mut self,
        old_index: Option<Arc<Vec<u32>>>,
        old_range: Option<Arc<Vec<(u32, u32)>>>,
    ) {
        let unchanged = old_index.as_deref() == Some(&self.index_list)
            && old_range.as_deref() == Some(&self.facet_ranges);
        let indices = std::mem::take(&mut self.index_list);
        let ranges = std::mem::take(&mut self.facet_ranges);
        self.banks.index.store_write(indices);
        self.banks.range.store_write(ranges);
        if unchanged {
            (self.index_portion, self.range_portion) = (old_index, old_range);
            return;
        }
        self.topology_edition = self.topology_edition.wrapping_add(1);
        self.index_portion = Some(self.banks.index.posted());
        self.range_portion = Some(self.banks.range.posted());
    }
}

/// `len` flags, set where `picks` names an index in range.
fn marks(len: usize, picks: impl Iterator<Item = u32>) -> Vec<bool> {
    let mut flags = vec![false; len];
    for pick in picks {
        if let Some(flag) = flags.get_mut(pick as usize) {
            *flag = true;
        }
    }
    flags
}

/// Packed rows being gathered for one bank, and whether every slot so far
/// had one row per vertex (once one did not, no rows are kept).
struct PackedOut {
    row_list: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    ok: bool,
}

impl PackedOut {
    fn new(row_list: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>) -> Self {
        Self { row_list, ok: true }
    }

    fn give_up(&mut self) {
        self.ok = false;
        self.row_list.clear();
    }

    /// Appends all of a slot's packed rows.
    fn take_whole(&mut self, source: &Source<'_>) {
        if !self.ok || source.decoded_tally == 0 {
            return;
        }
        match source.exact {
            Some(rows) => self.row_list.extend_from_slice(rows),
            None => self.give_up(),
        }
    }
}

/// Bringing one packed bank up to date with the slots of the current layout.
struct Patch<'s, 'a> {
    sources: &'s [Source<'a>; OWNERS],
    mask: u8,
    /// The slots' vertex revisions now.
    revs: [u64; OWNERS],
    /// The bank came back empty because every bank was held.
    new_bank: bool,
}

impl Patch<'_, '_> {
    fn laid(&self, i: usize) -> bool {
        self.mask & SLOTS[i].bit() != 0
    }

    fn live(&self, i: usize) -> bool {
        self.laid(i) && self.sources[i].decoded_tally != 0
    }

    /// Copies in place when the bank already has this layout's size, else
    /// rebuilds it. `out.ok` ends false when some slot lacks one packed row
    /// per vertex.
    fn apply(&self, out: &mut PackedOut, bank_revs: &mut [u64; OWNERS], segments: &mut CompactSegments) {
        let counts: [usize; OWNERS] =
            std::array::from_fn(|i| if self.live(i) { self.sources[i].packed_len } else { 0 });
        let total = counts.iter().fold(0usize, |sum, &n| sum.saturating_add(n));
        if self.new_bank || out.row_list.len() != total {
            out.row_list.clear();
            out.row_list.reserve(total);
            for i in (0..OWNERS).filter(|&i| self.laid(i)) {
                out.take_whole(&self.sources[i]);
            }
            *bank_revs = if out.ok { self.revs } else { [0; OWNERS] };
            segments.forget();
            return;
        }
        let mut end = 0usize;
        for i in (0..OWNERS).filter(|&i| self.live(i)) {
            let Some(rows) = self.sources[i].exact else {
                out.give_up();
                *bank_revs = [0; OWNERS];
                segments.forget();
                return;
            };
            let start = end;
            end += rows.len();
            if bank_revs[i] != self.revs[i] {
                out.row_list[start..end].copy_from_slice(rows);
            }
        }
        *bank_revs = self.revs;
        let lens = counts.map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        let mut starts = [0u32; OWNERS];
        for i in 1..OWNERS {
            starts[i] = starts[i - 1].saturating_add(lens[i - 1]);
        }
        segments.publish(std::array::from_fn(|i| CompactSegment {
            start: starts[i],
            rows: lens[i],
            revision: self.revs[i],
        }));
    }
}

/// Numbers the fx draws densely after the other producers' ids (`past` is
/// the highest of those): one id per run of draws from the same source
/// object, appended to `kept`. Draws past the last id are dropped; the
/// return is how many.
fn dense_effect_object_ids(
    fx: &[FigureFacetDraw],
    past: Option<u16>,
    kept: &mut Vec<FigureFacetDraw>,
) -> u32 {
    let first = past.map_or(Some(FIGURE_OBJECT_ID_EFFECT_BASE), |top| {
        top.checked_add(1).map(|id| id.max(FIGURE_OBJECT_ID_EFFECT_BASE))
    });
    kept.reserve(fx.len());
    let runs = fx.chunk_by(|a, b| a.object_tag == b.object_tag);
    runs.enumerate().fold(0u32, |dropped, (nth, run)| {
        let id = u16::try_from(nth)
            .ok()
            .zip(first)
            .and_then(|(nth, first)| first.checked_add(nth));
        match id {
            Some(object_tag) => {
                kept.extend(run.iter().map(|d| FigureFacetDraw { object_tag, ..*d }));
                dropped
            }
            None => dropped.saturating_add(run.len() as u32),
        }
    })
}

pub(crate) fn apply_resolved_figure_illum(
    table: Res<crate::prepare::scene::model_illum_cache::ResolvedModelIllumTable>,
    mut targets: (
        ResMut<ScriptedModelPaintPlan>,
        ResMut<PickupPaintPlan>,
        ResMut<ThrownPaintPlan>,
        ResMut<DynEntPaintPlan>,
        ResMut<FirstPersonPaintPlan>,
        ResMut<crate::assemble::draw_items::DrawFocus>,
    ),
) {
    let (scripted_ent, item, missile, debris_bit, fpv, focus) = &mut targets;
    let failed = scripted_ent.finalize_illum(&table);
    item.finalize_illum(&table);
    missile.finalize_illum(&table);
    debris_bit.finalize_illum(&table);
    fpv.finalize_illum(&table);

    let Some(frame) = focus.frame.as_mut() else {
        return;
    };
    let Some(focused) = frame.object_tag else {
        return;
    };
    if failed.contains(&focused) {
        let outcome = "lighting_failed";
        frame.outcome = outcome;
        frame.illum_handle = None;
        return;
    }
    if let Some(d) = scripted_ent.draws().iter().find(|d| d.object_tag == focused) {
        frame.illum_handle = Some(d.illum_handle);
    }
}

pub(crate) fn apply_resolved_effect_model_illum(
    resolved: Res<crate::prepare::scene::model_illum_cache::ResolvedModelIllumTable>,
    mut plan: ResMut<FxModelDrawList>,
) {
    plan.finalize_illum(&resolved);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(facet: u32, shade: u32, object_tag: u16) -> FigureFacetDraw {
        let at = Placement {
            world: Mat4::IDENTITY,
            lighting: 0,
            light: 0,
            probe: 0,
            scope: false,
            entity: None,
        };
        at.draw(facet, shade, object_tag)
    }

    fn row(tag: u8) -> [u8; asset_t6::T6_PACKED_VERTEX] {
        [tag; asset_t6::T6_PACKED_VERTEX]
    }

    /// A material with no maps; the tests only count materials.
    fn material() -> PropPassMaterial {
        use super::super::prop_mesh::{live_model_tables, unlit_pass_material};
        unlit_pass_material(live_model_tables(None, &Default::default(), &[]), 0, None)
    }

    /// Geometry for one slot: two triangles, six vertices, a surface each.
    struct Mesh {
        packed: PackedVertexPayload,
        /// Indices, then surface ranges.
        topology: (Vec<u32>, Vec<(u32, u32)>),
        materials: Vec<PropPassMaterial>,
    }

    fn mesh(tags: std::ops::RangeInclusive<u8>) -> Mesh {
        Mesh {
            packed: PackedVertexPayload::T6(tags.map(row).collect()),
            topology: ((0..6).collect(), vec![(0, 3), (3, 3)]),
            materials: vec![material(), material()],
        }
    }

    impl Mesh {
        fn source(&self) -> Source<'_> {
            let (index_list, ranges) = &self.topology;
            let revisions = InputChangeCounters::default();
            let mats = &self.materials;
            Source::new(1, revisions, 6, &self.packed, index_list, ranges, mats)
        }
    }

    fn uncut_plan() -> FigureDrawPlan {
        let mut plan = FigureDrawPlan::default();
        plan.memo.whole = true;
        plan
    }

    #[test]
    fn fx_object_ids_follow_the_others_one_per_source_object() {
        let fx = [draw(0, 0, 7), draw(1, 0, 7), draw(2, 0, 9)];
        let mut dense = Vec::new();
        let spent = dense_effect_object_ids(&fx, Some(FIGURE_OBJECT_ID_EFFECT_BASE + 4), &mut dense);
        let next = FIGURE_OBJECT_ID_EFFECT_BASE + 5;
        let ids: Vec<u16> = dense.into_iter().map(|d| d.object_tag).collect();
        assert_eq!((ids, spent), (vec![next, next, next + 1], 0));
    }

    #[test]
    fn fx_draws_with_no_object_id_left_are_counted_and_dropped() {
        let mut dense = Vec::new();
        let spent = dense_effect_object_ids(&[draw(0, 0, 7), draw(1, 0, 8)], Some(u16::MAX), &mut dense);
        assert_eq!((dense.len(), spent), (0, 2));
    }

    #[test]
    fn a_whole_owner_is_laid_after_the_one_before() {
        let meshes = [mesh(1..=6), mesh(1..=6)];
        let mut plan = uncut_plan();
        let mut out = PackedOut::new(Vec::new());
        for m in &meshes {
            plan.append_owner(&mut out, &m.source(), vec![draw(0, 0, 1), draw(1, 1, 1)]);
        }
        assert!(out.ok && plan.memo.whole);
        assert_eq!(plan.decoded_tally, 12);
        assert_eq!(plan.index_list[6..], [6, 7, 8, 9, 10, 11]);
        assert_eq!(plan.facet_ranges, [(0, 3), (3, 3), (6, 3), (9, 3)]);
        assert_eq!(plan.materials.len(), 4);
        let pointed: Vec<(u32, u32)> = plan.draws.iter().map(|d| (d.surface, d.material)).collect();
        assert_eq!(pointed, [(0, 0), (1, 1), (2, 2), (3, 3)]);
        assert_eq!(out.row_list.len(), 12);
    }

    #[test]
    fn an_owner_with_an_unused_surface_is_cut_down_to_what_draws() {
        let m = mesh(1..=6);
        let mut plan = uncut_plan();
        let mut out = PackedOut::new(Vec::new());
        plan.append_owner(&mut out, &m.source(), vec![draw(1, 1, 1)]);
        assert!(out.ok && !plan.memo.whole);
        assert_eq!(plan.decoded_tally, 3);
        assert_eq!(plan.index_list, [0, 1, 2]);
        assert_eq!(plan.facet_ranges, [(0, 3)]);
        assert_eq!(plan.materials.len(), 1);
        let only = plan.draws[0];
        assert_eq!((only.surface, only.material), (0, 0));
        assert_eq!(out.row_list, [row(4), row(5), row(6)]);
    }

    #[test]
    fn rows_that_do_not_match_the_vertices_drop_every_packed_row() {
        let mut m = mesh(1..=6);
        m.packed = PackedVertexPayload::T6(vec![row(1)]);
        let mut plan = FigureDrawPlan::default();
        let mut out = PackedOut::new(vec![row(9)]);
        plan.append_owner(&mut out, &m.source(), vec![draw(0, 0, 1), draw(1, 1, 1)]);
        assert!(!out.ok);
        assert!(out.row_list.is_empty());
    }

    #[test]
    fn a_bank_with_the_layout_is_patched_only_where_a_revision_moved() {
        let (body, fpv, moved) = (mesh(1..=6), mesh(1..=6), mesh(11..=16));
        let mask = Slot::Body.bit() | Slot::ViewModelPerspective.bit();
        let mut sources = [Source::absent(); OWNERS];
        sources[Slot::Body.at()] = body.source();
        sources[Slot::ViewModelPerspective.at()] = fpv.source();
        let mut bank = PackedOut::new(Vec::new());
        let mut bank_revs = [0; OWNERS];
        let mut segments = CompactSegments::default();

        let first = Patch { sources: &sources, mask, revs: [0; OWNERS], new_bank: true };
        first.apply(&mut bank, &mut bank_revs, &mut segments);
        assert!(bank.ok);
        assert!(bank.row_list.len() == 12 && !segments.live);

        sources[Slot::ViewModelPerspective.at()] = moved.source();
        let mut revs = [0u64; OWNERS];
        revs[Slot::ViewModelPerspective.at()] = 1;
        let second = Patch { sources: &sources, mask, revs, new_bank: false };
        second.apply(&mut bank, &mut bank_revs, &mut segments);
        assert!(bank.ok && segments.live);
        assert_eq!([bank.row_list[0], bank.row_list[6], bank.row_list[11]], [row(1), row(11), row(16)]);
        assert_eq!(bank_revs, revs);
        let span = &segments.spans[Slot::ViewModelPerspective.at()];
        assert_eq!((span.start, span.rows), (6, 6));
    }

    #[test]
    fn merging_nothing_leaves_an_empty_plan_with_no_packed_rows() {
        let mut plan = FigureDrawPlan::default();
        let fpv = FirstPersonPaintPlan::default();
        let bodies = OtherBodyPaintPlan::default();
        let scripts = ScriptedModelPaintPlan::default();
        let missiles = ThrownPaintPlan::default();
        let items = PickupPaintPlan::default();
        let fx = FxModelDrawList::without_geometry(0);
        let dynents = DynEntPaintPlan::default();
        for _ in 0..2 {
            merge_figure_draw_plan(
                &mut plan, &fpv, &bodies, &scripts, &missiles, &items, &fx, &dynents, None, None,
            );
        }
        assert!(plan.draws.is_empty());
        assert!(plan.compact_rows().is_none());
        assert_eq!(plan.decoded_tally, 0);
        assert!(plan.precise_compact_vertices().is_err());
    }
}
