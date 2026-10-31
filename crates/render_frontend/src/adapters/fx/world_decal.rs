//! Marks left by impacts on the world: a bullet hole or scorch is cut out of
//! the world's surfaces, its windows and its props around the impact, and
//! handed to the host's mark store.
//!
//! The host calls [`FrontendEffectStage`] when an impact finishes; the marks that
//! land on moving models are made by `entity_decal`.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use bevy::math::{Mat4, Vec3};
use fx::EffectSystemHost;
use marks_t6::{
    DecalAllocationAsk, DecalClipTarget, EffectStainStagingPoint, EffectStainStagingTri,
    GfxMarkContext, StainLevelStaging, impact_mark_material,
};
use render_fx::EntityMarkBoard;

use crate::prepare::scene::world::LevelStage;
use crate::prepare::scene::world::PropStainCpu;

/// The scene the effects' impacts are marked on.
pub struct FrontendEffectStage<'a> {
    level: &'a LevelStage,
    marks: &'a EntityMarkBoard,
}

impl<'a> FrontendEffectStage<'a> {
    pub fn wrap(scene: Option<&'a LevelStage>, marks: &'a EntityMarkBoard) -> Option<Self> {
        scene.map(|level| Self {
            level,
            marks,
        })
    }
}

impl render_fx::effect_play::EffectStage for FrontendEffectStage<'_> {
    fn light_at(&self, spot: [f32; 3]) -> Option<[u8; 3]> {
        let grid = self.level.light_grid.as_ref()?;
        asset_model::probe_light_cells(&grid.view(), spot)
            .ok()
            .map(|probe| probe.packed)
    }

    /// Finish one impact: queue its mark for the moving models it hit, and
    /// cut it into the world's surfaces and props.
    fn finish_impact_stains(
        &self,
        fx_host: &mut EffectSystemHost,
        definition_slot: u8,
        against_level: bool,
        versus_models: bool,
    ) {
        if versus_models {
            super::entity_decal::queue(fx_host, self.marks);
        }
        if against_level {
            mark_world(fx_host, self.level, definition_slot);
        }
        if versus_models {
            mark_props(fx_host, self.level, definition_slot);
        }
    }
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    centre: [f32; 3],
    half_size: [f32; 3],
}

/// The box around the points fed to it.
struct Extent {
    lo: [f32; 3],
    hi: [f32; 3],
}

impl Extent {
    const NOTHING: Self = Self {
        lo: [f32::MAX; 3],
        hi: [f32::MIN; 3],
    };

    fn include(&mut self, pt: [f32; 3]) {
        self.lo = core::array::from_fn(|k| self.lo[k].min(pt[k]));
        self.hi = core::array::from_fn(|k| self.hi[k].max(pt[k]));
    }

    /// `None` while nothing has been included.
    fn bounds(&self) -> Option<Bounds> {
        if self.lo[0] > self.hi[0] {
            return None;
        }
        Some(Bounds {
            centre: core::array::from_fn(|k| (self.lo[k] + self.hi[k]) * 0.5),
            half_size: core::array::from_fn(|k| (self.hi[k] - self.lo[k]) * 0.5),
        })
    }
}

/// The `count` indices from `start`, cut short at the end of the list.
fn index_run(indices: &[u32], start: u32, count: u32) -> &[u32] {
    let first = (start as usize).min(indices.len());
    let end = first.saturating_add(count as usize).min(indices.len());
    &indices[first..end]
}

/// Add the points the indices pick to `extent`; indices past the positions
/// are ignored.
fn extend_by_indices(extent: &mut Extent, pos_list: &[[f32; 3]], index_list: &[u32]) {
    let picked = index_list.iter().filter_map(|&at| pos_list.get(at as usize));
    picked.for_each(|&point| extent.include(point));
}

/// The box of each surface's run of the index list; `None` for a surface
/// without a point.
fn facet_bounds(
    pos_list: &[[f32; 3]],
    index_list: &[u32],
    runs: &[(u32, u32)],
) -> Vec<Option<Bounds>> {
    runs.iter()
        .map(|&(start, count)| {
            let mut extent = Extent::NOTHING;
            extend_by_indices(&mut extent, pos_list, index_run(index_list, start, count));
            extent.bounds()
        })
        .collect()
}

/// A box carried by a transform: the box around the transformed original.
fn transformed_bounds(matrix: &Mat4, bounds: &Bounds) -> Bounds {
    let columns = [matrix.x_axis, matrix.y_axis, matrix.z_axis];
    let moved_centre = matrix.transform_point3(Vec3::from_array(bounds.centre));
    Bounds {
        centre: moved_centre.to_array(),
        half_size: core::array::from_fn(|k| {
            (0..3)
                .map(|j| columns[j][k].abs() * bounds.half_size[j])
                .sum::<f32>()
        }),
    }
}

