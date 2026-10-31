//! Draw plans: for every kind of model the presenter shows, the rows it
//! leaves for the renderer each frame, and the revision counters a consumer
//! reads to learn which part of them changed.

use asset_t6::T6_PACKED_VERTEX;
use asset_world::{LevelModelAssetId, PackedVertexPayload};
use bevy::prelude::*;
use render_frame::{PropVertex, publish_rows};
use render_scene::{
    ModelIllumOwner, ResolvedModelIllum, ResolvedModelIllumTable,
};
use render_scene::{PropPassMaterial, FigureFacetDraw};

pub use render_frame::InputChangeCounters;

/// Why a viewmodel plan has no packed vertices to hand over.
pub const FPV_PACKED_MISSING: &str = concat!(
    "viewmodel packed vertices absent, ",
    "or their count disagrees with the float rows"
);
pub const FPV_PACKED_BLANK_PLAN: &str = concat!("viewmodel plan ", "holds no vertices");
pub const BODY_VERTS_DROPPED_NOTE: &str = concat!("remote body packed vertices ", "were never kept");

/// Each kind of model numbers its draws from its own base, which lets a failed
/// lighting lookup be traced to the plan that asked.
pub const DRAW_ID_START_BODIES: u16 = 2;
pub const DRAW_ID_START_ROCKETS: u16 = 0x400;
pub const PAINT_ID_FIRST_PICKUP: u16 = 0x500;
pub const PAINT_ID_FIRST_PROP: u16 = 0x600;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunSurfaceBatch {
    pub surf: u32,
    pub material: u32,
    pub is_scoped_view: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtherBodyFacetRun {
    pub surf: u32,
    pub material: u32,
    pub model_to_world: Mat4,
    pub illum_handle: u32,
    pub stage_light_index: u8,
    pub env_probe_index: u8,
    pub stage_actor_num: Option<u32>,
}

/// A script model's asset as the plan keeps it: what it was drawn with, and
/// its `(surface, material)` pairs.
#[derive(Clone, Debug)]
pub struct ScriptPropMeshPaint {
    pub key: LevelModelAssetId,
    pub puppet_state: xmodel_runtime::RigPoseRecord,
    pub viewer_detail_levels: Vec<Option<u8>>,
    pub surfs: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptPropOwnerRun {
    pub ent: Entity,
    pub active_mesh: LevelModelAssetId,
    pub object_tag: u16,
}

/// For plans whose owners need nothing but an object id to be told apart.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelOwnerPaint {
    pub object_tag: u16,
    pub xmodel: String,
}

pub type MissileOwnerDraw = ModelOwnerPaint;
pub type ItemOwnerDraw = ModelOwnerPaint;
pub type DebrisOwnerDraw = ModelOwnerPaint;

#[derive(Clone, Debug)]
pub struct PickupMeshPaint {
    pub xmodel_name: String,
    pub surf_list: Vec<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub struct PropMeshPaint {
    pub key: LevelModelAssetId,
    pub view_lod: Option<u8>,
    pub surf_pairs: Vec<(u32, u32)>,
}

/// What a plan's owner rows must tell the lighting step: which draw they
/// belong to.
trait OwnerRow {
    fn row_id(&self) -> u16;
}

impl OwnerRow for ScriptPropOwnerRun {
    fn row_id(&self) -> u16 {
        let Self { object_tag, .. } = self;
        *object_tag
    }
}

impl OwnerRow for ModelOwnerPaint {
    fn row_id(&self) -> u16 {
        let Self { object_tag, .. } = self;
        *object_tag
    }
}

/// How one draw fared when its lighting was seated.
enum Seating {
    Failed,
    Unchanged,
    Changed,
}

/// Seats one draw's pending lighting request from `table`. `None` when the draw
/// asked for nothing.
fn seat_draw(draw: &mut FigureFacetDraw, table: &ResolvedModelIllumTable) -> Option<Seating> {
    let request = draw.queued_illum.take()?;
    let seated = match table.answer_for(request.owner) {
        Some(ResolvedModelIllum::Seated {
            handle,
            stage_light_index,
            env_probe_index,
            compact_illum,
        }) => (handle, stage_light_index, env_probe_index, compact_illum),
        _ => return Some(Seating::Failed),
    };
    let lit = |d: &FigureFacetDraw| {
        (d.illum_handle, d.stage_light_index, d.env_probe_index, d.compact_illum)
    };
    let held = lit(draw);
    if held == seated {
        return Some(Seating::Unchanged);
    }
    (draw.illum_handle, draw.stage_light_index, draw.env_probe_index, draw.compact_illum) = seated;
    Some(Seating::Changed)
}

/// Read-only views of what a plan published. Handing out the rows mutably
/// would let a consumer change them without moving the revision.
macro_rules! published_views {
    ($plan:ty, $draw:ty) => {
        impl $plan {
            pub fn draws(&self) -> &[$draw] {
                &self.draws
            }

            pub fn materials(&self) -> &[PropPassMaterial] {
                self.mat_list.as_slice()
            }

            pub fn indices(&self) -> &[u32] {
                &self.indices
            }

            pub fn facet_ranges(&self) -> &[(u32, u32)] {
                &self.facet_ranges
            }

            pub fn compact_vertices(&self) -> &PackedVertexPayload {
                &self.compact_vertices
            }
        }
    };
}

/// The count of vertices a packed-only plan holds.
macro_rules! decoded_view {
    ($plan:ty) => {
        impl $plan {
            pub fn decoded_tally(&self) -> usize {
                let Self { decoded_tally, .. } = self;
                *decoded_tally
            }
        }
    };
}

/// Declares a plan resource. Every plan carries the same geometry, draw and
/// revision fields; whatever only this plan needs follows the draw type.
macro_rules! draw_plan {
    (
        $(#[$meta:meta])*
        $name:ident, draw: $draw:ty,
        $($(#[$field_meta:meta])* $field_vis:vis $field:ident: $field_ty:ty),* $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Resource, Clone, Debug, Default)]
        pub struct $name {
            pub(crate) verts: Vec<PropVertex>,
            pub(crate) indices: Vec<u32>,
            pub(crate) facet_ranges: Vec<(u32, u32)>,
            pub(crate) mat_list: Vec<PropPassMaterial>,
            pub(crate) draws: Vec<$draw>,
            pub(crate) compact_vertices: PackedVertexPayload,
            pub plan_rev: u64,
            pub generation: u64,
            pub revisions: InputChangeCounters,
            $($(#[$field_meta])* $field_vis $field: $field_ty,)*
        }

        published_views!($name, $draw);

        impl $name {
            pub fn vertices(&self) -> &[PropVertex] {
                &self.verts
            }
        }
    };
}

/// Borrows of the parts every owned-row plan changes together: the draws, the
/// owners that go with them, and the counters that say something moved.
struct RowSet<'a, O> {
    draw_list: &'a mut Vec<FigureFacetDraw>,
    owners: &'a mut Vec<O>,
    revisions: &'a mut InputChangeCounters,
    revision: &'a mut u64,
}

impl<O: OwnerRow + PartialEq> RowSet<'_, O> {
    fn count_change(&mut self) {
        *self.revision = next_revision(*self.revision);
        let revs: &mut InputChangeCounters = self.revisions;
        InputChangeCounters::advance_drawlist_stamp(revs);
    }

    /// Takes the new rows without counting; says whether either side differed.
    /// Both sides are always handed over, so neither comparison is skipped.
    fn exchange(&mut self, fresh_draws: &mut Vec<FigureFacetDraw>, fresh_owners: &mut Vec<O>) -> bool {
        let draws_moved = publish_rows(self.draw_list, fresh_draws);
        let owners_moved = publish_rows(self.owners, fresh_owners);
        draws_moved || owners_moved
    }

    fn take_frame(&mut self, fresh_draws: &mut Vec<FigureFacetDraw>, fresh_owners: &mut Vec<O>) {
        if self.exchange(fresh_draws, fresh_owners) {
            self.count_change();
        }
    }

    fn take_nothing(&mut self) {
        if self.draw_list.is_empty() && self.owners.is_empty() {
            return;
        }
        self.owners.clear();
        self.draw_list.clear();
        let revs: &mut InputChangeCounters = self.revisions;
        revs.advance_acceptance_stamp();
        self.count_change();
    }

    /// Seats each draw's lighting from `table`. A draw whose lookup failed goes,
    /// with its owner, and its object id is returned.
    fn seat_lighting(&mut self, table: &ResolvedModelIllumTable) -> Vec<u16> {
        let mut lost: Vec<u16> = Vec::new();
        let mut relit = false;
        let asking = self.draw_list.iter_mut().filter(|d| d.queued_illum.is_some());
        for draw in asking {
            match seat_draw(draw, table) {
                Some(Seating::Changed) => relit = true,
                Some(Seating::Failed) => lost.push(draw.object_tag),
                Some(Seating::Unchanged) | None => {}
            }
        }
        let revs: &mut InputChangeCounters = self.revisions;
        if !lost.is_empty() {
            self.draw_list.retain(|draw| !lost.contains(&draw.object_tag));
            self.owners.retain(|owner| !lost.contains(&owner.row_id()));
            revs.advance_acceptance_stamp();
            relit = true;
        }
        if relit {
            revs.advance_drawlist_stamp();
        }
        lost
    }
}

/// Gives a plan the rows view and the step that seats the lighting its draws
/// asked for.
macro_rules! owned_rows {
    ($plan:ty, $owner:ty) => {
        impl $plan {
            fn row_set(&mut self) -> RowSet<'_, $owner> {
                RowSet {
                    draw_list: &mut self.draws,
                    owners: &mut self.owners,
                    revisions: &mut self.revisions,
                    revision: &mut self.plan_rev,
                }
            }

            /// Seats each draw's lighting from `table`. A draw whose lookup
            /// failed goes, with its owner, and its object id is returned.
            pub fn finalize_illum(&mut self, table: &ResolvedModelIllumTable) -> Vec<u16> {
                self.row_set().seat_lighting(table)
            }
        }
    };
}

/// Lets a plan whose rows are rebuilt each frame take the new rows and judge
/// by itself whether anything moved, so no consumer re-hashes them.
macro_rules! frame_rows {
    ($plan:ty, $owner:ty) => {
        impl $plan {
            pub fn publish_frame_rows(
                &mut self,
                frame_draws: &mut Vec<FigureFacetDraw>,
                frame_owners: &mut Vec<$owner>,
            ) {
                self.row_set().take_frame(frame_draws, frame_owners)
            }

            /// For a frame in which the producer found nothing. Returning
            /// early would leave the previous frame's rows standing under the
            /// new revision, so emptying them is counted as a change - once.
            pub fn publish_no_rows(&mut self) {
                self.row_set().take_nothing()
            }
        }
    };
}

draw_plan!(
    OtherBodyPaintPlan, draw: OtherBodyFacetRun,
    /// Vertices the packed rows hold; bodies keep no float copy.
    pub(crate) decoded_tally: usize,
    pub prev_pack_id: Option<u64>,
    pub prev_draw_id: Option<u64>,
);

draw_plan!(
    ScriptedModelPaintPlan, draw: FigureFacetDraw,
    pub(crate) assets: Vec<ScriptPropMeshPaint>,
    pub(crate) owners: Vec<ScriptPropOwnerRun>,
);

draw_plan!(
    ThrownPaintPlan, draw: FigureFacetDraw,
    pub(crate) owners: Vec<ModelOwnerPaint>,
);

draw_plan!(
    PickupPaintPlan, draw: FigureFacetDraw,
    pub(crate) assets: Vec<PickupMeshPaint>,
    pub(crate) owners: Vec<ModelOwnerPaint>,
);

draw_plan!(
    DynEntPaintPlan, draw: FigureFacetDraw,
    pub(crate) store: Vec<PropMeshPaint>,
    pub(crate) owners: Vec<ModelOwnerPaint>,
);

owned_rows!(ScriptedModelPaintPlan, ScriptPropOwnerRun);
owned_rows!(ThrownPaintPlan, ModelOwnerPaint);
owned_rows!(PickupPaintPlan, ModelOwnerPaint);
owned_rows!(DynEntPaintPlan, ModelOwnerPaint);

frame_rows!(ScriptedModelPaintPlan, ScriptPropOwnerRun);
frame_rows!(PickupPaintPlan, ModelOwnerPaint);
frame_rows!(DynEntPaintPlan, ModelOwnerPaint);

decoded_view!(OtherBodyPaintPlan);
decoded_view!(FirstPersonPaintPlan);

impl OtherBodyPaintPlan {
    /// Withdraws everything published. That is a change like any other: a
    /// consumer holding the old revision would otherwise keep drawing bodies
    /// that are gone.
    pub fn drop_mesh(&mut self) {
        let Self {
            verts: vertices,
            indices,
            facet_ranges,
            mat_list,
            draws,
            compact_vertices,
            plan_rev: revision,
            revisions,
            decoded_tally,
            prev_pack_id,
            prev_draw_id,
            ..
        } = self;
        let had_rows = *decoded_tally > 0 || !draws.is_empty();
        *compact_vertices = body_packed_absent();
        draws.clear();
        vertices.truncate(0);
        mat_list.clear();
        indices.truncate(0);
        facet_ranges.clear();
        *prev_pack_id = None;
        *prev_draw_id = None;
        *decoded_tally = 0;
        if had_rows {
            revisions.stamp_topology(&[], &[], 0);
            revisions.bump_surfaces();
            for bump in [InputChangeCounters::advance_acceptance_stamp, InputChangeCounters::advance_drawlist_stamp] {
                bump(revisions);
            }
            *revision = next_revision(*revision);
        }
    }
}

fn body_packed_absent() -> PackedVertexPayload {
    PackedVertexPayload::Unavailable {
        origin_layout: BODY_VERTS_DROPPED_NOTE,
    }
}

impl ScriptedModelPaintPlan {
    /// Index of the asset already held for this key, draw state and detail_tiers.
    pub fn draw_asset_slot(
        &self,
        key: &LevelModelAssetId,
        puppet_state: &xmodel_runtime::RigPoseRecord,
        viewer_detail_levels: &[Option<u8>],
    ) -> Option<usize> {
        let agrees = |held: &ScriptPropMeshPaint| {
            held.viewer_detail_levels == viewer_detail_levels && held.key == *key && held.puppet_state == *puppet_state
        };
        self.assets.iter().position(agrees)
    }
}

impl DynEntPaintPlan {
    pub fn asset_for(&self, key: &LevelModelAssetId, view_lod: Option<u8>) -> Option<&PropMeshPaint> {
        self.store
            .iter()
            .find(|held| held.view_lod == view_lod && held.key == *key)
    }
}

impl ThrownPaintPlan {
    /// Empties a staging plan before the next rebuild, keeping its capacity.
    pub fn clear_rebuild(&mut self) {
        let Self {
            verts: vertices,
            indices,
            facet_ranges,
            mat_list,
            compact_vertices,
            ..
        } = self;
        mat_list.clear();
        vertices.truncate(0);
        facet_ranges.clear();
        indices.truncate(0);
        *compact_vertices = missile_packed_absent();
    }

    /// Takes a rebuilt plan and counts what changed. Vertex bytes are not
    /// compared: missiles are posed through `world_from_local`, so a different
    /// mesh always shows in the index layout, ranges, materials or owners.
    pub fn publish_rebuild(
        &mut self,
        staged: &mut Self,
        new_draws: &mut Vec<FigureFacetDraw>,
        new_owners: &mut Vec<ModelOwnerPaint>,
    ) {
        // Every comparison also hands over its rows, so none may be skipped.
        let moved = [
            publish_rows(&mut self.mat_list, &mut staged.mat_list),
            publish_rows(&mut self.indices, &mut staged.indices),
            publish_rows(&mut self.facet_ranges, &mut staged.facet_ranges),
        ];
        let materials_moved = moved[0];
        let relaid = moved.contains(&true) || self.verts.len() != staged.verts.len();

        if materials_moved {
            self.revisions.bump_materials();
        }
        if relaid {
            self.adopt_staged_vertices(staged);
        }
        staged.verts.clear();

        let rows_moved = self.row_set().exchange(new_draws, new_owners);
        if relaid || rows_moved {
            self.row_set().count_change();
        }
    }

    fn adopt_staged_vertices(&mut self, staged: &mut Self) {
        std::mem::swap(&mut self.verts, &mut staged.verts);
        let fresh_packed = std::mem::replace(&mut staged.compact_vertices, missile_packed_absent());
        self.compact_vertices = fresh_packed;
        self.revisions.advance_vertexdata_stamp();
        self.revisions
            .stamp_topology(&self.indices, &self.facet_ranges, self.verts.len());
    }
}

fn missile_packed_absent() -> PackedVertexPayload {
    PackedVertexPayload::Unavailable {
        origin_layout: crate::FIGURE_COMPACT_UNAVAILABLE,
    }
}

/// Fingerprint of an index layout; consumers compare two to see whether the
/// draw ranges moved.
pub fn ix_layout_digest(indices: &[u32], spans: &[(u32, u32)], decoded_tally: usize) -> u64 {
    let fill = |target: &mut InputChangeCounters| target.stamp_topology(indices, spans, decoded_tally);
    let mut scratch = InputChangeCounters::default();
    fill(&mut scratch);
    scratch.topology
}

/// Notes a plan's new topology. A different one is a change of the draws, and
/// the plan's revision, returned, advances with it.
pub fn bump_plan_mesh_rev(revisions: &mut InputChangeCounters, prior_rev: u64, new_topology: u64) -> u64 {
    if revisions.topology != new_topology {
        let _previous = std::mem::replace(&mut revisions.topology, new_topology);
        InputChangeCounters::advance_drawlist_stamp(revisions);
        return next_revision(prior_rev);
    }
    prior_rev
}

/// The revision after `revision`; the counter wraps rather than overflows.
fn next_revision(revision: u64) -> u64 {
    revision.overflowing_add(1).0
}

/// One frame of the first-person gun and arms for the fallback draw: skinned
/// packed rows in the legacy layout, local space, the indices, and for each
/// draw its first index, index count and the material's catalog index.
pub struct FpvFallbackRows<'a> {
    pub rows: &'a [[u8; T6_PACKED_VERTEX]],
    pub indices: &'a [u32],
    pub draws: Vec<(u32, u32, Option<u32>)>,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct FirstPersonPaintPlan {
    /// Vertices the packed rows hold.
    pub(crate) decoded_tally: usize,
    pub(crate) indices: Vec<u32>,
    pub(crate) facet_ranges: Vec<(u32, u32)>,
    pub(crate) mat_list: Vec<PropPassMaterial>,
    pub(crate) draws: Vec<GunSurfaceBatch>,
    pub(crate) compact_vertices: PackedVertexPayload,

    pub model_to_world: Mat4,
    pub illum_handle: u32,
    pub visible: bool,

    pub hand_batches_planned: Option<u32>,
    pub gun_batches_planned: Option<u32>,
    pub gun_tint_skipped: Option<u32>,
    pub gun_index_skipped: Option<u32>,
    pub gun_tint_skip_list: Option<String>,
    pub scope_batches: Option<u32>,
    pub scope_body_batches: Option<u32>,
    pub scope_glass_batches: Option<u32>,
    pub batches_drawn: Option<u32>,
    pub batch_material_notes: Option<String>,
    pub batches_skipped: Option<u32>,
    pub viewmodel_pass_flag: Option<i32>,

    pub stage_light_index: u8,
    pub env_probe_index: u8,

    pub plan_rev: u64,
    pub generation: u64,
    pub revisions: InputChangeCounters,

    /// Whether the placement matrix is valid this frame. Placement and
    /// geometry come from separate systems that each report their own half,
    /// and `settle_visible` joins them.
    pub placement_ok: bool,

    /// Whether the rig skinned rows into this plan this frame.
    pub geometry_ok: bool,

    /// The prepared rig the rows came from. A rig publishes its rows once; a
    /// frame still answering this generation publishes vertices.
    pub rig_generation: u64,
}

published_views!(FirstPersonPaintPlan, GunSurfaceBatch);

impl FirstPersonPaintPlan {
    /// The skinned rows as `FpvFallbackRows`; `None` while this frame has none.
    pub fn fallback_rows(&self) -> Option<FpvFallbackRows<'_>> {
        let PackedVertexPayload::T6(rows) = &self.compact_vertices else {
            return None;
        };
        if self.indices.is_empty() || rows.is_empty() {
            return None;
        }
        let draws = self
            .draws
            .iter()
            .filter_map(|draw| {
                let &(start_ix, count) = self.facet_ranges.get(draw.surf as usize)?;
                let slot = draw.material as usize;
                let catalogued = self.mat_list.get(slot).and_then(|held| held.t6_material);
                Some((start_ix, count, catalogued))
            })
            .collect::<_>();
        Some(FpvFallbackRows {
            rows,
            indices: &self.indices,
            draws,
        })
    }

    /// The rows to skin into this frame; `None` when nothing packed exists,
    /// for an empty plan or models that never carried packed vertices.
    pub fn packed_rows_mut(&mut self) -> Option<&mut [[u8; T6_PACKED_VERTEX]]> {
        let PackedVertexPayload::T6(rows) = &mut self.compact_vertices else {
            return None;
        };
        Some(rows.as_mut_slice())
    }

    /// Whether the viewmodel may be coloured: shown, lit, and not switched off.
    #[must_use]
    pub fn allows_tint(&self) -> bool {
        let switched_off = self.viewmodel_pass_flag == Some(0);
        self.visible && self.illum_handle != 0 && !switched_off
    }

    /// Visible only when both halves hold: rows nobody can place draw no more
    /// than a placement with nothing to draw.
    pub fn settle_visible(&mut self) {
        let wanted = self.viewmodel_pass_flag != Some(0);
        self.visible = wanted && self.geometry_ok && self.placement_ok;
    }

    /// Takes the eye's lighting, or hides the viewmodel if it has none.
    pub fn finalize_illum(&mut self, table: &ResolvedModelIllumTable) {
        let before = self.lit_state();
        let eye = match table.answer_for(ModelIllumOwner::Eye) {
            Some(ResolvedModelIllum::Seated {
                handle: hnd,
                stage_light_index: light,
                env_probe_index: probe,
                ..
            }) => Some((hnd, light, probe)),
            _ => None,
        };
        if let Some((handle, light, probe)) = eye {
            (self.illum_handle, self.stage_light_index, self.env_probe_index) = (handle, light, probe);
            if !self.draws.is_empty() {
                self.settle_visible();
            }
        } else {
            self.visible = false;
            (self.illum_handle, self.stage_light_index, self.env_probe_index) = (0, 0, 0);
        }
        let after = self.lit_state();
        let revs = &mut self.revisions;
        if before.0 != after.0 {
            revs.advance_acceptance_stamp();
        }
        if before != after {
            revs.advance_drawlist_stamp();
        }
    }

    /// What `finalize_illum` compares across a frame: visibility first,
    /// then the lighting the eye seated.
    fn lit_state(&self) -> (bool, u32, u8, u8) {
        (
            self.visible,
            self.illum_handle,
            self.stage_light_index,
            self.env_probe_index,
        )
    }
}

/// A pass material with nothing bound, for tests in this crate that only
/// care about how many there are and in which order.
#[cfg(test)]
pub(crate) fn plain_test_material(sort_key: u8) -> PropPassMaterial {
    let zeros = [0.0; 4];
    PropPassMaterial {
        order_key: sort_key,
        t6_material: None,
        material_draw_rank_slot: None,
        atlas_find: zeros,
        illum_lookup_scale: zeros,
        env_map_params: zeros,
        face_cull: None, paint_mode: None, see_through: AlphaMode::default(),
        atlas: None, probe: None, specular: None, color: None,
        model_illum_required: false,
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use render_scene::ModelIllumRequest;

    /// Rows and tables the tests below build their plans from.
    mod fixture {
        use crate::strokes::tests::*;

        /// A plan, or any resource, with nothing in it yet.
        pub fn blank<P: Default>() -> P {
            P::default()
        }

        pub fn ask(owner: ModelIllumOwner) -> ModelIllumRequest {
            ModelIllumRequest {
                lookup_backup: 0,
                origin: [0.0; 3],
                owner,
            }
        }

        pub fn draw(object_tag: u16, owner: Option<ModelIllumOwner>) -> FigureFacetDraw {
            let queued_illum = owner.map(ask);
            let model_matrix = Mat4::IDENTITY;
            FigureFacetDraw {
                object_tag,
                queued_illum,
                map_from_self: model_matrix,
                is_scoped_view: false,
                caster_bound: Default::default(),
                stage_actor_num: Default::default(),
                compact_illum: Default::default(),
                env_probe_index: Default::default(),
                stage_light_index: Default::default(),
                colour_refusal: Default::default(),
                illum_handle: Default::default(),
                material: Default::default(),
                surface: Default::default(),
            }
        }

        pub fn owner(object_tag: u16) -> ModelOwnerPaint {
            ModelOwnerPaint {
                object_tag,
                xmodel: format!("owner-{object_tag}"),
            }
        }

        pub fn seated(handle: u32) -> ResolvedModelIllum {
            let compact_illum = Some([1, 2, 3, 4]);
            ResolvedModelIllum::Seated {
                handle,
                stage_light_index: 3,
                env_probe_index: 4,
                compact_illum,
            }
        }

        pub fn table_of(entries: &[(ModelIllumOwner, ResolvedModelIllum)]) -> ResolvedModelIllumTable {
            entries.iter().fold(blank::<ResolvedModelIllumTable>(), |mut table, (owner, lighting)| {
                table.keep_first(*owner, lighting.clone());
                table
            })
        }

        pub fn payload_says(payload: &PackedVertexPayload, reason_in: &str) -> bool {
            matches!(payload, PackedVertexPayload::Unavailable { origin_layout } if *origin_layout == reason_in)
        }

        /// `(admission, draws)`: the two counters lighting moves.
        pub fn lit_counters(revisions: &InputChangeCounters) -> (u64, u64) {
            (revisions.admission, revisions.draws)
        }
    }
    use fixture::*;

    mod topology {
        use crate::strokes::tests::*;

        #[test]
        fn every_input_shows_in_the_fingerprint() {
            let reference = ix_layout_digest(&[0, 1, 2], &[(0, 3)], 3);
            let variants = [
                (&[0u32, 2, 1][..], &[(0u32, 3u32)][..], 3usize),
                (&[0, 1, 2], &[(0, 2)], 3),
                (&[0, 1, 2], &[(0, 3)], 4),
            ];
            assert_eq!(reference, ix_layout_digest(&[0, 1, 2], &[(0, 3)], 3));
            for (indices, ranges, count) in variants {
                assert_ne!(reference, ix_layout_digest(indices, ranges, count));
            }
        }

        #[test]
        fn stamping_the_topology_already_held_is_free() {
            let mut revisions_in = InputChangeCounters {
                topology: 9,
                ..Default::default()
            };
            let before = revisions_in;
            let revision = bump_plan_mesh_rev(&mut revisions_in, 5, 9);
            assert_eq!((revision, revisions_in), (5, before));
        }

        #[test]
        fn stamping_another_topology_advances_and_wraps() {
            let mut revisions_v: InputChangeCounters = blank();
            let first = bump_plan_mesh_rev(&mut revisions_v, 5, 9);
            assert_eq!((first, revisions_v.topology, revisions_v.draws), (6, 9, 1));
            let wrapped = bump_plan_mesh_rev(&mut revisions_v, u64::MAX, 10);
            assert_eq!(wrapped, 0);
        }
    }

    mod body_plan {
        use crate::strokes::tests::*;

        #[test]
        fn clearing_a_plan_that_never_held_rows_is_silent() {
            let mut plan = OtherBodyPaintPlan::default();
            plan.drop_mesh();
            assert_eq!((plan.plan_rev, plan.revisions), (0, InputChangeCounters::default()));
            assert!(payload_says(plan.compact_vertices(), BODY_VERTS_DROPPED_NOTE));
        }

        #[test]
        fn clearing_a_plan_that_held_rows_is_reported_once() {
            let mut plan = OtherBodyPaintPlan {
                decoded_tally: 4,
                indices: Vec::from([0, 1, 2]),
                prev_pack_id: Some(7),
                prev_draw_id: Some(8),
                ..Default::default()
            };
            plan.drop_mesh();
            let ids = (plan.prev_pack_id, plan.prev_draw_id);
            assert_eq!((plan.decoded_tally(), ids), (0, (None, None)));
            assert!(plan.indices().is_empty());
            assert_eq!(plan.plan_rev, 1);
            assert_eq!(lit_counters(&plan.revisions), (1, 1));
            assert_eq!(plan.revisions.topology, ix_layout_digest(&[], &[], 0));
        }
    }

    mod assets {
        use crate::strokes::tests::*;

        #[test]
        fn script_assets_need_key_state_and_lods_to_all_agree() {
            let state = xmodel_runtime::RigPoseRecord::neutral_pose("crate".to_owned(), 1, 1);
            let detail_tiers = vec![Some(1), None];
            let crate_key = LevelModelAssetId("crate".to_owned());
            let mut plan = ScriptedModelPaintPlan::default();
            plan.assets = vec![ScriptPropMeshPaint {
                surfs: vec![(0, 0)],
                viewer_detail_levels: detail_tiers.clone(),
                puppet_state: state.clone(),
                key: crate_key.clone(),
            }];
            let barrel = LevelModelAssetId("barrel".to_owned());
            let answers = [
                plan.draw_asset_slot(&crate_key, &state, &detail_tiers),
                plan.draw_asset_slot(&crate_key, &state, &detail_tiers[..1]),
                plan.draw_asset_slot(&barrel, &state, &detail_tiers),
            ];
            assert_eq!(answers, [Some(0), None, None]);
        }

        #[test]
        fn dynent_assets_need_key_and_lod_to_agree() {
            let door = LevelModelAssetId("door".to_owned());
            let mut plan = DynEntPaintPlan::default();
            plan.store.extend([PropMeshPaint {
                surf_pairs: vec![(1, 0)],
                view_lod: Some(2),
                key: door.clone(),
            }]);
            assert_eq!(plan.asset_for(&door, Some(2)).map(|a| a.surf_pairs.clone()), Some(vec![(1, 0)]));
            assert!(plan.asset_for(&door, None).is_none());
        }
    }

    mod frame_rows {
        use crate::strokes::tests::*;

        #[test]
        fn rows_equal_to_the_published_ones_leave_the_revision_alone() {
            let mut plan = PickupPaintPlan::default();
            for round in 0..2 {
                let (mut draws, mut owners) = (vec![draw(1, None)], vec![owner(1)]);
                plan.publish_frame_rows(&mut draws, &mut owners);
                assert!(draws.is_empty() && owners.is_empty(), "round {round}: buffers come back empty");
                assert_eq!((plan.plan_rev, plan.revisions.draws), (1, 1));
            }
        }

        #[test]
        fn a_changed_owner_alone_is_a_change() {
            let mut plan = DynEntPaintPlan::default();
            for owner_id in [1, 2] {
                plan.publish_frame_rows(&mut vec![draw(1, None)], &mut vec![owner(owner_id)]);
            }
            assert_eq!((plan.plan_rev, plan.owners.clone()), (2, vec![owner(2)]));
        }

        #[test]
        fn an_empty_frame_is_counted_the_first_time_only() {
            let mut plan = ScriptedModelPaintPlan::default();
            plan.publish_no_rows();
            assert_eq!(plan.plan_rev, 0);

            plan.draws.push(draw(1, None));
            plan.publish_no_rows();
            assert!(plan.draws().is_empty());
            assert_eq!((plan.plan_rev, lit_counters(&plan.revisions)), (1, (1, 1)));

            plan.publish_no_rows();
            assert_eq!(plan.plan_rev, 1);
        }
    }

    mod missile_rebuild {
        use crate::strokes::tests::*;

        fn staged(indices: &[u32], sort_key: u8) -> ThrownPaintPlan {
            let up = [0.0, 0.0, 1.0];
            let vertex = PropVertex { first_texcoord: [0.0; 2], color: [1.0; 4], normal: up, position: [0.0; 3] };
            ThrownPaintPlan {
                verts: vec![vertex],
                indices: indices.to_vec(),
                facet_ranges: vec![(0, indices.len() as u32)],
                mat_list: vec![plain_test_material(sort_key)],
                compact_vertices: PackedVertexPayload::T6(vec![[0; T6_PACKED_VERTEX]]),
                ..Default::default()
            }
        }

        /// Publishes `next` into `plan` with no rows and returns what is left of it.
        fn publish(plan: &mut ThrownPaintPlan, mut next: ThrownPaintPlan) -> ThrownPaintPlan {
            plan.publish_rebuild(&mut next, &mut Vec::new(), &mut Vec::new());
            next
        }

        #[test]
        fn a_rebuild_equal_to_the_published_plan_moves_nothing() {
            let mut plan = blank::<ThrownPaintPlan>();
            publish(&mut plan, staged(&[0, 0, 0], 1));
            let standing = (plan.plan_rev, plan.revisions);

            let leftover = publish(&mut plan, staged(&[0, 0, 0], 1));
            assert_eq!((plan.plan_rev, plan.revisions), standing);
            assert!(leftover.verts.is_empty());
        }

        #[test]
        fn a_new_layout_brings_vertices_topology_and_rows() {
            let mut plan = blank::<ThrownPaintPlan>();
            let mut next = staged(&[0, 0, 0], 1);
            plan.publish_rebuild(&mut next, &mut vec![draw(0x400, None)], &mut vec![owner(0x400)]);

            let packed = match plan.compact_vertices() {
                PackedVertexPayload::T6(rows) => Some(rows.len()),
                _ => None,
            };
            assert_eq!((plan.vertices().len(), packed, plan.draws().len()), (1, Some(1), 1));
            assert!(payload_says(&next.compact_vertices, crate::FIGURE_COMPACT_UNAVAILABLE));
            let revs = plan.revisions;
            assert_eq!((plan.plan_rev, revs.vertices, revs.materials), (1, 1, 1));
            assert_eq!(revs.topology, ix_layout_digest(&[0, 0, 0], &[(0, 3)], 1));
        }

        #[test]
        fn another_material_alone_moves_only_the_material_counter() {
            let mut plan = blank::<ThrownPaintPlan>();
            publish(&mut plan, staged(&[0, 0, 0], 1));
            let before = plan.revisions;
            publish(&mut plan, staged(&[0, 0, 0], 2));
            assert_eq!(plan.materials()[0].order_key, 2);
            assert_eq!((plan.revisions.materials, plan.plan_rev), (before.materials + 1, 2));
        }

        #[test]
        fn clearing_a_staging_plan_leaves_no_geometry() {
            let mut next = staged(&[0, 0, 0], 1);
            next.clear_rebuild();
            let empty = [
                next.verts.is_empty(),
                next.indices.is_empty(),
                next.facet_ranges.is_empty(),
                next.mat_list.is_empty(),
            ];
            assert_eq!(empty, [true; 4]);
            assert!(payload_says(&next.compact_vertices, crate::FIGURE_COMPACT_UNAVAILABLE));
        }
    }

    mod lighting_cases {
        use crate::strokes::tests::*;

        #[test]
        fn seating_counts_only_when_the_lighting_differs() {
            let asked = ModelIllumOwner::Item(9);
            let table = table_of(&[(asked, seated(11))]);
            let mut plan = PickupPaintPlan::default();
            plan.draws.push(draw(5, Some(asked)));
            plan.owners.push(owner(5));

            let first_pass = plan.finalize_illum(&table);
            assert_eq!(first_pass, Vec::<u16>::new());
            let row = plan.draws()[0];
            assert!(row.queued_illum.is_none());
            assert_eq!(
                (row.illum_handle, row.stage_light_index, row.env_probe_index, row.compact_illum),
                (11, 3, 4, Some([1, 2, 3, 4]))
            );
            assert_eq!(plan.revisions.draws, 1);

            plan.draws[0].queued_illum = Some(ask(asked));
            let second_pass = plan.finalize_illum(&table);
            assert!(second_pass.is_empty());
            assert_eq!(plan.revisions.draws, 1, "seating the same lighting is no change");
        }

        #[test]
        fn a_failed_lookup_takes_the_draw_and_its_owner_with_it() {
            let (kept, lost) = (ModelIllumOwner::Shell(1), ModelIllumOwner::Shell(2));
            let table = table_of(&[(kept, seated(6)), (lost, ResolvedModelIllum::Failed)]);
            let mut plan = blank::<ThrownPaintPlan>();
            plan.draws.extend([draw(1, Some(kept)), draw(2, Some(lost))]);
            plan.owners.extend([owner(1), owner(2)]);

            assert_eq!(plan.finalize_illum(&table), [2]);
            let ids: Vec<u16> = plan.draws().iter().map(|row| row.object_tag).collect();
            assert_eq!((ids, plan.owners.clone()), (vec![1], vec![owner(1)]));
            assert_eq!(lit_counters(&plan.revisions), (1, 1));
        }

        #[test]
        fn a_lookup_nobody_answered_counts_as_failed() {
            let mut plan = DynEntPaintPlan::default();
            plan.draws.push(draw(7, Some(ModelIllumOwner::Pane(1))));
            plan.owners.push(owner(7));
            let dropped = plan.finalize_illum(&table_of(&[]));
            assert_eq!((dropped, plan.draws().len()), (vec![7], 0));
        }

        #[test]
        fn script_model_owners_follow_their_dropped_draws_by_object_id() {
            let entity = Entity::from_raw_u32(3).expect("valid index");
            let mut plan = ScriptedModelPaintPlan::default();
            plan.draws.push(draw(8, Some(ModelIllumOwner::ScriptModel(entity))));
            plan.owners.extend([ScriptPropOwnerRun {
                object_tag: 8,
                active_mesh: LevelModelAssetId("crate".to_owned()),
                ent: entity,
            }]);
            assert_eq!(plan.finalize_illum(&table_of(&[])), [8]);
            assert!(plan.owners.is_empty());
        }
    }

    mod viewmodel_cases {
        use crate::strokes::tests::*;

        fn skinned() -> FirstPersonPaintPlan {
            let mut catalogued = plain_test_material(0);
            catalogued.t6_material = Some(42);
            let surface = |surface, material, is_scoped_view| GunSurfaceBatch {
                surf: surface,
                material,
                is_scoped_view,
            };
            FirstPersonPaintPlan {
                decoded_tally: 3,
                indices: Vec::from([0, 1, 2, 2, 1, 0]),
                facet_ranges: vec![(0, 3), (3, 3)],
                mat_list: vec![catalogued, plain_test_material(0)],
                draws: vec![surface(1, 0, false), surface(0, 1, true), surface(9, 0, false)],
                compact_vertices: PackedVertexPayload::T6(vec![[0; T6_PACKED_VERTEX]; 3]),
                ..Default::default()
            }
        }

        fn ready() -> FirstPersonPaintPlan {
            FirstPersonPaintPlan {
                geometry_ok: true,
                placement_ok: true,
                ..skinned()
            }
        }

        fn eye(handle: u32) -> ResolvedModelIllumTable {
            table_of(&[(ModelIllumOwner::Eye, seated(handle))])
        }

        #[test]
        fn fallback_rows_drop_draws_with_no_range_for_their_surface() {
            let plan = skinned();
            let rows = plan.fallback_rows().expect("the plan is skinned");
            assert_eq!(
                (rows.rows.len(), rows.indices.len(), rows.draws),
                (3, 6, vec![(3, 3, Some(42)), (0, 3, None)])
            );
        }

        #[test]
        fn fallback_rows_need_packed_rows_and_indices_both() {
            let mut bare = skinned();
            bare.indices.clear();
            assert!(bare.fallback_rows().is_none());

            for packed in [PackedVertexPayload::T6(Vec::new()), PackedVertexPayload::default()] {
                let mut plan = skinned();
                plan.compact_vertices = packed;
                assert!(plan.fallback_rows().is_none());
            }
        }

        #[test]
        fn packed_rows_open_for_writing_only_while_kept() {
            let mut plan = skinned();
            plan.packed_rows_mut().expect("kept")[1] = [9; T6_PACKED_VERTEX];
            let written = match &plan.compact_vertices {
                PackedVertexPayload::T6(rows) => rows[1][0],
                _ => 0,
            };
            assert_eq!(written, 9);

            plan.compact_vertices = PackedVertexPayload::default();
            assert!(plan.packed_rows_mut().is_none());
        }

        #[test]
        fn visibility_needs_rows_a_placement_and_the_gun_on() {
            let mut plan = blank::<FirstPersonPaintPlan>();
            let mut seen = Vec::new();
            for (geometry, placement, viewmodel_pass_flag) in [(false, false, None), (true, true, None), (true, true, Some(0))] {
                (plan.geometry_ok, plan.placement_ok, plan.viewmodel_pass_flag) = (geometry, placement, viewmodel_pass_flag);
                plan.settle_visible();
                seen.push(plan.visible);
            }
            assert_eq!(seen, [false, true, false]);
        }

        #[test]
        fn colour_needs_a_shown_lit_gun_that_is_not_switched_off() {
            let plan = |illum_handle, viewmodel_pass_flag| FirstPersonPaintPlan { viewmodel_pass_flag, illum_handle, visible: true, ..Default::default() };
            assert!(plan(2, None).allows_tint());
            assert!(!plan(2, Some(0)).allows_tint());
            assert!(!plan(0, Some(1)).allows_tint());
        }

        #[test]
        fn eye_lighting_shows_a_viewmodel_that_has_rows_and_a_placement() {
            let mut plan = ready();
            plan.finalize_illum(&eye(5));
            assert!(plan.visible);
            let seated_at = (plan.illum_handle, plan.stage_light_index, plan.env_probe_index);
            assert_eq!((seated_at, lit_counters(&plan.revisions)), ((5, 3, 4), (1, 1)));

            plan.finalize_illum(&eye(5));
            assert_eq!(lit_counters(&plan.revisions), (1, 1));
        }

        #[test]
        fn no_eye_lighting_hides_and_unlights_the_viewmodel() {
            let mut plan = FirstPersonPaintPlan { stage_light_index: 3, illum_handle: 5, visible: true, ..skinned() };
            plan.finalize_illum(&table_of(&[]));
            let lit = (plan.illum_handle, plan.stage_light_index, plan.env_probe_index);
            assert_eq!((plan.visible, lit), (false, (0, 0, 0)));
            assert_eq!(lit_counters(&plan.revisions), (1, 1));
        }

        #[test]
        fn a_plan_with_no_draws_keeps_its_visibility_when_lit() {
            let mut plan = FirstPersonPaintPlan { visible: true, ..blank::<FirstPersonPaintPlan>() };
            plan.finalize_illum(&eye(5));
            assert_eq!((plan.visible, plan.revisions.admission), (true, 0));
        }
    }
}
