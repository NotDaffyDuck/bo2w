//! What a missile can meet, and the small questions it asks of the world around it.

use trace_t6::surface_kind_of_flags;

use super::facts::GadgetLiveStats;
use super::vec3::{dot, norm};
use crate::shot_trace::{EntityRayShape, PlayerHitboxPose};
use crate::step_view::StepWorldView;
use crate::match_ids::ThrownId;
use crate::{ShotRayRequest, HitTargetId, ScriptedModel, SimContent, RayResult, trace_shot_against_models};

pub(super) const SURF_FLESH: u8 = 7;
const SURF_GLASS: u8 = 9;
pub(super) const SURF_WATER: u8 = 20;
pub(super) const SURF_RIOTSHIELD: u8 = 31;
const CONTENTS_WATER: u32 = movement_t6::water::CONTENTS_WATER;

/// The players a missile can meet. The server meets them by their hit boxes as a bullet does; the client draws the
/// server's path and never bounces a missile off a player. A prediction copy knows the other players only as a
/// whole-body box, so those are left out here and the next snapshot brings the bounce.
pub(super) fn missile_bodies(world: &StepWorldView) -> Vec<PlayerHitboxPose> {
    let remote = world.prediction_remote_bodies();
    let mut bodies = world.alive_collision_poses();
    bodies.retain(|pose| remote.iter().all(|body| body.client != pose.client));
    bodies
}

/// The equipment row of `weapon`, or its launcher row. A missile with neither is a bug in the content, not something
/// to run with zeroes.
pub(super) fn required_facts(world: &StepWorldView, weapon: u32) -> GadgetLiveStats {
    match world.gadget_stats_lookup(weapon).or_else(|| world.rocket_launch_facts(weapon)) {
        Some(facts) => facts,
        None => panic!("a missile needs a validated equipment row; its bounce and damage figures cannot fall back to zero"),
    }
}

/// A fired projectile's (impact damage, blast radius) for the BO2 scripts' `projectile_impact` and the models its path
/// goes through.
pub(crate) fn projectile_impact_facts(world: &StepWorldView, weapon: u32) -> Option<(i32, f32)> {
    let f = world.gadget_stats_lookup(weapon).or_else(|| world.rocket_launch_facts(weapon))?;
    Some((f.collision_harm.max(0), f.blast_outer_reach.max(f.blast_reach_floor).max(0) as f32))
}

/// Every scripted solid except the ones attached to missile `id`.
pub(super) fn script_models_without(world: &StepWorldView, id: ThrownId) -> Vec<EntityRayShape> {
    let attached = world.missile_collision_models(id);
    world
        .entity_hit_support()
        .iter()
        .filter(|caps| match caps.owner.scripted_model() {
            Some(model) => !attached.contains(&model),
            None => true,
        })
        .map(|caps| caps.trace_geom())
        .collect()
}

/// Whether `at` is in water.
pub(super) fn in_water(world: &StepWorldView, at: [f32; 3]) -> bool {
    let probe = world.trace_clip(at, at, [0.0; 3], [0.0; 3], CONTENTS_WATER);
    probe.starts_inside || probe.fully_embedded
}

/// A scripted glass that takes damage struck by a missile: the model, the bone hit (None for a linked brush) and
/// whether it can break. None for anything else (world glass breaks as a pane).
pub(super) fn entity_glass(world: &mut StepWorldView, collider: HitTargetId) -> Option<(ScriptedModel, Option<usize>, bool)> {
    let (owner, bone, flags) = match collider {
        HitTargetId::PosedBone { owner, bone, surface_flags: face_bits, .. } => (owner, Some(usize::from(bone)), face_bits),
        HitTargetId::AttachedBrush { owner, surface_flags: face_bits, .. } => (owner, None, face_bits),
        HitTargetId::World { .. } | HitTargetId::Player { .. } => return None,
    };
    if surface_kind_of_flags(flags) != SURF_GLASS {
        return None;
    }
    let model = owner.scripted_model()?;
    let breakable = crate::zombie_rt::destructible::takes_damage(world.ecs(), model)?;
    Some((model, bone, breakable))
}

/// A missile that stuck to a scripted entity tells the scripts which one.
pub(super) fn tell_struck_entity(world: &mut StepWorldView, id: ThrownId, collider: HitTargetId) {
    let owner = match collider {
        HitTargetId::PosedBone { owner, .. } | HitTargetId::AttachedBrush { owner, .. } => owner,
        HitTargetId::World { .. } | HitTargetId::Player { .. } => return,
    };
    if let Some(model) = owner.scripted_model() {
        crate::zombie_rt::grenade_stuck::struck(world.ecs(), id.0, model);
    }
}

