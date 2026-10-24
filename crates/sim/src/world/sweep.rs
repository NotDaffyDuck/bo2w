//! Sweeps through the map's clip data: the world, the brush models linked on top of it, the windows still standing,
//! and the placed static models. The `SimState` methods read what the match holds; the free functions below them
//! work on whatever clip data they are handed.

use std::cell::RefCell;

use super::catalog::{SimBrush, SimClipBsp, SimClipMesh};
use super::state::SimState;
use crate::bullet_collision::LinkedBrushCollisionBrush;
use movement_t6::bridge::GroundTraceInput;

// ---- what the match traces against ----------------------------------------------------------------------------

impl SimState {
    /// A box swept from `start` to `end` through the world, the linked brush models, and the windows still standing.
    pub fn trace_world(&self, start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3], mask: u32) -> trace_t6::Trace {
        self.trace_clip(start, end, mins, maxs, mask)
    }

    /// The same sweep with the world alone: no linked models.
    pub fn trace_static_world(
        &self,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> trace_t6::Trace {
        let content = &self.content;
        clip_trace(
            content.clip_brushes(),
            content.clip_bsp(),
            content.clip_mesh(),
            start,
            end,
            mins,
            maxs,
            mask,
            &|piece| self.world_objects.glass_is_solid(piece as u32),
        )
    }

    /// A line through the world and every linked brush model except those of `skip`. The damage check leaves out
    /// its target and the blast's inflictor this way; a window stops the line while it stands.
    pub(crate) fn trace_world_skipping(
        &self,
        start: [f32; 3],
        end: [f32; 3],
        mask: u32,
        skip: &[crate::AuthorityModelOwner],
    ) -> trace_t6::Trace {
        let content = &self.content;
        let world = clip_trace(
            content.clip_brushes(),
            content.clip_bsp(),
            content.clip_mesh(),
            start,
            end,
            [0.0; 3],
            [0.0; 3],
            mask,
            &|piece| self.world_objects.glass_is_solid(piece as u32),
        );
        let linked = self
            .space
            .caps
            .iter()
            .filter(|owner| !skip.contains(&owner.owner))
            .flat_map(|owner| owner.solid_brushes())
            .map(|brush| (trace_t6::ENTITYNUM_WORLD, brush));
        let line = GroundTraceInput { start, end, mins: [0.0; 3], maxs: [0.0; 3], tracemask: mask };
        clip_move_to_bmodels(world, content.clip_bsp().t6, linked, line)
    }

    /// A line through the map that also meets the placed static models (fences, cars, crates), the way a bullet
    /// does. Script traces (`bullettrace`, `bullettracepassed`, `sighttracepassed`) see this.
    pub fn trace_static_world_models(&self, start: [f32; 3], end: [f32; 3], mask: u32) -> trace_t6::Trace {
        let mut hit = self.trace_static_world(start, end, [0.0; 3], [0.0; 3], mask);
        let statics = self.content.clip_mesh().t6_static;
        if let Some(statics) = statics.filter(|_| hit.fraction != 0.0) {
            t6_static_trace(statics, start, end, mask, &mut hit);
        }
        hit
    }

    /// Whether the loaded map has anything to collide with.
    pub fn has_world_clip(&self) -> bool {
        let content = &self.content;
        !content.clip_brushes().is_empty() || content.clip_bsp().t6.is_some_and(|map| !map.tri_indices.is_empty())
    }

    pub(crate) fn trace_clip(&self, start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3], mask: u32) -> trace_t6::Trace {
        let content = &self.content;
        self.trace_clip_maps(content.clip_brushes(), content.clip_bsp(), content.clip_mesh(), start, end, mins, maxs, mask)
    }

    /// As [`Self::trace_clip`] over clip data the caller names (another map's, or a test's).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn trace_clip_maps(
        &self,
        brushes: &[SimBrush],
        bsp: &SimClipBsp,
        mesh: &SimClipMesh,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> trace_t6::Trace {
        let input = GroundTraceInput { start, end, mins, maxs, tracemask: mask };
        self.sweep_linked(brushes, bsp, mesh, input, false)
    }

    /// Whether a bot could be walked through, as if every window were already broken. Nothing of the match changes
    /// and the linked models still block.
    pub fn trace_navigation(&self, start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3], mask: u32) -> trace_t6::Trace {
        let content = &self.content;
        let input = GroundTraceInput { start, end, mins, maxs, tracemask: mask };
        self.sweep_linked(content.clip_brushes(), content.clip_bsp(), content.clip_mesh(), input, true)
    }

    /// The world sweep, then the linked brush models. With `glass_gone` no window stops it.
    fn sweep_linked(
        &self,
        brushes: &[SimBrush],
        bsp: &SimClipBsp,
        mesh: &SimClipMesh,
        input: GroundTraceInput,
        glass_gone: bool,
    ) -> trace_t6::Trace {
        let world = clip_trace(
            brushes,
            bsp,
            mesh,
            input.start,
            input.end,
            input.mins,
            input.maxs,
            input.tracemask,
            &|piece| !glass_gone && self.world_objects.glass_is_solid(piece as u32),
        );
        let linked = self
            .space
            .caps
            .iter()
            .flat_map(|owner| owner.solid_brushes())
            .map(|brush| (trace_t6::ENTITYNUM_WORLD, brush));
        clip_move_to_bmodels(world, bsp.t6, linked, input)
    }
}