/// The unit normal of a triangle; `None` when it is too thin to have one.
fn triangle_normal([a, b, c]: [Vec3; 3]) -> Option<Vec3> {
    let normal = (b - a).cross(c - a);
    let length = normal.length();
    (length >= 1e-6).then(|| normal / length)
}

/// BO2 takes marks on at most this many windows per impact.
const MARK_GLASS_PANE_LIMIT: usize = 8;

/// The windows a mark's box touches that are not shattered (whole or
/// cracked), in pane order, the first `MARK_GLASS_PANE_LIMIT`.
fn mark_glass_panes(
    panes: &[asset_world::T6GlassPaneDraw],
    broken: impl Fn(u32) -> bool,
    mins: [f32; 3],
    maxs: [f32; 3],
) -> Vec<usize> {
    panes
        .iter()
        .enumerate()
        .filter(|&(i, pane)| {
            !broken(i as u32)
                && (0..3).all(|a| pane.absmin[a] <= maxs[a] && mins[a] <= pane.absmax[a])
        })
        .map(|(i, _)| i)
        .take(MARK_GLASS_PANE_LIMIT)
        .collect()
}

/// What the host recorded about the impact being marked.
struct ImpactPose {
    pos: [f32; 3],
    rad: f32,
    basis: [[f32; 3]; 3],
    mark_material: Option<String>,
    color: u32,
}

/// The impact's pose, if the host's latest decal was cast against `against`
/// and is complete.
fn impact_pose(host: &EffectSystemHost, against: DecalClipTarget) -> Option<ImpactPose> {
    let cast_against = match against {
        DecalClipTarget::WorldBrushes => host.last_stain_on_world,
        DecalClipTarget::Models => host.last_stain_on_models,
    };
    if cast_against != Some(true) {
        return None;
    }
    let (Some(origin), Some(reach_r), Some(axes_m), Some(rot_q), Some(color)) = (
        host.last_stain_origin,
        host.last_stain_size0,
        host.last_stain_axis,
        host.last_stain_rotation,
        host.last_stain_color,
    ) else {
        return None;
    };
    let names = [
        host.last_stain_mat0.as_deref(),
        host.prior_decal_second_surface.as_deref(),
    ];
    Some(ImpactPose {
        pos: origin,
        rad: reach_r,
        basis: fx_t6::runtime::impact_mark_axis(axes_m, rot_q),
        mark_material: impact_mark_material(names, against).map(str::to_owned),
        color,
    })
}

/// What every part of one impact mark shares.
struct StagedMark<'a> {
    centre: [f32; 3],
    size: f32,
    axes_m: [[f32; 3]; 3],
    authored_tint: u32,
    material_name: Option<&'a str>,
}

impl<'a> From<&'a ImpactPose> for StagedMark<'a> {
    fn from(pose: &'a ImpactPose) -> Self {
        Self {
            axes_m: pose.basis,
            centre: pose.pos,
            size: pose.rad,
            material_name: pose.mark_material.as_deref(),
            authored_tint: pose.color,
        }
    }
}

/// The runtime material called `name`, if the world has one.
pub(crate) fn live_material_by_name<'a>(
    scene: &'a LevelStage,
    name: &str,
) -> Option<&'a crate::assemble::draw_items::LiveMaterial> {
    let want = asset_core::AssetPointer::bare_name(name);
    let materials = &scene.live_material_ledger.materials;
    materials
        .iter()
        .find(|material| asset_core::AssetPointer::bare_name(&material.name) == want)
}

/// What a material says about taking marks, for `mark_allow`.
pub(crate) fn mark_receiver(
    scene: &LevelStage,
    material_slot: Option<assets::SurfaceMaterialId>,
) -> Option<marks_t6::DecalSurfaceGate> {
    let ledger = &scene.live_material_ledger;
    let computed = ledger.derived(material_slot?)?;
    Some(marks_t6::DecalSurfaceGate {
        facet_type_bits: computed.facet_type_bits,
        game_flags: computed.game_flag_info,
        draw_rank: computed.draw_sort_key,
    })
}

/// The surface type bits of the mark's own material.
fn mark_surface_bits(scene: &LevelStage, material: Option<&str>) -> Option<u32> {
    material
        .and_then(|name| live_material_by_name(scene, name))
        .and_then(|m| m.facet_type_bits)
}

