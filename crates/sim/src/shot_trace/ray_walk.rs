//! One straight trace. The static world, the players and the scripted entities each offer strikes along the line;
//! the nearest wins.

use std::cmp::Ordering;

use trace_t6::Trace;

use super::collider::{
    ServerModelSource, ShotRayRequest, HitTargetId, HITLOC_SHIELD, RayRejectCause, RayResult,
};
use super::entity_caps::EntityRayShape;
use super::ray_boxes::{BoxHit, RayBox, geom_bounds, pose_bones_bounds, pose_box, ray_aabb_box, ray_obb};
use super::logs::{log_shield_holders, shield_log_on};
use super::posed_coll::{ServerModelHitTrace, entity_normal_to_world, world_point_to_entity};
use super::poses::PlayerHitboxPose;
use super::{SOLID_PLAYER_BODY, contents_match_mask};
use crate::sim_parts::{SolidBrush, CollisionTreeData, CollisionSubmodels, CollisionTriangleData};

/// Two strikes closer than this along the line count as the same distance (the legacy trace backs off a brush's
/// plane by as much).
const SURFACE_CLIP_SLACK: f32 = 0.125;

/// Something the line passes through, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Strike {
    pub share: f32,
    pub finish_point: [f32; 3],
    pub face_dir: [f32; 3],
    pub collider: HitTargetId,
}

impl Strike {
    fn on(collider: HitTargetId, hit: BoxHit) -> Self {
        Self { share: hit.share, finish_point: hit.finish_point, face_dir: hit.face_dir, collider }
    }
}

/// Everything a trace can run into.
#[derive(Clone, Copy)]
pub(super) struct Scene<'a> {
    pub brushes: &'a [SolidBrush],
    pub bsp: &'a CollisionTreeData,
    pub cmodels: &'a CollisionSubmodels,
    pub mesh: &'a CollisionTriangleData,
    pub players: &'a [PlayerHitboxPose],
    pub placed_models: &'a [EntityRayShape],
    pub pane_blocks_shots: &'a dyn Fn(u16) -> bool,
}

