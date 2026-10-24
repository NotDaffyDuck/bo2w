//! The merged model draw plan: the eight producers of animated and
//! script-placed models (remote bodies, the viewmodel, script models,
//! missiles, items, fx models, dynamic entities and the sky model) each keep
//! their own geometry and draws; this module lays them end to end into the
//! one set of rows the renderer takes, and only redoes the part a producer's
//! published revisions say changed.

use render_anim::geometry::install_retained_packed;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::prelude::*;

use super::sky::SkyModelDrawPlan;
use super::smodel::{PackedVertexRefusal, SmodelPassMaterial, SmodelVertex};

pub use render_anim::{
    BODY_PACKED_UNAVAILABLE, DynEntAssetDraw, DynEntDrawPlan, DynEntOwnerDraw,
    FPV_PACKED_EMPTY_PLAN, FPV_PACKED_UNAVAILABLE, FpvDrawPlan, FpvSurfaceDraw, ItemAssetDraw,
    ItemDrawPlan, ItemOwnerDraw, MissileDrawPlan, MissileOwnerDraw, RemoteBodyDrawPlan,
    RemoteBodySurfaceDraw, ScriptModelAssetDraw, ScriptModelDrawPlan, ScriptModelOwnerDraw,
    XMODEL_OBJECT_ID_BODY_BASE, XMODEL_OBJECT_ID_DYNENT_BASE, XMODEL_OBJECT_ID_ITEM_BASE,
    XMODEL_OBJECT_ID_MISSILE_BASE,
};
pub use render_anim::{
    BodyPackedSession, append_dynent_asset, append_dynent_surfaces, append_item_surfaces,
    append_missile_surfaces, append_remote_body_cpu_blob, append_script_model_asset,
    authored_lit_xmodel_pass_material, body_lit_pass_material, bound_lit_xmodel_pass_material,
    finish_remote_body_draw_plan, install_body_packed_session, overwrite_script_model_asset,
    push_remote_body_cpu_draw, retain_script_model_assets, take_body_packed_session,
};
pub use render_fx::append_fx_model_asset;

pub use render_frame::XMODEL_OBJECT_ID_VIEWMODEL;
pub use render_fx::{FxModelAssetDraw, FxModelDrawPlan, XMODEL_OBJECT_ID_FX_BASE};
pub use render_scene::{XModelColourRefusal, XModelSurfaceDraw};

type PackedRow = [u8; asset_t6::T6_PACKED_VERTEX];

pub const XMODEL_PACKED_UNAVAILABLE: &str =
    "xmodel merge GfxPackedVertex missing or count-mismatched; decoded float is not packed VB";
pub const XMODEL_PACKED_EMPTY_PLAN: &str = "xmodel plan has no vertices";

#[derive(Resource, Clone, Debug, Default)]
pub struct XModelDrawPlan {
    /// Vertices laid out so far (the merged rows' count).
    pub decoded_n: usize,
    pub indices: Vec<u32>,
    pub surface_ranges: Vec<(u32, u32)>,
    pub materials: Vec<SmodelPassMaterial>,
    pub draws: Vec<XModelSurfaceDraw>,

    /// Fx model draws dropped because the object ids ran out.
    pub fx_object_id_exhausted: u32,
    /// Bumps whenever the published packed rows change.
    pub revision: u64,
    /// Bumps whenever the published indices or ranges change.
    pub topology_revision: u64,

    pub packed_vertices: asset_world::PackedVertexPayload,
    pub packed_share: Option<Arc<Vec<PackedRow>>>,
    /// Where each owner's rows sit in the packed share.
    pub packed_segments: render_frame::PackedSegments,
    pub index_share: Option<Arc<Vec<u32>>>,
    pub range_share: Option<Arc<Vec<(u32, u32)>>>,
    /// The merge keeps no decoded float rows; the extract still asks.
    pub decoded_share: Option<Arc<Vec<SmodelVertex>>>,

    published_packed: Option<PackedContentKey>,
    packed_banks: super::ShareBanks<PackedRow>,
    index_banks: super::ShareBanks<u32>,
    range_banks: super::ShareBanks<(u32, u32)>,

    last_input: Option<Stamp>,
    last_layout: Option<LayoutKey>,
    /// True while every owner's geometry is laid out whole, so a producer's
    /// new rows can be copied over its segment in place.
    concat_layout: bool,
    last_admitted: u8,
    /// Per packed bank, the vertex revisions its rows were copied at.
    bank_vertices: [[u64; OWNERS]; super::SHARE_BANKS],
}

impl XModelDrawPlan {
    pub fn clear(&mut self) {
        self.decoded_n = 0;
        self.indices.clear();
        self.surface_ranges.clear();
        self.materials.clear();
        self.draws.clear();
        self.fx_object_id_exhausted = 0;
        self.packed_share = None;
        self.packed_segments.forget();
        self.index_share = None;
        self.range_share = None;
        self.decoded_share = None;
        self.last_input = None;
        self.last_layout = None;
        self.concat_layout = false;
        self.last_admitted = 0;
        self.bank_vertices = Default::default();
        self.packed_vertices = asset_world::PackedVertexPayload::Unavailable {
            source_layout: XMODEL_PACKED_UNAVAILABLE,
        };
    }

