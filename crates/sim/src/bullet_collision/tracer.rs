//! One straight trace. The static world, the players and the scripted entities each offer strikes along the line;
//! the nearest wins.

use std::cmp::Ordering;

use trace_t6::Trace;

use super::collider::{
    AuthorityModelOwner, BulletTraceQuery, ColliderId, HITLOC_SHIELD, TraceInvalidReason, TraceOutcome,
};
use super::entity_caps::EntityCollisionTraceGeom;
use super::geometry::{BoxHit, RayBox, geom_bounds, pose_bones_bounds, pose_box, ray_aabb_box, ray_obb};
use super::logs::{log_shield_holders, shield_log_on};
use super::posed_coll::{AuthorityDObjCollTrace, entity_normal_to_world, world_point_to_entity};
use super::poses::PlayerCollisionPose;
use super::{CONTENTS_BODY, contents_match_mask};
use crate::world::{SimBrush, SimClipBsp, SimClipCmodels, SimClipMesh};

/// Two strikes closer than this along the line count as the same distance (the legacy trace backs off a brush's
/// plane by as much).
const SURFACE_CLIP_EPSILON: f32 = 0.125;

/// Something the line passes through, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Strike {
    pub fraction: f32,
    pub endpos: [f32; 3],
    pub normal: [f32; 3],
    pub collider: ColliderId,
}

impl Strike {
    fn on(collider: ColliderId, hit: BoxHit) -> Self {
        Self { fraction: hit.fraction, endpos: hit.endpos, normal: hit.normal, collider }
    }
}

/// Everything a trace can run into.
#[derive(Clone, Copy)]
pub(super) struct Scene<'a> {
    pub brushes: &'a [SimBrush],
    pub bsp: &'a SimClipBsp,
    pub cmodels: &'a SimClipCmodels,
    pub mesh: &'a SimClipMesh,
    pub players: &'a [PlayerCollisionPose],
    pub script_models: &'a [EntityCollisionTraceGeom],
    pub glass_is_solid: &'a dyn Fn(u16) -> bool,
}

