//! What a missile can meet, and the small questions it asks of the world around it.

use trace_t6::surface_type_from_flags;

use super::facts::EquipmentRuntimeFacts;
use super::vec3::{dot, norm};
use crate::bullet_collision::{AuthorityDObjCollisionBone, EntityCollisionTraceGeom, PlayerCollisionPose};
use crate::frame::FrameWorld;
use crate::identities::ProjectileId;
use crate::{BulletTraceQuery, ColliderId, ScriptModelId, SimContent, TraceOutcome, bullet_trace_with_entity_models};

pub(super) const SURF_FLESH: u8 = 7;
const SURF_GLASS: u8 = 9;
pub(super) const SURF_WATER: u8 = 20;
pub(super) const SURF_RIOTSHIELD: u8 = 31;
const CONTENTS_WATER: u32 = movement_t6::water::CONTENTS_WATER;

/// The players a missile can meet. The server meets them by their hit boxes as a bullet does; the client draws the
/// server's path and never bounces a missile off a player. A prediction copy knows the other players only as a
/// whole-body box, so those are left out here and the next snapshot brings the bounce.
pub(super) fn missile_bodies(world: &FrameWorld) -> Vec<PlayerCollisionPose> {
    let remote = world.prediction_remote_bodies();
    let mut bodies = world.alive_collision_poses();
    bodies.retain(|pose| remote.iter().all(|body| body.client != pose.client));
    bodies
}

/// The equipment row of `weapon`, or its launcher row. A missile with neither is a bug in the content, not something
/// to run with zeroes.
pub(super) fn required_facts(world: &FrameWorld, weapon: u32) -> EquipmentRuntimeFacts {
    match world.equipment_facts_for(weapon).or_else(|| world.missile_launch_facts(weapon)) {
        Some(facts) => facts,
        None => panic!("a missile needs a validated equipment row; bounce/damage must not default to zero"),
    }
}

/// Every scripted solid except the ones attached to missile `id`.
pub(super) fn script_models_without(world: &FrameWorld, id: ProjectileId) -> Vec<EntityCollisionTraceGeom> {
    let attached = world.missile_collision_models(id);
    world
        .entity_collision_capabilities()
        .iter()
        .filter(|caps| match caps.owner.script_model() {
            Some(model) => !attached.contains(&model),
            None => true,
        })
        .map(|caps| caps.trace_geom())
        .collect()
}

/// Whether `at` is in water.
pub(super) fn in_water(world: &FrameWorld, at: [f32; 3]) -> bool {
    let probe = world.trace_clip(at, at, [0.0; 3], [0.0; 3], CONTENTS_WATER);
    probe.startsolid || probe.allsolid
}

/// A scripted glass that takes damage struck by a missile: the model, the bone hit (None for a linked brush) and
/// whether it can break. None for anything else (world glass breaks as a pane).
pub(super) fn entity_glass(world: &mut FrameWorld, collider: ColliderId) -> Option<(ScriptModelId, Option<usize>, bool)> {
    let (owner, bone, flags) = match collider {
        ColliderId::EntityDObjBone { owner, bone, surface_flags, .. } => (owner, Some(usize::from(bone)), surface_flags),
        ColliderId::EntityLinkedBrush { owner, surface_flags, .. } => (owner, None, surface_flags),
        ColliderId::World { .. } | ColliderId::Player { .. } => return None,
    };
    if surface_type_from_flags(flags) != SURF_GLASS {
        return None;
    }
    let model = owner.script_model()?;
    let breakable = crate::t6::destructible::takes_damage(world.ecs(), model)?;
    Some((model, bone, breakable))
}

/// A missile that stuck to a scripted entity tells the scripts which one.
pub(super) fn tell_struck_entity(world: &mut FrameWorld, id: ProjectileId, collider: ColliderId) {
    let owner = match collider {
        ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. } => owner,
        ColliderId::World { .. } | ColliderId::Player { .. } => return,
    };
    if let Some(model) = owner.script_model() {
        crate::t6::grenade_stuck::struck(world.ecs(), id.0, model);
    }
}

