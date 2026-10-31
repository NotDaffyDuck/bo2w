//! A bullet's whole flight: the legs it travels, the glass it breaks, and (for a gun that can) the surfaces it goes
//! through. Two routines, as in BO2: a bullet that cannot penetrate flies on only through glass and, for a rifle, a
//! player; one that can walks through each surface it meets and loses strength by the thickness.

use weapon_t6::penetration::{
    STEP_FORECAST_NUDGE, STEP_BACKCAST_NUDGE, PierceCapability, BRUSH_IS_PANE, PIERCE_LONG_PASS_LIMIT, PIERCE_PASS_LIMIT,
    PIERCE_DEPTH_MINIMUM, BulletPierceDepthChart, RIFLE_PASS_THROUGH_FACTOR, advance_trace,
};

use super::collider::{
    ShotRouteKind, ShotRayRequest, ShotPathLeg, HitTargetId, GLASS_HIT_CODE, SKY_PENETRATION_DEPTH, RayResult,
    is_sky,
};
use super::entity_caps::EntityRayShape;
use super::logs::pen_stop_log;
use super::poses::PlayerHitboxPose;
use super::ray_walk::Scene;
use crate::match_ids::ScriptedModel;
use crate::sim_parts::{SeatNo, SolidBrush, CollisionTreeData, CollisionSubmodels, CollisionTriangleData};

/// bo2zm: what is left of a bullet that has gone through all it can (rounding leaves ~1e-7).
const SPENT_BULLET: f32 = 1e-3;

