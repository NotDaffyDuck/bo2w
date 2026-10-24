//! Marks left by impacts on the world: a bullet hole or scorch is cut out of
//! the world's surfaces, its windows and its props around the impact, and
//! handed to the host's mark store.
//!
//! The host calls [`FrontendFxScene`] when an impact finishes; the marks that
//! land on moving models are made by `entity_mark`.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use bevy::math::{Mat4, Vec3};
use fx::FxSystemHost;
use marks_t6::{
    FxAllocMarkRequest, FxMarkStagingPoint, FxMarkStagingTri, GfxMarkContext, MarkFragmentsAgainst,
    MarkWorldStaging, impact_mark_material,
};
use render_fx::EntityMarks;

use crate::prepare::scene::world::{SmodelMarkCpu, WorldScene};

/// The scene the effects' impacts are marked on.
pub struct FrontendFxScene<'a> {
    world: &'a WorldScene,
    marks: &'a EntityMarks,
}

impl<'a> FrontendFxScene<'a> {
    pub fn wrap(scene: Option<&'a WorldScene>, marks: &'a EntityMarks) -> Option<Self> {
        scene.map(|world| Self { world, marks })
    }
}

impl render_fx::present::FxScene for FrontendFxScene<'_> {
    fn sample_atpoint_rgb(&self, origin: [f32; 3]) -> Option<[u8; 3]> {
        let grid = self.world.light_grid.as_ref()?;
        asset_model::sample_light_grid(&grid.view(), origin)
            .ok()
            .map(|sample| sample.compressed)
    }

    fn finish_impact_marks(
        &self,
        host: &mut FxSystemHost,
        def_index: u8,
        against_world: bool,
        against_models: bool,
    ) {
        finish_impact_marks(
            host,
            self.world,
            self.marks,
            def_index,
            against_world,
            against_models,
        );
    }
}

/// Finish one impact: queue its mark for the moving models it hit, and cut
/// it into the world's surfaces and props.
pub(crate) fn finish_impact_marks(
    host: &mut FxSystemHost,
    scene: &WorldScene,
    marks: &EntityMarks,
    def_index: u8,
    against_world: bool,
    against_models: bool,
) {
    if against_models {
        super::entity_mark::queue(host, marks);
    }
    if against_world {
        mark_world(host, scene, def_index);
    }
    if against_models {
        mark_props(host, scene, def_index);
    }
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    mid: [f32; 3],
    half: [f32; 3],
}

/// The box around the points fed to it.
struct Extent {
    lo: [f32; 3],
    hi: [f32; 3],
}

impl Extent {
    const EMPTY: Self = Self {
        lo: [f32::MAX; 3],
        hi: [f32::MIN; 3],
    };

    fn include(&mut self, point: [f32; 3]) {
        self.lo = core::array::from_fn(|k| self.lo[k].min(point[k]));
        self.hi = core::array::from_fn(|k| self.hi[k].max(point[k]));
    }