pub fn bullet_trace(
    brushes: &[SimBrush],
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> TraceOutcome {
    bullet_trace_with_entity_models(
        brushes,
        &SimClipBsp::default(),
        &SimClipCmodels::default(),
        &SimClipMesh::default(),
        players,
        &[],
        query,
        glass_is_solid,
    )
}

pub fn bullet_trace_with_entity_models(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> TraceOutcome {
    let scene = Scene { brushes, bsp, cmodels, mesh, players, script_models, glass_is_solid };
    scene.trace(query, false)
}

/// True when the model has collision triangles a line with this mask could meet.
pub(crate) fn coll_tris_clip_available(coll: Option<&AuthorityDObjCollTrace>, mask: u32) -> bool {
    coll.is_some_and(|coll| {
        coll.model.coll_lod >= 0 && coll.model.surfs.iter().any(|surf| surf.contents & mask != 0 && surf.tri_count != 0)
    })
}

/// What tracing a model's triangles came to.
enum TriangleVerdict {
    Hit(Strike),
    Missed,
    /// The model has no usable triangles; its part boxes stand in.
    Unavailable,
}

fn trace_model_triangles(
    coll: Option<&AuthorityDObjCollTrace>,
    owner: AuthorityModelOwner,
    query: &BulletTraceQuery,
) -> TriangleVerdict {
    let Some(coll) = coll.filter(|coll| coll_tris_clip_available(Some(coll), query.mask)) else {
        return TriangleVerdict::Unavailable;
    };
    let (Some(local_start), Some(local_end)) =
        (world_point_to_entity(coll.world_from_model, query.start), world_point_to_entity(coll.world_from_model, query.end))
    else {
        return TriangleVerdict::Unavailable;
    };
    let mut found = Trace { fraction: 1.0, ..Trace::default() };
    let bone =
        coll.model.trace_line_animated(&mut found, local_start, local_end, query.mask, &coll.bones, &coll.hide_part_bits);
    let Ok(bone) = u16::try_from(bone) else {
        return TriangleVerdict::Missed;
    };
    let along = |axis: usize| query.start[axis] + (query.end[axis] - query.start[axis]) * found.fraction;
    TriangleVerdict::Hit(Strike {
        fraction: found.fraction,
        endpos: [along(0), along(1), along(2)],
        normal: entity_normal_to_world(coll.world_from_model, found.normal),
        collider: ColliderId::EntityDObjBone {
            owner,
            bone,
            part_classification: 0,
            surface_flags: found.sflags,
        },
    })
}

impl Scene<'_> {
    /// The first thing the line meets. `locational` is set for a gun's bullet, which meets a model's triangles and
    /// never its collision brushes.
    pub(super) fn trace(&self, query: &BulletTraceQuery, locational: bool) -> TraceOutcome {
        if !is_finite(query.start) || !is_finite(query.end) {
            return TraceOutcome::Invalid { reason: TraceInvalidReason::NonFiniteInput };
        }

        let (world, glass_encoded) = self.trace_world(query);
        let world_collider = ColliderId::World { surface_flags: world.sflags, contents: world.cflags, glass_encoded };
        if world.startsolid || world.allsolid {
            return TraceOutcome::StartSolid { collider: Some(world_collider), end: query.start };
        }

        let mut strikes = Vec::new();
        // Players and entities only count up to the point the world already stopped the line.
        let mut reach = query.end;
        if world.fraction < 1.0 {
            reach = world.end_pos(query.start, query.end);
            strikes.push(Strike { fraction: world.fraction, endpos: reach, normal: world.normal, collider: world_collider });
        }

        if let Some(inside) = self.scan_players(query, reach, &mut strikes) {
            return inside;
        }
        if let Some(inside) = self.scan_script_models(query, locational, &mut strikes) {
            return inside;
        }

        mark_shield_strikes(&mut strikes, self.players, query);
        let outcome = nearest_strike(query, &mut strikes);
        if shield_log_on() {
            log_shield_holders(self.players, query, reach, &strikes, &outcome);
        }
        outcome
    }

    fn trace_world(&self, query: &BulletTraceQuery) -> (Trace, u16) {
        let mut hit = crate::world::clip_trace(
            self.brushes,
            self.bsp,
            self.mesh,
            query.start,
            query.end,
            [0.0; 3],
            [0.0; 3],
            query.mask,
            self.glass_is_solid,
        );
        if let Some(statics) = self.mesh.t6_static.filter(|_| hit.fraction != 0.0) {
            crate::world::t6_static_trace(statics, query.start, query.end, query.mask, &mut hit);
        }
        let glass = trace_t6::trace_get_glass_hit_id(&hit).unwrap_or(0);
        (hit, glass)
    }

    /// Adds a strike for every player box the line crosses. A line that starts inside a box ends the trace at once
    /// (`Some`).
    fn scan_players(&self, query: &BulletTraceQuery, reach: [f32; 3], strikes: &mut Vec<Strike>) -> Option<TraceOutcome> {
        if query.mask & CONTENTS_BODY == 0 {
            return None;
        }
        for pose in self.players {
            if query.ignore == Some(pose.client) || query.ignore_hit == Some(pose.client) {
                continue;
            }
            if pose.bones.is_empty() {
                let (mins, maxs) = pose_box(pose);
                let collider = ColliderId::Player { client: pose.client, life: pose.life_sequence, hitloc: 0 };
                match ray_aabb_box(query.start, query.end, mins, maxs) {
                    RayBox::Inside => return Some(TraceOutcome::StartSolid { collider: Some(collider), end: query.start }),
                    RayBox::Hit(hit) => strikes.push(Strike::on(collider, hit)),
                    RayBox::Miss => {}
                }
                continue;
            }
            if let Some((mins, maxs)) = pose_bones_bounds(pose)
                && matches!(ray_aabb_box(query.start, reach, mins, maxs), RayBox::Miss)
            {
                continue;
            }
            for bone in &pose.bones {
                let collider = ColliderId::Player {
                    client: pose.client,
                    life: pose.life_sequence,
                    hitloc: bone.part_classification,
                };
                if let Some(hit) = ray_obb(query.start, query.end, bone, collider, Some(pose.origin)) {
                    strikes.push(Strike::on(collider, hit));
                }
            }
        }
        None
    }

    /// Adds a strike for every scripted-entity surface the line crosses. A line that starts inside a linked brush
    /// ends the trace at once (`Some`).
    fn scan_script_models(
        &self,
        query: &BulletTraceQuery,
        locational: bool,
        strikes: &mut Vec<Strike>,
    ) -> Option<TraceOutcome> {
        for geom in self.script_models {
            if query.ignore_model.is_some() && geom.owner.script_model() == query.ignore_model {
                continue;
            }
            // Nothing to hit, or the line cannot reach what there is.
            let Some((mins, maxs)) = geom_bounds(geom, self.cmodels) else {
                continue;
            };
            if matches!(ray_aabb_box(query.start, query.end, mins, maxs), RayBox::Miss) {
                continue;
            }

            if let Some(inside) = self.scan_linked_brushes(geom, query, locational, strikes) {
                return Some(inside);
            }

            let Some(posed) = geom.collision.as_ref() else {
                continue;
            };
            if !contents_match_mask(geom.dobj_contents, query.mask) {
                continue;
            }
            match trace_model_triangles(posed.coll.as_ref(), geom.owner, query) {
                TriangleVerdict::Hit(strike) => {
                    strikes.push(strike);
                    continue;
                }
                TriangleVerdict::Missed => continue,
                TriangleVerdict::Unavailable => {}
            }
            let model_origin = posed.coll.as_ref().map(|coll| coll.world_from_model.w_axis.truncate().to_array());
            for bone in &posed.bones {
                let collider = ColliderId::EntityDObjBone {
                    owner: geom.owner,
                    bone: bone.bone,
                    part_classification: bone.part_classification,
                    surface_flags: 0,
                };
                if let Some(hit) = ray_obb(query.start, query.end, bone, collider, model_origin) {
                    strikes.push(Strike::on(collider, hit));
                }
            }
        }
        None
    }

    fn scan_linked_brushes(
        &self,
        geom: &EntityCollisionTraceGeom,
        query: &BulletTraceQuery,
        locational: bool,
        strikes: &mut Vec<Strike>,
    ) -> Option<TraceOutcome> {
        let Some(map) = self.bsp.t6 else {
            return None;
        };
        // A gun's bullet meets a script model's triangles alone, never its collision brushes (the way BO2 traces a
        // model): the walk-in brushes of a car stop no bullet, so one goes in through a broken window.
        let triangles_only = locational && geom.model_key.is_some();
        for brush in &geom.linked_brushes {
            if triangles_only && self.cmodels.model_cmodels.values().any(|&handle| handle == brush.cmodel_handle) {
                continue;
            }
            let hit = crate::world::t6_transformed_trace(
                map,
                query.start,
                query.end,
                [0.0; 3],
                [0.0; 3],
                crate::world::T6Model::Brush(brush.cmodel_handle),
                query.mask,
                brush.origin,
                brush.angles,
            );
            let collider = ColliderId::EntityLinkedBrush {
                owner: geom.owner,
                cmodel_handle: brush.cmodel_handle,
                surface_flags: hit.sflags,
            };
            if hit.startsolid || hit.allsolid {
                return Some(TraceOutcome::StartSolid { collider: Some(collider), end: query.start });
            }
            if hit.fraction < 1.0 {
                strikes.push(Strike {
                    fraction: hit.fraction,
                    endpos: hit.end_pos(query.start, query.end),
                    normal: hit.normal,
                    collider,
                });
            }
        }
        None
    }
}

/// A player whose shield has no boxes of its own takes a hit from the side the shield faces as a hit on the shield.
/// (When the shield has boxes the bullet meets them, or the body, directly.)
fn mark_shield_strikes(strikes: &mut [Strike], players: &[PlayerCollisionPose], query: &BulletTraceQuery) {
    let dir = [query.end[0] - query.start[0], query.end[1] - query.start[1], query.end[2] - query.start[2]];
    for strike in strikes {
        let ColliderId::Player { client, life, .. } = strike.collider else {
            continue;
        };
        let holder = players.iter().find(|pose| pose.client == client && pose.life_sequence == life);
        if holder.is_some_and(|pose| pose.shield_stands_in_for(dir)) {
            strike.collider = ColliderId::Player { client, life, hitloc: HITLOC_SHIELD };
        }
    }
}

fn line_length(query: &BulletTraceQuery) -> f32 {
    let d = [query.end[0] - query.start[0], query.end[1] - query.start[1], query.end[2] - query.start[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// True when one strike is the world and the other something that can be damaged, lying within a clip epsilon of
/// each other: then the thing wins, as it does in BO2 for a prop standing against a wall.
fn flush_with_world(query: &BulletTraceQuery, a: &Strike, b: &Strike) -> bool {
    let world_vs_thing = (a.collider.is_world() && b.collider.is_damageable())
        || (b.collider.is_world() && a.collider.is_damageable());
    if !world_vs_thing {
        return false;
    }
    let len = line_length(query);
    (a.fraction * len - b.fraction * len).abs() <= SURFACE_CLIP_EPSILON
}

/// How strikes at the same distance are told apart: players first, then entity parts, then entity brushes, then the
/// world.
fn tie_break(strike: &Strike) -> (u8, u32) {
    match strike.collider {
        ColliderId::Player { client, .. } => (0, client.0),
        ColliderId::EntityDObjBone { owner, bone, .. } => (1, owner.order_key().wrapping_add(u32::from(bone))),
        ColliderId::EntityLinkedBrush { owner, .. } => (2, owner.order_key()),
        ColliderId::World { .. } => (3, 0),
    }
}

fn nearest_strike(query: &BulletTraceQuery, strikes: &mut [Strike]) -> TraceOutcome {
    strikes.sort_by(|a, b| {
        if flush_with_world(query, a, b) {
            return tie_break(a).cmp(&tie_break(b));
        }
        match a.fraction.partial_cmp(&b.fraction).unwrap_or(Ordering::Equal) {
            Ordering::Equal => tie_break(a).cmp(&tie_break(b)),
            by_distance => by_distance,
        }
    });
    match strikes.first() {
        None => TraceOutcome::Miss { end: query.end },
        Some(best) => TraceOutcome::Hit {
            fraction: best.fraction,
            end: best.endpos,
            normal: best.normal,
            collider: best.collider,
        },
    }
}

fn is_finite(v: [f32; 3]) -> bool {
    v.iter().all(|c| c.is_finite())
}

pub(super) fn is_finite_vec3(v: [f32; 3]) -> bool {
    is_finite(v)
}