pub fn trace_shot_legs(
    brushes: &[SolidBrush],
    players: &[PlayerHitboxPose],
    query: &ShotRayRequest,
    pen: PierceCapability,
    table: &BulletPierceDepthChart,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> (Vec<ShotPathLeg>, Option<HitTargetId>) {
    shot_legs_against_models(
        brushes,
        &CollisionTreeData::default(),
        &CollisionSubmodels::default(),
        &CollisionTriangleData::default(),
        players,
        &[],
        query,
        pen,
        table,
        pane_blocks_shots,
    )
}

pub fn shot_legs_against_models(
    brushes: &[SolidBrush],
    bsp: &CollisionTreeData,
    cmodels: &CollisionSubmodels,
    mesh: &CollisionTriangleData,
    players: &[PlayerHitboxPose],
    placed_models: &[EntityRayShape],
    query: &ShotRayRequest,
    pen: PierceCapability,
    table: &BulletPierceDepthChart,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> (Vec<ShotPathLeg>, Option<HitTargetId>) {
    shot_legs_with_filter(
        brushes,
        bsp,
        cmodels,
        mesh,
        players,
        placed_models,
        query,
        pen,
        table,
        pane_blocks_shots,
        None,
    )
}

/// The same flight, calling `on_glass_hit(pane, point)` for each pane that is still solid when the bullet reaches it.
pub(crate) fn shot_legs_with_filter(
    brushes: &[SolidBrush],
    bsp: &CollisionTreeData,
    cmodels: &CollisionSubmodels,
    mesh: &CollisionTriangleData,
    players: &[PlayerHitboxPose],
    placed_models: &[EntityRayShape],
    probe: &ShotRayRequest,
    pen: PierceCapability,
    table: &BulletPierceDepthChart,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
    on_glass_hit: Option<&dyn Fn(u16, [f32; 3])>,
) -> (Vec<ShotPathLeg>, Option<HitTargetId>) {
    let finite = super::ray_walk::is_finite_vec3(probe.from) && super::ray_walk::is_finite_vec3(probe.to);
    let travel = [probe.to[0] - probe.from[0], probe.to[1] - probe.from[1], probe.to[2] - probe.from[2]];
    let length = distance(probe.from, probe.to);
    // A non-finite or zero-length shot flies nowhere and strikes nothing.
    if !finite || length <= 0.0 {
        return (Vec::new(), None);
    }
    let shot = Shot {
        scene: Scene { brushes, bsp, cmodels, mesh, players, placed_models, pane_blocks_shots },
        query: probe,
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
    face_dir: [f32; 3],
    collider: HitTargetId,
}

struct Shot<'a> {
    scene: Scene<'a>,
    query: &'a ShotRayRequest,
    /// Unit direction of travel.
    dir: [f32; 3],
    pen: PierceCapability,
    on_glass_hit: Option<&'a dyn Fn(u16, [f32; 3])>,
}

impl Shot<'_> {
    /// A trace from `start` toward `end`, never seeing the shooter, `skip_client_hit` or `ignore_model`.
    fn trace_to(
        &self,
        from: [f32; 3],
        end: [f32; 3],
        skip_client_hit: Option<SeatNo>,
        ignore_model: Option<ScriptedModel>,
    ) -> RayResult {
        let query = ShotRayRequest {
            from,
            to: end,
            bits: self.query.bits,
            ignore: self.query.ignore,
            skip_client_hit,
            ignore_model,
        };
        self.scene.trace(&query, true)
    }

    /// A trace on toward the end of the shot.
    fn trace_on(&self, from: [f32; 3], skip_client_hit: Option<SeatNo>, ignore_model: Option<ScriptedModel>) -> RayResult {
        self.trace_to(from, self.query.to, skip_client_hit, ignore_model)
    }

    /// A pane that has already broken: the bullet passes it as if it were not there.
    fn is_open_pane(&self, collider: HitTargetId) -> bool {
        collider.pane().is_some_and(|pane| !(self.scene.pane_blocks_shots)(pane))
    }

    fn note_glass_hit(&self, collider: HitTargetId, at: [f32; 3]) {
        if let (Some(pane), Some(report)) = (collider.pane(), self.on_glass_hit)
            && (self.scene.pane_blocks_shots)(pane)
        {
            report(pane, at);
        }
    }

    fn step_past(&self, hit: &Impact, through_world: bool, nudge: f32) -> Option<[f32; 3]> {
        advance_trace(hit.end, hit.face_dir, self.dir, through_world, nudge)
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
    path: ShotRouteKind,
    harm_multiplier: f32,
    from: [f32; 3],
    end: [f32; 3],
    face: [f32; 3],
    collider: Option<HitTargetId>,
    began_inside: bool,
) -> ShotPathLeg {
    let (impact_category, struck_entity_num) = collider.map_or((0, 0), |c| c.hit_kind(began_inside));
    ShotPathLeg {
        from,
        to: end,
        face_dir: face,
        face_kind: collider.map_or(0, HitTargetId::surf_type),
        face_bits: collider.map_or(0, HitTargetId::surface_flags),
        passed_through_wall: false,
        depth: 0.0,
        harm_multiplier,
        path,
        collider,
        began_inside,
        glass_hit_code: collider.map_or(0, HitTargetId::glass_hit_code),
        impact_category,
        struck_entity_num,
    }
}

fn leg_to(path: ShotRouteKind, harm_multiplier: f32, from: [f32; 3], struck: &Impact, startsolid: bool) -> ShotPathLeg {
    leg(path, harm_multiplier, from, struck.end, struck.face_dir, Some(struck.collider), startsolid)
}

/// The flight of a bullet that cannot penetrate: it goes on through broken glass, through glass it breaks, and (a
/// rifle round) through a player at a fraction of its strength; anything else stops it.
fn fly_past_glass(shot: &Shot<'_>) -> (Vec<ShotPathLeg>, Option<HitTargetId>) {
    let probe = shot.query;
    let path = ShotRouteKind::Extended;
    let mut legs = Vec::new();
    let mut from = probe.from;
    let mut leg_start = probe.from;
    let mut skip_client_hit = probe.skip_client_hit;
    let mut strength = 1.0_f32;
    let mut last_struck = None;

    for _ in 0..PIERCE_LONG_PASS_LIMIT {
        match shot.trace_on(from, skip_client_hit, probe.ignore_model) {
            RayResult::Invalid { .. } => break,
            RayResult::Miss { end: till } => {
                legs.push(leg(path, strength, leg_start, till, [0.0; 3], None, false));
                break;
            }
            RayResult::BeganInside { end: till, struck } => {
                legs.push(leg(path, strength, leg_start, till, [0.0; 3], struck, true));
                if struck.is_some() {
                    last_struck = struck;
                }
                break;
            }
            RayResult::Hit { end: till, normal, struck, .. } => {
                let hit = Impact { end: till, face_dir: normal, collider: struck };
                let is_glass = struck.contents() & BRUSH_IS_PANE != 0;
                if is_glass {
                    if shot.is_open_pane(struck) {
                        let Some(after) = shot.step_past(&hit, true, STEP_FORECAST_NUDGE) else {
                            break;
                        };
                        from = after;
                        continue;
                    }
                    shot.note_glass_hit(struck, till);
                }
                legs.push(ShotPathLeg {
                    passed_through_wall: is_glass || struck.is_player(),
                    ..leg_to(path, strength, leg_start, &hit, false)
                });
                last_struck = Some(struck);
                if is_glass {
                    let Some(after) = shot.step_past(&hit, true, STEP_FORECAST_NUDGE) else {
                        break;
                    };
                    from = after;
                    leg_start = till;
                    continue;
                }
                if !struck.is_damageable() {
                    break;
                }
                skip_client_hit = struck.client().or(skip_client_hit);
                if !shot.pen.long_gun_round || !struck.is_player() || struck.is_shield() {
                    break;
                }
                strength *= RIFLE_PASS_THROUGH_FACTOR;
                let Some(after) = shot.step_past(&hit, false, 0.0) else {
                    break;
                };
                from = after;
                leg_start = till;
            }
        }
    }
    (legs, last_struck)
}

/// The first surface a penetrating bullet enters, passing through broken panes on the way, and whether the bullet
/// started inside it. None means the flight is already over; any closing leg has been added to `legs`.
fn first_surface(shot: &Shot<'_>, legs: &mut Vec<ShotPathLeg>) -> Option<(Impact, bool)> {
    let probe = shot.query;
    let closing = |till, startsolid| leg(ShotRouteKind::Penetrate, 1.0, probe.from, till, [0.0; 3], None, startsolid);
    let mut from = probe.from;
    // Panes passed so far; the last allowed one ends the flight without a leg.
    for passed in 1..=PIERCE_LONG_PASS_LIMIT {
        let pane = match shot.trace_on(from, probe.skip_client_hit, probe.ignore_model) {
            RayResult::Invalid { .. } => return None,
            RayResult::Miss { end: till } => {
                legs.push(closing(till, false));
                return None;
            }
            RayResult::BeganInside { struck: None, .. } => {
                legs.push(closing(probe.from, true));
                return None;
            }
            RayResult::BeganInside { struck: Some(struck), .. } => {
                return Some((Impact { end: probe.from, face_dir: negated(shot.dir), collider: struck }, true));
            }
            RayResult::Hit { end: till, normal, struck, .. } => Impact { end: till, face_dir: normal, collider: struck },
        };
        if !shot.is_open_pane(pane.collider) {
            return Some((pane, false));
        }
        from = shot.step_past(&pane, true, STEP_FORECAST_NUDGE)?;
        if passed == PIERCE_LONG_PASS_LIMIT {
            return None;
        }
    }
    None
}

/// The flight of a bullet that can penetrate. It enters a surface, finds where it comes out the far side by tracing
/// back, and pays the thickness out of its strength; it stops when none is left, when the surface has no depth in the
/// table, or at a riot shield.
fn fly_through_surfaces(shot: &Shot<'_>, table: &BulletPierceDepthChart) -> (Vec<ShotPathLeg>, Option<HitTargetId>) {
    let probe = shot.query;
    let dir = shot.dir;
    let path = ShotRouteKind::Penetrate;
    let mut legs = Vec::new();
    let mut strength = 1.0_f32;

    let Some((first, began_inside)) = first_surface(shot, &mut legs) else {
        return (legs, None);
    };
    let mut prior_face = first.collider.depth_surf(0);
    legs.push(ShotPathLeg { passed_through_wall: true, ..leg_to(path, strength, probe.from, &first, began_inside) });
    shot.note_glass_hit(first.collider, first.end);
    let mut last_struck = Some(first.collider);
    let mut entered = first;
    let mut skip_client_hit = probe.skip_client_hit;

    // One pass per surface, as bo2mp Bullet_FirePenetrate counts sv_penetrationCount: each pass pays for the surface
    // entered and strikes the next, so the one the last pass strikes is hit but never paid for.
    for _ in 0..PIERCE_PASS_LIMIT {
        if entered.collider.is_shield() {
            pen_stop_log("riot shield", entered.end);
            break;
        }
        let entry_depth =
            if is_sky(entered.collider) { SKY_PENETRATION_DEPTH } else { table.depth(shot.pen.wall_pierce_class, prior_face) };
        let deepest = entry_depth * shot.pen.wall_pierce_scale;
        if deepest <= 0.0 {
            pen_stop_log("no depth for this surface", entered.end);
            break;
        }
        let Some(inside_start) = shot.step_past(&entered, entered.collider.is_world(), STEP_FORECAST_NUDGE) else {
            break;
        };
        skip_client_hit = entered.collider.client().or(skip_client_hit);
        let ignore_model = entered.collider.script_model().or(probe.ignore_model);
        let leg_start = entered.end;
        let entry_pos = entered.end;

        // The trace starts inside what was entered and meets the next surface beyond it.
        let far = match shot.trace_on(inside_start, skip_client_hit, ignore_model) {
            RayResult::Hit { end: till, normal, struck, .. } => Impact { end: till, face_dir: normal, collider: struck },
            RayResult::Miss { end: till } => {
                legs.push(leg(path, strength, leg_start, till, [0.0; 3], None, false));
                break;
            }
            RayResult::BeganInside { .. } => {
                pen_stop_log("never gets out of a solid", inside_start);
                break;
            }
            RayResult::Invalid { .. } => {
                pen_stop_log("invalid trace", inside_start);
                break;
            }
        };
        if shot.is_open_pane(far.collider) {
            let Some(next) = shot.step_past(&far, true, STEP_FORECAST_NUDGE) else {
                break;
            };
            entered.end = next;
            entered.face_dir = far.face_dir;
            continue;
        }
        shot.note_glass_hit(far.collider, far.end);

        // Trace back from the far surface to find where the bullet came out of what it entered.
        let back_dir = negated(dir);
        let back_end = point_ahead(entry_pos, STEP_BACKCAST_NUDGE, back_dir);
        let back_start = advance_trace(
            far.end,
            negated(far.face_dir),
            back_dir,
            far.collider.is_world(),
            STEP_BACKCAST_NUDGE,
        )
        .unwrap_or(far.end);
        let back = shot.trace_to(
            back_start,
            back_end,
            far.collider.client(),
            far.collider.script_model().or(probe.ignore_model),
        );
        let exit = match back {
            RayResult::Hit { end: till, struck, .. } => Some((till, struck.depth_surf(0), is_sky(struck))),
            RayResult::BeganInside { end: till, struck, .. } => {
                Some((till, struck.map_or(0, |c| c.depth_surf(0)), struck.is_some_and(is_sky)))
            }
            _ => None,
        };

        // With no exit found the far leg carries no thickness and nothing is paid.
        let thickness = match exit {
            None => 0.0,
            Some((exit_pos, exit_surf, exit_is_sky)) => {
                let crossed = distance(entry_pos, exit_pos).max(PIERCE_DEPTH_MINIMUM);
                // The thinner of the entry and exit allowances applies, except that a sky face is a fixed depth.
                let allowance = if exit_is_sky {
                    SKY_PENETRATION_DEPTH
                } else {
                    deepest.min(table.depth(shot.pen.wall_pierce_class, exit_surf) * shot.pen.wall_pierce_scale)
                };
                if allowance <= 0.0 {
                    break;
                }
                strength -= crossed / allowance;
                if strength <= SPENT_BULLET {
                    break;
                }
                legs.push(ShotPathLeg {
                    passed_through_wall: true,
                    depth: crossed,
                    face_bits: leg_surface_with_exit_flag(entered.collider),
                    ..leg(path, strength, entry_pos, exit_pos, dir, Some(entered.collider), false)
                });
                crossed
            }
        };

        prior_face = far.collider.depth_surf(prior_face);
        legs.push(ShotPathLeg { passed_through_wall: true, depth: thickness, ..leg_to(path, strength, leg_start, &far, false) });
        last_struck = Some(far.collider);
        entered = far;
    }
    (legs, last_struck)
}

/// The surface flags of the exit mark left on `collider`.
fn leg_surface_with_exit_flag(collider: HitTargetId) -> u32 {
    collider.surface_flags() | fx_t6::runtime::HIT_ON_WAY_OUT_BIT
}

/// The pane a glass hit struck, from the hit type and id a segment carries.
pub fn pane_from_hit(impact_category: i32, struck_entity_num: u16) -> Option<u32> {
    (impact_category == GLASS_HIT_CODE && struck_entity_num != 0).then(|| u32::from(struck_entity_num) - 1)
}
