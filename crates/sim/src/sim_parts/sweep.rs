//! Sweeps through the map's clip data: the world, the brush models linked on top of it, the windows still standing,
//! and the placed static models. The `MatchSimulation` methods read what the match holds; the free functions below them
//! work on whatever clip data they are handed.

use std::cell::RefCell;
use std::collections::HashMap;

use super::content_book::{SolidBrush, CollisionTreeData, CollisionTriangleData};
use crate::shot_trace::AttachedSolidShape;
use movement_t6::bridge::SweepAsk;

// ---- what the match traces against ----------------------------------------------------------------------------

impl super::state::MatchSimulation {
    /// A box swept from `start` to `end` through the world, the linked brush models, and the windows still standing.
    pub fn trace_world(&self, from: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3], mask: u32) -> trace_t6::Trace {
        Self::trace_clip(self, from, till, lo, hi, mask)
    }

    /// The same sweep with the world alone: no linked models.
    pub fn trace_static_world(
        &self,
        begin_at: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> trace_t6::Trace {
        let content = &self.content;
        let pane_blocks = |pane: u16| self.world_objects.pane_blocks_shots(u32::from(pane));
        sweep_box_trace(
            content.collision_solids(),
            content.collision_tree(),
            content.collision_triangles(),
            begin_at,
            end,
            mins,
            maxs,
            mask,
            &pane_blocks,
        )
    }

    /// A line through the world and every linked brush model except those of `skip`. The damage check leaves out
    /// its target and the blast's inflictor this way; a window stops the line while it stands.
    pub(crate) fn trace_world_skipping(
        &self,
        begin_at: [f32; 3],
        end: [f32; 3],
        mask: u32,
        skip: &[crate::ServerModelSource],
    ) -> trace_t6::Trace {
        let content = &self.content;
        let pane_blocks = |pane: u16| self.world_objects.pane_blocks_shots(u32::from(pane));
        let world = sweep_box_trace(
            content.collision_solids(),
            content.collision_tree(),
            content.collision_triangles(),
            begin_at,
            end,
            [0.0; 3],
            [0.0; 3],
            mask,
            &pane_blocks,
        );
        let linked = self
            .space
            .caps
            .iter()
            .filter(|owner| !skip.contains(&owner.owner))
            .flat_map(|owner| owner.solid_brushes())
            .map(|brush| (trace_t6::WORLD_ENTITY_SLOT, brush));
        let line = SweepAsk { start: begin_at, end, lo_corner: [0.0; 3], hi_corner: [0.0; 3], collide_filter: mask };
        let tree = content.collision_tree();
        clip_move_to_bmodels(world, tree.t6, &tree.t6_model_brushes, linked, line)
    }

    /// A line through the map that also meets the placed static models (fences, cars, crates), the way a bullet
    /// does. Script traces (`bullettrace`, `bullettracepassed`, `sighttracepassed`) see this.
    pub fn trace_static_world_models(&self, from: [f32; 3], till: [f32; 3], mask: u32) -> trace_t6::Trace {
        let mut hit = self.trace_static_world(from, till, [0.0; 3], [0.0; 3], mask);
        let statics = self.content.collision_triangles().t6_static;
        if let Some(statics) = statics.filter(|_| hit.fraction != 0.0) {
            t6_static_trace(statics, from, till, mask, &mut hit);
        }
        hit
    }

    /// Whether the loaded map has anything to collide with.
    pub fn has_static_collision(
        &self,
    ) -> bool {
        let content = &self.content;
        !content.collision_solids().is_empty() || content.collision_tree().t6.is_some_and(|map| !map.tri_indices.is_empty())
    }

    pub(crate) fn trace_clip(&self, from: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3], mask: u32) -> trace_t6::Trace {
        let content = &self.content;
        self.trace_clip_over(content.collision_solids(), content.collision_tree(), content.collision_triangles(), from, till, lo, hi, mask)
    }

    /// As [`Self::trace_clip`] over clip data the caller names (another map's, or a test's).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn trace_clip_over(
        &self,
        brushes: &[SolidBrush],
        bsp: &CollisionTreeData,
        mesh: &CollisionTriangleData,
        from: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> trace_t6::Trace {
        let probe = SweepAsk { start: from, end, lo_corner: mins, hi_corner: maxs, collide_filter: mask };
        self.sweep_linked(brushes, bsp, mesh, probe, false)
    }

    /// Whether a bot could be walked through, as if every window were already broken. Nothing of the match changes
    /// and the linked models still block.
    pub fn trace_navigation(&self, from: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3], mask: u32) -> trace_t6::Trace {
        let content = &self.content;
        let probe = SweepAsk { start: from, end: till, lo_corner: lo, hi_corner: hi, collide_filter: mask };
        self.sweep_linked(content.collision_solids(), content.collision_tree(), content.collision_triangles(), probe, true)
    }

    /// The world sweep, then the linked brush models. With `glass_gone` no window stops it.
    fn sweep_linked(
        &self,
        brushes: &[SolidBrush],
        bsp: &CollisionTreeData,
        mesh: &CollisionTriangleData,
        probe: SweepAsk,
        glass_gone: bool,
    ) -> trace_t6::Trace {
        let world = sweep_box_trace(
            brushes,
            bsp,
            mesh,
            probe.start,
            probe.end,
            probe.lo_corner,
            probe.hi_corner,
            probe.collide_filter,
            &|piece| !glass_gone && self.world_objects.pane_blocks_shots(piece as u32),
        );
        let linked = self
            .space
            .caps
            .iter()
            .flat_map(|owner| owner.solid_brushes())
            .map(|brush| (trace_t6::WORLD_ENTITY_SLOT, brush));
        clip_move_to_bmodels(world, bsp.t6, &bsp.t6_model_brushes, linked, probe)
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
        fully_embedded: raw.fully_embedded,
        starts_inside: raw.starts_inside,
        walkable: raw.walkable,
        ..trace_t6::Trace::default()
    }
}