    /// `None` while nothing has been included.
    fn bounds(&self) -> Option<Bounds> {
        if self.lo[0] > self.hi[0] {
            return None;
        }
        Some(Bounds {
            mid: core::array::from_fn(|k| (self.lo[k] + self.hi[k]) * 0.5),
            half: core::array::from_fn(|k| (self.hi[k] - self.lo[k]) * 0.5),
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
fn extend_by_indices(extent: &mut Extent, positions: &[[f32; 3]], indices: &[u32]) {
    for &index in indices {
        if let Some(&point) = positions.get(index as usize) {
            extent.include(point);
        }
    }
}

/// The box of each surface's run of the index list; `None` for a surface
/// without a point.
fn surface_bounds(
    positions: &[[f32; 3]],
    indices: &[u32],
    runs: &[(u32, u32)],
) -> Vec<Option<Bounds>> {
    runs.iter()
        .map(|&(start, count)| {
            let mut extent = Extent::EMPTY;
            extend_by_indices(&mut extent, positions, index_run(indices, start, count));
            extent.bounds()
        })
        .collect()
}

/// A box carried by a transform: the box around the transformed original.
fn transformed_bounds(matrix: &Mat4, bounds: &Bounds) -> Bounds {
    let columns = [matrix.x_axis, matrix.y_axis, matrix.z_axis];
    Bounds {
        mid: matrix
            .transform_point3(Vec3::from_array(bounds.mid))
            .to_array(),
        half: core::array::from_fn(|k| {
            (0..3)
                .map(|j| columns[j][k].abs() * bounds.half[j])
                .sum::<f32>()
        }),
    }
}

/// The unit normal of a triangle; `None` when it is too thin to have one.
fn triangle_normal(corners: [[f32; 3]; 3]) -> Option<[f32; 3]> {
    let [a, b, c] = corners.map(Vec3::from_array);
    let normal = (b - a).cross(c - a);
    let length = normal.length();
    (length >= 1e-6).then(|| (normal / length).to_array())
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
    origin: [f32; 3],
    radius: f32,
    axis: [[f32; 3]; 3],
    material: Option<String>,
    color: u32,
}

/// The impact's pose, if the host's latest decal was cast against `against`
/// and is complete.
fn impact_pose(host: &FxSystemHost, against: MarkFragmentsAgainst) -> Option<ImpactPose> {
    let cast_against = match against {
        MarkFragmentsAgainst::WorldBrushes => host.last_decal_against_world,
        MarkFragmentsAgainst::Models => host.last_decal_against_models,
    };
    if cast_against != Some(true) {
        return None;
    }
    let materials = [
        host.last_decal_mat0.as_deref(),
        host.last_decal_mat1.as_deref(),
    ];
    Some(ImpactPose {
        origin: host.last_decal_origin?,
        radius: host.last_decal_size0?,
        axis: fx_t6::runtime::impact_mark_axis(host.last_decal_axis?, host.last_decal_rotation?),
        material: impact_mark_material(materials, against).map(str::to_owned),
        color: host.last_decal_color?,
    })
}

/// What every part of one impact mark shares.
struct StagedMark<'a> {
    origin: [f32; 3],
    radius: f32,
    axis: [[f32; 3]; 3],
    native_color: u32,
    material: Option<&'a str>,
}

impl<'a> From<&'a ImpactPose> for StagedMark<'a> {
    fn from(pose: &'a ImpactPose) -> Self {
        Self {
            origin: pose.origin,
            radius: pose.radius,
            axis: pose.axis,
            native_color: pose.color,
            material: pose.material.as_deref(),
        }
    }
}

/// The runtime material called `name`, if the world has one.
pub(crate) fn runtime_material_by_name<'a>(
    scene: &'a WorldScene,
    name: &str,
) -> Option<&'a crate::assemble::drawsurf::RuntimeMaterial> {
    let want = asset_core::AssetRef::bare_name(name);
    scene
        .runtime_material_catalog
        .materials
        .iter()
        .find(|material| asset_core::AssetRef::bare_name(&material.name) == want)
}

/// What a material says about taking marks, for `mark_allow`.
pub(crate) fn mark_receiver(
    scene: &WorldScene,
    mat_slot: Option<assets::MaterialIndex>,
) -> Option<marks_t6::MarkReceiver> {
    let material = mat_slot.and_then(|id| scene.runtime_material_catalog.derived(id))?;
    Some(marks_t6::MarkReceiver {
        sort_key: material.sort_key,
        game_flags: material.info_game_flags,
        surface_type_bits: material.surface_type_bits,
    })
}

/// The surface type bits of the mark's own material.
fn mark_surface_bits(scene: &WorldScene, material: Option<&str>) -> Option<u32> {
    material
        .and_then(|name| runtime_material_by_name(scene, name))
        .and_then(|m| m.surface_type_bits)
}

/// BO2's `R_AllowMarks` for one surface: no decals, no no-marks materials,
/// and the surface type bits must hold the mark's.
fn surface_takes_mark(
    scene: &WorldScene,
    slot: Option<assets::MaterialIndex>,
    mark_bits: Option<u32>,
) -> bool {
    marks_t6::mark_include_in_world_clip(marks_t6::mark_allow(
        mark_receiver(scene, slot),
        mark_bits,
    ))
}

/// Which surfaces of the world a mark may touch, and what they are lit by.
#[derive(Default)]
struct WorldFilter {
    mids: Vec<[f32; 3]>,
    halves: Vec<[f32; 3]>,
    ranges: Vec<(u32, u32)>,
    contexts: Vec<GfxMarkContext>,
}

impl WorldFilter {
    fn clear(&mut self) {
        self.mids.clear();
        self.halves.clear();
        self.ranges.clear();
        self.contexts.clear();
    }