pub fn trace_shot(
    brushes: &[SolidBrush],
    players: &[PlayerHitboxPose],
    query: &ShotRayRequest,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> RayResult {
    trace_shot_against_models(
        brushes,
        &CollisionTreeData::default(),
        &CollisionSubmodels::default(),
        &CollisionTriangleData::default(),
        players,
        &[],
        query,
        pane_blocks_shots,
    )
}

pub fn trace_shot_against_models(
    brushes: &[SolidBrush],
    bsp: &CollisionTreeData,
    cmodels: &CollisionSubmodels,
    mesh: &CollisionTriangleData,
    players: &[PlayerHitboxPose],
    placed_models: &[EntityRayShape],
    query: &ShotRayRequest,
    pane_blocks_shots: &dyn Fn(u16) -> bool,
) -> RayResult {
    let scene = Scene { brushes, bsp, cmodels, mesh, players, placed_models, pane_blocks_shots };
    scene.trace(query, false)
}

/// True when the model has collision triangles a line with this mask could meet.
pub(crate) fn coll_tris_clip_available(coll: Option<&ServerModelHitTrace>, filter: u32) -> bool {
    coll.is_some_and(|coll| {
        coll.model.collision_detail_tier >= 0 && coll.model.surfs.iter().any(|surf| surf.solid_bits & filter != 0 && surf.tri_tally != 0)
    })
}

/// What tracing a model's triangles came to.
enum TriangleVerdict {
    Hit(Strike),
    Missed,
    /// The model has no usable triangles; its part boxes stand in.
    NoTriangles,
}

fn trace_model_triangles(
    posed: Option<&ServerModelHitTrace>,
    owner: ServerModelSource,
    probe: &ShotRayRequest,
) -> TriangleVerdict {
    let Some(posed) = posed.filter(|posed| coll_tris_clip_available(Some(posed), probe.bits)) else {
        return TriangleVerdict::NoTriangles;
    };
    let (Some(near_from), Some(local_end)) =
        (world_point_to_entity(posed.placement_matrix, probe.from), world_point_to_entity(posed.placement_matrix, probe.to))
    else {
        return TriangleVerdict::NoTriangles;
    };
    let mut found = Trace { fraction: 1.0, ..Trace::default() };
    let struck =
        posed.model.trace_line_animated(&mut found, near_from, local_end, probe.bits, &posed.bones, &posed.hidden_parts);
    // A negative bone is a miss.
    let Some(bone) = u16::try_from(struck).ok() else {
        return TriangleVerdict::Missed;
    };
    let along = |axis: usize| probe.from[axis] + (probe.to[axis] - probe.from[axis]) * found.fraction;
    TriangleVerdict::Hit(Strike {
        share: found.fraction,
        finish_point: [along(0), along(1), along(2)],
        face_dir: entity_normal_to_world(posed.placement_matrix, found.normal),
        collider: HitTargetId::PosedBone {
            owner,
            bone,
            body_part_category: 0,
            surface_flags: found.sflags,
        },
    })
}

impl Scene<'_> {
    /// The first thing the line meets. `locational` is set for a gun's bullet, which meets a model's triangles and
    /// never its collision brushes.
    pub(super) fn trace(&self, probe: &ShotRayRequest, locational: bool) -> RayResult {
        if !is_finite(probe.from) || !is_finite(probe.to) {
            return RayResult::Invalid { reason: RayRejectCause::InfiniteOrNanInput };
        }

        let (world, glass_hit_code) = self.trace_world(probe);
        let level_obstacle = HitTargetId::World { surface_flags: world.sflags, contents: world.cflags, glass_hit_code };
        if began_inside(&world, locational) {
            return RayResult::BeganInside { struck: Some(level_obstacle), end: probe.from };
        }

        let mut strikes = Vec::new();
        // Players and entities only count up to the point the world already stopped the line.
        let world_stopped = world.fraction < 1.0;
        let reach = if world_stopped { world.finish_location(probe.from, probe.to) } else { probe.to };
        if world_stopped {
            strikes.push(Strike { collider: level_obstacle, face_dir: world.normal, finish_point: reach, share: world.fraction });
        }

        if let Some(inside) = self.scan_players(probe, reach, &mut strikes) {
            return inside;
        }
        if let Some(inside) = self.scan_script_models(probe, locational, &mut strikes) {
            return inside;
        }

        mark_shield_strikes(&mut strikes, self.players, probe);
        let outcome = nearest_strike(probe, &mut strikes);
        if shield_log_on() {
            log_shield_holders(self.players, probe, reach, &strikes, &outcome);
        }
        outcome
    }

    fn trace_world(&self, probe: &ShotRayRequest) -> (Trace, u16) {
        let (from, to, filter) = (probe.from, probe.to, probe.bits);
        let point = [0.0; 3];
        let mut hit = crate::sim_parts::sweep_box_trace(self.brushes, self.bsp, self.mesh, from, to, point, point, filter, self.pane_blocks_shots);
        if let Some(statics) = self.mesh.t6_static.filter(|_| hit.fraction != 0.0) {
            crate::sim_parts::t6_static_trace(statics, from, to, filter, &mut hit);
        }
        let mut glass = trace_t6::glass_hit_slot(&hit).unwrap_or(0);
        if glass == 0 && hit.fraction < 1.0 && hit.cflags & weapon_t6::penetration::BRUSH_IS_PANE != 0 {
            glass = crate::sim_parts::standing_pane_struck(self.brushes, self.bsp, from, to, hit.fraction, self.pane_blocks_shots);
        }
        (hit, glass)
    }

    /// Adds a strike for every player box the line crosses. A line that starts inside a box ends the trace at once
    /// (`Some`).
    fn scan_players(&self, probe: &ShotRayRequest, reach: [f32; 3], strikes: &mut Vec<Strike>) -> Option<RayResult> {
        if probe.bits & SOLID_PLAYER_BODY == 0 {
            return None;
        }
        for stance in self.players {
            if probe.ignore == Some(stance.client) || probe.skip_client_hit == Some(stance.client) {
                continue;
            }
            if stance.joints.is_empty() {
                let (lo, maxs) = pose_box(stance);
                let struck = HitTargetId::Player { client: stance.client, life: stance.life_sequence, hitloc: 0 };
                match ray_aabb_box(probe.from, probe.to, lo, maxs) {
                    RayBox::Inside => return Some(RayResult::BeganInside { struck: Some(struck), end: probe.from }),
                    RayBox::Hit(hit) => strikes.push(Strike::on(struck, hit)),
                    RayBox::Miss => {}
                }
                continue;
            }
            if let Some((lo, maxs)) = pose_bones_bounds(stance)
                && matches!(ray_aabb_box(probe.from, reach, lo, maxs), RayBox::Miss)
            {
                continue;
            }
            strikes.extend(stance.joints.iter().filter_map(|bone| {
                let struck = HitTargetId::Player { client: stance.client, life: stance.life_sequence, hitloc: bone.body_part_category };
                ray_obb(probe.from, probe.to, bone, struck, Some(stance.origin)).map(|hit| Strike::on(struck, hit))
            }));
        }
        None
    }

    /// Adds a strike for every scripted-entity surface the line crosses. A line that starts inside a linked brush
    /// ends the trace at once (`Some`).
    fn scan_script_models(
        &self,
        probe: &ShotRayRequest,
        locational: bool,
        strikes: &mut Vec<Strike>,
    ) -> Option<RayResult> {
        for shape_data in self.placed_models {
            if probe.ignore_model.is_some() && shape_data.owner.scripted_model() == probe.ignore_model {
                continue;
            }
            // Nothing to hit, or the line cannot reach what there is.
            let Some((mins, maxs)) = geom_bounds(shape_data, self.cmodels) else {
                continue;
            };
            if matches!(ray_aabb_box(probe.from, probe.to, mins, maxs), RayBox::Miss) {
                continue;
            }

            if let Some(inside) = self.scan_linked_brushes(shape_data, probe, locational, strikes) {
                return Some(inside);
            }

            let Some(posed) = shape_data.collision.as_ref() else {
                continue;
            };
            if !contents_match_mask(shape_data.model_content_mask, probe.bits) {
                continue;
            }
            match trace_model_triangles(posed.coll.as_ref(), shape_data.owner, probe) {
                TriangleVerdict::Hit(strike) => {
                    strikes.push(strike);
                    continue;
                }
                TriangleVerdict::Missed => continue,
                TriangleVerdict::NoTriangles => {}
            }
            let model_origin = posed.coll.as_ref().map(|coll| coll.placement_matrix.w_axis.truncate().to_array());
            let owner = shape_data.owner;
            strikes.extend(posed.bones.iter().filter_map(|part| {
                let (bone, body_part_category) = (part.bone, part.body_part_category);
                // A prop's box keeps its model's surface (a mannequin is plastic); a body's boxes have none and play
                // blood.
                let face_bits = posed
                    .coll
                    .as_ref()
                    .and_then(|c| {
                        c.model.surfs.iter().find(|s| s.joint_position == i32::from(bone) && s.face_property_bits != 0)
                    })
                    .map_or(posed.face_bits, |s| s.face_property_bits);
                let collider = HitTargetId::PosedBone { owner, bone, body_part_category, surface_flags: face_bits };
                ray_obb(probe.from, probe.to, part, collider, model_origin).map(|hit| Strike::on(collider, hit))
            }));
        }
        None
    }

    fn scan_linked_brushes(
        &self,
        shape_data: &EntityRayShape,
        probe: &ShotRayRequest,
        locational: bool,
        strikes: &mut Vec<Strike>,
    ) -> Option<RayResult> {
        let Some(map) = self.bsp.t6 else {
            return None;
        };
        // A gun's bullet meets a script model's triangles alone, never its collision brushes (the way BO2 traces a
        // model): the walk-in brushes of a car stop no bullet, so one goes in through a broken window.
        let triangles_only = locational && shape_data.mesh_asset_name.is_some();
        for brush in &shape_data.attached_solids {
            if triangles_only && self.cmodels.model_cmodels.values().any(|&handle| handle == brush.collision_model_ref) {
                continue;
            }
            let Some((model, _)) = crate::sim_parts::linked_model(map, &self.bsp.t6_model_brushes, brush.collision_model_ref) else {
                continue;
            };
            let (point, place) = ([0.0; 3], (brush.origin, brush.turn));
            let hit = crate::sim_parts::t6_transformed_trace(map, probe.from, probe.to, point, point, model, probe.bits, place.0, place.1);
            let struck = HitTargetId::AttachedBrush {
                owner: shape_data.owner,
                collision_model_ref: brush.collision_model_ref,
                surface_flags: hit.sflags,
            };
            if began_inside(&hit, locational) {
                return Some(RayResult::BeganInside { struck: Some(struck), end: probe.from });
            }
            let crossed = hit.fraction < 1.0;
            if crossed {
                let finish_point = hit.finish_location(probe.from, probe.to);
                strikes.push(Strike { collider: struck, face_dir: hit.normal, finish_point, share: hit.fraction });
            }
        }
        None
    }
}