/// BO2's `R_AllowMarks` for one surface: no decals, no no-marks materials,
/// and the surface type bits must hold the mark's.
fn surface_takes_mark(
    scene: &LevelStage,
    slot: Option<assets::SurfaceMaterialId>,
    stain_bits: Option<u32>,
) -> bool {
    marks_t6::mark_include_in_world_clip(marks_t6::mark_allow(
        mark_receiver(scene, slot),
        stain_bits,
    ))
}

/// Which surfaces of the world a mark may touch, and what they are lit by.
#[derive(Default)]
struct WorldFilter {
    centres: Vec<[f32; 3]>,
    half_sizes: Vec<[f32; 3]>,
    spans: Vec<(u32, u32)>,
    contexts: Vec<GfxMarkContext>,
}

impl WorldFilter {
    /// Empties every list, keeping their room.
    fn reset(&mut self) {
        let Self {
            centres,
            half_sizes,
            spans,
            contexts,
        } = self;
        centres.clear();
        half_sizes.clear();
        spans.clear();
        contexts.clear();
    }

    fn add_box(&mut self, bbox: Bounds, range: (u32, u32), context: GfxMarkContext) {
        self.centres.push(bbox.centre);
        self.half_sizes.push(bbox.half_size);
        self.spans.push(range);
        self.contexts.push(context);
    }
}

/// The room the clip cuts a mark's triangles into.
#[derive(Default)]
struct Staging {
    triangle_list: Vec<EffectStainStagingTri>,
    verts: Vec<EffectStainStagingPoint>,
}

impl Staging {
    /// Both buffers at the size the clip is allowed to fill.
    fn open(&mut self) -> (&mut [EffectStainStagingTri], &mut [EffectStainStagingPoint]) {
        let triangle_tally = marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING as usize;
        let point_tally = marks_t6::DECAL_FRAGMENT_VERTEX_CEILING as usize;
        if self.triangle_list.len() != triangle_tally {
            self.triangle_list.clear();
            self.triangle_list
                .resize(triangle_tally, EffectStainStagingTri::ZERO);
        }
        if self.verts.len() != point_tally {
            self.verts = vec![EffectStainStagingPoint::ZERO; point_tally];
        }
        (&mut self.triangle_list, &mut self.verts)
    }
}

std::thread_local! {
    static WORLD_FILTER: RefCell<WorldFilter> = RefCell::new(WorldFilter::default());
    static STAGING: RefCell<Staging> = RefCell::new(Staging::default());
    /// Per world surface, its box from the retained positions; keyed by the
    /// positions it was built from.
    static SURFACE_BOUNDS: RefCell<(usize, Vec<Option<Bounds>>)> =
        const { RefCell::new((0, Vec::new())) };
}

/// The lighting a world surface gives a mark made on it.
fn world_surface_context(scene: &LevelStage, facet: usize) -> GfxMarkContext {
    let (lightmap, primary, probe) = match scene.culling.as_ref() {
        Some(cull) => (
            cull.facet_lightbake_indices
                .get(facet)
                .copied()
                .unwrap_or(marks_t6::SURFACE_BAKED_LIGHT_ABSENT),
            cull.facet_key_lights.get(facet).copied().unwrap_or(0),
            cull.facet_env_probes.get(facet).copied().unwrap_or(0),
        ),
        None => (marks_t6::SURFACE_BAKED_LIGHT_ABSENT, 0, 0),
    };
    GfxMarkContext::world_surface(lightmap, primary, probe)
}

/// Fill the world filter with the surfaces the mark's sphere touches and
/// that take marks; how many there are. Every surface's box is tested.
fn fill_world_filter(scene: &LevelStage, pose: &ImpactPose) -> usize {
    let Some(cull_set) = scene.culling.as_ref() else {
        return 0;
    };
    let positions = &scene.held_positions;
    let key = positions.as_ptr() as usize ^ positions.len() ^ cull_set.compact_indices.len();
    let stain_bits = mark_surface_bits(scene, pose.mark_material.as_deref());
    let reach_squared = pose.rad * pose.rad;
    SURFACE_BOUNDS.with_borrow_mut(|cache| {
        if cache.0 != key {
            cache.1 = facet_bounds(positions, &cull_set.compact_indices, &cull_set.facet_index_ranges);
            cache.0 = key;
        }
        let near = |b: &Bounds| {
            marks_t6::mark_sphere_hits_bounds(pose.pos, reach_squared, b.centre, b.half_size)
        };
        let takes = |facet: usize| {
            let slot = cull_set.facet_materials.get(facet).copied().flatten();
            surface_takes_mark(scene, slot, stain_bits)
        };
        WORLD_FILTER.with_borrow_mut(|filter| {
            filter.reset();
            let boxed = cache.1.iter().enumerate();
            let candidates =
                boxed.filter_map(|(facet, b)| b.filter(|b| near(b)).map(|b| (facet, b)));
            for (facet, facet_box) in candidates {
                let span = cull_set.facet_index_ranges.get(facet).copied();
                if let Some(span) = span.filter(|_| takes(facet)) {
                    filter.add_box(facet_box, span, world_surface_context(scene, facet));
                }
            }
            filter.spans.len()
        })
    })
}

