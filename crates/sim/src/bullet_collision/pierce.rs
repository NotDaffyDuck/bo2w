//! A bullet's whole flight: the legs it travels, the glass it breaks, and (for a gun that can) the surfaces it goes
//! through. Two routines, as in BO2: a bullet that cannot penetrate flies on only through glass and, for a rifle, a
//! player; one that can walks through each surface it meets and loses strength by the thickness.

use weapon_t6::penetration::{
    ADVANCE_TRACE_FWD, ADVANCE_TRACE_REV, BulletPenFacts, CONTENTS_GLASS, MAX_EXTENDED_STEPS, MAX_PENETRATE_STEPS,
    PEN_THICKNESS_FLOOR, PenetrationDepthTable, REV_END_EPS, RIFLE_COLLATERAL_SCALE, advance_trace,
};

use super::collider::{
    BulletPath, BulletTraceQuery, BulletTraceSegment, ColliderId, HITTYPE_GLASS, SKY_PENETRATION_DEPTH, TraceOutcome,
    is_sky,
};
use super::entity_caps::EntityCollisionTraceGeom;
use super::logs::pen_stop_log;
use super::poses::PlayerCollisionPose;
use super::tracer::Scene;
use crate::identities::ScriptModelId;
use crate::world::{ClientId, SimBrush, SimClipBsp, SimClipCmodels, SimClipMesh};

/// The flight of one bullet: every leg, and the last thing it struck.
pub type Flight = (Vec<BulletTraceSegment>, Option<ColliderId>);

pub fn bullet_trace_segments(
    brushes: &[SimBrush],
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> Flight {
    bullet_trace_segments_with_entity_models(
        brushes,
        &SimClipBsp::default(),
        &SimClipCmodels::default(),
        &SimClipMesh::default(),
        players,
        &[],
        query,
        pen,
        table,
        glass_is_solid,
    )
}

pub fn bullet_trace_segments_with_entity_models(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> Flight {
    bullet_trace_segments_filtered(
        brushes,
        bsp,
        cmodels,
        mesh,
        players,
        script_models,
        query,
        pen,
        table,
        glass_is_solid,
        None,
    )
}

/// The same flight, calling `on_glass_hit(pane, point)` for each pane that is still solid when the bullet reaches it.
pub(crate) fn bullet_trace_segments_filtered(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
    on_glass_hit: Option<&dyn Fn(u16, [f32; 3])>,
) -> Flight {
    if !super::tracer::is_finite_vec3(query.start) || !super::tracer::is_finite_vec3(query.end) {
        return (Vec::new(), None);
    }
    let travel = [query.end[0] - query.start[0], query.end[1] - query.start[1], query.end[2] - query.start[2]];
    let length = (travel[0] * travel[0] + travel[1] * travel[1] + travel[2] * travel[2]).sqrt();
    if length <= 0.0 {
        return (Vec::new(), None);
    }
    let shot = Shot {
        scene: Scene { brushes, bsp, cmodels, mesh, players, script_models, glass_is_solid },
        query,
        dir: travel.map(|c| c / length),
        pen,
        on_glass_hit,
    };
    if pen.pen_gate() { fly_through_surfaces(&shot, table) } else { fly_past_glass(&shot) }
}

/// The surface the bullet stopped on, as a hit.
#[derive(Clone, Copy)]
struct Impact {
    end: [f32; 3],
    normal: [f32; 3],
    collider: ColliderId,
}

struct Shot<'a> {
    scene: Scene<'a>,
    query: &'a BulletTraceQuery,
    /// Unit direction of travel.
    dir: [f32; 3],
    pen: BulletPenFacts,
    on_glass_hit: Option<&'a dyn Fn(u16, [f32; 3])>,
}

impl Shot<'_> {
    /// A trace from `start` toward `end`, never seeing the shooter, `ignore_hit` or `ignore_model`.
    fn trace_to(
        &self,
        start: [f32; 3],
        end: [f32; 3],
        ignore_hit: Option<ClientId>,
        ignore_model: Option<ScriptModelId>,
    ) -> TraceOutcome {
        let query = BulletTraceQuery {
            start,
            end,
            mask: self.query.mask,
            ignore: self.query.ignore,
            ignore_hit,
            ignore_model,
        };
        self.scene.trace(&query, true)
    }

    /// A trace on toward the end of the shot.
    fn trace_on(&self, start: [f32; 3], ignore_hit: Option<ClientId>, ignore_model: Option<ScriptModelId>) -> TraceOutcome {
        self.trace_to(start, self.query.end, ignore_hit, ignore_model)
    }

    /// A pane that has already broken: the bullet passes it as if it were not there.
    fn is_open_pane(&self, collider: ColliderId) -> bool {
        collider.pane().is_some_and(|pane| !(self.scene.glass_is_solid)(pane))
    }

    fn note_glass_hit(&self, collider: ColliderId, at: [f32; 3]) {
        if let (Some(pane), Some(report)) = (collider.pane(), self.on_glass_hit)
            && (self.scene.glass_is_solid)(pane)
        {
            report(pane, at);
        }
    }

    /// Where a bullet that begins at `from` (inside a solid) first finds clear space, searched up to `limit` ahead.
    /// `None` when it never does, or the trace is refused.
    fn find_clear_space(
        &self,
        from: [f32; 3],
        ignore_hit: Option<ClientId>,
        ignore_model: Option<ScriptModelId>,
        limit: f32,
    ) -> Option<[f32; 3]> {
        const STEP: f32 = 0.25;
        let mut traveled = STEP;
        while traveled <= limit {
            let probe = point_ahead(from, traveled, self.dir);
            match self.trace_on(probe, ignore_hit, ignore_model) {
                TraceOutcome::StartSolid { .. } => {}
                TraceOutcome::Invalid { .. } => return None,
                _ => return Some(probe),
            }
            traveled += STEP;
        }
        None
    }

    fn step_past(&self, hit: &Impact, through_world: bool, nudge: f32) -> Option<[f32; 3]> {
        advance_trace(hit.end, hit.normal, self.dir, through_world, nudge)
    }
}

