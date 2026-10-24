use std::cell::RefCell;

use fx::FxSystemHost;
use marks_t6::{FxAllocMarkRequest, MarkFragmentsAgainst, impact_mark_material};

use crate::prepare::scene::world::WorldScene;
use render_fx::EntityMarks;

pub struct FrontendFxScene<'a> {
    world: &'a WorldScene,
    marks: &'a EntityMarks,
}

impl<'a> FrontendFxScene<'a> {
    pub fn wrap(scene: Option<&'a WorldScene>, marks: &'a EntityMarks) -> Option<Self> {
        scene.map(|world| Self { world, marks })
    }
}

struct MarkClipScratch {
    mids: Vec<[f32; 3]>,
    halves: Vec<[f32; 3]>,
    ranges: Vec<(u32, u32)>,
    contexts: Vec<marks_t6::GfxMarkContext>,
    tris: Vec<marks_t6::FxMarkStagingTri>,
    points: Vec<marks_t6::FxMarkStagingPoint>,
}

impl MarkClipScratch {
    fn ensure_stage_bufs(&mut self) {
        let tri_n = marks_t6::R_MARK_FRAGMENTS_MAX_TRIS as usize;
        let point_n = marks_t6::R_MARK_FRAGMENTS_MAX_POINTS as usize;
        if self.tris.len() != tri_n {
            self.tris.clear();
            self.tris.resize(tri_n, marks_t6::FxMarkStagingTri::ZERO);
        }
        if self.points.len() != point_n {
            self.points.clear();
            self.points
                .resize(point_n, marks_t6::FxMarkStagingPoint::ZERO);
        }
    }

    fn clear_filter(&mut self) {
        self.mids.clear();
        self.halves.clear();
        self.ranges.clear();
        self.contexts.clear();
    }
}

thread_local! {
    static MARK_CLIP_SCRATCH: RefCell<MarkClipScratch> = const {
        RefCell::new(MarkClipScratch {
            mids: Vec::new(),
            halves: Vec::new(),
            ranges: Vec::new(),
            contexts: Vec::new(),
            tris: Vec::new(),
            points: Vec::new(),
        })
    };
}

fn pose_material<'a>(host: &'a FxSystemHost, against: MarkFragmentsAgainst) -> Option<&'a str> {
    impact_mark_material(
        [
            host.last_decal_mat0.as_deref(),
            host.last_decal_mat1.as_deref(),
        ],
        against,
    )
}

pub(crate) struct WorldMarkPose<'a> {
    pub origin: [f32; 3],
    pub radius: f32,
    pub axis: Option<[[f32; 3]; 3]>,

    pub material: Option<&'a str>,
}

struct SurfaceMarkLookup {
    lmap: u8,
    primary: u8,
    probe: u8,
}

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

fn lookup_world_surface_mark(scene: &WorldScene, surf: usize) -> SurfaceMarkLookup {
    let none = SurfaceMarkLookup {
        lmap: marks_t6::GFX_SURFACE_LIGHTMAP_NONE,
        primary: 0,
        probe: 0,
    };
    let Some(cull) = scene.cull.as_ref() else {
        return none;
    };
    let primary = cull.surface_primary_lights.get(surf).copied().unwrap_or(0);
    let lmap = cull
        .surface_lightmap_indices
        .get(surf)
        .copied()
        .unwrap_or(marks_t6::GFX_SURFACE_LIGHTMAP_NONE);
    let probe = cull
        .surface_reflection_probes
        .get(surf)
        .copied()
        .unwrap_or(0);
    SurfaceMarkLookup {
        lmap,
        primary,
        probe,
    }
}

fn first_tri_context(tri: &marks_t6::FxMarkStagingTri) -> marks_t6::GfxMarkContext {
    tri.context
}

fn impact_mark_axis(host: &FxSystemHost) -> Option<[[f32; 3]; 3]> {
    Some(fx_t6::runtime::impact_mark_axis(
        host.last_decal_axis?,
        host.last_decal_rotation?,
    ))
}