/// The geometry a mark is clipped against: per surface its box and its run
/// of the index list, over one shared set of vertices.
struct ClipInput<'a> {
    centers: &'a [[f32; 3]],
    reach: &'a [[f32; 3]],
    positions: &'a [[f32; 3]],
    texcoord_set: &'a [[f32; 2]],
    surface_directions: &'a [[f32; 3]],
    indices: &'a [u32],
    spans: &'a [(u32, u32)],
    contexts: &'a [GfxMarkContext],
}

/// Clip the mark against `input` into the staging; what was kept.
fn clip_into_staging(mark: &StagedMark<'_>, input: &ClipInput<'_>) -> StainLevelStaging {
    STAGING.with_borrow_mut(|staging| {
        let (tri_room, point_room) = staging.open();
        marks_t6::mark_stage_world_surfaces(
            mark.centre,
            mark.size,
            mark.axes_m,
            input.centers,
            input.reach,
            input.positions,
            input.texcoord_set,
            input.surface_directions,
            input.indices,
            input.spans,
            input.contexts,
            marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING,
            marks_t6::DECAL_FRAGMENT_VERTEX_CEILING,
            tri_room,
            point_room,
            false,
        )
    })
}

/// Cut the mark out of the surfaces in the world filter, into the staging.
fn stage_world_surfaces(scene: &LevelStage, mark: &StagedMark<'_>) -> Option<StainLevelStaging> {
    let retained = scene.held_positions.len();
    if retained == 0
        || scene.held_lightbake_uvs.len() != retained
        || scene.held_normals.len() != retained
    {
        return None;
    }
    let packed = scene
        .culling
        .as_ref()
        .map(|cull| cull.compact_indices.as_slice())
        .unwrap_or_default();
    WORLD_FILTER.with_borrow(|filter| {
        if filter.spans.is_empty() {
            return Some(StainLevelStaging::default());
        }
        let input = ClipInput {
            centers: &filter.centres,
            reach: &filter.half_sizes,
            positions: &scene.held_positions,
            texcoord_set: &scene.held_lightbake_uvs,
            surface_directions: &scene.held_normals,
            indices: packed,
            spans: &filter.spans,
            contexts: &filter.contexts,
        };
        Some(clip_into_staging(mark, &input))
    })
}

/// Hand what the staging holds to the host as one mark: its handle, or none
/// when the staging is empty, overflowed, or the host has no room.
fn alloc_staged_mark(
    fx_host: &mut EffectSystemHost,
    mark: &StagedMark<'_>,
    staged: StainLevelStaging,
) -> Option<u16> {
    if staged.triangles_consumed == 0 || staged.overflow {
        return None;
    }
    let (first_context, triangle_list, points) = STAGING.with_borrow(|staging| {
        let tri_end = (staged.triangles_consumed as usize).min(staging.triangle_list.len());
        let point_range_stop = (staged.points_consumed as usize).min(staging.verts.len());
        (
            staging.triangle_list.first().map(|tri| tri.context),
            staging.triangle_list[..tri_end].to_vec(),
            staging.verts[..point_range_stop].to_vec(),
        )
    });
    let material = 0;
    let request = DecalAllocationAsk {
        any_stains: true,
        tri_tally: staged.triangles_consumed,
        vertex_total_in_mark: staged.points_consumed,
        origin: mark.centre,
        radius: mark.size,
        uv_projection_axis: mark.axes_m[1],
        authored_tint: mark.authored_tint,
        material,
        anim_steps: fx_host.frame_tick_no,
        leading_triangle_mark_info: first_context?,
    };
    let handle = fx_host
        .marks
        .claim_decal_for_scene_hook(request, &triangle_list, &points)?;
    fx_host.marks.assign_material_name(handle, mark.material_name);
    Some(handle)
}