// ---- turning clip-map results into the sim's own ----------------------------------------------------------------

/// The sim's trace from a clip-map one: surface flags in the layout the sim reads, and no hit named.
fn sim_trace(raw: trace_t6::Trace) -> trace_t6::Trace {
    trace_t6::Trace {
        fraction: raw.fraction,
        normal: raw.normal,
        sflags: asset_world::t5_surface_flags(raw.sflags),
        cflags: raw.cflags,
        allsolid: raw.allsolid,
        startsolid: raw.startsolid,
        walkable: raw.walkable,
        ..trace_t6::Trace::default()
    }
}

/// Names the world as what a trace struck.
fn struck_world(mut trace: trace_t6::Trace) -> trace_t6::Trace {
    trace.hit_type = trace_t6::TraceHitType::Entity;
    trace.hit_id = trace_t6::ENTITYNUM_WORLD;
    trace
}

/// Views of brushes in the form the clip map sweeps. `parts` holds what `t6_brush_parts` made of each, and has to
/// outlive the views.
fn brush_views<'a>(parts: &'a [(clipmap_t6::StoredBrush, Vec<trace_t6::BrushSide>)]) -> Vec<trace_t6::Brush<'a>> {
    parts
        .iter()
        .map(|(brush, sides)| trace_t6::Brush {
            mins: brush.mins,
            maxs: brush.maxs,
            contents: brush.contents,
            sides,
            axial_cflags: brush.axial_cflags,
            axial_sflags: brush.axial_sflags,
        })
        .collect()
}

// ---- the check count the clip map keeps between sweeps ----------------------------------------------------------

/// The clip map counts the sweeps it has made and marks each partition it has looked at with that count, so one
/// partition is not looked at twice in a sweep. The count and the marks carry from one sweep to the next; they are
/// kept per thread, for the map whose partition table sits at `table`.
struct CheckMarks {
    table: usize,
    global: u16,
    marks: Vec<u16>,
}

thread_local! {
    static CHECK_MARKS: RefCell<CheckMarks> = const { RefCell::new(CheckMarks { table: 0, global: 0, marks: Vec::new() }) };
}

/// Runs `sweep` with the check count of `map`, starting a fresh one when the map is not the one last swept.
fn with_check_marks<R>(map: &clipmap_t6::ClipMap<'_>, sweep: impl FnOnce(&mut clipmap_t6::CheckCount<'_>) -> R) -> R {
    CHECK_MARKS.with(|cell| {
        let mut held = cell.borrow_mut();
        let table = map.partitions.as_ptr() as usize;
        if held.table != table || held.marks.len() != map.partitions.len() {
            held.table = table;
            held.global = 0;
            held.marks.clear();
            held.marks.resize(map.partitions.len(), 0);
        }
        let global = held.global;
        let mut check = clipmap_t6::CheckCount { global, partitions: &mut held.marks[..] };
        let out = sweep(&mut check);
        held.global = check.global;
        out
    })
}

// ---- sweeps of one model ----------------------------------------------------------------------------------------