    fn push(&mut self, bounds: Bounds, range: (u32, u32), context: GfxMarkContext) {
        self.mids.push(bounds.mid);
        self.halves.push(bounds.half);
        self.ranges.push(range);
        self.contexts.push(context);
    }
}

/// The room the clip cuts a mark's triangles into.
#[derive(Default)]
struct Staging {
    tris: Vec<FxMarkStagingTri>,
    points: Vec<FxMarkStagingPoint>,
}

impl Staging {
    /// Both buffers at the size the clip is allowed to fill.
    fn open(&mut self) -> (&mut [FxMarkStagingTri], &mut [FxMarkStagingPoint]) {
        let tri_n = marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize;
        let point_n = marks_t6::R_MARK_FRAGMENTS_MAX_POINTS as usize;
        if self.tris.len() != tri_n {
            self.tris.clear();
            self.tris.resize(tri_n, FxMarkStagingTri::ZERO);
        }
        if self.points.len() != point_n {
            self.points.clear();
            self.points.resize(point_n, FxMarkStagingPoint::ZERO);
        }
        (&mut self.tris, &mut self.points)
    }
}

thread_local! {
    static WORLD_FILTER: RefCell<WorldFilter> = RefCell::new(WorldFilter::default());
    static STAGING: RefCell<Staging> = RefCell::new(Staging::default());
    /// Per world surface, its box from the retained positions; keyed by the
    /// positions it was built from.
    static SURFACE_BOUNDS: RefCell<(usize, Vec<Option<Bounds>>)> =
        const { RefCell::new((0, Vec::new())) };
}

/// The lighting a world surface gives a mark made on it.
fn world_surface_context(scene: &WorldScene, surface: usize) -> GfxMarkContext {
    let (lightmap, primary, probe) = match scene.cull.as_ref() {
        Some(cull) => (
            cull.surface_lightmap_indices
                .get(surface)
                .copied()
                .unwrap_or(marks_t6::GFX_SURFACE_LIGHTMAP_NONE),
            cull.surface_primary_lights
                .get(surface)
                .copied()
                .unwrap_or(0),
            cull.surface_reflection_probes
                .get(surface)
                .copied()
                .unwrap_or(0),
        ),
        None => (marks_t6::GFX_SURFACE_LIGHTMAP_NONE, 0, 0),
    };
    GfxMarkContext::world_surface(lightmap, primary, probe)
}

/// Fill the world filter with the surfaces the mark's sphere touches and
/// that take marks; how many there are. Every surface's box is tested.
fn fill_world_filter(scene: &WorldScene, pose: &ImpactPose) -> usize {
    let Some(cull) = scene.cull.as_ref() else {
        return 0;
    };
    let positions = &scene.retained_positions;
    let key = positions.as_ptr() as usize ^ positions.len() ^ cull.packed_indices.len();
    let mark_bits = mark_surface_bits(scene, pose.material.as_deref());
    let radius_sq = pose.radius * pose.radius;
    SURFACE_BOUNDS.with_borrow_mut(|cache| {
        if cache.0 != key {
            cache.1 = surface_bounds(positions, &cull.packed_indices, &cull.surface_index_ranges);
            cache.0 = key;
        }
        WORLD_FILTER.with_borrow_mut(|filter| {
            filter.clear();
            for (surface, bounds) in cache.1.iter().enumerate() {
                let Some(bounds) = bounds else {
                    continue;
                };
                if !marks_t6::mark_sphere_hits_bounds(
                    pose.origin,
                    radius_sq,
                    bounds.mid,
                    bounds.half,
                ) {
                    continue;
                }
                let Some(&range) = cull.surface_index_ranges.get(surface) else {
                    continue;
                };
                let slot = cull.surface_materials.get(surface).copied().flatten();
                if !surface_takes_mark(scene, slot, mark_bits) {
                    continue;
                }
                filter.push(*bounds, range, world_surface_context(scene, surface));
            }
            filter.ranges.len()
        })
    })
}