fn pose_from_fx_host(host: &FxSystemHost) -> Option<WorldMarkPose<'_>> {
    if host.last_decal_against_world != Some(true) {
        return None;
    }
    Some(WorldMarkPose {
        origin: host.last_decal_origin?,
        radius: host.last_decal_size0?,
        axis: Some(impact_mark_axis(host)?),
        material: pose_material(host, MarkFragmentsAgainst::WorldBrushes),
    })
}

thread_local! {
    /// bo2zm: per world surface, its bounds (mid, half), from the retained
    /// positions; keyed by the positions they were built from.
    static T6_SURFACE_BOUNDS: std::cell::RefCell<(usize, Vec<([f32; 3], [f32; 3])>)> =
        const { std::cell::RefCell::new((0, Vec::new())) };
}

/// bo2zm: the world surfaces a mark's sphere touches, into the clip scratch
/// (every surface's bounds tested; Nuketown has under 10k).
fn fill_world_filter_t6(scene: &WorldScene, pose: &WorldMarkPose<'_>) -> Option<usize> {
    let cull = scene.cull.as_ref()?;
    let positions = &scene.retained_positions;
    let key = positions.as_ptr() as usize ^ positions.len() ^ cull.packed_indices.len();
    T6_SURFACE_BOUNDS.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.0 != key {
            cache.1 = cull
                .surface_index_ranges
                .iter()
                .map(|&(start, count)| {
                    let mut lo = [f32::MAX; 3];
                    let mut hi = [f32::MIN; 3];
                    let s = start as usize;
                    let e = s.saturating_add(count as usize).min(cull.packed_indices.len());
                    for &i in &cull.packed_indices[s.min(e)..e] {
                        if let Some(p) = positions.get(i as usize) {
                            for a in 0..3 {
                                lo[a] = lo[a].min(p[a]);
                                hi[a] = hi[a].max(p[a]);
                            }
                        }
                    }
                    if lo[0] > hi[0] {
                        return ([0.0; 3], [-1.0; 3]);
                    }
                    (
                        [0, 1, 2].map(|a| (lo[a] + hi[a]) * 0.5),
                        [0, 1, 2].map(|a| (hi[a] - lo[a]) * 0.5),
                    )
                })
                .collect();
            cache.0 = key;
        }
        let radius_sq = pose.radius * pose.radius;
        let mark_bits = pose
            .material
            .and_then(|name| runtime_material_by_name(scene, name))
            .and_then(|m| m.surface_type_bits);
        MARK_CLIP_SCRATCH.with(|clip_cell| {
            let mut clip = clip_cell.borrow_mut();
            clip.clear_filter();
            for (surf, &(mid, half)) in cache.1.iter().enumerate() {
                if half[0] < 0.0
                    || !marks_t6::mark_sphere_hits_bounds(pose.origin, radius_sq, mid, half)
                {
                    continue;
                }
                let Some(&(start, count)) = cull.surface_index_ranges.get(surf) else {
                    continue;
                };
                // BO2's R_AllowMarks per surface: no decals, no no-marks
                // materials, and the surface type bits must hold the mark's.
                let slot = cull.surface_materials.get(surf).copied().flatten();
                if !marks_t6::mark_include_in_world_clip(marks_t6::mark_allow(
                    mark_receiver(scene, slot),
                    mark_bits,
                )) {
                    continue;
                }
                clip.mids.push(mid);
                clip.halves.push(half);
                clip.ranges.push((start, count));
                let lookup = lookup_world_surface_mark(scene, surf);
                clip.contexts.push(marks_t6::GfxMarkContext::world_surface(
                    lookup.lmap,
                    lookup.primary,
                    lookup.probe,
                ));
            }
            Some(clip.ranges.len())
        })
    })
}