/// Whether a trace counts as begun inside what it met. A bullet's trace (bo2mp Bullet_Trace) is stopped only by a
/// solid it never gets out of: one that starts inside a wall and leaves it meets whatever lies beyond.
fn began_inside(hit: &Trace, locational: bool) -> bool {
    hit.fully_embedded || (hit.starts_inside && !locational)
}

/// A player whose shield has no boxes of its own takes a hit from the side the shield faces as a hit on the shield.
/// (When the shield has boxes the bullet meets them, or the body, directly.)
fn mark_shield_strikes(strikes: &mut [Strike], players: &[PlayerHitboxPose], probe: &ShotRayRequest) {
    let dir = [probe.to[0] - probe.from[0], probe.to[1] - probe.from[1], probe.to[2] - probe.from[2]];
    for strike in strikes {
        let HitTargetId::Player { client, life, .. } = strike.collider else {
            continue;
        };
        let holder = players.iter().find(|pose| pose.client == client && pose.life_sequence == life);
        if holder.is_some_and(|pose| pose.shield_stands_in_for(dir)) {
            strike.collider = HitTargetId::Player { client, life, hitloc: HITLOC_SHIELD };
        }
    }
}

fn line_length(probe: &ShotRayRequest) -> f32 {
    let d = [probe.to[0] - probe.from[0], probe.to[1] - probe.from[1], probe.to[2] - probe.from[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// True when one strike is the world and the other something that can be damaged, lying within a clip epsilon of
/// each other: then the thing wins, as it does in BO2 for a prop standing against a wall.
fn flush_with_world(query: &ShotRayRequest, a: &Strike, b: &Strike) -> bool {
    let world_vs_thing = (a.collider.is_world() && b.collider.is_damageable())
        || (b.collider.is_world() && a.collider.is_damageable());
    if !world_vs_thing {
        return false;
    }
    let len = line_length(query);
    (a.share * len - b.share * len).abs() <= SURFACE_CLIP_SLACK
}

/// How strikes at the same distance are told apart: players first, then entity parts, then entity brushes, then the
/// world.
fn tie_break(strike: &Strike) -> (u8, u32) {
    match strike.collider {
        HitTargetId::Player { client, .. } => (0, client.0),
        HitTargetId::PosedBone { owner, bone: joint, .. } => (1, owner.order_key().wrapping_add(u32::from(joint))),
        HitTargetId::AttachedBrush { owner, .. } => (2, owner.order_key()),
        HitTargetId::World { .. } => (3, 0),
    }
}

fn nearest_strike(query: &ShotRayRequest, strikes: &mut [Strike]) -> RayResult {
    strikes.sort_by(|a, b| {
        if flush_with_world(query, a, b) {
            return tie_break(a).cmp(&tie_break(b));
        }
        match a.share.partial_cmp(&b.share).unwrap_or(Ordering::Equal) {
            Ordering::Equal => tie_break(a).cmp(&tie_break(b)),
            by_distance => by_distance,
        }
    });
    match strikes.first() {
        None => RayResult::Miss { end: query.to },
        Some(&Strike { share: fraction, finish_point, face_dir: normal, collider, .. }) => {
            RayResult::Hit { fraction, end: finish_point, normal, struck: collider }
        }
    }
}

fn is_finite(v: [f32; 3]) -> bool {
    v.into_iter().all(f32::is_finite)
}

pub(super) fn is_finite_vec3(v: [f32; 3]) -> bool {
    is_finite(v)
}