    pub fn packed_rows(&self) -> Option<&[PackedRow]> {
        match (&self.packed_share, &self.packed_vertices) {
            (Some(share), _) => Some(share.as_slice()),
            (None, asset_world::PackedVertexPayload::T6(rows)) => Some(rows.as_slice()),
            (None, asset_world::PackedVertexPayload::Unavailable { .. }) => None,
        }
    }

    pub fn index_rows(&self) -> &[u32] {
        super::published_rows(&self.index_share)
    }

    pub fn range_rows(&self) -> &[(u32, u32)] {
        super::published_rows(&self.range_share)
    }

    pub fn exact_packed_vertices(&self) -> Result<&[PackedRow], PackedVertexRefusal> {
        let table_stride = render_material::decl_table::stream_extent(
            render_material::decl_table::PACKED_VERTEX_TYPE,
            0,
        );
        if table_stride != Some(asset_t6::T6_PACKED_VERTEX as u16) {
            return Err(PackedVertexRefusal::StrideMismatch { table_stride });
        }
        let Some(rows) = self.packed_rows() else {
            let source_layout = match &self.packed_vertices {
                asset_world::PackedVertexPayload::Unavailable { source_layout } => *source_layout,
                asset_world::PackedVertexPayload::T6(_) => XMODEL_PACKED_UNAVAILABLE,
            };
            return Err(PackedVertexRefusal::ForeignLayout { source_layout });
        };
        if rows.len() != self.decoded_n {
            return Err(PackedVertexRefusal::VertexCountMismatch {
                packed: rows.len(),
                decoded: self.decoded_n,
            });
        }
        Ok(rows)
    }
}

// The owners, in the order their rows are laid out. An owner's bit in the
// admitted mask is `1 << order`.
const BODY: usize = 0;
const FPV: usize = 1;
const SCRIPT: usize = 2;
const MISSILE: usize = 3;
const ITEM: usize = 4;
const FX: usize = 5;
const DYNENT: usize = 6;
const SKY: usize = 7;
const OWNERS: usize = render_frame::PACKED_SEGMENT_OWNERS;

const fn bit(owner: usize) -> u8 {
    1 << owner
}

static NO_ROWS: asset_world::PackedVertexPayload = asset_world::PackedVertexPayload::Unavailable {
    source_layout: XMODEL_PACKED_UNAVAILABLE,
};

/// One producer's published geometry and revisions.
#[derive(Clone, Copy)]
struct Owner<'a> {
    generation: u64,
    revisions: render_frame::SourceRevisions,
    decoded_n: usize,
    packed: &'a asset_world::PackedVertexPayload,
    indices: &'a [u32],
    ranges: &'a [(u32, u32)],
    materials: &'a [SmodelPassMaterial],
}

impl<'a> Owner<'a> {
    fn absent() -> Self {
        Self {
            generation: 0,
            revisions: Default::default(),
            decoded_n: 0,
            packed: &NO_ROWS,
            indices: &[],
            ranges: &[],
            materials: &[],
        }
    }

    /// The packed rows, when they exist and match the vertex count.
    fn rows(&self) -> Option<&'a [PackedRow]> {
        match self.packed {
            asset_world::PackedVertexPayload::T6(rows) if rows.len() == self.decoded_n => {
                Some(rows.as_slice())
            }
            _ => None,
        }
    }

    fn packed_len(&self) -> usize {
        match self.packed {
            asset_world::PackedVertexPayload::T6(rows) => rows.len(),
            asset_world::PackedVertexPayload::Unavailable { .. } => 0,
        }
    }
}

/// Everything one merge reads.
struct Producers<'a> {
    fpv: &'a FpvDrawPlan,
    bodies: &'a RemoteBodyDrawPlan,
    scripts: &'a ScriptModelDrawPlan,
    missiles: &'a MissileDrawPlan,
    items: &'a ItemDrawPlan,
    fx_models: &'a FxModelDrawPlan,
    dynents: &'a DynEntDrawPlan,
    scene: Option<&'a crate::GfxScene>,
    sky: Option<(&'a SkyModelDrawPlan, Vec3)>,
}