/// Cut the mark out of the surfaces in the world filter, into the staging.
fn stage_world_surfaces(scene: &WorldScene, mark: &StagedMark<'_>) -> Option<MarkWorldStaging> {
    let retained = scene.retained_positions.len();
    if retained == 0
        || scene.retained_lightmap_uvs.len() != retained
        || scene.retained_normals.len() != retained
    {
        return None;
    }
    let packed = scene
        .cull
        .as_ref()
        .map_or(&[][..], |cull| cull.packed_indices.as_slice());
    WORLD_FILTER.with_borrow(|filter| {
        if filter.ranges.is_empty() {
            return Some(MarkWorldStaging::default());
        }
        STAGING.with_borrow_mut(|staging| {
            let (tris, points) = staging.open();
            Some(marks_t6::mark_stage_world_surfaces(
                mark.origin,
                mark.radius,
                mark.axis,
                &filter.mids,
                &filter.halves,
                &scene.retained_positions,
                &scene.retained_lightmap_uvs,
                &scene.retained_normals,
                packed,
                &filter.ranges,
                &filter.contexts,
                marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
                marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
                tris,
                points,
                false,
            ))
        })
    })
}

/// Hand what the staging holds to the host as one mark: its handle, or none
/// when the staging is empty, overflowed, or the host has no room.
fn alloc_staged_mark(
    host: &mut FxSystemHost,
    mark: &StagedMark<'_>,
    staged: MarkWorldStaging,
) -> Option<u16> {
    if staged.used_tri == 0 || staged.overflow {
        return None;
    }
    let (first_context, tris, points) = STAGING.with_borrow(|staging| {
        let tri_end = (staged.used_tri as usize).min(staging.tris.len());
        let point_end = (staged.used_point as usize).min(staging.points.len());
        (
            staging.tris.first().map(|tri| tri.context),
            staging.tris[..tri_end].to_vec(),
            staging.points[..point_end].to_vec(),
        )
    });
    let request = FxAllocMarkRequest {
        any_marks: true,
        tri_count: staged.used_tri,
        point_count: staged.used_point,
        origin: mark.origin,
        radius: mark.radius,
        tex_coord_axis: mark.axis[1],
        native_color: mark.native_color,
        material: 0,
        frame_count: host.frame_stamp,
        first_tri_context: first_context?,
    };
    let handle = host
        .marks
        .alloc_mark_from_go_callback(request, &tris, &points)?;
    host.marks.set_material_name(handle, mark.material);
    Some(handle)
}

/// The world's part of an impact: the surfaces around it as one mark, then
/// each window the mark's box touches as a mark of its own.
fn mark_world(host: &mut FxSystemHost, scene: &WorldScene, def_index: u8) {
    let Some(pose) = impact_pose(host, MarkFragmentsAgainst::WorldBrushes) else {
        host.note_world_go_skip(def_index);
        return;
    };
    let mark = StagedMark::from(&pose);
    let mut fired = false;
    if fill_world_filter(scene, &pose) != 0
        && let Some(staged) = stage_world_surfaces(scene, &mark)
        && let Some(handle) = alloc_staged_mark(host, &mark, staged)
    {
        host.last_mark_alloc_slot = Some(handle);
        fired = true;
    }
    fired |= mark_glass(host, scene, &mark);
    if !fired {
        host.note_world_go_skip(def_index);
    }
}