/// The facing a hit gets when the move started inside something: straight back along the move, or none when it did
/// not move.
pub(super) fn startsolid_normal(start: [f32; 3], end: [f32; 3]) -> [f32; 3] {
    let back = [start[0] - end[0], start[1] - end[1], start[2] - end[2]];
    let len = norm(back);
    if len < 1e-6 {
        return [0.0; 3];
    }
    [back[0] / len, back[1] / len, back[2] / len]
}

/// Whether `point` lies inside the box of `bone`.
fn inside_bone(bone: &AuthorityDObjCollisionBone, point: [f32; 3]) -> bool {
    let offset: [f32; 3] = core::array::from_fn(|axis| point[axis] - bone.center[axis]);
    (0..3).all(|axis| dot(offset, bone.axes[axis]).abs() <= bone.half_size[axis])
}

/// A missile's trace against player hit boxes. One that starts inside a hit box and leaves it passes out freely; one
/// that stays inside while it moves in toward the player's middle stops where it is, facing level and straight out
/// from his middle. (The shared trace calls every start inside a hit box a start inside something.)
pub(super) fn missile_trace(
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    trace: &dyn Fn(&[PlayerCollisionPose], &BulletTraceQuery) -> TraceOutcome,
) -> TraceOutcome {
    let first = trace(players, query);
    if !matches!(first, TraceOutcome::StartSolid { collider: Some(ColliderId::Player { .. }), .. }) {
        return first;
    }
    let heading = [query.end[0] - query.start[0], query.end[1] - query.start[1]];
    let mut outside = players.to_vec();
    for pose in &mut outside {
        if query.ignore == Some(pose.client) || query.ignore_hit == Some(pose.client) {
            continue;
        }
        let away = [query.start[0] - pose.origin[0], query.start[1] - pose.origin[1]];
        if heading[0] * away[0] + heading[1] * away[1] < 0.0
            && let Some(bone) = pose.bones.iter().find(|b| inside_bone(b, query.start) && inside_bone(b, query.end))
        {
            let len = away[0].hypot(away[1]);
            let normal = if len > 0.0 { [away[0] / len, away[1] / len, 0.0] } else { [0.0; 3] };
            return TraceOutcome::Hit {
                fraction: 0.0,
                end: query.start,
                normal,
                collider: ColliderId::Player {
                    client: pose.client,
                    life: pose.life_sequence,
                    hitloc: bone.part_classification,
                },
            };
        }
        pose.bones.retain(|b| !inside_bone(b, query.start));
    }
    trace(&outside, query)
}

/// Whether glass pane `piece` still stands, given the damage each hit pane has taken.
pub(super) fn glass_standing(damage: &[(u32, u16)], piece: u16) -> bool {
    let taken = damage.iter().find(|(id, _)| *id == u32::from(piece)).map_or(0, |(_, taken)| *taken);
    crate::world_objects::glass_piece_is_solid(taken)
}

/// What one missile's traces run against during a tick: the map, the players it can meet and the scripted solids.
pub(super) struct Sweep<'a> {
    pub content: &'a SimContent,
    pub bodies: &'a [PlayerCollisionPose],
    pub models: &'a [EntityCollisionTraceGeom],
}

impl Sweep<'_> {
    /// The plain bullet trace over `players`, with `solid` deciding which glass panes stop it.
    pub(super) fn bullet(
        &self,
        players: &[PlayerCollisionPose],
        query: &BulletTraceQuery,
        solid: &dyn Fn(u16) -> bool,
    ) -> TraceOutcome {
        bullet_trace_with_entity_models(
            self.content.clip_brushes(),
            self.content.clip_bsp(),
            self.content.clip_cmodels(),
            self.content.clip_mesh(),
            players,
            self.models,
            query,
            solid,
        )
    }

    /// The missile trace (player hit boxes handled as `missile_trace` describes), with the panes in `damage` as hurt.
    pub(super) fn missile(&self, query: &BulletTraceQuery, damage: &[(u32, u16)]) -> TraceOutcome {
        let solid = |piece: u16| glass_standing(damage, piece);
        missile_trace(self.bodies, query, &|players, wanted| self.bullet(players, wanted, &solid))
    }
}