impl<'a> Producers<'a> {
    fn owner(&self, which: usize) -> Owner<'a> {
        macro_rules! owner {
            ($plan:expr, $decoded_n:expr) => {
                Owner {
                    generation: $plan.generation,
                    revisions: $plan.revisions,
                    decoded_n: $decoded_n,
                    packed: $plan.packed_vertices(),
                    indices: $plan.indices(),
                    ranges: $plan.surface_ranges(),
                    materials: $plan.materials(),
                }
            };
        }
        match which {
            BODY => owner!(self.bodies, self.bodies.decoded_n()),
            FPV => owner!(self.fpv, self.fpv.decoded_n()),
            SCRIPT => owner!(self.scripts, self.scripts.vertices().len()),
            MISSILE => owner!(self.missiles, self.missiles.vertices().len()),
            ITEM => owner!(self.items, self.items.vertices().len()),
            FX => owner!(self.fx_models, self.fx_models.vertices().len()),
            DYNENT => owner!(self.dynents, self.dynents.vertices().len()),
            _ => match self.sky {
                Some((sky, _)) => Owner {
                    generation: sky.generation,
                    revisions: sky.revisions,
                    decoded_n: sky.geometry.decoded_vertices().len(),
                    packed: &sky.geometry.packed_vertices,
                    indices: sky.geometry.indices(),
                    ranges: sky.geometry.surface_ranges(),
                    materials: &sky.geometry.materials,
                },
                None => Owner::absent(),
            },
        }
    }

    fn owners(&self) -> [Owner<'a>; OWNERS] {
        std::array::from_fn(|which| self.owner(which))
    }

    /// Whether the scene lets a draw of this entity through this frame.
    fn admits(&self, entnum: Option<u32>) -> bool {
        match entnum {
            None => true,
            Some(n) => {
                self.scene
                    .is_some_and(|s| s.scene_ent_live(n) && !s.scene_ent_skips_draw(n))
            }
        }
    }

    fn fpv_colour(&self) -> bool {
        self.fpv.admits_colour()
            && self
                .scene
                .is_some_and(|s| s.scene_dobj_live(crate::SCENE_VIEWMODEL_ENTNUM))
    }

    /// The scene's say on which entities draw, folded into one number.
    fn occupancy(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.fpv_colour().hash(&mut hasher);
        let entnums = self
            .bodies
            .draws()
            .iter()
            .map(|d| d.scene_entnum)
            .chain(self.scripts.draws().iter().map(|d| d.scene_entnum))
            .chain(self.missiles.draws().iter().map(|d| d.scene_entnum))
            .chain(self.items.draws().iter().map(|d| d.scene_entnum));
        for entnum in entnums {
            self.admits(entnum).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn stamp(&self, owners: &[Owner<'_>; OWNERS]) -> Stamp {
        let column = |field: fn(&Owner<'_>) -> u64| -> [u64; OWNERS] {
            std::array::from_fn(|which| field(&owners[which]))
        };
        Stamp {
            layout: LayoutKey {
                generation: column(|o| o.generation),
                topology: column(|o| o.revisions.topology),
                admission: column(|o| o.revisions.admission),
                occupancy: self.occupancy(),
            },
            vertices: column(|o| o.revisions.vertices),
            materials: column(|o| o.revisions.materials),
            draws: column(|o| o.revisions.draws),
            fpv: FpvStamp {
                world: self.fpv.world_from_local.to_cols_array().map(f32::to_bits),
                lighting: self.fpv.lighting_handle,
                scene_light: self.fpv.scene_light_index,
                probe: self.fpv.reflection_probe_index,
                visible: self.fpv.visible,
                drawgun: self.fpv.drawgun,
                colour: self.fpv_colour(),
            },
            sky_eye: self.sky.map(|(_, eye)| eye.to_array().map(f32::to_bits)),
        }
    }

    /// Each owner's draws that go out this frame, ready to merge, and how
    /// many fx draws ran out of object ids.
    fn draws(&self) -> ([Vec<XModelSurfaceDraw>; OWNERS], u32) {
        let mut out: [Vec<XModelSurfaceDraw>; OWNERS] = Default::default();
        let mut next_body_object = XMODEL_OBJECT_ID_BODY_BASE;
        out[BODY] = self
            .bodies
            .draws()
            .iter()
            .filter(|d| self.admits(d.scene_entnum))
            .map(|d| {
                let object_id = next_body_object;
                next_body_object = next_body_object.saturating_add(1);
                XModelSurfaceDraw {
                    surface: d.surface,
                    material: d.material,
                    world_from_local: d.world_from_local,
                    lighting_handle: d.lighting_handle,
                    pending_lighting: None,
                    colour_refusal: None,
                    object_id,
                    scene_light_index: d.scene_light_index,
                    reflection_probe_index: d.reflection_probe_index,
                    packed_lighting: None,
                    is_scope: false,
                    scene_entnum: d.scene_entnum,
                    // Remote bodies carry no radius here; unbounded keeps
                    // every player caster in both partitions.
                    caster_bound: None,
                }
            })
            .collect();
        if self.fpv_colour() {
            let fpv = self.fpv;
            out[FPV] = fpv
                .draws()
                .iter()
                .map(|d| XModelSurfaceDraw {
                    surface: d.surface,
                    material: d.material,
                    world_from_local: fpv.world_from_local,
                    lighting_handle: fpv.lighting_handle,
                    pending_lighting: None,
                    colour_refusal: None,
                    object_id: XMODEL_OBJECT_ID_VIEWMODEL,
                    scene_light_index: fpv.scene_light_index,
                    reflection_probe_index: fpv.reflection_probe_index,
                    packed_lighting: None,
                    is_scope: d.is_scope,
                    scene_entnum: Some(crate::SCENE_VIEWMODEL_ENTNUM),
                    caster_bound: None,
                })
                .collect();
        }
        let admitted = |draws: &[XModelSurfaceDraw]| -> Vec<XModelSurfaceDraw> {
            draws
                .iter()
                .copied()
                .filter(|d| self.admits(d.scene_entnum))
                .collect()
        };
        out[SCRIPT] = admitted(self.scripts.draws());
        out[MISSILE] = admitted(self.missiles.draws());
        out[ITEM] = admitted(self.items.draws());
        let (fx, exhausted) = dense_fx_object_ids(
            self.fx_models.draws(),
            self.dynents.draws().iter().map(|d| d.object_id).max(),
        );
        out[FX] = fx;
        out[DYNENT] = self.dynents.draws().to_vec();
        if let Some((sky, eye)) = self.sky {
            out[SKY] = sky
                .draws
                .iter()
                .map(|d| XModelSurfaceDraw {
                    world_from_local: Mat4::from_translation(eye),
                    ..*d
                })
                .collect();
        }
        (out, exhausted)
    }
}

/// Which owners have draws, as a mask.
fn admitted_mask(draws: &[Vec<XModelSurfaceDraw>; OWNERS]) -> u8 {
    draws
        .iter()
        .enumerate()
        .filter(|(_, owner_draws)| !owner_draws.is_empty())
        .fold(0, |mask, (which, _)| mask | bit(which))
}

/// What decides the layout of the merged rows: when it is the same, the
/// geometry only needs refreshing in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayoutKey {
    generation: [u64; OWNERS],
    topology: [u64; OWNERS],
    admission: [u64; OWNERS],
    occupancy: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FpvStamp {
    world: [u32; 16],
    lighting: u32,
    scene_light: u8,
    probe: u8,
    visible: bool,
    drawgun: Option<i32>,
    colour: bool,
}

/// Everything one merge's output depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    layout: LayoutKey,
    vertices: [u64; OWNERS],
    materials: [u64; OWNERS],
    draws: [u64; OWNERS],
    fpv: FpvStamp,
    sky_eye: Option<[u32; 3]>,
}

impl Stamp {
    /// Whether the draw and material rows differ from `then`'s.
    fn draws_differ(&self, then: &Stamp) -> bool {
        self.draws != then.draws
            || self.materials != then.materials
            || self.fpv != then.fpv
            || self.sky_eye != then.sky_eye
    }
}

/// What the published packed rows were built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PackedContentKey {
    layout: LayoutKey,
    admitted: u8,
    rows: usize,
    vertices: [u64; OWNERS],
}