fn point_ahead(from: [f32; 3], distance: f32, dir: [f32; 3]) -> [f32; 3] {
    [from[0] + distance * dir[0], from[1] + distance * dir[1], from[2] + distance * dir[2]]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn negated(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

/// A leg of the flight with nothing special about it: no thickness, nothing passed through. `startsolid` is set
/// when the bullet began inside what it struck.
fn leg(
    path: BulletPath,
    damage_mult: f32,
    start: [f32; 3],
    end: [f32; 3],
    normal: [f32; 3],
    collider: Option<ColliderId>,
    startsolid: bool,
) -> BulletTraceSegment {
    let (hit_type, hit_id) = collider.map_or((0, 0), |c| c.hit_kind(startsolid));
    BulletTraceSegment {
        start,
        end,
        normal,
        surf_type: collider.map_or(0, ColliderId::surf_type),
        surface_flags: collider.map_or(0, ColliderId::surface_flags),
        penetrated: false,
        thickness: 0.0,
        damage_mult,
        path,
        collider,
        startsolid,
        glass_encoded: collider.map_or(0, ColliderId::glass_encoded),
        hit_type,
        hit_id,
    }
}

fn leg_to(path: BulletPath, damage_mult: f32, start: [f32; 3], hit: &Impact, startsolid: bool) -> BulletTraceSegment {
    leg(path, damage_mult, start, hit.end, hit.normal, Some(hit.collider), startsolid)
}

/// The flight of a bullet that cannot penetrate: it goes on through broken glass, through glass it breaks, and (a
/// rifle round) through a player at a fraction of its strength; anything else stops it.
fn fly_past_glass(shot: &Shot<'_>) -> Flight {
    let query = shot.query;
    let path = BulletPath::Extended;
    let mut legs = Vec::new();
    let mut start = query.start;
    let mut leg_start = query.start;
    let mut ignore_hit = query.ignore_hit;
    let mut strength = 1.0_f32;
    let mut last_struck = None;

    for _ in 0..MAX_EXTENDED_STEPS {
        match shot.trace_on(start, ignore_hit, query.ignore_model) {
            TraceOutcome::Invalid { .. } => break,
            TraceOutcome::Miss { end } => {
                legs.push(leg(path, strength, leg_start, end, [0.0; 3], None, false));
                break;
            }
            TraceOutcome::StartSolid { end, collider } => {
                legs.push(leg(path, strength, leg_start, end, [0.0; 3], collider, true));
                if collider.is_some() {
                    last_struck = collider;
                }
                break;
            }
            TraceOutcome::Hit { end, normal, collider, .. } => {
                let hit = Impact { end, normal, collider };
                let is_glass = collider.contents() & CONTENTS_GLASS != 0;
                if is_glass {
                    if shot.is_open_pane(collider) {
                        let Some(next) = shot.step_past(&hit, true, ADVANCE_TRACE_FWD) else {
                            break;
                        };
                        start = next;
                        continue;
                    }
                    shot.note_glass_hit(collider, end);
                }
                legs.push(BulletTraceSegment {
                    penetrated: is_glass || collider.is_player(),
                    ..leg_to(path, strength, leg_start, &hit, false)
                });
                last_struck = Some(collider);
                if is_glass {
                    let Some(next) = shot.step_past(&hit, true, ADVANCE_TRACE_FWD) else {
                        break;
                    };
                    start = next;
                    leg_start = end;
                    continue;
                }
                if !collider.is_damageable() {
                    break;
                }
                ignore_hit = collider.client().or(ignore_hit);
                if !shot.pen.rifle_bullet || !collider.is_player() || collider.is_shield() {
                    break;
                }
                strength *= RIFLE_COLLATERAL_SCALE;
                let Some(next) = shot.step_past(&hit, false, 0.0) else {
                    break;
                };
                start = next;
                leg_start = end;
            }
        }
    }
    (legs, last_struck)
}

/// The first surface a penetrating bullet meets, skipping broken glass. `Ok` carries it and whether the bullet began
/// inside it; `Err` is the finished flight when the bullet never meets anything to enter.
fn first_surface(shot: &Shot<'_>, legs: &mut Vec<BulletTraceSegment>) -> Result<(Impact, bool), ()> {
    let query = shot.query;
    let path = BulletPath::Penetrate;
    let mut start = query.start;
    let mut skipped_panes = 0usize;
    loop {
        match shot.trace_on(start, query.ignore_hit, query.ignore_model) {
            TraceOutcome::Invalid { .. } => return Err(()),
            TraceOutcome::Miss { end } => {
                legs.push(leg(path, 1.0, query.start, end, [0.0; 3], None, false));
                return Err(());
            }
            TraceOutcome::StartSolid { collider: None, .. } => {
                legs.push(leg(path, 1.0, query.start, query.start, [0.0; 3], None, true));
                return Err(());
            }
            TraceOutcome::StartSolid { collider: Some(collider), .. } => {
                let inside = Impact { end: query.start, normal: negated(shot.dir), collider };
                return Ok((inside, true));
            }
            TraceOutcome::Hit { end, normal, collider, .. } => {
                let hit = Impact { end, normal, collider };
                if !shot.is_open_pane(collider) {
                    return Ok((hit, false));
                }
                let Some(next) = shot.step_past(&hit, true, ADVANCE_TRACE_FWD) else {
                    return Err(());
                };
                skipped_panes += 1;
                if skipped_panes >= MAX_EXTENDED_STEPS {
                    return Err(());
                }
                start = next;
            }
        }
    }
}

/// The flight of a bullet that can penetrate. It enters a surface, finds where it comes out the far side by tracing
/// back, and pays the thickness out of its strength; it stops when none is left, when the surface has no depth in the
/// table, or at a riot shield.
fn fly_through_surfaces(shot: &Shot<'_>, table: &PenetrationDepthTable) -> Flight {
    let query = shot.query;
    let dir = shot.dir;
    let path = BulletPath::Penetrate;
    let mut legs = Vec::new();
    let mut strength = 1.0_f32;

    let Ok((first, began_inside)) = first_surface(shot, &mut legs) else {
        return (legs, None);
    };
    let mut last_surf = first.collider.depth_surf(0);
    legs.push(BulletTraceSegment { penetrated: true, ..leg_to(path, strength, query.start, &first, began_inside) });
    shot.note_glass_hit(first.collider, first.end);
    let mut last_struck = Some(first.collider);
    let mut entered = first;
    let mut ignore_hit = query.ignore_hit;

    for _ in 0..MAX_PENETRATE_STEPS {
        if entered.collider.is_shield() {
            pen_stop_log("riot shield", entered.end);
            break;
        }
        let entry_depth =
            if is_sky(entered.collider) { SKY_PENETRATION_DEPTH } else { table.depth(shot.pen.penetrate_type, last_surf) };
        let mut max_depth = entry_depth * shot.pen.penetrate_multiplier;
        if max_depth <= 0.0 {
            pen_stop_log("no depth for this surface", entered.end);
            break;
        }
        let Some(inside_start) = shot.step_past(&entered, entered.collider.is_world(), ADVANCE_TRACE_FWD) else {
            break;
        };
        ignore_hit = entered.collider.client().or(ignore_hit);
        let ignore_model = entered.collider.script_model().or(query.ignore_model);
        let leg_start = entered.end;
        let entry_pos = entered.end;

        let far = match shot.trace_on(inside_start, ignore_hit, ignore_model) {
            TraceOutcome::Hit { end, normal, collider, .. } => Impact { end, normal, collider },
            TraceOutcome::Miss { end } => {
                legs.push(leg(path, strength, leg_start, end, [0.0; 3], None, false));
                break;
            }
            TraceOutcome::StartSolid { collider, .. } => {
                // Still inside something at the far side of the nudge: walk on until clear.
                let reach = (max_depth + PEN_THICKNESS_FLOOR).max(2.0);
                let Some(exit) = shot.find_clear_space(inside_start, ignore_hit, ignore_model, reach) else {
                    pen_stop_log("started inside a solid it could not leave", inside_start);
                    break;
                };
                let thickness = distance(entry_pos, exit).max(PEN_THICKNESS_FLOOR);
                strength -= thickness / max_depth;
                if strength <= 0.0 {
                    break;
                }
                entered.end = exit;
                entered.normal = negated(dir);
                if let Some(thing) = collider.filter(|c| c.is_damageable()) {
                    last_struck = Some(thing);
                }
                continue;
            }
            TraceOutcome::Invalid { .. } => {
                pen_stop_log("invalid trace", inside_start);
                break;
            }
        };
        if shot.is_open_pane(far.collider) {
            let Some(next) = shot.step_past(&far, true, ADVANCE_TRACE_FWD) else {
                break;
            };
            entered.end = next;
            entered.normal = far.normal;
            continue;
        }
        shot.note_glass_hit(far.collider, far.end);

        // Trace back from the far surface to find where the bullet came out of what it entered.
        let back_dir = negated(dir);
        let back_end = point_ahead(entry_pos, REV_END_EPS, back_dir);
        let back_start = advance_trace(
            far.end,
            negated(far.normal),
            back_dir,
            far.collider.is_world(),
            ADVANCE_TRACE_REV,
        )
        .unwrap_or(far.end);
        let back = shot.trace_to(
            back_start,
            back_end,
            far.collider.client(),
            far.collider.script_model().or(query.ignore_model),
        );
        let exit = match back {
            TraceOutcome::Hit { end, collider, .. } => Some((end, collider.depth_surf(0), is_sky(collider))),
            TraceOutcome::StartSolid { end, collider, .. } => {
                Some((end, collider.map_or(0, |c| c.depth_surf(0)), collider.is_some_and(is_sky)))
            }
            _ => None,
        };

        let mut thickness = 0.0_f32;
        if let Some((exit_pos, exit_surf, exit_is_sky)) = exit {
            thickness = distance(entry_pos, exit_pos).max(PEN_THICKNESS_FLOOR);
            let exit_depth = table.depth(shot.pen.penetrate_type, exit_surf) * shot.pen.penetrate_multiplier;
            max_depth = max_depth.min(exit_depth);
            // Leaving through a sky face is always the same depth.
            if exit_is_sky {
                max_depth = SKY_PENETRATION_DEPTH;
            } else if max_depth <= 0.0 {
                break;
            }
            strength -= thickness / max_depth;
            if strength <= 0.0 {
                break;
            }
            legs.push(BulletTraceSegment {
                penetrated: true,
                thickness,
                surface_flags: leg_surface_with_exit_flag(entered.collider),
                ..leg(path, strength, entry_pos, exit_pos, dir, Some(entered.collider), false)
            });
        }

        last_surf = far.collider.depth_surf(last_surf);
        legs.push(BulletTraceSegment { penetrated: true, thickness, ..leg_to(path, strength, leg_start, &far, false) });
        last_struck = Some(far.collider);
        entered = far;
    }
    (legs, last_struck)
}

/// The surface flags of the exit mark left on `collider`.
fn leg_surface_with_exit_flag(collider: ColliderId) -> u32 {
    collider.surface_flags() | fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG
}

/// The pane a glass hit struck, from the hit type and id a segment carries.
pub fn glass_piece_from_hit(hit_type: i32, hit_id: u16) -> Option<u32> {
    (hit_type == HITTYPE_GLASS && hit_id != 0).then(|| u32::from(hit_id) - 1)
}