/// The world's part of an impact: the surfaces around it as one mark, then
/// each window the mark's box touches as a mark of its own.
fn mark_world(host: &mut EffectSystemHost, scene: &LevelStage, definition_slot: u8) {
    let Some(pose) = impact_pose(host, DecalClipTarget::WorldBrushes) else {
        host.note_fragments_skipped(definition_slot);
        return;
    };
    let mark = StagedMark::from(&pose);
    let mut shots_fired = false;
    if fill_world_filter(scene, &pose) != 0
        && let Some(staged) = stage_world_surfaces(scene, &mark)
        && let Some(handle) = alloc_staged_mark(host, &mark, staged)
    {
        host.last_stain_alloc_slot = Some(handle);
        shots_fired = true;
    }
    shots_fired |= mark_glass(host, scene, &mark);
    if !shots_fired {
        host.note_fragments_skipped(definition_slot);
    }
}

/// The windows' part of a world mark: each pane the mark's box touches is
/// clipped as a brush model and flushed as its own mark on the world list,
/// with the pane as its model index so a shatter frees it. Pane `i` stands
/// for `Glass::index`, as in the shatter's detach and the snapshots.
fn mark_glass(host: &mut EffectSystemHost, scene: &LevelStage, mark: &StagedMark<'_>) -> bool {
    let Some(glass) = scene.t6_glass.as_ref() else {
        return false;
    };
    let mins = mark.centre.map(|c| c - mark.size);
    let maxs = mark.centre.map(|c| c + mark.size);
    let picked = mark_glass_panes(&glass.panes, |p| host.glass.pane_broken(p), mins, maxs);
    let stain_bits = mark_surface_bits(scene, mark.material_name);
    let facet_materials = scene
        .culling
        .as_ref()
        .map(|cull| cull.facet_materials.as_slice())
        .unwrap_or_default();
    let takes = |surface: u32| {
        let slot = facet_materials.get(surface as usize).copied().flatten();
        surface_takes_mark(scene, slot, stain_bits)
    };
    let mut shots_fired = false;
    for index in picked {
        let staged = stage_glass_pane(&glass.panes[index], index as u16, mark, &takes);
        if let Some(handle) = alloc_staged_mark(host, mark, staged) {
            host.last_stain_alloc_slot.get_or_insert(handle);
            shots_fired = true;
        }
    }
    shots_fired
}

/// One window's surfaces through the brush-surface clip, into the staging.
/// BO2 moves the mark into the pane's space and the points back out; the
/// pane's vertices are already in world space, so this clips there, the same
/// up to rounding. Each surface's lightmap, primary light and probe light the
/// mark. `takes` says whether a world surface's material takes the mark.
fn stage_glass_pane(
    pane: &asset_world::T6GlassPaneDraw,
    pane_id: u16,
    mark: &StagedMark<'_>,
    takes: &dyn Fn(u32) -> bool,
) -> StainLevelStaging {
    let surfaces: Vec<_> = pane.surfaces.iter().filter(|s| takes(s.surface)).collect();
    let positions: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.position).collect();
    let lightbake_uvs: Vec<[f32; 2]> = pane.vertices.iter().map(|v| v.lightmap_uv).collect();
    let surface_directions: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.hit_facing).collect();
    let ranges: Vec<(u32, u32)> = surfaces.iter().map(|s| (s.first, s.count)).collect();
    let contexts: Vec<GfxMarkContext> = surfaces
        .iter()
        .map(|s| {
            GfxMarkContext::glass_pane(s.lightmap, s.dominant_light, s.reflection_probe, pane_id)
        })
        .collect();
    // The pane's own box test stands in for the per-surface sphere test.
    let unused = vec![[0.0; 3]; ranges.len()];
    let input = ClipInput {
        centers: &unused,
        reach: &unused,
        positions: &positions,
        texcoord_set: &lightbake_uvs,
        surface_directions: &surface_directions,
        indices: &pane.tri_list,
        spans: &ranges,
        contexts: &contexts,
    };
    clip_into_staging(mark, &input)
}

/// Per static-model mesh, its box in model space; keyed by the mark copy it
/// came from. Built once per map and shared by every thread that marks.
static SMODEL_BOUNDS: Mutex<(usize, Option<Arc<Vec<Option<Bounds>>>>)> = Mutex::new((0, None));

/// The box of each mesh, if it has a point.

/// The mesh boxes of `cpu`, built on first use.
fn smodel_bounds(cpu: &PropStainCpu) -> Option<Arc<Vec<Option<Bounds>>>> {
    let key = cpu.pos_list.as_ptr() as usize ^ cpu.pos_list.len() ^ cpu.meshes.len();
    let mut slot = SMODEL_BOUNDS.lock().ok()?;
    if slot.0 == key
        && let Some(bounds) = slot.1.as_ref()
    {
        return Some(Arc::clone(bounds));
    }
    let bounds: Vec<Option<Bounds>> = cpu
        .meshes
        .iter()
        .map(|surf_list| {
            let mut extent = Extent::NOTHING;
            for surface in surf_list {
                let (first, count) = (surface.first_triangle_slot, surface.index_total);
                let run = index_run(&cpu.tri_list, first, count);
                extend_by_indices(&mut extent, &cpu.pos_list, run);
            }
            extent.bounds()
        })
        .collect();
    let bounds = Arc::new(bounds);
    *slot = (key, Some(Arc::clone(&bounds)));
    Some(bounds)
}