/// The windows' part of a world mark: each pane the mark's box touches is
/// clipped as a brush model and flushed as its own mark on the world list,
/// with the pane as its model index so a shatter frees it. Pane `i` stands
/// for `Glass::index`, as in the shatter's detach and the snapshots.
fn mark_glass(host: &mut FxSystemHost, scene: &WorldScene, mark: &StagedMark<'_>) -> bool {
    let Some(glass) = scene.t6_glass.as_ref() else {
        return false;
    };
    let mins = mark.origin.map(|c| c - mark.radius);
    let maxs = mark.origin.map(|c| c + mark.radius);
    let picked = mark_glass_panes(&glass.panes, |p| host.glass.pane_broken(p), mins, maxs);
    let mark_bits = mark_surface_bits(scene, mark.material);
    let surface_materials = scene
        .cull
        .as_ref()
        .map_or(&[][..], |cull| cull.surface_materials.as_slice());
    let takes = |surface: u32| {
        let slot = surface_materials.get(surface as usize).copied().flatten();
        surface_takes_mark(scene, slot, mark_bits)
    };
    let mut fired = false;
    for index in picked {
        let staged = stage_glass_pane(&glass.panes[index], index as u16, mark, &takes);
        if let Some(handle) = alloc_staged_mark(host, mark, staged) {
            host.last_mark_alloc_slot.get_or_insert(handle);
            fired = true;
        }
    }
    fired
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
) -> MarkWorldStaging {
    let surfaces: Vec<_> = pane.surfaces.iter().filter(|s| takes(s.surface)).collect();
    let positions: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.position).collect();
    let lightmap_uvs: Vec<[f32; 2]> = pane.vertices.iter().map(|v| v.lightmap_uv).collect();
    let normals: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.normal).collect();
    let ranges: Vec<(u32, u32)> = surfaces.iter().map(|s| (s.start, s.count)).collect();
    let contexts: Vec<GfxMarkContext> = surfaces
        .iter()
        .map(|s| {
            GfxMarkContext::glass_pane(s.lightmap, s.primary_light, s.reflection_probe, pane_id)
        })
        .collect();
    // The pane's own box test stands in for the per-surface sphere test.
    let unused = vec![[0.0; 3]; ranges.len()];
    STAGING.with_borrow_mut(|staging| {
        let (tris, points) = staging.open();
        marks_t6::mark_stage_world_surfaces(
            mark.origin,
            mark.radius,
            mark.axis,
            &unused,
            &unused,
            &positions,
            &lightmap_uvs,
            &normals,
            &pane.indices,
            &ranges,
            &contexts,
            marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
            marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
            tris,
            points,
            false,
        )
    })
}

/// Per static-model mesh, its box in model space; keyed by the mark copy it
/// came from. Built once per map and shared by every thread that marks.
static SMODEL_BOUNDS: Mutex<(usize, Option<MeshBounds>)> = Mutex::new((0, None));

/// The box of each mesh, if it has a point.
type MeshBounds = Arc<Vec<Option<Bounds>>>;