/// bo2zm: `complete_world_generate` for a Black Ops II world: the world
/// brushes around the impact as one mark, then each window the mark's box
/// touches as a mark of its own.
fn complete_world_generate_t6(host: &mut FxSystemHost, scene: &WorldScene, def_index: u8) {
    let Some(pose) = pose_from_fx_host(host) else {
        host.note_world_go_skip(def_index);
        return;
    };
    let (origin, radius) = (pose.origin, pose.radius);
    let Some(axis) = pose.axis else {
        host.note_world_go_skip(def_index);
        return;
    };
    let material = pose.material.map(str::to_owned);
    let Some(native_color) = host.last_decal_color else {
        host.note_world_go_skip(def_index);
        return;
    };
    let mark = StagedMark {
        origin,
        radius,
        axis,
        native_color,
        material: material.as_deref(),
    };
    let mut fired = false;
    if fill_world_filter_t6(scene, &pose).is_some_and(|n| n != 0)
        && let Some(staging) = stage_clip_retained(scene, origin, radius, axis)
        && let Some(handle) = alloc_staged_mark(host, &mark, staging)
    {
        host.last_mark_alloc_slot = Some(handle);
        fired = true;
    }
    fired |= complete_glass_generate_t6(host, scene, &mark);
    if fired {
        host.note_world_go_fire();
    } else {
        host.note_world_go_skip(def_index);
    }
}

/// What every part of one impact mark shares.
struct StagedMark<'a> {
    origin: [f32; 3],
    radius: f32,
    axis: [[f32; 3]; 3],
    native_color: u32,
    material: Option<&'a str>,
}

/// The mark callback for what the clip scratch holds: one mark of
/// `staging`'s triangles, or none when it is empty or overflowed.
fn alloc_staged_mark(
    host: &mut FxSystemHost,
    mark: &StagedMark<'_>,
    staging: marks_t6::MarkWorldStaging,
) -> Option<u16> {
    if staging.used_tri == 0 || staging.overflow {
        return None;
    }
    let used_t = staging.used_tri as usize;
    let used_p = staging.used_point as usize;
    let (first_ctx, tris, points) = MARK_CLIP_SCRATCH.with(|clip_cell| {
        let clip = clip_cell.borrow();
        let ctx = clip.tris.first().map(first_tri_context);
        (
            ctx,
            clip.tris[..used_t.min(clip.tris.len())].to_vec(),
            clip.points[..used_p.min(clip.points.len())].to_vec(),
        )
    });
    let req = FxAllocMarkRequest {
        any_marks: true,
        tri_count: staging.used_tri,
        point_count: staging.used_point,
        origin: mark.origin,
        radius: mark.radius,
        tex_coord_axis: mark.axis[1],
        native_color: mark.native_color,
        material: 0,
        frame_count: host.frame_stamp,
        first_tri_context: first_ctx?,
    };
    let handle = host.marks.alloc_mark_from_go_callback(req, &tris, &points)?;
    host.marks.set_material_name(handle, mark.material);
    Some(handle)
}

/// BO2 takes marks on at most this many windows per impact.
const MARK_GLASS_PANE_LIMIT: usize = 8;

/// BO2's `AreaGlasses` for a mark: the windows not shattered (pristine or
/// cracked) whose bounds overlap the mark's box, in pane order, the first
/// `MARK_GLASS_PANE_LIMIT`.
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

/// The windows' part of a world mark: each pane the mark's box touches is
/// clipped as a brush model and flushed as its own mark, on the world list
/// with the pane as its modelIndex, so a shatter frees it. Pane `i` stands
/// for `Glass::index` here, as it does in the shatter's detach and the
/// snapshots.
fn complete_glass_generate_t6(host: &mut FxSystemHost, scene: &WorldScene, mark: &StagedMark<'_>) -> bool {
    let Some(glass) = scene.t6_glass.as_ref() else {
        return false;
    };
    let mins = [0, 1, 2].map(|a| mark.origin[a] - mark.radius);
    let maxs = [0, 1, 2].map(|a| mark.origin[a] + mark.radius);
    let picked = mark_glass_panes(&glass.panes, |p| host.glass.pane_broken(p), mins, maxs);
    // BO2 skips a pane surface whose material has the decal sort key or the
    // no-marks game flag, or whose surfaceTypeBits miss any of the mark
    // material's.
    let mark_bits = mark
        .material
        .and_then(|name| runtime_material_by_name(scene, name))
        .and_then(|m| m.surface_type_bits);
    let surface_materials = scene.cull.as_ref().map_or(&[][..], |c| c.surface_materials.as_slice());
    let takes = |surf: u32| {
        let slot = surface_materials.get(surf as usize).copied().flatten();
        marks_t6::mark_include_in_world_clip(marks_t6::mark_allow(mark_receiver(scene, slot), mark_bits))
    };
    let mut fired = false;
    for i in picked {
        let staging = stage_clip_glass_pane(&glass.panes[i], i as u16, mark, &takes);
        if let Some(handle) = alloc_staged_mark(host, mark, staging) {
            if host.last_mark_alloc_slot.is_none() {
                host.last_mark_alloc_slot = Some(handle);
            }
            fired = true;
        }
    }
    fired
}