/// The triangles of the props around one mark, placed in the world.
#[derive(Default)]
struct PropTriangles {
    placed: Vec<[f32; 3]>,
    surface_directions: Vec<[f32; 3]>,
    texcoord_set: Vec<[f32; 2]>,
    index_list: Vec<u32>,
    filter: WorldFilter,
}

impl PropTriangles {
    /// Empties the triangles and their surfaces, keeping the room.
    fn reset(&mut self) {
        self.filter.reset();
        self.index_list.clear();
        self.placed.clear();
        self.texcoord_set.clear();
        self.surface_directions.clear();
    }

    /// Add a triangle, flat shaded; `false` when it has no area.
    fn push_triangle(&mut self, corners: [[f32; 3]; 3]) -> bool {
        let Some(normal) = triangle_normal(corners.map(Vec3::from_array)) else {
            return false;
        };
        for point in corners {
            self.index_list.push(self.placed.len() as u32);
            self.placed.push(point);
            self.surface_directions.push(normal.to_array());
            self.texcoord_set.push([0.0, 0.0]);
        }
        true
    }

    /// Close the triangles pushed since `start` as one surface.
    fn finish_surface(&mut self, start: u32, bbox: Bounds, context: GfxMarkContext) {
        let count = self.index_list.len() as u32 - start;
        if count >= 3 {
            self.filter.add_box(bbox, (start, count), context);
        }
    }
}

std::thread_local! {
    static PROP_TRIANGLES: RefCell<PropTriangles> = RefCell::new(PropTriangles::default());
}

/// Do props' surfaces take marks? Glass, decals and other blended or unlit
/// surfaces do not.
fn prop_takes_marks(scene: &LevelStage, material: Option<assets::SurfaceMaterialId>) -> bool {
    material
        .and_then(|id| scene.live_material_ledger.derived(id))
        .and_then(|m| m.t6_draw)
        .is_some_and(|draw| matches!(draw.blend, asset_core::T6Blend::Opaque) && !draw.unlit)
}

/// Gather the triangles of every prop whose box the mark's sphere touches.
fn gather_prop_triangles(
    scene: &LevelStage,
    cpu: &PropStainCpu,
    bounds: &[Option<Bounds>],
    pose: &ImpactPose,
    triangle_list: &mut PropTriangles,
) {
    let reach_squared = pose.rad * pose.rad;
    for instance in scene.prop_instances.iter().flatten() {
        let Some(Some(local)) = bounds.get(instance.mesh) else {
            continue;
        };
        let matrix = instance.transform.to_matrix();
        let placed = transformed_bounds(&matrix, local);
        let (centre, half_size) = (placed.centre, placed.half_size);
        if !marks_t6::mark_sphere_hits_bounds(pose.pos, reach_squared, centre, half_size) {
            continue;
        }
        let Some(surf_list) = cpu.meshes.get(instance.mesh) else {
            continue;
        };
        let context = GfxMarkContext::world_surface(
            marks_t6::SURFACE_BAKED_LIGHT_ABSENT,
            instance.key_light_index,
            instance.env_probe_index,
        );
        let place = |p: [f32; 3]| matrix.transform_point3(Vec3::from_array(p)).to_array();
        for facet in surf_list {
            if !prop_takes_marks(scene, facet.material) {
                continue;
            }
            let start = triangle_list.index_list.len() as u32;
            let (first, count) = (facet.first_triangle_slot, facet.index_total);
            let run = index_run(&cpu.tri_list, first, count);
            for &[ia, ib, ic] in run.as_chunks::<3>().0 {
                let at = |i: u32| cpu.pos_list.get(i as usize).copied().map(place);
                if let (Some(a), Some(b), Some(c)) = (at(ia), at(ib), at(ic)) {
                    triangle_list.push_triangle([a, b, c]);
                }
            }
            triangle_list.finish_surface(start, placed, context);
        }
    }
}