/// What [`t6_transformed_trace`] sweeps: a brush model by its handle (0 is the world), or a temporary box that is
/// swept as a capsule.
#[derive(Clone, Copy, Debug)]
pub(crate) enum T6Model {
    Brush(u32),
    TempBox { mins: [f32; 3], maxs: [f32; 3], contents: u32 },
}

/// Sweeps `model` as it stands at `origin`, turned by `angles`. The move is taken into the model's space and the
/// normal that comes back is turned out again. How the box is sized when the model is turned is not known; it
/// stays as it is, unturned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn t6_transformed_trace(
    map: &clipmap_t6::ClipMap<'_>,
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    model: T6Model,
    mask: u32,
    origin: [f32; 3],
    angles: [f32; 3],
) -> trace_t6::Trace {
    let mut map = *map;
    let handle = match model {
        T6Model::Brush(handle) => handle,
        T6Model::TempBox { mins, maxs, contents } => map.temp_box_model(mins, maxs, contents),
    };
    let turned = angles != [0.0; 3];
    let axis = clipmap_t6::angles_to_axis(angles);
    let into_model = |point: [f32; 3]| {
        let rel = [point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]];
        if turned {
            axis.map(|a| a[0] * rel[0] + a[1] * rel[1] + a[2] * rel[2])
        } else {
            rel
        }
    };
    let mut raw = with_check_marks(&map, |check| {
        map.box_trace(check, into_model(start), into_model(end), mins, maxs, handle, mask)
    });
    if turned && raw.fraction < 1.0 {
        let n = raw.normal;
        raw.normal = [0, 1, 2].map(|j| axis[0][j] * n[0] + axis[1][j] * n[1] + axis[2][j] * n[2]);
    }
    sim_trace(raw)
}

/// Sweeps `brushes` alone, as a temporary brush model that needs none of a map's tables.
pub(crate) fn t6_brushes_trace(
    brushes: &[&SimBrush],
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
) -> trace_t6::Trace {
    let parts: Vec<_> = brushes.iter().map(|brush| crate::clip_fallback::t6_brush_parts(brush)).collect();
    let views = brush_views(&parts);
    let mut map = clipmap_t6::ClipMap::default();
    let handle = map.temp_brush_model(&views);
    // A temporary brush model reads no mesh, so there are no partitions to mark.
    let mut check = clipmap_t6::CheckCount { global: 0, partitions: &mut [] };
    sim_trace(map.box_trace(&mut check, start, end, mins, maxs, handle, mask))
}

/// After a world sweep, the placed static models: one that is nearer than the hit takes it, reported as the world
/// with its surface flags raw.
pub(crate) fn t6_static_trace(statics: &clipmap_t6::StaticWorld, start: [f32; 3], end: [f32; 3], mask: u32, hit: &mut trace_t6::Trace) {
    let Some(found) = statics.nearest_hit(start, end, hit.fraction, mask) else {
        return;
    };
    hit.fraction = found.fraction;
    hit.normal = found.normal;
    hit.sflags = found.sflags;
    hit.cflags = found.cflags;
    hit.allsolid = false;
    hit.startsolid = false;
    *hit = struck_world(*hit);
}

// ---- the world sweep ----------------------------------------------------------------------------------------------

/// The windows still standing, swept as one temporary brush model; only a hit nearer than `nearest` replaces it.
#[allow(clippy::too_many_arguments)]
fn sweep_standing_glass(
    map: &clipmap_t6::ClipMap<'_>,
    glass: &[SimBrush],
    glass_is_solid: &dyn Fn(u16) -> bool,
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    out: &mut trace_t6::Trace,
) {
    let standing: Vec<_> = glass
        .iter()
        .filter(|pane| pane.glass_encoded == 0 || glass_is_solid(pane.glass_encoded - 1))
        .map(crate::clip_fallback::t6_brush_parts)
        .collect();
    let panes = brush_views(&standing);
    if panes.is_empty() {
        return;
    }
    let mut temp = *map;
    let handle = temp.temp_brush_model(&panes);
    let raw = with_check_marks(&temp, |check| temp.box_trace(check, start, end, mins, maxs, handle, mask));
    if raw.fraction < out.fraction {
        *out = struck_world(sim_trace(raw));
    } else if raw.startsolid {
        out.startsolid = true;
    }
}