/// One window's surfaces through the brush-surface clip, into the clip
/// scratch. BO2 moves the mark into the pane's own space and the points
/// back out; the pane's vertices are already in world space, so this clips
/// there, the same up to rounding. Lightmap, primary light and probe come
/// from each surface, as a world brush's do. `takes` says whether a world
/// surface's material takes the mark.
fn stage_clip_glass_pane(
    pane: &asset_world::T6GlassPaneDraw,
    pane_id: u16,
    mark: &StagedMark<'_>,
    takes: &dyn Fn(u32) -> bool,
) -> marks_t6::MarkWorldStaging {
    let surfaces: Vec<_> = pane.surfaces.iter().filter(|s| takes(s.surface)).collect();
    let positions: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.position).collect();
    let lightmap_uvs: Vec<[f32; 2]> = pane.vertices.iter().map(|v| v.lightmap_uv).collect();
    let normals: Vec<[f32; 3]> = pane.vertices.iter().map(|v| v.normal).collect();
    let ranges: Vec<(u32, u32)> = surfaces.iter().map(|s| (s.start, s.count)).collect();
    let contexts: Vec<marks_t6::GfxMarkContext> = surfaces
        .iter()
        .map(|s| {
            marks_t6::GfxMarkContext::glass_pane(s.lightmap, s.primary_light, s.reflection_probe, pane_id)
        })
        .collect();
    // The pane's box test stands in for the per-surface sphere test.
    let unused = vec![[0.0; 3]; ranges.len()];
    MARK_CLIP_SCRATCH.with(|clip_cell| {
        let mut clip = clip_cell.borrow_mut();
        clip.ensure_stage_bufs();
        let MarkClipScratch { tris, points, .. } = &mut *clip;
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

/// bo2zm fix list 2: per static model mesh, its first-LOD bounds in model
/// space (mid, half); keyed by the mark CPU copy they came from. Built once
/// per map and shared by every thread that makes marks.
static T6_SMODEL_BOUNDS: std::sync::Mutex<(usize, Option<std::sync::Arc<Vec<([f32; 3], [f32; 3])>>>)> =
    std::sync::Mutex::new((0, None));

thread_local! {
    /// The props' triangles around one mark, in world space.
    static T6_SMODEL_TRIS: std::cell::RefCell<T6SmodelTris> =
        std::cell::RefCell::new(T6SmodelTris::default());
}

/// The mesh bounds of `cpu`, building them on first use.
fn t6_smodel_bounds(
    cpu: &crate::prepare::scene::world::SmodelMarkCpu,
) -> Option<std::sync::Arc<Vec<([f32; 3], [f32; 3])>>> {
    let key = cpu.positions.as_ptr() as usize ^ cpu.positions.len() ^ cpu.meshes.len();
    let mut slot = T6_SMODEL_BOUNDS.lock().ok()?;
    if slot.0 == key
        && let Some(bounds) = slot.1.as_ref()
    {
        return Some(std::sync::Arc::clone(bounds));
    }
    let bounds: Vec<([f32; 3], [f32; 3])> = cpu
        .meshes
        .iter()
        .map(|surfs| {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for s in surfs {
                let a = s.index_start as usize;
                let b = a.saturating_add(s.index_count as usize).min(cpu.indices.len());
                for &i in &cpu.indices[a.min(b)..b] {
                    if let Some(p) = cpu.positions.get(i as usize) {
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                }
            }
            if lo[0] > hi[0] {
                return ([0.0; 3], [-1.0; 3]);
            }
            (
                [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5),
                [0, 1, 2].map(|k| (hi[k] - lo[k]) * 0.5),
            )
        })
        .collect();
    let bounds = std::sync::Arc::new(bounds);
    *slot = (key, Some(std::sync::Arc::clone(&bounds)));
    Some(bounds)
}

#[derive(Default)]
struct T6SmodelTris {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    mids: Vec<[f32; 3]>,
    halves: Vec<[f32; 3]>,
    ranges: Vec<(u32, u32)>,
    contexts: Vec<marks_t6::GfxMarkContext>,
}

/// bo2zm fix list 2: `complete_models_generate` for a Black Ops II world.
/// Bullets stop at the map's props now (their collision), so their holes go
/// on the props as BO2's do (the effect's `mc_` mark materials, lit by the
/// light grid like the props): every opaque first-LOD triangle of the props
/// whose bounds the mark's sphere touches, placed in the world and clipped
/// to the mark's box as world surfaces are.
fn complete_models_generate_t6(host: &mut FxSystemHost, scene: &WorldScene, def_index: u8) {
    let began = std::time::Instant::now();
    complete_models_generate_t6_inner(host, scene, def_index);
    let ms = began.elapsed().as_secs_f64() * 1000.0;
    if ms > 4.0 {
        diag::warn!(World, "bo2zm prop marks: slow mark on props, {ms:.1} ms");
    }
}

fn complete_models_generate_t6_inner(host: &mut FxSystemHost, scene: &WorldScene, def_index: u8) {
    let Some(pose) = pose_from_fx_host_models(host) else {
        host.note_models_go_skip(def_index);
        return;
    };
    let (origin, radius) = (pose.origin, pose.radius);
    let Some(axis) = pose.axis else {
        host.note_models_go_skip(def_index);
        return;
    };
    let Some(native_color) = host.last_decal_color else {
        host.note_models_go_skip(def_index);
        return;
    };
    let material = pose.material.map(str::to_owned);
    let Some(cpu) = scene.smodel_mark_cpu.as_ref() else {
        host.note_models_go_skip(def_index);
        return;
    };
    let radius_sq = radius * radius;
    let Some(bounds) = t6_smodel_bounds(cpu) else {
        host.note_models_go_skip(def_index);
        return;
    };
    let staged = {
        T6_SMODEL_TRIS.with(|tris_cell| {
            let mut t = tris_cell.borrow_mut();
            let t = &mut *t;
            t.positions.clear();
            t.normals.clear();
            t.uvs.clear();
            t.indices.clear();
            t.mids.clear();
            t.halves.clear();
            t.ranges.clear();
            t.contexts.clear();
            for instance in scene.static_model_instances.iter().flatten() {
                let Some(&(mid, half)) = bounds.get(instance.mesh) else {
                    continue;
                };
                if half[0] < 0.0 {
                    continue;
                }
                // The placement's world box, by the transform it is drawn with.
                let m = instance.transform.to_matrix();
                let cols = [m.x_axis, m.y_axis, m.z_axis];
                let world_mid = m
                    .transform_point3(bevy::math::Vec3::from_array(mid))
                    .to_array();
                let world_half: [f32; 3] = core::array::from_fn(|k| {
                    (0..3).map(|j| cols[j][k].abs() * half[j]).sum::<f32>()
                });
                if !marks_t6::mark_sphere_hits_bounds(origin, radius_sq, world_mid, world_half) {
                    continue;
                }
                let Some(surfs) = cpu.meshes.get(instance.mesh) else {
                    continue;
                };
                let context = marks_t6::GfxMarkContext::world_surface(
                    marks_t6::GFX_SURFACE_LIGHTMAP_NONE,
                    instance.primary_light_index,
                    instance.reflection_probe_index,
                );
                let place = |p: [f32; 3]| -> [f32; 3] {
                    m.transform_point3(bevy::math::Vec3::from_array(p)).to_array()
                };
                for surf in surfs {
                    // Glass, decals and other blended surfaces take no marks.
                    let opaque = surf
                        .material
                        .and_then(|id| scene.runtime_material_catalog.derived(id))
                        .and_then(|m| m.t6_draw)
                        .is_some_and(|d| matches!(d.blend, asset_core::T6Blend::Opaque) && !d.unlit);
                    if !opaque {
                        continue;
                    }
                    let a = surf.index_start as usize;
                    let b = a.saturating_add(surf.index_count as usize).min(cpu.indices.len());
                    let start = t.indices.len() as u32;
                    for tri in cpu.indices[a.min(b)..b].chunks_exact(3) {
                        let (Some(&l0), Some(&l1), Some(&l2)) = (
                            cpu.positions.get(tri[0] as usize),
                            cpu.positions.get(tri[1] as usize),
                            cpu.positions.get(tri[2] as usize),
                        ) else {
                            continue;
                        };
                        let w = [place(l0), place(l1), place(l2)];
                        let e1: [f32; 3] = core::array::from_fn(|k| w[1][k] - w[0][k]);
                        let e2: [f32; 3] = core::array::from_fn(|k| w[2][k] - w[0][k]);
                        let n = [
                            e1[1] * e2[2] - e1[2] * e2[1],
                            e1[2] * e2[0] - e1[0] * e2[2],
                            e1[0] * e2[1] - e1[1] * e2[0],
                        ];
                        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                        if len < 1e-6 {
                            continue;
                        }
                        let n = n.map(|c| c / len);
                        for p in w {
                            t.indices.push(t.positions.len() as u32);
                            t.positions.push(p);
                            t.normals.push(n);
                            t.uvs.push([0.0, 0.0]);
                        }
                    }
                    let count = t.indices.len() as u32 - start;
                    if count >= 3 {
                        t.ranges.push((start, count));
                        t.mids.push(world_mid);
                        t.halves.push(world_half);
                        t.contexts.push(context);
                    }
                }
            }
            if t.ranges.is_empty() {
                return None;
            }
            MARK_CLIP_SCRATCH.with(|clip_cell| {
                let mut clip = clip_cell.borrow_mut();
                clip.ensure_stage_bufs();
                let MarkClipScratch { tris, points, .. } = &mut *clip;
                let staging = marks_t6::mark_stage_world_surfaces(
                    origin,
                    radius,
                    axis,
                    &t.mids,
                    &t.halves,
                    &t.positions,
                    &t.uvs,
                    &t.normals,
                    &t.indices,
                    &t.ranges,
                    &t.contexts,
                    marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
                    marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
                    tris,
                    points,
                    false,
                );
                if staging.used_tri == 0 || staging.overflow {
                    return None;
                }
                let (ut, up) = (staging.used_tri as usize, staging.used_point as usize);
                Some((staging, tris[..ut].to_vec(), points[..up].to_vec()))
            })
        })
    };
    let Some((staging, tris, points)) = staged else {
        host.note_models_go_skip(def_index);
        return;
    };
    let Some(first_ctx) = tris.first().map(first_tri_context) else {
        host.note_models_go_skip(def_index);
        return;
    };
    let req = FxAllocMarkRequest {
        any_marks: true,
        tri_count: staging.used_tri,
        point_count: staging.used_point,
        origin,
        radius,
        tex_coord_axis: axis[1],
        native_color,
        material: 0,
        frame_count: host.frame_stamp,
        first_tri_context: first_ctx,
    };
    match host.marks.alloc_mark_from_go_callback(req, &tris, &points) {
        Some(handle) => {
            host.marks.set_material_name(handle, material.as_deref());
            if host.last_mark_alloc_slot.is_none() {
                host.last_mark_alloc_slot = Some(handle);
            }
            host.note_models_go_fire();
        }
        None => host.note_models_go_skip(def_index),
    }
}

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
    // bo2zm: a Black Ops II world clips marks against the surfaces around
    // the impact directly.
    if against_world {
        complete_world_generate_t6(host, scene, def_index);
    }
    if against_models {
        complete_models_generate_t6(host, scene, def_index);
    }
}

impl render_fx::present::FxScene for FrontendFxScene<'_> {
    fn sample_atpoint_rgb(&self, origin: [f32; 3]) -> Option<[u8; 3]> {
        self.world
            .light_grid
            .as_ref()
            .and_then(|grid| asset_model::sample_light_grid(&grid.view(), origin).ok())
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

fn stage_clip_retained(
    scene: &WorldScene,
    origin: [f32; 3],
    radius: f32,
    axis: [[f32; 3]; 3],
) -> Option<marks_t6::MarkWorldStaging> {
    if scene.retained_positions.is_empty() {
        return None;
    }
    if scene.retained_lightmap_uvs.len() != scene.retained_positions.len()
        || scene.retained_normals.len() != scene.retained_positions.len()
    {
        return None;
    }
    let packed = match scene.cull.as_ref() {
        Some(cull) => cull.packed_indices.as_slice(),
        None => &[],
    };
    MARK_CLIP_SCRATCH.with(|clip_cell| {
        let mut clip = clip_cell.borrow_mut();
        if clip.ranges.is_empty() {
            return Some(marks_t6::MarkWorldStaging::default());
        }
        clip.ensure_stage_bufs();
        let MarkClipScratch {
            mids,
            halves,
            ranges,
            contexts,
            tris,
            points,
            ..
        } = &mut *clip;
        Some(marks_t6::mark_stage_world_surfaces(
            origin,
            radius,
            axis,
            mids,
            halves,
            &scene.retained_positions,
            &scene.retained_lightmap_uvs,
            &scene.retained_normals,
            packed,
            ranges,
            contexts,
            marks_t6::R_MARK_FRAGMENTS_MAX_TRIS,
            marks_t6::R_MARK_FRAGMENTS_MAX_POINTS,
            tris,
            points,
            false,
        ))
    })
}

fn pose_from_fx_host_models(host: &FxSystemHost) -> Option<WorldMarkPose<'_>> {
    if host.last_decal_against_models != Some(true) {
        return None;
    }
    Some(WorldMarkPose {
        origin: host.last_decal_origin?,
        radius: host.last_decal_size0?,
        axis: Some(impact_mark_axis(host)?),
        material: pose_material(host, MarkFragmentsAgainst::Models),
    })
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
        let broken = |p: u32| p == 2;
        let picked = mark_glass_panes(&panes, broken, [5.0; 3], [15.0; 3]);
        assert_eq!(picked, vec![0, 3, 4, 5, 6, 7, 8, 9]);
        // Touching faces overlap.
        assert_eq!(mark_glass_panes(&panes[1..2], |_| false, [10.0; 3], [20.0; 3]), vec![0]);
    }

    #[test]
    fn a_glass_mark_is_on_the_world_list_with_its_pane_as_model_index() {
        let v = |x: f32, y: f32| asset_world::T6GlassVertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            ..Default::default()
        };
        let glass = asset_world::T6GlassPaneDraw {
            vertices: vec![v(-50.0, -50.0), v(50.0, -50.0), v(0.0, 50.0)],
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
        let staging = stage_clip_glass_pane(&glass, 7, &mark, &|surf| surf == 9);
        assert!(staging.used_tri > 0 && !staging.overflow, "{staging:?}");
        let ctx = MARK_CLIP_SCRATCH.with(|c| c.borrow().tris[0].context);
        assert_eq!(ctx, marks_t6::GfxMarkContext::glass_pane(4, 5, 6, 7));
        // A surface whose material does not take the mark is skipped.
        let skipped = stage_clip_glass_pane(&glass, 7, &mark, &|_| false);
        assert_eq!(skipped.used_tri, 0);
    }
}