/// The props' part of an impact. Bullets stop at the map's props (their
/// collision), so their holes go on the props as BO2's do: every opaque
/// first-LOD triangle of the props whose box the mark's sphere touches,
/// placed in the world and clipped to the mark's box as world surfaces are.
/// Marks are made with the effect's `mc_` materials and lit by the light grid.
fn mark_props(host: &mut EffectSystemHost, scene: &LevelStage, definition_slot: u8) {
    let began = std::time::Instant::now();
    let fired = mark_props_inner(host, scene);
    if let Some(handle) = fired {
        host.last_stain_alloc_slot.get_or_insert(handle);
    } else {
        host.note_fragments_skipped(definition_slot);
    }
    let ms = began.elapsed().as_secs_f64() * 1000.0;
    if ms > 4.0 {
        diag::warn!(World, "prop marks: slow mark on props, {ms:.1} ms");
    }
}

fn mark_props_inner(host: &mut EffectSystemHost, scene: &LevelStage) -> Option<u16> {
    let pose = impact_pose(host, DecalClipTarget::Models)?;
    let cpu = scene.smodel_mark_cpu.as_ref()?;
    let bounds = smodel_bounds(cpu)?;
    let mark = StagedMark::from(&pose);
    let staged = PROP_TRIANGLES.with_borrow_mut(|triangle_list| {
        triangle_list.reset();
        gather_prop_triangles(scene, cpu, &bounds, &pose, triangle_list);
        if triangle_list.filter.spans.is_empty() {
            return None;
        }
        let input = ClipInput {
            centers: &triangle_list.filter.centres,
            reach: &triangle_list.filter.half_sizes,
            positions: &triangle_list.placed,
            texcoord_set: &triangle_list.texcoord_set,
            surface_directions: &triangle_list.surface_directions,
            indices: &triangle_list.index_list,
            spans: &triangle_list.filter.spans,
            contexts: &triangle_list.filter.contexts,
        };
        Some(clip_into_staging(&mark, &input))
    })?;
    alloc_staged_mark(host, &mark, staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(lo: f32, hi: f32) -> asset_world::T6GlassPaneDraw {
        asset_world::T6GlassPaneDraw {
            absmin: [lo; 3],
            absmax: [hi; 3],
            ..Default::default()
        }
    }

    #[test]
    fn a_mark_takes_the_first_eight_whole_or_cracked_panes_it_overlaps() {
        let mut panes: Vec<_> = (0..12).map(|_| pane(0.0, 10.0)).collect();
        panes[1] = pane(20.0, 30.0);
        let picked = mark_glass_panes(&panes, |p| p == 2, [5.0; 3], [15.0; 3]);
        assert_eq!(picked, vec![0, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn panes_whose_faces_only_touch_the_mark_box_still_count() {
        let panes = [pane(0.0, 10.0)];
        assert_eq!(
            mark_glass_panes(&panes, |_| false, [10.0; 3], [20.0; 3]),
            vec![0]
        );
        assert!(mark_glass_panes(&panes, |_| false, [10.1; 3], [20.0; 3]).is_empty());
    }

    #[test]
    fn a_glass_mark_is_on_the_world_list_with_its_pane_as_model_index() {
        let vertex = |x: f32, y: f32| asset_world::T6GlassVertex {
            position: [x, y, 0.0],
            hit_facing: Vec3::Z.to_array(),
            ..Default::default()
        };
        let glass = asset_world::T6GlassPaneDraw {
            vertices: vec![vertex(-50.0, -50.0), vertex(50.0, -50.0), vertex(0.0, 50.0)],
            tri_list: vec![0, 1, 2],
            surfaces: vec![asset_world::T6GlassSurface {
                surface: 9,
                first: 0,
                count: 3,
                lightmap: 4,
                dominant_light: 5,
                reflection_probe: 6,
            }],
            ..pane(-50.0, 50.0)
        };
        let mark = StagedMark {
            centre: [0.0, 0.0, 1.0],
            size: 8.0,
            axes_m: [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            authored_tint: 0,
            material_name: None,
        };
        let staged = stage_glass_pane(&glass, 7, &mark, &|surface| surface == 9);
        assert!(
            staged.triangles_consumed > 0 && !staged.overflow,
            "{staged:?}"
        );
        let context = STAGING.with_borrow(|s| s.triangle_list[0].context);
        assert_eq!(context, GfxMarkContext::glass_pane(4, 5, 6, 7));
        let skipped = stage_glass_pane(&glass, 7, &mark, &|_| false);
        assert_eq!(skipped.triangles_consumed, 0);
    }

    #[test]
    fn an_index_run_stops_at_the_end_of_the_list() {
        let indices = [0, 1, 2, 3, 4, 5];
        assert_eq!(index_run(&indices, 1, 3), &[1, 2, 3]);
        assert_eq!(index_run(&indices, 4, 10), &[4, 5]);
        assert!(index_run(&indices, 9, 3).is_empty());
        assert!(index_run(&indices, 2, 0).is_empty());
        assert_eq!(index_run(&indices, 0, u32::MAX).len(), 6);
    }

    #[test]
    fn extents_grow_to_their_points_and_are_empty_without_any() {
        let mut extent = Extent::NOTHING;
        assert_eq!(extent.bounds(), None);
        extent.include([1.0, 2.0, 3.0]);
        extent.include([3.0, -2.0, 3.0]);
        assert_eq!(
            extent.bounds(),
            Some(Bounds {
                centre: [2.0, 0.0, 3.0],
                half_size: [1.0, 2.0, 0.0],
            })
        );
    }

    #[test]
    fn surface_boxes_follow_their_index_runs_and_skip_bad_indices() {
        let positions = [[0.0; 3], [2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [100.0; 3]];
        let indices = [0, 1, 2, 9, 3, 3];
        let boxes = facet_bounds(&positions, &indices, &[(0, 4), (3, 1), (4, 2), (8, 2)]);
        assert_eq!(
            boxes[0],
            Some(Bounds {
                centre: [1.0, 2.0, 0.0],
                half_size: [1.0, 2.0, 0.0],
            })
        );
        assert_eq!(boxes[1], None, "an index past the positions is ignored");
        assert_eq!(
            boxes[2],
            Some(Bounds {
                centre: [100.0; 3],
                half_size: [0.0; 3],
            })
        );
        assert_eq!(boxes[3], None, "a run past the list is empty");
    }

    #[test]
    fn a_rotated_box_grows_to_hold_the_original() {
        let bounds = Bounds {
            centre: [1.0, 0.0, 0.0],
            half_size: [2.0, 1.0, 0.5],
        };
        let turned = transformed_bounds(
            &Mat4::from_rotation_z(core::f32::consts::FRAC_PI_2),
            &bounds,
        );
        assert!((turned.centre[0]).abs() < 1e-5 && (turned.centre[1] - 1.0).abs() < 1e-5);
        assert!((turned.half_size[0] - 1.0).abs() < 1e-5);
        assert!((turned.half_size[1] - 2.0).abs() < 1e-5);
        assert!((turned.half_size[2] - 0.5).abs() < 1e-5);
        let moved = transformed_bounds(&Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)), &bounds);
        assert_eq!(moved.centre, [6.0, 0.0, 0.0]);
        assert_eq!(moved.half_size, bounds.half_size);
    }

    #[test]
    fn triangle_normals_are_unit_and_thin_triangles_have_none() {
        let normal = triangle_normal([Vec3::ZERO, Vec3::X * 2.0, Vec3::Y * 2.0]).unwrap();
        assert_eq!(normal, Vec3::Z);
        assert_eq!(
            triangle_normal([Vec3::ZERO, Vec3::X, Vec3::X * 2.0]),
            None
        );
        assert_eq!(triangle_normal([Vec3::ONE; 3]), None);
    }

    #[test]
    fn prop_triangles_are_flat_shaded_and_closed_per_surface() {
        let mut triangle_list = PropTriangles::default();
        let bounds = Bounds {
            centre: [0.0; 3],
            half_size: [1.0; 3],
        };
        let context = GfxMarkContext::world_surface(marks_t6::SURFACE_BAKED_LIGHT_ABSENT, 1, 2);
        let start = triangle_list.index_list.len() as u32;
        assert!(triangle_list.push_triangle([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        assert!(!triangle_list.push_triangle([[0.0; 3]; 3]));
        triangle_list.finish_surface(start, bounds, context);
        assert_eq!(triangle_list.filter.spans, vec![(0, 3)]);
        assert_eq!(triangle_list.surface_directions, vec![[0.0, 0.0, 1.0]; 3]);
        let empty_start = triangle_list.index_list.len() as u32;
        triangle_list.finish_surface(empty_start, bounds, context);
        assert_eq!(
            triangle_list.filter.spans.len(),
            1,
            "a surface with no triangle is dropped"
        );
        triangle_list.reset();
        assert!(triangle_list.placed.is_empty() && triangle_list.filter.spans.is_empty());
    }

    #[test]
    fn the_staging_opens_to_full_size_once() {
        let mut staging = Staging::default();
        let (triangle_list, points) = staging.open();
        assert_eq!(
            triangle_list.len(),
            marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING as usize
        );
        assert_eq!(
            points.len(),
            marks_t6::DECAL_FRAGMENT_VERTEX_CEILING as usize
        );
    }
}