impl PackedContentKey {
    fn new(stamp: &Stamp, admitted: u8, rows: usize) -> Self {
        Self {
            layout: stamp.layout,
            admitted,
            rows,
            vertices: stamp.vertices,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn merge_xmodel_draw_plan(
    merged: &mut XModelDrawPlan,
    fpv: &FpvDrawPlan,
    bodies: &RemoteBodyDrawPlan,
    scripts: &ScriptModelDrawPlan,
    missiles: &MissileDrawPlan,
    items: &ItemDrawPlan,
    fx_models: &FxModelDrawPlan,
    dynents: &DynEntDrawPlan,
    scene: Option<&crate::GfxScene>,
    sky: Option<(&SkyModelDrawPlan, Vec3)>,
) {
    let producers = Producers {
        fpv,
        bodies,
        scripts,
        missiles,
        items,
        fx_models,
        dynents,
        scene,
        sky,
    };
    let owners = producers.owners();
    let stamp = producers.stamp(&owners);
    if merged.last_input == Some(stamp) {
        return;
    }
    if merged.concat_layout && merged.last_layout == Some(stamp.layout) {
        let then = merged.last_input;
        if then.is_none_or(|then| then.vertices != stamp.vertices) {
            merged.recopy_packed(&owners, &stamp);
        }
        if then.is_none_or(|then| stamp.draws_differ(&then)) {
            merged.refresh_draws(&producers, &owners);
        }
        merged.last_input = Some(stamp);
        return;
    }
    merged.relayout(&producers, &owners, stamp);
}

impl XModelDrawPlan {
    /// Lays every owner out again from scratch.
    fn relayout(&mut self, producers: &Producers<'_>, owners: &[Owner<'_>; OWNERS], stamp: Stamp) {
        let prev_packed = self.packed_share.take();
        let prev_index = self.index_share.take();
        let prev_range = self.range_share.take();
        let mut packed = self.packed_banks.take_write();
        let write = self.packed_banks.write_index();
        let indices = self.index_banks.take_write();
        let ranges = self.range_banks.take_write();
        self.clear();
        self.indices = indices;
        self.surface_ranges = ranges;
        self.concat_layout = true;

        // The sky's rows were never counted here; the reserve is a hint.
        packed.reserve(owners[..SKY].iter().map(Owner::packed_len).sum());
        let (mut draws, exhausted) = producers.draws();
        self.fx_object_id_exhausted = exhausted;
        self.last_admitted = admitted_mask(&draws);
        let mut packed_ok = true;
        for (owner, owner_draws) in owners.iter().zip(&mut draws) {
            if !owner_draws.is_empty() {
                self.append_owner(&mut packed, &mut packed_ok, owner, std::mem::take(owner_draws));
            }
        }
        if packed_ok {
            self.bank_vertices[write] = stamp.vertices;
        }
        let key = packed_ok.then(|| PackedContentKey::new(&stamp, self.last_admitted, packed.len()));
        let payload = install_retained_packed(
            packed_ok,
            packed,
            self.decoded_n,
            XMODEL_PACKED_EMPTY_PLAN,
            XMODEL_PACKED_UNAVAILABLE,
        );
        self.publish_packed(prev_packed, payload, key);
        self.publish_topology(prev_index, prev_range);
        self.last_input = Some(stamp);
        self.last_layout = Some(stamp.layout);
    }

    /// Copies the owners' changed packed rows over their segments.
    fn recopy_packed(&mut self, owners: &[Owner<'_>; OWNERS], stamp: &Stamp) {
        let prev_packed = self.packed_share.take();
        let (mut packed, fresh) = self.packed_banks.take_write_keep();
        let write = self.packed_banks.write_index();
        if fresh {
            self.bank_vertices[write] = [0; OWNERS];
        }
        let ok = recopy_concat_packed(
            &mut packed,
            fresh,
            &mut self.bank_vertices[write],
            stamp.vertices,
            self.last_admitted,
            owners,
            &mut self.packed_segments,
        );
        let key = ok.then(|| PackedContentKey::new(stamp, self.last_admitted, packed.len()));
        let payload = install_retained_packed(
            ok,
            packed,
            self.decoded_n,
            XMODEL_PACKED_EMPTY_PLAN,
            XMODEL_PACKED_UNAVAILABLE,
        );
        self.publish_packed(prev_packed, payload, key);
    }

    /// Rebuilds the draw and material rows over an unchanged layout.
    fn refresh_draws(&mut self, producers: &Producers<'_>, owners: &[Owner<'_>; OWNERS]) {
        self.draws.clear();
        self.materials.clear();
        let (draws, exhausted) = producers.draws();
        self.fx_object_id_exhausted = exhausted;
        let mut range_base = 0u32;
        for (owner, owner_draws) in owners.iter().zip(draws) {
            if owner_draws.is_empty() {
                continue;
            }
            self.push_draws(range_base, owner.materials, owner_draws);
            range_base = range_base.saturating_add(owner.ranges.len() as u32);
        }
    }

    /// Adds an owner's materials and its draws, moved to the merged rows.
    fn push_draws(
        &mut self,
        range_base: u32,
        materials: &[SmodelPassMaterial],
        draws: Vec<XModelSurfaceDraw>,
    ) {
        let material_base = self.materials.len() as u32;
        self.materials.extend_from_slice(materials);
        self.draws.extend(draws.into_iter().map(|d| XModelSurfaceDraw {
            surface: d.surface.saturating_add(range_base),
            material: d.material.saturating_add(material_base),
            ..d
        }));
    }

    /// Adds one owner: whole when every surface is drawn, else only the
    /// surfaces and vertices its draws use.
    fn append_owner(
        &mut self,
        packed: &mut Vec<PackedRow>,
        packed_ok: &mut bool,
        owner: &Owner<'_>,
        draws: Vec<XModelSurfaceDraw>,
    ) {
        let mut used = vec![false; owner.ranges.len()];
        for draw in &draws {
            if let Some(slot) = used.get_mut(draw.surface as usize) {
                *slot = true;
            }
        }
        if used.iter().all(|&live| live) && !used.is_empty() {
            self.append_whole(owner, draws);
            append_packed(packed, packed_ok, owner);
        } else {
            self.append_used(packed, packed_ok, owner, &used, draws);
        }
    }

    fn append_whole(&mut self, owner: &Owner<'_>, draws: Vec<XModelSurfaceDraw>) {
        let vertex_base = self.decoded_n as u32;
        let index_base = self.indices.len() as u32;
        let range_base = self.surface_ranges.len() as u32;
        self.decoded_n = self.decoded_n.saturating_add(owner.decoded_n);
        self.indices
            .extend(owner.indices.iter().map(|i| i.saturating_add(vertex_base)));
        self.surface_ranges.extend(
            owner
                .ranges
                .iter()
                .map(|&(start, count)| (start.saturating_add(index_base), count)),
        );
        self.push_draws(range_base, owner.materials, draws);
    }

    /// Adds only the surfaces in `used`, renumbering their vertices and the
    /// materials the draws name. The layout is no longer whole, so later
    /// frames take the full path.
    fn append_used(
        &mut self,
        packed: &mut Vec<PackedRow>,
        packed_ok: &mut bool,
        owner: &Owner<'_>,
        used: &[bool],
        mut draws: Vec<XModelSurfaceDraw>,
    ) {
        const UNMAPPED: u32 = u32::MAX;
        let source_rows = owner.rows();
        if source_rows.is_none() && owner.decoded_n != 0 {
            *packed_ok = false;
            packed.clear();
        }
        let vertex_base = self.decoded_n as u32;
        let mut new_vertices = 0u32;
        let mut vertex_at = vec![UNMAPPED; owner.decoded_n];
        let mut surface_at = vec![UNMAPPED; owner.ranges.len()];
        for (surface, &(start, count)) in owner.ranges.iter().enumerate() {
            if !used[surface] {
                continue;
            }
            let start = start as usize;
            let Some(span) = owner.indices.get(start..start.saturating_add(count as usize)) else {
                continue;
            };
            let new_start = self.indices.len() as u32;
            for &old in span {
                let Some(slot) = vertex_at.get_mut(old as usize) else {
                    continue;
                };
                if *slot == UNMAPPED {
                    *slot = vertex_base.saturating_add(new_vertices);
                    new_vertices = new_vertices.saturating_add(1);
                    if let Some(rows) = source_rows.filter(|_| *packed_ok) {
                        packed.push(rows[old as usize]);
                    }
                }
                self.indices.push(*slot);
            }
            surface_at[surface] = self.surface_ranges.len() as u32;
            self.surface_ranges.push((new_start, count));
        }

        let mut material_at = vec![UNMAPPED; owner.materials.len()];
        for draw in &draws {
            if let Some(slot) = material_at.get_mut(draw.material as usize)
                && *slot == UNMAPPED
            {
                *slot = 0;
            }
        }
        for (old, slot) in material_at.iter_mut().enumerate() {
            if *slot != UNMAPPED {
                *slot = self.materials.len() as u32;
                self.materials.push(owner.materials[old].clone());
            }
        }
        self.decoded_n = self.decoded_n.saturating_add(new_vertices as usize);
        self.concat_layout = false;
        for draw in &mut draws {
            if let Some(&at) = surface_at.get(draw.surface as usize)
                && at != UNMAPPED
            {
                draw.surface = at;
            }
            if let Some(&at) = material_at.get(draw.material as usize)
                && at != UNMAPPED
            {
                draw.material = at;
            }
        }
        self.draws.extend(draws);
    }

    /// Publishes the packed rows. When they are the same content as the
    /// ones already shared, the shared bank stays.
    fn publish_packed(
        &mut self,
        prev: Option<Arc<Vec<PackedRow>>>,
        payload: asset_world::PackedVertexPayload,
        key: Option<PackedContentKey>,
    ) {
        match payload {
            asset_world::PackedVertexPayload::T6(rows) => {
                let resident = key.is_some() && self.published_packed == key;
                self.packed_banks.put_write(rows);
                match prev.filter(|_| resident) {
                    Some(prev) => self.packed_share = Some(prev),
                    None => {
                        self.revision = self.revision.wrapping_add(1);
                        self.packed_share = Some(self.packed_banks.published());
                        self.published_packed = key;
                    }
                }
                self.packed_vertices = asset_world::PackedVertexPayload::Unavailable {
                    source_layout: XMODEL_PACKED_UNAVAILABLE,
                };
            }
            other => {
                if prev.is_some() {
                    self.revision = self.revision.wrapping_add(1);
                }
                self.packed_share = None;
                self.published_packed = None;
                self.packed_vertices = other;
            }
        }
    }

    /// Publishes the merged indices and ranges; unchanged ones keep the
    /// shares they had and the topology revision.
    fn publish_topology(
        &mut self,
        prev_index: Option<Arc<Vec<u32>>>,
        prev_range: Option<Arc<Vec<(u32, u32)>>>,
    ) {
        let same_index = prev_index.as_deref().is_some_and(|p| *p == self.indices);
        let same_range = prev_range
            .as_deref()
            .is_some_and(|p| *p == self.surface_ranges);
        self.index_banks.put_write(std::mem::take(&mut self.indices));
        self.range_banks
            .put_write(std::mem::take(&mut self.surface_ranges));
        if same_index && same_range {
            self.index_share = prev_index;
            self.range_share = prev_range;
        } else {
            self.topology_revision = self.topology_revision.wrapping_add(1);
            self.index_share = Some(self.index_banks.published());
            self.range_share = Some(self.range_banks.published());
        }
    }
}

/// Appends an owner's packed rows; any mismatch drops them all.
fn append_packed(packed: &mut Vec<PackedRow>, packed_ok: &mut bool, owner: &Owner<'_>) {
    if !*packed_ok || owner.decoded_n == 0 {
        return;
    }
    match owner.rows() {
        Some(rows) => packed.extend_from_slice(rows),
        None => {
            *packed_ok = false;
            packed.clear();
        }
    }
}

/// Brings the packed bank up to date with the owners' rows, copying in
/// place when the bank already has this layout, else rebuilding it. False
/// when some owner's rows are missing or the wrong size.
fn recopy_concat_packed(
    packed: &mut Vec<PackedRow>,
    fresh: bool,
    bank_vertices: &mut [u64; OWNERS],
    stamp_vertices: [u64; OWNERS],
    admitted: u8,
    owners: &[Owner<'_>; OWNERS],
    published: &mut render_frame::PackedSegments,
) -> bool {
    let live = |which: usize| admitted & bit(which) != 0 && owners[which].decoded_n != 0;
    let counts: [usize; OWNERS] =
        std::array::from_fn(|which| if live(which) { owners[which].packed_len() } else { 0 });
    let expected = counts.iter().fold(0usize, |sum, &n| sum.saturating_add(n));
    if fresh || packed.len() != expected {
        packed.clear();
        packed.reserve(expected);
        let mut ok = true;
        for (which, owner) in owners.iter().enumerate() {
            if admitted & bit(which) != 0 {
                append_packed(packed, &mut ok, owner);
            }
        }
        *bank_vertices = if ok { stamp_vertices } else { [0; OWNERS] };
        published.forget();
        return ok;
    }
    let mut at = 0usize;
    for (which, owner) in owners.iter().enumerate() {
        if !live(which) {
            continue;
        }
        let Some(rows) = owner.rows() else {
            packed.clear();
            *bank_vertices = [0; OWNERS];
            published.forget();
            return false;
        };
        if bank_vertices[which] != stamp_vertices[which] {
            packed[at..at + rows.len()].copy_from_slice(rows);
        }
        at = at.saturating_add(rows.len());
    }
    *bank_vertices = stamp_vertices;
    let mut start = 0u32;
    published.publish(std::array::from_fn(|which| {
        let rows = u32::try_from(counts[which]).unwrap_or(u32::MAX);
        let segment = render_frame::PackedSegment {
            start,
            rows,
            revision: stamp_vertices[which],
        };
        start = start.saturating_add(rows);
        segment
    }));
    true
}

/// Gives the fx draws object ids that are dense after the others' (`past`
/// is the highest id they use), one per source object, and counts the draws
/// that found no id left.
fn dense_fx_object_ids(
    draws: &[XModelSurfaceDraw],
    past: Option<u16>,
) -> (Vec<XModelSurfaceDraw>, u32) {
    let mut next = match past {
        Some(max_id) => max_id
            .checked_add(1)
            .map(|id| id.max(XMODEL_OBJECT_ID_FX_BASE)),
        None => Some(XMODEL_OBJECT_ID_FX_BASE),
    };
    let mut source = None;
    let mut current = None;
    let mut exhausted = 0u32;
    let mut dense = Vec::with_capacity(draws.len());
    for draw in draws {
        if source != Some(draw.object_id) {
            source = Some(draw.object_id);
            current = next;
            next = next.and_then(|id| id.checked_add(1));
        }
        match current {
            Some(object_id) => dense.push(XModelSurfaceDraw { object_id, ..*draw }),
            None => exhausted = exhausted.saturating_add(1),
        }
    }
    (dense, exhausted)
}

pub(crate) fn apply_resolved_xmodel_lighting(
    resolved: Res<crate::prepare::scene::model_lighting_cache::ResolvedModelLightingTable>,
    mut script: ResMut<ScriptModelDrawPlan>,
    mut item: ResMut<ItemDrawPlan>,
    mut missile: ResMut<MissileDrawPlan>,
    mut dynent: ResMut<DynEntDrawPlan>,
    mut fpv: ResMut<FpvDrawPlan>,
    mut focus: ResMut<crate::assemble::drawsurf::RenderFocus>,
) {
    let script = script.as_mut();
    let script_failed = script.finalize_lighting(&resolved);
    item.finalize_lighting(&resolved);
    missile.finalize_lighting(&resolved);
    dynent.finalize_lighting(&resolved);
    fpv.finalize_lighting(&resolved);
    if let Some(frame) = focus.frame.as_mut()
        && let Some(object_id) = frame.object_id
    {
        if script_failed.contains(&object_id) {
            frame.outcome = "lighting_failed";
            frame.lighting_handle = None;
        } else if let Some(draw) = script.draws().iter().find(|d| d.object_id == object_id) {
            frame.lighting_handle = Some(draw.lighting_handle);
        }
    }
}

pub(crate) fn apply_resolved_fx_model_lighting(
    resolved: Res<crate::prepare::scene::model_lighting_cache::ResolvedModelLightingTable>,
    mut plan: ResMut<FxModelDrawPlan>,
) {
    plan.finalize_lighting(&resolved);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(surface: u32, material: u32, object_id: u16) -> XModelSurfaceDraw {
        XModelSurfaceDraw {
            surface,
            material,
            world_from_local: Mat4::IDENTITY,
            lighting_handle: 0,
            pending_lighting: None,
            colour_refusal: None,
            object_id,
            scene_light_index: 0,
            reflection_probe_index: 0,
            packed_lighting: None,
            is_scope: false,
            scene_entnum: None,
            caster_bound: None,
        }
    }

    fn row(tag: u8) -> PackedRow {
        [tag; asset_t6::T6_PACKED_VERTEX]
    }

    fn material() -> SmodelPassMaterial {
        super::super::smodel::unlit_pass_material(
            super::super::smodel::AuthoredMaps {
                color: None,
                specular: None,
                alpha_mode: AlphaMode::Opaque,
                draw_mode: None,
                cull_mode: None,
                env_map_parms: [0.0; 4],
            },
            0,
            None,
        )
    }

    /// Two triangles on separate vertices, each its own surface.
    struct TwoTriangles {
        packed: asset_world::PackedVertexPayload,
        indices: Vec<u32>,
        ranges: Vec<(u32, u32)>,
        materials: Vec<SmodelPassMaterial>,
    }

    impl TwoTriangles {
        fn new() -> Self {
            Self {
                packed: asset_world::PackedVertexPayload::T6((1..=6).map(row).collect()),
                indices: vec![0, 1, 2, 3, 4, 5],
                ranges: vec![(0, 3), (3, 3)],
                materials: vec![material(); 2],
            }
        }

        fn owner(&self) -> Owner<'_> {
            Owner {
                generation: 1,
                revisions: Default::default(),
                decoded_n: 6,
                packed: &self.packed,
                indices: &self.indices,
                ranges: &self.ranges,
                materials: &self.materials,
            }
        }
    }

    #[test]
    fn fx_object_ids_follow_the_others_one_per_source_object() {
        let fx = [draw(0, 0, 7), draw(1, 0, 7), draw(2, 0, 9)];
        let (dense, exhausted) = dense_fx_object_ids(&fx, Some(XMODEL_OBJECT_ID_FX_BASE + 4));
        let ids: Vec<u16> = dense.iter().map(|d| d.object_id).collect();
        let first = XMODEL_OBJECT_ID_FX_BASE + 5;
        assert_eq!((ids, exhausted), (vec![first, first, first + 1], 0));
    }

    #[test]
    fn fx_draws_with_no_object_id_left_are_counted_and_dropped() {
        let fx = [draw(0, 0, 7), draw(1, 0, 8)];
        let (dense, exhausted) = dense_fx_object_ids(&fx, Some(u16::MAX));
        assert!(dense.is_empty());
        assert_eq!(exhausted, 2);
    }

    #[test]
    fn a_whole_owner_is_laid_after_the_one_before() {
        let (a, b) = (TwoTriangles::new(), TwoTriangles::new());
        let mut plan = XModelDrawPlan::default();
        plan.concat_layout = true;
        let mut packed = Vec::new();
        let mut ok = true;
        for owner in [a.owner(), b.owner()] {
            let draws = vec![draw(0, 0, 1), draw(1, 1, 1)];
            plan.append_owner(&mut packed, &mut ok, &owner, draws);
        }
        assert!(ok && plan.concat_layout);
        assert_eq!(plan.decoded_n, 12);
        assert_eq!(plan.indices[6..], [6, 7, 8, 9, 10, 11]);
        assert_eq!(plan.surface_ranges, [(0, 3), (3, 3), (6, 3), (9, 3)]);
        assert_eq!(plan.materials.len(), 4);
        let at: Vec<(u32, u32)> = plan.draws.iter().map(|d| (d.surface, d.material)).collect();
        assert_eq!(at, [(0, 0), (1, 1), (2, 2), (3, 3)]);
        assert_eq!(packed.len(), 12);
    }

    #[test]
    fn an_owner_with_an_unused_surface_is_cut_down_to_what_draws() {
        let two = TwoTriangles::new();
        let mut plan = XModelDrawPlan::default();
        plan.concat_layout = true;
        let mut packed = Vec::new();
        let mut ok = true;
        plan.append_owner(&mut packed, &mut ok, &two.owner(), vec![draw(1, 1, 1)]);
        assert!(ok);
        assert!(!plan.concat_layout);
        assert_eq!(plan.decoded_n, 3);
        assert_eq!(plan.indices, [0, 1, 2]);
        assert_eq!(plan.surface_ranges, [(0, 3)]);
        assert_eq!(plan.materials.len(), 1);
        assert_eq!((plan.draws[0].surface, plan.draws[0].material), (0, 0));
        assert_eq!(packed, [row(4), row(5), row(6)]);
    }

    #[test]
    fn rows_that_do_not_match_the_vertices_drop_every_packed_row() {
        let mut two = TwoTriangles::new();
        two.packed = asset_world::PackedVertexPayload::T6(vec![row(1)]);
        let mut plan = XModelDrawPlan::default();
        let mut packed = vec![row(9)];
        let mut ok = true;
        plan.append_owner(&mut packed, &mut ok, &two.owner(), vec![draw(0, 0, 1), draw(1, 1, 1)]);
        assert!(!ok && packed.is_empty());
    }

    #[test]
    fn a_bank_with_the_layout_is_patched_only_where_a_revision_moved() {
        let (a, b) = (TwoTriangles::new(), TwoTriangles::new());
        let mut changed = TwoTriangles::new();
        changed.packed = asset_world::PackedVertexPayload::T6((11..=16).map(row).collect());
        let admitted = bit(BODY) | bit(FPV);
        let mut owners = [Owner::absent(); OWNERS];
        owners[BODY] = a.owner();
        owners[FPV] = b.owner();
        let mut bank = Vec::new();
        let mut bank_vertices = [0; OWNERS];
        let mut segments = render_frame::PackedSegments::default();
        let mut at = [0u64; OWNERS];
        let built = recopy_concat_packed(
            &mut bank, true, &mut bank_vertices, at, admitted, &owners, &mut segments,
        );
        assert!(built && bank.len() == 12 && !segments.live);

        owners[FPV] = changed.owner();
        at[FPV] = 1;
        let patched = recopy_concat_packed(
            &mut bank, false, &mut bank_vertices, at, admitted, &owners, &mut segments,
        );
        assert!(patched && segments.live);
        assert_eq!((bank[0], bank[6], bank[11]), (row(1), row(11), row(16)));
        assert_eq!(bank_vertices, at);
        assert_eq!((segments.owners[FPV].start, segments.owners[FPV].rows), (6, 6));
    }

    #[test]
    fn merging_nothing_leaves_an_empty_plan_with_no_packed_rows() {
        let mut plan = XModelDrawPlan::default();
        let empty = (
            FpvDrawPlan::default(),
            RemoteBodyDrawPlan::default(),
            ScriptModelDrawPlan::default(),
            MissileDrawPlan::default(),
            ItemDrawPlan::default(),
            FxModelDrawPlan::default(),
            DynEntDrawPlan::default(),
        );
        for _ in 0..2 {
            merge_xmodel_draw_plan(
                &mut plan, &empty.0, &empty.1, &empty.2, &empty.3, &empty.4, &empty.5, &empty.6,
                None, None,
            );
        }
        assert!(plan.draws.is_empty() && plan.packed_rows().is_none());
        assert_eq!(plan.decoded_n, 0);
        assert!(plan.exact_packed_vertices().is_err());
    }
}