/// Names the world as what a trace struck.
fn struck_world(mut trace: trace_t6::Trace) -> trace_t6::Trace {
    trace.impact_category = trace_t6::TraceHitType::Entity;
    trace.struck_entity_num = trace_t6::WORLD_ENTITY_SLOT;
    trace
}

/// Views of brushes in the form the clip map sweeps. `parts` holds what `t6_brush_parts` made of each, and has to
/// outlive the views.
fn brush_views<'a>(parts: &'a [(clipmap_t6::StoredBrush, Vec<trace_t6::BrushSide>)]) -> Vec<trace_t6::Brush<'a>> {
    parts
        .iter()
        .map(|(stored, sides)| trace_t6::Brush {
            lower: stored.mins,
            upper: stored.maxs,
            contents: stored.contents,
            sides,
            axial_cflags: stored.axial_cflags,
            axial_sflags: stored.axial_sflags,
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

std::thread_local! {
    static CHECK_MARKS: RefCell<CheckMarks> = const { RefCell::new(CheckMarks { table: 0, global: 0, marks: Vec::new() }) };
}

/// Runs `sweep` with the check count of `map`, starting a fresh one when the map is not the one last swept.
fn with_check_marks<R>(map: &clipmap_t6::ClipMap<'_>, sweep: impl FnOnce(&mut clipmap_t6::CheckCount<'_>) -> R) -> R {
    CHECK_MARKS.with(|cell| {
        let mut held = cell.borrow_mut();
        let grid = map.partitions.as_ptr() as usize;
        if held.table != grid || held.marks.len() != map.partitions.len() {
            held.table = grid;
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

/// What [`t6_transformed_trace`] sweeps: a brush model by its handle (0 is the world), a temporary box that is
/// swept as a capsule, or a model's collision-map brushes as the temporary brush model.
#[derive(Clone, Copy, Debug)]
pub(crate) enum T6Model {
    Brush(u32),
    TempBox { mins: [f32; 3], maxs: [f32; 3], contents: u32 },
    Collmap(&'static [trace_t6::Brush<'static>]),
}

/// The model a linked handle sweeps as, and how far it reaches from its origin. `SV_ClipHandleForEntity` gives a
/// brush model's own handle, which `CM_ClipHandleToModel` finds in the map's models; an entity whose model has a
/// collmap gets `CM_TempBrushModel`'s handle over those brushes instead. The models' collision appended after the
/// map's (`CollisionTreeData::t6_model_brushes`) are those collmaps.
pub(crate) fn linked_model(
    map: &clipmap_t6::ClipMap<'_>,
    collmaps: &HashMap<u32, &'static [trace_t6::Brush<'static>]>,
    handle: u32,
) -> Option<(T6Model, f32)> {
    if let Some(model) = map.cmodels.get(handle as usize) {
        return Some((T6Model::Brush(handle), model.radius));
    }
    let brushes = *collmaps.get(&handle)?;
    let reach = |axis: usize| brushes.iter().map(|b| b.lower[axis].abs().max(b.upper[axis].abs())).fold(0.0f32, f32::max);
    let radius = (0..3).map(|axis| reach(axis).powi(2)).sum::<f32>().sqrt();
    Some((T6Model::Collmap(brushes), radius))
}

/// Sweeps `model` as it stands at `origin`, turned by `angles`. The move is taken into the model's space and the
/// normal that comes back is turned out again. How the box is sized when the model is turned is not known; it
/// stays as it is, unturned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn t6_transformed_trace(
    map: &clipmap_t6::ClipMap<'_>,
    from: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    model: T6Model,
    mask: u32,
    spot: [f32; 3],
    aim: [f32; 3],
) -> trace_t6::Trace {
    let mut map = *map;
    let handle = match model {
        T6Model::Brush(handle) => handle,
        T6Model::TempBox { mins, maxs, contents } => map.temp_box_model(mins, maxs, contents),
        T6Model::Collmap(brushes) => map.temp_brush_model(brushes),
    };
    let turned = aim != [0.0; 3];
    let axis = clipmap_t6::angles_to_axis(aim);
    let into_model = |point: [f32; 3]| {
        let rel = [point[0] - spot[0], point[1] - spot[1], point[2] - spot[2]];
        if turned {
            axis.map(|a| a[0] * rel[0] + a[1] * rel[1] + a[2] * rel[2])
        } else {
            rel
        }
    };
    let mut raw = with_check_marks(&map, |check| {
        map.box_trace(check, into_model(from), into_model(end), mins, maxs, handle, mask)
    });
    if turned && raw.fraction < 1.0 {
        let n = raw.normal;
        raw.normal = [0, 1, 2].map(|j| axis[0][j] * n[0] + axis[1][j] * n[1] + axis[2][j] * n[2]);
    }
    sim_trace(raw)
}

/// Sweeps `brushes` alone, as a temporary brush model that needs none of a map's tables.
pub(crate) fn t6_brushes_trace(
    brushes: &[&SolidBrush],
    from: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
) -> trace_t6::Trace {
    let parts: Vec<_> = brushes.iter().map(|brush| crate::brush_clip::t6_brush_parts(brush)).collect();
    let views = brush_views(&parts);
    let mut map = clipmap_t6::ClipMap::default();
    let handle = map.temp_brush_model(&views);
    // A temporary brush model reads no mesh, so there are no partitions to mark.
    let mut check = clipmap_t6::CheckCount { global: 0, partitions: &mut [] };
    sim_trace(map.box_trace(&mut check, from, end, mins, maxs, handle, mask))
}

/// After a world sweep, the placed static models: one that is nearer than the hit takes it, reported as the world
/// with its surface flags raw.
pub(crate) fn t6_static_trace(statics: &clipmap_t6::StaticWorld, from: [f32; 3], till: [f32; 3], mask: u32, struck: &mut trace_t6::Trace) {
    let Some(found) = statics.nearest_hit(from, till, struck.fraction, mask) else {
        return;
    };
    struck.fraction = found.fraction;
    struck.normal = found.normal;
    struck.sflags = found.sflags;
    struck.cflags = found.cflags;
    struck.fully_embedded = false;
    struck.starts_inside = false;
    *struck = struck_world(*struck);
}

// ---- the world sweep ----------------------------------------------------------------------------------------------

/// The windows still standing, swept as one temporary brush model; only a hit nearer than `nearest` replaces it.
#[allow(clippy::too_many_arguments)]
fn sweep_standing_glass(
    map: &clipmap_t6::ClipMap<'_>,
    glass: &[SolidBrush],
    pane_blocks_shots: &dyn Fn(u16) -> bool,
    from: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    out: &mut trace_t6::Trace,
) {
    let standing: Vec<_> = glass
        .iter()
        .filter(|pane| pane.glass_hit_code == 0 || pane_blocks_shots(pane.glass_hit_code - 1))
        .map(crate::brush_clip::t6_brush_parts)
        .collect();
    let panes = brush_views(&standing);
    if panes.is_empty() {
        return;
    }
    let mut temp = *map;
    let handle = temp.temp_brush_model(&panes);
    let raw = with_check_marks(&temp, |check| temp.box_trace(check, from, end, mins, maxs, handle, mask));
    if raw.fraction < out.fraction {
        *out = struck_world(sim_trace(raw));
    } else if raw.starts_inside {
        out.starts_inside = true;
    }
}

/// A sweep through a map's world model, then through the windows still standing. A world hit names the world.
#[allow(clippy::too_many_arguments)]
fn sweep_map(
    map: &clipmap_t6::ClipMap<'_>,
    glass: &[SolidBrush],
    from: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> trace_t6::Trace {
    let mut out = t6_transformed_trace(map, from, end, mins, maxs, T6Model::Brush(0), mask, [0.0; 3], [0.0; 3]);
    if out.fraction < 1.0 || out.starts_inside {
        out = struck_world(out);
    }
    if !glass.is_empty() && out.fraction > 0.0 {
        sweep_standing_glass(map, glass, pane_blocks_shots, from, end, mins, maxs, mask, &mut out);
    }
    out
}

/// The world sweep through `bsp`'s clip map. With none, a map is built from `brushes` for this one sweep (a test's
/// hand-made room); with no brushes either, nothing is hit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sweep_box_trace(
    brushes: &[SolidBrush],
    bsp: &CollisionTreeData,
    _mesh: &CollisionTriangleData,
    from: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> trace_t6::Trace {
    if let Some(map) = bsp.t6 {
        return sweep_map(map, &bsp.t6_glass, from, end, mins, maxs, mask, pane_blocks_shots);
    }
    if brushes.is_empty() {
        return trace_t6::Trace::new();
    }
    let glass = CollisionTreeData::glass_of(brushes);
    crate::brush_clip::clip_store(brushes, &[], &[])
        .with_map(|map| sweep_map(map, &glass, from, end, mins, maxs, mask, pane_blocks_shots))
}

/// The standing window a line from `from` to `end` met at `fraction` (its `glass_hit_code`, 0 for none). The world
/// sweep reports a window hit as the world, so a shot asks here which pane it was.
pub(crate) fn standing_pane_struck(
    brushes: &[SolidBrush],
    bsp: &CollisionTreeData,
    from: [f32; 3],
    end: [f32; 3],
    fraction: f32,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> u16 {
    let owned;
    let glass: &[SolidBrush] = if bsp.t6.is_some() {
        &bsp.t6_glass
    } else {
        owned = CollisionTreeData::glass_of(brushes);
        &owned
    };
    glass
        .iter()
        .filter(|pane| pane.glass_hit_code != 0 && pane_blocks_shots(pane.glass_hit_code - 1))
        .map(|pane| (pane.glass_hit_code, t6_brushes_trace(&[pane], from, end, [0.0; 3], [0.0; 3], u32::MAX).fraction))
        .filter(|&(_, met)| met < 1.0 && (met - fraction).abs() < 0.01)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(0, |(code, _)| code)
}

// ---- linked brush models ------------------------------------------------------------------------------------------

/// Whether the swept box misses a model that reaches `reach` around `centre`.
fn misses_reach(input: &SweepAsk, centre: [f32; 3], reach: f32) -> bool {
    (0..3).any(|axis| {
        input.start[axis].max(input.end[axis]) + input.hi_corner[axis] < centre[axis] - reach
            || input.start[axis].min(input.end[axis]) + input.lo_corner[axis] > centre[axis] + reach
    })
}

/// Sweeps the linked brush models after the world sweep gave `hit`; the nearest of the two wins.
pub(crate) fn clip_move_to_bmodels<'a>(
    hit: trace_t6::Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    collmaps: &HashMap<u32, &'static [trace_t6::Brush<'static>]>,
    linked: impl IntoIterator<Item = (u16, &'a AttachedSolidShape)>,
    probe: SweepAsk,
) -> trace_t6::Trace {
    clip_move_to_bmodels_struck(hit, t6, collmaps, linked, probe).0
}

/// As [`clip_move_to_bmodels`], and also the place in `linked` of the brush the move struck or starts inside, for
/// the touch that causes. `t6` is the clip map those brush models belong to; with none there are no models.
/// `collmaps` holds the models' collision appended after the map's (see [`linked_model`]).
pub(crate) fn clip_move_to_bmodels_struck<'a>(
    mut hit: trace_t6::Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    collmaps: &HashMap<u32, &'static [trace_t6::Brush<'static>]>,
    linked: impl IntoIterator<Item = (u16, &'a AttachedSolidShape)>,
    probe: SweepAsk,
) -> (trace_t6::Trace, Option<usize>) {
    let mut struck = None;
    let Some(map) = t6.filter(|_| hit.fraction != 0.0) else {
        return (hit, struck);
    };
    for (place, (entnum, model_brush)) in linked.into_iter().enumerate() {
        let Some((model, radius)) = linked_model(map, collmaps, model_brush.collision_model_ref) else {
            continue;
        };
        // The swept box against the model's reach first: a big map links dozens of these.
        if misses_reach(&probe, model_brush.origin, radius + 1.0) {
            continue;
        }
        let other = t6_transformed_trace(
            map,
            probe.start,
            probe.end,
            probe.lo_corner,
            probe.hi_corner,
            model,
            probe.collide_filter,
            model_brush.origin,
            model_brush.turn,
        );
        if other.fraction >= hit.fraction && !other.starts_inside && !other.fully_embedded {
            continue;
        }
        let merged_start = hit.starts_inside | other.starts_inside;
        let merged_all = hit.fully_embedded | other.fully_embedded;
        let nearer = other.fraction < hit.fraction;
        if nearer || struck.is_none() {
            struck = Some(place);
        }
        if nearer {
            // A model nobody numbered counts as the world, as a world hit does.
            hit = trace_t6::Trace { impact_category: trace_t6::TraceHitType::Entity, struck_entity_num: entnum, ..other };
        }
        (hit.starts_inside, hit.fully_embedded) = (merged_start, merged_all);
        if hit.fraction == 0.0 && merged_start {
            break;
        }
    }
    (hit, struck)
}