/// The facing a hit gets when the move started inside something: straight back along the move, or none when it did
/// not move.
pub(super) fn startsolid_normal(from: [f32; 3], till: [f32; 3]) -> [f32; 3] {
    let back = [from[0] - till[0], from[1] - till[1], from[2] - till[2]];
    let len = norm(back);
    if len < 1e-6 {
        return [0.0; 3];
    }
    [back[0] / len, back[1] / len, back[2] / len]
}

/// Whether `point` lies inside the box of `bone`.
fn inside_bone(bone: &xmodel_runtime::HitShapeBone, at: [f32; 3]) -> bool {
    let offset: [f32; 3] = core::array::from_fn(|axis| at[axis] - bone.center[axis]);
    (0..3).all(|axis| dot(offset, bone.axes[axis]).abs() <= bone.half_ext[axis])
}

/// A missile's trace against player hit boxes. One that starts inside a hit box and leaves it passes out freely; one
/// that stays inside while it moves in toward the player's middle stops where it is, facing level and straight out
/// from his middle. (The shared trace calls every start inside a hit box a start inside something.)
pub(super) fn missile_trace(
    players: &[PlayerHitboxPose],
    probe: &ShotRayRequest,
    trace: &dyn Fn(&[PlayerHitboxPose], &ShotRayRequest) -> RayResult,
) -> RayResult {
    let first = trace(players, probe);
    if !matches!(first, RayResult::BeganInside { struck: Some(HitTargetId::Player { .. }), .. }) {
        return first;
    }
    let heading = [probe.to[0] - probe.from[0], probe.to[1] - probe.from[1]];
    let mut outside = players.to_vec();
    for stance in &mut outside {
        if probe.ignore == Some(stance.client) || probe.skip_client_hit == Some(stance.client) {
            continue;
        }
        let away = [probe.from[0] - stance.origin[0], probe.from[1] - stance.origin[1]];
        if heading[0] * away[0] + heading[1] * away[1] < 0.0
            && let Some(bone) = stance.joints.iter().find(|b| inside_bone(b, probe.from) && inside_bone(b, probe.to))
        {
            let len = away[0].hypot(away[1]);
            let normal = if len > 0.0 { [away[0] / len, away[1] / len, 0.0] } else { [0.0; 3] };
            return RayResult::Hit {
                fraction: 0.0,
                end: probe.from,
                normal,
                struck: HitTargetId::Player {
                    client: stance.client,
                    life: stance.life_sequence,
                    hitloc: bone.body_part_category,
                },
            };
        }
        stance.joints.retain(|b| !inside_bone(b, probe.from));
    }
    trace(&outside, probe)
}

/// Whether glass pane `piece` still stands, given the damage each hit pane has taken.
pub(super) fn glass_standing(damage: &[(u32, u16)], piece: u16) -> bool {
    let taken = damage.iter().find(|(id, _)| *id == u32::from(piece)).map_or(0, |(_, taken)| *taken);
    crate::glass_and_props::shard_blocks_shots(taken)
}

/// What one missile's traces run against during a tick: the map, the players it can meet and the scripted solids.
pub(super) struct Sweep<'a> {
    pub content: &'a SimContent,
    pub bodies: &'a [PlayerHitboxPose],
    pub models: &'a [EntityRayShape],
}

impl Sweep<'_> {
    /// The plain bullet trace over `players`, with `solid` deciding which glass panes stop it.
    pub(super) fn bullet(
        &self,
        players: &[PlayerHitboxPose],
        query: &ShotRayRequest,
        solid: &dyn Fn(u16) -> bool,
    ) -> RayResult {
        trace_shot_against_models(
            self.content.collision_solids(),
            self.content.collision_tree(),
            self.content.collision_submodel_set(),
            self.content.collision_triangles(),
            players,
            self.models,
            query,
            solid,
        )
    }

    /// The missile trace (player hit boxes handled as `missile_trace` describes), with the panes in `damage` as hurt.
    pub(super) fn missile(&self, query: &ShotRayRequest, damage: &[(u32, u16)]) -> RayResult {
        let solid = |piece: u16| glass_standing(damage, piece);
        missile_trace(self.bodies, query, &|players, wanted| self.bullet(players, wanted, &solid))
    }
}