/// The mesh boxes of `cpu`, built on first use.
fn smodel_bounds(cpu: &SmodelMarkCpu) -> Option<MeshBounds> {
    let key = cpu.positions.as_ptr() as usize ^ cpu.positions.len() ^ cpu.meshes.len();
    let mut slot = SMODEL_BOUNDS.lock().ok()?;
    if slot.0 == key
        && let Some(bounds) = slot.1.as_ref()
    {
        return Some(Arc::clone(bounds));
    }
    let bounds: Vec<Option<Bounds>> = cpu
        .meshes
        .iter()
        .map(|surfaces| {
            let mut extent = Extent::EMPTY;
            for surface in surfaces {
                let run = index_run(&cpu.indices, surface.index_start, surface.index_count);
                extend_by_indices(&mut extent, &cpu.positions, run);
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
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    filter: WorldFilter,
}

impl PropTriangles {
    fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.indices.clear();
        self.filter.clear();
    }

    /// Add a triangle, flat shaded; `false` when it has no area.
    fn push_triangle(&mut self, corners: [[f32; 3]; 3]) -> bool {
        let Some(normal) = triangle_normal(corners) else {
            return false;
        };
        for corner in corners {
            self.indices.push(self.positions.len() as u32);
            self.positions.push(corner);
            self.normals.push(normal);
            self.uvs.push([0.0, 0.0]);
        }
        true
    }

    /// Close the triangles pushed since `start` as one surface.
    fn finish_surface(&mut self, start: u32, bounds: Bounds, context: GfxMarkContext) {
        let count = self.indices.len() as u32 - start;
        if count >= 3 {
            self.filter.push(bounds, (start, count), context);
        }
    }
}

thread_local! {
    static PROP_TRIANGLES: RefCell<PropTriangles> = RefCell::new(PropTriangles::default());
}

/// Do props' surfaces take marks? Glass, decals and other blended or unlit
/// surfaces do not.
fn prop_takes_marks(scene: &WorldScene, material: Option<assets::MaterialIndex>) -> bool {
    material
        .and_then(|id| scene.runtime_material_catalog.derived(id))
        .and_then(|m| m.t6_draw)
        .is_some_and(|draw| matches!(draw.blend, asset_core::T6Blend::Opaque) && !draw.unlit)
}

/// Gather the triangles of every prop whose box the mark's sphere touches.
fn gather_prop_triangles(
    scene: &WorldScene,
    cpu: &SmodelMarkCpu,
    bounds: &[Option<Bounds>],
    pose: &ImpactPose,
    tris: &mut PropTriangles,
) {
    let radius_sq = pose.radius * pose.radius;
    for instance in scene.static_model_instances.iter().flatten() {
        let Some(Some(local)) = bounds.get(instance.mesh) else {
            continue;
        };
        let matrix = instance.transform.to_matrix();
        let placed = transformed_bounds(&matrix, local);
        if !marks_t6::mark_sphere_hits_bounds(pose.origin, radius_sq, placed.mid, placed.half) {
            continue;
        }
        let Some(surfaces) = cpu.meshes.get(instance.mesh) else {
            continue;
        };
        let context = GfxMarkContext::world_surface(
            marks_t6::GFX_SURFACE_LIGHTMAP_NONE,
            instance.primary_light_index,
            instance.reflection_probe_index,
        );
        let place = |p: [f32; 3]| matrix.transform_point3(Vec3::from_array(p)).to_array();
        for surface in surfaces {
            if !prop_takes_marks(scene, surface.material) {
                continue;
            }
            let start = tris.indices.len() as u32;
            let run = index_run(&cpu.indices, surface.index_start, surface.index_count);
            for &[ia, ib, ic] in run.as_chunks::<3>().0 {
                let at = |i: u32| cpu.positions.get(i as usize).copied().map(place);
                if let (Some(a), Some(b), Some(c)) = (at(ia), at(ib), at(ic)) {
                    tris.push_triangle([a, b, c]);
                }
            }
            tris.finish_surface(start, placed, context);
        }
    }
}

/// The props' part of an impact. Bullets stop at the map's props (their
/// collision), so their holes go on the props as BO2's do: every opaque
/// first-LOD triangle of the props whose box the mark's sphere touches,
/// placed in the world and clipped to the mark's box as world surfaces are.
/// Marks are made with the effect's `mc_` materials and lit by the light grid.
fn mark_props(host: &mut FxSystemHost, scene: &WorldScene, def_index: u8) {
    let began = std::time::Instant::now();
    let fired = mark_props_inner(host, scene);
    match fired {
        Some(handle) => {
            host.last_mark_alloc_slot.get_or_insert(handle);
        }
        None => host.note_models_go_skip(def_index),
    }
    let ms = began.elapsed().as_secs_f64() * 1000.0;
    if ms > 4.0 {
        diag::warn!(World, "prop marks: slow mark on props, {ms:.1} ms");
    }
}

fn mark_props_inner(host: &mut FxSystemHost, scene: &WorldScene) -> Option<u16> {
    let pose = impact_pose(host, MarkFragmentsAgainst::Models)?;
    let cpu = scene.smodel_mark_cpu.as_ref()?;
    let bounds = smodel_bounds(cpu)?;
    let mark = StagedMark::from(&pose);
    let staged = PROP_TRIANGLES.with_borrow_mut(|tris| {
        tris.clear();
        gather_prop_triangles(scene, cpu, &bounds, &pose, tris);
        if tris.filter.ranges.is_empty() {
            return None;
        }
        STAGING.with_borrow_mut(|staging| {
            let (out_tris, out_points) = staging.open();
            Some(marks_t6::mark_stage_world_surfaces(
                mark.origin,
                mark.radius,
                mark.axis,
                &tris.filter.mids,
                &tris.filter.halves,
                &tris.positions,
                &tris.uvs,
                &tris.normals,
                &tris.indices,
                &tris.filter.ranges,
                &tris.filter.contexts,
                marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
                marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
                out_tris,
                out_points,
                false,
            ))
        })
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
            normal: [0.0, 0.0, 1.0],
            ..Default::default()
        };
        let glass = asset_world::T6GlassPaneDraw {
            vertices: vec![vertex(-50.0, -50.0), vertex(50.0, -50.0), vertex(0.0, 50.0)],
            indices: vec![0, 1, 2],
            surfaces: vec![asset_world::T6GlassSurface {
                surface: 9,
                start: 0,
                count: 3,
                lightmap: 4,
                primary_light: 5,
                reflection_probe: 6,
            }],
            ..pane(-50.0, 50.0)
        };
        let mark = StagedMark {
            origin: [0.0, 0.0, 1.0],
            radius: 8.0,
            axis: [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            native_color: 0,
            material: None,
        };
        let staged = stage_glass_pane(&glass, 7, &mark, &|surface| surface == 9);
        assert!(staged.used_tri > 0 && !staged.overflow, "{staged:?}");
        let context = STAGING.with_borrow(|s| s.tris[0].context);
        assert_eq!(context, GfxMarkContext::glass_pane(4, 5, 6, 7));
        let skipped = stage_glass_pane(&glass, 7, &mark, &|_| false);
        assert_eq!(skipped.used_tri, 0);
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
        let mut extent = Extent::EMPTY;
        assert_eq!(extent.bounds(), None);
        extent.include([1.0, 2.0, 3.0]);
        extent.include([3.0, -2.0, 3.0]);
        assert_eq!(
            extent.bounds(),
            Some(Bounds {
                mid: [2.0, 0.0, 3.0],
                half: [1.0, 2.0, 0.0],
            })
        );
    }

    #[test]
    fn surface_boxes_follow_their_index_runs_and_skip_bad_indices() {
        let positions = [[0.0; 3], [2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [100.0; 3]];
        let indices = [0, 1, 2, 9, 3, 3];
        let boxes = surface_bounds(&positions, &indices, &[(0, 4), (3, 1), (4, 2), (8, 2)]);
        assert_eq!(
            boxes[0],
            Some(Bounds {
                mid: [1.0, 2.0, 0.0],
                half: [1.0, 2.0, 0.0],
            })
        );
        assert_eq!(boxes[1], None, "an index past the positions is ignored");
        assert_eq!(
            boxes[2],
            Some(Bounds {
                mid: [100.0; 3],
                half: [0.0; 3],
            })
        );
        assert_eq!(boxes[3], None, "a run past the list is empty");
    }

    #[test]
    fn a_rotated_box_grows_to_hold_the_original() {
        let bounds = Bounds {
            mid: [1.0, 0.0, 0.0],
            half: [2.0, 1.0, 0.5],
        };
        let turned = transformed_bounds(
            &Mat4::from_rotation_z(core::f32::consts::FRAC_PI_2),
            &bounds,
        );
        assert!((turned.mid[0]).abs() < 1e-5 && (turned.mid[1] - 1.0).abs() < 1e-5);
        assert!((turned.half[0] - 1.0).abs() < 1e-5);
        assert!((turned.half[1] - 2.0).abs() < 1e-5);
        assert!((turned.half[2] - 0.5).abs() < 1e-5);
        let moved = transformed_bounds(&Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)), &bounds);
        assert_eq!(moved.mid, [6.0, 0.0, 0.0]);
        assert_eq!(moved.half, bounds.half);
    }

    #[test]
    fn triangle_normals_are_unit_and_thin_triangles_have_none() {
        let normal = triangle_normal([[0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]]).unwrap();
        assert_eq!(normal, [0.0, 0.0, 1.0]);
        assert_eq!(
            triangle_normal([[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]),
            None
        );
        assert_eq!(triangle_normal([[1.0; 3]; 3]), None);
    }

    #[test]
    fn prop_triangles_are_flat_shaded_and_closed_per_surface() {
        let mut tris = PropTriangles::default();
        let bounds = Bounds {
            mid: [0.0; 3],
            half: [1.0; 3],
        };
        let context = GfxMarkContext::world_surface(marks_t6::GFX_SURFACE_LIGHTMAP_NONE, 1, 2);
        let start = tris.indices.len() as u32;
        assert!(tris.push_triangle([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        assert!(!tris.push_triangle([[0.0; 3]; 3]));
        tris.finish_surface(start, bounds, context);
        assert_eq!(tris.filter.ranges, vec![(0, 3)]);
        assert_eq!(tris.normals, vec![[0.0, 0.0, 1.0]; 3]);
        let empty_start = tris.indices.len() as u32;
        tris.finish_surface(empty_start, bounds, context);
        assert_eq!(
            tris.filter.ranges.len(),
            1,
            "a surface with no triangle is dropped"
        );
        tris.clear();
        assert!(tris.positions.is_empty() && tris.filter.ranges.is_empty());
    }

    #[test]
    fn the_staging_opens_to_full_size_once() {
        let mut staging = Staging::default();
        let (tris, points) = staging.open();
        assert_eq!(tris.len(), marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize);
        assert_eq!(points.len(), marks_t6::R_MARK_FRAGMENTS_MAX_POINTS as usize);
    }
}