/// A sweep through a map's world model, then through the windows still standing. A world hit names the world.
#[allow(clippy::too_many_arguments)]
fn sweep_map(
    map: &clipmap_t6::ClipMap<'_>,
    glass: &[SimBrush],
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> trace_t6::Trace {
    let mut out = t6_transformed_trace(map, start, end, mins, maxs, T6Model::Brush(0), mask, [0.0; 3], [0.0; 3]);
    if out.fraction < 1.0 || out.startsolid {
        out = struck_world(out);
    }
    if !glass.is_empty() && out.fraction > 0.0 {
        sweep_standing_glass(map, glass, glass_is_solid, start, end, mins, maxs, mask, &mut out);
    }
    out
}

/// The world sweep through `bsp`'s clip map. With none, a map is built from `brushes` for this one sweep (a test's
/// hand-made room); with no brushes either, nothing is hit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn clip_trace(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    _mesh: &SimClipMesh,
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> trace_t6::Trace {
    if let Some(map) = bsp.t6 {
        return sweep_map(map, &bsp.t6_glass, start, end, mins, maxs, mask, glass_is_solid);
    }
    if brushes.is_empty() {
        return trace_t6::Trace::new();
    }
    let glass = SimClipBsp::glass_of(brushes);
    crate::clip_fallback::clip_store(brushes, &[], &[])
        .with_map(|map| sweep_map(map, &glass, start, end, mins, maxs, mask, glass_is_solid))
}

// ---- linked brush models ------------------------------------------------------------------------------------------

/// Whether the swept box misses a model that reaches `reach` around `centre`.
fn misses_reach(input: &GroundTraceInput, centre: [f32; 3], reach: f32) -> bool {
    (0..3).any(|axis| {
        input.start[axis].max(input.end[axis]) + input.maxs[axis] < centre[axis] - reach
            || input.start[axis].min(input.end[axis]) + input.mins[axis] > centre[axis] + reach
    })
}

/// Sweeps the linked brush models after the world sweep gave `hit`; the nearest of the two wins.
pub(crate) fn clip_move_to_bmodels<'a>(
    hit: trace_t6::Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    linked: impl IntoIterator<Item = (u16, &'a LinkedBrushCollisionBrush)>,
    input: GroundTraceInput,
) -> trace_t6::Trace {
    clip_move_to_bmodels_struck(hit, t6, linked, input).0
}

/// As [`clip_move_to_bmodels`], and also the place in `linked` of the brush the move struck or starts inside, for
/// the touch that causes. `t6` is the clip map those brush models belong to; with none there are no models.
pub(crate) fn clip_move_to_bmodels_struck<'a>(
    mut hit: trace_t6::Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    linked: impl IntoIterator<Item = (u16, &'a LinkedBrushCollisionBrush)>,
    input: GroundTraceInput,
) -> (trace_t6::Trace, Option<usize>) {
    let mut struck = None;
    let Some(map) = t6 else {
        return (hit, struck);
    };
    if hit.fraction == 0.0 {
        return (hit, struck);
    }
    for (place, (entnum, brush)) in linked.into_iter().enumerate() {
        let Some(radius) = map.cmodels.get(brush.cmodel_handle as usize).map(|model| model.radius) else {
            continue;
        };
        // The swept box against the model's reach first: a big map links dozens of these.
        if misses_reach(&input, brush.origin, radius + 1.0) {
            continue;
        }
        let other = t6_transformed_trace(
            map,
            input.start,
            input.end,
            input.mins,
            input.maxs,
            T6Model::Brush(brush.cmodel_handle),
            input.tracemask,
            brush.origin,
            brush.angles,
        );
        if other.fraction >= hit.fraction && !other.startsolid && !other.allsolid {
            continue;
        }
        let startsolid = hit.startsolid | other.startsolid;
        let allsolid = hit.allsolid | other.allsolid;
        if other.fraction < hit.fraction || struck.is_none() {
            struck = Some(place);
        }
        if other.fraction < hit.fraction {
            hit = other;
            // A model nobody numbered counts as the world, as a world hit does.
            hit.hit_type = trace_t6::TraceHitType::Entity;
            hit.hit_id = entnum;
        }
        hit.startsolid = startsolid;
        hit.allsolid = allsolid;
        if hit.fraction == 0.0 && startsolid {
            break;
        }
    }
    (hit, struck)
}
